// Stats and health endpoints

use crate::http::{
    error::ApiResult,
    models::{ApiResponse, HealthStatus, PrometheusMetrics, MetricLine, StatsSummary},
    AppState,
};
use axum::{extract::State, Json};
use std::time::{SystemTime, UNIX_EPOCH};

lazy_static::lazy_static! {
    static ref START_TIME: SystemTime = SystemTime::now();
}

/// GET /api/v1/health - Health check
pub async fn health(State(state): State<AppState>) -> ApiResult<Json<ApiResponse<HealthStatus>>> {
    let uptime = START_TIME
        .elapsed()
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let status = HealthStatus {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: uptime,
        ebpf_enabled: state.ebpf_manager.is_some(),
    };

    Ok(Json(ApiResponse::ok(status)))
}

/// GET /api/v1/stats/summary - Overall system statistics
pub async fn summary(State(state): State<AppState>) -> ApiResult<Json<ApiResponse<StatsSummary>>> {
    // Get lock contention count if eBPF enabled
    let high_contention_count = state.ebpf_manager
        .as_ref()
        .map(|mgr| mgr.get_high_contention().len())
        .unwrap_or(0);

    // Placeholder stats - in production these would come from persistence
    let stats = StatsSummary {
        total_queries_analyzed: 0,
        total_recommendations: 0,
        total_changes_applied: 0,
        high_contention_tables: high_contention_count,
        avg_query_time_ms: 0.0,
        system_health: "healthy".to_string(),
    };

    Ok(Json(ApiResponse::ok(stats)))
}

/// GET /api/v1/stats/metrics - Prometheus-compatible metrics
pub async fn metrics(State(state): State<AppState>) -> ApiResult<String> {
    let mut lines = Vec::new();

    // Uptime metric
    let uptime = START_TIME.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    lines.push(format!("# HELP db_optimizer_uptime_seconds Uptime in seconds"));
    lines.push(format!("# TYPE db_optimizer_uptime_seconds counter"));
    lines.push(format!("db_optimizer_uptime_seconds {}", uptime));

    // eBPF status
    let ebpf_enabled = if state.ebpf_manager.is_some() { 1 } else { 0 };
    lines.push(format!("# HELP db_optimizer_ebpf_enabled eBPF status (1=enabled, 0=disabled)"));
    lines.push(format!("# TYPE db_optimizer_ebpf_enabled gauge"));
    lines.push(format!("db_optimizer_ebpf_enabled {}", ebpf_enabled));

    // Lock contention metrics if available
    if let Some(ref manager) = state.ebpf_manager {
        let summary = manager.get_summary();
        
        lines.push(format!("# HELP db_optimizer_tables_tracked Total tables tracked"));
        lines.push(format!("# TYPE db_optimizer_tables_tracked gauge"));
        lines.push(format!("db_optimizer_tables_tracked {}", summary.total_tables_tracked));

        lines.push(format!("# HELP db_optimizer_high_contention_tables Tables with high contention"));
        lines.push(format!("# TYPE db_optimizer_high_contention_tables gauge"));
        lines.push(format!("db_optimizer_high_contention_tables {}", summary.high_contention_tables));

        lines.push(format!("# HELP db_optimizer_total_lock_events Total lock events processed"));
        lines.push(format!("# TYPE db_optimizer_total_lock_events counter"));
        lines.push(format!("db_optimizer_total_lock_events {}", summary.total_lock_events));
    }

    Ok(lines.join("\n"))
}
