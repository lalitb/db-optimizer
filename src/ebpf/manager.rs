// eBPF Manager - Orchestrates eBPF programs and event processing
// Integrates lock tracking with the rest of the optimizer
//
// Architecture (v2):
// - In privileged mode: Runs EbpfDaemon directly with full kernel access
// - In client mode: Connects to daemon via Unix socket IPC
// - Supports both modes transparently via MetricsSource trait

use super::aggregator::AggregatedMetrics;
use super::daemon::DaemonClient;
use super::loader::{EbpfEventStream, EbpfProgram, EbpfTarget, EventStreamStats};
use super::lock_tracer::{LockContentionMetrics, LockContentionSummary, LockEvent, LockTracer};
use super::metrics::{EbpfMetrics, SharedMetrics};
use super::protocol::MetricsData;
use super::SystemMetrics;
use crate::config::EbpfConfig;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info};

/// Mode of operation for the eBPF manager
#[derive(Debug, Clone, PartialEq)]
pub enum EbpfMode {
    /// Disabled - no eBPF collection
    Disabled,
    /// Connect to privileged daemon via Unix socket
    Client { socket_path: PathBuf },
    /// Run in privileged mode (requires CAP_BPF, etc.)
    Privileged,
    /// Use mock data for testing
    Mock,
}

impl Default for EbpfMode {
    fn default() -> Self {
        Self::Disabled
    }
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
    metrics_rx: Option<mpsc::Receiver<AggregatedMetrics>>,
    /// Daemon client (when in Client mode)
    daemon_client: Option<DaemonClient>,
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
            EbpfMode::Mock // Default to mock if enabled but no socket specified
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
        }
    }

    /// Create a manager with existing lock tracer
    pub fn with_lock_tracer(config: EbpfConfig, lock_tracer: Arc<LockTracer>) -> Self {
        let mode = if config.enabled {
            EbpfMode::Mock
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

    /// Start the eBPF manager
    ///
    /// Behavior depends on mode:
    /// - Disabled: Does nothing
    /// - Client: Connects to daemon via Unix socket
    /// - Privileged: Loads and attaches eBPF programs directly
    /// - Mock: Uses mock event generator for testing
    pub async fn start(&mut self, target: EbpfTarget) -> Result<()> {
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
        let config = EbpfConfig {
            enabled: true,
            sampling_interval_ms: 100,
            collect_cpu_metrics: true,
            collect_io_metrics: true,
            collect_memory_metrics: true,
            collect_lock_metrics: true,
            max_events_buffer: 10000,
        };

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

        let config = EbpfConfig {
            enabled: true,
            sampling_interval_ms: 100,
            collect_cpu_metrics: true,
            collect_io_metrics: true,
            collect_memory_metrics: true,
            collect_lock_metrics: true,
            max_events_buffer: 10000,
        };

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
