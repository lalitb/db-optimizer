// src/azure/pricing.rs - Azure pricing integration for cost optimization

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Azure Postgres SKU identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AzureSku {
    pub tier: String,    // GeneralPurpose, MemoryOptimized, Burstable
    pub compute: String, // Gen5, Ddsv4, Edsv4, etc.
    pub cores: u32,      // Number of vCores
}

impl AzureSku {
    pub fn new(tier: &str, compute: &str, cores: u32) -> Self {
        Self {
            tier: tier.to_string(),
            compute: compute.to_string(),
            cores,
        }
    }

    /// Parse from Azure SKU name (e.g., "GP_Gen5_8")
    pub fn from_name(name: &str) -> Option<Self> {
        let parts: Vec<&str> = name.split('_').collect();
        if parts.len() != 3 {
            return None;
        }

        let tier = match parts[0] {
            "GP" => "GeneralPurpose",
            "MO" => "MemoryOptimized",
            "B" => "Burstable",
            _ => return None,
        };

        let cores = parts[2].parse::<u32>().ok()?;

        Some(Self::new(tier, parts[1], cores))
    }

    /// Format as Azure SKU name
    pub fn to_name(&self) -> String {
        let tier_prefix = match self.tier.as_str() {
            "GeneralPurpose" => "GP",
            "MemoryOptimized" => "MO",
            "Burstable" => "B",
            _ => "GP",
        };

        format!("{}_{}_{}", tier_prefix, self.compute, self.cores)
    }
}

/// Azure pricing information
#[derive(Debug, Clone)]
pub struct AzurePricing {
    /// Price per hour for each SKU
    pub compute_pricing: HashMap<AzureSku, f64>,

    /// Storage pricing per GB/month
    pub storage_price_per_gb: f64,

    /// IOPS pricing (if applicable)
    pub iops_price_per_1000: f64,
}

impl AzurePricing {
    /// Create with default pricing (approximate USD prices)
    pub fn new_with_defaults() -> Self {
        let mut pricing = HashMap::new();

        // General Purpose pricing (approximate)
        pricing.insert(AzureSku::new("GeneralPurpose", "Gen5", 2), 0.19);
        pricing.insert(AzureSku::new("GeneralPurpose", "Gen5", 4), 0.38);
        pricing.insert(AzureSku::new("GeneralPurpose", "Gen5", 8), 0.76);
        pricing.insert(AzureSku::new("GeneralPurpose", "Gen5", 16), 1.52);
        pricing.insert(AzureSku::new("GeneralPurpose", "Gen5", 32), 3.04);
        pricing.insert(AzureSku::new("GeneralPurpose", "Gen5", 64), 6.08);

        // Memory Optimized pricing (approximate)
        pricing.insert(AzureSku::new("MemoryOptimized", "Gen5", 2), 0.24);
        pricing.insert(AzureSku::new("MemoryOptimized", "Gen5", 4), 0.48);
        pricing.insert(AzureSku::new("MemoryOptimized", "Gen5", 8), 0.96);
        pricing.insert(AzureSku::new("MemoryOptimized", "Gen5", 16), 1.92);
        pricing.insert(AzureSku::new("MemoryOptimized", "Gen5", 32), 3.84);

        // Burstable pricing (approximate)
        pricing.insert(AzureSku::new("Burstable", "B1ms", 1), 0.013);
        pricing.insert(AzureSku::new("Burstable", "B2s", 2), 0.051);

        Self {
            compute_pricing: pricing,
            storage_price_per_gb: 0.115, // Per GB/month
            iops_price_per_1000: 0.05,   // Per 1000 IOPS/month
        }
    }

    /// Get price per hour for a SKU
    pub fn get_price_per_hour(&self, sku: &AzureSku) -> Option<f64> {
        self.compute_pricing.get(sku).copied()
    }

    /// Calculate monthly cost
    pub fn monthly_cost(&self, sku: &AzureSku) -> Option<f64> {
        self.get_price_per_hour(sku).map(|hourly| hourly * 730.0)
    }
}

/// Calculator for cost optimization
pub struct PricingCalculator {
    pricing: AzurePricing,
}

impl PricingCalculator {
    pub fn new(pricing: AzurePricing) -> Self {
        Self { pricing }
    }

