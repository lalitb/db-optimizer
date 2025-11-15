use serde::{Deserialize, Serialize};
use anyhow::{Result, Context, bail};
use std::collections::HashMap;
use std::path::Path;
use chrono::{Weekday, Utc, Timelike, Datelike};

/// Main application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Database connection settings
    pub database: DatabaseConfig,
    
    /// Operation mode for the optimizer
    pub operation_mode: OperationMode,
    
    /// Analysis configuration
    pub analysis: AnalysisConfig,
    
    /// Safety and governance rules
    pub safety: SafetyConfig,
    
    /// Maintenance window configuration
    #[serde(default)]
    pub maintenance_window: MaintenanceWindow,
    
    /// eBPF configuration
    #[serde(default)]
    pub ebpf: EbpfConfig,
    
    /// Auto-apply configuration
    #[serde(default)]
    pub auto_apply: AutoApplyConfigExtended,
    
    /// Persistence configuration
    #[serde(default)]
    pub persistence: PersistenceConfig,
    
    /// HTTP server configuration
    #[serde(default)]
    pub http_server: HttpServerConfig,
    
    /// Notification configuration
    #[serde(default)]
    pub notifications: NotificationConfig,
    
    /// Guardrails configuration for system resource limits
    #[serde(default)]
    pub guardrails: GuardrailsConfig,
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database: DatabaseConfig::from_env()?,
            operation_mode: OperationMode::from_env(),
            analysis: AnalysisConfig::from_env(),
            safety: SafetyConfig::default(),
            maintenance_window: MaintenanceWindow::default(),
            ebpf: EbpfConfig::default(),
            auto_apply: AutoApplyConfigExtended::default(),
            persistence: PersistenceConfig::default(),
            http_server: HttpServerConfig::default(),
            notifications: NotificationConfig::default(),
            guardrails: GuardrailsConfig::default(),
        })
    }
    
    /// Load configuration from a YAML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .context("Failed to read config file")?;
        
        let config: Self = serde_yaml::from_str(&content)
            .context("Failed to parse config file")?;
        
        config.validate()?;
        Ok(config)
    }
    
    /// Load configuration from YAML file with environment variable overrides
    pub fn from_file_with_env<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut config = Self::from_file(path)?;
        config.apply_env_overrides()?;
        config.validate()?;
        Ok(config)
    }
    
    /// Apply environment variable overrides with DBO_ prefix
    fn apply_env_overrides(&mut self) -> Result<()> {
        // Database overrides
        if let Ok(url) = std::env::var("DBO_DATABASE_URL") {
            self.database.url = url;
        } else if let Ok(url) = std::env::var("DATABASE_URL") {
            self.database.url = url;
        }
        
        // Operation mode override
        if let Ok(mode) = std::env::var("DBO_OPERATION_MODE") {
            self.operation_mode = mode.parse()?;
        } else if let Ok(mode) = std::env::var("OPERATION_MODE") {
            self.operation_mode = mode.parse()?;
        }
        
        // Slack webhook override
        if let Ok(webhook) = std::env::var("DBO_SLACK_WEBHOOK") {
            self.notifications.slack_webhook = Some(webhook);
        } else if let Ok(webhook) = std::env::var("SLACK_WEBHOOK") {
            self.notifications.slack_webhook = Some(webhook);
        }
        
        // Guardrails overrides
        if let Ok(enabled) = std::env::var("DBO_GUARDRAILS_ENABLED") {
            self.guardrails.enabled = enabled.parse().unwrap_or(true);
        }
        
        if let Ok(mem) = std::env::var("DBO_GUARDRAILS_MEMORY_THRESHOLD") {
            if let Ok(threshold) = mem.parse() {
                self.guardrails.memory_threshold_percent = threshold;
            }
        }
        
        if let Ok(cpu) = std::env::var("DBO_GUARDRAILS_CPU_THRESHOLD") {
            if let Ok(threshold) = cpu.parse() {
                self.guardrails.cpu_threshold_percent = threshold;
            }
        }
        
        if let Ok(disk) = std::env::var("DBO_GUARDRAILS_DISK_THRESHOLD") {
            if let Ok(threshold) = disk.parse() {
                self.guardrails.disk_threshold_percent = threshold;
            }
        }
        
        if let Ok(conn) = std::env::var("DBO_GUARDRAILS_MAX_CONNECTIONS") {
            if let Ok(max) = conn.parse() {
                self.guardrails.max_connections_threshold = max;
            }
        }
        
        // eBPF override
        if let Ok(enabled) = std::env::var("DBO_EBPF_ENABLED") {
            self.ebpf.enabled = enabled.parse().unwrap_or(false);
        }
        
        // HTTP server overrides
        if let Ok(enabled) = std::env::var("DBO_HTTP_ENABLED") {
            self.http_server.enabled = enabled.parse().unwrap_or(false);
        }
        
        if let Ok(port) = std::env::var("DBO_HTTP_PORT") {
            if let Ok(p) = port.parse() {
                self.http_server.port = p;
            }
        }
        
        Ok(())
    }
    
    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        // Validate database URL is not empty
        if self.database.url.is_empty() {
            bail!("Database URL cannot be empty");
        }
        
        // Validate safety limits
        if self.safety.max_index_size_mb <= 0.0 {
            bail!("max_index_size_mb must be positive");
        }
        
        if self.safety.max_ddl_lock_time_secs == 0 {
            bail!("max_ddl_lock_time_secs must be positive");
        }
        
        // Validate eBPF config
        if self.ebpf.enabled && self.ebpf.sampling_interval_ms == 0 {
            bail!("eBPF sampling_interval_ms must be positive when enabled");
        }
        
        // Validate auto-apply thresholds
        if self.auto_apply.base.min_savings_threshold < 0.0 {
            bail!("min_savings_threshold cannot be negative");
        }
        
        if self.auto_apply.base.min_improvement_threshold <= 0.0 || self.auto_apply.base.min_improvement_threshold > 100.0 {
            bail!("min_improvement_threshold must be between 0 and 100");
        }
        
        // Validate maintenance window
        if self.maintenance_window.enabled {
            let (start, end) = self.maintenance_window.allowed_hours_utc;
            if start > 23 || end > 23 {
                bail!("Maintenance window hours must be 0-23");
            }
        }
        
        // Validate guardrails
        if self.guardrails.enabled {
            if self.guardrails.memory_threshold_percent > 100 {
                bail!("Memory threshold cannot exceed 100%");
            }
            if self.guardrails.cpu_threshold_percent > 100 {
                bail!("CPU threshold cannot exceed 100%");
            }
            if self.guardrails.disk_threshold_percent > 100 {
                bail!("Disk threshold cannot exceed 100%");
            }
        }
        
        Ok(())
    }
    
    /// Check if we are currently in a maintenance window
    pub fn in_maintenance_window(&self) -> bool {
        if !self.maintenance_window.enabled {
            return true; // No restrictions if disabled
        }
        
        let now = Utc::now();
        let current_hour = now.hour() as u8;
        let current_day = now.weekday();
        
        // Check if current day is allowed
        if !self.maintenance_window.allowed_days.contains(&current_day) {
            return false;
        }
        
        // Check if current hour is in range
        let (start, end) = self.maintenance_window.allowed_hours_utc;
        if start <= end {
            current_hour >= start && current_hour <= end
        } else {
            // Handle ranges that cross midnight
            current_hour >= start || current_hour <= end
        }
    }
}

