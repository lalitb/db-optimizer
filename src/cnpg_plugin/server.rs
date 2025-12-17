use crate::agent::AgentState;
use crate::cnpg_plugin::cnpgi::operator_v1;
use crate::cnpg_plugin::config::PluginConfig;
use crate::config_tuner::{
    models::{DiskType, OsType, SystemInfo, WorkloadType},
    ConfigTuner,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

/// Vaanyze CNPG Operator plugin implementation (CNPG-I `Operator` service)
#[derive(Clone)]
pub struct VaanyzeOperatorService {
    config: PluginConfig,
    state: Arc<Mutex<AgentState>>,
    grpc_ready: Arc<AtomicBool>,
}

impl VaanyzeOperatorService {
    pub fn new(
        config: PluginConfig,
        state: Arc<Mutex<AgentState>>,
        grpc_ready: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            state,
            grpc_ready,
        }
    }

    fn is_opted_in(cluster: &Value) -> bool {
        parse_bool(Self::annotation(cluster, "vaanyze.io/enable-tuning"))
    }

    fn workload(cluster: &Value) -> WorkloadType {
        match Self::annotation(cluster, "vaanyze.io/workload")
            .unwrap_or("mixed")
            .trim()
            .to_lowercase()
            .as_str()
        {
            "web" => WorkloadType::Web,
            "oltp" => WorkloadType::OLTP,
            "dw" | "datawarehouse" | "data-warehouse" => WorkloadType::DataWarehouse,
            "desktop" => WorkloadType::Desktop,
            _ => WorkloadType::Mixed,
        }
    }

    fn manage_max_connections(cluster: &Value) -> bool {
        parse_bool(Self::annotation(
            cluster,
            "vaanyze.io/manage-max-connections",
        ))
    }

    fn disk_type(cluster: &Value) -> DiskType {
        match Self::annotation(cluster, "vaanyze.io/disk")
            .unwrap_or("ssd")
            .trim()
            .to_lowercase()
            .as_str()
        {
            "hdd" => DiskType::HDD,
            "san" => DiskType::SAN,
            _ => DiskType::SSD,
        }
    }

    fn annotation<'a>(cluster: &'a Value, key: &str) -> Option<&'a str> {
        cluster
            .get("metadata")
            .and_then(|m| m.get("annotations"))
            .and_then(|a| a.get(key))
            .and_then(|v| v.as_str())
    }

    fn has_existing_parameter(cluster: &Value, key: &str) -> bool {
        cluster
            .get("spec")
            .and_then(|s| s.get("postgresql"))
            .and_then(|p| p.get("parameters"))
            .and_then(|p| p.get(key))
            .is_some()
    }

    fn infer_system_info(&self, cluster: &Value) -> SystemInfo {
        let (cpu_count, total_ram_kb) = infer_cpu_and_memory_from_cluster(cluster).unwrap_or((
            self.config.default_cpus,
            self.config.default_ram_gb * 1024 * 1024,
        ));

        SystemInfo {
            total_ram_kb,
            cpu_count,
            disk_type: Self::disk_type(cluster),
            os_type: OsType::Linux,
            pg_version: 16.0,
        }
    }

    fn optimal_parameters(&self, cluster: &Value) -> BTreeMap<String, String> {
        let system_info = self.infer_system_info(cluster);
        let workload = Self::workload(cluster);

        let tuner = ConfigTuner::new();
        let optimal = tuner.calculate_optimal_config(&system_info, workload);

        let mut out = BTreeMap::new();
        out.insert(
            "shared_buffers".to_string(),
            optimal.shared_buffers_display(),
        );
        out.insert(
            "effective_cache_size".to_string(),
            optimal.effective_cache_size_display(),
        );
        out.insert(
            "maintenance_work_mem".to_string(),
            optimal.maintenance_work_mem_display(),
        );
        out.insert("work_mem".to_string(), optimal.work_mem_display());
        out.insert(
            "min_wal_size".to_string(),
            format!("{}MB", optimal.min_wal_size_mb),
        );
        out.insert(
            "max_wal_size".to_string(),
            format!("{}MB", optimal.max_wal_size_mb),
        );
        out.insert(
            "checkpoint_completion_target".to_string(),
            format!("{:.2}", optimal.checkpoint_completion_target),
        );
        out.insert(
            "random_page_cost".to_string(),
            format!("{:.2}", optimal.random_page_cost),
        );
        if let Some(eioc) = optimal.effective_io_concurrency {
            out.insert("effective_io_concurrency".to_string(), eioc.to_string());
        }
        out.insert("huge_pages".to_string(), optimal.huge_pages.to_string());

        if Self::manage_max_connections(cluster) {
            out.insert(
                "max_connections".to_string(),
                optimal.max_connections.to_string(),
            );
        }

        out
    }
}

