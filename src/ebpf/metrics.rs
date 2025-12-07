// src/ebpf/metrics.rs
//
// Prometheus metrics for eBPF self-observability
// Reference: Enhanced eBPF Implementation Plan v2 - "Self-Observability" section
//
// Tracks:
// - Event throughput and latency
// - Drop rates and sampling effectiveness
// - Correlation success/failure rates
// - Daemon health and IPC performance

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Counter for monotonically increasing values
#[derive(Debug, Default)]
pub struct Counter {
    value: AtomicU64,
}

impl Counter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_by(&self, v: u64) {
        self.value.fetch_add(v, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Gauge for values that can go up and down
#[derive(Debug, Default)]
pub struct Gauge {
    value: AtomicU64,
}

impl Gauge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, v: u64) {
        self.value.store(v, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Histogram for tracking distributions (simplified fixed-bucket implementation)
#[derive(Debug)]
pub struct Histogram {
    /// Bucket boundaries in microseconds
    buckets: Vec<u64>,
    /// Count per bucket
    counts: Vec<AtomicU64>,
    /// Total sum of all observations
    sum: AtomicU64,
    /// Total count of all observations
    count: AtomicU64,
}

impl Histogram {
    /// Create histogram with default latency buckets (in microseconds)
    pub fn new_latency() -> Self {
        // Buckets: 10us, 50us, 100us, 500us, 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s
        let buckets = vec![
            10, 50, 100, 500, 1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000,
        ];
        let counts = buckets.iter().map(|_| AtomicU64::new(0)).collect();
        Self {
            buckets,
            counts,
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Create histogram with size buckets (in bytes)
    pub fn new_size() -> Self {
        // Buckets: 64B, 256B, 1KB, 4KB, 16KB, 64KB, 256KB, 1MB
        let buckets = vec![64, 256, 1024, 4096, 16384, 65536, 262144, 1048576];
        let counts = buckets.iter().map(|_| AtomicU64::new(0)).collect();
        Self {
            buckets,
            counts,
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Observe a value
    pub fn observe(&self, value: u64) {
        self.sum.fetch_add(value, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        // Find the right bucket
        for (i, bound) in self.buckets.iter().enumerate() {
            if value <= *bound {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        // Value exceeds all buckets - add to last bucket
        if let Some(last) = self.counts.last() {
            last.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get bucket counts for Prometheus exposition
    pub fn get_buckets(&self) -> Vec<(u64, u64)> {
        self.buckets
            .iter()
            .zip(self.counts.iter())
            .map(|(bound, count)| (*bound, count.load(Ordering::Relaxed)))
            .collect()
    }

    /// Get total count
    pub fn get_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get sum
    pub fn get_sum(&self) -> u64 {
        self.sum.load(Ordering::Relaxed)
    }

    /// Get average
    pub fn get_avg(&self) -> f64 {
        let count = self.get_count();
        if count == 0 {
            return 0.0;
        }
        self.get_sum() as f64 / count as f64
    }
}

/// All eBPF-related metrics
#[derive(Debug)]
pub struct EbpfMetrics {
    // Event processing
    pub events_received: Counter,
    pub events_processed: Counter,
    pub events_dropped_kernel: Counter,
    pub events_dropped_userspace: Counter,
    pub events_parse_errors: Counter,

    // Sampling
    pub sampling_rate: Gauge,
    pub events_sampled: Counter,
    pub events_skipped_sampling: Counter,

    // Aggregation
    pub aggregation_flushes: Counter,
    pub aggregation_window_duration_us: Histogram,
    pub queries_aggregated: Counter,

    // Correlation
    pub pg_stat_polls: Counter,
    pub pg_stat_poll_errors: Counter,
    pub correlation_hits: Counter,
    pub correlation_misses: Counter,
    pub pid_cache_size: Gauge,

    // IPC
    pub ipc_messages_sent: Counter,
    pub ipc_messages_received: Counter,
    pub ipc_bytes_sent: Counter,
    pub ipc_bytes_received: Counter,
    pub ipc_errors: Counter,
    pub ipc_latency_us: Histogram,

    // Daemon
    pub daemon_uptime_seconds: Gauge,
    pub active_clients: Gauge,
    pub active_probes: Gauge,

    // BPF programs
    pub bpf_programs_loaded: Gauge,
    pub bpf_program_load_errors: Counter,
    pub bpf_attach_errors: Counter,
    pub perf_buffer_read_errors: Counter,
}

impl Default for EbpfMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl EbpfMetrics {
    pub fn new() -> Self {
        Self {
            // Event processing
            events_received: Counter::new(),
            events_processed: Counter::new(),
            events_dropped_kernel: Counter::new(),
            events_dropped_userspace: Counter::new(),
            events_parse_errors: Counter::new(),

            // Sampling
            sampling_rate: Gauge::new(),
            events_sampled: Counter::new(),
            events_skipped_sampling: Counter::new(),

            // Aggregation
            aggregation_flushes: Counter::new(),
            aggregation_window_duration_us: Histogram::new_latency(),
            queries_aggregated: Counter::new(),

            // Correlation
            pg_stat_polls: Counter::new(),
            pg_stat_poll_errors: Counter::new(),
            correlation_hits: Counter::new(),
            correlation_misses: Counter::new(),
            pid_cache_size: Gauge::new(),

            // IPC
            ipc_messages_sent: Counter::new(),
            ipc_messages_received: Counter::new(),
            ipc_bytes_sent: Counter::new(),
            ipc_bytes_received: Counter::new(),
            ipc_errors: Counter::new(),
            ipc_latency_us: Histogram::new_latency(),

            // Daemon
            daemon_uptime_seconds: Gauge::new(),
            active_clients: Gauge::new(),
            active_probes: Gauge::new(),

            // BPF programs
            bpf_programs_loaded: Gauge::new(),
            bpf_program_load_errors: Counter::new(),
            bpf_attach_errors: Counter::new(),
            perf_buffer_read_errors: Counter::new(),
        }
    }

    /// Calculate drop rate as a percentage
    pub fn drop_rate(&self) -> f64 {
        let received = self.events_received.get();
        if received == 0 {
            return 0.0;
        }
        let dropped = self.events_dropped_kernel.get() + self.events_dropped_userspace.get();
        (dropped as f64 / received as f64) * 100.0
    }

    /// Calculate correlation hit rate
    pub fn correlation_rate(&self) -> f64 {
        let total = self.correlation_hits.get() + self.correlation_misses.get();
        if total == 0 {
            return 0.0;
        }
        (self.correlation_hits.get() as f64 / total as f64) * 100.0
    }

    /// Export metrics in Prometheus text format
    pub fn prometheus_export(&self) -> String {
        let mut output = String::new();

        // Helper macro for counters
        macro_rules! counter {
            ($name:expr, $help:expr, $value:expr) => {
                output.push_str(&format!(
                    "# HELP {} {}\n# TYPE {} counter\n{} {}\n",
                    $name, $help, $name, $name, $value
                ));
            };
        }

        // Helper macro for gauges
        macro_rules! gauge {
            ($name:expr, $help:expr, $value:expr) => {
                output.push_str(&format!(
                    "# HELP {} {}\n# TYPE {} gauge\n{} {}\n",
                    $name, $help, $name, $name, $value
                ));
            };
        }

        // Event processing metrics
        counter!(
            "ebpf_events_received_total",
            "Total number of events received from kernel",
            self.events_received.get()
        );
        counter!(
            "ebpf_events_processed_total",
            "Total number of events successfully processed",
            self.events_processed.get()
        );
        counter!(
            "ebpf_events_dropped_kernel_total",
            "Events dropped in kernel perf buffer",
            self.events_dropped_kernel.get()
        );
        counter!(
            "ebpf_events_dropped_userspace_total",
            "Events dropped in userspace processing",
            self.events_dropped_userspace.get()
        );
        counter!(
            "ebpf_events_parse_errors_total",
            "Events with parse errors",
            self.events_parse_errors.get()
        );

        // Sampling metrics
        gauge!(
            "ebpf_sampling_rate",
            "Current sampling rate (1 = all events, N = 1-in-N sampling)",
            self.sampling_rate.get()
        );
        counter!(
            "ebpf_events_sampled_total",
            "Events that passed sampling",
            self.events_sampled.get()
        );
        counter!(
            "ebpf_events_skipped_sampling_total",
            "Events skipped due to sampling",
            self.events_skipped_sampling.get()
        );

        // Aggregation metrics
        counter!(
            "ebpf_aggregation_flushes_total",
            "Number of aggregation window flushes",
            self.aggregation_flushes.get()
        );
        counter!(
            "ebpf_queries_aggregated_total",
            "Total queries aggregated",
            self.queries_aggregated.get()
        );

        // Correlation metrics
        counter!(
            "ebpf_pg_stat_polls_total",
            "Total pg_stat_activity polls",
            self.pg_stat_polls.get()
        );
        counter!(
            "ebpf_pg_stat_poll_errors_total",
            "Failed pg_stat_activity polls",
            self.pg_stat_poll_errors.get()
        );
        counter!(
            "ebpf_correlation_hits_total",
            "PIDs successfully correlated to queries",
            self.correlation_hits.get()
        );
        counter!(
            "ebpf_correlation_misses_total",
            "PIDs that could not be correlated",
            self.correlation_misses.get()
        );
        gauge!(
            "ebpf_pid_cache_size",
            "Current size of PID-to-query cache",
            self.pid_cache_size.get()
        );

        // IPC metrics
        counter!(
            "ebpf_ipc_messages_sent_total",
            "Total IPC messages sent",
            self.ipc_messages_sent.get()
        );
        counter!(
            "ebpf_ipc_messages_received_total",
            "Total IPC messages received",
            self.ipc_messages_received.get()
        );
        counter!(
            "ebpf_ipc_bytes_sent_total",
            "Total IPC bytes sent",
            self.ipc_bytes_sent.get()
        );
        counter!(
            "ebpf_ipc_bytes_received_total",
            "Total IPC bytes received",
            self.ipc_bytes_received.get()
        );
        counter!(
            "ebpf_ipc_errors_total",
            "Total IPC errors",
            self.ipc_errors.get()
        );

        // Daemon metrics
        gauge!(
            "ebpf_daemon_uptime_seconds",
            "Daemon uptime in seconds",
            self.daemon_uptime_seconds.get()
        );
        gauge!(
            "ebpf_active_clients",
            "Number of active IPC clients",
            self.active_clients.get()
        );
        gauge!(
            "ebpf_active_probes",
            "Number of active eBPF probes",
            self.active_probes.get()
        );

        // BPF program metrics
        gauge!(
            "ebpf_bpf_programs_loaded",
            "Number of BPF programs currently loaded",
            self.bpf_programs_loaded.get()
        );
        counter!(
            "ebpf_bpf_program_load_errors_total",
            "BPF program load failures",
            self.bpf_program_load_errors.get()
        );
        counter!(
            "ebpf_bpf_attach_errors_total",
            "BPF probe attach failures",
            self.bpf_attach_errors.get()
        );
        counter!(
            "ebpf_perf_buffer_read_errors_total",
            "Perf buffer read failures",
            self.perf_buffer_read_errors.get()
        );

        // Histogram for IPC latency
        output.push_str("# HELP ebpf_ipc_latency_us IPC round-trip latency in microseconds\n");
        output.push_str("# TYPE ebpf_ipc_latency_us histogram\n");
        let mut cumulative = 0u64;
        for (bound, count) in self.ipc_latency_us.get_buckets() {
            cumulative += count;
            output.push_str(&format!(
                "ebpf_ipc_latency_us_bucket{{le=\"{}\"}} {}\n",
                bound, cumulative
            ));
        }
        output.push_str(&format!(
            "ebpf_ipc_latency_us_bucket{{le=\"+Inf\"}} {}\n",
            self.ipc_latency_us.get_count()
        ));
        output.push_str(&format!(
            "ebpf_ipc_latency_us_sum {}\n",
            self.ipc_latency_us.get_sum()
        ));
        output.push_str(&format!(
            "ebpf_ipc_latency_us_count {}\n",
            self.ipc_latency_us.get_count()
        ));

        output
    }
}

/// Shared metrics instance
pub type SharedMetrics = Arc<EbpfMetrics>;

/// Create a new shared metrics instance
pub fn new_shared_metrics() -> SharedMetrics {
    Arc::new(EbpfMetrics::new())
}

/// Timer guard for measuring durations
pub struct Timer {
    start: Instant,
    histogram: Option<*const Histogram>,
}

impl Timer {
    /// Start a new timer
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
            histogram: None,
        }
    }

    /// Start a timer that will record to a histogram on drop
    ///
    /// # Safety
    /// The histogram must outlive this timer
    pub fn start_with_histogram(histogram: &Histogram) -> Self {
        Self {
            start: Instant::now(),
            histogram: Some(histogram as *const Histogram),
        }
    }

    /// Get elapsed time in microseconds
    pub fn elapsed_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    /// Get elapsed duration
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Stop the timer and return elapsed microseconds
    pub fn stop(self) -> u64 {
        self.elapsed_us()
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(histogram_ptr) = self.histogram {
            // SAFETY: We trust that the histogram outlives the timer
            unsafe {
                (*histogram_ptr).observe(self.elapsed_us());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter() {
        let counter = Counter::new();
        assert_eq!(counter.get(), 0);

        counter.inc();
        assert_eq!(counter.get(), 1);

        counter.inc_by(10);
        assert_eq!(counter.get(), 11);
    }

    #[test]
    fn test_gauge() {
        let gauge = Gauge::new();
        assert_eq!(gauge.get(), 0);

        gauge.set(42);
        assert_eq!(gauge.get(), 42);

        gauge.inc();
        assert_eq!(gauge.get(), 43);

        gauge.dec();
        assert_eq!(gauge.get(), 42);
    }

    #[test]
    fn test_histogram() {
        let histogram = Histogram::new_latency();

        histogram.observe(5); // 10us bucket
        histogram.observe(25); // 50us bucket
        histogram.observe(75); // 100us bucket

        assert_eq!(histogram.get_count(), 3);
        assert_eq!(histogram.get_sum(), 105);

        let buckets = histogram.get_buckets();
        assert_eq!(buckets[0], (10, 1)); // 10us bucket has 1 observation
        assert_eq!(buckets[1], (50, 1)); // 50us bucket has 1 observation
        assert_eq!(buckets[2], (100, 1)); // 100us bucket has 1 observation
    }

    #[test]
    fn test_prometheus_export() {
        let metrics = EbpfMetrics::new();
        metrics.events_received.inc_by(1000);
        metrics.events_processed.inc_by(990);
        metrics.events_dropped_kernel.inc_by(10);

        let output = metrics.prometheus_export();
        assert!(output.contains("ebpf_events_received_total 1000"));
        assert!(output.contains("ebpf_events_processed_total 990"));
        assert!(output.contains("ebpf_events_dropped_kernel_total 10"));
    }

    #[test]
    fn test_drop_rate() {
        let metrics = EbpfMetrics::new();
        metrics.events_received.inc_by(1000);
        metrics.events_dropped_kernel.inc_by(50);
        metrics.events_dropped_userspace.inc_by(50);

        let drop_rate = metrics.drop_rate();
        assert!((drop_rate - 10.0).abs() < 0.001); // 10% drop rate
    }

    #[test]
    fn test_correlation_rate() {
        let metrics = EbpfMetrics::new();
        metrics.correlation_hits.inc_by(80);
        metrics.correlation_misses.inc_by(20);

        let rate = metrics.correlation_rate();
        assert!((rate - 80.0).abs() < 0.001); // 80% correlation rate
    }

    #[test]
    fn test_timer() {
        let timer = Timer::start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.stop();
        assert!(elapsed >= 10_000); // At least 10ms in microseconds
    }
}
