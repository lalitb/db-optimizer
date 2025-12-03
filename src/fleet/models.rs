use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use crate::models::ReportSummary;
use crate::ebpf::lock_tracer::LockContentionSummary;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInstance {
    pub id: String,
    pub name: String,
    pub connection_string: String, // In a real app, this should be encrypted or stored securely
    pub agent_url: Option<String>, // URL of the Vaanyze Node Agent (e.g., http://host:9090)
    pub environment: String, // "production", "staging", "dev"
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_check_at: Option<DateTime<Utc>>,
    pub health_score: Option<u8>,
    pub status: InstanceStatus,
    pub version: Option<String>,
    
    // Detailed metrics
    pub metrics: Option<InstanceMetrics>,
    pub summary: Option<ReportSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceMetrics {
    // Storage
    pub total_size_mb: f64,
    pub table_count: usize,
    pub index_count: usize,
    
    // Connections
    pub active_connections: i32,
    pub max_connections: i32,
    pub connection_usage_pct: f64,
    pub idle_in_transaction_count: i32,
    pub locked_connections_count: i32,

    // eBPF Metrics (if available)
    pub ebpf_lock_summary: Option<LockContentionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InstanceStatus {
    Unknown,
    Healthy,
    Warning,
    Critical,
    Unreachable,
}

impl DatabaseInstance {
    pub fn new(name: String, connection_string: String, environment: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            connection_string,
            agent_url: None,
            environment,
            tags: Vec::new(),
            created_at: Utc::now(),
            last_check_at: None,
            health_score: None,
            status: InstanceStatus::Unknown,
            version: None,
            metrics: None,
            summary: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetSummary {
    pub total_databases: usize,
    pub healthy_count: usize,
    pub warning_count: usize,
    pub critical_count: usize,
    pub unreachable_count: usize,
    pub average_health_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub connection_string: String,
    pub environment: String,
    pub tags: Option<Vec<String>>,
}