fn parse_bool(v: Option<&str>) -> bool {
    matches!(
        v.map(|s| s.trim().to_lowercase()),
        Some(ref s) if s == "true" || s == "1" || s == "yes" || s == "on"
    )
}

fn infer_cpu_and_memory_from_cluster(cluster: &Value) -> Option<(u32, u64)> {
    // Best-effort parsing from common Kubernetes resource fields.
    // CNPG Cluster structure varies by version, so we search a few likely locations.
    let candidates = [
        "/spec/resources/requests",
        "/spec/resources/limits",
        "/spec/instancesResources/requests",
        "/spec/instancesResources/limits",
        "/spec/pods/resources/requests",
        "/spec/pods/resources/limits",
    ];

    for base in candidates {
        let mem = get_json_pointer(cluster, &format!("{}/memory", base)).and_then(|v| v.as_str());
        let cpu = get_json_pointer(cluster, &format!("{}/cpu", base)).and_then(|v| v.as_str());

        let mem_kb = mem.and_then(parse_k8s_memory_to_kb);
        let cpu_cores = cpu.and_then(parse_k8s_cpu_to_cores);

        if let (Some(cpu_cores), Some(mem_kb)) = (cpu_cores, mem_kb) {
            return Some((cpu_cores, mem_kb));
        }

        // If one is missing, keep searching other blocks.
    }

    None
}

fn get_json_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    root.pointer(pointer)
}

fn parse_k8s_memory_to_kb(s: &str) -> Option<u64> {
    // Supports: Ki, Mi, Gi, Ti and plain bytes.
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (num, unit) = split_num_unit(s);
    let value: f64 = num.parse().ok()?;
    let bytes = match unit {
        "Ki" => value * 1024.0,
        "Mi" => value * 1024.0 * 1024.0,
        "Gi" => value * 1024.0 * 1024.0 * 1024.0,
        "Ti" => value * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "K" => value * 1000.0,
        "M" => value * 1000.0 * 1000.0,
        "G" => value * 1000.0 * 1000.0 * 1000.0,
        "T" => value * 1000.0 * 1000.0 * 1000.0 * 1000.0,
        "" => value,
        _ => return None,
    };

    Some((bytes / 1024.0).round() as u64)
}

fn parse_k8s_cpu_to_cores(s: &str) -> Option<u32> {
    // Supports millicores: 500m, or plain cores: 2
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_suffix('m') {
        let milli: f64 = rest.parse().ok()?;
        let cores = (milli / 1000.0).ceil();
        return Some(cores.max(1.0) as u32);
    }

    let cores: f64 = s.parse().ok()?;
    Some(cores.ceil().max(1.0) as u32)
}

fn split_num_unit(s: &str) -> (&str, &str) {
    let mut idx = s.len();
    for (i, ch) in s.char_indices() {
        if !(ch.is_ascii_digit() || ch == '.') {
            idx = i;
            break;
        }
    }
    (&s[..idx], &s[idx..])
}

fn json_pointer_escape_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn ensure_object_ops(cluster: &Value, path: &str) -> Option<Value> {
    // If the object at JSON pointer `path` doesn't exist, return a JSONPatch op to add it as {}.
    if cluster.pointer(path).is_none() {
        return Some(serde_json::json!({"op":"add","path": path, "value": {}}));
    }
    None
}

fn build_tuning_patch(service: &VaanyzeOperatorService, cluster: &Value) -> Vec<Value> {
    let mut ops = Vec::new();

    // Ensure metadata.annotations exists so we can mark the cluster as managed.
    if let Some(op) = ensure_object_ops(cluster, "/metadata/annotations") {
        ops.push(op);
    }

    // Ensure spec.postgresql.parameters exists for parameter injection.
    if cluster.pointer("/spec/postgresql").is_none() {
        ops.push(serde_json::json!({"op":"add","path":"/spec/postgresql","value": {}}));
    }
    if let Some(op) = ensure_object_ops(cluster, "/spec/postgresql/parameters") {
        ops.push(op);
    }

    // Add managed marker.
    ops.push(serde_json::json!({
        "op":"add",
        "path": format!("/metadata/annotations/{}", json_pointer_escape_segment("vaanyze.io/managed")),
        "value": "true"
    }));

    let recommended = service.optimal_parameters(cluster);
    for (key, val) in recommended {
        if VaanyzeOperatorService::has_existing_parameter(cluster, &key) {
            continue;
        }
        ops.push(serde_json::json!({
            "op":"add",
            "path": format!("/spec/postgresql/parameters/{}", json_pointer_escape_segment(&key)),
            "value": val
        }));
    }

    ops
}

