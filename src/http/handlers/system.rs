//! HTTP handlers for system status and health checks.
//!
//! Provides endpoints for:
//! - System health with operation mode
//! - Environment detection
//! - Kill switch control
//! - Vaanyze-managed indexes list

use crate::analyzer::environment::{detect_environment, EnvironmentInfo};
use crate::auto_apply::recovery::{
    cleanup_orphaned_candidates, check_invalid_indexes, list_vaanyze_indexes, VaanyzeIndex,
};
use crate::auto_apply::OperationGuard;
use crate::http::{
    error::{ApiError, ApiResult},
    models::ApiResponse,
};
use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for system handlers
pub struct SystemHandlerState {
    pub db_pool: PgPool,
    /// Global kill switch - when true, all apply/rollback operations are disabled
    pub kill_switch: Arc<AtomicBool>,
    /// Current operation mode (observe_only, safe_auto, aggressive)
    pub operation_mode: Arc<RwLock<String>>,
    /// Read-only mode flag
    pub read_only: Arc<AtomicBool>,
    /// Start time for uptime calculation
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Centralized operation guard for mode enforcement (optional for backward compat)
    pub operation_guard: Option<Arc<OperationGuard>>,
}

/// System health response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub operation_mode: String,
    pub read_only: bool,
    pub kill_switch_active: bool,
    pub database_connected: bool,
    pub uptime_secs: i64,
    pub warnings: Vec<String>,
    /// Operation guard status showing effective mode and enforcement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_guard: Option<crate::auto_apply::OperationGuardStatus>,
}

/// GET /api/v1/health - System health check with mode info
pub async fn health(
    State(state): State<Arc<SystemHandlerState>>,
) -> ApiResult<Json<ApiResponse<HealthResponse>>> {
    let mut warnings = Vec::new();

    // Check database connectivity
    let db_connected = sqlx::query("SELECT 1")
        .fetch_optional(&state.db_pool)
        .await
        .is_ok();

    if !db_connected {
        warnings.push("Database connection unavailable".to_string());
    }

    let read_only = state.read_only.load(Ordering::SeqCst);
    let kill_switch = state.kill_switch.load(Ordering::SeqCst);
    let operation_mode = state.operation_mode.read().await.clone();

    // Get operation guard status if available
    let guard_status = if let Some(guard) = &state.operation_guard {
        let status = guard.get_status().await;
        if status.is_forced {
            warnings.push(format!(
                "Operations forced to {} mode: {}",
                status.effective_mode,
                status.force_reason.as_deref().unwrap_or("unknown")
            ));
        }
        Some(status)
    } else {
        None
    };

    if kill_switch {
        warnings.push("Kill switch is active - all apply/rollback operations disabled".to_string());
    }

    if read_only {
        warnings.push("System is in read-only mode".to_string());
    }

    let status = if db_connected && !kill_switch { "healthy" } else { "degraded" };

    let uptime = (chrono::Utc::now() - state.started_at).num_seconds();

    Ok(Json(ApiResponse::ok(HealthResponse {
        status: status.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        operation_mode,
        read_only,
        kill_switch_active: kill_switch,
        database_connected: db_connected,
        uptime_secs: uptime,
        warnings,
        operation_guard: guard_status,
    })))
}

/// GET /api/v1/system/environment - Detect database environment
pub async fn environment(
    State(state): State<Arc<SystemHandlerState>>,
) -> ApiResult<Json<ApiResponse<EnvironmentInfo>>> {
    let env_info = detect_environment(&state.db_pool).await.map_err(|e| {
        ApiError::Internal(format!("Failed to detect environment: {}", e))
    })?;

    Ok(Json(ApiResponse::ok(env_info)))
}

/// Kill switch request
#[derive(Debug, Deserialize)]
pub struct KillSwitchRequest {
    /// Enable or disable the kill switch
    pub enabled: bool,
    /// Optional reason for the change
    #[serde(default)]
    pub reason: Option<String>,
}

/// Kill switch response
#[derive(Debug, Serialize)]
pub struct KillSwitchResponse {
    pub kill_switch_active: bool,
    pub message: String,
    pub previous_state: bool,
}

/// POST /api/v1/system/kill-switch - Enable/disable emergency kill switch
pub async fn kill_switch(
    State(state): State<Arc<SystemHandlerState>>,
    Json(req): Json<KillSwitchRequest>,
) -> ApiResult<Json<ApiResponse<KillSwitchResponse>>> {
    let previous = state.kill_switch.swap(req.enabled, Ordering::SeqCst);

    // Also update the operation guard if available
    if let Some(guard) = &state.operation_guard {
        guard.set_kill_switch(req.enabled);
    }

    let message = if req.enabled {
        let reason = req.reason.unwrap_or_else(|| "Manual activation".to_string());
        tracing::warn!(
            reason = %reason,
            "Kill switch ACTIVATED - all apply/rollback operations disabled"
        );
        format!("Kill switch activated: {}", reason)
    } else {
        tracing::info!("Kill switch deactivated - operations resumed");
        "Kill switch deactivated - operations resumed".to_string()
    };

    Ok(Json(ApiResponse::ok(KillSwitchResponse {
        kill_switch_active: req.enabled,
        message,
        previous_state: previous,
    })))
}

