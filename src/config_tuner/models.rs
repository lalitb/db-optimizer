// Type-safe models for configuration tuning

use serde::{Deserialize, Serialize};
use std::fmt;

/// System information for configuration calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    /// Total RAM in kilobytes
    pub total_ram_kb: u64,
    /// Number of CPU cores
    pub cpu_count: u32,
    /// Disk type (affects random_page_cost and io_concurrency)
    pub disk_type: DiskType,
    /// Operating system type
    pub os_type: OsType,
    /// PostgreSQL version
    pub pg_version: f32,
}

impl SystemInfo {
    /// Detect system information from the host
    pub fn detect() -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Self::detect_linux()
        }
        #[cfg(target_os = "macos")]
        {
            Self::detect_macos()
        }
        #[cfg(target_os = "windows")]
        {
            Self::detect_windows()
        }
    }

    #[cfg(target_os = "linux")]
    fn detect_linux() -> anyhow::Result<Self> {
        use std::fs;

        // Read total memory from /proc/meminfo
        let meminfo = fs::read_to_string("/proc/meminfo")?;
        let total_ram_kb = meminfo
            .lines()
            .find(|line| line.starts_with("MemTotal:"))
            .and_then(|line| {
                line.split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .ok_or_else(|| anyhow::anyhow!("Failed to parse MemTotal"))?;

        // Get CPU count
        let cpu_count = num_cpus::get() as u32;

        // Detect disk type (simplified - assume SSD for now)
        // In production, you'd check /sys/block/*/queue/rotational
        let disk_type = DiskType::SSD;

        Ok(Self {
            total_ram_kb,
            cpu_count,
            disk_type,
            os_type: OsType::Linux,
            pg_version: 16.0, // Default, should be detected from connection
        })
    }

    #[cfg(target_os = "macos")]
    fn detect_macos() -> anyhow::Result<Self> {
        use std::process::Command;

        // Get total memory using sysctl
        let output = Command::new("sysctl")
            .args(&["-n", "hw.memsize"])
            .output()?;
        let mem_bytes = String::from_utf8(output.stdout)?
            .trim()
            .parse::<u64>()?;
        let total_ram_kb = mem_bytes / 1024;

        // Get CPU count
        let cpu_count = num_cpus::get() as u32;

        Ok(Self {
            total_ram_kb,
            cpu_count,
            disk_type: DiskType::SSD, // Assume SSD on macOS
            os_type: OsType::MacOS,
            pg_version: 16.0,
        })
    }

    #[cfg(target_os = "windows")]
    fn detect_windows() -> anyhow::Result<Self> {
        // Windows detection - simplified
        Ok(Self {
            total_ram_kb: 16 * 1024 * 1024, // Default 16GB
            cpu_count: num_cpus::get() as u32,
            disk_type: DiskType::SSD,
            os_type: OsType::Windows,
            pg_version: 16.0,
        })
    }

    /// Convert RAM to gigabytes for display
    pub fn ram_gb(&self) -> f64 {
        self.total_ram_kb as f64 / (1024.0 * 1024.0)
    }
}

/// PostgreSQL workload type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkloadType {
    /// Web application - mixed workload, moderate concurrency
    Web,
    /// OLTP - high concurrency, short transactions
    OLTP,
    /// Data warehouse - complex queries, low concurrency
    #[serde(rename = "dw")]
    DataWarehouse,
    /// Desktop - local development
    Desktop,
    /// Mixed - general purpose
    Mixed,
}

impl fmt::Display for WorkloadType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Web => write!(f, "Web"),
            Self::OLTP => write!(f, "OLTP"),
            Self::DataWarehouse => write!(f, "Data Warehouse"),
            Self::Desktop => write!(f, "Desktop"),
            Self::Mixed => write!(f, "Mixed"),
        }
    }
}

/// Disk storage type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiskType {
    /// Hard disk drive (spinning disk)
    HDD,
    /// Solid state drive
    SSD,
    /// Storage area network
    SAN,
}

/// Operating system type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsType {
    Linux,
    Windows,
    MacOS,
}

/// Huge pages mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HugePagesMode {
    /// Disable huge pages
    Off,
    /// Try to use huge pages
    Try,
    /// Force huge pages (fail if not available)
    On,
}

impl fmt::Display for HugePagesMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Try => write!(f, "try"),
            Self::On => write!(f, "on"),
        }
    }
}

