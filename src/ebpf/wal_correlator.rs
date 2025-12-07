//! WAL Write and I/O Correlation
//!
//! Correlates eBPF I/O events with PostgreSQL WAL writes and checkpoint activity.
//! Distinguishes between different types of I/O: WAL, data files, temp files, etc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, trace};

/// I/O type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IoType {
    /// WAL segment writes
    Wal,
    /// Data file reads/writes
    DataFile,
    /// Temporary file I/O (sorts, hash joins)
    TempFile,
    /// CLOG/commit log
    Clog,
    /// Subtransaction log
    Subtrans,
    /// Multixact
    Multixact,
    /// pg_stat files
    Stats,
    /// Relation map files
    RelMap,
    /// Control file
    Control,
    /// Configuration files
    Config,
    /// Unknown/other
    Other,
}

impl IoType {
    /// Classify a file path by I/O type
    pub fn from_path(path: &str) -> Self {
        let path_lower = path.to_lowercase();

        // WAL files
        if path_lower.contains("/pg_wal/") || path_lower.contains("/pg_xlog/") {
            return IoType::Wal;
        }

        // Temporary files
        if path_lower.contains("/pgsql_tmp/") || path_lower.contains("/pg_tmp/") {
            return IoType::TempFile;
        }

        // CLOG
        if path_lower.contains("/pg_clog/") || path_lower.contains("/pg_xact/") {
            return IoType::Clog;
        }

        // Subtransaction
        if path_lower.contains("/pg_subtrans/") {
            return IoType::Subtrans;
        }

        // Multixact
        if path_lower.contains("/pg_multixact/") {
            return IoType::Multixact;
        }

        // Stats
        if path_lower.contains("/pg_stat/") || path_lower.contains("/pg_stat_tmp/") {
            return IoType::Stats;
        }

        // Relation map
        if path_lower.ends_with("/pg_filenode.map") {
            return IoType::RelMap;
        }

        // Control file
        if path_lower.ends_with("/global/pg_control") {
            return IoType::Control;
        }

        // Config files
        if path_lower.ends_with(".conf") {
            return IoType::Config;
        }

        // Data files (base directory)
        if path_lower.contains("/base/") || path_lower.contains("/global/") {
            return IoType::DataFile;
        }

        IoType::Other
    }

    /// Whether this I/O type should be attributed to queries
    pub fn is_query_attributable(&self) -> bool {
        matches!(self, IoType::DataFile | IoType::TempFile)
    }

    /// Whether this is background/system I/O
    pub fn is_background(&self) -> bool {
        matches!(
            self,
            IoType::Wal | IoType::Clog | IoType::Stats | IoType::Control
        )
    }
}

/// I/O event with classification
#[derive(Debug, Clone)]
pub struct ClassifiedIoEvent {
    /// Process ID
    pub pid: u32,
    /// File descriptor
    pub fd: i32,
    /// File path (if resolved)
    pub path: Option<String>,
    /// I/O type
    pub io_type: IoType,
    /// Read or write
    pub is_write: bool,
    /// Bytes transferred
    pub bytes: u64,
    /// Latency in microseconds
    pub latency_us: u64,
    /// Timestamp
    pub timestamp: Instant,
}

/// WAL write tracking
#[derive(Debug, Clone)]
pub struct WalWriteInfo {
    /// Total bytes written to WAL
    pub bytes_written: u64,
    /// Number of WAL writes
    pub write_count: u64,
    /// Total WAL write latency (microseconds)
    pub total_latency_us: u64,
    /// Last WAL segment
    pub last_segment: Option<String>,
    /// Last write time
    pub last_write: Instant,
}

impl Default for WalWriteInfo {
    fn default() -> Self {
        Self {
            bytes_written: 0,
            write_count: 0,
            total_latency_us: 0,
            last_segment: None,
            last_write: Instant::now(),
        }
    }
}

