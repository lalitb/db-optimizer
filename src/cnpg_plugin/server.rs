use tonic::{Request, Response, Status};
use crate::cnpg_plugin::proto::{
    OperatorLifecycle, OperatorLifecycleServer,
    OperatorLifecycleRequest, OperatorLifecycleResponse,
    OperatorLifecycleCapabilitiesRequest, OperatorLifecycleCapabilitiesResponse,
    OperatorLifecycleCapabilities, OperatorOperationType,
    operator_operation_type::Type as OpType,
    Operator, OperatorServer,
    OperatorCapabilitiesRequest, OperatorCapabilitiesResult,
    OperatorMutateClusterRequest, OperatorMutateClusterResult,
    OperatorValidateClusterCreateRequest, OperatorValidateClusterCreateResult,
    OperatorValidateClusterChangeRequest, OperatorValidateClusterChangeResult,
    SetStatusInClusterRequest, SetStatusInClusterResponse,
    DeregisterRequest, DeregisterResponse,
    OperatorCapability, Rpc, RpcType
};
use crate::cnpg_plugin::config::PluginConfig;
use crate::agent::AgentState;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error, warn};
use sqlx::postgres::PgPoolOptions;
use crate::collector::DatabaseCollector;
use crate::analyzer::HealthCalculator;
use crate::config_tuner::{ConfigTuner, SystemInfo, WorkloadType, DiskType, OsType};
use tonic::transport::Server;
use serde_json::json;

#[cfg(feature = "cnpg")]
use kube::{Client, Api, core::ObjectMeta};
#[cfg(feature = "cnpg")]
use k8s_openapi::api::core::v1::Secret;

#[derive(Clone)]
pub struct VaanyzeOperatorService {
    _config: PluginConfig,
    _state: Arc<Mutex<AgentState>>,
    _tuner: Arc<ConfigTuner>,
}

impl VaanyzeOperatorService {
    pub fn new(config: PluginConfig, state: Arc<Mutex<AgentState>>) -> Self {
        Self { 
            _config: config, 
            _state: state,
            _tuner: Arc::new(ConfigTuner::new()),
        }
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

    fn parse_k8s_quantity(quantity: &str) -> Option<u64> {
        // Simple parser for K8s quantities like "1Gi", "512Mi", "1000m"
        let quantity = quantity.trim();
        if quantity.is_empty() { return None; }
        
        let (num_str, unit) = quantity.split_at(quantity.find(|c: char| !c.is_numeric() && c != '.').unwrap_or(quantity.len()));
        let num = num_str.parse::<f64>().ok()?;
        
        match unit {
            "Ki" => Some((num * 1024.0) as u64),
            "Mi" => Some((num * 1024.0 * 1024.0) as u64),
            "Gi" => Some((num * 1024.0 * 1024.0 * 1024.0) as u64),
            "Ti" => Some((num * 1024.0 * 1024.0 * 1024.0 * 1024.0) as u64),
            "m" => Some((num / 1000.0) as u64), // For CPU, but we return u64... handle separately?
            "" => Some(num as u64),
            _ => None,
        }
    }

    fn extract_system_info(&self, cluster_definition: &[u8]) -> Option<SystemInfo> {
        let json: serde_json::Value = serde_json::from_slice(cluster_definition).ok()?;
        
        // Try to find resources in spec.resources (standard K8s) or spec.postgresql.resources (CNPG specific?)
        // CNPG usually puts resources in spec.resources
        let resources = json.get("spec")?.get("resources")?;
        let requests = resources.get("requests").or_else(|| resources.get("limits"))?;
        
        let memory_str = requests.get("memory")?.as_str()?;
        let cpu_str = requests.get("cpu")?.as_str()?;
        
        let memory_bytes = Self::parse_k8s_quantity(memory_str)?;
        // CPU is a bit tricky, parse_k8s_quantity returns bytes for memory. 
        // Let's do a quick parse for CPU
        let cpu_count = if cpu_str.ends_with('m') {
            let m = cpu_str.trim_end_matches('m').parse::<f64>().ok()?;
            (m / 1000.0).ceil() as u32
        } else {
            cpu_str.parse::<f64>().ok()?.ceil() as u32
        };
        
        Some(SystemInfo {
            total_ram_kb: memory_bytes / 1024,
            cpu_count: std::cmp::max(1, cpu_count),
            disk_type: DiskType::SSD, // Assume SSD in cloud
            os_type: OsType::Linux,
            pg_version: 15.0, // Default
        })
    }

    async fn extract_connection_info(&self, cluster_definition: &[u8]) -> Option<String> {
        let json: serde_json::Value = match serde_json::from_slice(cluster_definition) {
            Ok(j) => j,
            Err(e) => {
                error!("Failed to parse cluster definition JSON: {}", e);
                return None;
            }
        };

        let name = json.get("metadata")?.get("name")?.as_str()?;
        let namespace = json.get("metadata")?.get("namespace")?.as_str().unwrap_or("default");
        
        info!("Processing cluster: {}/{}", namespace, name);

        // Construct service name (standard CNPG convention: <cluster-name>-rw)
        let service_host = format!("{}-rw.{}.svc", name, namespace);
        
        // Fetch password from Kubernetes Secret
        // Secret name is usually <cluster-name>-superuser or <cluster-name>-app
        // We'll try superuser first
        let secret_name = format!("{}-superuser", name);
        
        #[cfg(feature = "cnpg")]
        {
            match Client::try_default().await {
                Ok(client) => {
                    let secrets: Api<Secret> = Api::namespaced(client, namespace);
                    match secrets.get(&secret_name).await {
                        Ok(secret) => {
                            if let Some(data) = secret.data {
                                if let Some(password_bytes) = data.get("password") {
                                    let password = String::from_utf8_lossy(&password_bytes.0);
                                    // Construct connection string
                                    // postgres://postgres:<password>@<service-host>:5432/postgres
                                    return Some(format!("postgres://postgres:{}@{}:5432/postgres", password, service_host));
                                }
                            }
                        },
                        Err(e) => {
                            warn!("Failed to fetch secret {}: {}", secret_name, e);
                        }
                    }
                },
                Err(e) => {
                    warn!("Failed to create Kubernetes client: {}", e);
                }
            }
        }
        
        #[cfg(not(feature = "cnpg"))]
        warn!("CNPG feature not enabled, cannot fetch secrets from Kubernetes");

        None 
    }
}

#[tonic::async_trait]
impl OperatorLifecycle for VaanyzeOperatorService {
    async fn get_capabilities(
        &self,
        _request: Request<OperatorLifecycleCapabilitiesRequest>,
    ) -> Result<Response<OperatorLifecycleCapabilitiesResponse>, Status> {
        Ok(Response::new(OperatorLifecycleCapabilitiesResponse {
            lifecycle_capabilities: vec![
                OperatorLifecycleCapabilities {
                    group: "".to_string(),
                    kind: "Cluster".to_string(),
                    operation_types: vec![
                        OperatorOperationType {
                            r#type: OpType::Evaluate as i32,
                        }
                    ],
                }
            ],
        }))
    }

