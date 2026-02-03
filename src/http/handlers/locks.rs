// Lock contention endpoints

use crate::http::{
    error::{ApiError, ApiResult},
    models::{
        ApiResponse, LockContentionEventDto, LockContentionResponse, LwLockEventDto,
        PaginationParams, RowLockEventDto,
    },
    AppState,
};
use axum::{
    extract::{Path, Query, State},
    Json,
};

/// GET /api/v1/locks/contention - High contention tables
pub async fn contention(
    State(state): State<AppState>,
) -> ApiResult<Json<ApiResponse<LockContentionResponse>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let high_contention = manager.get_high_contention();
    let summary = manager.get_summary();

    Ok(Json(ApiResponse::ok(LockContentionResponse {
        high_contention_tables: high_contention,
        summary,
    })))
}

/// GET /api/v1/locks/summary - Lock statistics summary
pub async fn summary(
    State(state): State<AppState>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let summary = manager.get_summary();
    Ok(Json(ApiResponse::ok(serde_json::json!(summary))))
}

/// GET /api/v1/locks/events - Recent lock events (paginated)
pub async fn events(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let all_metrics = manager.get_lock_metrics();

    // Apply pagination
    let total = all_metrics.len();
    let paginated: Vec<_> = all_metrics
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "events": paginated,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    }))))
}

/// GET /api/v1/locks/table/:oid - Table-specific lock breakdown
pub async fn table_locks(
    State(state): State<AppState>,
    Path(oid): Path<u32>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let metrics = manager.get_lock_metrics();
    let table_metrics: Vec<_> = metrics.into_iter().filter(|m| m.table_oid == oid).collect();

    if table_metrics.is_empty() {
        return Err(ApiError::NotFound(format!(
            "No lock data for table OID {}",
            oid
        )));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "table_oid": oid,
        "metrics": table_metrics,
    }))))
}

/// GET /api/v1/locks/lwlocks/events - Recent LWLock events
pub async fn lwlock_events(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let tracer = manager.lock_tracer();
    let events = tracer
        .recent_lwlock_events()
        .into_iter()
        .map(LwLockEventDto::from)
        .collect::<Vec<_>>();

    let total = events.len();
    let paginated: Vec<_> = events
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "events": paginated,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    }))))
}

/// GET /api/v1/locks/rows/events - Recent row lock events
pub async fn row_lock_events(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let tracer = manager.lock_tracer();
    let events = tracer
        .recent_row_lock_events()
        .into_iter()
        .map(RowLockEventDto::from)
        .collect::<Vec<_>>();

    let total = events.len();
    let paginated: Vec<_> = events
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "events": paginated,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    }))))
}

/// GET /api/v1/locks/contention/events - Recent lock contention events (blocking/waiting PIDs)
pub async fn contention_events(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let manager = state
        .ebpf_manager
        .ok_or_else(|| ApiError::ServiceUnavailable("eBPF not enabled".to_string()))?;

    let tracer = manager.lock_tracer();
    let events = tracer
        .recent_contention_events()
        .into_iter()
        .map(LockContentionEventDto::from)
        .collect::<Vec<_>>();

    let total = events.len();
    let paginated: Vec<_> = events
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .collect();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "events": paginated,
        "total": total,
        "limit": params.limit,
        "offset": params.offset,
    }))))
}