/// Database connection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// PostgreSQL connection string
    pub url: String,
    
    /// Maximum number of connections in the pool
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    
    /// Connection timeout in seconds
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout_secs: u64,
}

impl DatabaseConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            url: std::env::var("DATABASE_URL")
                .context("DATABASE_URL environment variable not set")?,
            max_connections: default_max_connections(),
            connection_timeout_secs: default_connection_timeout(),
        })
    }
}

fn default_max_connections() -> u32 { 5 }
fn default_connection_timeout() -> u64 { 30 }

/// Operation mode for the optimizer
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationMode {
    /// Read-only analysis, no changes to database
    ObserveOnly,
    
    /// Auto-apply only safe operations (e.g., CREATE INDEX CONCURRENTLY)
    SafeAuto,
    
    /// More aggressive optimization (for staging or explicit opt-in)
    /// WARNING: Use with caution in production
    Aggressive,
}

impl OperationMode {
    pub fn from_env() -> Self {
        std::env::var("OPERATION_MODE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(Self::ObserveOnly)
    }
    
    pub fn allows_auto_apply(&self) -> bool {
        matches!(self, Self::SafeAuto | Self::Aggressive)
    }
    
    pub fn is_aggressive(&self) -> bool {
        matches!(self, Self::Aggressive)
    }
}

impl std::str::FromStr for OperationMode {
    type Err = anyhow::Error;
    
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "observe_only" | "observe" => Ok(Self::ObserveOnly),
            "safe_auto" | "safe" => Ok(Self::SafeAuto),
            "aggressive" | "aggro" => Ok(Self::Aggressive),
            _ => bail!("Invalid operation mode: {}. Valid options: observe_only, safe_auto, aggressive", s),
        }
    }
}

