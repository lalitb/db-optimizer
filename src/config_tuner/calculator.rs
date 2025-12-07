// Configuration calculator - implements PGTune formulas

use super::models::{DiskType, HugePagesMode, OptimalConfig, OsType, SystemInfo, WorkloadType};

/// Configuration calculator using PGTune algorithms
pub struct ConfigCalculator;

impl ConfigCalculator {
    /// Create a new configuration calculator
    pub fn new() -> Self {
        Self
    }

    /// Calculate optimal configuration for given system and workload
    pub fn calculate(&self, system: &SystemInfo, workload: WorkloadType) -> OptimalConfig {
        // Calculate all settings
        let shared_buffers_kb = self.calc_shared_buffers(system, workload);
        let effective_cache_size_kb = self.calc_effective_cache_size(system, workload);
        let maintenance_work_mem_kb = self.calc_maintenance_work_mem(system, workload);
        let max_connections = self.calc_max_connections(workload);
        let work_mem_kb = self.calc_work_mem(system, shared_buffers_kb, max_connections);
        let (min_wal_size_mb, max_wal_size_mb) = self.calc_wal_sizes(workload);
        let random_page_cost = self.calc_random_page_cost(system.disk_type);
        let effective_io_concurrency = self.calc_effective_io_concurrency(system);
        let huge_pages = self.recommend_huge_pages(system.total_ram_kb);

        OptimalConfig {
            shared_buffers_kb,
            effective_cache_size_kb,
            maintenance_work_mem_kb,
            work_mem_kb,
            max_connections,
            min_wal_size_mb,
            max_wal_size_mb,
            checkpoint_completion_target: 0.9, // Universal best practice
            random_page_cost,
            effective_io_concurrency,
            huge_pages,
            workload_type: workload,
            system_ram_gb: system.ram_gb(),
        }
    }

    /// Calculate shared_buffers (25% of RAM for most workloads)
    fn calc_shared_buffers(&self, system: &SystemInfo, workload: WorkloadType) -> u64 {
        let base_value = match workload {
            WorkloadType::Web => system.total_ram_kb / 4,  // 25%
            WorkloadType::OLTP => system.total_ram_kb / 4, // 25%
            WorkloadType::DataWarehouse => system.total_ram_kb / 4, // 25%
            WorkloadType::Desktop => system.total_ram_kb / 16, // 6.25%
            WorkloadType::Mixed => system.total_ram_kb / 4, // 25%
        };

        // Windows PG<10 limitation: max 512MB
        if system.pg_version < 10.0 && system.os_type == OsType::Windows {
            let win_limit = (512 * 1024 * 1024) / 1024; // 512MB in KB
            return base_value.min(win_limit);
        }

        base_value
    }

    /// Calculate effective_cache_size (75% of RAM for servers)
    fn calc_effective_cache_size(&self, system: &SystemInfo, workload: WorkloadType) -> u64 {
        match workload {
            WorkloadType::Web => (system.total_ram_kb * 3) / 4, // 75%
            WorkloadType::OLTP => (system.total_ram_kb * 3) / 4, // 75%
            WorkloadType::DataWarehouse => (system.total_ram_kb * 3) / 4, // 75%
            WorkloadType::Desktop => system.total_ram_kb / 4,   // 25%
            WorkloadType::Mixed => (system.total_ram_kb * 3) / 4, // 75%
        }
    }

    /// Calculate maintenance_work_mem (capped at 2GB)
    fn calc_maintenance_work_mem(&self, system: &SystemInfo, workload: WorkloadType) -> u64 {
        let base_value = match workload {
            WorkloadType::Web => system.total_ram_kb / 16, // 6.25%
            WorkloadType::OLTP => system.total_ram_kb / 16, // 6.25%
            WorkloadType::DataWarehouse => system.total_ram_kb / 8, // 12.5%
            WorkloadType::Desktop => system.total_ram_kb / 16, // 6.25%
            WorkloadType::Mixed => system.total_ram_kb / 16, // 6.25%
        };

        // Cap at 2GB
        let limit_kb = (2 * 1024 * 1024 * 1024) / 1024; // 2GB in KB
        let final_value = base_value.min(limit_kb);

        // Windows: reduce by 1MB to avoid errors
        if system.os_type == OsType::Windows && final_value >= limit_kb {
            return final_value - (1024 * 1024); // Subtract 1MB
        }

        final_value
    }

