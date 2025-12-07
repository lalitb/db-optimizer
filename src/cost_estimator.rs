use crate::models::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CloudProvider {
    Aws,
    Azure,
    Gcp,
    OnPremise,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    pub provider: CloudProvider,
    pub cost_per_vcpu_hour: f64,
    pub cost_per_million_iops: f64,
    pub cost_per_gb_storage_month: f64,
    pub currency: String,
}

impl Default for PricingConfig {
    fn default() -> Self {
        // Default to AWS RDS pricing (approximate)
        Self {
            provider: CloudProvider::Aws,
            cost_per_vcpu_hour: 0.0416, // db.t3.medium (2 vCPU) is ~$0.0832/hr -> $0.0416/vCPU
            cost_per_million_iops: 0.20, // Aurora Standard I/O rate
            cost_per_gb_storage_month: 0.115, // GP3 storage
            currency: "USD".to_string(),
        }
    }
}

impl PricingConfig {
    pub fn azure_default() -> Self {
        Self {
            provider: CloudProvider::Azure,
            cost_per_vcpu_hour: 0.05,    // General Purpose Gen5
            cost_per_million_iops: 0.05, // Flexible Server IOPS
            cost_per_gb_storage_month: 0.115,
            currency: "USD".to_string(),
        }
    }
}

pub struct CostEstimator {
    config: PricingConfig,
}

impl CostEstimator {
    pub fn new(config: PricingConfig) -> Self {
        Self { config }
    }

    pub fn aws() -> Self {
        Self::new(PricingConfig::default())
    }

    pub fn azure() -> Self {
        Self::new(PricingConfig::azure_default())
    }

    /// Estimate the monthly cost of a specific query
    pub fn estimate_query_cost_monthly(&self, stats: &QueryStats) -> f64 {
        // 1. CPU Cost
        // Convert ms to hours: ms / (1000 * 3600)
        // We assume the query uses 1 vCPU fully during execution (simplification)
        let cpu_hours_per_call = stats.mean_exec_time_ms / (1000.0 * 3600.0);
        let total_cpu_hours_monthly = cpu_hours_per_call * stats.calls as f64 * 30.0; // Extrapolate daily calls to monthly
        let cpu_cost = total_cpu_hours_monthly * self.config.cost_per_vcpu_hour;

        // 2. I/O Cost
        // shared_blks_read is the number of 8KB blocks read from disk
        // We treat each block read as 1 I/O operation
        let iops_per_call = stats.shared_blks_read as f64 / stats.calls.max(1) as f64;
        let total_iops_monthly = iops_per_call * stats.calls as f64 * 30.0;
        let io_cost = (total_iops_monthly / 1_000_000.0) * self.config.cost_per_million_iops;

        cpu_cost + io_cost
    }

    /// Estimate monthly cost savings for a recommendation
    pub fn estimate_savings(&self, rec: &mut Recommendation) {
        let monthly_savings = match rec.issue_type {
            IssueType::MissingIndex => {
                // For missing index, we estimate savings based on reduced I/O and CPU
                // Assumption: Index scan is 90% cheaper on I/O and 50% cheaper on CPU than Seq Scan
                if let Some(time_saved_ms) = rec.estimated_time_savings_ms {
                    if let Some(calls) = rec.affected_queries_count {
                        // CPU Savings
                        let cpu_hours_saved =
                            (time_saved_ms * calls as f64 * 30.0) / (1000.0 * 3600.0);
                        let cpu_savings = cpu_hours_saved * self.config.cost_per_vcpu_hour;

                        // I/O Savings (Heuristic: assume 90% reduction in I/O if we have time savings)
                        // We don't have the raw I/O stats in the Recommendation struct, so we estimate
                        // based on the fact that I/O is usually the bottleneck for slow queries.
                        // Let's assume 1ms of saved time roughly correlates to 0.1 I/O ops saved (very rough heuristic)
                        // Better approach: Pass QueryStats to this method if possible.
                        // For now, we'll stick to CPU savings + a multiplier for I/O heavy workloads

                        cpu_savings * 1.5 // 1.5x multiplier to account for I/O savings
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            }
            IssueType::UnusedIndex => {
                // Storage savings
                rec.estimated_cost_savings_usd_monthly.unwrap_or(0.0)
            }
            IssueType::SlowQuery => {
                // Similar to missing index, but maybe less dramatic
                if let Some(time_saved_ms) = rec.estimated_time_savings_ms {
                    if let Some(calls) = rec.affected_queries_count {
                        let cpu_hours_saved =
                            (time_saved_ms * calls as f64 * 30.0) / (1000.0 * 3600.0);
                        cpu_hours_saved * self.config.cost_per_vcpu_hour
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            }
            IssueType::HighVolumeQuery => {
                if let Some(time_saved_ms) = rec.estimated_time_savings_ms {
                    if let Some(calls) = rec.affected_queries_count {
                        let cpu_hours_saved =
                            (time_saved_ms * calls as f64 * 30.0) / (1000.0 * 3600.0);
                        cpu_hours_saved * self.config.cost_per_vcpu_hour
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            }
            _ => rec.estimated_cost_savings_usd_monthly.unwrap_or(0.0),
        };

        rec.estimated_cost_savings_usd_monthly = Some(monthly_savings.max(0.0));
    }

    /// Calculate storage cost for a given size in MB
    pub fn calculate_storage_cost(&self, size_mb: f64) -> f64 {
        let size_gb = size_mb / 1024.0;
        size_gb * self.config.cost_per_gb_storage_month
    }
}

impl Default for CostEstimator {
    fn default() -> Self {
        Self::aws()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_cost_estimation() {
        let estimator = CostEstimator::aws();

        // Simulate a heavy query: 1000ms execution, 1000 blocks read, called 1000 times/day
        let stats = QueryStats {
            queryid: Some(1),
            query: "SELECT * FROM heavy".to_string(),
            calls: 1000, // Daily calls
            total_exec_time_ms: 1_000_000.0,
            mean_exec_time_ms: 1000.0,
            rows: 1000,
            shared_blks_read: 1_000_000, // 1000 blocks per call * 1000 calls
            shared_blks_hit: 0,
        };

        let monthly_cost = estimator.estimate_query_cost_monthly(&stats);

        // CPU: 1000ms * 1000 calls * 30 days = 30,000,000 ms = 8.33 hours
        // CPU Cost: 8.33 * $0.0416 = ~$0.34

        // I/O: 1000 blocks * 1000 calls * 30 days = 30,000,000 IOPS
        // I/O Cost: 30 * $0.20 = $6.00

        // Total should be around $6.34
        assert!(monthly_cost > 6.0 && monthly_cost < 7.0);
    }

    #[test]
    fn test_storage_savings() {
        let estimator = CostEstimator::aws();
        // 10GB unused index
        let cost = estimator.calculate_storage_cost(10240.0);
        // 10GB * $0.115 = $1.15
        assert!((cost - 1.15).abs() < 0.01);
    }
}
