//! Live eBPF Map Updates
//!
//! Provides hot-reload capability for eBPF configuration:
//! - Update sampling rates without restarting
//! - Modify filter patterns on the fly
//! - Add/remove PIDs to monitor
//! - Dynamic threshold adjustments

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Map update operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapOperation {
    /// Insert a new key-value pair
    Insert,
    /// Update an existing key-value pair
    Update,
    /// Delete a key
    Delete,
    /// Update or insert (upsert)
    Upsert,
}

/// Result of a map update
#[derive(Debug, Clone)]
pub struct MapUpdateResult {
    /// Whether the update succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Time taken for the update
    pub duration: Duration,
}

/// Configuration for live map updates
#[derive(Debug, Clone)]
pub struct LiveMapConfig {
    /// Whether live updates are enabled
    pub enabled: bool,
    /// Batch updates for efficiency
    pub batch_updates: bool,
    /// Maximum batch size
    pub max_batch_size: usize,
    /// Batch timeout
    pub batch_timeout: Duration,
    /// Rate limit for updates (per second)
    pub rate_limit: f64,
}

impl Default for LiveMapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            batch_updates: true,
            max_batch_size: 100,
            batch_timeout: Duration::from_millis(100),
            rate_limit: 100.0,
        }
    }
}

/// Sampling rate configuration
#[derive(Debug, Clone)]
pub struct SamplingConfig {
    /// Base sampling rate (0.0 - 1.0)
    pub base_rate: f64,
    /// Minimum sampling rate
    pub min_rate: f64,
    /// Maximum sampling rate
    pub max_rate: f64,
    /// Adaptive sampling enabled
    pub adaptive: bool,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            base_rate: 1.0,
            min_rate: 0.01,
            max_rate: 1.0,
            adaptive: true,
        }
    }
}

/// PID filter configuration
#[derive(Debug, Clone)]
pub struct PidFilterConfig {
    /// PIDs to include (empty = include all)
    pub include_pids: Vec<u32>,
    /// PIDs to exclude
    pub exclude_pids: Vec<u32>,
    /// Process name patterns to include
    pub include_patterns: Vec<String>,
    /// Process name patterns to exclude
    pub exclude_patterns: Vec<String>,
}

impl Default for PidFilterConfig {
    fn default() -> Self {
        Self {
            include_pids: Vec::new(),
            exclude_pids: Vec::new(),
            include_patterns: vec!["postgres".to_string(), "postmaster".to_string()],
            exclude_patterns: Vec::new(),
        }
    }
}

/// Latency threshold configuration
#[derive(Debug, Clone)]
pub struct LatencyThresholds {
    /// Fast I/O threshold (microseconds)
    pub fast_us: u64,
    /// Normal I/O threshold (microseconds)
    pub normal_us: u64,
    /// Slow I/O threshold (microseconds)
    pub slow_us: u64,
    /// Very slow I/O threshold (microseconds)
    pub very_slow_us: u64,
}

impl Default for LatencyThresholds {
    fn default() -> Self {
        Self {
            fast_us: 100,
            normal_us: 1000,
            slow_us: 10000,
            very_slow_us: 100000,
        }
    }
}

/// A pending map update
#[derive(Debug, Clone)]
struct PendingUpdate {
    map_name: String,
    key: Vec<u8>,
    value: Option<Vec<u8>>,
    operation: MapOperation,
    queued_at: Instant,
}

/// Live eBPF map manager
pub struct LiveMapManager {
    /// Configuration
    config: Arc<RwLock<LiveMapConfig>>,
    /// Current sampling config
    sampling: Arc<RwLock<SamplingConfig>>,
    /// PID filter config
    pid_filter: Arc<RwLock<PidFilterConfig>>,
    /// Latency thresholds
    thresholds: Arc<RwLock<LatencyThresholds>>,
    /// Pending updates (for batching)
    pending_updates: Arc<RwLock<Vec<PendingUpdate>>>,
    /// Update history for debugging
    update_history: Arc<RwLock<Vec<MapUpdateRecord>>>,
    /// Rate limiter state
    last_update: Arc<RwLock<Instant>>,
    /// Statistics
    stats: Arc<RwLock<MapUpdateStats>>,
    /// Whether daemon is connected
    daemon_connected: Arc<RwLock<bool>>,
}

