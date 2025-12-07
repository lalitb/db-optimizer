//! cgroup-aware Container Metrics
//!
//! Provides container-aware resource attribution for PostgreSQL running in:
//! - Docker containers
//! - Kubernetes pods
//! - Other cgroup-based containerization
//!
//! Reads cgroup v1 and v2 metrics to understand resource limits and usage.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Container runtime type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntime {
    /// Docker container
    Docker,
    /// Kubernetes pod (containerd/CRI-O)
    Kubernetes,
    /// Podman container
    Podman,
    /// LXC/LXD container
    Lxc,
    /// systemd slice
    Systemd,
    /// Not in a container
    None,
    /// Unknown container type
    Unknown,
}

impl ContainerRuntime {
    /// Detect container runtime from environment
    pub fn detect() -> Self {
        // Check for Kubernetes
        if std::env::var("KUBERNETES_SERVICE_HOST").is_ok() {
            return ContainerRuntime::Kubernetes;
        }

        // Check for Docker
        if Path::new("/.dockerenv").exists() {
            return ContainerRuntime::Docker;
        }

        // Check cgroup for container hints
        if let Ok(cgroup) = fs::read_to_string("/proc/1/cgroup") {
            if cgroup.contains("docker") {
                return ContainerRuntime::Docker;
            }
            if cgroup.contains("kubepods") {
                return ContainerRuntime::Kubernetes;
            }
            if cgroup.contains("libpod") {
                return ContainerRuntime::Podman;
            }
            if cgroup.contains("lxc") {
                return ContainerRuntime::Lxc;
            }
        }

        // Check for systemd slice
        if let Ok(cgroup) = fs::read_to_string("/proc/self/cgroup") {
            if cgroup.contains(".slice") && !cgroup.contains("docker") {
                return ContainerRuntime::Systemd;
            }
        }

        ContainerRuntime::None
    }
}

/// cgroup version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupVersion {
    V1,
    V2,
    Hybrid,
    Unknown,
}

impl CgroupVersion {
    /// Detect cgroup version
    pub fn detect() -> Self {
        // cgroup v2 unified hierarchy
        if Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
            // Check if v1 controllers also exist
            if Path::new("/sys/fs/cgroup/cpu").exists() {
                return CgroupVersion::Hybrid;
            }
            return CgroupVersion::V2;
        }

        // cgroup v1
        if Path::new("/sys/fs/cgroup/cpu").exists() {
            return CgroupVersion::V1;
        }

        CgroupVersion::Unknown
    }
}

/// CPU limits and usage
#[derive(Debug, Clone, Default)]
pub struct CpuMetrics {
    /// CPU quota in microseconds per period (0 = unlimited)
    pub quota_us: u64,
    /// CPU period in microseconds
    pub period_us: u64,
    /// CPU shares (weight in v2)
    pub shares: u64,
    /// Total CPU usage in nanoseconds
    pub usage_ns: u64,
    /// User CPU time in nanoseconds
    pub user_ns: u64,
    /// System CPU time in nanoseconds
    pub system_ns: u64,
    /// Number of periods with throttling
    pub nr_throttled: u64,
    /// Total throttled time in nanoseconds
    pub throttled_ns: u64,
}

impl CpuMetrics {
    /// Calculate CPU limit as number of cores
    pub fn cpu_limit_cores(&self) -> f64 {
        if self.quota_us == 0 || self.period_us == 0 {
            return f64::MAX; // Unlimited
        }
        self.quota_us as f64 / self.period_us as f64
    }

    /// Calculate throttling percentage
    pub fn throttle_pct(&self) -> f64 {
        if self.nr_throttled == 0 {
            return 0.0;
        }
        // This is a rough estimate
        if self.usage_ns == 0 {
            return 0.0;
        }
        (self.throttled_ns as f64 / self.usage_ns as f64) * 100.0
    }
}

/// Memory limits and usage
#[derive(Debug, Clone, Default)]
pub struct MemoryMetrics {
    /// Memory limit in bytes (0 = unlimited)
    pub limit_bytes: u64,
    /// Current memory usage in bytes
    pub usage_bytes: u64,
    /// Maximum memory usage observed
    pub max_usage_bytes: u64,
    /// Memory + swap limit
    pub memsw_limit_bytes: u64,
    /// Memory + swap usage
    pub memsw_usage_bytes: u64,
    /// Number of times limit was hit
    pub failcnt: u64,
    /// OOM kill count
    pub oom_kill_count: u64,
    /// Cache/page cache bytes
    pub cache_bytes: u64,
    /// RSS (resident set size) bytes
    pub rss_bytes: u64,
    /// Anonymous memory bytes
    pub anon_bytes: u64,
    /// File-backed memory bytes
    pub file_bytes: u64,
}