/// Checkpoint tracking
#[derive(Debug, Clone)]
pub struct CheckpointInfo {
    /// Whether a checkpoint is currently in progress
    pub in_progress: bool,
    /// Start time of current checkpoint
    pub start_time: Option<Instant>,
    /// I/O attributed to current checkpoint
    pub checkpoint_io_bytes: u64,
    /// Number of checkpoints completed
    pub completed_count: u64,
    /// Last checkpoint duration
    pub last_duration: Option<Duration>,
    /// Average checkpoint duration
    pub avg_duration_ms: f64,
}

impl Default for CheckpointInfo {
    fn default() -> Self {
        Self {
            in_progress: false,
            start_time: None,
            checkpoint_io_bytes: 0,
            completed_count: 0,
            last_duration: None,
            avg_duration_ms: 0.0,
        }
    }
}

/// I/O statistics by type
#[derive(Debug, Clone, Default)]
pub struct IoStats {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_count: u64,
    pub write_count: u64,
    pub total_read_latency_us: u64,
    pub total_write_latency_us: u64,
}

impl IoStats {
    pub fn avg_read_latency_us(&self) -> f64 {
        if self.read_count == 0 {
            0.0
        } else {
            self.total_read_latency_us as f64 / self.read_count as f64
        }
    }

    pub fn avg_write_latency_us(&self) -> f64 {
        if self.write_count == 0 {
            0.0
        } else {
            self.total_write_latency_us as f64 / self.write_count as f64
        }
    }
}

/// File descriptor to path mapping
struct FdResolver {
    /// Cached fd -> path mappings by pid
    fd_cache: HashMap<(u32, i32), String>,
    /// Maximum cache size
    #[allow(dead_code)] // Used for cache eviction
    max_cache_size: usize,
}

impl FdResolver {
    fn new(max_cache_size: usize) -> Self {
        Self {
            fd_cache: HashMap::with_capacity(max_cache_size),
            max_cache_size,
        }
    }

