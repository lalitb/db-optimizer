//! Prometheus Metrics Export
//!
//! Exports eBPF and database metrics in Prometheus format:
//! - Query latency histograms
//! - I/O metrics (read/write bytes, operations)
//! - Lock contention metrics
//! - Container resource usage
//! - Health scores and recommendations

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Metric type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    /// Counter (monotonically increasing)
    Counter,
    /// Gauge (can go up or down)
    Gauge,
    /// Histogram (distribution of values)
    Histogram,
    /// Summary (quantiles)
    Summary,
}

/// A single metric value
#[derive(Debug, Clone)]
pub struct MetricValue {
    /// Metric name
    pub name: String,
    /// Metric type
    pub metric_type: MetricType,
    /// Help text
    pub help: String,
    /// Labels
    pub labels: HashMap<String, String>,
    /// Value (for counter/gauge)
    pub value: f64,
    /// Histogram buckets (for histogram)
    pub buckets: Option<Vec<(f64, u64)>>,
    /// Sum (for histogram/summary)
    pub sum: Option<f64>,
    /// Count (for histogram/summary)
    pub count: Option<u64>,
}

impl MetricValue {
    /// Create a new counter
    pub fn counter(name: &str, help: &str, value: f64) -> Self {
        Self {
            name: name.to_string(),
            metric_type: MetricType::Counter,
            help: help.to_string(),
            labels: HashMap::new(),
            value,
            buckets: None,
            sum: None,
            count: None,
        }
    }

    /// Create a new gauge
    pub fn gauge(name: &str, help: &str, value: f64) -> Self {
        Self {
            name: name.to_string(),
            metric_type: MetricType::Gauge,
            help: help.to_string(),
            labels: HashMap::new(),
            value,
            buckets: None,
            sum: None,
            count: None,
        }
    }

    /// Create a new histogram
    pub fn histogram(
        name: &str,
        help: &str,
        buckets: Vec<(f64, u64)>,
        sum: f64,
        count: u64,
    ) -> Self {
        Self {
            name: name.to_string(),
            metric_type: MetricType::Histogram,
            help: help.to_string(),
            labels: HashMap::new(),
            value: 0.0,
            buckets: Some(buckets),
            sum: Some(sum),
            count: Some(count),
        }
    }

    /// Add a label
    pub fn with_label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_string(), value.to_string());
        self
    }

    /// Format as Prometheus text
    pub fn to_prometheus(&self) -> String {
        let mut output = String::new();

        // HELP line
        output.push_str(&format!("# HELP {} {}\n", self.name, self.help));

        // TYPE line
        let type_str = match self.metric_type {
            MetricType::Counter => "counter",
            MetricType::Gauge => "gauge",
            MetricType::Histogram => "histogram",
            MetricType::Summary => "summary",
        };
        output.push_str(&format!("# TYPE {} {}\n", self.name, type_str));

        // Format labels
        let labels_str = if self.labels.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = self
                .labels
                .iter()
                .map(|(k, v)| format!("{}=\"{}\"", k, v.replace('"', "\\\"")))
                .collect();
            format!("{{{}}}", pairs.join(","))
        };

        match self.metric_type {
            MetricType::Counter | MetricType::Gauge => {
                output.push_str(&format!("{}{} {}\n", self.name, labels_str, self.value));
            }
            MetricType::Histogram => {
                if let Some(ref buckets) = self.buckets {
                    for (le, count) in buckets {
                        let bucket_labels = if self.labels.is_empty() {
                            format!("{{le=\"{}\"}}", le)
                        } else {
                            let pairs: Vec<String> = self
                                .labels
                                .iter()
                                .map(|(k, v)| format!("{}=\"{}\"", k, v))
                                .collect();
                            format!("{{le=\"{}\",{}}}", le, pairs.join(","))
                        };
                        output.push_str(&format!(
                            "{}_bucket{} {}\n",
                            self.name, bucket_labels, count
                        ));
                    }
                    // +Inf bucket
                    let inf_labels = if self.labels.is_empty() {
                        "{le=\"+Inf\"}".to_string()
                    } else {
                        let pairs: Vec<String> = self
                            .labels
                            .iter()
                            .map(|(k, v)| format!("{}=\"{}\"", k, v))
                            .collect();
                        format!("{{le=\"+Inf\",{}}}", pairs.join(","))
                    };
                    output.push_str(&format!(
                        "{}_bucket{} {}\n",
                        self.name,
                        inf_labels,
                        self.count.unwrap_or(0)
                    ));
                }
                output.push_str(&format!(
                    "{}_sum{} {}\n",
                    self.name,
                    labels_str,
                    self.sum.unwrap_or(0.0)
                ));
                output.push_str(&format!(
                    "{}_count{} {}\n",
                    self.name,
                    labels_str,
                    self.count.unwrap_or(0)
                ));
            }
            MetricType::Summary => {
                output.push_str(&format!(
                    "{}_sum{} {}\n",
                    self.name,
                    labels_str,
                    self.sum.unwrap_or(0.0)
                ));
                output.push_str(&format!(
                    "{}_count{} {}\n",
                    self.name,
                    labels_str,
                    self.count.unwrap_or(0)
                ));
            }
        }

        output
    }
}