impl MemoryMetrics {
    /// Memory utilization percentage
    pub fn usage_pct(&self) -> f64 {
        if self.limit_bytes == 0 {
            return 0.0;
        }
        (self.usage_bytes as f64 / self.limit_bytes as f64) * 100.0
    }

    /// Available memory in bytes
    pub fn available_bytes(&self) -> u64 {
        if self.limit_bytes == 0 {
            return u64::MAX;
        }
        self.limit_bytes.saturating_sub(self.usage_bytes)
    }

    /// Check if memory is under pressure
    pub fn is_under_pressure(&self) -> bool {
        self.usage_pct() > 90.0 || self.failcnt > 0
    }
}

/// I/O limits and usage
#[derive(Debug, Clone, Default)]
pub struct IoMetrics {
    /// Read bytes
    pub read_bytes: u64,
    /// Write bytes
    pub write_bytes: u64,
    /// Read operations
    pub read_ios: u64,
    /// Write operations
    pub write_ios: u64,
    /// Read BPS limit (0 = unlimited)
    pub read_bps_limit: u64,
    /// Write BPS limit (0 = unlimited)
    pub write_bps_limit: u64,
    /// Read IOPS limit (0 = unlimited)
    pub read_iops_limit: u64,
    /// Write IOPS limit (0 = unlimited)
    pub write_iops_limit: u64,
}

/// Container resource metrics
#[derive(Debug, Clone, Default)]
pub struct ContainerMetrics {
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub io: IoMetrics,
}

/// cgroup metrics collector
pub struct CgroupCollector {
    /// Detected cgroup version
    cgroup_version: CgroupVersion,
    /// Container runtime
    container_runtime: ContainerRuntime,
    /// Path to this process's cgroup
    cgroup_path: Option<PathBuf>,
    /// Cached metrics
    cached_metrics: Arc<RwLock<ContainerMetrics>>,
    /// Last collection time
    last_collection: Arc<RwLock<std::time::Instant>>,
}

impl CgroupCollector {
    /// Create a new cgroup collector
    pub fn new() -> Self {
        let cgroup_version = CgroupVersion::detect();
        let container_runtime = ContainerRuntime::detect();
        let cgroup_path = Self::detect_cgroup_path(cgroup_version);

        debug!(
            version = ?cgroup_version,
            runtime = ?container_runtime,
            path = ?cgroup_path,
            "Initialized cgroup collector"
        );

        Self {
            cgroup_version,
            container_runtime,
            cgroup_path,
            cached_metrics: Arc::new(RwLock::new(ContainerMetrics::default())),
            last_collection: Arc::new(RwLock::new(std::time::Instant::now())),
        }
    }

    /// Detect the cgroup path for this process
    fn detect_cgroup_path(version: CgroupVersion) -> Option<PathBuf> {
        match version {
            CgroupVersion::V2 => {
                // In cgroup v2, read the unified path
                if let Ok(content) = fs::read_to_string("/proc/self/cgroup") {
                    for line in content.lines() {
                        if line.starts_with("0::") {
                            let path = line.trim_start_matches("0::");
                            return Some(
                                PathBuf::from("/sys/fs/cgroup").join(path.trim_start_matches('/')),
                            );
                        }
                    }
                }
            }
            CgroupVersion::V1 | CgroupVersion::Hybrid => {
                // In cgroup v1, use the memory controller path
                if let Ok(content) = fs::read_to_string("/proc/self/cgroup") {
                    for line in content.lines() {
                        if line.contains("memory") {
                            let parts: Vec<&str> = line.split(':').collect();
                            if parts.len() >= 3 {
                                let path = parts[2];
                                return Some(
                                    PathBuf::from("/sys/fs/cgroup/memory")
                                        .join(path.trim_start_matches('/')),
                                );
                            }
                        }
                    }
                }
            }
            CgroupVersion::Unknown => {}
        }
        None
    }

    /// Get cgroup version
    pub fn cgroup_version(&self) -> CgroupVersion {
        self.cgroup_version
    }

