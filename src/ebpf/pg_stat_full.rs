//! Full pg_stat Integration
//!
//! Comprehensive correlation between eBPF metrics and PostgreSQL statistics:
//! - pg_stat_statements correlation
//! - pg_stat_activity integration
//! - pg_stat_user_tables and pg_stat_user_indexes
//! - pg_stat_bgwriter and checkpoint stats
//! - Lock and wait event correlation

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Query statistics from pg_stat_statements
#[derive(Debug, Clone)]
pub struct PgStatStatement {
    /// Query ID (pg 14+ native, or md5 hash)
    pub queryid: i64,
    /// Database OID
    pub dbid: u32,
    /// User OID
    pub userid: u32,
    /// Normalized query text
    pub query: String,
    /// Number of times executed
    pub calls: i64,
    /// Total time spent (seconds)
    pub total_exec_time: f64,
    /// Minimum execution time
    pub min_exec_time: f64,
    /// Maximum execution time
    pub max_exec_time: f64,
    /// Mean execution time
    pub mean_exec_time: f64,
    /// Standard deviation
    pub stddev_exec_time: f64,
    /// Rows returned/affected
    pub rows: i64,
    /// Shared blocks hit
    pub shared_blks_hit: i64,
    /// Shared blocks read
    pub shared_blks_read: i64,
    /// Shared blocks dirtied
    pub shared_blks_dirtied: i64,
    /// Shared blocks written
    pub shared_blks_written: i64,
    /// Local blocks hit
    pub local_blks_hit: i64,
    /// Local blocks read
    pub local_blks_read: i64,
    /// Temp blocks read
    pub temp_blks_read: i64,
    /// Temp blocks written
    pub temp_blks_written: i64,
    /// Block I/O time (seconds) for reads
    pub blk_read_time: f64,
    /// Block I/O time (seconds) for writes
    pub blk_write_time: f64,
    /// WAL bytes generated (pg 13+)
    pub wal_bytes: Option<i64>,
    /// WAL full page images (pg 13+)
    pub wal_fpi: Option<i64>,
    /// WAL records (pg 13+)
    pub wal_records: Option<i64>,
    /// JIT generation time (pg 15+)
    pub jit_generation_time: Option<f64>,
    /// JIT inlining time (pg 15+)
    pub jit_inlining_time: Option<f64>,
    /// JIT optimization time (pg 15+)
    pub jit_optimization_time: Option<f64>,
    /// JIT emission time (pg 15+)
    pub jit_emission_time: Option<f64>,
}

/// Current session activity from pg_stat_activity
#[derive(Debug, Clone)]
pub struct PgStatActivity {
    /// Process ID
    pub pid: i32,
    /// Database OID
    pub datid: u32,
    /// Database name
    pub datname: String,
    /// User OID
    pub usesysid: u32,
    /// User name
    pub usename: String,
    /// Application name
    pub application_name: String,
    /// Client address
    pub client_addr: Option<String>,
    /// Client port
    pub client_port: Option<i32>,
    /// Backend start time
    pub backend_start: Option<String>,
    /// Transaction start time
    pub xact_start: Option<String>,
    /// Query start time
    pub query_start: Option<String>,
    /// State change time
    pub state_change: Option<String>,
    /// Wait event type
    pub wait_event_type: Option<String>,
    /// Wait event
    pub wait_event: Option<String>,
    /// State (active, idle, etc.)
    pub state: String,
    /// Backend type (client backend, autovacuum worker, etc.)
    pub backend_type: String,
    /// Current query
    pub query: String,
    /// Query ID (pg 14+)
    pub query_id: Option<i64>,
}

/// Table statistics from pg_stat_user_tables
#[derive(Debug, Clone)]
pub struct PgStatTable {
    /// Relation OID
    pub relid: u32,
    /// Schema name
    pub schemaname: String,
    /// Table name
    pub relname: String,
    /// Sequential scans
    pub seq_scan: i64,
    /// Rows from sequential scans
    pub seq_tup_read: i64,
    /// Index scans
    pub idx_scan: i64,
    /// Rows from index scans
    pub idx_tup_fetch: i64,
    /// Rows inserted
    pub n_tup_ins: i64,
    /// Rows updated
    pub n_tup_upd: i64,
    /// Rows deleted
    pub n_tup_del: i64,
    /// Hot updates
    pub n_tup_hot_upd: i64,
    /// Live tuples
    pub n_live_tup: i64,
    /// Dead tuples
    pub n_dead_tup: i64,
    /// Last vacuum time
    pub last_vacuum: Option<String>,
    /// Last autovacuum time
    pub last_autovacuum: Option<String>,
    /// Last analyze time
    pub last_analyze: Option<String>,
    /// Last autoanalyze time
    pub last_autoanalyze: Option<String>,
    /// Vacuum count
    pub vacuum_count: i64,
    /// Autovacuum count
    pub autovacuum_count: i64,
}