impl std::fmt::Display for OperationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ObserveOnly => write!(f, "observe_only"),
            Self::SafeAuto => write!(f, "safe_auto"),
            Self::Aggressive => write!(f, "aggressive"),
        }
    }
}

/// Analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    /// Number of top queries to analyze
    #[serde(default = "default_query_limit")]
    pub query_limit: i64,
    
    /// Database compute cost per hour (USD) for cost estimation
    #[serde(default = "default_cost_per_hour")]
    pub cost_per_hour_usd: f64,
    
    /// Minimum severity to include in reports
    #[serde(default)]
    pub min_severity: crate::models::Severity,
}

impl AnalysisConfig {
    pub fn from_env() -> Self {
        Self {
            query_limit: std::env::var("QUERY_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(default_query_limit),
            cost_per_hour_usd: std::env::var("COST_PER_HOUR")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(default_cost_per_hour),
            min_severity: crate::models::Severity::Low,
        }
    }
}

fn default_query_limit() -> i64 { 100 }
fn default_cost_per_hour() -> f64 { 0.07 }

/// Safety and governance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    /// Schemas that cannot be modified by auto-apply
    #[serde(default)]
    pub protected_schemas: Vec<String>,
    
    /// Tables that cannot be modified by auto-apply
    #[serde(default)]
    pub protected_tables: Vec<String>,
    
    /// Columns that cannot be modified (table -> columns)
    #[serde(default)]
    pub protected_columns: HashMap<String, Vec<String>>,
    
    /// Maximum allowed index size in MB
    #[serde(default = "default_max_index_size")]
    pub max_index_size_mb: f64,
    
    /// Maximum allowed DDL lock time in seconds
    #[serde(default = "default_max_ddl_lock_time")]
    pub max_ddl_lock_time_secs: u64,
    
    /// Never modify primary keys
    #[serde(default = "default_true")]
    pub protect_primary_keys: bool,
    
    /// Never modify unique constraints
    #[serde(default = "default_true")]
    pub protect_unique_constraints: bool,
    
    /// Require explicit approval for foreign key changes
    #[serde(default = "default_true")]
    pub protect_foreign_keys: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            protected_schemas: vec!["pg_catalog".to_string(), "information_schema".to_string()],
            protected_tables: vec![],
            protected_columns: HashMap::new(),
            max_index_size_mb: default_max_index_size(),
            max_ddl_lock_time_secs: default_max_ddl_lock_time(),
            protect_primary_keys: true,
            protect_unique_constraints: true,
            protect_foreign_keys: true,
        }
    }
}