    /// Calculate work_mem per operation
    fn calc_work_mem(
        &self,
        system: &SystemInfo,
        shared_buffers_kb: u64,
        max_connections: i32,
    ) -> u64 {
        // Available memory for work_mem
        let work_mem_total = system.total_ram_kb.saturating_sub(shared_buffers_kb);

        // Assume 2 parallel workers per query (conservative)
        let parallel_multiplier = 2;

        // Calculate per-connection work_mem
        let work_mem = work_mem_total / (max_connections as u64 * parallel_multiplier);

        // Minimum 64KB, maximum 512MB per operation
        let min_kb = 64;
        let max_kb = 512 * 1024; // 512MB

        work_mem.max(min_kb).min(max_kb)
    }

    /// Calculate max_connections based on workload
    fn calc_max_connections(&self, workload: WorkloadType) -> i32 {
        match workload {
            WorkloadType::Web => 200,
            WorkloadType::OLTP => 300,
            WorkloadType::DataWarehouse => 40,
            WorkloadType::Desktop => 20,
            WorkloadType::Mixed => 100,
        }
    }

    /// Calculate WAL sizes (in MB)
    fn calc_wal_sizes(&self, workload: WorkloadType) -> (u32, u32) {
        match workload {
            WorkloadType::Web => (1024, 4096),            // 1GB min, 4GB max
            WorkloadType::OLTP => (2048, 8192),           // 2GB min, 8GB max
            WorkloadType::DataWarehouse => (4096, 16384), // 4GB min, 16GB max
            WorkloadType::Desktop => (100, 2048),         // 100MB min, 2GB max
            WorkloadType::Mixed => (1024, 4096),          // 1GB min, 4GB max
        }
    }

    /// Calculate random_page_cost based on disk type
    fn calc_random_page_cost(&self, disk_type: DiskType) -> f32 {
        match disk_type {
            DiskType::HDD => 4.0,
            DiskType::SSD => 1.1,
            DiskType::SAN => 1.1,
        }
    }

    /// Calculate effective_io_concurrency (Linux only)
    fn calc_effective_io_concurrency(&self, system: &SystemInfo) -> Option<u32> {
        if system.os_type != OsType::Linux {
            return None; // Not supported on non-Linux
        }

        Some(match system.disk_type {
            DiskType::HDD => 2,
            DiskType::SSD => 200,
            DiskType::SAN => 300,
        })
    }

    /// Recommend huge pages (for systems with >=32GB RAM)
    fn recommend_huge_pages(&self, total_ram_kb: u64) -> HugePagesMode {
        // Enable for systems with >32GB RAM
        const THRESHOLD_KB: u64 = 32 * 1024 * 1024; // 32GB
        if total_ram_kb >= THRESHOLD_KB {
            HugePagesMode::Try
        } else {
            HugePagesMode::Off
        }
    }
}

