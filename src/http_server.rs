// src/http_server.rs - HTTP/JSON API server for db-optimizer

use anyhow::Result;
use axum::{
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use serde::Serialize;

/// Stats summary response
#[derive(Debug, Serialize)]
pub struct StatsSummary {
    pub status: String,
    pub service: String,
    pub version: String,
    pub message: String,
}

/// Create and configure the HTTP server
pub async fn create_server(port: u16) -> Result<()> {
    let app = Router::new()
        // Health check
        .route("/health", get(health_check))
        .route("/", get(root_handler));

    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("Starting HTTP server on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}

/// GET / - Root handler
async fn root_handler() -> Json<StatsSummary> {
    Json(StatsSummary {
        status: "ok".to_string(),
        service: "db-optimizer".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        message: "HTTP API server running. Full REST API endpoints coming soon. Use CLI for now.".to_string(),
    })
}

/// GET /health - Health check endpoint
async fn health_check() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "healthy",
            "service": "db-optimizer",
            "version": env!("CARGO_PKG_VERSION"),
        }))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_check() {
        let (status, response) = health_check().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response.0["status"], "healthy");
    }
}
