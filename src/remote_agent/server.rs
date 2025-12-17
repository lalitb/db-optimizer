use super::models::{AgentMetrics, AgentStatus};
use crate::ebpf::manager::EbpfManager;
use axum::{extract::State, routing::get, Json, Router};
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AgentServerState {
    pub ebpf_manager: Arc<RwLock<EbpfManager>>,
    pub start_time: chrono::DateTime<Utc>,
}

pub async fn run_agent_server(manager: Arc<RwLock<EbpfManager>>, port: u16) {
    let state = Arc::new(AgentServerState {
        ebpf_manager: manager,
        start_time: Utc::now(),
    });

    let app = Router::new()
        .route("/v1/health", get(health_check))
        .route("/v1/metrics", get(get_metrics))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Agent server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health_check(State(state): State<Arc<AgentServerState>>) -> Json<AgentStatus> {
    let manager = state.ebpf_manager.read().await;

    Json(AgentStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: (Utc::now() - state.start_time).num_seconds() as u64,
        ebpf_enabled: manager.is_running(),
        attached_pid: None, // In a real impl, we'd track which PID we attached to
    })
}

async fn get_metrics(State(state): State<Arc<AgentServerState>>) -> Json<AgentMetrics> {
    let manager = state.ebpf_manager.read().await;

    // Get lock summary
    let lock_summary = if manager.is_running() {
        Some(manager.get_summary())
    } else {
        None
    };

    Json(AgentMetrics {
        timestamp: Utc::now(),
        lock_summary,
    })
}
