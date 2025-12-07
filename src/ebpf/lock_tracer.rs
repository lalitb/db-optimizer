// Lock contention tracking and analysis
// Collects lock events from eBPF probes and provides aggregated metrics

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// PostgreSQL lock modes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum LockMode {
    NoLock = 0,
    AccessShareLock = 1,
    RowShareLock = 2,
    RowExclusiveLock = 3,
    ShareUpdateExclusiveLock = 4,
    ShareLock = 5,
    ShareRowExclusiveLock = 6,
    ExclusiveLock = 7,
    AccessExclusiveLock = 8,
}

impl LockMode {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(LockMode::NoLock),
            1 => Some(LockMode::AccessShareLock),
            2 => Some(LockMode::RowShareLock),
            3 => Some(LockMode::RowExclusiveLock),
            4 => Some(LockMode::ShareUpdateExclusiveLock),
            5 => Some(LockMode::ShareLock),
            6 => Some(LockMode::ShareRowExclusiveLock),
            7 => Some(LockMode::ExclusiveLock),
            8 => Some(LockMode::AccessExclusiveLock),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LockMode::NoLock => "NoLock",
            LockMode::AccessShareLock => "AccessShareLock",
            LockMode::RowShareLock => "RowShareLock",
            LockMode::RowExclusiveLock => "RowExclusiveLock",
            LockMode::ShareUpdateExclusiveLock => "ShareUpdateExclusiveLock",
            LockMode::ShareLock => "ShareLock",
            LockMode::ShareRowExclusiveLock => "ShareRowExclusiveLock",
            LockMode::ExclusiveLock => "ExclusiveLock",
            LockMode::AccessExclusiveLock => "AccessExclusiveLock",
        }
    }

    /// Returns true if this is a blocking lock mode (exclusive or share)
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            LockMode::RowExclusiveLock
                | LockMode::ShareUpdateExclusiveLock
                | LockMode::ShareLock
                | LockMode::ShareRowExclusiveLock
                | LockMode::ExclusiveLock
                | LockMode::AccessExclusiveLock
        )
    }
}

/// Lock event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LockEventType {
    AcquireStart = 1,
    AcquireSuccess = 2,
    AcquireFailed = 3,
    Release = 4,
}

impl LockEventType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(LockEventType::AcquireStart),
            2 => Some(LockEventType::AcquireSuccess),
            3 => Some(LockEventType::AcquireFailed),
            4 => Some(LockEventType::Release),
            _ => None,
        }
    }
}

/// Raw lock event from eBPF
#[derive(Debug, Clone)]
pub struct LockEvent {
    pub timestamp: DateTime<Utc>,
    pub pid: u32,
    pub table_oid: u32,
    pub lock_mode: LockMode,
    pub event_type: LockEventType,
    pub granted_fast: bool,
    pub wait_time_ns: u64,
    pub process_name: String,
}

/// Aggregated lock metrics for a table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockContentionMetrics {
    pub table_oid: u32,
    pub table_name: Option<String>,
    pub lock_mode: LockMode,
    pub wait_count: u64,
    pub total_wait_time_ns: u64,
    pub avg_wait_time_ns: u64,
    pub max_wait_time_ns: u64,
    pub min_wait_time_ns: u64,
    pub failed_count: u64,
    pub fastpath_count: u64,
    pub slowpath_count: u64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl LockContentionMetrics {
    pub fn avg_wait_time_ms(&self) -> f64 {
        self.avg_wait_time_ns as f64 / 1_000_000.0
    }

    pub fn max_wait_time_ms(&self) -> f64 {
        self.max_wait_time_ns as f64 / 1_000_000.0
    }

    pub fn total_wait_time_ms(&self) -> f64 {
        self.total_wait_time_ns as f64 / 1_000_000.0
    }

    /// Returns contention severity: 0.0 (none) to 1.0 (critical)
    pub fn contention_severity(&self) -> f64 {
        // Heuristic based on:
        // - Average wait time (>100ms is concerning)
        // - Failed acquisitions (indicates high contention)
        // - Slowpath usage (fastpath bypass means contention)

        let wait_score = (self.avg_wait_time_ms() / 100.0).min(1.0);
        let failure_rate = if self.wait_count > 0 {
            self.failed_count as f64 / self.wait_count as f64
        } else {
            0.0
        };
        let slowpath_rate = if (self.fastpath_count + self.slowpath_count) > 0 {
            self.slowpath_count as f64 / (self.fastpath_count + self.slowpath_count) as f64
        } else {
            0.0
        };

        // Weighted average
        (wait_score * 0.5 + failure_rate * 0.3 + slowpath_rate * 0.2).min(1.0)
    }

    /// Returns true if contention is high enough to warrant attention
    pub fn is_high_contention(&self) -> bool {
        // High contention if:
        // - Average wait time > 50ms, OR
        // - >5% failure rate, OR
        // - >50% slowpath
        self.avg_wait_time_ms() > 50.0
            || (self.wait_count > 10 && (self.failed_count as f64 / self.wait_count as f64) > 0.05)
            || (self.wait_count > 10
                && (self.slowpath_count as f64
                    / (self.fastpath_count + self.slowpath_count) as f64)
                    > 0.5)
    }
}