    async fn lifecycle_hook(
        &self,
        request: Request<OperatorLifecycleRequest>,
    ) -> Result<Response<OperatorLifecycleResponse>, Status> {
        let req = request.into_inner();
        
        // Check operation type
        if let Some(op_type) = req.operation_type {
             if op_type.r#type != (OpType::Evaluate as i32) {
                 return Ok(Response::new(OperatorLifecycleResponse {
                     json_patch: vec![],
                 }));
             }
        }

        let mut json_patch = vec![];

        // Auto-optimization logic
        if self._config.enable_auto_optimize {
            if let Some(system_info) = self.extract_system_info(&req.cluster_definition) {
                // Detect workload (try to connect, fallback to Web)
                let workload = if let Some(conn_str) = self.extract_connection_info(&req.cluster_definition).await {
                    if let Ok(pool) = PgPoolOptions::new().connect(&conn_str).await {
                        self._tuner.detect_workload(&pool).await.unwrap_or(WorkloadType::Web)
                    } else {
                        WorkloadType::Web
                    }
                } else {
                    WorkloadType::Web
                };

                info!("Calculating optimal config for {:?} workload on {} CPUs, {} MB RAM", 
                    workload, system_info.cpu_count, system_info.total_ram_kb / 1024);

                let optimal_config = self._tuner.calculate_optimal_config(&system_info, workload);
                
                // Generate JSON Patch
                // We need to check if spec.postgresql.parameters exists in the input JSON
                let cluster_json: serde_json::Value = serde_json::from_slice(&req.cluster_definition).unwrap_or(json!({}));
                let has_parameters = cluster_json.get("spec")
                    .and_then(|s| s.get("postgresql"))
                    .and_then(|p| p.get("parameters"))
                    .is_some();

                let mut patch_ops = vec![];

                // If parameters map doesn't exist, we'd ideally create it, but JSON Patch is tricky with nested missing objects.
                // For now, we assume the user has at least an empty parameters map or we use a safe path.
                // Actually, CNPG CRD structure is spec.postgresql.parameters map[string]string
                
                // Let's generate a patch for each parameter
                // Note: This assumes spec.postgresql.parameters exists. 
                // If not, we should add the whole map.
                
                if !has_parameters {
                    // Add the parameters map with our values
                    let mut params_map = serde_json::Map::new();
                    params_map.insert("shared_buffers".to_string(), json!(optimal_config.shared_buffers));
                    params_map.insert("effective_cache_size".to_string(), json!(optimal_config.effective_cache_size));
                    params_map.insert("work_mem".to_string(), json!(optimal_config.work_mem));
                    params_map.insert("maintenance_work_mem".to_string(), json!(optimal_config.maintenance_work_mem));
                    params_map.insert("max_connections".to_string(), json!(optimal_config.max_connections.to_string()));
                    
                    patch_ops.push(json!({
                        "op": "add",
                        "path": "/spec/postgresql/parameters",
                        "value": params_map
                    }));
                } else {
                    // Update individual parameters
                    patch_ops.push(json!({ "op": "add", "path": "/spec/postgresql/parameters/shared_buffers", "value": optimal_config.shared_buffers }));
                    patch_ops.push(json!({ "op": "add", "path": "/spec/postgresql/parameters/effective_cache_size", "value": optimal_config.effective_cache_size }));
                    patch_ops.push(json!({ "op": "add", "path": "/spec/postgresql/parameters/work_mem", "value": optimal_config.work_mem }));
                    patch_ops.push(json!({ "op": "add", "path": "/spec/postgresql/parameters/maintenance_work_mem", "value": optimal_config.maintenance_work_mem }));
                    patch_ops.push(json!({ "op": "add", "path": "/spec/postgresql/parameters/max_connections", "value": optimal_config.max_connections.to_string() }));
                }

                if !patch_ops.is_empty() {
                    info!("Generated {} optimization patches", patch_ops.len());
                    json_patch = serde_json::to_vec(&patch_ops).unwrap_or_default();
                }
            }
        }

