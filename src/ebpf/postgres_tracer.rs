// src/ebpf/postgres_tracer.rs - Comprehensive PostgreSQL eBPF tracer
// Integrates protocol-level tracing, I/O attribution, and lock contention

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Query event captured from eBPF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresQueryEvent {
    pub timestamp: DateTime<Utc>,
    pub pid: u32,
    pub tid: u32,
    pub query_hash: u64,
    pub duration_ns: u64,
    pub network_latency_ns: u64,
    pub io_wait_ns: u64,
    pub cpu_time_ns: u64,
    pub bytes_sent: u32,
    pub bytes_received: u32,
    pub status: QueryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryStatus {
    Success,
    Error,
}

/// I/O statistics for a query
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryIoStats {
    pub total_io_time_ns: u64,
    pub read_time_ns: u64,
    pub write_time_ns: u64,
    pub fsync_time_ns: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_count: u32,
    pub write_count: u32,
    pub fsync_count: u32,
    pub disk_seeks: u32,
    pub page_faults: u32,
}

impl QueryIoStats {
    /// Calculate I/O percentage of total query time
    pub fn io_percentage(&self, total_duration_ns: u64) -> f64 {
        if total_duration_ns == 0 {
            return 0.0;
        }
        (self.total_io_time_ns as f64 / total_duration_ns as f64) * 100.0
    }
}

/// Lock contention information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockContentionEvent {
    pub timestamp: DateTime<Utc>,
    pub blocking_pid: u32,
    pub waiting_pid: u32,
    pub blocking_query_hash: u64,
    pub waiting_query_hash: u64,
    pub wait_duration_ns: u64,
    pub lock_type: LockType,
    pub lock_mode: LockMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockType {
    Row,
    Table,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LockMode {
    Shared,
    Exclusive,
}

/// Blocking query statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockingStats {
    pub blocking_query_hash: u64,
    pub num_awaiting_queries: u32,
    pub awaiting_duration_total_ms: u64,
    pub futex_waits: u64,
    pub blocking_count: u32,
}

/// Query breakdown with eBPF-enhanced metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryBreakdown {
    pub query_id: i64,
    pub query_hash: u64,
    pub total_time_ms: f64,

    // Time breakdown from eBPF
    pub io_time_ms: f64,
    pub cpu_time_ms: f64,
    pub network_time_ms: f64,
    pub lock_wait_time_ms: f64,

    // Percentages
    pub io_percentage: f64,
    pub cpu_percentage: f64,
    pub network_percentage: f64,
    pub lock_percentage: f64,

    // I/O details
    pub io_stats: QueryIoStats,

    // Lock contention details
    pub blocking_info: Option<BlockingStats>,

    // Recommendation
    pub recommendation: Recommendation,
    pub confidence: f64,
}

