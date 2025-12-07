// src/adapters/recommendation_engine.rs - RecommendationEngine trait implementation

use anyhow::Result;
use async_trait::async_trait;

use crate::models::{IndexInfo, QueryStats, Recommendation, TableInfo};
use crate::recommendations::RecommendationEngine as LegacyEngine;
use crate::traits::RecommendationGenerator;

/// Adapter for the existing RecommendationEngine to implement the RecommendationEngine trait
pub struct RecommendationEngineAdapter {
    engine: LegacyEngine,
}

impl RecommendationEngineAdapter {
    /// Create a new RecommendationEngineAdapter
    pub fn new() -> Self {
        Self {
            engine: LegacyEngine::new(),
        }
    }

    /// Create with custom cost per hour
    pub fn with_cost_per_hour(cost_per_hour: f64) -> Self {
        let mut config = crate::cost_estimator::PricingConfig::default();
        config.cost_per_vcpu_hour = cost_per_hour;
        Self {
            engine: LegacyEngine::new().with_pricing_config(config),
        }
    }
}

impl Default for RecommendationEngineAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RecommendationGenerator for RecommendationEngineAdapter {
    async fn generate_recommendations(
        &self,
        queries: &[QueryStats],
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>> {
        // Note: Legacy engine expects (queries, tables, indexes) but trait expects (queries, indexes, tables)
        // Connection monitoring data is not available through this adapter interface (passes None)
        let recommendations = self.engine.generate_recommendations(
            queries, tables, indexes, None, // connection_stats
            None, // connections
        );
        Ok(recommendations)
    }

    fn filter_recommendations(
        &self,
        recommendations: Vec<Recommendation>,
        min_severity: crate::models::Severity,
    ) -> Vec<Recommendation> {
        recommendations
            .into_iter()
            .filter(|rec| rec.severity >= min_severity)
            .collect()
    }

    fn sort_by_priority(&self, recommendations: &mut [Recommendation]) {
        recommendations.sort_by(|a, b| {
            b.severity.cmp(&a.severity).then_with(|| {
                let a_savings = a.estimated_cost_savings_usd_monthly.unwrap_or(0.0);
                let b_savings = b.estimated_cost_savings_usd_monthly.unwrap_or(0.0);
                b_savings
                    .partial_cmp(&a_savings)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
    }
}