/// Key for lock statistics aggregation
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct LockStatsKey {
    table_oid: u32,
    lock_mode: LockMode,
}

/// Lock tracer that collects and aggregates lock events
pub struct LockTracer {
    /// Aggregated statistics per table/lock_mode
    stats: Arc<Mutex<HashMap<LockStatsKey, LockContentionMetrics>>>,

    /// Table OID to name mapping (populated from PostgreSQL metadata)
    table_names: Arc<Mutex<HashMap<u32, String>>>,

    /// Recent events (for debugging/analysis)
    recent_events: Arc<Mutex<Vec<LockEvent>>>,
    max_recent_events: usize,
}

impl LockTracer {
    pub fn new() -> Self {
        Self {
            stats: Arc::new(Mutex::new(HashMap::new())),
            table_names: Arc::new(Mutex::new(HashMap::new())),
            recent_events: Arc::new(Mutex::new(Vec::new())),
            max_recent_events: 1000,
        }
    }

    /// Process a lock event from eBPF
    pub fn process_event(&self, event: LockEvent) -> Result<()> {
        // Store in recent events
        {
            let mut recent = self.recent_events.lock().unwrap();
            recent.push(event.clone());
            if recent.len() > self.max_recent_events {
                recent.remove(0);
            }
        }

        // Update aggregated statistics
        let key = LockStatsKey {
            table_oid: event.table_oid,
            lock_mode: event.lock_mode,
        };

        let mut stats = self.stats.lock().unwrap();
        let metrics = stats.entry(key).or_insert_with(|| LockContentionMetrics {
            table_oid: event.table_oid,
            table_name: None,
            lock_mode: event.lock_mode,
            wait_count: 0,
            total_wait_time_ns: 0,
            avg_wait_time_ns: 0,
            max_wait_time_ns: 0,
            min_wait_time_ns: u64::MAX,
            failed_count: 0,
            fastpath_count: 0,
            slowpath_count: 0,
            first_seen: event.timestamp,
            last_seen: event.timestamp,
        });

        // Update based on event type
        match event.event_type {
            LockEventType::AcquireStart => {
                metrics.wait_count += 1;
            }
            LockEventType::AcquireSuccess => {
                if event.wait_time_ns > 0 {
                    metrics.total_wait_time_ns += event.wait_time_ns;
                    metrics.max_wait_time_ns = metrics.max_wait_time_ns.max(event.wait_time_ns);
                    metrics.min_wait_time_ns = metrics.min_wait_time_ns.min(event.wait_time_ns);
                }
                if event.granted_fast {
                    metrics.fastpath_count += 1;
                } else {
                    metrics.slowpath_count += 1;
                }
            }
            LockEventType::AcquireFailed => {
                metrics.failed_count += 1;
            }
            LockEventType::Release => {
                // Just record the release
            }
        }

        // Recalculate averages
        if metrics.wait_count > 0 {
            metrics.avg_wait_time_ns = metrics.total_wait_time_ns / metrics.wait_count;
        }
        metrics.last_seen = event.timestamp;

        Ok(())
    }

    /// Update table name mapping from PostgreSQL metadata
    pub fn update_table_names(&self, oid_to_name: HashMap<u32, String>) {
        let mut names = self.table_names.lock().unwrap();
        names.extend(oid_to_name);

        // Update existing metrics with table names
        let mut stats = self.stats.lock().unwrap();
        for metrics in stats.values_mut() {
            if let Some(name) = names.get(&metrics.table_oid) {
                metrics.table_name = Some(name.clone());
            }
        }
    }

