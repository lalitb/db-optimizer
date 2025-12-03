use serde::{Deserialize, Serialize};
use crate::ebpf::lock_tracer::LockContentionSummary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub version: String,
    pub uptime_seconds: u64,
    pub ebpf_enabled: bool,
    pub attached_pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetrics {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub lock_summary: Option<LockContentionSummary>,
    // Future: IO stats, CPU stats
}
