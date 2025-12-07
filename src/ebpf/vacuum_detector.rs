//! Vacuum and Autovacuum Detection
//!
//! Identifies and tracks vacuum and autovacuum activity to:
//! - Distinguish maintenance I/O from query I/O
//! - Track autovacuum impact on query performance
//! - Detect vacuum bottlenecks and tuning opportunities

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Vacuum operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VacuumType {
    /// Manual VACUUM command
    Manual,
    /// Automatic vacuum (autovacuum worker)
    Auto,
    /// VACUUM FULL (rewrites table)
    Full,
    /// VACUUM ANALYZE
    Analyze,
    /// Standalone ANALYZE
    AnalyzeOnly,
}

impl VacuumType {
    /// Whether this vacuum type blocks writes
    pub fn blocks_writes(&self) -> bool {
        matches!(self, VacuumType::Full)
    }

    /// Whether this vacuum type is automatic
    pub fn is_automatic(&self) -> bool {
        matches!(self, VacuumType::Auto)
    }

    /// Human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            VacuumType::Manual => "VACUUM",
            VacuumType::Auto => "autovacuum",
            VacuumType::Full => "VACUUM FULL",
            VacuumType::Analyze => "VACUUM ANALYZE",
            VacuumType::AnalyzeOnly => "ANALYZE",
        }
    }
}

/// State of a vacuum operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VacuumPhase {
    /// Scanning heap
    ScanningHeap,
    /// Vacuuming indexes
    VacuumingIndexes,
    /// Vacuuming heap
    VacuumingHeap,
    /// Cleaning up indexes
    CleaningUpIndexes,
    /// Truncating heap
    TruncatingHeap,
    /// Performing final cleanup
    PerformingFinalCleanup,
    /// Unknown/other phase
    Unknown,
}

impl VacuumPhase {
    /// Parse from pg_stat_progress_vacuum.phase
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "scanning heap" => VacuumPhase::ScanningHeap,
            "vacuuming indexes" => VacuumPhase::VacuumingIndexes,
            "vacuuming heap" => VacuumPhase::VacuumingHeap,
            "cleaning up indexes" => VacuumPhase::CleaningUpIndexes,
            "truncating heap" => VacuumPhase::TruncatingHeap,
            "performing final cleanup" => VacuumPhase::PerformingFinalCleanup,
            _ => VacuumPhase::Unknown,
        }
    }
}

/// Information about an active vacuum operation
#[derive(Debug, Clone)]
pub struct ActiveVacuum {
    /// Process ID (backend or autovacuum worker)
    pub pid: u32,
    /// Type of vacuum
    pub vacuum_type: VacuumType,
    /// Table being vacuumed (schema.table)
    pub table_name: String,
    /// Database name
    pub database: String,
    /// Current phase
    pub phase: VacuumPhase,
    /// Start time
    pub start_time: Instant,
    /// Heap blocks scanned
    pub heap_blks_scanned: u64,
    /// Heap blocks vacuumed
    pub heap_blks_vacuumed: u64,
    /// Total heap blocks
    pub heap_blks_total: u64,
    /// Index vacuum count
    pub index_vacuum_count: u64,
    /// Dead tuples collected
    pub dead_tuples: u64,
    /// I/O bytes read
    pub io_read_bytes: u64,
    /// I/O bytes written
    pub io_write_bytes: u64,
}

impl ActiveVacuum {
    pub fn new(pid: u32, vacuum_type: VacuumType, table_name: String, database: String) -> Self {
        Self {
            pid,
            vacuum_type,
            table_name,
            database,
            phase: VacuumPhase::Unknown,
            start_time: Instant::now(),
            heap_blks_scanned: 0,
            heap_blks_vacuumed: 0,
            heap_blks_total: 0,
            index_vacuum_count: 0,
            dead_tuples: 0,
            io_read_bytes: 0,
            io_write_bytes: 0,
        }
    }

    /// Progress percentage (0-100)
    pub fn progress_pct(&self) -> f64 {
        if self.heap_blks_total == 0 {
            0.0
        } else {
            (self.heap_blks_scanned as f64 / self.heap_blks_total as f64) * 100.0
        }
    }

