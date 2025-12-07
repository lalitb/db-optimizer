//! Graceful Degradation
//!
//! Provides automatic fallback when eBPF is unavailable or degraded:
//! - Detect eBPF availability and capabilities
//! - Fall back to pg_stat_* only mode
//! - Maintain functionality with reduced precision
//! - Report degradation to operators

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// eBPF capability level (ordered from worst to best for comparison)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityLevel {
    /// Completely disabled
    Disabled,
    /// pg_stat_* only (no kernel metrics)
    PgStatOnly,
    /// eBPF available but daemon not running
    DaemonOnly,
    /// Limited eBPF (some probes unavailable)
    Limited,
    /// Full eBPF support with all features
    Full,
}

impl CapabilityLevel {
    /// Human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::Full => "Full eBPF support with kernel-level metrics",
            Self::Limited => "Limited eBPF support (some features unavailable)",
            Self::DaemonOnly => "eBPF daemon available but not fully attached",
            Self::PgStatOnly => "PostgreSQL statistics only (no kernel metrics)",
            Self::Disabled => "Metrics collection disabled",
        }
    }

    /// Whether kernel I/O metrics are available
    pub fn has_io_metrics(&self) -> bool {
        matches!(self, Self::Full | Self::Limited)
    }

    /// Whether lock contention tracking is available
    pub fn has_lock_metrics(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether pg_stat correlation is available
    pub fn has_pg_stat(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Confidence multiplier for recommendations
    pub fn confidence_multiplier(&self) -> f64 {
        match self {
            Self::Full => 1.0,
            Self::Limited => 0.8,
            Self::DaemonOnly => 0.6,
            Self::PgStatOnly => 0.5,
            Self::Disabled => 0.0,
        }
    }
}

/// Reason for degradation
#[derive(Debug, Clone)]
pub enum DegradationReason {
    /// eBPF not supported on this kernel
    KernelNotSupported(String),
    /// Missing CAP_BPF or CAP_SYS_ADMIN
    InsufficientPermissions,
    /// BTF (BPF Type Format) not available
    NoBtf,
    /// eBPF daemon not running
    DaemonNotRunning,
    /// Connection to daemon lost
    DaemonConnectionLost,
    /// Probe attachment failed
    ProbeAttachFailed(String),
    /// High event drop rate
    HighDropRate(f64),
    /// PostgreSQL connection failed
    PgConnectionFailed(String),
    /// User disabled via configuration
    UserDisabled,
    /// Running in container without privileges
    ContainerRestricted,
    /// Unknown error
    Unknown(String),
}

impl DegradationReason {
    /// Suggested remediation
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::KernelNotSupported(_) => "Upgrade to Linux kernel 5.8+ for full eBPF support",
            Self::InsufficientPermissions => {
                "Run with CAP_BPF and CAP_PERFMON capabilities, or use the privileged daemon"
            }
            Self::NoBtf => {
                "Enable CONFIG_DEBUG_INFO_BTF in kernel config or install kernel-debuginfo"
            }
            Self::DaemonNotRunning => {
                "Start the db-optimizer-ebpf daemon: systemctl start db-optimizer-ebpf"
            }
            Self::DaemonConnectionLost => "Check daemon status and restart if needed",
            Self::ProbeAttachFailed(_) => {
                "Check kernel version compatibility or use a different probe type"
            }
            Self::HighDropRate(_) => "Increase buffer size or reduce sampling rate",
            Self::PgConnectionFailed(_) => "Check PostgreSQL connection settings and credentials",
            Self::UserDisabled => "Enable eBPF in configuration: ebpf.enabled = true",
            Self::ContainerRestricted => {
                "Run container with --privileged or use host PID namespace"
            }
            Self::Unknown(_) => "Check logs for more details",
        }
    }
}

/// Degradation event
#[derive(Debug, Clone)]
pub struct DegradationEvent {
    /// When the degradation occurred
    pub timestamp: Instant,
    /// Previous capability level
    pub from_level: CapabilityLevel,
    /// New capability level
    pub to_level: CapabilityLevel,
    /// Reason for degradation
    pub reason: DegradationReason,
}

/// Recovery event
#[derive(Debug, Clone)]
pub struct RecoveryEvent {
    /// When recovery occurred
    pub timestamp: Instant,
    /// Previous capability level
    pub from_level: CapabilityLevel,
    /// New capability level
    pub to_level: CapabilityLevel,
    /// How long we were degraded
    pub degraded_duration: Duration,
}