/// Optimal PostgreSQL configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimalConfig {
    // Memory settings
    pub shared_buffers_kb: u64,
    pub effective_cache_size_kb: u64,
    pub maintenance_work_mem_kb: u64,
    pub work_mem_kb: u64,

    // Connection settings
    pub max_connections: i32,

    // WAL settings
    pub min_wal_size_mb: u32,
    pub max_wal_size_mb: u32,
    pub checkpoint_completion_target: f32,

    // Storage settings
    pub random_page_cost: f32,
    pub effective_io_concurrency: Option<u32>,

    // Advanced settings
    pub huge_pages: HugePagesMode,

    // Metadata
    pub workload_type: WorkloadType,
    pub system_ram_gb: f64,
}

impl OptimalConfig {
    /// Format shared_buffers in human-readable format
    pub fn shared_buffers_display(&self) -> String {
        Self::format_memory_kb(self.shared_buffers_kb)
    }

    /// Format effective_cache_size in human-readable format
    pub fn effective_cache_size_display(&self) -> String {
        Self::format_memory_kb(self.effective_cache_size_kb)
    }

    /// Format maintenance_work_mem in human-readable format
    pub fn maintenance_work_mem_display(&self) -> String {
        Self::format_memory_kb(self.maintenance_work_mem_kb)
    }

    /// Format work_mem in human-readable format
    pub fn work_mem_display(&self) -> String {
        Self::format_memory_kb(self.work_mem_kb)
    }

    /// Format memory size in KB to human-readable string
    fn format_memory_kb(kb: u64) -> String {
        if kb >= 1024 * 1024 {
            format!("{}GB", kb / (1024 * 1024))
        } else if kb >= 1024 {
            format!("{}MB", kb / 1024)
        } else {
            format!("{}kB", kb)
        }
    }

    /// Convert to PostgreSQL configuration format
    pub fn to_postgresql_conf(&self) -> String {
        format!(
            r#"# PostgreSQL Configuration - Generated by db-optimizer
# Workload: {} | System RAM: {:.1}GB

# Memory Settings
shared_buffers = {}
effective_cache_size = {}
maintenance_work_mem = {}
work_mem = {}

# Connection Settings
max_connections = {}

# WAL Settings
min_wal_size = {}MB
max_wal_size = {}MB
checkpoint_completion_target = {}

# Storage Settings
random_page_cost = {}
{}

# Advanced Settings
huge_pages = {}
"#,
            self.workload_type,
            self.system_ram_gb,
            self.shared_buffers_display(),
            self.effective_cache_size_display(),
            self.maintenance_work_mem_display(),
            self.work_mem_display(),
            self.max_connections,
            self.min_wal_size_mb,
            self.max_wal_size_mb,
            self.checkpoint_completion_target,
            self.random_page_cost,
            if let Some(io_conc) = self.effective_io_concurrency {
                format!("effective_io_concurrency = {}", io_conc)
            } else {
                "# effective_io_concurrency not set (not supported on this OS)".to_string()
            },
            self.huge_pages,
        )
    }
}

/// Validation issue severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl fmt::Display for ValidationSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => write!(f, "LOW"),
            Self::Medium => write!(f, "MEDIUM"),
            Self::High => write!(f, "HIGH"),
            Self::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Configuration validation issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: ValidationSeverity,
    pub category: String,
    pub title: String,
    pub description: String,
    pub recommendation: String,
    pub current_value: Option<String>,
    pub recommended_value: Option<String>,
}

impl ValidationIssue {
    /// Create a new critical issue
    pub fn critical(category: impl Into<String>, title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Critical,
            category: category.into(),
            title: title.into(),
            description: description.into(),
            recommendation: String::new(),
            current_value: None,
            recommended_value: None,
        }
    }

    /// Create a new high severity issue
    pub fn high(category: impl Into<String>, title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::High,
            category: category.into(),
            title: title.into(),
            description: description.into(),
            recommendation: String::new(),
            current_value: None,
            recommended_value: None,
        }
    }

    /// Create a new medium severity issue
    pub fn medium(category: impl Into<String>, title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Medium,
            category: category.into(),
            title: title.into(),
            description: description.into(),
            recommendation: String::new(),
            current_value: None,
            recommended_value: None,
        }
    }

    /// Create a new low severity issue
    pub fn low(category: impl Into<String>, title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            severity: ValidationSeverity::Low,
            category: category.into(),
            title: title.into(),
            description: description.into(),
            recommendation: String::new(),
            current_value: None,
            recommended_value: None,
        }
    }

    /// Set recommendation
    pub fn with_recommendation(mut self, recommendation: impl Into<String>) -> Self {
        self.recommendation = recommendation.into();
        self
    }

    /// Set current and recommended values
    pub fn with_values(mut self, current: impl Into<String>, recommended: impl Into<String>) -> Self {
        self.current_value = Some(current.into());
        self.recommended_value = Some(recommended.into());
        self
    }
}

