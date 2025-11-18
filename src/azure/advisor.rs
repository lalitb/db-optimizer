// src/azure/advisor.rs - Azure cost optimization advisor

use super::pricing::{AzureSku, PricingCalculator};
use serde::{Deserialize, Serialize};

/// Cost justification based on eBPF metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostJustification {
    pub cpu_utilization: f64,
    pub memory_utilization: f64,
    pub io_utilization: f64,
    
    // eBPF-verified workload characteristics
    pub peak_cpu_during_business_hours: f64,
    pub queries_per_second_p95: f64,
    pub avg_query_latency_ms: f64,
    
    // I/O breakdown from eBPF
    pub io_wait_percentage: f64,
    pub disk_seeks_per_query: f64,
    pub page_cache_hit_rate: f64,
}

/// SKU recommendation with justification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkuRecommendation {
    pub current_sku: AzureSku,
    pub recommended_sku: AzureSku,
    pub monthly_savings: f64,
    pub performance_impact: String,
    pub confidence: f64,
    pub justification: CostJustification,
}

/// Cost optimization recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostOptimization {
    pub sku_recommendation: Option<SkuRecommendation>,
    pub storage_recommendations: Vec<String>,
    pub configuration_recommendations: Vec<String>,
}

/// Azure cost optimization advisor
pub struct AzureAdvisor {
    calculator: PricingCalculator,
}

impl AzureAdvisor {
    pub fn new(calculator: PricingCalculator) -> Self {
        Self { calculator }
    }
    
    /// Generate cost optimization recommendations
    pub fn generate_recommendations(
        &self,
        current_sku: &AzureSku,
        justification: CostJustification,
    ) -> CostOptimization {
        let sku_recommendation = self.generate_sku_recommendation(
            current_sku,
            &justification,
        );
        
        let storage_recommendations = self.generate_storage_recommendations(&justification);
        let configuration_recommendations = self.generate_config_recommendations(&justification);
        
        CostOptimization {
            sku_recommendation,
            storage_recommendations,
            configuration_recommendations,
        }
    }
    
    fn generate_sku_recommendation(
        &self,
        current_sku: &AzureSku,
        justification: &CostJustification,
    ) -> Option<SkuRecommendation> {
        // Use pricing calculator to find optimal SKU
        let recommended_sku = self.calculator.recommend_sku(
            current_sku,
            justification.cpu_utilization,
            justification.memory_utilization,
            justification.io_utilization,
        )?;
        
        // Calculate savings
        let monthly_savings = self.calculator.calculate_savings(
            current_sku,
            &recommended_sku,
        )?;
        
        // Only recommend if savings are significant
        if monthly_savings < 10.0 {
            return None;
        }
        
        // Determine performance impact
        let performance_impact = self.assess_performance_impact(
            current_sku,
            &recommended_sku,
            justification,
        );
        
        // Calculate confidence based on data quality and utilization patterns
        let confidence = self.calculate_confidence(justification);
        
        Some(SkuRecommendation {
            current_sku: current_sku.clone(),
            recommended_sku,
            monthly_savings,
            performance_impact,
            confidence,
            justification: justification.clone(),
        })
    }
    
    fn assess_performance_impact(
        &self,
        current: &AzureSku,
        recommended: &AzureSku,
        justification: &CostJustification,
    ) -> String {
        if recommended.cores < current.cores {
            // Downgrade
            if justification.peak_cpu_during_business_hours < 50.0 {
                "No degradation expected - current resources are underutilized".to_string()
            } else if justification.peak_cpu_during_business_hours < 70.0 {
                "Minimal impact expected - peak usage will increase but remain acceptable".to_string()
            } else {
                "Monitor closely - peak CPU will be near capacity".to_string()
            }
        } else if recommended.cores > current.cores {
            // Upgrade
            format!(
                "Performance improvement expected - will reduce CPU bottlenecks (current peak: {:.1}%)",
                justification.peak_cpu_during_business_hours
            )
        } else if recommended.tier != current.tier {
            // Tier change
            if recommended.tier == "MemoryOptimized" {
                "Performance improvement for memory-intensive workloads".to_string()
            } else {
                "Cost optimization with similar performance characteristics".to_string()
            }
        } else {
            "No significant impact expected".to_string()
        }
    }
    
