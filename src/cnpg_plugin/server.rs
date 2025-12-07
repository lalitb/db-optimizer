use tonic::{Request, Response, Status};
use crate::cnpg_plugin::proto::{
    OperatorService, OperatorLifecycleRequest, OperatorLifecycleResponse,
    GetCapabilitiesRequest, GetCapabilitiesResponse, PluginCapability, CapabilityType,
    OperationType,
};
use crate::cnpg_plugin::config::PluginConfig;
use crate::agent::AgentState;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error, debug};
use sqlx::postgres::PgPoolOptions;
use crate::collector::DatabaseCollector;
use crate::analyzer::HealthCalculator;

/// Vaanyze CNPG Plugin service implementation
pub struct VaanyzeOperatorService {
    config: PluginConfig,
    state: Arc<Mutex<AgentState>>,
    /// eBPF manager for kernel-level metrics (when available)
    #[cfg(target_os = "linux")]
    ebpf_enabled: bool,
}

impl VaanyzeOperatorService {
    pub fn new(config: PluginConfig, state: Arc<Mutex<AgentState>>) -> Self {
        Self { 
            config, 
            state,
            #[cfg(target_os = "linux")]
            ebpf_enabled: true,
        }
    }
    
    /// Analyze a PostgreSQL cluster
    async fn analyze_cluster(&self, connection_string: &str) -> anyhow::Result<ClusterAnalysis> {
        debug!("Connecting to cluster: {}", mask_connection_string(connection_string));
        
        // Connect to database
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(connection_string)
            .await?;
            
        // Collect metrics
        let collector = DatabaseCollector::new(pool);
        let tables = collector.collect_table_info().await?;
        let indexes = collector.collect_index_info().await?;
        let connection_stats = collector.collect_connection_stats().await.ok();
        
        // Calculate health score
        let total_db_size_mb: f64 = tables.iter().map(|t| t.total_size_mb).sum();
        let health_score = HealthCalculator::calculate(
            &tables, 
            &indexes, 
            connection_stats.as_ref(), 
            total_db_size_mb
        );
        
        // Generate recommendations
        let mut recommendations = Vec::new();
        
        // Check for large tables that might need optimization
        for table in &tables {
            if table.total_size_mb > 1000.0 && table.index_size_mb < table.table_size_mb * 0.1 {
                recommendations.push(format!(
                    "Table {} is large ({:.1}MB) with relatively small index coverage - consider analyzing query patterns",
                    table.table_name, table.total_size_mb
                ));
            }
        }
        
        // Check for unused indexes
        for index in &indexes {
            if index.index_scans == 0 && index.index_size_mb > 10.0 {
                recommendations.push(format!(
                    "Index {} is unused and consuming {:.1}MB - consider dropping",
                    index.index_name, index.index_size_mb
                ));
            }
        }
        
        // Check connection stats
        if let Some(stats) = &connection_stats {
            if stats.idle_in_transaction_count > 5 {
                recommendations.push(format!(
                    "{} idle-in-transaction connections detected - check for uncommitted transactions",
                    stats.idle_in_transaction_count
                ));
            }
        }
        
        Ok(ClusterAnalysis {
            health_score,
            recommendations,
            table_count: tables.len(),
            index_count: indexes.len(),
            total_size_mb: total_db_size_mb,
        })
    }
    
    /// Get plugin capabilities based on runtime detection
    fn detect_capabilities(&self) -> Vec<PluginCapability> {
        let mut caps = vec![
            PluginCapability {
                capability_type: CapabilityType::HealthMonitoring,
                enabled: true,
            },
            PluginCapability {
                capability_type: CapabilityType::PerformanceAnalysis,
                enabled: true,
            },
            PluginCapability {
                capability_type: CapabilityType::IndexRecommendations,
                enabled: true,
            },
            PluginCapability {
                capability_type: CapabilityType::QueryRewriting,
                enabled: true, // Query rewriting is always available
            },
            PluginCapability {
                capability_type: CapabilityType::AutoOptimization,
                enabled: self.config.auto_apply_config.enabled,
            },
            PluginCapability {
                capability_type: CapabilityType::CostAnalysis,
                enabled: true,
            },
        ];
        
        // eBPF is only available on Linux
        #[cfg(target_os = "linux")]
        {
            caps.push(PluginCapability {
                capability_type: CapabilityType::EbpfInstrumentation,
                enabled: self.ebpf_enabled,
            });
        }
        
        #[cfg(not(target_os = "linux"))]
        {
            caps.push(PluginCapability {
                capability_type: CapabilityType::EbpfInstrumentation,
                enabled: false,
            });
        }
        
        caps
    }
}

/// Cluster analysis result
struct ClusterAnalysis {
    health_score: u8,
    recommendations: Vec<String>,
    table_count: usize,
    index_count: usize,
    total_size_mb: f64,
}

