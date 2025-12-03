use tonic::{Request, Response, Status};
use crate::cnpg_plugin::proto::{OperatorService, OperatorLifecycleRequest, OperatorLifecycleResponse};
use crate::cnpg_plugin::config::PluginConfig;
use crate::agent::AgentState;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error};
use sqlx::postgres::PgPoolOptions;
use crate::collector::DatabaseCollector;
use crate::analyzer::HealthCalculator;

pub struct VaanyzeOperatorService {
    _config: PluginConfig,
    _state: Arc<Mutex<AgentState>>,
}

impl VaanyzeOperatorService {
    pub fn new(config: PluginConfig, state: Arc<Mutex<AgentState>>) -> Self {
        Self { _config: config, _state: state }
    }
    
    async fn analyze_cluster(&self, connection_string: &str) -> anyhow::Result<u8> {
        // Connect to database
        let pool = PgPoolOptions::new()
            .max_connections(5)
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
        
        Ok(health_score)
    }
}

#[tonic::async_trait]
impl OperatorService for VaanyzeOperatorService {
    async fn lifecycle(
        &self,
        request: Request<OperatorLifecycleRequest>,
    ) -> Result<Response<OperatorLifecycleResponse>, Status> {
        let req = request.into_inner();
        info!("Received lifecycle request for cluster: {}/{}", req.namespace, req.cluster_name);
        
        // Only analyze primary instances
        if req.role.to_lowercase() == "primary" {
            match self.analyze_cluster(&req.connection_string).await {
                Ok(score) => {
                    info!("Cluster health score: {}", score);
                    
                    // Here we would implement logic to:
                    // 1. Report score to Vaanyze Cloud
                    // 2. Trigger auto-optimization if enabled and score is low
                    
                    return Ok(Response::new(OperatorLifecycleResponse {
                        status: "Healthy".to_string(),
                        message: format!("Health Score: {}", score),
                    }));
                }
                Err(e) => {
                    error!("Failed to analyze cluster: {}", e);
                    return Ok(Response::new(OperatorLifecycleResponse {
                        status: "Warning".to_string(),
                        message: format!("Analysis failed: {}", e),
                    }));
                }
            }
        }
        
        Ok(Response::new(OperatorLifecycleResponse {
            status: "Skipped".to_string(),
            message: "Not primary instance".to_string(),
        }))
    }
}

pub async fn start_grpc_server(
    port: u16,
    config: PluginConfig,
    state: Arc<Mutex<AgentState>>,
) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let _service = VaanyzeOperatorService::new(config, state);
    
    info!("gRPC server listening on {}", addr);
    
    // In a real implementation, we would use the generated Server struct
    // Server::builder()
    //     .add_service(OperatorServer::new(service))
    //     .serve(addr)
    //     .await?;
        
    Ok(())
}
