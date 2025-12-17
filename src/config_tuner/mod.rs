// Configuration tuning module - hardware-aware PostgreSQL optimization
// Based on PGTune formulas and PostgreSQLTuner validation rules

pub mod calculator;
pub mod models;
pub mod validator;
pub mod workload_detector;

pub use calculator::ConfigCalculator;
pub use models::{
    DiskType, OptimalConfig, OsType, SystemInfo, ValidationIssue, ValidationReport,
    ValidationSeverity, WorkloadType,
};
pub use validator::ConfigValidator;
pub use workload_detector::WorkloadDetector;

use anyhow::Result;
use sqlx::PgPool;

/// Main configuration tuning interface
#[derive(Clone)]
pub struct ConfigTuner {
    calculator: ConfigCalculator,
    validator: ConfigValidator,
    workload_detector: WorkloadDetector,
}

impl ConfigTuner {
    /// Create a new configuration tuner
    pub fn new() -> Self {
        Self {
            calculator: ConfigCalculator::new(),
            validator: ConfigValidator::new(),
            workload_detector: WorkloadDetector::new(),
        }
    }

    /// Generate optimal configuration for given system and workload
    pub fn calculate_optimal_config(
        &self,
        system_info: &SystemInfo,
        workload: WorkloadType,
    ) -> OptimalConfig {
        self.calculator.calculate(system_info, workload)
    }

    /// Detect workload type from database statistics
    pub async fn detect_workload(&self, pool: &PgPool) -> Result<WorkloadType> {
        self.workload_detector.detect(pool).await
    }

    /// Validate current configuration against best practices
    pub async fn validate_config(&self, pool: &PgPool) -> Result<ValidationReport> {
        self.validator.validate(pool).await
    }

    /// Get system information from the host
    pub fn detect_system_info(&self) -> Result<SystemInfo> {
        SystemInfo::detect()
    }
}

impl Default for ConfigTuner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_tuner_creation() {
        let tuner = ConfigTuner::new();
        assert!(true, "ConfigTuner created successfully");
    }

    #[test]
    fn test_calculate_optimal_config() {
        let tuner = ConfigTuner::new();
        let system_info = SystemInfo {
            total_ram_kb: 64 * 1024 * 1024, // 64GB
            cpu_count: 16,
            disk_type: DiskType::SSD,
            os_type: OsType::Linux,
            pg_version: 16.0,
        };

        let config = tuner.calculate_optimal_config(&system_info, WorkloadType::OLTP);

        // Validate basic constraints
        assert!(config.shared_buffers_kb > 0);
        assert!(config.work_mem_kb > 0);
        assert!(config.max_connections > 0);
    }
}
