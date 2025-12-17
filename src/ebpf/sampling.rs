// src/ebpf/sampling.rs - Dynamic sampling rate control
//
// This module provides runtime-configurable sampling rates for eBPF events.
// Sampling helps reduce overhead under high load while still capturing
// statistically meaningful data.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

use super::events::EventType;

/// Default sampling rates for different event types
/// Rate of N means capture 1 in N events
pub const DEFAULT_IO_SAMPLING_RATE: u32 = 1; // Every I/O event
pub const DEFAULT_SCHED_SAMPLING_RATE: u32 = 10; // 1 in 10 context switches
pub const DEFAULT_MEMORY_SAMPLING_RATE: u32 = 1; // Every page fault
pub const DEFAULT_QUERY_SAMPLING_RATE: u32 = 1; // Every query event

/// Sampling configuration
#[derive(Debug, Clone)]
pub struct SamplingConfig {
    /// I/O event sampling rate (read/write)
    pub io_rate: u32,
    /// Scheduler event sampling rate (context switches)
    pub sched_rate: u32,
    /// Memory event sampling rate (page faults)
    pub memory_rate: u32,
    /// Query event sampling rate (PostgreSQL uprobes)
    pub query_rate: u32,
    /// Enable adaptive sampling based on load
    pub adaptive_enabled: bool,
    /// Target event rate per second
    pub target_events_per_sec: u32,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            io_rate: DEFAULT_IO_SAMPLING_RATE,
            sched_rate: DEFAULT_SCHED_SAMPLING_RATE,
            memory_rate: DEFAULT_MEMORY_SAMPLING_RATE,
            query_rate: DEFAULT_QUERY_SAMPLING_RATE,
            adaptive_enabled: true,
            target_events_per_sec: 10_000,
        }
    }
}

/// Controller for dynamically adjusting sampling rates
pub struct SamplingController {
    /// Current I/O sampling rate
    io_rate: Arc<AtomicU32>,
    /// Current scheduler sampling rate
    sched_rate: Arc<AtomicU32>,
    /// Current memory sampling rate
    memory_rate: Arc<AtomicU32>,
    /// Current query sampling rate
    query_rate: Arc<AtomicU32>,

    /// Configuration
    config: Arc<RwLock<SamplingConfig>>,

    /// Recent event counts for adaptive adjustment
    recent_event_counts: Arc<RwLock<Vec<u64>>>,
}

impl SamplingController {
    /// Create a new sampling controller with default config
    pub fn new() -> Self {
        Self::with_config(SamplingConfig::default())
    }

    /// Create a new sampling controller with custom config
    pub fn with_config(config: SamplingConfig) -> Self {
        Self {
            io_rate: Arc::new(AtomicU32::new(config.io_rate)),
            sched_rate: Arc::new(AtomicU32::new(config.sched_rate)),
            memory_rate: Arc::new(AtomicU32::new(config.memory_rate)),
            query_rate: Arc::new(AtomicU32::new(config.query_rate)),
            config: Arc::new(RwLock::new(config)),
            recent_event_counts: Arc::new(RwLock::new(Vec::with_capacity(60))),
        }
    }

    /// Get sampling rate for a specific event type
    pub fn get_rate(&self, event_type: EventType) -> u32 {
        match event_type {
            EventType::Read | EventType::Write | EventType::Fsync => {
                self.io_rate.load(Ordering::Relaxed)
            }
            EventType::SchedSwitch => self.sched_rate.load(Ordering::Relaxed),
            EventType::PageFault => self.memory_rate.load(Ordering::Relaxed),
            EventType::QueryStart | EventType::QueryEnd => self.query_rate.load(Ordering::Relaxed),
            _ => 1, // Don't sample unknown events
        }
    }

    /// Set sampling rate for a specific event type
    pub fn set_rate(&self, event_type: EventType, rate: u32) {
        let rate = rate.max(1); // Minimum rate is 1 (every event)

        match event_type {
            EventType::Read | EventType::Write | EventType::Fsync => {
                self.io_rate.store(rate, Ordering::Relaxed);
                info!("Set I/O sampling rate to 1/{}", rate);
            }
            EventType::SchedSwitch => {
                self.sched_rate.store(rate, Ordering::Relaxed);
                info!("Set scheduler sampling rate to 1/{}", rate);
            }
            EventType::PageFault => {
                self.memory_rate.store(rate, Ordering::Relaxed);
                info!("Set memory sampling rate to 1/{}", rate);
            }
            EventType::QueryStart | EventType::QueryEnd => {
                self.query_rate.store(rate, Ordering::Relaxed);
                info!("Set query sampling rate to 1/{}", rate);
            }
            _ => {}
        }
    }

    /// Set all rates at once
    pub fn set_all_rates(&self, io: u32, sched: u32, memory: u32, query: u32) {
        self.io_rate.store(io.max(1), Ordering::Relaxed);
        self.sched_rate.store(sched.max(1), Ordering::Relaxed);
        self.memory_rate.store(memory.max(1), Ordering::Relaxed);
        self.query_rate.store(query.max(1), Ordering::Relaxed);

        info!(
            "Set sampling rates: io=1/{}, sched=1/{}, memory=1/{}, query=1/{}",
            io, sched, memory, query
        );
    }

