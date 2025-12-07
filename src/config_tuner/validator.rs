// Configuration validator - implements PostgreSQLTuner validation rules
#![allow(clippy::needless_borrow)]
#![allow(clippy::needless_borrows_for_generic_args)]

use super::models::{SystemInfo, ValidationIssue, ValidationReport};
#[cfg(test)]
use super::models::ValidationSeverity;
use anyhow::Result;
use sqlx::{PgPool, Row};

/// Configuration validator
pub struct ConfigValidator {
    safety: SafetyValidator,
    memory: MemoryValidator,
    connection: ConnectionValidator,
}

impl ConfigValidator {
    pub fn new() -> Self {
        Self {
            safety: SafetyValidator,
            memory: MemoryValidator,
            connection: ConnectionValidator,
        }
    }

    /// Validate current PostgreSQL configuration
    pub async fn validate(&self, pool: &PgPool) -> Result<ValidationReport> {
        let mut all_issues = Vec::new();

        // Get current config
        let config = self.fetch_current_config(pool).await?;
        let system_info = SystemInfo::detect()?;

        // Run all validators
        all_issues.extend(self.safety.validate(&config));
        all_issues.extend(self.memory.validate(&config, &system_info));
        all_issues.extend(self.connection.validate_config(&config));

        // Get runtime stats for connection validator
        if let Ok(conn_issues) = self.connection.validate_runtime(pool).await {
            all_issues.extend(conn_issues);
        }

        Ok(ValidationReport::from_issues(all_issues))
    }

    /// Fetch current PostgreSQL configuration
    async fn fetch_current_config(&self, pool: &PgPool) -> Result<PgConfig> {
        let rows = sqlx::query(
            r#"
            SELECT name, setting, unit
            FROM pg_settings
            WHERE name IN (
                'shared_buffers',
                'effective_cache_size',
                'maintenance_work_mem',
                'work_mem',
                'max_connections',
                'fsync',
                'full_page_writes',
                'synchronous_commit',
                'checkpoint_completion_target',
                'log_min_duration_statement',
                'log_checkpoints',
                'superuser_reserved_connections'
            )
            "#,
        )
        .fetch_all(pool)
        .await?;

        let mut config = PgConfig::default();

        for row in rows {
            let name: String = row.try_get("name")?;
            let setting: String = row.try_get("setting")?;
            let unit: Option<String> = row.try_get("unit").ok();

            match name.as_str() {
                "shared_buffers" => {
                    config.shared_buffers_kb = Self::parse_memory(&setting, unit.as_deref())?
                }
                "effective_cache_size" => {
                    config.effective_cache_size_kb = Self::parse_memory(&setting, unit.as_deref())?
                }
                "maintenance_work_mem" => {
                    config.maintenance_work_mem_kb = Self::parse_memory(&setting, unit.as_deref())?
                }
                "work_mem" => config.work_mem_kb = Self::parse_memory(&setting, unit.as_deref())?,
                "max_connections" => config.max_connections = setting.parse()?,
                "fsync" => config.fsync = setting == "on",
                "full_page_writes" => config.full_page_writes = setting == "on",
                "synchronous_commit" => config.synchronous_commit = setting,
                "checkpoint_completion_target" => {
                    config.checkpoint_completion_target = setting.parse()?
                }
                "log_min_duration_statement" => {
                    config.log_min_duration_statement = if setting == "-1" {
                        None
                    } else {
                        Some(setting.parse()?)
                    };
                }
                "log_checkpoints" => config.log_checkpoints = setting == "on",
                "superuser_reserved_connections" => {
                    config.superuser_reserved_connections = setting.parse()?
                }
                _ => {}
            }
        }

        Ok(config)
    }

    /// Parse memory setting with unit
    fn parse_memory(value: &str, unit: Option<&str>) -> Result<u64> {
        let num: u64 = value.parse()?;
        let multiplier = match unit {
            Some("kB") => 1,
            Some("MB") => 1024,
            Some("GB") => 1024 * 1024,
            Some("8kB") => 8,
            Some("16kB") => 16,
            _ => 1, // Default to KB
        };
        Ok(num * multiplier)
    }
}