fn default_max_index_size() -> f64 { 10_000.0 } // 10 GB
fn default_max_ddl_lock_time() -> u64 { 300 } // 5 minutes
fn default_true() -> bool { true }

/// Maintenance window configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    /// Whether maintenance window is enabled
    #[serde(default)]
    pub enabled: bool,
    
    /// Allowed days of week for DDL operations
    #[serde(default = "default_all_days")]
    pub allowed_days: Vec<Weekday>,
    
    /// Allowed hours (UTC) for DDL operations (start, end inclusive)
    #[serde(default = "default_hours")]
    pub allowed_hours_utc: (u8, u8),
}

impl Default for MaintenanceWindow {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_days: default_all_days(),
            allowed_hours_utc: default_hours(),
        }
    }
}

fn default_all_days() -> Vec<Weekday> {
    vec![
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ]
}

fn default_hours() -> (u8, u8) {
    (0, 23) // All hours
}

/// eBPF configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EbpfConfig {
    /// Whether eBPF collection is enabled
    #[serde(default)]
    pub enabled: bool,
    
    /// Sampling interval in milliseconds
    #[serde(default = "default_sampling_interval")]
    pub sampling_interval_ms: u64,
    
    /// Collect CPU metrics
    #[serde(default = "default_true")]
    pub collect_cpu_metrics: bool,
    
    /// Collect I/O metrics
    #[serde(default = "default_true")]
    pub collect_io_metrics: bool,
    
    /// Collect memory metrics
    #[serde(default = "default_true")]
    pub collect_memory_metrics: bool,
    
    /// Collect lock/contention metrics
    #[serde(default = "default_true")]
    pub collect_lock_metrics: bool,
    
    /// Maximum events to buffer before dropping
    #[serde(default = "default_max_events")]
    pub max_events_buffer: usize,
}

impl Default for EbpfConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default (requires privileges)
            sampling_interval_ms: default_sampling_interval(),
            collect_cpu_metrics: true,
            collect_io_metrics: true,
            collect_memory_metrics: true,
            collect_lock_metrics: true,
            max_events_buffer: default_max_events(),
        }
    }
}

fn default_sampling_interval() -> u64 { 100 } // 100ms
fn default_max_events() -> usize { 10_000 }

/// Extended auto-apply configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoApplyConfigExtended {
    /// Base auto-apply config (from models.rs)
    #[serde(flatten)]
    pub base: crate::models::AutoApplyConfig,
    
    /// Custom success criteria beyond default improvement threshold
    #[serde(default)]
    pub custom_success_criteria: Vec<SuccessCriterion>,
    
    /// Soak period in seconds before declaring success
    #[serde(default = "default_soak_period")]
    pub soak_period_secs: u64,
    
    /// Maximum concurrent auto-apply operations
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_applies: usize,
}

impl Default for AutoApplyConfigExtended {
    fn default() -> Self {
        Self {
            base: crate::models::AutoApplyConfig::default(),
            custom_success_criteria: vec![],
            soak_period_secs: default_soak_period(),
            max_concurrent_applies: default_max_concurrent(),
        }
    }
}

fn default_soak_period() -> u64 { 60 } // 1 minute
fn default_max_concurrent() -> usize { 1 } // One at a time by default

/// Custom success criterion for auto-apply verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessCriterion {
    /// Metric to evaluate
    pub metric: SuccessMetric,
    
    /// Minimum improvement required (percentage)
    pub min_improvement_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessMetric {
    /// Mean execution time
    MeanTime,
    
    /// P95 execution time
    P95Time,
    
    /// P99 execution time
    P99Time,
    
    /// Mean latency (alias for MeanTime)
    MeanLatency,
    
    /// P95 latency (alias for P95Time)
    P95Latency,
    
    /// Throughput (queries per second)
    Throughput,
    
    /// Error rate (percentage)
    ErrorRate,
    
    /// CPU time (from eBPF)
    CpuTime,
    
    /// IO wait time (from eBPF)
    IoWaitTime,
    
    /// IO wait (alias for IoWaitTime)
    IoWait,
}

