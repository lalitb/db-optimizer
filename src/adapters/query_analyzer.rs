// src/adapters/query_analyzer.rs - Analyzer trait implementation

use anyhow::Result;
use async_trait::async_trait;

use crate::traits::Analyzer;
use crate::models::{QueryStats, IndexInfo, TableInfo, Recommendation};
use crate::analyzer::QueryAnalyzer as LegacyAnalyzer;

/// Adapter for the existing QueryAnalyzer to implement the Analyzer trait
pub struct QueryAnalyzerAdapter {
    analyzer: LegacyAnalyzer,
}

impl QueryAnalyzerAdapter {
    /// Create a new QueryAnalyzerAdapter
    pub fn new(analyzer: LegacyAnalyzer) -> Self {
        Self { analyzer }
    }
    
    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self {
            analyzer: LegacyAnalyzer::new(0.07), // Default cost per hour
        }
    }
    
    /// Create with custom cost per hour
    pub fn with_cost(cost_per_hour: f64) -> Self {
        Self {
            analyzer: LegacyAnalyzer::new(cost_per_hour),
        }
    }
}

#[async_trait]
impl Analyzer for QueryAnalyzerAdapter {
    async fn analyze_queries(
        &self,
        queries: &[QueryStats],
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>> {
        // The legacy analyzer returns recommendations directly
        // No need for async here since the legacy implementation is synchronous
        let recommendations = self.analyzer.analyze_queries(queries, indexes, tables)?;
        Ok(recommendations)
    }
    
    async fn analyze_single_query(
        &self,
        query: &QueryStats,
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>> {
        // Analyze a single query by passing it as a slice
        self.analyze_queries(&[query.clone()], indexes, tables).await
    }
}
