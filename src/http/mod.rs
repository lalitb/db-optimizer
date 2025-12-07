// HTTP API module - Production-ready REST API for db-optimizer
// Provides comprehensive endpoints for monitoring, analysis, and management

pub mod error;
pub mod handlers;
pub mod models;

use crate::config::HttpServerConfig;
use crate::ebpf::EbpfManager;
use crate::fleet::FleetManager;
use crate::persistence::PersistenceStore;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::info;

pub use error::{ApiError, ApiResult};
pub use models::{ApiResponse, HealthStatus};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub persistence: Arc<RwLock<Box<dyn PersistenceStore>>>,
    pub ebpf_manager: Option<Arc<EbpfManager>>,
    pub config: Arc<HttpServerConfig>,
    pub db_pool: Option<sqlx::PgPool>,
    pub fleet_manager: Option<Arc<FleetManager>>,
}

/// Create and configure the HTTP router
pub fn create_router(state: AppState) -> Router {
    let api_v1 = Router::new()
        // Health & Stats
        .route("/health", get(handlers::stats::health))
        .route("/stats/summary", get(handlers::stats::summary))
        .route("/stats/metrics", get(handlers::stats::metrics))
        // Lock Endpoints
        .route("/locks/contention", get(handlers::locks::contention))
        .route("/locks/summary", get(handlers::locks::summary))
        .route("/locks/events", get(handlers::locks::events))
        .route("/locks/table/:oid", get(handlers::locks::table_locks))
        // Query Endpoints
        .route("/queries/slow", get(handlers::queries::slow_queries))
        .route("/queries/analyze", post(handlers::queries::analyze_query))
        .route("/queries/rewrite", post(handlers::queries::rewrite_query))
        // Fleet Management Endpoints
        .route(
            "/fleet",
            get(handlers::fleet::list_instances).post(handlers::fleet::register_instance),
        )
        .route("/fleet/summary", get(handlers::fleet::get_summary))
        .route(
            "/fleet/:id",
            get(handlers::fleet::get_instance).delete(handlers::fleet::remove_instance),
        )
        .route("/fleet/:id/check", post(handlers::fleet::check_instance))
        // Recommendations
        .route("/recommendations", get(handlers::recommendations::list))
        .route(
            "/recommendations/:id/approve",
            post(handlers::recommendations::approve),
        );

    let api_v1 = api_v1.with_state(state.clone());

    // Add config tuning routes if database pool is available
    let api_v1 = if let Some(pool) = &state.db_pool {
        let config_state =
            Arc::new(handlers::config_tuning::ConfigTuningState { pool: pool.clone() });

        let config_router = Router::new()
            .route("/tune", post(handlers::config_tuning::tune_config))
            .route("/validate", get(handlers::config_tuning::validate_config))
            .route(
                "/detect-workload",
                get(handlers::config_tuning::detect_workload),
            )
            .with_state(config_state);

        api_v1.nest("/config", config_router)
    } else {
        api_v1
    };

    Router::new()
        .nest("/api/v1", api_v1)
        .nest_service("/", ServeDir::new("landing-page"))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the HTTP server
pub async fn serve(
    config: HttpServerConfig,
    persistence: Arc<RwLock<Box<dyn PersistenceStore>>>,
    ebpf_manager: Option<Arc<EbpfManager>>,
    db_pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.bind_address, config.port);

    let fleet_manager = Arc::new(FleetManager::new());

    // Start background fleet monitoring task
    let fleet_monitor = fleet_manager.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            tracing::info!("Running scheduled fleet health check...");
            fleet_monitor.check_all_instances().await;
        }
    });

    let state = AppState {
        persistence,
        ebpf_manager,
        config: Arc::new(config),
        db_pool,
        fleet_manager: Some(fleet_manager),
    };

    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
