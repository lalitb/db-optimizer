// HTTP API module - Production-ready REST API for db-optimizer
// Provides comprehensive endpoints for monitoring, analysis, and management

pub mod error;
pub mod handlers;
pub mod models;

use crate::config::HttpServerConfig;
use crate::ebpf::EbpfManager;
use crate::persistence::PersistenceStore;
use axum::{
    extract::State,
    http::{header, Request, StatusCode},
    middleware::{from_fn_with_state, Next},
    routing::{get, post},
    response::IntoResponse,
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::http::error::ApiErrorResponse;

pub use error::{ApiError, ApiResult};
pub use models::{ApiResponse, HealthStatus};

async fn require_bearer_auth(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    let expected = match state.config.auth_token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return next.run(req).await,
    };

    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);

    let alt = req
        .headers()
        .get("x-dbo-token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim);

    let provided = bearer.or(alt);

    if provided == Some(expected) {
        return next.run(req).await;
    }

    let body = axum::Json(ApiErrorResponse {
        error: "Unauthorized".to_string(),
        code: "UNAUTHORIZED".to_string(),
        details: Some("Provide Authorization: Bearer <token>".to_string()),
    });

    (StatusCode::UNAUTHORIZED, body).into_response()
}

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub persistence: Arc<RwLock<Box<dyn PersistenceStore>>>,
    pub ebpf_manager: Option<Arc<EbpfManager>>,
    pub config: Arc<HttpServerConfig>,
    pub db_pool: Option<sqlx::PgPool>,
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
        .route("/queries/:query_id", get(handlers::queries::query_detail))
        .route("/queries/analyze", post(handlers::queries::analyze_query))
        .route("/queries/rewrite", post(handlers::queries::rewrite_query))
        
        // Recommendation Endpoints
        .route("/recommendations", get(handlers::recommendations::list))
        .route("/recommendations/:id", get(handlers::recommendations::get_by_id))
        .route("/recommendations/:id/approve", post(handlers::recommendations::approve))
        .route("/recommendations/:id/reject", post(handlers::recommendations::reject));
    
    let api_v1 = api_v1.with_state(state.clone());

    // Optional auth: if a token is configured, require it for all /api/v1 routes.
    let api_v1 = if state.config.auth_token.as_deref().unwrap_or("").is_empty() {
        api_v1
    } else {
        api_v1.layer(from_fn_with_state(state.clone(), require_bearer_auth))
    };
    
    // Add config tuning routes if database pool is available
    let api_v1 = if let Some(pool) = &state.db_pool {
        let config_state = Arc::new(handlers::config_tuning::ConfigTuningState {
            pool: pool.clone(),
        });
        
        let config_router = Router::new()
            .route("/tune", post(handlers::config_tuning::tune_config))
            .route("/validate", get(handlers::config_tuning::validate_config))
            .route("/detect-workload", get(handlers::config_tuning::detect_workload))
            .with_state(config_state);
        
        api_v1.nest("/config", config_router)
    } else {
        api_v1
    };

    let router = Router::new()
        .route("/", get(handlers::root))
        .nest("/api/v1", api_v1)
        .layer(TraceLayer::new_for_http());

    if state.config.enable_cors {
        router.layer(CorsLayer::permissive())
    } else {
        router
    }
}

/// Start the HTTP server
pub async fn serve(
    config: HttpServerConfig,
    persistence: Arc<RwLock<Box<dyn PersistenceStore>>>,
    ebpf_manager: Option<Arc<EbpfManager>>,
    db_pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.bind_address, config.port);
    
    let state = AppState {
        persistence,
        ebpf_manager,
        config: Arc::new(config),
        db_pool,
    };

    let app = create_router(state);
    
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP server listening on {}", addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}