impl Default for ConfigValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// PostgreSQL configuration snapshot
#[derive(Debug, Clone, Default)]
pub struct PgConfig {
    pub shared_buffers_kb: u64,
    pub effective_cache_size_kb: u64,
    pub maintenance_work_mem_kb: u64,
    pub work_mem_kb: u64,
    pub max_connections: i32,
    pub superuser_reserved_connections: i32,
    pub fsync: bool,
    pub full_page_writes: bool,
    pub synchronous_commit: String,
    pub checkpoint_completion_target: f32,
    pub log_min_duration_statement: Option<i32>,
    pub log_checkpoints: bool,
}

/// Safety validator - checks for dangerous settings
struct SafetyValidator;

impl SafetyValidator {
    fn validate(&self, config: &PgConfig) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        // CRITICAL: fsync
        if !config.fsync {
            issues.push(
                ValidationIssue::critical(
                    "safety",
                    "fsync disabled",
                    "DATA LOSS RISK! Database may lose data after a crash.",
                )
                .with_recommendation("SET fsync = on IMMEDIATELY")
                .with_values("off", "on"),
            );
        }

        // CRITICAL: full_page_writes
        if !config.full_page_writes {
            issues.push(
                ValidationIssue::critical(
                    "safety",
                    "full_page_writes disabled",
                    "Data corruption risk after crash",
                )
                .with_recommendation("Enable full_page_writes immediately")
                .with_values("off", "on"),
            );
        }

        // HIGH: synchronous_commit
        if config.synchronous_commit == "off" {
            issues.push(
                ValidationIssue::high(
                    "safety",
                    "synchronous_commit off",
                    "Recent transactions may be lost after crash",
                )
                .with_recommendation("Consider 'local' or 'on' for durability")
                .with_values("off", "local or on"),
            );
        }

        // MEDIUM: slow query logging
        if config.log_min_duration_statement.is_none() {
            issues.push(
                ValidationIssue::medium(
                    "observability",
                    "Slow query logging disabled",
                    "Cannot identify performance issues",
                )
                .with_recommendation("Set log_min_duration_statement = 1000 (ms)")
                .with_values("-1 (disabled)", "1000"),
            );
        }

        // LOW: checkpoint logging
        if !config.log_checkpoints {
            issues.push(
                ValidationIssue::low(
                    "observability",
                    "Checkpoint logging disabled",
                    "Cannot monitor checkpoint performance",
                )
                .with_recommendation("Enable log_checkpoints")
                .with_values("off", "on"),
            );
        }

        // MEDIUM: checkpoint_completion_target
        if config.checkpoint_completion_target < 0.8 {
            issues.push(
                ValidationIssue::medium(
                    "performance",
                    "checkpoint_completion_target too low",
                    "Checkpoint I/O not spread out enough, may cause I/O spikes",
                )
                .with_recommendation("Increase to 0.9 to spread I/O over time")
                .with_values(&config.checkpoint_completion_target.to_string(), "0.9"),
            );
        }

        issues
    }
}

/// Memory validator - checks memory configuration
struct MemoryValidator;

impl MemoryValidator {
    fn validate(&self, config: &PgConfig, system: &SystemInfo) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        // Check shared_buffers ratio
        let sb_ratio = config.shared_buffers_kb as f64 / system.total_ram_kb as f64;
        if sb_ratio < 0.25 {
            issues.push(
                ValidationIssue::medium(
                    "memory",
                    "shared_buffers too low",
                    format!("Using {:.1}% of RAM, recommend 25-40%", sb_ratio * 100.0),
                )
                .with_recommendation("Increase shared_buffers to 25-40% of RAM")
                .with_values(&format!("{:.1}%", sb_ratio * 100.0), "25-40%"),
            );
        } else if sb_ratio > 0.40 {
            issues.push(
                ValidationIssue::high(
                    "memory",
                    "shared_buffers too high",
                    format!("Using {:.1}% of RAM, may cause swapping", sb_ratio * 100.0),
                )
                .with_recommendation("Reduce shared_buffers to 25-40% of RAM")
                .with_values(&format!("{:.1}%", sb_ratio * 100.0), "25-40%"),
            );
        }

        // Check total memory usage (simplified)
        let estimated_usage = config.shared_buffers_kb
            + (config.work_mem_kb * config.max_connections as u64)
            + (config.maintenance_work_mem_kb * 2); // Assume 2 maintenance workers

