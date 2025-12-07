// src/ebpf/mod.rs - Main eBPF module
//
// Architecture (Enhanced eBPF Implementation Plan v2):
// - Kernel: Minimal footprint - emit raw events only (no aggregation)
// - Userspace: EventAggregator handles all aggregation logic
// - IPC: Binary protocol over Unix socket for privilege separation
// - Correlation: pg_stat_activity polling for PID-to-query mapping
// - Sampling: Adaptive rates based on system load

// Core modules (always available)
pub mod correlator;
pub mod loader;
pub mod lock_tracer;
pub mod manager;
pub mod metrics_collector;
pub mod postgres_tracer;

// New v2 implementation modules
pub mod aggregator;
pub mod daemon;
pub mod events;
pub mod metrics;
pub mod mock;
pub mod pg_stat_correlator;
pub mod protocol;
pub mod sampling;

// Additional feature modules
pub mod query_normalizer;
pub mod session_tracker;
pub mod wal_correlator;
pub mod vacuum_detector;
pub mod cgroup_metrics;
pub mod degradation;
pub mod live_maps;
pub mod pg_stat_full;
pub mod program_loader;
pub mod prometheus;

use std::error::Error as StdError;
use std::fmt;

// Core re-exports
pub use correlator::DefaultMetricsCorrelator;
pub use loader::{EbpfProgram, EbpfEventStream, EbpfTarget, EventStreamStats};
pub use lock_tracer::{
    LockContentionMetrics, LockContentionSummary, LockEvent, LockEventType,
    LockMode, LockTracer,
};
pub use manager::{EbpfManager, EbpfMode, ManagerStats};
pub use metrics_collector::{EbpfMetricsCollector, MockSystemMetricsCollector, EbpfCollectorConfig};
pub use postgres_tracer::{
    PostgresTracer, PostgresQueryEvent, QueryStatus, QueryIoStats,
    LockContentionEvent, BlockingStats, QueryBreakdown, Recommendation,
};

// New v2 re-exports
pub use aggregator::{EventAggregator, AggregatedMetrics};
pub use daemon::{EbpfDaemon, DaemonConfig, DaemonClient};
pub use events::{RawEvent, EventType, ParseError};
pub use metrics::{EbpfMetrics, SharedMetrics, Counter, Gauge, Histogram, Timer, new_shared_metrics};
pub use mock::{MockEventGenerator, MockProfile, MockDaemonServer, ReplayProvider, MockScenarioBuilder};
pub use pg_stat_correlator::PgStatCorrelator;
pub use protocol::{Message, MessagePayload, ProtocolCodec, MetricsData, DaemonStatus};
pub use sampling::SamplingController;

// Additional feature re-exports
pub use query_normalizer::{QueryNormalizer, NormalizerStats};
pub use session_tracker::{SessionTracker, SessionState, PreparedStatement, Portal, TrackerStats, CloseType, QueryContext};
pub use wal_correlator::{WalIoCorrelator, IoType, ClassifiedIoEvent, WalWriteInfo, CheckpointInfo, IoStats, IoSummary};
pub use vacuum_detector::{VacuumDetector, VacuumType, VacuumPhase, ActiveVacuum, CompletedVacuum, AutovacuumWorker, VacuumStats, VacuumAnalysis};
pub use cgroup_metrics::{CgroupCollector, ContainerRuntime, CgroupVersion, CpuMetrics, MemoryMetrics, IoMetrics, ContainerMetrics, ResourceLimits};
pub use degradation::{DegradationManager, CapabilityLevel, DegradationReason, DegradationEvent, RecoveryEvent, DegradationStatus};
pub use live_maps::{LiveMapManager, LiveMapConfig, SamplingConfig, PidFilterConfig, LatencyThresholds, MapUpdateResult, MapUpdateStats};
pub use pg_stat_full::{PgStatIntegrator, PgStatStatement, PgStatActivity, PgStatTable, PgStatIndex, PgStatBgWriter, CorrelatedQueryMetrics, CorrelationSummary};
pub use program_loader::{EbpfLoader, LoaderConfig, LoadResult, ProgramType, MapType, ProgramDef, MapDef, LoadedProgram, LoadedMap};
pub use prometheus::{PrometheusRegistry, MetricType, MetricValue};

/// System metrics collected from kernel events
#[derive(Debug, Clone, Default)]
pub struct SystemMetrics {
    // Time breakdown
    pub total_time_us: u64,
    pub cpu_time_us: u64,
    pub io_wait_time_us: u64,
    pub lock_wait_time_us: u64,
    
    // I/O metrics
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_syscalls: u32,
    pub write_syscalls: u32,
    pub fsync_count: u32,
    
    // Memory metrics
    pub page_cache_hits: u32,
    pub page_cache_misses: u32,
    pub page_faults: u32,
    pub context_switches: u32,
    
    // Event quality
    pub events_processed: u64,
    pub events_dropped: u64,
}

impl SystemMetrics {
    /// Calculate confidence score based on event quality
    pub fn confidence(&self) -> f64 {
        if self.events_processed == 0 {
            return 0.0;
        }
        
        let total = self.events_processed + self.events_dropped;
        let success_rate = self.events_processed as f64 / total as f64;
        
        match success_rate {
            r if r >= 0.99 => 0.95,  // <1% loss = high confidence
            r if r >= 0.95 => 0.80,  // <5% loss = medium confidence
            r if r >= 0.90 => 0.60,  // <10% loss = low confidence
            _ => 0.40,                // >10% loss = very low confidence
        }
    }
    
    /// Get I/O percentage of total time
    pub fn io_percentage(&self) -> f64 {
        if self.total_time_us == 0 {
            return 0.0;
        }
        (self.io_wait_time_us as f64 / self.total_time_us as f64) * 100.0
    }
    
    /// Get CPU percentage of total time
    pub fn cpu_percentage(&self) -> f64 {
        if self.total_time_us == 0 {
            return 0.0;
        }
        (self.cpu_time_us as f64 / self.total_time_us as f64) * 100.0
    }
}

/// eBPF error types
#[derive(Debug)]
pub enum EbpfError {
    /// Failed to load eBPF program
    LoadError(String),
    /// Failed to attach to probe
    AttachError(String),
    /// Perf buffer error
    PerfBufferError(String),
    /// IPC communication error
    IpcError(String),
    /// Permission denied
    PermissionError(String),
    /// Parse error
    ParseError(String),
    /// Other error
    Other(String),
}

impl fmt::Display for EbpfError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::LoadError(msg) => write!(f, "Failed to load eBPF program: {}", msg),
            Self::AttachError(msg) => write!(f, "Failed to attach probe: {}", msg),
            Self::PerfBufferError(msg) => write!(f, "Perf buffer error: {}", msg),
            Self::IpcError(msg) => write!(f, "IPC error: {}", msg),
            Self::PermissionError(msg) => write!(f, "Permission denied: {}", msg),
            Self::ParseError(msg) => write!(f, "Parse error: {}", msg),
            Self::Other(msg) => write!(f, "eBPF error: {}", msg),
        }
    }
}

impl StdError for EbpfError {}

pub type Result<T> = std::result::Result<T, EbpfError>;