impl QueryBreakdown {
    /// Generate recommendation based on eBPF metrics
    pub fn recommend(&self) -> Recommendation {
        // High I/O percentage suggests missing index
        if self.io_percentage > 70.0 && self.io_stats.disk_seeks > 100 {
            return Recommendation::CreateIndex {
                confidence: 0.95,
                reason: format!(
                    "{:.1}% time in I/O with {} disk seeks per query",
                    self.io_percentage, self.io_stats.disk_seeks
                ),
            };
        }

        // Low I/O with high CPU suggests query rewrite
        if self.io_percentage < 20.0 && self.cpu_percentage > 60.0 {
            return Recommendation::RewriteQuery {
                confidence: 0.90,
                reason: format!(
                    "{:.1}% time in CPU, query is CPU-bound",
                    self.cpu_percentage
                ),
            };
        }

        // High lock wait time
        if self.lock_percentage > 30.0 {
            return Recommendation::ReduceLockContention {
                confidence: 0.85,
                reason: format!("{:.1}% time waiting for locks", self.lock_percentage),
            };
        }

        // High network latency
        if self.network_percentage > 20.0 {
            return Recommendation::OptimizeNetwork {
                confidence: 0.80,
                reason: format!("{:.1}% time in network latency", self.network_percentage),
            };
        }

        // High page faults suggest memory pressure
        if self.io_stats.page_faults > 1000 {
            return Recommendation::IncreaseMemory {
                confidence: 0.85,
                reason: format!(
                    "{} page faults indicate memory pressure",
                    self.io_stats.page_faults
                ),
            };
        }

        Recommendation::Investigate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Recommendation {
    CreateIndex { confidence: f64, reason: String },
    RewriteQuery { confidence: f64, reason: String },
    ReduceLockContention { confidence: f64, reason: String },
    OptimizeNetwork { confidence: f64, reason: String },
    IncreaseMemory { confidence: f64, reason: String },
    Investigate,
}

/// PostgreSQL eBPF tracer
pub struct PostgresTracer {
    query_events: Arc<Mutex<Vec<PostgresQueryEvent>>>,
    io_stats: Arc<Mutex<HashMap<u64, QueryIoStats>>>,
    lock_events: Arc<Mutex<Vec<LockContentionEvent>>>,
    blocking_stats: Arc<Mutex<HashMap<u64, BlockingStats>>>,
}

impl PostgresTracer {
    pub fn new() -> Self {
        Self {
            query_events: Arc::new(Mutex::new(Vec::new())),
            io_stats: Arc::new(Mutex::new(HashMap::new())),
            lock_events: Arc::new(Mutex::new(Vec::new())),
            blocking_stats: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record a query event from eBPF
    pub fn record_query_event(&self, event: PostgresQueryEvent) {
        if let Ok(mut events) = self.query_events.lock() {
            events.push(event);

            // Keep only last 10000 events
            if events.len() > 10000 {
                let len = events.len();
                events.drain(0..len - 10000);
            }
        }
    }

    /// Record I/O statistics
    pub fn record_io_stats(&self, query_hash: u64, stats: QueryIoStats) {
        if let Ok(mut io_stats) = self.io_stats.lock() {
            io_stats.insert(query_hash, stats);
        }
    }

    /// Record lock contention event
    pub fn record_lock_event(&self, event: LockContentionEvent) {
        if let Ok(mut events) = self.lock_events.lock() {
            events.push(event);

            // Keep only last 5000 events
            if events.len() > 5000 {
                let len = events.len();
                events.drain(0..len - 5000);
            }
        }
    }

    /// Get query breakdown with eBPF metrics
    pub fn get_query_breakdown(&self, query_hash: u64) -> Option<QueryBreakdown> {
        let events = self.query_events.lock().ok()?;
        let io_stats = self.io_stats.lock().ok()?;
        let blocking_stats = self.blocking_stats.lock().ok()?;

        // Find events for this query
        let query_events: Vec<_> = events
            .iter()
            .filter(|e| e.query_hash == query_hash)
            .collect();

        if query_events.is_empty() {
            return None;
        }

        // Calculate averages
        let count = query_events.len() as f64;
        let total_duration_ns: u64 = query_events.iter().map(|e| e.duration_ns).sum();
        let total_io_ns: u64 = query_events.iter().map(|e| e.io_wait_ns).sum();
        let total_cpu_ns: u64 = query_events.iter().map(|e| e.cpu_time_ns).sum();
        let total_network_ns: u64 = query_events.iter().map(|e| e.network_latency_ns).sum();

        let avg_total = (total_duration_ns as f64 / count) / 1_000_000.0; // Convert to ms
        let avg_io = (total_io_ns as f64 / count) / 1_000_000.0;
        let avg_cpu = (total_cpu_ns as f64 / count) / 1_000_000.0;
        let avg_network = (total_network_ns as f64 / count) / 1_000_000.0;

        let io_pct = if avg_total > 0.0 {
            (avg_io / avg_total) * 100.0
        } else {
            0.0
        };
        let cpu_pct = if avg_total > 0.0 {
            (avg_cpu / avg_total) * 100.0
        } else {
            0.0
        };
        let network_pct = if avg_total > 0.0 {
            (avg_network / avg_total) * 100.0
        } else {
            0.0
        };

        let io_stats = io_stats.get(&query_hash).cloned().unwrap_or_default();
        let blocking_info = blocking_stats.get(&query_hash).cloned();

        let mut breakdown = QueryBreakdown {
            query_id: query_hash as i64,
            query_hash,
            total_time_ms: avg_total,
            io_time_ms: avg_io,
            cpu_time_ms: avg_cpu,
            network_time_ms: avg_network,
            lock_wait_time_ms: 0.0, // Will be calculated from lock events
            io_percentage: io_pct,
            cpu_percentage: cpu_pct,
            network_percentage: network_pct,
            lock_percentage: 0.0,
            io_stats,
            blocking_info,
            recommendation: Recommendation::Investigate,
            confidence: 0.85,
        };

        breakdown.recommendation = breakdown.recommend();

        Some(breakdown)
    }

    /// Get all tracked query hashes
    pub fn get_tracked_queries(&self) -> Vec<u64> {
        if let Ok(events) = self.query_events.lock() {
            let mut hashes: Vec<u64> = events.iter().map(|e| e.query_hash).collect();
            hashes.sort_unstable();
            hashes.dedup();
            hashes
        } else {
            Vec::new()
        }
    }

    /// Clear old data
    pub fn clear_old_data(&self, older_than: DateTime<Utc>) {
        if let Ok(mut events) = self.query_events.lock() {
            events.retain(|e| e.timestamp > older_than);
        }
        if let Ok(mut events) = self.lock_events.lock() {
            events.retain(|e| e.timestamp > older_than);
        }
    }
}

impl Default for PostgresTracer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_breakdown_recommendation() {
        let breakdown = QueryBreakdown {
            query_id: 1,
            query_hash: 12345,
            total_time_ms: 100.0,
            io_time_ms: 80.0,
            cpu_time_ms: 15.0,
            network_time_ms: 2.0,
            lock_wait_time_ms: 3.0,
            io_percentage: 80.0,
            cpu_percentage: 15.0,
            network_percentage: 2.0,
            lock_percentage: 3.0,
            io_stats: QueryIoStats {
                disk_seeks: 250,
                ..Default::default()
            },
            blocking_info: None,
            recommendation: Recommendation::Investigate,
            confidence: 0.85,
        };

        let rec = breakdown.recommend();
        match rec {
            Recommendation::CreateIndex { confidence, .. } => {
                assert!(confidence > 0.9);
            }
            _ => panic!("Expected CreateIndex recommendation"),
        }
    }
}
