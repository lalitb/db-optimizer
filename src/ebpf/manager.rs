// eBPF Manager - Orchestrates eBPF programs and event processing
// Integrates lock tracking with the rest of the optimizer
//
// Architecture (v2):
// - In privileged mode: Runs EbpfDaemon directly with full kernel access
// - In client mode: Connects to daemon via Unix socket IPC
// - Supports both modes transparently via MetricsSource trait

use super::aggregator::AggregatedMetrics;
use super::daemon::DaemonClient;
#[cfg(all(target_os = "linux", feature = "ebpf"))]
use super::events::{LockContentionEventRaw, LwLockEventRaw, RowLockEventRaw};
#[cfg(test)]
use super::loader::EventStreamStats;
use super::loader::{EbpfEventStream, EbpfProgram, EbpfTarget};
use super::lock_tracer::{LockContentionMetrics, LockContentionSummary, LockEvent, LockTracer};
use super::metrics::{EbpfMetrics, SharedMetrics};
use super::program_loader::{EbpfLoader, LoadResult};
use super::protocol::MetricsData;
use super::SystemMetrics;
use crate::config::EbpfConfig;
use anyhow::{anyhow, Context, Result};
#[cfg(all(target_os = "linux", feature = "ebpf"))]
use sqlx::postgres::PgPoolOptions;
#[cfg(all(target_os = "linux", feature = "ebpf"))]
use sqlx::Row;
use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::watch;
use tokio::sync::{mpsc, RwLock};
#[cfg(all(target_os = "linux", feature = "ebpf"))]
use tracing::warn;
use tracing::{debug, error, info};

/// Mode of operation for the eBPF manager
#[derive(Debug, Clone, PartialEq, Default)]
pub enum EbpfMode {
    /// Disabled - no eBPF collection
    #[default]
    Disabled,
    /// Connect to privileged daemon via Unix socket
    Client { socket_path: PathBuf },
    /// Run in privileged mode (requires CAP_BPF, etc.)
    Privileged,
    /// Use mock data for testing
    Mock,
}

/// Manages eBPF programs and event processing
pub struct EbpfManager {
    config: EbpfConfig,
    mode: EbpfMode,
    programs: Vec<Box<dyn EbpfProgram>>,
    lock_tracer: Arc<LockTracer>,
    running: Arc<AtomicBool>,
    stats: Arc<RwLock<ManagerStats>>,
    metrics: SharedMetrics,
    /// Channel for receiving aggregated metrics (from daemon or direct collection)
    #[allow(dead_code)] // Used for async metrics processing
    metrics_rx: Option<mpsc::Receiver<AggregatedMetrics>>,
    /// Daemon client (when in Client mode)
    daemon_client: Option<DaemonClient>,
    /// Stop signal for background tasks
    stop_sender: Option<watch::Sender<bool>>,
}

/// Statistics about the eBPF manager
#[derive(Debug, Clone, Default)]
pub struct ManagerStats {
    pub programs_loaded: usize,
    pub programs_attached: usize,
    pub total_events_processed: u64,
    pub total_events_dropped: u64,
    pub last_cycle_duration_ms: u64,
    pub mode: String,
    pub daemon_connected: bool,
}