    /// Get container runtime
    pub fn container_runtime(&self) -> ContainerRuntime {
        self.container_runtime
    }

    /// Check if running in a container
    pub fn is_containerized(&self) -> bool {
        !matches!(self.container_runtime, ContainerRuntime::None)
    }

    /// Collect current metrics
    pub async fn collect(&self) -> ContainerMetrics {
        let metrics = match self.cgroup_version {
            CgroupVersion::V2 => self.collect_v2(),
            CgroupVersion::V1 | CgroupVersion::Hybrid => self.collect_v1(),
            CgroupVersion::Unknown => ContainerMetrics::default(),
        };

        // Update cache
        *self.cached_metrics.write().await = metrics.clone();
        *self.last_collection.write().await = std::time::Instant::now();

        metrics
    }

    /// Get cached metrics
    pub async fn get_cached(&self) -> ContainerMetrics {
        self.cached_metrics.read().await.clone()
    }

    /// Collect cgroup v1 metrics
    fn collect_v1(&self) -> ContainerMetrics {
        let mut metrics = ContainerMetrics::default();

        // CPU metrics
        if let Some(cpu) = self.read_v1_cpu() {
            metrics.cpu = cpu;
        }

        // Memory metrics
        if let Some(memory) = self.read_v1_memory() {
            metrics.memory = memory;
        }

        // I/O metrics
        if let Some(io) = self.read_v1_io() {
            metrics.io = io;
        }

        metrics
    }

    /// Collect cgroup v2 metrics
    fn collect_v2(&self) -> ContainerMetrics {
        let mut metrics = ContainerMetrics::default();

        let base_path = match &self.cgroup_path {
            Some(p) => p.clone(),
            None => return metrics,
        };

        // CPU metrics
        if let Some(cpu) = self.read_v2_cpu(&base_path) {
            metrics.cpu = cpu;
        }

        // Memory metrics
        if let Some(memory) = self.read_v2_memory(&base_path) {
            metrics.memory = memory;
        }

        // I/O metrics
        if let Some(io) = self.read_v2_io(&base_path) {
            metrics.io = io;
        }

        metrics
    }