        if let Some(conn_str) = self.extract_connection_info(&req.cluster_definition).await {
            match self.analyze_cluster(&conn_str).await {
                Ok(score) => {
                    info!("Cluster health score: {}", score);
                }
                Err(e) => {
                    error!("Failed to analyze cluster: {}", e);
                }
            }
        }
        
        Ok(Response::new(OperatorLifecycleResponse {
            json_patch,
        }))
    }
}

#[tonic::async_trait]
impl Operator for VaanyzeOperatorService {
    async fn get_capabilities(
        &self,
        _request: Request<OperatorCapabilitiesRequest>,
    ) -> Result<Response<OperatorCapabilitiesResult>, Status> {
        Ok(Response::new(OperatorCapabilitiesResult {
            capabilities: vec![
                OperatorCapability {
                    r#type: Some(Rpc {
                        r#type: RpcType::MutateCluster as i32,
                    }),
                }
            ],
        }))
    }

    async fn mutate_cluster(
        &self,
        _request: Request<OperatorMutateClusterRequest>,
    ) -> Result<Response<OperatorMutateClusterResult>, Status> {
        // Placeholder for future eBPF sidecar injection
        // In the future, we will inject a sidecar container here that runs
        // the eBPF agent (io_latency.bpf.c, lock_contention.bpf.c)
        // alongside the PostgreSQL container.
        
        Ok(Response::new(OperatorMutateClusterResult {
            json_patch: vec![],
        }))
    }

    async fn validate_cluster_create(
        &self,
        _request: Request<OperatorValidateClusterCreateRequest>,
    ) -> Result<Response<OperatorValidateClusterCreateResult>, Status> {
        Ok(Response::new(OperatorValidateClusterCreateResult {
            validation_errors: vec![],
        }))
    }

    async fn validate_cluster_change(
        &self,
        _request: Request<OperatorValidateClusterChangeRequest>,
    ) -> Result<Response<OperatorValidateClusterChangeResult>, Status> {
        Ok(Response::new(OperatorValidateClusterChangeResult {
            validation_errors: vec![],
        }))
    }

    async fn set_status_in_cluster(
        &self,
        _request: Request<SetStatusInClusterRequest>,
    ) -> Result<Response<SetStatusInClusterResponse>, Status> {
        Ok(Response::new(SetStatusInClusterResponse {
            json_status: vec![],
        }))
    }

    async fn deregister(
        &self,
        _request: Request<DeregisterRequest>,
    ) -> Result<Response<DeregisterResponse>, Status> {
        Ok(Response::new(DeregisterResponse {}))
    }
}

pub async fn start_grpc_server(
    port: u16,
    config: PluginConfig,
    state: Arc<Mutex<AgentState>>,
) -> anyhow::Result<()> {
    let addr = format!("0.0.0.0:{}", port).parse()?;
    let service = VaanyzeOperatorService::new(config, state);
    
    info!("gRPC server listening on {}", addr);
    
    Server::builder()
        .add_service(OperatorLifecycleServer::new(service.clone()))
        .add_service(OperatorServer::new(service))
        .serve(addr)
        .await?;
        
    Ok(())
}