#[tonic::async_trait]
impl operator_v1::operator_server::Operator for VaanyzeOperatorService {
    async fn get_capabilities(
        &self,
        _request: Request<operator_v1::OperatorCapabilitiesRequest>,
    ) -> Result<Response<operator_v1::OperatorCapabilitiesResult>, Status> {
        use operator_v1::operator_capability::{rpc::Type as RpcType, Rpc};
        use operator_v1::OperatorCapability;

        let capabilities = vec![
            OperatorCapability {
                r#type: Some(operator_v1::operator_capability::Type::Rpc(Rpc {
                    r#type: RpcType::MutateCluster as i32,
                })),
            },
            OperatorCapability {
                r#type: Some(operator_v1::operator_capability::Type::Rpc(Rpc {
                    r#type: RpcType::SetStatusInCluster as i32,
                })),
            },
        ];

        Ok(Response::new(operator_v1::OperatorCapabilitiesResult {
            capabilities,
        }))
    }

    async fn validate_cluster_create(
        &self,
        _request: Request<operator_v1::OperatorValidateClusterCreateRequest>,
    ) -> Result<Response<operator_v1::OperatorValidateClusterCreateResult>, Status> {
        Ok(Response::new(
            operator_v1::OperatorValidateClusterCreateResult {
                validation_errors: vec![],
            },
        ))
    }

    async fn validate_cluster_change(
        &self,
        _request: Request<operator_v1::OperatorValidateClusterChangeRequest>,
    ) -> Result<Response<operator_v1::OperatorValidateClusterChangeResult>, Status> {
        Ok(Response::new(
            operator_v1::OperatorValidateClusterChangeResult {
                validation_errors: vec![],
            },
        ))
    }

    async fn mutate_cluster(
        &self,
        request: Request<operator_v1::OperatorMutateClusterRequest>,
    ) -> Result<Response<operator_v1::OperatorMutateClusterResult>, Status> {
        let req = request.into_inner();

        let cluster: Value = serde_json::from_slice(&req.definition).map_err(|e| {
            Status::invalid_argument(format!("invalid cluster definition JSON: {}", e))
        })?;

        if !Self::is_opted_in(&cluster) {
            debug!("CNPG tuning: cluster not opted-in (vaanyze.io/enable-tuning != true)");
            return Ok(Response::new(operator_v1::OperatorMutateClusterResult {
                json_patch: vec![],
            }));
        }

        let ops = build_tuning_patch(self, &cluster);
        let patch_bytes = serde_json::to_vec(&ops)
            .map_err(|e| Status::internal(format!("failed to encode json_patch: {}", e)))?;

        // Update runtime state (best-effort)
        {
            let mut state = self.state.lock().await;
            state.update_last_run();
        }

        Ok(Response::new(operator_v1::OperatorMutateClusterResult {
            json_patch: patch_bytes,
        }))
    }

    async fn set_status_in_cluster(
        &self,
        request: Request<operator_v1::SetStatusInClusterRequest>,
    ) -> Result<Response<operator_v1::SetStatusInClusterResponse>, Status> {
        let req = request.into_inner();
        let cluster: Value = serde_json::from_slice(&req.cluster)
            .map_err(|e| Status::invalid_argument(format!("invalid cluster JSON: {}", e)))?;

        let workload = Self::workload(&cluster);
        let opted_in = Self::is_opted_in(&cluster);

        let last_run = {
            let state = self.state.lock().await;
            state.last_run
        };

        let status = serde_json::json!({
            "plugin": "vaanyze",
            "version": env!("CARGO_PKG_VERSION"),
            "opted_in": opted_in,
            "workload": format!("{:?}", workload),
            "last_run": last_run.to_rfc3339(),
            "grpc_ready": self.grpc_ready.load(Ordering::Relaxed),
        });

        let bytes = serde_json::to_vec(&status)
            .map_err(|e| Status::internal(format!("failed to encode json_status: {}", e)))?;

        Ok(Response::new(operator_v1::SetStatusInClusterResponse {
            json_status: bytes,
        }))
    }

    async fn deregister(
        &self,
        _request: Request<operator_v1::DeregisterRequest>,
    ) -> Result<Response<operator_v1::DeregisterResponse>, Status> {
        Ok(Response::new(operator_v1::DeregisterResponse {}))
    }
}