    fn read_v1_cpu(&self) -> Option<CpuMetrics> {
        let mut cpu = CpuMetrics::default();

        // Read quota and period
        if let Ok(quota) = fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_quota_us") {
            cpu.quota_us = quota.trim().parse().unwrap_or(0);
            if cpu.quota_us == u64::MAX || quota.trim() == "-1" {
                cpu.quota_us = 0; // Unlimited
            }
        }
        if let Ok(period) = fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_period_us") {
            cpu.period_us = period.trim().parse().unwrap_or(100_000);
        }

        // Read shares
        if let Ok(shares) = fs::read_to_string("/sys/fs/cgroup/cpu/cpu.shares") {
            cpu.shares = shares.trim().parse().unwrap_or(1024);
        }

        // Read usage
        if let Ok(usage) = fs::read_to_string("/sys/fs/cgroup/cpuacct/cpuacct.usage") {
            cpu.usage_ns = usage.trim().parse().unwrap_or(0);
        }

        // Read throttling stats
        if let Ok(stat) = fs::read_to_string("/sys/fs/cgroup/cpu/cpu.stat") {
            for line in stat.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    match parts[0] {
                        "nr_throttled" => cpu.nr_throttled = parts[1].parse().unwrap_or(0),
                        "throttled_time" => cpu.throttled_ns = parts[1].parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
        }

        Some(cpu)
    }

    fn read_v1_memory(&self) -> Option<MemoryMetrics> {
        let mut memory = MemoryMetrics::default();

        // Read limit
        if let Ok(limit) = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
            memory.limit_bytes = limit.trim().parse().unwrap_or(0);
            // Check for "unlimited" (typically very large number)
            if memory.limit_bytes > 1 << 62 {
                memory.limit_bytes = 0;
            }
        }

        // Read usage
        if let Ok(usage) = fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes") {
            memory.usage_bytes = usage.trim().parse().unwrap_or(0);
        }

        // Read max usage
        if let Ok(max) = fs::read_to_string("/sys/fs/cgroup/memory/memory.max_usage_in_bytes") {
            memory.max_usage_bytes = max.trim().parse().unwrap_or(0);
        }

        // Read failcnt
        if let Ok(failcnt) = fs::read_to_string("/sys/fs/cgroup/memory/memory.failcnt") {
            memory.failcnt = failcnt.trim().parse().unwrap_or(0);
        }

        // Read stat for detailed breakdown
        if let Ok(stat) = fs::read_to_string("/sys/fs/cgroup/memory/memory.stat") {
            for line in stat.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let value: u64 = parts[1].parse().unwrap_or(0);
                    match parts[0] {
                        "cache" => memory.cache_bytes = value,
                        "rss" => memory.rss_bytes = value,
                        _ => {}
                    }
                }
            }
        }

        Some(memory)
    }

    fn read_v1_io(&self) -> Option<IoMetrics> {
        let mut io = IoMetrics::default();

        // Read blkio throttle stats
        if let Ok(stat) = fs::read_to_string("/sys/fs/cgroup/blkio/blkio.throttle.io_service_bytes")
        {
            for line in stat.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let value: u64 = parts[2].parse().unwrap_or(0);
                    match parts[1] {
                        "Read" => io.read_bytes += value,
                        "Write" => io.write_bytes += value,
                        _ => {}
                    }
                }
            }
        }

        Some(io)
    }

    fn read_v2_cpu(&self, base_path: &Path) -> Option<CpuMetrics> {
        let mut cpu = CpuMetrics::default();

        // Read cpu.max (quota and period)
        if let Ok(max) = fs::read_to_string(base_path.join("cpu.max")) {
            let parts: Vec<&str> = max.trim().split_whitespace().collect();
            if parts.len() >= 2 {
                cpu.quota_us = if parts[0] == "max" {
                    0 // Unlimited
                } else {
                    parts[0].parse().unwrap_or(0)
                };
                cpu.period_us = parts[1].parse().unwrap_or(100_000);
            }
        }

        // Read cpu.weight
        if let Ok(weight) = fs::read_to_string(base_path.join("cpu.weight")) {
            cpu.shares = weight.trim().parse().unwrap_or(100);
        }

        // Read cpu.stat
        if let Ok(stat) = fs::read_to_string(base_path.join("cpu.stat")) {
            for line in stat.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let value: u64 = parts[1].parse().unwrap_or(0);
                    match parts[0] {
                        "usage_usec" => cpu.usage_ns = value * 1000,
                        "user_usec" => cpu.user_ns = value * 1000,
                        "system_usec" => cpu.system_ns = value * 1000,
                        "nr_throttled" => cpu.nr_throttled = value,
                        "throttled_usec" => cpu.throttled_ns = value * 1000,
                        _ => {}
                    }
                }
            }
        }

        Some(cpu)
    }

    fn read_v2_memory(&self, base_path: &Path) -> Option<MemoryMetrics> {
        let mut memory = MemoryMetrics::default();

        // Read memory.max
        if let Ok(max) = fs::read_to_string(base_path.join("memory.max")) {
            memory.limit_bytes = if max.trim() == "max" {
                0 // Unlimited
            } else {
                max.trim().parse().unwrap_or(0)
            };
        }

        // Read memory.current
        if let Ok(current) = fs::read_to_string(base_path.join("memory.current")) {
            memory.usage_bytes = current.trim().parse().unwrap_or(0);
        }

        // Read memory.peak (if available)
        if let Ok(peak) = fs::read_to_string(base_path.join("memory.peak")) {
            memory.max_usage_bytes = peak.trim().parse().unwrap_or(0);
        }

        // Read memory.stat
        if let Ok(stat) = fs::read_to_string(base_path.join("memory.stat")) {
            for line in stat.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let value: u64 = parts[1].parse().unwrap_or(0);
                    match parts[0] {
                        "anon" => memory.anon_bytes = value,
                        "file" => memory.file_bytes = value,
                        _ => {}
                    }
                }
            }
        }

        // Read memory.events for OOM info
        if let Ok(events) = fs::read_to_string(base_path.join("memory.events")) {
            for line in events.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[0] == "oom_kill" {
                    memory.oom_kill_count = parts[1].parse().unwrap_or(0);
                }
            }
        }

        Some(memory)
    }

    fn read_v2_io(&self, base_path: &Path) -> Option<IoMetrics> {
        let mut io = IoMetrics::default();

        // Read io.stat
        if let Ok(stat) = fs::read_to_string(base_path.join("io.stat")) {
            for line in stat.lines() {
                // Format: "major:minor rbytes=X wbytes=X rios=X wios=X"
                for part in line.split_whitespace() {
                    if let Some(value) = part.strip_prefix("rbytes=") {
                        io.read_bytes += value.parse::<u64>().unwrap_or(0);
                    } else if let Some(value) = part.strip_prefix("wbytes=") {
                        io.write_bytes += value.parse::<u64>().unwrap_or(0);
                    } else if let Some(value) = part.strip_prefix("rios=") {
                        io.read_ios += value.parse::<u64>().unwrap_or(0);
                    } else if let Some(value) = part.strip_prefix("wios=") {
                        io.write_ios += value.parse::<u64>().unwrap_or(0);
                    }
                }
            }
        }

        Some(io)
    }

    /// Get resource limits summary
    pub async fn get_limits(&self) -> ResourceLimits {
        let metrics = self.collect().await;

        ResourceLimits {
            cpu_cores: metrics.cpu.cpu_limit_cores(),
            memory_bytes: if metrics.memory.limit_bytes > 0 {
                Some(metrics.memory.limit_bytes)
            } else {
                None
            },
            is_containerized: self.is_containerized(),
            container_runtime: self.container_runtime,
        }
    }
}

