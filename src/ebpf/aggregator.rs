// src/ebpf/aggregator.rs - Userspace event aggregation with drop detection
//
// This module processes raw eBPF events from the perf ring buffer and
// aggregates them into meaningful metrics. All aggregation logic lives
// here in userspace - the kernel programs only emit lightweight events.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use super::events::{EventType, RawEvent};
use super::SystemMetrics;

/// Aggregated metrics for a single query/process
#[derive(Debug, Clone, Default)]
pub struct AggregatedMetrics {
    /// Timestamp of first event
    pub start_time_ns: u64,
    /// Timestamp of last event
    pub last_event_ns: u64,

    // I/O metrics
    pub total_bytes_read: u64,
    pub total_bytes_written: u64,
    pub read_ops: u32,
    pub write_ops: u32,
    pub fsync_count: u32,

    // Scheduling metrics
    pub context_switches: u32,
    pub total_off_cpu_ns: u64,
    pub last_switch_time: u64,

    // Memory metrics
    pub page_faults: u32,

    // Query lifecycle
    pub query_start_ns: Option<u64>,
    pub query_hash: Option<u64>,
}

impl AggregatedMetrics {
    /// Convert to SystemMetrics for the correlation engine
    pub fn to_system_metrics(&self, events_processed: u64, events_dropped: u64) -> SystemMetrics {
        let total_time = if self.last_event_ns > self.start_time_ns {
            (self.last_event_ns - self.start_time_ns) / 1000 // Convert ns to us
        } else {
            0
        };

        // Estimate CPU time as total time minus off-cpu time
        let off_cpu_us = self.total_off_cpu_ns / 1000;
        let cpu_time = total_time.saturating_sub(off_cpu_us);

        SystemMetrics {
            total_time_us: total_time,
            cpu_time_us: cpu_time,
            io_wait_time_us: 0,   // TODO: Calculate from block I/O events
            lock_wait_time_us: 0, // Populated by lock tracer
            bytes_read: self.total_bytes_read,
            bytes_written: self.total_bytes_written,
            read_syscalls: self.read_ops,
            write_syscalls: self.write_ops,
            fsync_count: self.fsync_count,
            page_cache_hits: 0, // TODO: Track from page cache events
            page_cache_misses: 0,
            page_faults: self.page_faults,
            context_switches: self.context_switches,
            events_processed,
            events_dropped,
        }
    }
}

/// Event aggregator that processes eBPF events in userspace
///
/// Key design principles:
/// 1. All aggregation happens here, not in kernel
/// 2. Tracks dropped events for confidence scoring
/// 3. Thread-safe for concurrent access
/// 4. Handles clock offset between kernel and userspace
pub struct EventAggregator {
    /// Per-PID aggregated metrics
    query_metrics: Arc<RwLock<HashMap<u32, AggregatedMetrics>>>,

    /// Total events successfully processed
    events_processed: Arc<AtomicU64>,

    /// Total events dropped due to buffer overflow
    events_dropped: Arc<AtomicU64>,

    /// Offset between kernel monotonic clock and system time
    monotonic_offset_ns: i64,
}

impl EventAggregator {
    /// Create a new event aggregator
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let offset = Self::calculate_clock_offset()?;
        debug!("Calculated monotonic clock offset: {}ns", offset);