/// Managed indexes response
#[derive(Debug, Serialize)]
pub struct ManagedIndexesResponse {
    pub indexes: Vec<VaanyzeIndex>,
    pub total: usize,
    pub total_idx_scan: i64,
}

/// GET /api/v1/system/indexes - List Vaanyze-managed indexes
pub async fn managed_indexes(
    State(state): State<Arc<SystemHandlerState>>,
) -> ApiResult<Json<ApiResponse<ManagedIndexesResponse>>> {
    let indexes = list_vaanyze_indexes(&state.db_pool).await.map_err(|e| {
        ApiError::Internal(format!("Failed to list indexes: {}", e))
    })?;

    let total = indexes.len();
    let total_idx_scan: i64 = indexes.iter().map(|i| i.idx_scan).sum();

    Ok(Json(ApiResponse::ok(ManagedIndexesResponse {
        indexes,
        total,
        total_idx_scan,
    })))
}

/// Recovery response
#[derive(Debug, Serialize)]
pub struct RecoveryResponse {
    pub orphaned_candidates_cleaned: Vec<String>,
    pub invalid_indexes_found: Vec<String>,
    pub message: String,
}

/// POST /api/v1/system/recover - Clean up orphaned indexes and check consistency
pub async fn recover(
    State(state): State<Arc<SystemHandlerState>>,
) -> ApiResult<Json<ApiResponse<RecoveryResponse>>> {
    // Check kill switch
    if state.kill_switch.load(Ordering::SeqCst) {
        return Err(ApiError::BadRequest(
            "Kill switch is active - recovery disabled".to_string(),
        ));
    }

    // Check read-only
    if state.read_only.load(Ordering::SeqCst) {
        return Err(ApiError::BadRequest(
            "System is in read-only mode - recovery disabled".to_string(),
        ));
    }

    let cleaned = cleanup_orphaned_candidates(&state.db_pool).await.map_err(|e| {
        ApiError::Internal(format!("Failed to cleanup candidates: {}", e))
    })?;

    let invalid = check_invalid_indexes(&state.db_pool).await.map_err(|e| {
        ApiError::Internal(format!("Failed to check invalid indexes: {}", e))
    })?;

    let message = match (cleaned.len(), invalid.len()) {
        (0, 0) => "No orphaned or invalid indexes found".to_string(),
        (c, 0) => format!("Cleaned up {} orphaned candidate index(es)", c),
        (0, i) => format!("Found {} invalid index(es) - manual review recommended", i),
        (c, i) => format!(
            "Cleaned up {} orphaned candidate(s), found {} invalid index(es)",
            c, i
        ),
    };

    Ok(Json(ApiResponse::ok(RecoveryResponse {
        orphaned_candidates_cleaned: cleaned,
        invalid_indexes_found: invalid,
        message,
    })))
}

/// Set operation mode request
#[derive(Debug, Deserialize)]
pub struct SetModeRequest {
    pub mode: String,
}

/// POST /api/v1/system/mode - Set operation mode
pub async fn set_mode(
    State(state): State<Arc<SystemHandlerState>>,
    Json(req): Json<SetModeRequest>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let valid_modes = ["observe_only", "safe_auto", "aggressive"];

    if !valid_modes.contains(&req.mode.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "Invalid mode '{}'. Valid modes: {:?}",
            req.mode, valid_modes
        )));
    }

    let previous = {
        let mut mode = state.operation_mode.write().await;
        let prev = mode.clone();
        *mode = req.mode.clone();
        prev
    };

    // Update read_only flag based on mode
    state.read_only.store(
        req.mode == "observe_only",
        Ordering::SeqCst,
    );

    // Also update the operation guard if available
    if let Some(guard) = &state.operation_guard {
        let mode_str = match req.mode.as_str() {
            "observe_only" => "observe_only",
            "safe_auto" => "safe_auto",
            "aggressive" => "aggressive",
            _ => "observe_only",
        };
        guard.set_mode(mode_str).await;
    }

    tracing::info!(
        previous_mode = %previous,
        new_mode = %req.mode,
        "Operation mode changed"
    );

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "previous_mode": previous,
        "current_mode": req.mode,
        "read_only": req.mode == "observe_only",
    }))))
}
