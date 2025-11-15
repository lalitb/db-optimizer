// src/adapters/recommendation_engine.rs - RecommendationGenerator trait implementation

use anyhow::Result;
use async_trait::async_trait;

use crate::traits::RecommendationGenerator;
use crate::models::{QueryStats, IndexInfo, TableInfo, Recommendation, Severity};
use crate::recommendations::RecommendationEngine as LegacyEngine;

/// Adapter for the existing RecommendationEngine to implement the RecommendationGenerator trait
pub struct RecommendationEngineAdapter {
    engine: LegacyEngine,
}

impl RecommendationEngineAdapter {
    /// Create a new RecommendationEngineAdapter
    pub fn new(engine: LegacyEngine) -> Self {
        Self { engine }
    }
    
    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self {
            engine: LegacyEngine::new(0.07), // Default cost per hour
        }
    }
    
    /// Create with custom cost per hour
    pub fn with_cost(cost_per_hour: f64) -> Self {
        Self {
            engine: LegacyEngine::new(cost_per_hour),
        }
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
        // The legacy engine generates recommendations synchronously
        let recommendations = self.engine.generate_recommendations(queries, indexes, tables);
        Ok(recommendations)
    }
    
    fn filter_recommendations(
        &self,
        mut recommendations: Vec<Recommendation>,
        min_severity: Severity,
    ) -> Vec<Recommendation> {
        recommendations.retain(|rec| rec.severity >= min_severity);
        recommendations
    }
    
    fn sort_by_priority(&self, recommendations: &mut [Recommendation]) {
        // Sort by severity (descending) then by estimated savings (descending)
        recommendations.sort_by(|a, b| {
            // First compare by severity
            match b.severity.cmp(&a.severity) {
                std::cmp::Ordering::Equal => {
                    // If same severity, compare by estimated savings
                    let a_savings = a.estimated_cost_savings_usd_monthly.unwrap_or(0.0);
                    let b_savings = b.estimated_cost_savings_usd_monthly.unwrap_or(0.0);
                    b_savings.partial_cmp(&a_savings).unwrap_or(std::cmp::Ordering::Equal)
                }
                other => other,
            }
        });
    }
}