/// Prometheus metrics registry
pub struct PrometheusRegistry {
    /// Metric prefix
    prefix: String,
    /// Database labels
    db_labels: Arc<RwLock<HashMap<String, String>>>,
    /// Cached metrics
    metrics: Arc<RwLock<Vec<MetricValue>>>,
    /// Last update time
    last_update: Arc<RwLock<Instant>>,
}

impl PrometheusRegistry {
    /// Create a new registry
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_string(),
            db_labels: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(Vec::new())),
            last_update: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Set database labels
    pub async fn set_db_labels(&self, labels: HashMap<String, String>) {
        *self.db_labels.write().await = labels;
    }

    /// Add a metric
    pub async fn add_metric(&self, mut metric: MetricValue) {
        // Apply prefix
        metric.name = format!("{}_{}", self.prefix, metric.name);

        // Apply default labels
        let db_labels = self.db_labels.read().await;
        for (k, v) in db_labels.iter() {
            metric.labels.entry(k.clone()).or_insert_with(|| v.clone());
        }

        self.metrics.write().await.push(metric);
    }

    /// Clear all metrics
    pub async fn clear(&self) {
        self.metrics.write().await.clear();
    }

    /// Export all metrics as Prometheus text format
    pub async fn export(&self) -> String {
        let metrics = self.metrics.read().await;
        let mut output = String::new();

        for metric in metrics.iter() {
            output.push_str(&metric.to_prometheus());
            output.push('\n');
        }

        output
    }

    /// Record eBPF query metrics
    pub async fn record_query_metrics(
        &self,
        query_hash: &str,
        latency_us: f64,
        io_time_us: f64,
        cpu_time_us: f64,
        lock_wait_us: f64,
    ) {
        // Latency histogram buckets (in microseconds)
        let buckets = vec![
            (100.0, 0),    // 100us
            (500.0, 0),    // 500us
            (1000.0, 0),   // 1ms
            (5000.0, 0),   // 5ms
            (10000.0, 0),  // 10ms
            (50000.0, 0),  // 50ms
            (100000.0, 0), // 100ms
            (500000.0, 0), // 500ms
        ];

        // Calculate bucket counts
        let mut filled_buckets = buckets.clone();
        for (le, count) in filled_buckets.iter_mut() {
            if latency_us <= *le {
                *count = 1;
            }
        }

        self.add_metric(
            MetricValue::histogram(
                "query_latency_us",
                "Query latency in microseconds",
                filled_buckets,
                latency_us,
                1,
            )
            .with_label("query_hash", query_hash),
        )
        .await;

        self.add_metric(
            MetricValue::gauge(
                "query_io_time_us",
                "Query I/O time in microseconds",
                io_time_us,
            )
            .with_label("query_hash", query_hash),
        )
        .await;

        self.add_metric(
            MetricValue::gauge(
                "query_cpu_time_us",
                "Query CPU time in microseconds",
                cpu_time_us,
            )
            .with_label("query_hash", query_hash),
        )
        .await;

        self.add_metric(
            MetricValue::gauge(
                "query_lock_wait_us",
                "Query lock wait time in microseconds",
                lock_wait_us,
            )
            .with_label("query_hash", query_hash),
        )
        .await;
    }

    /// Record I/O metrics
    pub async fn record_io_metrics(
        &self,
        read_bytes: u64,
        write_bytes: u64,
        read_ops: u64,
        write_ops: u64,
        fsync_ops: u64,
    ) {
        self.add_metric(MetricValue::counter(
            "io_read_bytes_total",
            "Total bytes read",
            read_bytes as f64,
        ))
        .await;

        self.add_metric(MetricValue::counter(
            "io_write_bytes_total",
            "Total bytes written",
            write_bytes as f64,
        ))
        .await;

        self.add_metric(MetricValue::counter(
            "io_read_ops_total",
            "Total read operations",
            read_ops as f64,
        ))
        .await;

        self.add_metric(MetricValue::counter(
            "io_write_ops_total",
            "Total write operations",
            write_ops as f64,
        ))
        .await;

        self.add_metric(MetricValue::counter(
            "io_fsync_ops_total",
            "Total fsync operations",
            fsync_ops as f64,
        ))
        .await;
    }

    /// Record container metrics
    pub async fn record_container_metrics(
        &self,
        container_id: &str,
        cpu_usage_pct: f64,
        memory_usage_bytes: u64,
        memory_limit_bytes: u64,
        throttle_pct: f64,
    ) {
        self.add_metric(
            MetricValue::gauge(
                "container_cpu_usage_percent",
                "Container CPU usage percentage",
                cpu_usage_pct,
            )
            .with_label("container_id", container_id),
        )
        .await;

        self.add_metric(
            MetricValue::gauge(
                "container_memory_usage_bytes",
                "Container memory usage in bytes",
                memory_usage_bytes as f64,
            )
            .with_label("container_id", container_id),
        )
        .await;

        self.add_metric(
            MetricValue::gauge(
                "container_memory_limit_bytes",
                "Container memory limit in bytes",
                memory_limit_bytes as f64,
            )
            .with_label("container_id", container_id),
        )
        .await;

        self.add_metric(
            MetricValue::gauge(
                "container_cpu_throttle_percent",
                "Container CPU throttle percentage",
                throttle_pct,
            )
            .with_label("container_id", container_id),
        )
        .await;
    }

    /// Record health score
    pub async fn record_health_score(&self, score: f64, database: &str) {
        self.add_metric(
            MetricValue::gauge("health_score", "Database health score (0-100)", score)
                .with_label("database", database),
        )
        .await;
    }

    /// Record recommendation metrics
    pub async fn record_recommendations(
        &self,
        total: u64,
        applied: u64,
        pending: u64,
        rejected: u64,
    ) {
        self.add_metric(MetricValue::gauge(
            "recommendations_total",
            "Total recommendations generated",
            total as f64,
        ))
        .await;

        self.add_metric(MetricValue::gauge(
            "recommendations_applied",
            "Recommendations successfully applied",
            applied as f64,
        ))
        .await;

        self.add_metric(MetricValue::gauge(
            "recommendations_pending",
            "Recommendations pending review",
            pending as f64,
        ))
        .await;

        self.add_metric(MetricValue::gauge(
            "recommendations_rejected",
            "Recommendations rejected or rolled back",
            rejected as f64,
        ))
        .await;
    }

    /// Record eBPF status
    pub async fn record_ebpf_status(
        &self,
        capability_level: &str,
        events_per_sec: f64,
        drop_rate: f64,
    ) {
        self.add_metric(MetricValue::gauge(
            "ebpf_capability_level",
            "eBPF capability level (0=disabled, 4=full)",
            match capability_level {
                "Full" => 4.0,
                "Limited" => 3.0,
                "DaemonOnly" => 2.0,
                "PgStatOnly" => 1.0,
                _ => 0.0,
            },
        ))
        .await;

        self.add_metric(MetricValue::gauge(
            "ebpf_events_per_second",
            "eBPF events processed per second",
            events_per_sec,
        ))
        .await;

        self.add_metric(MetricValue::gauge(
            "ebpf_drop_rate",
            "eBPF event drop rate (0-1)",
            drop_rate,
        ))
        .await;
    }

    /// Record cost savings
    pub async fn record_cost_savings(&self, monthly_savings_usd: f64, currency: &str) {
        self.add_metric(
            MetricValue::gauge(
                "cost_savings_monthly_usd",
                "Estimated monthly cost savings in USD",
                monthly_savings_usd,
            )
            .with_label("currency", currency),
        )
        .await;
    }
}