pub async fn start_grpc_server(
    port: u16,
    config: PluginConfig,
    state: Arc<Mutex<AgentState>>,
) -> anyhow::Result<()> {
    use axum::{routing::get, Router};
    use std::net::SocketAddr;
    use tonic::transport::Server;

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let grpc_ready = Arc::new(AtomicBool::new(false));
    let service = VaanyzeOperatorService::new(config.clone(), state.clone(), grpc_ready.clone());

    info!(
        "Vaanyze CNPG Plugin v{} starting on {}",
        env!("CARGO_PKG_VERSION"),
        addr
    );
    // Start health check HTTP server on port+1 for Kubernetes probes
    let health_addr: SocketAddr = format!("0.0.0.0:{}", port + 1).parse()?;
    let health_service = service.clone();

    let health_router = Router::new()
        .route("/health", get(move || async move { "OK" }))
        .route(
            "/ready",
            get(move || async move {
                if health_service.grpc_ready.load(Ordering::Relaxed) {
                    "Ready"
                } else {
                    "Not Ready"
                }
            }),
        );

    info!("Health check endpoints on http://{}", health_addr);
    info!("  - GET /health - Liveness probe");
    info!("  - GET /ready - Readiness probe");

    // Spawn health check server
    let health_server = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(health_addr)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to bind health server: {}", e))?;

        axum::serve(listener, health_router)
            .await
            .map_err(|e| anyhow::anyhow!("Health server error: {}", e))
    });

    // Start real CNPG-I gRPC server.
    // Mark gRPC ready immediately before serving.
    grpc_ready.store(true, Ordering::Relaxed);

    info!("gRPC server listening on {}", addr);
    info!("Press Ctrl+C to stop");

    let grpc_server = tokio::spawn(async move {
        Server::builder()
            .add_service(operator_v1::operator_server::OperatorServer::new(service))
            .serve_with_shutdown(addr, async {
                tokio::signal::ctrl_c().await.ok();
            })
            .await
            .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))
    });

    tokio::select! {
        result = health_server => {
            match result {
                Ok(Ok(())) => info!("Health server stopped"),
                Ok(Err(e)) => error!("Health server error: {}", e),
                Err(e) => error!("Health server task error: {}", e),
            }
        }
        result = grpc_server => {
            match result {
                Ok(Ok(())) => info!("gRPC server stopped"),
                Ok(Err(e)) => error!("gRPC server error: {}", e),
                Err(e) => error!("gRPC server task error: {}", e),
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cnpg_plugin::cnpgi::operator_v1;
    use crate::cnpg_plugin::cnpgi::operator_v1::operator_server::Operator;

    fn make_service() -> VaanyzeOperatorService {
        let config = PluginConfig::default();
        let state = Arc::new(Mutex::new(AgentState::default()));
        let grpc_ready = Arc::new(AtomicBool::new(true));
        VaanyzeOperatorService::new(config, state, grpc_ready)
    }

    #[tokio::test]
    async fn mutate_cluster_noop_when_not_opted_in() {
        let service = make_service();

        let cluster = serde_json::json!({
            "apiVersion": "postgresql.cnpg.io/v1",
            "kind": "Cluster",
            "metadata": {"name": "demo", "annotations": {"cnpg.io/plugins": "vaanyze"}},
            "spec": {"instances": 1, "storage": {"size": "1Gi"}}
        });

        let req = operator_v1::OperatorMutateClusterRequest {
            definition: serde_json::to_vec(&cluster).unwrap(),
        };

        let resp = service
            .mutate_cluster(Request::new(req))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.json_patch.is_empty());
    }

    #[tokio::test]
    async fn mutate_cluster_generates_parameter_patch_when_opted_in() {
        let service = make_service();

        // Note: include a user-provided shared_buffers to ensure we do not overwrite it.
        let cluster = serde_json::json!({
            "apiVersion": "postgresql.cnpg.io/v1",
            "kind": "Cluster",
            "metadata": {
                "name": "demo",
                "annotations": {
                    "cnpg.io/plugins": "vaanyze",
                    "vaanyze.io/enable-tuning": "true",
                    "vaanyze.io/workload": "mixed"
                }
            },
            "spec": {
                "instances": 1,
                "storage": {"size": "1Gi"},
                "postgresql": {
                    "parameters": {
                        "shared_buffers": "256MB"
                    }
                }
            }
        });

        let req = operator_v1::OperatorMutateClusterRequest {
            definition: serde_json::to_vec(&cluster).unwrap(),
        };

        let resp = service
            .mutate_cluster(Request::new(req))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.json_patch.is_empty());

        let ops: Vec<Value> = serde_json::from_slice(&resp.json_patch).unwrap();
        let paths: Vec<String> = ops
            .iter()
            .filter_map(|v| {
                v.get("path")
                    .and_then(|p| p.as_str())
                    .map(|s| s.to_string())
            })
            .collect();

        // Marker annotation should be added.
        assert!(paths
            .iter()
            .any(|p| p == "/metadata/annotations/vaanyze.io~1managed"));

        // Should add some parameters we didn't already specify.
        assert!(paths
            .iter()
            .any(|p| p == "/spec/postgresql/parameters/effective_cache_size"));

        // Should NOT attempt to overwrite shared_buffers.
        assert!(!paths
            .iter()
            .any(|p| p == "/spec/postgresql/parameters/shared_buffers"));
    }
}