    /// Calculate savings from SKU change
    pub fn calculate_savings(
        &self,
        current_sku: &AzureSku,
        recommended_sku: &AzureSku,
    ) -> Option<f64> {
        let current_monthly = self.pricing.monthly_cost(current_sku)?;
        let recommended_monthly = self.pricing.monthly_cost(recommended_sku)?;

        Some(current_monthly - recommended_monthly)
    }

    /// Find optimal SKU based on resource utilization
    pub fn recommend_sku(
        &self,
        current_sku: &AzureSku,
        cpu_utilization: f64,
        memory_utilization: f64,
        io_utilization: f64,
    ) -> Option<AzureSku> {
        // If all metrics are low, recommend downsizing
        if cpu_utilization < 30.0 && memory_utilization < 40.0 && io_utilization < 20.0 {
            // Try half the cores
            if current_sku.cores > 2 {
                let new_cores = (current_sku.cores / 2).max(2);
                return Some(AzureSku::new(
                    &current_sku.tier,
                    &current_sku.compute,
                    new_cores,
                ));
            }
        }

        // If CPU is high but memory is fine, check if General Purpose is sufficient
        if cpu_utilization > 80.0
            && memory_utilization < 50.0
            && current_sku.tier == "MemoryOptimized"
        {
            return Some(AzureSku::new("GeneralPurpose", "Gen5", current_sku.cores));
        }

        // If memory is high, recommend Memory Optimized
        if memory_utilization > 80.0 && current_sku.tier != "MemoryOptimized" {
            return Some(AzureSku::new("MemoryOptimized", "Gen5", current_sku.cores));
        }

        // If all metrics are high, recommend upsizing
        if cpu_utilization > 85.0 || memory_utilization > 85.0 {
            let new_cores = (current_sku.cores * 2).min(64);
            return Some(AzureSku::new(
                &current_sku.tier,
                &current_sku.compute,
                new_cores,
            ));
        }

        None
    }

    /// Get available SKUs in the same tier
    pub fn get_available_skus(&self, tier: &str) -> Vec<AzureSku> {
        self.pricing
            .compute_pricing
            .keys()
            .filter(|sku| sku.tier == tier)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sku_parsing() {
        let sku = AzureSku::from_name("GP_Gen5_8").unwrap();
        assert_eq!(sku.tier, "GeneralPurpose");
        assert_eq!(sku.compute, "Gen5");
        assert_eq!(sku.cores, 8);
        assert_eq!(sku.to_name(), "GP_Gen5_8");
    }

    #[test]
    fn test_pricing_calculation() {
        let pricing = AzurePricing::new_with_defaults();
        let sku = AzureSku::new("GeneralPurpose", "Gen5", 8);

        let hourly = pricing.get_price_per_hour(&sku).unwrap();
        assert!((hourly - 0.76).abs() < 0.01);

        let monthly = pricing.monthly_cost(&sku).unwrap();
        assert!((monthly - 554.8).abs() < 1.0);
    }

    #[test]
    fn test_savings_calculation() {
        let pricing = AzurePricing::new_with_defaults();
        let calculator = PricingCalculator::new(pricing);

        let current = AzureSku::new("GeneralPurpose", "Gen5", 8);
        let recommended = AzureSku::new("GeneralPurpose", "Gen5", 4);

        let savings = calculator
            .calculate_savings(&current, &recommended)
            .unwrap();
        assert!(savings > 0.0);
        assert!((savings - 277.4).abs() < 1.0); // Approximately half the cost
    }

    #[test]
    fn test_sku_recommendation_downsize() {
        let pricing = AzurePricing::new_with_defaults();
        let calculator = PricingCalculator::new(pricing);

        let current = AzureSku::new("GeneralPurpose", "Gen5", 8);
        let recommended = calculator.recommend_sku(
            &current, 25.0, // Low CPU
            35.0, // Low memory
            15.0, // Low I/O
        );

        assert!(recommended.is_some());
        let rec = recommended.unwrap();
        assert_eq!(rec.cores, 4);
    }

    #[test]
    fn test_sku_recommendation_upsize() {
        let pricing = AzurePricing::new_with_defaults();
        let calculator = PricingCalculator::new(pricing);

        // Start with MemoryOptimized so we don't trigger the "switch to MemoryOptimized" rule
        let current = AzureSku::new("MemoryOptimized", "Gen5", 8);
        let recommended = calculator.recommend_sku(
            &current, 90.0, // High CPU
            88.0, // High memory
            70.0, // High I/O
        );

        assert!(recommended.is_some());
        let rec = recommended.unwrap();
        assert_eq!(rec.cores, 16);
    }
}