    /// Elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Estimated time remaining
    pub fn eta(&self) -> Option<Duration> {
        if self.heap_blks_scanned == 0 {
            return None;
        }

        let elapsed = self.elapsed().as_secs_f64();
        let rate = self.heap_blks_scanned as f64 / elapsed;

        if rate <= 0.0 {
            return None;
        }

        let remaining_blocks = self.heap_blks_total.saturating_sub(self.heap_blks_scanned);
        let remaining_secs = remaining_blocks as f64 / rate;

        Some(Duration::from_secs_f64(remaining_secs))
    }
}

/// Completed vacuum statistics
#[derive(Debug, Clone)]
pub struct CompletedVacuum {
    /// Table that was vacuumed
    pub table_name: String,
    /// Database name
    pub database: String,
    /// Type of vacuum
    pub vacuum_type: VacuumType,
    /// Duration
    pub duration: Duration,
    /// Dead tuples removed
    pub dead_tuples_removed: u64,
    /// Pages scanned
    pub pages_scanned: u64,
    /// Pages removed (if any)
    pub pages_removed: u64,
    /// I/O read bytes
    pub io_read_bytes: u64,
    /// I/O write bytes
    pub io_write_bytes: u64,
    /// Completion time
    pub completed_at: Instant,
}

/// Autovacuum worker tracking
#[derive(Debug, Clone)]
pub struct AutovacuumWorker {
    /// Worker PID
    pub pid: u32,
    /// Database being serviced
    pub database: String,
    /// Tables vacuumed by this worker
    pub tables_vacuumed: u64,
    /// Total time spent vacuuming
    pub total_vacuum_time: Duration,
    /// Worker start time
    pub started_at: Instant,
}

/// Vacuum detection and tracking
pub struct VacuumDetector {
    /// Known autovacuum launcher PID
    autovacuum_launcher_pid: Arc<RwLock<Option<u32>>>,
    /// Active autovacuum worker PIDs
    autovacuum_worker_pids: Arc<RwLock<HashMap<u32, AutovacuumWorker>>>,
    /// Currently active vacuum operations
    active_vacuums: Arc<RwLock<HashMap<u32, ActiveVacuum>>>,
    /// Recently completed vacuums (for analysis)
    completed_vacuums: Arc<RwLock<Vec<CompletedVacuum>>>,
    /// Maximum completed vacuums to keep
    max_completed_history: usize,
    /// Statistics
    stats: Arc<RwLock<VacuumStats>>,
}

#[derive(Debug, Default, Clone)]
pub struct VacuumStats {
    /// Total vacuums detected
    pub total_vacuums: u64,
    /// Manual vacuums
    pub manual_vacuums: u64,
    /// Autovacuums
    pub auto_vacuums: u64,
    /// Total dead tuples removed
    pub total_dead_tuples_removed: u64,
    /// Total I/O attributed to vacuum
    pub total_vacuum_io_bytes: u64,
    /// Current active vacuum count
    pub active_vacuum_count: usize,
    /// Current autovacuum worker count
    pub autovacuum_worker_count: usize,
}

impl VacuumDetector {
    /// Create a new vacuum detector
    pub fn new() -> Self {
        Self::with_history_size(100)
    }

    /// Create with custom history size
    pub fn with_history_size(max_completed_history: usize) -> Self {
        Self {
            autovacuum_launcher_pid: Arc::new(RwLock::new(None)),
            autovacuum_worker_pids: Arc::new(RwLock::new(HashMap::new())),
            active_vacuums: Arc::new(RwLock::new(HashMap::new())),
            completed_vacuums: Arc::new(RwLock::new(Vec::with_capacity(max_completed_history))),
            max_completed_history,
            stats: Arc::new(RwLock::new(VacuumStats::default())),
        }
    }

    /// Set the autovacuum launcher PID
    pub async fn set_launcher_pid(&self, pid: u32) {
        *self.autovacuum_launcher_pid.write().await = Some(pid);
    }

    /// Register an autovacuum worker
    pub async fn register_worker(&self, pid: u32, database: String) {
        let worker = AutovacuumWorker {
            pid,
            database,
            tables_vacuumed: 0,
            total_vacuum_time: Duration::ZERO,
            started_at: Instant::now(),
        };

        let mut workers = self.autovacuum_worker_pids.write().await;
        workers.insert(pid, worker);

        let mut stats = self.stats.write().await;
        stats.autovacuum_worker_count = workers.len();

        debug!(pid = pid, "Registered autovacuum worker");
    }