/// Graceful degradation manager
pub struct DegradationManager {
    /// Current capability level
    current_level: Arc<RwLock<CapabilityLevel>>,
    /// Target (desired) capability level
    target_level: Arc<RwLock<CapabilityLevel>>,
    /// Active degradation reasons
    active_reasons: Arc<RwLock<Vec<DegradationReason>>>,
    /// Degradation history
    degradation_history: Arc<RwLock<Vec<DegradationEvent>>>,
    /// Recovery history
    recovery_history: Arc<RwLock<Vec<RecoveryEvent>>>,
    /// When current degradation started
    degradation_start: Arc<RwLock<Option<Instant>>>,
    /// Maximum history size
    max_history: usize,
    /// Callbacks for degradation events
    on_degrade: Arc<RwLock<Option<Box<dyn Fn(DegradationEvent) + Send + Sync>>>>,
    /// Callbacks for recovery events
    on_recover: Arc<RwLock<Option<Box<dyn Fn(RecoveryEvent) + Send + Sync>>>>,
}

impl DegradationManager {
    /// Create a new degradation manager
    pub fn new() -> Self {
        Self {
            current_level: Arc::new(RwLock::new(CapabilityLevel::Disabled)),
            target_level: Arc::new(RwLock::new(CapabilityLevel::Full)),
            active_reasons: Arc::new(RwLock::new(Vec::new())),
            degradation_history: Arc::new(RwLock::new(Vec::new())),
            recovery_history: Arc::new(RwLock::new(Vec::new())),
            degradation_start: Arc::new(RwLock::new(None)),
            max_history: 100,
            on_degrade: Arc::new(RwLock::new(None)),
            on_recover: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the target capability level
    pub async fn set_target_level(&self, level: CapabilityLevel) {
        *self.target_level.write().await = level;
    }

    /// Get current capability level
    pub async fn current_level(&self) -> CapabilityLevel {
        *self.current_level.read().await
    }

    /// Get target capability level
    pub async fn target_level(&self) -> CapabilityLevel {
        *self.target_level.read().await
    }

    /// Check if currently degraded (below target)
    pub async fn is_degraded(&self) -> bool {
        let current = *self.current_level.read().await;
        let target = *self.target_level.read().await;
        current < target
    }

    /// Report a degradation
    pub async fn report_degradation(&self, reason: DegradationReason) {
        let from_level = *self.current_level.read().await;

        // Add reason if not already present
        {
            let mut reasons = self.active_reasons.write().await;
            let reason_str = format!("{:?}", reason);
            if !reasons.iter().any(|r| format!("{:?}", r) == reason_str) {
                reasons.push(reason.clone());
            }
        }

        // Calculate new level AFTER adding reason
        let new_level = self.calculate_level_for_reasons().await;

        // Update level if changed
        if new_level != from_level {
            *self.current_level.write().await = new_level;

            // Record degradation start
            if self.degradation_start.read().await.is_none() {
                *self.degradation_start.write().await = Some(Instant::now());
            }

            let event = DegradationEvent {
                timestamp: Instant::now(),
                from_level,
                to_level: new_level,
                reason: reason.clone(),
            };

            // Store in history
            {
                let mut history = self.degradation_history.write().await;
                if history.len() >= self.max_history {
                    history.remove(0);
                }
                history.push(event.clone());
            }

            // Call callback
            if let Some(callback) = self.on_degrade.read().await.as_ref() {
                callback(event.clone());
            }

            warn!(
                from = ?from_level,
                to = ?new_level,
                reason = ?reason,
                "Capability degradation detected"
            );
        }
    }

    /// Clear a degradation reason (e.g., daemon reconnected)
    pub async fn clear_reason(&self, reason_type: &str) {
        let mut reasons = self.active_reasons.write().await;
        reasons.retain(|r| !format!("{:?}", r).starts_with(reason_type));
        drop(reasons);

        // Recalculate level
        self.try_recover().await;
    }

    /// Try to recover to higher capability level
    pub async fn try_recover(&self) -> bool {
        let from_level = *self.current_level.read().await;
        let new_level = self.calculate_level_for_reasons().await;
        let target = *self.target_level.read().await;

        if new_level > from_level && new_level <= target {
            *self.current_level.write().await = new_level;

            let degraded_duration = self
                .degradation_start
                .read()
                .await
                .map(|s| s.elapsed())
                .unwrap_or(Duration::ZERO);

            // Clear degradation start if fully recovered
            if new_level >= target {
                *self.degradation_start.write().await = None;
            }

            let event = RecoveryEvent {
                timestamp: Instant::now(),
                from_level,
                to_level: new_level,
                degraded_duration,
            };

            // Store in history
            {
                let mut history = self.recovery_history.write().await;
                if history.len() >= self.max_history {
                    history.remove(0);
                }
                history.push(event.clone());
            }

            // Call callback
            if let Some(callback) = self.on_recover.read().await.as_ref() {
                callback(event.clone());
            }

            info!(
                from = ?from_level,
                to = ?new_level,
                "Capability recovered"
            );

            return true;
        }

        false
    }

    /// Calculate capability level based on active reasons
    async fn calculate_level_for_reasons(&self) -> CapabilityLevel {
        let reasons = self.active_reasons.read().await;

        if reasons.is_empty() {
            return CapabilityLevel::Full;
        }

        // Find the most severe degradation
        let mut level = CapabilityLevel::Full;

        for reason in reasons.iter() {
            let reason_level = match reason {
                DegradationReason::UserDisabled => CapabilityLevel::Disabled,
                DegradationReason::KernelNotSupported(_) => CapabilityLevel::PgStatOnly,
                DegradationReason::InsufficientPermissions => CapabilityLevel::PgStatOnly,
                DegradationReason::NoBtf => CapabilityLevel::PgStatOnly,
                DegradationReason::DaemonNotRunning => CapabilityLevel::DaemonOnly,
                DegradationReason::DaemonConnectionLost => CapabilityLevel::PgStatOnly,
                DegradationReason::ContainerRestricted => CapabilityLevel::PgStatOnly,
                DegradationReason::ProbeAttachFailed(_) => CapabilityLevel::Limited,
                DegradationReason::HighDropRate(_) => CapabilityLevel::Limited,
                DegradationReason::PgConnectionFailed(_) => CapabilityLevel::Limited,
                DegradationReason::Unknown(_) => CapabilityLevel::Limited,
            };

            if reason_level < level {
                level = reason_level;
            }
        }

        level
    }

    /// Get active degradation reasons
    pub async fn get_reasons(&self) -> Vec<DegradationReason> {
        self.active_reasons.read().await.clone()
    }

    /// Get degradation history
    pub async fn get_degradation_history(&self) -> Vec<DegradationEvent> {
        self.degradation_history.read().await.clone()
    }

    /// Get recovery history
    pub async fn get_recovery_history(&self) -> Vec<RecoveryEvent> {
        self.recovery_history.read().await.clone()
    }

    /// Get degraded duration
    pub async fn degraded_duration(&self) -> Option<Duration> {
        self.degradation_start.read().await.map(|s| s.elapsed())
    }

    /// Set callback for degradation events
    pub async fn on_degradation<F>(&self, callback: F)
    where
        F: Fn(DegradationEvent) + Send + Sync + 'static,
    {
        *self.on_degrade.write().await = Some(Box::new(callback));
    }

    /// Set callback for recovery events
    pub async fn on_recovery<F>(&self, callback: F)
    where
        F: Fn(RecoveryEvent) + Send + Sync + 'static,
    {
        *self.on_recover.write().await = Some(Box::new(callback));
    }

    /// Get status summary
    pub async fn get_status(&self) -> DegradationStatus {
        let current_level = *self.current_level.read().await;
        let target_level = *self.target_level.read().await;
        let reasons = self.active_reasons.read().await.clone();

        DegradationStatus {
            current_level,
            target_level,
            is_degraded: current_level < target_level,
            active_reasons: reasons,
            degraded_since: *self.degradation_start.read().await,
        }
    }

    /// Initialize and detect capabilities
    pub async fn initialize(&self) {
        info!("Initializing degradation manager");

        // Start at disabled
        *self.current_level.write().await = CapabilityLevel::Disabled;

        // Detect capabilities
        let detected = self.detect_capabilities().await;

        // Set to detected level (may still be limited by reasons)
        *self.current_level.write().await = detected;

        info!(level = ?detected, "Initial capability level detected");
    }

    /// Detect available capabilities
    async fn detect_capabilities(&self) -> CapabilityLevel {
        // Check if eBPF is available
        #[cfg(target_os = "linux")]
        {
            use std::fs;

            // Check kernel version
            if let Ok(version) = fs::read_to_string("/proc/version") {
                // Extract major.minor version
                if let Some(v) = version.split_whitespace().nth(2) {
                    let parts: Vec<&str> = v.split('.').collect();
                    if parts.len() >= 2 {
                        let major: u32 = parts[0].parse().unwrap_or(0);
                        let minor: u32 = parts[1].parse().unwrap_or(0);

                        if major < 5 || (major == 5 && minor < 8) {
                            self.report_degradation(DegradationReason::KernelNotSupported(
                                format!("Kernel {}.{} detected, 5.8+ required", major, minor),
                            ))
                            .await;
                            return CapabilityLevel::PgStatOnly;
                        }
                    }
                }
            }

            // Check for BTF
            if !std::path::Path::new("/sys/kernel/btf/vmlinux").exists() {
                self.report_degradation(DegradationReason::NoBtf).await;
                return CapabilityLevel::PgStatOnly;
            }

            // Check capabilities (simplified check)
            // In reality, we'd try to load a simple BPF program
            if !std::path::Path::new("/sys/kernel/debug/tracing").exists() {
                self.report_degradation(DegradationReason::InsufficientPermissions)
                    .await;
                return CapabilityLevel::PgStatOnly;
            }

            CapabilityLevel::Full
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.report_degradation(DegradationReason::KernelNotSupported(
                "eBPF only supported on Linux".to_string(),
            ))
            .await;
            CapabilityLevel::PgStatOnly
        }
    }
}

impl Default for DegradationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Degradation status summary
#[derive(Debug, Clone)]
pub struct DegradationStatus {
    /// Current capability level
    pub current_level: CapabilityLevel,
    /// Target capability level
    pub target_level: CapabilityLevel,
    /// Whether currently degraded
    pub is_degraded: bool,
    /// Active degradation reasons
    pub active_reasons: Vec<DegradationReason>,
    /// When degradation started
    pub degraded_since: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_degradation_lifecycle() {
        let manager = DegradationManager::new();

        // Start at full
        *manager.current_level.write().await = CapabilityLevel::Full;

        assert!(!manager.is_degraded().await);

        // Report degradation
        manager
            .report_degradation(DegradationReason::DaemonConnectionLost)
            .await;

        assert!(manager.is_degraded().await);
        assert!(manager.current_level().await < CapabilityLevel::Full);

        // Clear reason
        manager.clear_reason("DaemonConnectionLost").await;

        // Should recover
        assert!(!manager.is_degraded().await);
    }

    #[tokio::test]
    async fn test_multiple_reasons() {
        let manager = DegradationManager::new();
        *manager.current_level.write().await = CapabilityLevel::Full;

        manager
            .report_degradation(DegradationReason::HighDropRate(0.15))
            .await;
        manager
            .report_degradation(DegradationReason::ProbeAttachFailed("uprobe".to_string()))
            .await;

        // Most severe should win
        assert_eq!(manager.current_level().await, CapabilityLevel::Limited);

        let reasons = manager.get_reasons().await;
        assert_eq!(reasons.len(), 2);
    }

    #[tokio::test]
    async fn test_capability_level_ordering() {
        assert!(CapabilityLevel::Full > CapabilityLevel::Limited);
        assert!(CapabilityLevel::Limited > CapabilityLevel::DaemonOnly);
        assert!(CapabilityLevel::DaemonOnly > CapabilityLevel::PgStatOnly);
        assert!(CapabilityLevel::PgStatOnly > CapabilityLevel::Disabled);
    }

    #[test]
    fn test_capability_features() {
        assert!(CapabilityLevel::Full.has_io_metrics());
        assert!(CapabilityLevel::Full.has_lock_metrics());
        assert!(CapabilityLevel::Limited.has_io_metrics());
        assert!(!CapabilityLevel::Limited.has_lock_metrics());
        assert!(!CapabilityLevel::PgStatOnly.has_io_metrics());
        assert!(CapabilityLevel::PgStatOnly.has_pg_stat());
    }

    #[test]
    fn test_remediation_suggestions() {
        let reason = DegradationReason::DaemonNotRunning;
        assert!(reason.remediation().contains("systemctl"));
    }
}