/// Persistence configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceConfig {
    /// Backend type
    #[serde(default)]
    pub backend: PersistenceBackend,
    
    /// SQLite database path (if backend is SQLite)
    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: String,
    
    /// Whether to use the same Postgres DB for persistence (if backend is Postgres)
    #[serde(default = "default_true")]
    pub use_same_db: bool,
    
    /// Separate Postgres URL for persistence (if use_same_db is false)
    pub persistence_db_url: Option<String>,
    
    /// Retention period for query samples in days
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            backend: PersistenceBackend::Sqlite,
            sqlite_path: default_sqlite_path(),
            use_same_db: true,
            persistence_db_url: None,
            retention_days: default_retention_days(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PersistenceBackend {
    /// No persistence
    None,
    
    /// SQLite embedded database
    Sqlite,
    
    /// PostgreSQL database
    Postgres,
}

impl Default for PersistenceBackend {
    fn default() -> Self {
        Self::Sqlite
    }
}

fn default_sqlite_path() -> String {
    "~/.db-optimizer/state.db".to_string()
}

fn default_retention_days() -> u32 { 30 }

/// HTTP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpServerConfig {
    /// Whether HTTP server is enabled
    #[serde(default)]
    pub enabled: bool,
    
    /// Bind address
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    
    /// Port
    #[serde(default = "default_port")]
    pub port: u16,
    
    /// Optional bearer token for authentication
    pub auth_token: Option<String>,
    
    /// Enable CORS
    #[serde(default = "default_true")]
    pub enable_cors: bool,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: default_bind_address(),
            port: default_port(),
            auth_token: None,
            enable_cors: true,
        }
    }
}

fn default_bind_address() -> String { "127.0.0.1".to_string() }
fn default_port() -> u16 { 9090 }

/// Notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Slack webhook URL
    pub slack_webhook: Option<String>,
    
    /// Email configuration (future)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<EmailConfig>,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            slack_webhook: None,
            email: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub from: String,
    pub to: Vec<String>,
}

/// Guardrails configuration for system resource limits
/// Prevents auto-apply operations when system resources are under stress
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    /// Whether guardrails are enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    
    /// Memory usage threshold (percentage). Stop auto-apply if exceeded.
    #[serde(default = "default_memory_threshold")]
    pub memory_threshold_percent: u8,
    
    /// CPU usage threshold (percentage). Stop auto-apply if exceeded.
    #[serde(default = "default_cpu_threshold")]
    pub cpu_threshold_percent: u8,
    
    /// Disk usage threshold (percentage). Stop auto-apply if exceeded.
    #[serde(default = "default_disk_threshold")]
    pub disk_threshold_percent: u8,
    
    /// Maximum active connections threshold. Stop auto-apply if exceeded.
    #[serde(default = "default_max_connections_threshold")]
    pub max_connections_threshold: u32,
    
    /// Check interval in seconds
    #[serde(default = "default_guardrail_check_interval")]
    pub check_interval_secs: u64,
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            memory_threshold_percent: default_memory_threshold(),
            cpu_threshold_percent: default_cpu_threshold(),
            disk_threshold_percent: default_disk_threshold(),
            max_connections_threshold: default_max_connections_threshold(),
            check_interval_secs: default_guardrail_check_interval(),
        }
    }
}