/// Index statistics from pg_stat_user_indexes
#[derive(Debug, Clone)]
pub struct PgStatIndex {
    /// Relation OID (table)
    pub relid: u32,
    /// Index OID
    pub indexrelid: u32,
    /// Schema name
    pub schemaname: String,
    /// Table name
    pub relname: String,
    /// Index name
    pub indexrelname: String,
    /// Index scans
    pub idx_scan: i64,
    /// Tuples read
    pub idx_tup_read: i64,
    /// Tuples fetched
    pub idx_tup_fetch: i64,
}

/// Background writer statistics
#[derive(Debug, Clone)]
pub struct PgStatBgWriter {
    /// Buffers written by checkpointer
    pub checkpoints_timed: i64,
    /// Requested checkpoints
    pub checkpoints_req: i64,
    /// Checkpoint write time (ms)
    pub checkpoint_write_time: f64,
    /// Checkpoint sync time (ms)
    pub checkpoint_sync_time: f64,
    /// Buffers written by bgwriter
    pub buffers_checkpoint: i64,
    /// Buffers cleaned by bgwriter
    pub buffers_clean: i64,
    /// Max written clean
    pub maxwritten_clean: i64,
    /// Buffers allocated
    pub buffers_alloc: i64,
    /// Stats reset time
    pub stats_reset: Option<String>,
}

/// Correlated query metrics (eBPF + pg_stat)
#[derive(Debug, Clone)]
pub struct CorrelatedQueryMetrics {
    /// pg_stat_statements data
    pub pg_stat: PgStatStatement,
    /// eBPF I/O latency (microseconds)
    pub ebpf_io_latency_p50_us: Option<f64>,
    pub ebpf_io_latency_p95_us: Option<f64>,
    pub ebpf_io_latency_p99_us: Option<f64>,
    /// eBPF observed I/O operations
    pub ebpf_read_ops: u64,
    pub ebpf_write_ops: u64,
    /// eBPF observed lock wait time
    pub ebpf_lock_wait_us: Option<f64>,
    /// Correlation confidence
    pub confidence: f64,
    /// Data source summary
    pub sources: Vec<String>,
}

/// pg_stat collector configuration
#[derive(Debug, Clone)]
pub struct PgStatConfig {
    /// How often to poll pg_stat views
    pub poll_interval: Duration,
    /// Whether to collect pg_stat_statements
    pub collect_statements: bool,
    /// Whether to collect pg_stat_activity
    pub collect_activity: bool,
    /// Whether to collect table stats
    pub collect_tables: bool,
    /// Whether to collect index stats
    pub collect_indexes: bool,
    /// Whether to collect bgwriter stats
    pub collect_bgwriter: bool,
    /// Maximum statements to track
    pub max_statements: usize,
    /// Minimum calls to include
    pub min_calls: i64,
}

impl Default for PgStatConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(10),
            collect_statements: true,
            collect_activity: true,
            collect_tables: true,
            collect_indexes: true,
            collect_bgwriter: true,
            max_statements: 1000,
            min_calls: 5,
        }
    }
}

/// Full pg_stat integrator
pub struct PgStatIntegrator {
    /// Configuration
    config: Arc<RwLock<PgStatConfig>>,
    /// Cached statements
    statements: Arc<RwLock<HashMap<i64, PgStatStatement>>>,
    /// Cached activity
    activity: Arc<RwLock<Vec<PgStatActivity>>>,
    /// Cached table stats
    tables: Arc<RwLock<HashMap<u32, PgStatTable>>>,
    /// Cached index stats
    indexes: Arc<RwLock<HashMap<u32, PgStatIndex>>>,
    /// Cached bgwriter stats
    bgwriter: Arc<RwLock<Option<PgStatBgWriter>>>,
    /// eBPF I/O metrics by query id
    ebpf_io_metrics: Arc<RwLock<HashMap<i64, EbpfQueryMetrics>>>,
    /// Last poll time
    last_poll: Arc<RwLock<Instant>>,
    /// PostgreSQL version
    pg_version: Arc<RwLock<u32>>,
}