impl EbpfManager {
    /// Create a new eBPF manager in disabled mode
    pub fn new(config: EbpfConfig) -> Self {
        let mode = if config.enabled {
            EbpfMode::Privileged
        } else {
            EbpfMode::Disabled
        };

        Self {
            config,
            mode,
            programs: Vec::new(),
            lock_tracer: Arc::new(LockTracer::new()),
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(ManagerStats::default())),
            metrics: Arc::new(EbpfMetrics::new()),
            metrics_rx: None,
            daemon_client: None,
            stop_sender: None,
        }
    }

    /// Create a manager that connects to a daemon via Unix socket
    pub fn client(config: EbpfConfig, socket_path: PathBuf) -> Self {
        Self {
            config,
            mode: EbpfMode::Client { socket_path },
            programs: Vec::new(),
            lock_tracer: Arc::new(LockTracer::new()),
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(ManagerStats::default())),
            metrics: Arc::new(EbpfMetrics::new()),
            metrics_rx: None,
            daemon_client: None,
            stop_sender: None,
        }
    }

    /// Create a manager with existing lock tracer
    pub fn with_lock_tracer(config: EbpfConfig, lock_tracer: Arc<LockTracer>) -> Self {
        let mode = if config.enabled {
            EbpfMode::Privileged
        } else {
            EbpfMode::Disabled
        };

        Self {
            config,
            mode,
            programs: Vec::new(),
            lock_tracer,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(ManagerStats::default())),
            metrics: Arc::new(EbpfMetrics::new()),
            metrics_rx: None,
            daemon_client: None,
            stop_sender: None,
        }
    }

    /// Set the operating mode
    pub fn set_mode(&mut self, mode: EbpfMode) {
        self.mode = mode;
    }

    /// Get current mode
    pub fn mode(&self) -> &EbpfMode {
        &self.mode
    }

    /// Get shared metrics handle
    pub fn metrics(&self) -> SharedMetrics {
        Arc::clone(&self.metrics)
    }

    /// Add an eBPF program to be managed
    pub fn add_program(&mut self, program: Box<dyn EbpfProgram>) {
        self.programs.push(program);
    }

    /// Add the default lock contention program (loads bpf/lock_contention.bpf.o)
    pub fn add_default_lock_program(&mut self) {
        let loader = EbpfLoader::new();
        let program = LoaderProgram::new(
            "lock_contention",
            "bpf/.output/lock_contention.bpf.o",
            loader,
        );
        self.add_program(Box::new(program));
    }

    /// Add LWLock tracer program
    pub fn add_lwlock_program(&mut self) {
        let loader = EbpfLoader::new();
        let program =
            LoaderProgram::new("lwlock_tracer", "bpf/.output/lwlock_tracer.bpf.o", loader);
        self.add_program(Box::new(program));
    }

    /// Add row lock tracer program
    pub fn add_row_lock_program(&mut self) {
        let loader = EbpfLoader::new();
        let program = LoaderProgram::new(
            "row_lock_tracer",
            "bpf/.output/row_lock_tracer.bpf.o",
            loader,
        );
        self.add_program(Box::new(program));
    }

    /// Update pid -> query hash mapping used by the eBPF program for correlation
    pub async fn set_pid_query_mapping(&self, pid: u32, query_hash: u64) -> Result<()> {
        for program in &self.programs {
            if let Some(loader_prog) = program.as_any().downcast_ref::<LoaderProgram>() {
                return loader_prog.update_pid_query_map(pid, query_hash).await;
            }
        }
        Err(anyhow!(
            "No loader-backed eBPF program available to update pid map"
        ))
    }

    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    async fn spawn_lock_event_consumer(&self) -> Result<()> {
        let mut buffers = self.open_perf_buffers_for_map("lock_events").await?;
        if buffers.is_empty() {
            return Err(anyhow!("No perf buffers opened for lock_events"));
        }

        let tracer = Arc::clone(&self.lock_tracer);
        let metrics = Arc::clone(&self.metrics);

        tokio::spawn(async move {
            let event_size = LockContentionEventRaw::size();
            let mut buf = vec![0u8; event_size * 16]; // read up to 16 events per poll

            loop {
                for perf_buf in buffers.iter_mut() {
                    match perf_buf.read_events(&mut buf) {
                        Ok(read) => {
                            let events = read / event_size;
                            for idx in 0..events {
                                let start = idx * event_size;
                                let end = start + event_size;
                                if end > buf.len() {
                                    break;
                                }
                                if let Ok(raw) = LockContentionEventRaw::parse(&buf[start..end]) {
                                    tracer.record_lock_contention_event(raw);
                                } else {
                                    metrics.perf_buffer_read_errors.inc();
                                }
                            }
                            if read % event_size != 0 {
                                metrics.perf_buffer_read_errors.inc();
                            }
                            metrics.events_processed.inc_by(events as u64);
                        }
                        Err(aya::maps::perf::PerfBufferError::NoData) => {}
                        Err(e) => {
                            metrics.perf_buffer_read_errors.inc();
                            error!("Error reading lock_events perf buffer: {}", e);
                        }
                    }
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });

        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    async fn spawn_lwlock_event_consumer(&self) -> Result<()> {
        let loader_prog = self
            .programs
            .iter()
            .find_map(|p| p.as_any().downcast_ref::<LoaderProgram>())
            .ok_or_else(|| anyhow!("No loader-backed program to read lwlock events"))?;

        let mut buffers = loader_prog
            .loader
            .open_perf_buffers("lwlock_events")
            .await?;
        if buffers.is_empty() {
            return Err(anyhow!("No perf buffers opened for lwlock_events"));
        }

        let tracer = Arc::clone(&self.lock_tracer);
        let metrics = Arc::clone(&self.metrics);

        tokio::spawn(async move {
            let event_size = LwLockEventRaw::size();
            let mut buf = vec![0u8; event_size * 16];

            loop {
                for perf_buf in buffers.iter_mut() {
                    match perf_buf.read_events(&mut buf) {
                        Ok(read) => {
                            let events = read / event_size;
                            for idx in 0..events {
                                let start = idx * event_size;
                                let end = start + event_size;
                                if end > buf.len() {
                                    break;
                                }
                                if let Ok(raw) = LwLockEventRaw::parse(&buf[start..end]) {
                                    tracer.record_lwlock_event(raw);
                                } else {
                                    metrics.perf_buffer_read_errors.inc();
                                }
                            }
                            if read % event_size != 0 {
                                metrics.perf_buffer_read_errors.inc();
                            }
                            metrics.events_processed.inc_by(events as u64);
                        }
                        Err(aya::maps::perf::PerfBufferError::NoData) => {}
                        Err(e) => {
                            metrics.perf_buffer_read_errors.inc();
                            error!("Error reading lwlock_events perf buffer: {}", e);
                        }
                    }
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });

        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    async fn spawn_row_lock_event_consumer(&self) -> Result<()> {
        let loader_prog = self
            .programs
            .iter()
            .find_map(|p| p.as_any().downcast_ref::<LoaderProgram>())
            .ok_or_else(|| anyhow!("No loader-backed program to read row lock events"))?;

        let mut buffers = loader_prog
            .loader
            .open_perf_buffers("row_lock_events")
            .await?;
        if buffers.is_empty() {
            return Err(anyhow!("No perf buffers opened for row_lock_events"));
        }

        let tracer = Arc::clone(&self.lock_tracer);
        let metrics = Arc::clone(&self.metrics);

        tokio::spawn(async move {
            let event_size = RowLockEventRaw::size();
            let mut buf = vec![0u8; event_size * 16];

            loop {
                for perf_buf in buffers.iter_mut() {
                    match perf_buf.read_events(&mut buf) {
                        Ok(read) => {
                            let events = read / event_size;
                            for idx in 0..events {
                                let start = idx * event_size;
                                let end = start + event_size;
                                if end > buf.len() {
                                    break;
                                }
                                if let Ok(raw) = RowLockEventRaw::parse(&buf[start..end]) {
                                    tracer.record_row_lock_event(raw);
                                } else {
                                    metrics.perf_buffer_read_errors.inc();
                                }
                            }
                            if read % event_size != 0 {
                                metrics.perf_buffer_read_errors.inc();
                            }
                            metrics.events_processed.inc_by(events as u64);
                        }
                        Err(aya::maps::perf::PerfBufferError::NoData) => {}
                        Err(e) => {
                            metrics.perf_buffer_read_errors.inc();
                            error!("Error reading row_lock_events perf buffer: {}", e);
                        }
                    }
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });

        Ok(())
    }

    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    async fn spawn_pid_map_refresher(
        &self,
        _target: EbpfTarget,
        mut stop: watch::Receiver<bool>,
    ) -> Result<()> {
        let interval_ms = self.config.pid_map_refresh_ms;
        if interval_ms == 0 {
            return Ok(());
        }

        let pid_query_sql = self.config.pid_query_sql.clone();
        let db_url = if let Some(url) = &self.config.pid_query_database_url {
            if url.is_empty() {
                None
            } else {
                Some(url.clone())
            }
        } else {
            None
        };

        if db_url.is_none() {
            warn!("pid->query map refresh is enabled but no pid_query_database_url provided; skipping");
            return Ok(());
        }

        let db_url = db_url.unwrap();

        // Collect loader-backed programs we can update
        let loaders: Vec<Arc<LoaderProgram>> = self
            .programs
            .iter()
            .filter_map(|p| p.as_any().downcast_ref::<LoaderProgram>())
            .map(|p| Arc::new(p.clone_shallow()))
            .collect();

        if loaders.is_empty() {
            return Err(anyhow!(
                "No loader-backed eBPF program available to update pid map"
            ));
        }

        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(interval_ms));
            let pool = match PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(3))
                .connect(&db_url)
                .await
            {
                Ok(pool) => pool,
                Err(e) => {
                    error!("Failed to connect for pid map refresh: {}", e);
                    return;
                }
            };

            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        match sqlx::query(&pid_query_sql).fetch_all(&pool).await {
                            Ok(rows) => {
                                for row in rows {
                                    let pid: i32 = match row.try_get(0) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            warn!("pid map row missing pid: {}", e);
                                            continue;
                                        }
                                    };
                                    let query_hash: i64 = match row.try_get(1) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            warn!("pid map row missing queryid: {}", e);
                                            continue;
                                        }
                                    };

                                    for loader in &loaders {
                                        if let Err(e) = loader.update_pid_query_map(pid as u32, query_hash as u64).await {
                                            warn!("Failed to update pid map for pid {}: {}", pid, e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to refresh pid->query map: {}", e);
                            }
                        }
                    }
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Start the eBPF manager
    ///
    /// Behavior depends on mode:
    /// - Disabled: Does nothing
    /// - Client: Connects to daemon via Unix socket
    /// - Privileged: Loads and attaches eBPF programs directly
    /// - Mock: Uses mock event generator for testing
    pub async fn start(&mut self, target: EbpfTarget) -> Result<()> {
        let (stop_sender, stop_receiver) = watch::channel(false);
        self.stop_sender = Some(stop_sender);
        #[cfg(not(all(target_os = "linux", feature = "ebpf")))]
        let _ = &stop_receiver;

        match &self.mode {
            EbpfMode::Disabled => {
                info!("eBPF is disabled, skipping start");
                return Ok(());
            }
            EbpfMode::Client { socket_path } => {
                info!(
                    "Starting eBPF manager in client mode, connecting to {:?}",
                    socket_path
                );
                let socket_str = socket_path.to_string_lossy();
                let client = DaemonClient::connect(&socket_str).map_err(|e| {
                    anyhow::anyhow!("Failed to connect to daemon at {:?}: {}", socket_path, e)
                })?;
                self.daemon_client = Some(client);

                let mut stats = self.stats.write().await;
                stats.mode = "client".to_string();
                stats.daemon_connected = true;
            }
            EbpfMode::Privileged => {
                info!(
                    "Starting eBPF manager in privileged mode with {} programs",
                    self.programs.len()
                );

                if self.programs.is_empty() {
                    return Err(anyhow!(
                        "No eBPF programs registered. Add a program before starting."
                    ));
                }

                // Load all programs
                for program in &mut self.programs {
                    program.load().with_context(|| {
                        format!("Failed to load eBPF program: {}", program.name())
                    })?;
                    info!("Loaded eBPF program: {}", program.name());
                }

                // Attach all programs
                for program in &mut self.programs {
                    program.attach(&target).with_context(|| {
                        format!("Failed to attach eBPF program: {}", program.name())
                    })?;
                    info!("Attached eBPF program: {} to {:?}", program.name(), target);
                }

                let mut stats = self.stats.write().await;
                stats.programs_loaded = self.programs.iter().filter(|p| p.is_loaded()).count();
                stats.programs_attached = self.programs.iter().filter(|p| p.is_attached()).count();
                stats.mode = "privileged".to_string();

                #[cfg(all(target_os = "linux", feature = "ebpf"))]
                if let Err(e) = self.spawn_lock_event_consumer().await {
                    error!("Failed to start lock event consumer: {}", e);
                }

                #[cfg(all(target_os = "linux", feature = "ebpf"))]
                if let Err(e) = self
                    .spawn_pid_map_refresher(target.clone(), stop_receiver.clone())
                    .await
                {
                    error!("Failed to start pid->query map refresher: {}", e);
                }
            }
            EbpfMode::Mock => {
                info!("Starting eBPF manager in mock mode");
                let mut stats = self.stats.write().await;
                stats.mode = "mock".to_string();
            }
        }

        self.running.store(true, Ordering::SeqCst);
        info!("eBPF manager started successfully");

        Ok(())
    }

    /// Stop the eBPF manager
    ///
    /// Detaches and unloads all programs
    pub async fn stop(&mut self) -> Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        info!("Stopping eBPF manager");
        self.running.store(false, Ordering::SeqCst);

        if let Some(stop) = self.stop_sender.take() {
            let _ = stop.send(true);
        }

        // Disconnect from daemon if in client mode
        if let Some(client) = self.daemon_client.take() {
            drop(client);
        }

        // Detach all programs if in privileged mode
        for program in &mut self.programs {
            if let Err(e) = program.detach() {
                error!("Failed to detach program {}: {}", program.name(), e);
            }
        }

        info!("eBPF manager stopped");
        Ok(())
    }

    /// Check if manager is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Process events from an event stream
    ///
    /// This would be called in a loop for each event stream
    pub async fn process_lock_events<S>(&self, stream: &mut S) -> Result<usize>
    where
        S: EbpfEventStream<Event = LockEvent>,
    {
        let timeout = Duration::from_millis(self.config.sampling_interval_ms);
        let events = stream.poll_events(timeout)?;

        let event_count = events.len();
        if event_count > 0 {
            debug!("Processing {} lock events", event_count);
        }

        // Process each event
        for event in events {
            if let Err(e) = self.lock_tracer.process_event(event) {
                error!("Failed to process lock event: {}", e);
            }
        }

        // Update stats
        let stream_stats = stream.stats();
        let mut stats = self.stats.write().await;
        stats.total_events_processed += stream_stats.events_processed;
        stats.total_events_dropped += stream_stats.events_dropped;

        Ok(event_count)
    }

    /// Get lock contention metrics
    pub fn get_lock_metrics(&self) -> Vec<LockContentionMetrics> {
        self.lock_tracer.get_all_metrics()
    }

    /// Get high contention tables
    pub fn get_high_contention(&self) -> Vec<LockContentionMetrics> {
        self.lock_tracer.get_high_contention()
    }

    /// Get lock contention summary
    pub fn get_summary(&self) -> LockContentionSummary {
        self.lock_tracer.get_summary()
    }

    /// Update table name mappings
    ///
    /// Call this periodically to keep table names in sync with database
    pub fn update_table_names(&self, oid_to_name: HashMap<u32, String>) {
        self.lock_tracer.update_table_names(oid_to_name);
    }

    /// Get manager statistics
    pub async fn get_stats(&self) -> ManagerStats {
        self.stats.read().await.clone()
    }

    /// Get reference to lock tracer
    pub fn lock_tracer(&self) -> Arc<LockTracer> {
        Arc::clone(&self.lock_tracer)
    }

    /// Poll for metrics from daemon (client mode only)
    ///
    /// Returns None if not in client mode or not connected
    pub fn get_pid_metrics(&mut self, pid: u32) -> Option<MetricsData> {
        if let Some(ref mut client) = self.daemon_client {
            match client.get_metrics(pid) {
                Ok(metrics_data) => {
                    self.metrics
                        .events_received
                        .inc_by(metrics_data.events_processed);
                    Some(metrics_data)
                }
                Err(e) => {
                    error!("Failed to poll metrics from daemon: {}", e);
                    self.metrics.ipc_errors.inc();
                    None
                }
            }
        } else {
            None
        }
    }

    /// Convert MetricsData to SystemMetrics for use with the correlation engine
    pub fn metrics_to_system_metrics(data: &MetricsData) -> SystemMetrics {
        SystemMetrics {
            total_time_us: data.total_time_us,
            cpu_time_us: data.cpu_time_us,
            io_wait_time_us: data.io_wait_time_us,
            lock_wait_time_us: data.lock_wait_time_us,
            bytes_read: data.bytes_read,
            bytes_written: data.bytes_written,
            read_syscalls: data.read_syscalls,
            write_syscalls: data.write_syscalls,
            fsync_count: data.fsync_count,
            page_cache_hits: 0,
            page_cache_misses: 0,
            page_faults: data.page_faults,
            context_switches: data.context_switches,
            events_processed: data.events_processed,
            events_dropped: data.events_dropped,
        }
    }

    /// Check if connected to daemon (client mode)
    pub fn is_daemon_connected(&self) -> bool {
        self.daemon_client.is_some()
    }
}

impl Drop for EbpfManager {
    fn drop(&mut self) {
        // Ensure we detach on drop
        for program in &mut self.programs {
            let _ = program.detach();
        }
    }
}

/// Loader-backed program that uses the in-repo program_loader abstraction
/// to load and attach a compiled eBPF object file.
#[derive(Clone)]
pub struct LoaderProgram {
    name: String,
    object_path: String,
    loader: EbpfLoader,
    loaded: bool,
    attached: bool,
}

impl LoaderProgram {
    pub fn new(name: &str, object_path: &str, loader: EbpfLoader) -> Self {
        Self {
            name: name.to_string(),
            object_path: object_path.to_string(),
            loader,
            loaded: false,
            attached: false,
        }
    }

    /// Create a shallow clone sharing the loader (used by background tasks)
    pub fn clone_shallow(&self) -> Self {
        Self {
            name: self.name.clone(),
            object_path: self.object_path.clone(),
            loader: self.loader.clone(),
            loaded: self.loaded,
            attached: self.attached,
        }
    }

    fn block_on<F: Future>(&self, fut: F) -> Result<F::Output>
    where
        F: Send,
        F::Output: Send,
    {
        if let Ok(handle) = Handle::try_current() {
            Ok(handle.block_on(fut))
        } else {
            let rt = Runtime::new()?;
            Ok(rt.block_on(fut))
        }
    }

    fn convert_load_result(&self, res: LoadResult, action: &str) -> Result<()> {
        match res {
            LoadResult::Success | LoadResult::SuccessWithWarnings(_) => Ok(()),
            LoadResult::Failed(err) => Err(anyhow!("Failed to {}: {}", action, err)),
            LoadResult::NotSupported(reason) => Err(anyhow!("eBPF not supported: {}", reason)),
        }
    }

    /// Update pid -> query hash map for correlation
    pub async fn update_pid_query_map(&self, pid: u32, query_hash: u64) -> Result<()> {
        self.loader
            .update_hash_map_u32_u64("pid_query_map", pid, query_hash)
            .await
    }
}

impl EbpfProgram for LoaderProgram {
    fn name(&self) -> &str {
        &self.name
    }

    fn load(&mut self) -> Result<()> {
        // Discover kernel capabilities and then load the object file.
        let detect = self.block_on(self.loader.detect_capabilities())?;
        self.convert_load_result(detect, "detect capabilities")?;

        // Try primary path, then a packaged fallback
        let candidates = vec![
            self.object_path.clone(),
            "/usr/local/lib/db-optimizer/lock_contention.bpf.o".to_string(),
        ];

        let mut last_err = None;
        for path in candidates {
            if !Path::new(&path).exists() {
                last_err = Some(format!("Object file not found: {}", path));
                continue;
            }
            let res = self.block_on(self.loader.load_object(&path))?;
            if self.convert_load_result(res, "load eBPF object").is_ok() {
                self.loaded = true;
                return Ok(());
            }
        }

        Err(anyhow!(
            "Failed to load eBPF object; last error: {}",
            last_err.unwrap_or_else(|| "unknown".to_string())
        ))
    }

    fn attach(&mut self, target: &EbpfTarget) -> Result<()> {
        let target_path = match target {
            EbpfTarget::BinaryPath(path) => Some(path.clone()),
            EbpfTarget::ProcessPid(pid) => fs::read_link(format!("/proc/{pid}/exe"))
                .ok()
                .map(|p| p.to_string_lossy().to_string()),
        };

        let res = self.block_on(self.loader.attach_all(target_path.as_deref()))?;
        self.convert_load_result(res, "attach eBPF program")?;

        self.attached = true;
        Ok(())
    }

    fn detach(&mut self) -> Result<()> {
        // Best-effort detach/unload; ignore errors to avoid poisoning shutdown.
        let _ = self.block_on(self.loader.detach_all());
        let _ = self.block_on(self.loader.unload());
        self.attached = false;
        Ok(())
    }

    fn is_loaded(&self) -> bool {
        self.loaded
    }

    fn is_attached(&self) -> bool {
        self.attached
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    pub async fn open_lock_perf_buffers(
        &self,
    ) -> Result<Vec<aya::maps::perf::PerfEventArrayBuffer>> {
        self.loader.open_perf_buffers("lock_events").await
    }
}

/// Mock eBPF program for testing
#[cfg(test)]
pub struct MockEbpfProgram {
    name: String,
    loaded: bool,
    attached: bool,
}

#[cfg(test)]
impl MockEbpfProgram {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            loaded: false,
            attached: false,
        }
    }
}

#[cfg(test)]
impl EbpfProgram for MockEbpfProgram {
    fn name(&self) -> &str {
        &self.name
    }

    fn load(&mut self) -> Result<()> {
        self.loaded = true;
        Ok(())
    }

    fn attach(&mut self, _target: &EbpfTarget) -> Result<()> {
        self.attached = true;
        Ok(())
    }

    fn detach(&mut self) -> Result<()> {
        self.attached = false;
        Ok(())
    }

    fn is_loaded(&self) -> bool {
        self.loaded
    }

    fn is_attached(&self) -> bool {
        self.attached
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebpf::lock_tracer::LockMode;
    use chrono::Utc;

    struct MockLockEventStream {
        events: Vec<LockEvent>,
        stats: EventStreamStats,
    }

    impl MockLockEventStream {
        fn new(events: Vec<LockEvent>) -> Self {
            Self {
                events,
                stats: EventStreamStats::default(),
            }
        }
    }

    impl EbpfEventStream for MockLockEventStream {
        type Event = LockEvent;

        fn poll_events(&mut self, _timeout: Duration) -> Result<Vec<Self::Event>> {
            let events = std::mem::take(&mut self.events);
            self.stats.events_processed += events.len() as u64;
            Ok(events)
        }

        fn stats(&self) -> EventStreamStats {
            self.stats.clone()
        }
    }

    #[tokio::test]
    async fn test_ebpf_manager_lifecycle() {
        let config = EbpfConfig::default();

        let mut manager = EbpfManager::new(config);
        manager.set_mode(EbpfMode::Privileged);
        manager.add_program(Box::new(MockEbpfProgram::new("test_program")));

        // Start manager
        manager.start(EbpfTarget::ProcessPid(1234)).await.unwrap();
        assert!(manager.is_running());

        let stats = manager.get_stats().await;
        assert_eq!(stats.programs_loaded, 1);
        assert_eq!(stats.programs_attached, 1);

        // Stop manager
        manager.stop().await.unwrap();
        assert!(!manager.is_running());
    }

    #[tokio::test]
    async fn test_process_lock_events() {
        use crate::ebpf::lock_tracer::LockEventType;

        let config = EbpfConfig::default();

        let manager = EbpfManager::new(config);

        let event = LockEvent {
            timestamp: Utc::now(),
            pid: 1234,
            table_oid: 5678,
            lock_mode: LockMode::ExclusiveLock,
            event_type: LockEventType::AcquireSuccess,
            granted_fast: false,
            wait_time_ns: 50_000_000, // 50ms
            process_name: "postgres".to_string(),
        };

        let mut stream = MockLockEventStream::new(vec![event]);
        let count = manager.process_lock_events(&mut stream).await.unwrap();

        assert_eq!(count, 1);

        let metrics = manager.get_lock_metrics();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].table_oid, 5678);
    }

    #[tokio::test]
    async fn test_disabled_manager() {
        let config = EbpfConfig {
            enabled: false,
            ..Default::default()
        };

        let mut manager = EbpfManager::new(config);

        // Should succeed but do nothing
        manager.start(EbpfTarget::ProcessPid(1234)).await.unwrap();
        assert!(!manager.is_running());
    }
}
