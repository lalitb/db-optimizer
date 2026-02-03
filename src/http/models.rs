// API request/response models

use crate::ebpf::events::LockContentionEventRaw;
use crate::ebpf::{LockContentionMetrics, LockContentionSummary};
use chrono::{DateTime, TimeZone, Utc};
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

/// Recent lock contention event
#[derive(Debug, Serialize)]
pub struct LockContentionEventDto {
    pub timestamp: DateTime<Utc>,
    pub blocking_pid: u32,
    pub waiting_pid: u32,
    pub blocking_query_hash: u64,
    pub waiting_query_hash: u64,
    pub wait_duration_ns: u64,
    pub lock_type: u8,
    pub lock_mode: u8,
    pub severity: String,
}

impl From<LockContentionEventRaw> for LockContentionEventDto {
    fn from(raw: LockContentionEventRaw) -> Self {
        let severity = if raw.wait_duration_ns > 100_000_000 {
            "critical"
        } else if raw.wait_duration_ns > 20_000_000 {
            "high"
        } else if raw.wait_duration_ns > 5_000_000 {
            "medium"
        } else {
            "low"
        };

        Self {
            timestamp: raw.timestamp(),
            blocking_pid: raw.blocking_pid,
            waiting_pid: raw.waiting_pid,
            blocking_query_hash: raw.blocking_query_hash,
            waiting_query_hash: raw.waiting_query_hash,
            wait_duration_ns: raw.wait_duration_ns,
            lock_type: raw.lock_type,
            lock_mode: raw.lock_mode,
            severity: severity.to_string(),
        }
    }
}

/// LWLock event DTO
#[derive(Debug, Serialize)]
pub struct LwLockEventDto {
    pub timestamp: DateTime<Utc>,
    pub pid: u32,
    pub lock_id: u32,
    pub mode: u32,
    pub wait_duration_ns: u64,
}

impl From<crate::ebpf::events::LwLockEventRaw> for LwLockEventDto {
    fn from(raw: crate::ebpf::events::LwLockEventRaw) -> Self {
        Self {
            timestamp: Utc
                .timestamp_opt(
                    (raw.timestamp_ns / 1_000_000_000) as i64,
                    (raw.timestamp_ns % 1_000_000_000) as u32,
                )
                .single()
                .unwrap_or_else(Utc::now),
            pid: raw.pid,
            lock_id: raw.lock_id,
            mode: raw.mode,
            wait_duration_ns: raw.wait_duration_ns,
        }
    }
}

/// Row lock event DTO
#[derive(Debug, Serialize)]
pub struct RowLockEventDto {
    pub timestamp: DateTime<Utc>,
    pub pid: u32,
    pub relfilenode: u64,
    pub tuple_offset: u64,
    pub lock_mode: u32,
    pub wait_duration_ns: u64,
}

impl From<crate::ebpf::events::RowLockEventRaw> for RowLockEventDto {
    fn from(raw: crate::ebpf::events::RowLockEventRaw) -> Self {
        Self {
            timestamp: Utc
                .timestamp_opt(
                    (raw.timestamp_ns / 1_000_000_000) as i64,
                    (raw.timestamp_ns % 1_000_000_000) as u32,
                )
                .single()
                .unwrap_or_else(Utc::now),
            pid: raw.pid,
            relfilenode: raw.relfilenode,
            tuple_offset: raw.tuple_offset,
            lock_mode: raw.lock_mode,
            wait_duration_ns: raw.wait_duration_ns,
        }
    }
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