/// Record of a map update
#[derive(Debug, Clone)]
pub struct MapUpdateRecord {
    /// When the update was performed
    pub timestamp: Instant,
    /// Map name
    pub map_name: String,
    /// Operation type
    pub operation: MapOperation,
    /// Whether successful
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// Statistics about map updates
#[derive(Debug, Clone, Default)]
pub struct MapUpdateStats {
    /// Total updates attempted
    pub total_updates: u64,
    /// Successful updates
    pub successful_updates: u64,
    /// Failed updates
    pub failed_updates: u64,
    /// Rate-limited updates
    pub rate_limited: u64,
    /// Batched updates
    pub batched_updates: u64,
    /// Average update latency
    pub avg_latency_us: f64,
}

impl LiveMapManager {
    /// Create a new live map manager
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(LiveMapConfig::default())),
            sampling: Arc::new(RwLock::new(SamplingConfig::default())),
            pid_filter: Arc::new(RwLock::new(PidFilterConfig::default())),
            thresholds: Arc::new(RwLock::new(LatencyThresholds::default())),
            pending_updates: Arc::new(RwLock::new(Vec::new())),
            update_history: Arc::new(RwLock::new(Vec::new())),
            last_update: Arc::new(RwLock::new(Instant::now())),
            stats: Arc::new(RwLock::new(MapUpdateStats::default())),
            daemon_connected: Arc::new(RwLock::new(false)),
        }
    }

    /// Set daemon connection status
    pub async fn set_daemon_connected(&self, connected: bool) {
        *self.daemon_connected.write().await = connected;
    }

    /// Update sampling rate
    pub async fn update_sampling_rate(&self, rate: f64) -> MapUpdateResult {
        let start = Instant::now();

        // Validate rate
        let config = self.sampling.read().await;
        let clamped_rate = rate.clamp(config.min_rate, config.max_rate);
        drop(config);

        // Update local config
        {
            let mut sampling = self.sampling.write().await;
            sampling.base_rate = clamped_rate;
        }

        // Queue update to eBPF map
        self.queue_map_update(
            "sampling_rate",
            vec![0u8; 4], // Key 0
            Some(clamped_rate.to_le_bytes().to_vec()),
            MapOperation::Upsert,
        )
        .await;

        // Flush if not batching
        let config = self.config.read().await;
        if !config.batch_updates {
            drop(config);
            let _ = self.flush_pending_updates().await;
        }

        info!(rate = clamped_rate, "Updated sampling rate");

        MapUpdateResult {
            success: true,
            error: None,
            duration: start.elapsed(),
        }
    }

    /// Add a PID to monitor
    pub async fn add_pid(&self, pid: u32) -> MapUpdateResult {
        let start = Instant::now();

        // Update local config
        {
            let mut filter = self.pid_filter.write().await;
            if !filter.include_pids.contains(&pid) {
                filter.include_pids.push(pid);
            }
        }

        // Queue update to eBPF map
        self.queue_map_update(
            "pid_filter",
            pid.to_le_bytes().to_vec(),
            Some(vec![1u8]), // 1 = include
            MapOperation::Upsert,
        )
        .await;

        debug!(pid = pid, "Added PID to filter");

        MapUpdateResult {
            success: true,
            error: None,
            duration: start.elapsed(),
        }
    }

    /// Remove a PID from monitoring
    pub async fn remove_pid(&self, pid: u32) -> MapUpdateResult {
        let start = Instant::now();

        // Update local config
        {
            let mut filter = self.pid_filter.write().await;
            filter.include_pids.retain(|&p| p != pid);
        }

        // Queue update to eBPF map
        self.queue_map_update(
            "pid_filter",
            pid.to_le_bytes().to_vec(),
            None,
            MapOperation::Delete,
        )
        .await;

        debug!(pid = pid, "Removed PID from filter");

        MapUpdateResult {
            success: true,
            error: None,
            duration: start.elapsed(),
        }
    }

    /// Update latency thresholds
    pub async fn update_thresholds(&self, thresholds: LatencyThresholds) -> MapUpdateResult {
        let start = Instant::now();

        // Update local config
        *self.thresholds.write().await = thresholds.clone();

        // Queue updates for each threshold
        let threshold_values = [
            (0u8, thresholds.fast_us),
            (1u8, thresholds.normal_us),
            (2u8, thresholds.slow_us),
            (3u8, thresholds.very_slow_us),
        ];

        for (key, value) in threshold_values {
            self.queue_map_update(
                "latency_thresholds",
                vec![key],
                Some(value.to_le_bytes().to_vec()),
                MapOperation::Upsert,
            )
            .await;
        }

        info!("Updated latency thresholds");

        MapUpdateResult {
            success: true,
            error: None,
            duration: start.elapsed(),
        }
    }

    /// Queue a map update
    async fn queue_map_update(
        &self,
        map_name: &str,
        key: Vec<u8>,
        value: Option<Vec<u8>>,
        operation: MapOperation,
    ) {
        let update = PendingUpdate {
            map_name: map_name.to_string(),
            key,
            value,
            operation,
            queued_at: Instant::now(),
        };

        let mut pending = self.pending_updates.write().await;
        pending.push(update);

        // Check if we should flush
        let config = self.config.read().await;
        if pending.len() >= config.max_batch_size {
            drop(pending);
            drop(config);
            let _ = self.flush_pending_updates().await;
        }
    }

    /// Flush pending updates to eBPF
    pub async fn flush_pending_updates(&self) -> Vec<MapUpdateResult> {
        let mut pending = self.pending_updates.write().await;
        if pending.is_empty() {
            return Vec::new();
        }

        let updates: Vec<PendingUpdate> = pending.drain(..).collect();
        drop(pending);

        // Check rate limit
        let can_update = self.check_rate_limit().await;
        if !can_update {
            let mut stats = self.stats.write().await;
            stats.rate_limited += updates.len() as u64;
            return updates
                .iter()
                .map(|_| MapUpdateResult {
                    success: false,
                    error: Some("Rate limited".to_string()),
                    duration: Duration::ZERO,
                })
                .collect();
        }

        // Check if daemon is connected
        let connected = *self.daemon_connected.read().await;

        let mut results = Vec::new();

        for update in updates {
            let start = Instant::now();

            let result = if connected {
                self.apply_map_update(&update).await
            } else {
                // In mock mode, just succeed
                debug!(
                    map = update.map_name,
                    operation = ?update.operation,
                    "Mock map update (daemon not connected)"
                );
                MapUpdateResult {
                    success: true,
                    error: None,
                    duration: start.elapsed(),
                }
            };

            // Record history
            let record = MapUpdateRecord {
                timestamp: Instant::now(),
                map_name: update.map_name,
                operation: update.operation,
                success: result.success,
                error: result.error.clone(),
            };

            {
                let mut history = self.update_history.write().await;
                if history.len() >= 1000 {
                    history.remove(0);
                }
                history.push(record);
            }

            // Update stats
            {
                let mut stats = self.stats.write().await;
                stats.total_updates += 1;
                if result.success {
                    stats.successful_updates += 1;
                } else {
                    stats.failed_updates += 1;
                }

                // Update average latency (exponential moving average)
                let latency_us = result.duration.as_micros() as f64;
                stats.avg_latency_us = stats.avg_latency_us * 0.9 + latency_us * 0.1;
            }

            results.push(result);
        }

        results
    }

    /// Apply a map update to eBPF
    async fn apply_map_update(&self, update: &PendingUpdate) -> MapUpdateResult {
        let start = Instant::now();

        // In a real implementation, this would communicate with the eBPF daemon
        // via the Unix socket to update the map
        //
        // For now, we simulate the update

        #[cfg(target_os = "linux")]
        {
            // Would use libbpf to update the map
            // bpf_map_update_elem(map_fd, &key, &value, BPF_ANY);
        }

        debug!(
            map = update.map_name,
            operation = ?update.operation,
            key_len = update.key.len(),
            "Applied map update"
        );

        MapUpdateResult {
            success: true,
            error: None,
            duration: start.elapsed(),
        }
    }

    /// Check rate limit
    async fn check_rate_limit(&self) -> bool {
        let config = self.config.read().await;
        let min_interval = Duration::from_secs_f64(1.0 / config.rate_limit);
        drop(config);

        let mut last = self.last_update.write().await;
        let elapsed = last.elapsed();

        if elapsed >= min_interval {
            *last = Instant::now();
            true
        } else {
            false
        }
    }

    /// Get current sampling config
    pub async fn get_sampling_config(&self) -> SamplingConfig {
        self.sampling.read().await.clone()
    }

    /// Get current PID filter config
    pub async fn get_pid_filter_config(&self) -> PidFilterConfig {
        self.pid_filter.read().await.clone()
    }

    /// Get current latency thresholds
    pub async fn get_thresholds(&self) -> LatencyThresholds {
        self.thresholds.read().await.clone()
    }

    /// Get update statistics
    pub async fn get_stats(&self) -> MapUpdateStats {
        self.stats.read().await.clone()
    }

    /// Get update history
    pub async fn get_history(&self) -> Vec<MapUpdateRecord> {
        self.update_history.read().await.clone()
    }

    /// Enable adaptive sampling
    pub async fn enable_adaptive_sampling(&self, enabled: bool) {
        let mut sampling = self.sampling.write().await;
        sampling.adaptive = enabled;
    }

    /// Adjust sampling based on load
    pub async fn adjust_sampling_for_load(&self, events_per_second: f64, drop_rate: f64) {
        let sampling = self.sampling.read().await;
        if !sampling.adaptive {
            return;
        }
        let current_rate = sampling.base_rate;
        let min_rate = sampling.min_rate;
        let max_rate = sampling.max_rate;
        drop(sampling);

        // Adaptive algorithm:
        // - If drop rate > 5%, reduce sampling
        // - If events are low and drop rate is 0, increase sampling
        let new_rate = if drop_rate > 0.05 {
            // Reduce sampling rate
            (current_rate * 0.8).max(min_rate)
        } else if events_per_second < 1000.0 && drop_rate < 0.01 {
            // Can increase sampling
            (current_rate * 1.1).min(max_rate)
        } else {
            current_rate
        };

        if (new_rate - current_rate).abs() > 0.01 {
            let _ = self.update_sampling_rate(new_rate).await;
            debug!(
                old_rate = current_rate,
                new_rate = new_rate,
                drop_rate = drop_rate,
                "Adjusted sampling rate"
            );
        }
    }
}