        let usage_ratio = estimated_usage as f64 / system.total_ram_kb as f64;
        if usage_ratio > 0.60 {
            issues.push(
                ValidationIssue::critical(
                    "memory",
                    "Risk of OOM (Out of Memory)",
                    format!(
                        "PostgreSQL may use {:.1}% of RAM under peak load",
                        usage_ratio * 100.0
                    ),
                )
                .with_recommendation("Reduce work_mem or max_connections")
                .with_values(&format!("{:.1}%", usage_ratio * 100.0), "<60%"),
            );
        }

        // Check effective_cache_size sanity
        let ecs_ratio = config.effective_cache_size_kb as f64 / system.total_ram_kb as f64;
        if ecs_ratio > 0.90 {
            issues.push(
                ValidationIssue::medium(
                    "memory",
                    "effective_cache_size too optimistic",
                    "May lead to poor query plans",
                )
                .with_recommendation("Set to 75% of RAM for servers, 25% for desktops")
                .with_values(&format!("{:.1}%", ecs_ratio * 100.0), "75%"),
            );
        }

        issues
    }
}

/// Connection validator - checks connection configuration
struct ConnectionValidator;

impl ConnectionValidator {
    fn validate_config(&self, config: &PgConfig) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        // Check superuser slots
        let su_ratio = config.superuser_reserved_connections as f64 / config.max_connections as f64;
        if su_ratio < 0.02 {
            issues.push(
                ValidationIssue::medium(
                    "connections",
                    "Few superuser connection slots reserved",
                    "May not be able to connect as superuser during issues",
                )
                .with_recommendation("Increase superuser_reserved_connections to 2-5%")
                .with_values(
                    &config.superuser_reserved_connections.to_string(),
                    &format!("{}", (config.max_connections as f64 * 0.03) as i32),
                ),
            );
        } else if su_ratio > 0.10 {
            issues.push(
                ValidationIssue::low(
                    "connections",
                    "Many superuser slots reserved",
                    "May limit regular user connections",
                )
                .with_recommendation("Reduce to 2-5% of max_connections"),
            );
        }