    /// Unregister an autovacuum worker
    pub async fn unregister_worker(&self, pid: u32) {
        let mut workers = self.autovacuum_worker_pids.write().await;
        workers.remove(&pid);

        let mut stats = self.stats.write().await;
        stats.autovacuum_worker_count = workers.len();

        debug!(pid = pid, "Unregistered autovacuum worker");
    }

    /// Check if a PID is an autovacuum process
    pub async fn is_autovacuum_pid(&self, pid: u32) -> bool {
        let launcher = self.autovacuum_launcher_pid.read().await;
        if *launcher == Some(pid) {
            return true;
        }

        let workers = self.autovacuum_worker_pids.read().await;
        workers.contains_key(&pid)
    }

    /// Start tracking a vacuum operation
    pub async fn start_vacuum(
        &self,
        pid: u32,
        vacuum_type: VacuumType,
        table_name: String,
        database: String,
    ) {
        let vacuum = ActiveVacuum::new(pid, vacuum_type, table_name.clone(), database);

        let mut active = self.active_vacuums.write().await;
        active.insert(pid, vacuum);

        let mut stats = self.stats.write().await;
        stats.total_vacuums += 1;
        stats.active_vacuum_count = active.len();

        if vacuum_type.is_automatic() {
            stats.auto_vacuums += 1;
        } else {
            stats.manual_vacuums += 1;
        }

        info!(
            pid = pid,
            table = %table_name,
            vacuum_type = %vacuum_type.name(),
            "Vacuum started"
        );
    }

    /// Update vacuum progress
    pub async fn update_vacuum_progress(
        &self,
        pid: u32,
        phase: VacuumPhase,
        heap_blks_scanned: u64,
        heap_blks_vacuumed: u64,
        heap_blks_total: u64,
        index_vacuum_count: u64,
        dead_tuples: u64,
    ) {
        let mut active = self.active_vacuums.write().await;

        if let Some(vacuum) = active.get_mut(&pid) {
            vacuum.phase = phase;
            vacuum.heap_blks_scanned = heap_blks_scanned;
            vacuum.heap_blks_vacuumed = heap_blks_vacuumed;
            vacuum.heap_blks_total = heap_blks_total;
            vacuum.index_vacuum_count = index_vacuum_count;
            vacuum.dead_tuples = dead_tuples;
        }
    }

    /// Record I/O for a vacuum operation
    pub async fn record_vacuum_io(&self, pid: u32, read_bytes: u64, write_bytes: u64) {
        let mut active = self.active_vacuums.write().await;

        if let Some(vacuum) = active.get_mut(&pid) {
            vacuum.io_read_bytes += read_bytes;
            vacuum.io_write_bytes += write_bytes;
        }
    }

    /// Complete a vacuum operation
    pub async fn complete_vacuum(&self, pid: u32) {
        let mut active = self.active_vacuums.write().await;

        if let Some(vacuum) = active.remove(&pid) {
            let completed = CompletedVacuum {
                table_name: vacuum.table_name.clone(),
                database: vacuum.database.clone(),
                vacuum_type: vacuum.vacuum_type,
                duration: vacuum.elapsed(),
                dead_tuples_removed: vacuum.dead_tuples,
                pages_scanned: vacuum.heap_blks_scanned,
                pages_removed: 0, // Would need more info
                io_read_bytes: vacuum.io_read_bytes,
                io_write_bytes: vacuum.io_write_bytes,
                completed_at: Instant::now(),
            };

            // Update stats
            {
                let mut stats = self.stats.write().await;
                stats.active_vacuum_count = active.len();
                stats.total_dead_tuples_removed += vacuum.dead_tuples;
                stats.total_vacuum_io_bytes += vacuum.io_read_bytes + vacuum.io_write_bytes;
            }

            // Update worker stats if autovacuum
            if vacuum.vacuum_type.is_automatic() {
                let mut workers = self.autovacuum_worker_pids.write().await;
                if let Some(worker) = workers.get_mut(&pid) {
                    worker.tables_vacuumed += 1;
                    worker.total_vacuum_time += vacuum.elapsed();
                }
            }

            // Store in history
            let mut completed_list = self.completed_vacuums.write().await;
            if completed_list.len() >= self.max_completed_history {
                completed_list.remove(0);
            }
            completed_list.push(completed);

            info!(
                pid = pid,
                table = %vacuum.table_name,
                duration_ms = vacuum.elapsed().as_millis(),
                dead_tuples = vacuum.dead_tuples,
                "Vacuum completed"
            );
        }
    }