/// eBPF metrics for a query
#[derive(Debug, Clone, Default)]
pub struct EbpfQueryMetrics {
    /// Sample count
    pub samples: u64,
    /// Total I/O latency (us)
    pub total_io_latency_us: u64,
    /// Min I/O latency (us)
    pub min_io_latency_us: u64,
    /// Max I/O latency (us)
    pub max_io_latency_us: u64,
    /// Read operations
    pub read_ops: u64,
    /// Write operations
    pub write_ops: u64,
    /// Lock wait time (us)
    pub lock_wait_us: u64,
    /// I/O latency samples for percentile calculation
    pub latency_samples: Vec<u64>,
}

impl PgStatIntegrator {
    /// Create a new integrator
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(PgStatConfig::default())),
            statements: Arc::new(RwLock::new(HashMap::new())),
            activity: Arc::new(RwLock::new(Vec::new())),
            tables: Arc::new(RwLock::new(HashMap::new())),
            indexes: Arc::new(RwLock::new(HashMap::new())),
            bgwriter: Arc::new(RwLock::new(None)),
            ebpf_io_metrics: Arc::new(RwLock::new(HashMap::new())),
            last_poll: Arc::new(RwLock::new(Instant::now())),
            pg_version: Arc::new(RwLock::new(0)),
        }
    }

    /// Set PostgreSQL version
    pub async fn set_pg_version(&self, version: u32) {
        *self.pg_version.write().await = version;
    }

    /// Ingest pg_stat_statements data
    pub async fn ingest_statements(&self, statements: Vec<PgStatStatement>) {
        let config = self.config.read().await;
        let min_calls = config.min_calls;
        let max_statements = config.max_statements;
        drop(config);

        let mut cache = self.statements.write().await;

        // Filter and limit
        let filtered: Vec<_> = statements
            .into_iter()
            .filter(|s| s.calls >= min_calls)
            .collect();

        cache.clear();
        for stmt in filtered.into_iter().take(max_statements) {
            cache.insert(stmt.queryid, stmt);
        }
    }

    /// Ingest pg_stat_activity data
    pub async fn ingest_activity(&self, activity: Vec<PgStatActivity>) {
        *self.activity.write().await = activity;
    }

    /// Ingest table stats
    pub async fn ingest_tables(&self, tables: Vec<PgStatTable>) {
        let mut cache = self.tables.write().await;
        cache.clear();
        for table in tables {
            cache.insert(table.relid, table);
        }
    }

    /// Ingest index stats
    pub async fn ingest_indexes(&self, indexes: Vec<PgStatIndex>) {
        let mut cache = self.indexes.write().await;
        cache.clear();
        for idx in indexes {
            cache.insert(idx.indexrelid, idx);
        }
    }

    /// Ingest bgwriter stats
    pub async fn ingest_bgwriter(&self, stats: PgStatBgWriter) {
        *self.bgwriter.write().await = Some(stats);
    }

    /// Record eBPF I/O event for a query
    pub async fn record_ebpf_io(&self, query_id: i64, latency_us: u64, is_write: bool) {
        let mut metrics = self.ebpf_io_metrics.write().await;

        let entry = metrics
            .entry(query_id)
            .or_insert_with(EbpfQueryMetrics::default);
        entry.samples += 1;
        entry.total_io_latency_us += latency_us;
        entry.min_io_latency_us = entry.min_io_latency_us.min(latency_us);
        if entry.min_io_latency_us == 0 {
            entry.min_io_latency_us = latency_us;
        }
        entry.max_io_latency_us = entry.max_io_latency_us.max(latency_us);

        if is_write {
            entry.write_ops += 1;
        } else {
            entry.read_ops += 1;
        }

        // Keep samples for percentile calculation (with reservoir sampling)
        const MAX_SAMPLES: usize = 1000;
        if entry.latency_samples.len() < MAX_SAMPLES {
            entry.latency_samples.push(latency_us);
        } else {
            // Reservoir sampling
            let idx = (rand_u64() as usize) % entry.samples as usize;
            if idx < MAX_SAMPLES {
                entry.latency_samples[idx] = latency_us;
            }
        }
    }

    /// Record eBPF lock event for a query
    pub async fn record_ebpf_lock(&self, query_id: i64, wait_us: u64) {
        let mut metrics = self.ebpf_io_metrics.write().await;
        let entry = metrics
            .entry(query_id)
            .or_insert_with(EbpfQueryMetrics::default);
        entry.lock_wait_us += wait_us;
    }

    /// Get correlated metrics for a query
    pub async fn get_correlated_metrics(&self, query_id: i64) -> Option<CorrelatedQueryMetrics> {
        let statements = self.statements.read().await;
        let pg_stat = statements.get(&query_id)?.clone();
        drop(statements);

        let ebpf_metrics = self.ebpf_io_metrics.read().await;
        let ebpf = ebpf_metrics.get(&query_id);

        let (p50, p95, p99) = if let Some(m) = ebpf {
            let mut samples = m.latency_samples.clone();
            samples.sort_unstable();
            let len = samples.len();
            if len > 0 {
                (
                    Some(samples[len * 50 / 100] as f64),
                    Some(samples[len * 95 / 100] as f64),
                    Some(
                        samples
                            .get(len * 99 / 100)
                            .copied()
                            .unwrap_or(samples[len - 1]) as f64,
                    ),
                )
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        let confidence = if ebpf.is_some() { 0.9 } else { 0.5 };

        let mut sources = vec!["pg_stat_statements".to_string()];
        if ebpf.is_some() {
            sources.push("ebpf".to_string());
        }

        Some(CorrelatedQueryMetrics {
            pg_stat,
            ebpf_io_latency_p50_us: p50,
            ebpf_io_latency_p95_us: p95,
            ebpf_io_latency_p99_us: p99,
            ebpf_read_ops: ebpf.map(|m| m.read_ops).unwrap_or(0),
            ebpf_write_ops: ebpf.map(|m| m.write_ops).unwrap_or(0),
            ebpf_lock_wait_us: ebpf.map(|m| m.lock_wait_us as f64),
            confidence,
            sources,
        })
    }

    /// Get top queries by total time
    pub async fn get_top_queries_by_time(&self, limit: usize) -> Vec<CorrelatedQueryMetrics> {
        let statements = self.statements.read().await;
        let mut sorted: Vec<_> = statements.values().cloned().collect();
        sorted.sort_by(|a, b| {
            b.total_exec_time
                .partial_cmp(&a.total_exec_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        drop(statements);

        let mut results = Vec::new();
        for stmt in sorted.into_iter().take(limit) {
            if let Some(correlated) = self.get_correlated_metrics(stmt.queryid).await {
                results.push(correlated);
            }
        }
        results
    }

    /// Get queries with high I/O
    pub async fn get_high_io_queries(&self, limit: usize) -> Vec<CorrelatedQueryMetrics> {
        let statements = self.statements.read().await;
        let mut sorted: Vec<_> = statements.values().cloned().collect();
        sorted.sort_by(|a, b| {
            let a_io = a.shared_blks_read + a.shared_blks_written;
            let b_io = b.shared_blks_read + b.shared_blks_written;
            b_io.cmp(&a_io)
        });

        drop(statements);

        let mut results = Vec::new();
        for stmt in sorted.into_iter().take(limit) {
            if let Some(correlated) = self.get_correlated_metrics(stmt.queryid).await {
                results.push(correlated);
            }
        }
        results
    }

    /// Get active sessions with wait events
    pub async fn get_waiting_sessions(&self) -> Vec<PgStatActivity> {
        let activity = self.activity.read().await;
        activity
            .iter()
            .filter(|a| a.wait_event_type.is_some())
            .cloned()
            .collect()
    }

    /// Get tables needing vacuum
    pub async fn get_tables_needing_vacuum(&self, threshold: i64) -> Vec<PgStatTable> {
        let tables = self.tables.read().await;
        tables
            .values()
            .filter(|t| t.n_dead_tup > threshold)
            .cloned()
            .collect()
    }

    /// Get unused indexes (never scanned)
    pub async fn get_unused_indexes(&self) -> Vec<PgStatIndex> {
        let indexes = self.indexes.read().await;
        indexes
            .values()
            .filter(|i| i.idx_scan == 0)
            .cloned()
            .collect()
    }

    /// Get checkpoint statistics
    pub async fn get_checkpoint_stats(&self) -> Option<PgStatBgWriter> {
        self.bgwriter.read().await.clone()
    }

    /// Get correlation summary
    pub async fn get_correlation_summary(&self) -> CorrelationSummary {
        let statements = self.statements.read().await;
        let ebpf_metrics = self.ebpf_io_metrics.read().await;

        let total_queries = statements.len();
        let queries_with_ebpf = statements
            .keys()
            .filter(|id| ebpf_metrics.contains_key(*id))
            .count();

        let coverage = if total_queries > 0 {
            queries_with_ebpf as f64 / total_queries as f64
        } else {
            0.0
        };

        CorrelationSummary {
            total_queries,
            queries_with_ebpf,
            coverage_ratio: coverage,
            ebpf_samples: ebpf_metrics.values().map(|m| m.samples).sum(),
        }
    }

    /// Clear eBPF metrics (for periodic reset)
    pub async fn clear_ebpf_metrics(&self) {
        self.ebpf_io_metrics.write().await.clear();
    }
}

impl Default for PgStatIntegrator {
    fn default() -> Self {
        Self::new()
    }
}

/// Correlation summary statistics
#[derive(Debug, Clone)]
pub struct CorrelationSummary {
    /// Total queries tracked
    pub total_queries: usize,
    /// Queries with eBPF data
    pub queries_with_ebpf: usize,
    /// Coverage ratio
    pub coverage_ratio: f64,
    /// Total eBPF samples
    pub ebpf_samples: u64,
}

/// Simple pseudo-random number generator for reservoir sampling
fn rand_u64() -> u64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    (nanos as u64)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_statement(queryid: i64, calls: i64, total_time: f64) -> PgStatStatement {
        PgStatStatement {
            queryid,
            dbid: 1,
            userid: 1,
            query: format!("SELECT * FROM test WHERE id = $1 -- qid:{}", queryid),
            calls,
            total_exec_time: total_time,
            min_exec_time: total_time / (calls as f64),
            max_exec_time: total_time / (calls as f64),
            mean_exec_time: total_time / (calls as f64),
            stddev_exec_time: 0.0,
            rows: calls * 10,
            shared_blks_hit: calls * 100,
            shared_blks_read: calls * 10,
            shared_blks_dirtied: 0,
            shared_blks_written: 0,
            local_blks_hit: 0,
            local_blks_read: 0,
            temp_blks_read: 0,
            temp_blks_written: 0,
            blk_read_time: 0.0,
            blk_write_time: 0.0,
            wal_bytes: None,
            wal_fpi: None,
            wal_records: None,
            jit_generation_time: None,
            jit_inlining_time: None,
            jit_optimization_time: None,
            jit_emission_time: None,
        }
    }

    #[tokio::test]
    async fn test_statement_ingestion() {
        let integrator = PgStatIntegrator::new();

        let statements = vec![
            make_test_statement(1, 100, 10.0),
            make_test_statement(2, 50, 20.0),
            make_test_statement(3, 2, 5.0), // Below min_calls, should be filtered
        ];

        integrator.ingest_statements(statements).await;

        let cached = integrator.statements.read().await;
        assert_eq!(cached.len(), 2); // Only 2 meet min_calls
    }

    #[tokio::test]
    async fn test_ebpf_io_recording() {
        let integrator = PgStatIntegrator::new();

        // Record some I/O events
        integrator.record_ebpf_io(1, 100, false).await;
        integrator.record_ebpf_io(1, 200, false).await;
        integrator.record_ebpf_io(1, 150, true).await;

        let metrics = integrator.ebpf_io_metrics.read().await;
        let m = metrics.get(&1).unwrap();

        assert_eq!(m.samples, 3);
        assert_eq!(m.read_ops, 2);
        assert_eq!(m.write_ops, 1);
        assert_eq!(m.min_io_latency_us, 100);
        assert_eq!(m.max_io_latency_us, 200);
    }

    #[tokio::test]
    async fn test_correlated_metrics() {
        let integrator = PgStatIntegrator::new();

        // Ingest statement
        let stmt = make_test_statement(1, 100, 10.0);
        integrator.ingest_statements(vec![stmt]).await;

        // Record eBPF data
        for i in 0..100 {
            integrator.record_ebpf_io(1, 100 + i * 10, false).await;
        }

        // Get correlated metrics
        let correlated = integrator.get_correlated_metrics(1).await.unwrap();

        assert_eq!(correlated.pg_stat.calls, 100);
        assert!(correlated.ebpf_io_latency_p50_us.is_some());
        assert!(correlated.confidence > 0.8);
        assert!(correlated.sources.contains(&"ebpf".to_string()));
    }

    #[tokio::test]
    async fn test_top_queries() {
        let integrator = PgStatIntegrator::new();

        let statements = vec![
            make_test_statement(1, 100, 10.0),
            make_test_statement(2, 50, 100.0), // Most time
            make_test_statement(3, 200, 5.0),
        ];

        integrator.ingest_statements(statements).await;

        let top = integrator.get_top_queries_by_time(2).await;
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].pg_stat.queryid, 2); // Highest total time
    }

    #[tokio::test]
    async fn test_correlation_summary() {
        let integrator = PgStatIntegrator::new();

        let statements = vec![
            make_test_statement(1, 100, 10.0),
            make_test_statement(2, 50, 20.0),
        ];
        integrator.ingest_statements(statements).await;

        // Only add eBPF data for one query
        integrator.record_ebpf_io(1, 100, false).await;

        let summary = integrator.get_correlation_summary().await;
        assert_eq!(summary.total_queries, 2);
        assert_eq!(summary.queries_with_ebpf, 1);
        assert!((summary.coverage_ratio - 0.5).abs() < 0.01);
    }
}