impl Default for LiveMapManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sampling_rate_update() {
        let manager = LiveMapManager::new();

        let result = manager.update_sampling_rate(0.5).await;
        assert!(result.success);

        let config = manager.get_sampling_config().await;
        assert_eq!(config.base_rate, 0.5);
    }

    #[tokio::test]
    async fn test_sampling_rate_clamping() {
        let manager = LiveMapManager::new();

        // Try to set rate above max
        let _ = manager.update_sampling_rate(2.0).await;
        let config = manager.get_sampling_config().await;
        assert_eq!(config.base_rate, 1.0); // Clamped to max

        // Try to set rate below min
        let _ = manager.update_sampling_rate(0.001).await;
        let config = manager.get_sampling_config().await;
        assert_eq!(config.base_rate, 0.01); // Clamped to min
    }

    #[tokio::test]
    async fn test_pid_management() {
        let manager = LiveMapManager::new();

        // Add PID
        let result = manager.add_pid(1234).await;
        assert!(result.success);

        let filter = manager.get_pid_filter_config().await;
        assert!(filter.include_pids.contains(&1234));

        // Remove PID
        let result = manager.remove_pid(1234).await;
        assert!(result.success);

        let filter = manager.get_pid_filter_config().await;
        assert!(!filter.include_pids.contains(&1234));
    }

    #[tokio::test]
    async fn test_threshold_update() {
        let manager = LiveMapManager::new();

        let thresholds = LatencyThresholds {
            fast_us: 50,
            normal_us: 500,
            slow_us: 5000,
            very_slow_us: 50000,
        };

        let result = manager.update_thresholds(thresholds.clone()).await;
        assert!(result.success);

        let current = manager.get_thresholds().await;
        assert_eq!(current.fast_us, 50);
        assert_eq!(current.very_slow_us, 50000);
    }

    #[tokio::test]
    async fn test_adaptive_sampling() {
        let manager = LiveMapManager::new();

        // Enable adaptive sampling
        manager.enable_adaptive_sampling(true).await;

        // High drop rate should reduce sampling
        let _ = manager.update_sampling_rate(1.0).await;
        manager.adjust_sampling_for_load(10000.0, 0.10).await;

        let config = manager.get_sampling_config().await;
        assert!(config.base_rate < 1.0);
    }

    #[tokio::test]
    async fn test_stats_tracking() {
        let manager = LiveMapManager::new();
        *manager.daemon_connected.write().await = false;

        // Reset the rate limiter to allow immediate updates
        *manager.last_update.write().await = Instant::now() - std::time::Duration::from_secs(1);

        // Perform some updates
        let _ = manager.update_sampling_rate(0.5).await;
        let _ = manager.add_pid(1234).await;
        let _ = manager.flush_pending_updates().await;

        let stats = manager.get_stats().await;
        assert!(stats.total_updates > 0);
    }
}