impl Default for CgroupCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of resource limits
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// CPU limit in cores (f64::MAX if unlimited)
    pub cpu_cores: f64,
    /// Memory limit in bytes (None if unlimited)
    pub memory_bytes: Option<u64>,
    /// Whether running in a container
    pub is_containerized: bool,
    /// Container runtime type
    pub container_runtime: ContainerRuntime,
}

impl ResourceLimits {
    /// Get effective CPU count for PostgreSQL configuration
    pub fn effective_cpu_count(&self) -> usize {
        let limit = self.cpu_cores;
        if limit == f64::MAX {
            num_cpus::get()
        } else {
            (limit.ceil() as usize).max(1)
        }
    }

    /// Get effective memory for PostgreSQL configuration
    pub fn effective_memory_bytes(&self) -> u64 {
        self.memory_bytes.unwrap_or_else(|| {
            // Fallback to system memory
            #[cfg(target_os = "linux")]
            {
                if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
                    for line in meminfo.lines() {
                        if line.starts_with("MemTotal:") {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 2 {
                                if let Ok(kb) = parts[1].parse::<u64>() {
                                    return kb * 1024;
                                }
                            }
                        }
                    }
                }
                0
            }
            #[cfg(not(target_os = "linux"))]
            {
                0
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_runtime_detect() {
        // This will vary based on where tests run
        let runtime = ContainerRuntime::detect();
        // Just ensure it doesn't panic
        println!("Detected runtime: {:?}", runtime);
    }

    #[test]
    fn test_cgroup_version_detect() {
        let version = CgroupVersion::detect();
        println!("Detected cgroup version: {:?}", version);
    }

    #[test]
    fn test_cpu_metrics_calculations() {
        let cpu = CpuMetrics {
            quota_us: 200_000,
            period_us: 100_000,
            nr_throttled: 10,
            throttled_ns: 1_000_000,
            usage_ns: 10_000_000,
            ..Default::default()
        };

        assert!((cpu.cpu_limit_cores() - 2.0).abs() < 0.01);
        assert!(cpu.throttle_pct() > 0.0);
    }

    #[test]
    fn test_memory_metrics_calculations() {
        let memory = MemoryMetrics {
            limit_bytes: 1024 * 1024 * 1024, // 1 GB
            usage_bytes: 512 * 1024 * 1024,  // 512 MB
            ..Default::default()
        };

        assert!((memory.usage_pct() - 50.0).abs() < 0.01);
        assert_eq!(memory.available_bytes(), 512 * 1024 * 1024);
        assert!(!memory.is_under_pressure());
    }

    #[test]
    fn test_unlimited_cpu() {
        let cpu = CpuMetrics {
            quota_us: 0,
            period_us: 100_000,
            ..Default::default()
        };

        assert_eq!(cpu.cpu_limit_cores(), f64::MAX);
    }

    #[tokio::test]
    async fn test_cgroup_collector() {
        let collector = CgroupCollector::new();

        // Just ensure collection doesn't panic
        let metrics = collector.collect().await;
        println!("CPU usage: {} ns", metrics.cpu.usage_ns);
        println!("Memory usage: {} bytes", metrics.memory.usage_bytes);

        let limits = collector.get_limits().await;
        println!("Effective CPU: {}", limits.effective_cpu_count());
    }
}