    /// Resolve fd to path (reads from /proc on Linux)
    #[cfg(target_os = "linux")]
    fn resolve(&mut self, pid: u32, fd: i32) -> Option<String> {
        // Check cache first
        if let Some(path) = self.fd_cache.get(&(pid, fd)) {
            return Some(path.clone());
        }

        // Read from /proc
        let link_path = format!("/proc/{}/fd/{}", pid, fd);
        match std::fs::read_link(&link_path) {
            Ok(target) => {
                let path_str = target.to_string_lossy().to_string();

                // Cache the result
                if self.fd_cache.len() >= self.max_cache_size {
                    // Simple eviction: clear half the cache
                    let to_remove: Vec<_> = self
                        .fd_cache
                        .keys()
                        .take(self.max_cache_size / 2)
                        .cloned()
                        .collect();
                    for key in to_remove {
                        self.fd_cache.remove(&key);
                    }
                }

                self.fd_cache.insert((pid, fd), path_str.clone());
                Some(path_str)
            }
            Err(_) => None,
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn resolve(&mut self, _pid: u32, _fd: i32) -> Option<String> {
        // FD resolution only works on Linux
        None
    }

    /// Clear cache for a specific PID (e.g., when process exits)
    fn clear_pid(&mut self, pid: u32) {
        self.fd_cache.retain(|(p, _), _| *p != pid);
    }
}

/// WAL and I/O correlator
pub struct WalIoCorrelator {
    /// I/O statistics by type
    io_stats: Arc<RwLock<HashMap<IoType, IoStats>>>,
    /// WAL write tracking
    wal_info: Arc<RwLock<WalWriteInfo>>,
    /// Checkpoint tracking
    checkpoint_info: Arc<RwLock<CheckpointInfo>>,
    /// Per-query I/O attribution
    query_io: Arc<RwLock<HashMap<u64, IoStats>>>,
    /// File descriptor resolver
    fd_resolver: Arc<RwLock<FdResolver>>,
    /// PostgreSQL data directory
    pg_data_dir: Option<PathBuf>,
    /// Checkpointer PID (if known)
    checkpointer_pid: Arc<RwLock<Option<u32>>>,
    /// WAL writer PID (if known)
    wal_writer_pid: Arc<RwLock<Option<u32>>>,
}

impl WalIoCorrelator {
    /// Create a new correlator
    pub fn new() -> Self {
        Self {
            io_stats: Arc::new(RwLock::new(HashMap::new())),
            wal_info: Arc::new(RwLock::new(WalWriteInfo::default())),
            checkpoint_info: Arc::new(RwLock::new(CheckpointInfo::default())),
            query_io: Arc::new(RwLock::new(HashMap::new())),
            fd_resolver: Arc::new(RwLock::new(FdResolver::new(10_000))),
            pg_data_dir: None,
            checkpointer_pid: Arc::new(RwLock::new(None)),
            wal_writer_pid: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with known PostgreSQL data directory
    pub fn with_data_dir(pg_data_dir: PathBuf) -> Self {
        let mut correlator = Self::new();
        correlator.pg_data_dir = Some(pg_data_dir);
        correlator
    }

    /// Set known background process PIDs
    pub async fn set_background_pids(&self, checkpointer: Option<u32>, wal_writer: Option<u32>) {
        *self.checkpointer_pid.write().await = checkpointer;
        *self.wal_writer_pid.write().await = wal_writer;
    }

    /// Process an I/O event
    pub async fn process_io_event(
        &self,
        pid: u32,
        fd: i32,
        is_write: bool,
        bytes: u64,
        latency_us: u64,
        query_hash: Option<u64>,
    ) -> ClassifiedIoEvent {
        // Resolve file path
        let path = {
            let mut resolver = self.fd_resolver.write().await;
            resolver.resolve(pid, fd)
        };

        // Classify I/O type
        let io_type = path
            .as_ref()
            .map(|p| IoType::from_path(p))
            .unwrap_or(IoType::Other);

        // Create classified event
        let event = ClassifiedIoEvent {
            pid,
            fd,
            path: path.clone(),
            io_type,
            is_write,
            bytes,
            latency_us,
            timestamp: Instant::now(),
        };

        // Update statistics by type
        {
            let mut stats = self.io_stats.write().await;
            let type_stats = stats.entry(io_type).or_insert_with(IoStats::default);

            if is_write {
                type_stats.write_bytes += bytes;
                type_stats.write_count += 1;
                type_stats.total_write_latency_us += latency_us;
            } else {
                type_stats.read_bytes += bytes;
                type_stats.read_count += 1;
                type_stats.total_read_latency_us += latency_us;
            }
        }

        // Track WAL writes specifically
        if io_type == IoType::Wal && is_write {
            let mut wal = self.wal_info.write().await;
            wal.bytes_written += bytes;
            wal.write_count += 1;
            wal.total_latency_us += latency_us;
            wal.last_write = Instant::now();

            if let Some(p) = &path {
                // Extract WAL segment name
                if let Some(segment) = p.rsplit('/').next() {
                    wal.last_segment = Some(segment.to_string());
                }
            }
        }

        // Track checkpoint I/O
        {
            let checkpointer = self.checkpointer_pid.read().await;
            if Some(pid) == *checkpointer {
                let mut checkpoint = self.checkpoint_info.write().await;
                if is_write {
                    checkpoint.checkpoint_io_bytes += bytes;
                }
            }
        }

        // Attribute to query if applicable
        if let Some(hash) = query_hash {
            if io_type.is_query_attributable() {
                let mut query_io = self.query_io.write().await;
                let stats = query_io.entry(hash).or_insert_with(IoStats::default);

                if is_write {
                    stats.write_bytes += bytes;
                    stats.write_count += 1;
                    stats.total_write_latency_us += latency_us;
                } else {
                    stats.read_bytes += bytes;
                    stats.read_count += 1;
                    stats.total_read_latency_us += latency_us;
                }
            }
        }

        trace!(
            pid = pid,
            io_type = ?io_type,
            is_write = is_write,
            bytes = bytes,
            latency_us = latency_us,
            "Processed I/O event"
        );

        event
    }

    /// Record checkpoint start
    pub async fn checkpoint_start(&self) {
        let mut checkpoint = self.checkpoint_info.write().await;
        checkpoint.in_progress = true;
        checkpoint.start_time = Some(Instant::now());
        checkpoint.checkpoint_io_bytes = 0;

        debug!("Checkpoint started");
    }

    /// Record checkpoint end
    pub async fn checkpoint_end(&self) {
        let mut checkpoint = self.checkpoint_info.write().await;

        if let Some(start) = checkpoint.start_time {
            let duration = start.elapsed();
            checkpoint.last_duration = Some(duration);
            checkpoint.completed_count += 1;

            // Update average
            let n = checkpoint.completed_count as f64;
            let old_avg = checkpoint.avg_duration_ms;
            let new_sample = duration.as_secs_f64() * 1000.0;
            checkpoint.avg_duration_ms = old_avg + (new_sample - old_avg) / n;

            debug!(
                duration_ms = duration.as_millis(),
                io_bytes = checkpoint.checkpoint_io_bytes,
                "Checkpoint completed"
            );
        }

        checkpoint.in_progress = false;
        checkpoint.start_time = None;
    }

    /// Get I/O statistics for a specific type
    pub async fn get_io_stats(&self, io_type: IoType) -> IoStats {
        let stats = self.io_stats.read().await;
        stats.get(&io_type).cloned().unwrap_or_default()
    }

    /// Get all I/O statistics
    pub async fn get_all_io_stats(&self) -> HashMap<IoType, IoStats> {
        self.io_stats.read().await.clone()
    }

    /// Get WAL write information
    pub async fn get_wal_info(&self) -> WalWriteInfo {
        self.wal_info.read().await.clone()
    }

    /// Get checkpoint information
    pub async fn get_checkpoint_info(&self) -> CheckpointInfo {
        self.checkpoint_info.read().await.clone()
    }

    /// Get I/O statistics for a query
    pub async fn get_query_io(&self, query_hash: u64) -> Option<IoStats> {
        let query_io = self.query_io.read().await;
        query_io.get(&query_hash).cloned()
    }

    /// Clear I/O statistics for a query (after aggregation window)
    pub async fn clear_query_io(&self, query_hash: u64) {
        let mut query_io = self.query_io.write().await;
        query_io.remove(&query_hash);
    }

    /// Clear all query I/O statistics
    pub async fn clear_all_query_io(&self) {
        let mut query_io = self.query_io.write().await;
        query_io.clear();
    }

    /// Handle process exit (clear cached state)
    pub async fn process_exit(&self, pid: u32) {
        let mut resolver = self.fd_resolver.write().await;
        resolver.clear_pid(pid);
    }

    /// Get summary of I/O by category
    pub async fn get_io_summary(&self) -> IoSummary {
        let stats = self.io_stats.read().await;
        let _wal = self.wal_info.read().await;
        let checkpoint = self.checkpoint_info.read().await;

        let mut summary = IoSummary::default();

        for (io_type, type_stats) in stats.iter() {
            match io_type {
                IoType::Wal => {
                    summary.wal_bytes = type_stats.write_bytes;
                    summary.wal_writes = type_stats.write_count;
                }
                IoType::DataFile => {
                    summary.data_read_bytes = type_stats.read_bytes;
                    summary.data_write_bytes = type_stats.write_bytes;
                }
                IoType::TempFile => {
                    summary.temp_bytes = type_stats.read_bytes + type_stats.write_bytes;
                }
                _ => {
                    summary.other_bytes += type_stats.read_bytes + type_stats.write_bytes;
                }
            }
        }

        summary.checkpoint_io_bytes = checkpoint.checkpoint_io_bytes;
        summary.checkpoints_completed = checkpoint.completed_count;
        summary.avg_checkpoint_duration_ms = checkpoint.avg_duration_ms;

        summary
    }
}

impl Default for WalIoCorrelator {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of I/O activity
#[derive(Debug, Clone, Default)]
pub struct IoSummary {
    /// WAL bytes written
    pub wal_bytes: u64,
    /// WAL write count
    pub wal_writes: u64,
    /// Data file bytes read
    pub data_read_bytes: u64,
    /// Data file bytes written
    pub data_write_bytes: u64,
    /// Temporary file bytes
    pub temp_bytes: u64,
    /// Other I/O bytes
    pub other_bytes: u64,
    /// Checkpoint I/O bytes
    pub checkpoint_io_bytes: u64,
    /// Number of checkpoints completed
    pub checkpoints_completed: u64,
    /// Average checkpoint duration (ms)
    pub avg_checkpoint_duration_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_type_classification() {
        assert_eq!(
            IoType::from_path("/var/lib/postgresql/14/main/pg_wal/000000010000000000000001"),
            IoType::Wal
        );
        assert_eq!(
            IoType::from_path("/var/lib/postgresql/14/main/base/16384/16385"),
            IoType::DataFile
        );
        assert_eq!(
            IoType::from_path("/var/lib/postgresql/14/main/base/pgsql_tmp/pgsql_tmp12345.0"),
            IoType::TempFile
        );
        assert_eq!(
            IoType::from_path("/var/lib/postgresql/14/main/pg_xact/0000"),
            IoType::Clog
        );
        assert_eq!(
            IoType::from_path("/var/lib/postgresql/14/main/global/pg_control"),
            IoType::Control
        );
        assert_eq!(IoType::from_path("/some/random/file.txt"), IoType::Other);
    }

    #[test]
    fn test_io_type_properties() {
        assert!(IoType::DataFile.is_query_attributable());
        assert!(IoType::TempFile.is_query_attributable());
        assert!(!IoType::Wal.is_query_attributable());

        assert!(IoType::Wal.is_background());
        assert!(IoType::Clog.is_background());
        assert!(!IoType::DataFile.is_background());
    }

    #[tokio::test]
    async fn test_io_event_processing() {
        let correlator = WalIoCorrelator::new();

        // Process a data file read
        let event = correlator
            .process_io_event(1000, 10, false, 8192, 500, Some(12345))
            .await;

        assert_eq!(event.bytes, 8192);
        assert!(!event.is_write);

        // On Linux, fd resolution works and we can track query I/O
        // On other platforms, fd resolution returns None -> IoType::Other
        // which is not query-attributable
        #[cfg(target_os = "linux")]
        {
            let query_io = correlator.get_query_io(12345).await;
            assert!(query_io.is_some());
            let io = query_io.unwrap();
            assert_eq!(io.read_bytes, 8192);
            assert_eq!(io.read_count, 1);
        }

        #[cfg(not(target_os = "linux"))]
        {
            // On non-Linux, IoType::Other is not query-attributable
            assert_eq!(event.io_type, IoType::Other);
        }
    }

    #[tokio::test]
    async fn test_checkpoint_tracking() {
        let correlator = WalIoCorrelator::new();

        correlator.checkpoint_start().await;

        let info = correlator.get_checkpoint_info().await;
        assert!(info.in_progress);

        // Simulate checkpoint I/O
        correlator.set_background_pids(Some(1000), None).await;
        correlator
            .process_io_event(1000, 10, true, 1024 * 1024, 10000, None)
            .await;

        tokio::time::sleep(Duration::from_millis(10)).await;

        correlator.checkpoint_end().await;

        let info = correlator.get_checkpoint_info().await;
        assert!(!info.in_progress);
        assert_eq!(info.completed_count, 1);
        assert!(info.last_duration.is_some());
    }

    #[tokio::test]
    async fn test_io_summary() {
        let correlator = WalIoCorrelator::new();

        // Simulate various I/O events
        correlator
            .process_io_event(1000, 10, true, 8192, 100, None)
            .await;

        let summary = correlator.get_io_summary().await;
        // Note: Without path resolution, events go to "Other"
        assert!(summary.other_bytes > 0 || summary.data_write_bytes > 0);
    }
}