/// Configuration validation report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
    pub total_issues: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
}

impl ValidationReport {
    /// Create a new validation report from issues
    pub fn from_issues(mut issues: Vec<ValidationIssue>) -> Self {
        // Sort by severity (critical first)
        issues.sort_by(|a, b| b.severity.cmp(&a.severity));

        let total_issues = issues.len();
        let critical_count = issues
            .iter()
            .filter(|i| i.severity == ValidationSeverity::Critical)
            .count();
        let high_count = issues
            .iter()
            .filter(|i| i.severity == ValidationSeverity::High)
            .count();
        let medium_count = issues
            .iter()
            .filter(|i| i.severity == ValidationSeverity::Medium)
            .count();
        let low_count = issues
            .iter()
            .filter(|i| i.severity == ValidationSeverity::Low)
            .count();

        Self {
            issues,
            total_issues,
            critical_count,
            high_count,
            medium_count,
            low_count,
        }
    }

    /// Check if configuration is healthy (no critical or high issues)
    pub fn is_healthy(&self) -> bool {
        self.critical_count == 0 && self.high_count == 0
    }

    /// Get summary string
    pub fn summary(&self) -> String {
        if self.total_issues == 0 {
            "✅ Configuration is healthy - no issues found".to_string()
        } else {
            format!(
                "Found {} issues: {} critical, {} high, {} medium, {} low",
                self.total_issues,
                self.critical_count,
                self.high_count,
                self.medium_count,
                self.low_count
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_info_detection() {
        let result = SystemInfo::detect();
        assert!(result.is_ok(), "Should detect system info");

        if let Ok(info) = result {
            assert!(info.total_ram_kb > 0, "Should have RAM");
            assert!(info.cpu_count > 0, "Should have CPUs");
        }
    }

    #[test]
    fn test_workload_type_display() {
        assert_eq!(WorkloadType::Web.to_string(), "Web");
        assert_eq!(WorkloadType::OLTP.to_string(), "OLTP");
        assert_eq!(WorkloadType::DataWarehouse.to_string(), "Data Warehouse");
    }

    #[test]
    fn test_optimal_config_formatting() {
        let config = OptimalConfig {
            shared_buffers_kb: 16 * 1024 * 1024, // 16GB
            effective_cache_size_kb: 48 * 1024 * 1024, // 48GB
            maintenance_work_mem_kb: 2 * 1024 * 1024, // 2GB
            work_mem_kb: 64 * 1024, // 64MB
            max_connections: 200,
            min_wal_size_mb: 1024,
            max_wal_size_mb: 4096,
            checkpoint_completion_target: 0.9,
            random_page_cost: 1.1,
            effective_io_concurrency: Some(200),
            huge_pages: HugePagesMode::Try,
            workload_type: WorkloadType::Web,
            system_ram_gb: 64.0,
        };

        assert_eq!(config.shared_buffers_display(), "16GB");
        assert_eq!(config.work_mem_display(), "64MB");

        let pg_conf = config.to_postgresql_conf();
        assert!(pg_conf.contains("shared_buffers = 16GB"));
        assert!(pg_conf.contains("max_connections = 200"));
    }

    #[test]
    fn test_validation_report() {
        let issues = vec![
            ValidationIssue::critical("safety", "fsync off", "Data loss risk"),
            ValidationIssue::high("memory", "shared_buffers low", "Performance impact"),
            ValidationIssue::medium("connection", "High utilization", "May need pooler"),
        ];

        let report = ValidationReport::from_issues(issues);
        assert_eq!(report.total_issues, 3);
        assert_eq!(report.critical_count, 1);
        assert_eq!(report.high_count, 1);
        assert_eq!(report.medium_count, 1);
        assert!(!report.is_healthy());
    }
}