    /// Get active vacuums
    pub async fn get_active_vacuums(&self) -> Vec<ActiveVacuum> {
        self.active_vacuums.read().await.values().cloned().collect()
    }

    /// Get active vacuum by PID
    pub async fn get_vacuum(&self, pid: u32) -> Option<ActiveVacuum> {
        self.active_vacuums.read().await.get(&pid).cloned()
    }

    /// Get recently completed vacuums
    pub async fn get_completed_vacuums(&self) -> Vec<CompletedVacuum> {
        self.completed_vacuums.read().await.clone()
    }

    /// Get autovacuum workers
    pub async fn get_autovacuum_workers(&self) -> Vec<AutovacuumWorker> {
        self.autovacuum_worker_pids
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// Get vacuum statistics
    pub async fn get_stats(&self) -> VacuumStats {
        self.stats.read().await.clone()
    }

    /// Check if any blocking vacuum is active
    pub async fn has_blocking_vacuum(&self) -> bool {
        self.active_vacuums
            .read()
            .await
            .values()
            .any(|v| v.vacuum_type.blocks_writes())
    }

    /// Get tables currently being vacuumed
    pub async fn get_tables_being_vacuumed(&self) -> Vec<String> {
        self.active_vacuums
            .read()
            .await
            .values()
            .map(|v| v.table_name.clone())
            .collect()
    }

    /// Analyze vacuum patterns and provide recommendations
    pub async fn analyze_patterns(&self) -> VacuumAnalysis {
        let completed = self.completed_vacuums.read().await;
        let stats = self.stats.read().await;

        let mut tables_vacuum_count: HashMap<String, u64> = HashMap::new();
        let mut tables_dead_tuples: HashMap<String, u64> = HashMap::new();
        let mut total_duration = Duration::ZERO;

        for vacuum in completed.iter() {
            *tables_vacuum_count
                .entry(vacuum.table_name.clone())
                .or_default() += 1;
            *tables_dead_tuples
                .entry(vacuum.table_name.clone())
                .or_default() += vacuum.dead_tuples_removed;
            total_duration += vacuum.duration;
        }

        // Find frequently vacuumed tables
        let mut frequent_tables: Vec<_> = tables_vacuum_count.into_iter().collect();
        frequent_tables.sort_by(|a, b| b.1.cmp(&a.1));

        // Find tables with high dead tuple counts
        let mut high_churn_tables: Vec<_> = tables_dead_tuples.into_iter().collect();
        high_churn_tables.sort_by(|a, b| b.1.cmp(&a.1));

        let recommendations = self.generate_recommendations(&frequent_tables, &high_churn_tables);

        VacuumAnalysis {
            total_vacuums: stats.total_vacuums,
            auto_vacuum_ratio: if stats.total_vacuums > 0 {
                stats.auto_vacuums as f64 / stats.total_vacuums as f64
            } else {
                0.0
            },
            avg_duration: if completed.is_empty() {
                Duration::ZERO
            } else {
                total_duration / completed.len() as u32
            },
            frequently_vacuumed_tables: frequent_tables.into_iter().take(10).collect(),
            high_churn_tables: high_churn_tables.into_iter().take(10).collect(),
            total_io_bytes: stats.total_vacuum_io_bytes,
            recommendations,
        }
    }

    fn generate_recommendations(
        &self,
        frequent_tables: &[(String, u64)],
        high_churn_tables: &[(String, u64)],
    ) -> Vec<String> {
        let mut recommendations = Vec::new();

        // Check for frequently vacuumed tables
        for (table, count) in frequent_tables.iter().take(3) {
            if *count > 10 {
                recommendations.push(format!(
                    "Table '{}' vacuumed {} times. Consider increasing \
                     autovacuum_vacuum_threshold or autovacuum_vacuum_scale_factor for this table.",
                    table, count
                ));
            }
        }

        // Check for high-churn tables
        for (table, dead_tuples) in high_churn_tables.iter().take(3) {
            if *dead_tuples > 100_000 {
                recommendations.push(format!(
                    "Table '{}' generated {} dead tuples. Consider using HOT updates \
                     or reviewing UPDATE patterns.",
                    table, dead_tuples
                ));
            }
        }

        recommendations
    }

    /// Handle process exit
    pub async fn process_exit(&self, pid: u32) {
        // Check if it was an active vacuum
        self.complete_vacuum(pid).await;

        // Check if it was an autovacuum worker
        self.unregister_worker(pid).await;
    }
}

impl Default for VacuumDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Analysis of vacuum patterns
#[derive(Debug, Clone)]
pub struct VacuumAnalysis {
    /// Total vacuums observed
    pub total_vacuums: u64,
    /// Ratio of autovacuums to manual vacuums
    pub auto_vacuum_ratio: f64,
    /// Average vacuum duration
    pub avg_duration: Duration,
    /// Tables that are vacuumed frequently (table, count)
    pub frequently_vacuumed_tables: Vec<(String, u64)>,
    /// Tables with high dead tuple counts (table, dead_tuples)
    pub high_churn_tables: Vec<(String, u64)>,
    /// Total I/O bytes from vacuum
    pub total_io_bytes: u64,
    /// Recommendations
    pub recommendations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_vacuum_lifecycle() {
        let detector = VacuumDetector::new();

        // Start a vacuum
        detector
            .start_vacuum(
                1000,
                VacuumType::Auto,
                "public.users".to_string(),
                "mydb".to_string(),
            )
            .await;

        let active = detector.get_active_vacuums().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].table_name, "public.users");