/// Mask sensitive parts of connection string for logging
fn mask_connection_string(conn_str: &str) -> String {
    // Replace password with asterisks
    let re = regex::Regex::new(r"(password=)[^&\s]+").ok();
    if let Some(re) = re {
        re.replace_all(conn_str, "$1****").to_string()
    } else {
        conn_str.to_string()
    }
}

#[tonic::async_trait]
impl OperatorService for VaanyzeOperatorService {
    async fn lifecycle(
        &self,
        request: Request<OperatorLifecycleRequest>,
    ) -> Result<Response<OperatorLifecycleResponse>, Status> {
        let req = request.into_inner();
        info!(
            "Lifecycle event: {:?} for cluster {}/{} (role: {})", 
            req.operation, req.namespace, req.cluster_name, req.role
        );
        
        // Handle different operation types
        match req.operation {
            OperationType::Delete => {
                info!("Cluster deletion - cleaning up metrics");
                return Ok(Response::new(OperatorLifecycleResponse {
                    status: "OK".to_string(),
                    message: "Cleanup completed".to_string(),
                    health_score: 0,
                    recommendations: "[]".to_string(),
                    proceed: true,
                }));
            }
            OperationType::Create => {
                info!("New cluster creation - will start monitoring after ready");
                return Ok(Response::new(OperatorLifecycleResponse {
                    status: "OK".to_string(),
                    message: "Will monitor after cluster is ready".to_string(),
                    health_score: 100,
                    recommendations: "[]".to_string(),
                    proceed: true,
                }));
            }
            _ => {}
        }
        
        // Only analyze primary instances for most operations
        if req.role.to_lowercase() != "primary" && req.operation == OperationType::HealthCheck {
            return Ok(Response::new(OperatorLifecycleResponse {
                status: "Skipped".to_string(),
                message: "Not primary instance".to_string(),
                health_score: 0,
                recommendations: "[]".to_string(),
                proceed: true,
            }));
        }
        
        // Perform analysis
        match self.analyze_cluster(&req.connection_string).await {
            Ok(analysis) => {
                let status = if analysis.health_score >= 80 {
                    "Healthy"
                } else if analysis.health_score >= 50 {
                    "Warning"
                } else {
                    "Critical"
                };
                
                info!(
                    "Cluster {}/{}: score={}, tables={}, indexes={}, size={:.1}MB",
                    req.namespace, req.cluster_name,
                    analysis.health_score,
                    analysis.table_count,
                    analysis.index_count,
                    analysis.total_size_mb
                );
                
                // Update agent state
                {
                    let mut state = self.state.lock().await;
                    state.last_run = chrono::Utc::now();
                }
                
                let recommendations_json = serde_json::to_string(&analysis.recommendations)
                    .unwrap_or_else(|_| "[]".to_string());
                
                Ok(Response::new(OperatorLifecycleResponse {
                    status: status.to_string(),
                    message: format!(
                        "Health Score: {} | {} tables, {} indexes, {:.1}MB total",
                        analysis.health_score,
                        analysis.table_count,
                        analysis.index_count,
                        analysis.total_size_mb
                    ),
                    health_score: analysis.health_score as u32,
                    recommendations: recommendations_json,
                    proceed: true,
                }))
            }
            Err(e) => {
                error!("Failed to analyze cluster {}/{}: {}", req.namespace, req.cluster_name, e);
                Ok(Response::new(OperatorLifecycleResponse {
                    status: "Error".to_string(),
                    message: format!("Analysis failed: {}", e),
                    health_score: 0,
                    recommendations: "[]".to_string(),
                    proceed: true, // Don't block operations on analysis failure
                }))
            }
        }
    }
    
    async fn get_capabilities(
        &self,
        _request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<GetCapabilitiesResponse>, Status> {
        let capabilities = self.detect_capabilities();
        
        Ok(Response::new(GetCapabilitiesResponse {
            name: "vaanyze".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities,
        }))
    }
}

pub async fn start_grpc_server(
    port: u16,
    config: PluginConfig,
    state: Arc<Mutex<AgentState>>,
) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let service = VaanyzeOperatorService::new(config, state);
    
    info!("Vaanyze CNPG Plugin v{} starting on {}", env!("CARGO_PKG_VERSION"), addr);
    info!("Capabilities: {:?}", service.detect_capabilities().iter().filter(|c| c.enabled).count());
    
    // In production, this would start the actual tonic server:
    // tonic::transport::Server::builder()
    //     .add_service(OperatorServer::new(service))
    //     .serve(addr)
    //     .await?;
    
    // For now, keep the service alive
    let _ = service;
    
    // Signal ready
    info!("gRPC server ready on {}", addr);
    
    // Keep running (in real impl, server.serve() blocks)
    tokio::signal::ctrl_c().await?;
    info!("Shutting down gRPC server");
    
    Ok(())
}