    /// Record event count for adaptive sampling
    pub async fn record_event_count(&self, count: u64) {
        let mut counts = self.recent_event_counts.write().await;
        counts.push(count);

        // Keep only last 60 samples (1 minute at 1Hz)
        if counts.len() > 60 {
            counts.remove(0);
        }
    }

    /// Enable or disable adaptive sampling
    pub async fn set_adaptive(&self, enabled: bool) {
        let mut config = self.config.write().await;
        config.adaptive_enabled = enabled;
        info!(
            "Adaptive sampling: {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// Run adaptive sampling adjustment based on current load
    ///
    /// Should be called periodically (e.g., every 10 seconds)
    pub async fn adjust_for_load(&self) {
        let config = self.config.read().await;
        if !config.adaptive_enabled {
            return;
        }

        let counts = self.recent_event_counts.read().await;
        if counts.is_empty() {
            return;
        }

        // Calculate average events per second
        let avg_events: f64 = counts.iter().sum::<u64>() as f64 / counts.len() as f64;
        let target = config.target_events_per_sec as f64;

        drop(counts);
        drop(config);

        // Calculate adjustment factor
        let ratio = avg_events / target;

        if ratio > 2.0 {
            // Too many events - increase sampling (reduce capture rate)
            let new_io = (self.io_rate.load(Ordering::Relaxed) as f64 * 1.5) as u32;
            let new_sched = (self.sched_rate.load(Ordering::Relaxed) as f64 * 1.5) as u32;

            self.io_rate.store(new_io.min(100), Ordering::Relaxed);
            self.sched_rate.store(new_sched.min(100), Ordering::Relaxed);

            debug!(
                "Adaptive: high load ({:.0} eps), increased sampling to io=1/{}, sched=1/{}",
                avg_events, new_io, new_sched
            );
        } else if ratio < 0.5 {
            // Low event rate - decrease sampling (capture more)
            let new_io = (self.io_rate.load(Ordering::Relaxed) as f64 * 0.75) as u32;
            let new_sched = (self.sched_rate.load(Ordering::Relaxed) as f64 * 0.75) as u32;

            self.io_rate.store(new_io.max(1), Ordering::Relaxed);
            self.sched_rate.store(new_sched.max(1), Ordering::Relaxed);

            debug!(
                "Adaptive: low load ({:.0} eps), decreased sampling to io=1/{}, sched=1/{}",
                avg_events, new_io, new_sched
            );
        }
    }

    /// Get current sampling rates as a vector for updating eBPF maps
    /// Returns [(event_type_key, rate), ...]
    pub fn get_all_rates(&self) -> Vec<(u32, u32)> {
        vec![
            (0, self.io_rate.load(Ordering::Relaxed)),     // I/O events
            (1, self.sched_rate.load(Ordering::Relaxed)),  // Scheduler events
            (2, self.memory_rate.load(Ordering::Relaxed)), // Memory events
            (3, self.query_rate.load(Ordering::Relaxed)),  // Query events
        ]
    }

    /// Start the adaptive sampling loop
    pub async fn run_adaptive_loop(self: Arc<Self>, interval: Duration) {
        let mut timer = tokio::time::interval(interval);

        loop {
            timer.tick().await;
            self.adjust_for_load().await;
        }
    }
}

impl Default for SamplingController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_rates() {
        let controller = SamplingController::new();

        assert_eq!(
            controller.get_rate(EventType::Read),
            DEFAULT_IO_SAMPLING_RATE
        );
        assert_eq!(
            controller.get_rate(EventType::SchedSwitch),
            DEFAULT_SCHED_SAMPLING_RATE
        );
        assert_eq!(
            controller.get_rate(EventType::PageFault),
            DEFAULT_MEMORY_SAMPLING_RATE
        );
    }

    #[test]
    fn test_set_rate() {
        let controller = SamplingController::new();

        controller.set_rate(EventType::Read, 50);
        assert_eq!(controller.get_rate(EventType::Read), 50);
        assert_eq!(controller.get_rate(EventType::Write), 50); // Same category
    }

    #[test]
    fn test_minimum_rate() {
        let controller = SamplingController::new();

        // Setting rate to 0 should clamp to 1
        controller.set_rate(EventType::Read, 0);
        assert_eq!(controller.get_rate(EventType::Read), 1);
    }

    #[tokio::test]
    async fn test_adaptive_sampling() {
        // Start with a higher io_rate so we can observe the increase
        let config = SamplingConfig {
            io_rate: 10,
            ..SamplingConfig::default()
        };
        let controller = SamplingController::with_config(config);

        // Record high event counts (5x the 10,000 target)
        for _ in 0..10 {
            controller.record_event_count(50_000).await;
        }

        controller.adjust_for_load().await;

        // Rates should have increased (more sampling = fewer events captured)
        // 10 * 1.5 = 15
        assert!(controller.get_rate(EventType::Read) > 10);
    }
}
