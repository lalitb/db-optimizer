// API request/response models

use crate::ebpf::{LockContentionMetrics, LockContentionSummary};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Generic API response wrapper
#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: T,
    pub timestamp: DateTime<Utc>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data,
            timestamp: Utc::now(),
        }
    }
}

/// Health check status
#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub ebpf_enabled: bool,
}

/// Lock contention response
#[derive(Debug, Serialize)]
pub struct LockContentionResponse {
    pub high_contention_tables: Vec<LockContentionMetrics>,
    pub summary: LockContentionSummary,
}

/// Query analysis request
#[derive(Debug, Deserialize)]
pub struct AnalyzeQueryRequest {
    pub query: String,
    #[serde(default)]
    pub explain: bool,
}

/// Query rewrite request
#[derive(Debug, Deserialize)]
pub struct RewriteQueryRequest {
    pub query: String,
    #[serde(default)]
    pub rules: Vec<String>,
}

/// Query rewrite response
#[derive(Debug, Serialize)]
pub struct RewriteQueryResponse {
    pub original: String,
    pub rewritten: String,
    pub rules_applied: Vec<String>,
    pub estimated_improvement_pct: Option<f64>,
    pub explanation: Vec<String>,
}

/// Pagination query parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    100
}

/// Stats summary response
#[derive(Debug, Serialize)]
pub struct StatsSummary {
    pub total_queries_analyzed: u64,
    pub total_recommendations: u64,
    pub total_changes_applied: u64,
    pub high_contention_tables: usize,
    pub avg_query_time_ms: f64,
    pub system_health: String,
}

/// Recommendation approval request
#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Prometheus metrics format
#[derive(Debug, Serialize)]
pub struct PrometheusMetrics {
    pub metrics: Vec<MetricLine>,
}

#[derive(Debug, Serialize)]
pub struct MetricLine {
    pub name: String,
    pub value: f64,
    pub labels: Vec<(String, String)>,
}