    /// Get all contention metrics
    pub fn get_all_metrics(&self) -> Vec<LockContentionMetrics> {
        let stats = self.stats.lock().unwrap();
        stats.values().cloned().collect()
    }

    /// Get high contention tables (sorted by severity)
    pub fn get_high_contention(&self) -> Vec<LockContentionMetrics> {
        let mut metrics: Vec<_> = self
            .get_all_metrics()
            .into_iter()
            .filter(|m| m.is_high_contention())
            .collect();

        metrics.sort_by(|a, b| {
            b.contention_severity()
                .partial_cmp(&a.contention_severity())
                .unwrap()
        });

        metrics
    }

    /// Get contention metrics for a specific table
    pub fn get_table_metrics(&self, table_oid: u32) -> Vec<LockContentionMetrics> {
        let stats = self.stats.lock().unwrap();
        stats
            .values()
            .filter(|m| m.table_oid == table_oid)
            .cloned()
            .collect()
    }

    /// Get recent events
    pub fn get_recent_events(&self, limit: usize) -> Vec<LockEvent> {
        let events = self.recent_events.lock().unwrap();
        events.iter().rev().take(limit).cloned().collect()
    }

    /// Clear all statistics
    pub fn clear_stats(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.clear();
        let mut events = self.recent_events.lock().unwrap();
        events.clear();
    }

    /// Get summary statistics
    pub fn get_summary(&self) -> LockContentionSummary {
        let stats = self.stats.lock().unwrap();
        let events = self.recent_events.lock().unwrap();

        let total_tables = stats
            .keys()
            .map(|k| k.table_oid)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let high_contention_count = stats.values().filter(|m| m.is_high_contention()).count();
        let total_wait_time_ms: f64 = stats.values().map(|m| m.total_wait_time_ms()).sum();
        let total_failed: u64 = stats.values().map(|m| m.failed_count).sum();

        LockContentionSummary {
            total_tables_tracked: total_tables,
            high_contention_tables: high_contention_count,
            total_lock_events: events.len(),
            total_wait_time_ms,
            total_failed_acquisitions: total_failed,
        }
    }
}

impl Default for LockTracer {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of lock contention across all tables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockContentionSummary {
    pub total_tables_tracked: usize,
    pub high_contention_tables: usize,
    pub total_lock_events: usize,
    pub total_wait_time_ms: f64,
    pub total_failed_acquisitions: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_mode_conversion() {
        assert_eq!(LockMode::from_u8(1), Some(LockMode::AccessShareLock));
        assert_eq!(LockMode::from_u8(8), Some(LockMode::AccessExclusiveLock));
        assert_eq!(LockMode::from_u8(99), None);
    }

    #[test]
    fn test_lock_mode_is_blocking() {
        assert!(!LockMode::AccessShareLock.is_blocking());
        assert!(LockMode::ExclusiveLock.is_blocking());
        assert!(LockMode::RowExclusiveLock.is_blocking());
    }

    #[test]
    fn test_contention_severity() {
        let metrics = LockContentionMetrics {
            table_oid: 123,
            table_name: Some("test_table".to_string()),
            lock_mode: LockMode::ExclusiveLock,
            wait_count: 100,
            total_wait_time_ns: 10_000_000_000, // 10 seconds
            avg_wait_time_ns: 100_000_000,      // 100ms average
            max_wait_time_ns: 500_000_000,
            min_wait_time_ns: 10_000_000,
            failed_count: 10,
            fastpath_count: 50,
            slowpath_count: 50,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };

        let severity = metrics.contention_severity();
        assert!(severity > 0.5, "Expected high severity, got {}", severity);
        assert!(metrics.is_high_contention());
    }

    #[test]
    fn test_lock_tracer() {
        let tracer = LockTracer::new();

        let event = LockEvent {
            timestamp: Utc::now(),
            pid: 1234,
            table_oid: 456,
            lock_mode: LockMode::RowExclusiveLock,
            event_type: LockEventType::AcquireSuccess,
            granted_fast: false,
            wait_time_ns: 50_000_000, // 50ms
            process_name: "postgres".to_string(),
        };

        tracer.process_event(event).unwrap();

        let metrics = tracer.get_all_metrics();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].table_oid, 456);
    }
}
