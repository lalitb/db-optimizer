// eBPF Manager - Orchestrates eBPF programs and event processing
// Integrates lock tracking with the rest of the optimizer

use super::loader::{EbpfProgram, EbpfEventStream, EbpfTarget, EventStreamStats};
use super::lock_tracer::{LockEvent, LockTracer, LockContentionMetrics, LockContentionSummary};
use crate::config::EbpfConfig;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Manages eBPF programs and event processing
pub struct EbpfManager {
    config: EbpfConfig,
    programs: Vec<Box<dyn EbpfProgram>>,
    lock_tracer: Arc<LockTracer>,
    running: Arc<AtomicBool>,
    stats: Arc<RwLock<ManagerStats>>,
}

/// Statistics about the eBPF manager
#[derive(Debug, Clone, Default)]
pub struct ManagerStats {
    pub programs_loaded: usize,
    pub programs_attached: usize,
    pub total_events_processed: u64,
    pub total_events_dropped: u64,
    pub last_cycle_duration_ms: u64,
}

impl EbpfManager {
    /// Create a new eBPF manager
    pub fn new(config: EbpfConfig) -> Self {
        Self {
            config,
            programs: Vec::new(),
            lock_tracer: Arc::new(LockTracer::new()),
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(ManagerStats::default())),
        }
    }

    /// Create a new manager with existing lock tracer
    pub fn with_lock_tracer(config: EbpfConfig, lock_tracer: Arc<LockTracer>) -> Self {
        Self {
            config,
            programs: Vec::new(),
            lock_tracer,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(RwLock::new(ManagerStats::default())),
        }
    }

    /// Add an eBPF program to be managed
    pub fn add_program(&mut self, program: Box<dyn EbpfProgram>) {
        self.programs.push(program);
    }

    /// Start the eBPF manager
    /// 
    /// Loads and attaches all programs, then begins event processing
    pub async fn start(&mut self, target: EbpfTarget) -> Result<()> {
        if !self.config.enabled {
            info!("eBPF is disabled in configuration, skipping start");
            return Ok(());
        }

        info!("Starting eBPF manager with {} programs", self.programs.len());

        // Load all programs
        for program in &mut self.programs {
            program.load()
                .with_context(|| format!("Failed to load eBPF program: {}", program.name()))?;
            info!("Loaded eBPF program: {}", program.name());
        }

        // Attach all programs
        for program in &mut self.programs {
            program.attach(&target)
                .with_context(|| format!("Failed to attach eBPF program: {}", program.name()))?;
            info!("Attached eBPF program: {} to {:?}", program.name(), target);
        }

        // Update stats
        let mut stats = self.stats.write().await;
        stats.programs_loaded = self.programs.iter().filter(|p| p.is_loaded()).count();
        stats.programs_attached = self.programs.iter().filter(|p| p.is_attached()).count();
        drop(stats);

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

        // Detach all programs
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