        // Update progress
        detector
            .update_vacuum_progress(1000, VacuumPhase::ScanningHeap, 500, 0, 1000, 0, 100)
            .await;

        let vacuum = detector.get_vacuum(1000).await.unwrap();
        assert_eq!(vacuum.heap_blks_scanned, 500);
        assert!((vacuum.progress_pct() - 50.0).abs() < 0.01);

        // Complete
        detector.complete_vacuum(1000).await;

        let active = detector.get_active_vacuums().await;
        assert!(active.is_empty());

        let completed = detector.get_completed_vacuums().await;
        assert_eq!(completed.len(), 1);
    }

    #[tokio::test]
    async fn test_autovacuum_worker_tracking() {
        let detector = VacuumDetector::new();

        detector.register_worker(2000, "mydb".to_string()).await;

        assert!(detector.is_autovacuum_pid(2000).await);
        assert!(!detector.is_autovacuum_pid(9999).await);

        let workers = detector.get_autovacuum_workers().await;
        assert_eq!(workers.len(), 1);

        detector.unregister_worker(2000).await;

        assert!(!detector.is_autovacuum_pid(2000).await);
    }

    #[tokio::test]
    async fn test_vacuum_io_tracking() {
        let detector = VacuumDetector::new();

        detector
            .start_vacuum(
                1000,
                VacuumType::Manual,
                "public.orders".to_string(),
                "mydb".to_string(),
            )
            .await;

        detector.record_vacuum_io(1000, 10000, 5000).await;
        detector.record_vacuum_io(1000, 5000, 2000).await;

        let vacuum = detector.get_vacuum(1000).await.unwrap();
        assert_eq!(vacuum.io_read_bytes, 15000);
        assert_eq!(vacuum.io_write_bytes, 7000);
    }

    #[tokio::test]
    async fn test_stats() {
        let detector = VacuumDetector::new();

        detector
            .start_vacuum(1000, VacuumType::Auto, "t1".to_string(), "db".to_string())
            .await;
        detector
            .start_vacuum(1001, VacuumType::Manual, "t2".to_string(), "db".to_string())
            .await;

        let stats = detector.get_stats().await;
        assert_eq!(stats.total_vacuums, 2);
        assert_eq!(stats.auto_vacuums, 1);
        assert_eq!(stats.manual_vacuums, 1);
    }

    #[test]
    fn test_vacuum_type_properties() {
        assert!(VacuumType::Full.blocks_writes());
        assert!(!VacuumType::Auto.blocks_writes());
        assert!(VacuumType::Auto.is_automatic());
        assert!(!VacuumType::Manual.is_automatic());
    }

    #[test]
    fn test_vacuum_phase_parsing() {
        assert_eq!(
            VacuumPhase::from_str("scanning heap"),
            VacuumPhase::ScanningHeap
        );
        assert_eq!(
            VacuumPhase::from_str("vacuuming indexes"),
            VacuumPhase::VacuumingIndexes
        );
        assert_eq!(VacuumPhase::from_str("unknown stuff"), VacuumPhase::Unknown);
    }
}