fn default_memory_threshold() -> u8 { 90 }
fn default_cpu_threshold() -> u8 { 85 }
fn default_disk_threshold() -> u8 { 95 }
fn default_max_connections_threshold() -> u32 { 1000 }
fn default_guardrail_check_interval() -> u64 { 30 }

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_operation_mode_parse() {
        assert_eq!("observe_only".parse::<OperationMode>().unwrap(), OperationMode::ObserveOnly);
        assert_eq!("safe_auto".parse::<OperationMode>().unwrap(), OperationMode::SafeAuto);
        assert_eq!("aggressive".parse::<OperationMode>().unwrap(), OperationMode::Aggressive);
        assert!("invalid".parse::<OperationMode>().is_err());
    }
    
    #[test]
    fn test_default_config_validation() {
        let config = Config {
            database: DatabaseConfig {
                url: "postgresql://localhost/test".to_string(),
                max_connections: 5,
                connection_timeout_secs: 30,
            },
            operation_mode: OperationMode::ObserveOnly,
            analysis: AnalysisConfig {
                query_limit: 100,
                cost_per_hour_usd: 0.07,
                min_severity: crate::models::Severity::Low,
            },
            safety: SafetyConfig::default(),
            maintenance_window: MaintenanceWindow::default(),
            ebpf: EbpfConfig::default(),
            auto_apply: AutoApplyConfigExtended::default(),
            persistence: PersistenceConfig::default(),
            http_server: HttpServerConfig::default(),
            notifications: NotificationConfig::default(),
            guardrails: GuardrailsConfig::default(),
        };
        
        assert!(config.validate().is_ok());
    }
    
    #[test]
    fn test_maintenance_window_all_allowed() {
        let config = Config {
            database: DatabaseConfig {
                url: "postgresql://localhost/test".to_string(),
                max_connections: 5,
                connection_timeout_secs: 30,
            },
            operation_mode: OperationMode::ObserveOnly,
            analysis: AnalysisConfig::from_env(),
            safety: SafetyConfig::default(),
            maintenance_window: MaintenanceWindow {
                enabled: false,
                allowed_days: default_all_days(),
                allowed_hours_utc: (0, 23),
            },
            ebpf: EbpfConfig::default(),
            auto_apply: AutoApplyConfigExtended::default(),
            persistence: PersistenceConfig::default(),
            http_server: HttpServerConfig::default(),
            notifications: NotificationConfig::default(),
            guardrails: GuardrailsConfig::default(),
        };
        
        // Should always be in maintenance window when disabled
        assert!(config.in_maintenance_window());
    }
    
    #[test]
    fn test_guardrails_defaults() {
        let guardrails = GuardrailsConfig::default();
        assert_eq!(guardrails.enabled, true);
        assert_eq!(guardrails.memory_threshold_percent, 90);
        assert_eq!(guardrails.cpu_threshold_percent, 85);
        assert_eq!(guardrails.disk_threshold_percent, 95);
        assert_eq!(guardrails.max_connections_threshold, 1000);
    }
    
    #[test]
    fn test_env_var_prefix() {
        // Test that DBO_ prefix works
        std::env::set_var("DBO_OPERATION_MODE", "safe_auto");
        let mut config = Config {
            database: DatabaseConfig {
                url: "postgresql://localhost/test".to_string(),
                max_connections: 5,
                connection_timeout_secs: 30,
            },
            operation_mode: OperationMode::ObserveOnly,
            analysis: AnalysisConfig::from_env(),
            safety: SafetyConfig::default(),
            maintenance_window: MaintenanceWindow::default(),
            ebpf: EbpfConfig::default(),
            auto_apply: AutoApplyConfigExtended::default(),
            persistence: PersistenceConfig::default(),
            http_server: HttpServerConfig::default(),
            notifications: NotificationConfig::default(),
            guardrails: GuardrailsConfig::default(),
        };
        
        config.apply_env_overrides().unwrap();
        assert_eq!(config.operation_mode, OperationMode::SafeAuto);
        std::env::remove_var("DBO_OPERATION_MODE");
    }
}