    fn calculate_confidence(&self, justification: &CostJustification) -> f64 {
        let mut confidence = 0.95_f64;
        
        // Reduce confidence if metrics show high variability
        if justification.peak_cpu_during_business_hours > justification.cpu_utilization * 2.0 {
            confidence -= 0.15;  // High variability
        }
        
        // Increase confidence if I/O patterns are stable
        if justification.page_cache_hit_rate > 0.90 {
            confidence = (confidence + 0.05).min(0.99_f64);
        }

        // Reduce confidence if query latency is highly variable
        if justification.avg_query_latency_ms > 100.0 {
            confidence -= 0.10;
        }

        confidence.max(0.40_f64)
    }
    
    fn generate_storage_recommendations(&self, justification: &CostJustification) -> Vec<String> {
        let mut recommendations = Vec::new();
        
        // High page cache hit rate suggests over-provisioned storage IOPS
        if justification.page_cache_hit_rate > 0.95 {
            recommendations.push(
                "Consider reducing provisioned IOPS - 95%+ page cache hit rate indicates \
                 most data is served from memory".to_string()
            );
        }
        
        // Low cache hit rate with high I/O suggests need for more memory or faster storage
        if justification.page_cache_hit_rate < 0.70 && justification.io_wait_percentage > 30.0 {
            recommendations.push(
                "Consider Memory Optimized SKU or Premium SSD storage - low cache hit rate \
                 combined with high I/O wait time".to_string()
            );
        }
        
        // High disk seeks suggest index optimization opportunities
        if justification.disk_seeks_per_query > 100.0 {
            recommendations.push(format!(
                "High disk seeks per query ({:.0}) suggests missing indexes or inefficient \
                 query patterns - focus on query optimization first",
                justification.disk_seeks_per_query
            ));
        }
        
        recommendations
    }
    
    fn generate_config_recommendations(&self, justification: &CostJustification) -> Vec<String> {
        let mut recommendations = Vec::new();
        
        // High I/O wait suggests configuration tuning
        if justification.io_wait_percentage > 40.0 {
            recommendations.push(
                "I/O wait time exceeds 40% - review shared_buffers and effective_cache_size \
                 configuration".to_string()
            );
        }
        
        // High CPU with low I/O suggests CPU-bound queries
        if justification.cpu_utilization > 70.0 && justification.io_wait_percentage < 20.0 {
            recommendations.push(
                "Workload is CPU-bound - focus on query optimization, consider parallel query \
                 settings".to_string()
            );
        }
        
        // High query rate with high latency
        if justification.queries_per_second_p95 > 1000.0 
            && justification.avg_query_latency_ms > 50.0 {
            recommendations.push(
                "High query rate with elevated latency - review connection pooling \
                 and max_connections settings".to_string()
            );
        }
        
        recommendations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::pricing::AzurePricing;
    
    #[test]
    fn test_downsize_recommendation() {
        let pricing = AzurePricing::new_with_defaults();
        let calculator = PricingCalculator::new(pricing);
        let advisor = AzureAdvisor::new(calculator);
        
        let current_sku = AzureSku::new("GeneralPurpose", "Gen5", 8);
        let justification = CostJustification {
            cpu_utilization: 25.0,
            memory_utilization: 35.0,
            io_utilization: 15.0,
            peak_cpu_during_business_hours: 45.0,
            queries_per_second_p95: 100.0,
            avg_query_latency_ms: 5.0,
            io_wait_percentage: 10.0,
            disk_seeks_per_query: 5.0,
            page_cache_hit_rate: 0.95,
        };
        
        let optimization = advisor.generate_recommendations(&current_sku, justification);
        
        assert!(optimization.sku_recommendation.is_some());
        let rec = optimization.sku_recommendation.unwrap();
        assert!(rec.recommended_sku.cores < current_sku.cores);
        assert!(rec.monthly_savings > 0.0);
        assert!(rec.confidence > 0.8);
    }
    
    #[test]
    fn test_no_recommendation_when_optimal() {
        let pricing = AzurePricing::new_with_defaults();
        let calculator = PricingCalculator::new(pricing);
        let advisor = AzureAdvisor::new(calculator);
        
        let current_sku = AzureSku::new("GeneralPurpose", "Gen5", 4);
        let justification = CostJustification {
            cpu_utilization: 65.0,
            memory_utilization: 60.0,
            io_utilization: 45.0,
            peak_cpu_during_business_hours: 75.0,
            queries_per_second_p95: 500.0,
            avg_query_latency_ms: 10.0,
            io_wait_percentage: 25.0,
            disk_seeks_per_query: 20.0,
            page_cache_hit_rate: 0.85,
        };
        
        let optimization = advisor.generate_recommendations(&current_sku, justification);
        
        // Should not recommend changes when system is well-utilized
        assert!(optimization.sku_recommendation.is_none());
    }
}