        Ok(Self {
            query_metrics: Arc::new(RwLock::new(HashMap::new())),
            events_processed: Arc::new(AtomicU64::new(0)),
            events_dropped: Arc::new(AtomicU64::new(0)),
            monotonic_offset_ns: offset,
        })
    }

    /// Calculate offset between kernel monotonic clock and system time
    fn calculate_clock_offset() -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };

        if result != 0 {
            return Err("Failed to get monotonic time".into());
        }

        let kernel_ns = ts.tv_sec as i64 * 1_000_000_000 + ts.tv_nsec as i64;
        let system_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as i64;

        Ok(system_ns - kernel_ns)
    }

    /// Process a batch of raw events
    pub async fn process_events(
        &self,
        events: Vec<RawEvent>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let count = events.len();

        for event in events {
            self.aggregate_event(event).await?;
        }

        self.events_processed
            .fetch_add(count as u64, Ordering::Relaxed);
        Ok(count)
    }

    /// Record dropped events from perf buffer overflow
    pub fn record_dropped_events(&self, count: u64) {
        self.events_dropped.fetch_add(count, Ordering::Relaxed);

        let total_processed = self.events_processed.load(Ordering::Relaxed);
        let total_dropped = self.events_dropped.load(Ordering::Relaxed);
        let loss_rate = if total_processed + total_dropped > 0 {
            (total_dropped as f64 / (total_processed + total_dropped) as f64) * 100.0
        } else {
            0.0
        };

        if loss_rate > 1.0 {
            warn!(
                "eBPF sampling degraded: {:.2}% events lost ({} dropped, {} processed)",
                loss_rate, total_dropped, total_processed
            );
        }
    }

    /// Aggregate a single event into the metrics store
    async fn aggregate_event(
        &self,
        event: RawEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut metrics = self.query_metrics.write().await;
        let metric = metrics.entry(event.pid).or_default();

        // Update timestamps
        if metric.start_time_ns == 0 {
            metric.start_time_ns = event.timestamp_ns;
        }
        metric.last_event_ns = event.timestamp_ns;

        // Aggregate based on event type
        match event.event_type() {
            EventType::Read => {
                metric.total_bytes_read += event.value1;
                metric.read_ops += 1;
            }
            EventType::Write => {
                metric.total_bytes_written += event.value1;
                metric.write_ops += 1;
            }
            EventType::SchedSwitch => {
                // Track off-CPU time
                if metric.last_switch_time > 0 {
                    let off_cpu = event.timestamp_ns.saturating_sub(metric.last_switch_time);
                    metric.total_off_cpu_ns += off_cpu;
                }
                metric.last_switch_time = event.timestamp_ns;
                metric.context_switches += 1;
            }
            EventType::PageFault => {
                metric.page_faults += 1;
            }
            EventType::QueryStart => {
                metric.query_start_ns = Some(event.timestamp_ns);
                metric.query_hash = Some(event.value1);
            }
            EventType::QueryEnd => {
                // Query duration is in value1 for end events
                // Could store this for histogram tracking
            }
            EventType::Fsync => {
                metric.fsync_count += 1;
            }
            EventType::LockWait => {
                // Lock events handled by lock_tracer
            }
            EventType::Unknown => {
                debug!("Unknown event type: {}", event.event_type_raw);
            }
        }

        Ok(())
    }

    /// Get metrics for a specific PID
    pub async fn get_metrics(&self, pid: u32) -> Option<AggregatedMetrics> {
        let metrics = self.query_metrics.read().await;
        metrics.get(&pid).cloned()
    }

    /// Get metrics for a PID and convert to SystemMetrics
    pub async fn get_system_metrics(&self, pid: u32) -> Option<SystemMetrics> {
        let metrics = self.query_metrics.read().await;
        metrics.get(&pid).map(|m| {
            m.to_system_metrics(
                self.events_processed.load(Ordering::Relaxed),
                self.events_dropped.load(Ordering::Relaxed),
            )
        })
    }

    /// Get all tracked PIDs
    pub async fn get_tracked_pids(&self) -> Vec<u32> {
        let metrics = self.query_metrics.read().await;
        metrics.keys().cloned().collect()
    }

    /// Clear metrics for a specific PID (e.g., after query completes)
    pub async fn clear_metrics(&self, pid: u32) {
        let mut metrics = self.query_metrics.write().await;
        metrics.remove(&pid);
    }

    /// Clear all metrics
    pub async fn clear_all(&self) {
        let mut metrics = self.query_metrics.write().await;
        metrics.clear();
    }

    /// Get confidence score based on event quality
    pub fn get_confidence(&self) -> f64 {
        let processed = self.events_processed.load(Ordering::Relaxed) as f64;
        let dropped = self.events_dropped.load(Ordering::Relaxed) as f64;

        if processed == 0.0 {
            return 0.0;
        }

        let success_rate = processed / (processed + dropped);

        match success_rate {
            r if r >= 0.99 => 0.95, // <1% loss = high confidence
            r if r >= 0.95 => 0.80, // <5% loss = medium confidence
            r if r >= 0.90 => 0.60, // <10% loss = low confidence
            _ => 0.40,              // >10% loss = very low confidence
        }
    }

    /// Get event processing statistics
    pub fn get_stats(&self) -> (u64, u64) {
        (
            self.events_processed.load(Ordering::Relaxed),
            self.events_dropped.load(Ordering::Relaxed),
        )
    }

    /// Convert kernel timestamp to wall clock time
    pub fn kernel_to_wall_time(&self, kernel_ns: u64) -> u64 {
        (kernel_ns as i64 + self.monotonic_offset_ns) as u64
    }
}

impl Default for EventAggregator {
    fn default() -> Self {
        Self::new().expect("Failed to create EventAggregator")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_aggregation() {
        let aggregator = EventAggregator::new().unwrap();

        let events = vec![
            RawEvent {
                timestamp_ns: 1000000,
                pid: 123,
                tid: 123,
                event_type_raw: 1, // Read
                _pad: [0; 3],
                value1: 4096,
                value2: 0,
            },
            RawEvent {
                timestamp_ns: 2000000,
                pid: 123,
                tid: 123,
                event_type_raw: 1, // Read
                _pad: [0; 3],
                value1: 8192,
                value2: 0,
            },
        ];

        let count = aggregator.process_events(events).await.unwrap();
        assert_eq!(count, 2);

        let metrics = aggregator.get_metrics(123).await.unwrap();
        assert_eq!(metrics.total_bytes_read, 4096 + 8192);
        assert_eq!(metrics.read_ops, 2);
    }

    #[tokio::test]
    async fn test_confidence_scoring() {
        let aggregator = EventAggregator::new().unwrap();

        // No events = no confidence
        assert_eq!(aggregator.get_confidence(), 0.0);

        // Process some events
        aggregator.events_processed.store(100, Ordering::Relaxed);
        assert!(aggregator.get_confidence() > 0.9);

        // Add dropped events
        aggregator.events_dropped.store(10, Ordering::Relaxed);
        let confidence = aggregator.get_confidence();
        assert!(confidence < 0.95 && confidence > 0.5);
    }
}
