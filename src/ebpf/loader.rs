// eBPF program loader abstraction
// Provides trait-based interface for loading and managing eBPF programs

use anyhow::Result;
use std::time::Duration;

/// Target for eBPF program attachment
#[derive(Debug, Clone)]
pub enum EbpfTarget {
    /// Attach to process by PID
    ProcessPid(u32),
    /// Attach to binary by path (will find running processes)
    BinaryPath(String),
}

/// Trait for eBPF programs
///
/// Allows for testing and mocking without requiring actual eBPF capabilities
pub trait EbpfProgram: Send + Sync {
    /// Program name (for logging/debugging)
    fn name(&self) -> &str;

    /// Load the eBPF program into the kernel
    fn load(&mut self) -> Result<()>;

    /// Attach to target process/binary
    fn attach(&mut self, target: &EbpfTarget) -> Result<()>;

    /// Detach from target
    fn detach(&mut self) -> Result<()>;

    /// Check if program is currently loaded
    fn is_loaded(&self) -> bool;

    /// Check if program is currently attached
    fn is_attached(&self) -> bool;
}

/// Trait for streaming events from eBPF programs
pub trait EbpfEventStream: Send + Sync {
    /// Event type produced by this stream
    type Event: Send + Sync;

    /// Poll for new events with timeout
    ///
    /// Returns empty vec if no events available within timeout
    fn poll_events(&mut self, timeout: Duration) -> Result<Vec<Self::Event>>;

    /// Get statistics about event processing
    fn stats(&self) -> EventStreamStats;
}

/// Statistics about event stream
#[derive(Debug, Clone, Default)]
pub struct EventStreamStats {
    pub events_processed: u64,
    pub events_dropped: u64,
    pub last_poll_duration_us: u64,
}

impl EventStreamStats {
    pub fn success_rate(&self) -> f64 {
        if self.events_processed == 0 {
            return 1.0;
        }
        let total = self.events_processed + self.events_dropped;
        self.events_processed as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_stream_stats() {
        let stats = EventStreamStats {
            events_processed: 95,
            events_dropped: 5,
            last_poll_duration_us: 1000,
        };

        assert_eq!(stats.success_rate(), 0.95);
    }

    #[test]
    fn test_event_stream_stats_no_events() {
        let stats = EventStreamStats::default();
        assert_eq!(stats.success_rate(), 1.0);
    }
}