impl Default for PrometheusRegistry {
    fn default() -> Self {
        Self::new("vaanyze")
    }
}

/// HTTP handler for /metrics endpoint
pub async fn metrics_handler(registry: Arc<PrometheusRegistry>) -> String {
    registry.export().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_format() {
        let metric = MetricValue::counter("requests_total", "Total requests", 42.0)
            .with_label("method", "GET");

        let output = metric.to_prometheus();
        assert!(output.contains("# HELP requests_total Total requests"));
        assert!(output.contains("# TYPE requests_total counter"));
        assert!(output.contains("requests_total{method=\"GET\"} 42"));
    }

    #[test]
    fn test_gauge_format() {
        let metric = MetricValue::gauge("temperature", "Current temperature", 23.5);

        let output = metric.to_prometheus();
        assert!(output.contains("# TYPE temperature gauge"));
        assert!(output.contains("temperature 23.5"));
    }

    #[test]
    fn test_histogram_format() {
        let buckets = vec![(100.0, 5), (500.0, 10), (1000.0, 15)];
        let metric = MetricValue::histogram("latency", "Request latency", buckets, 5000.0, 15);

        let output = metric.to_prometheus();
        assert!(output.contains("# TYPE latency histogram"));
        assert!(output.contains("latency_bucket{le=\"100\"} 5"));
        assert!(output.contains("latency_bucket{le=\"500\"} 10"));
        assert!(output.contains("latency_sum 5000"));
        assert!(output.contains("latency_count 15"));
    }

    #[tokio::test]
    async fn test_registry_export() {
        let registry = PrometheusRegistry::new("test");

        registry
            .add_metric(MetricValue::counter("requests", "Total requests", 100.0))
            .await;
        registry
            .add_metric(MetricValue::gauge("connections", "Active connections", 5.0))
            .await;

        let output = registry.export().await;
        assert!(output.contains("test_requests"));
        assert!(output.contains("test_connections"));
    }

    #[tokio::test]
    async fn test_record_query_metrics() {
        let registry = PrometheusRegistry::new("vaanyze");

        registry
            .record_query_metrics("abc123", 1500.0, 800.0, 600.0, 100.0)
            .await;

        let output = registry.export().await;
        assert!(output.contains("vaanyze_query_latency_us"));
        assert!(output.contains("query_hash=\"abc123\""));
    }
}