impl Default for ConfigCalculator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_system(ram_gb: u64, disk: DiskType) -> SystemInfo {
        SystemInfo {
            total_ram_kb: ram_gb * 1024 * 1024,
            cpu_count: 16,
            disk_type: disk,
            os_type: OsType::Linux,
            pg_version: 16.0,
        }
    }

    #[test]
    fn test_shared_buffers_calculation() {
        let calc = ConfigCalculator::new();
        let system = create_test_system(64, DiskType::SSD);

        // Web workload: 25% of RAM = 16GB
        let sb = calc.calc_shared_buffers(&system, WorkloadType::Web);
        assert_eq!(sb, 16 * 1024 * 1024); // 16GB in KB

        // Desktop: 6.25% of RAM = 4GB
        let sb = calc.calc_shared_buffers(&system, WorkloadType::Desktop);
        assert_eq!(sb, 4 * 1024 * 1024); // 4GB in KB
    }

    #[test]
    fn test_effective_cache_size_calculation() {
        let calc = ConfigCalculator::new();
        let system = create_test_system(64, DiskType::SSD);

        // Server workload: 75% of RAM = 48GB
        let ecs = calc.calc_effective_cache_size(&system, WorkloadType::OLTP);
        assert_eq!(ecs, 48 * 1024 * 1024); // 48GB in KB

        // Desktop: 25% of RAM = 16GB
        let ecs = calc.calc_effective_cache_size(&system, WorkloadType::Desktop);
        assert_eq!(ecs, 16 * 1024 * 1024); // 16GB in KB
    }

    #[test]
    fn test_maintenance_work_mem_capped_at_2gb() {
        let calc = ConfigCalculator::new();
        let system = create_test_system(128, DiskType::SSD); // 128GB RAM

        // DW: 12.5% of 128GB = 16GB, but should be capped at 2GB
        let mwm = calc.calc_maintenance_work_mem(&system, WorkloadType::DataWarehouse);
        assert_eq!(mwm, 2 * 1024 * 1024); // 2GB in KB
    }

    #[test]
    fn test_work_mem_bounds() {
        let calc = ConfigCalculator::new();

        // Small system: 4GB RAM
        let system = create_test_system(4, DiskType::SSD);
        let sb = calc.calc_shared_buffers(&system, WorkloadType::Web);
        let wm = calc.calc_work_mem(&system, sb, 200);

        assert!(wm >= 64); // Minimum 64KB
        assert!(wm <= 512 * 1024); // Maximum 512MB
    }

    #[test]
    fn test_max_connections_by_workload() {
        let calc = ConfigCalculator::new();

        assert_eq!(calc.calc_max_connections(WorkloadType::OLTP), 300);
        assert_eq!(calc.calc_max_connections(WorkloadType::Web), 200);
        assert_eq!(calc.calc_max_connections(WorkloadType::Mixed), 100);
        assert_eq!(calc.calc_max_connections(WorkloadType::DataWarehouse), 40);
        assert_eq!(calc.calc_max_connections(WorkloadType::Desktop), 20);
    }

    #[test]
    fn test_random_page_cost_by_disk() {
        let calc = ConfigCalculator::new();

        assert_eq!(calc.calc_random_page_cost(DiskType::HDD), 4.0);
        assert_eq!(calc.calc_random_page_cost(DiskType::SSD), 1.1);
        assert_eq!(calc.calc_random_page_cost(DiskType::SAN), 1.1);
    }

    #[test]
    fn test_effective_io_concurrency() {
        let calc = ConfigCalculator::new();

        // Linux with SSD
        let system = SystemInfo {
            total_ram_kb: 64 * 1024 * 1024,
            cpu_count: 16,
            disk_type: DiskType::SSD,
            os_type: OsType::Linux,
            pg_version: 16.0,
        };
        assert_eq!(calc.calc_effective_io_concurrency(&system), Some(200));

        // macOS - not supported
        let system = SystemInfo {
            total_ram_kb: 64 * 1024 * 1024,
            cpu_count: 16,
            disk_type: DiskType::SSD,
            os_type: OsType::MacOS,
            pg_version: 16.0,
        };
        assert_eq!(calc.calc_effective_io_concurrency(&system), None);
    }

    #[test]
    fn test_huge_pages_threshold() {
        let calc = ConfigCalculator::new();

        // <32GB: off
        assert_eq!(
            calc.recommend_huge_pages(16 * 1024 * 1024),
            HugePagesMode::Off
        );

        // >=32GB: try
        assert_eq!(
            calc.recommend_huge_pages(32 * 1024 * 1024),
            HugePagesMode::Try
        );
        assert_eq!(
            calc.recommend_huge_pages(64 * 1024 * 1024),
            HugePagesMode::Try
        );
    }

    #[test]
    fn test_complete_config_generation() {
        let calc = ConfigCalculator::new();
        let system = create_test_system(64, DiskType::SSD);
        let config = calc.calculate(&system, WorkloadType::OLTP);

        // Verify all fields are set
        assert!(config.shared_buffers_kb > 0);
        assert!(config.effective_cache_size_kb > 0);
        assert!(config.maintenance_work_mem_kb > 0);
        assert!(config.work_mem_kb > 0);
        assert!(config.max_connections > 0);
        assert!(config.min_wal_size_mb > 0);
        assert!(config.max_wal_size_mb > config.min_wal_size_mb);
        assert_eq!(config.checkpoint_completion_target, 0.9);
        assert!(config.random_page_cost > 0.0);
        assert!(config.effective_io_concurrency.is_some());
        assert_eq!(config.workload_type, WorkloadType::OLTP);
        assert_eq!(config.system_ram_gb, 64.0);

        // Test PostgreSQL conf generation
        let pg_conf = config.to_postgresql_conf();
        assert!(pg_conf.contains("shared_buffers"));
        assert!(pg_conf.contains("max_connections = 300"));
        assert!(pg_conf.contains("OLTP"));
    }

    #[test]
    fn test_windows_pg9_shared_buffers_limit() {
        let calc = ConfigCalculator::new();
        let system = SystemInfo {
            total_ram_kb: 64 * 1024 * 1024, // 64GB
            cpu_count: 16,
            disk_type: DiskType::SSD,
            os_type: OsType::Windows,
            pg_version: 9.6,
        };

        let sb = calc.calc_shared_buffers(&system, WorkloadType::Web);
        let max_kb = (512 * 1024 * 1024) / 1024; // 512MB
        assert!(sb <= max_kb);
    }
}