        issues
    }

    async fn validate_runtime(&self, pool: &PgPool) -> Result<Vec<ValidationIssue>> {
        let mut issues = Vec::new();

        // Get connection stats
        let row = sqlx::query(
            r#"
            SELECT 
                (SELECT setting::int FROM pg_settings WHERE name = 'max_connections') as max_conn,
                COUNT(*)::int as current_conn,
                COALESCE(AVG(EXTRACT(EPOCH FROM (now() - backend_start)))::int, 0) as avg_age_seconds
            FROM pg_stat_activity
            WHERE backend_type = 'client backend'
            "#
        )
        .fetch_one(pool)
        .await?;

        let max: i32 = row.try_get("max_conn")?;
        let current: i32 = row.try_get("current_conn")?;
        let avg_age: i32 = row.try_get("avg_age_seconds")?;

        let utilization = current as f64 / max as f64;

        if utilization > 0.90 {
            issues.push(
                ValidationIssue::critical(
                    "connections",
                    "Connection pool near exhaustion",
                    format!("Using {:.0}% of available connections", utilization * 100.0),
                )
                .with_recommendation("Increase max_connections or implement connection pooling")
                .with_values(
                    &format!("{}/{}", current, max),
                    &format!("{}", (max as f64 * 1.5) as i32),
                ),
            );
        } else if utilization > 0.70 {
            issues.push(
                ValidationIssue::high(
                    "connections",
                    "High connection utilization",
                    format!("Using {:.0}% of connections", utilization * 100.0),
                )
                .with_recommendation("Consider increasing max_connections"),
            );
        }

        // Check connection churn
        if avg_age < 60 {
            issues.push(
                ValidationIssue::high(
                    "connections",
                    "High connection churn detected",
                    "Average connection age < 1 minute",
                )
                .with_recommendation("Implement connection pooler (PgBouncer/Pgpool)"),
            );
        } else if avg_age < 600 {
            issues.push(
                ValidationIssue::medium(
                    "connections",
                    "Moderate connection churn",
                    "Average connection age < 10 minutes",
                )
                .with_recommendation("Consider connection pooling for better performance"),
            );
        }

        Ok(issues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> PgConfig {
        PgConfig {
            shared_buffers_kb: 16 * 1024 * 1024,       // 16GB
            effective_cache_size_kb: 48 * 1024 * 1024, // 48GB
            maintenance_work_mem_kb: 2 * 1024 * 1024,  // 2GB
            work_mem_kb: 64 * 1024,                    // 64MB
            max_connections: 200,
            superuser_reserved_connections: 3,
            fsync: true,
            full_page_writes: true,
            synchronous_commit: "on".to_string(),
            checkpoint_completion_target: 0.9,
            log_min_duration_statement: Some(1000),
            log_checkpoints: true,
        }
    }

    fn create_test_system() -> SystemInfo {
        SystemInfo {
            total_ram_kb: 64 * 1024 * 1024, // 64GB
            cpu_count: 16,
            disk_type: super::super::models::DiskType::SSD,
            os_type: super::super::models::OsType::Linux,
            pg_version: 16.0,
        }
    }

    #[test]
    fn test_safety_validator_fsync_off() {
        let validator = SafetyValidator;
        let mut config = create_test_config();
        config.fsync = false;

        let issues = validator.validate(&config);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|i| i.severity == ValidationSeverity::Critical));
        assert!(issues.iter().any(|i| i.title.contains("fsync")));
    }

    #[test]
    fn test_safety_validator_full_page_writes_off() {
        let validator = SafetyValidator;
        let mut config = create_test_config();
        config.full_page_writes = false;

        let issues = validator.validate(&config);
        assert!(issues
            .iter()
            .any(|i| i.severity == ValidationSeverity::Critical));
        assert!(issues.iter().any(|i| i.title.contains("full_page_writes")));
    }

    #[test]
    fn test_safety_validator_synchronous_commit_off() {
        let validator = SafetyValidator;
        let mut config = create_test_config();
        config.synchronous_commit = "off".to_string();

        let issues = validator.validate(&config);
        assert!(issues
            .iter()
            .any(|i| i.severity == ValidationSeverity::High));
        assert!(issues
            .iter()
            .any(|i| i.title.contains("synchronous_commit")));
    }

    #[test]
    fn test_memory_validator_shared_buffers_too_low() {
        let validator = MemoryValidator;
        let system = create_test_system();
        let mut config = create_test_config();
        config.shared_buffers_kb = 4 * 1024 * 1024; // 4GB (6.25% of 64GB)

        let issues = validator.validate(&config, &system);
        assert!(issues
            .iter()
            .any(|i| i.title.contains("shared_buffers too low")));
    }

    #[test]
    fn test_memory_validator_shared_buffers_too_high() {
        let validator = MemoryValidator;
        let system = create_test_system();
        let mut config = create_test_config();
        config.shared_buffers_kb = 32 * 1024 * 1024; // 32GB (50% of 64GB)

        let issues = validator.validate(&config, &system);
        assert!(issues
            .iter()
            .any(|i| i.title.contains("shared_buffers too high")));
    }

    #[test]
    fn test_memory_validator_oom_risk() {
        let validator = MemoryValidator;
        let system = create_test_system();
        let mut config = create_test_config();
        config.work_mem_kb = 512 * 1024; // 512MB per connection
        config.max_connections = 300;

        let issues = validator.validate(&config, &system);
        assert!(issues.iter().any(|i| i.title.contains("OOM")));
    }

    #[test]
    fn test_connection_validator_superuser_slots() {
        let validator = ConnectionValidator;
        let mut config = create_test_config();
        config.superuser_reserved_connections = 1; // < 2%
        config.max_connections = 200;

        let issues = validator.validate_config(&config);
        assert!(issues.iter().any(|i| i.title.contains("superuser")));
    }

    #[test]
    fn test_healthy_configuration() {
        let safety = SafetyValidator;
        let memory = MemoryValidator;
        let connection = ConnectionValidator;

        let config = create_test_config();
        let system = create_test_system();

        let safety_issues = safety.validate(&config);
        let memory_issues = memory.validate(&config, &system);
        let conn_issues = connection.validate_config(&config);

        // Should have minimal issues (maybe observability recommendations)
        let all_issues: Vec<_> = safety_issues
            .iter()
            .chain(memory_issues.iter())
            .chain(conn_issues.iter())
            .collect();

        let critical_or_high = all_issues
            .iter()
            .filter(|i| {
                i.severity == ValidationSeverity::Critical || i.severity == ValidationSeverity::High
            })
            .count();

        assert_eq!(
            critical_or_high, 0,
            "Healthy config should have no critical/high issues"
        );
    }
}
