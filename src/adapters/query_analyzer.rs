// src/adapters/query_analyzer.rs - Analyzer trait implementation

use anyhow::Result;
use async_trait::async_trait;

use crate::analyzer::QueryAnalyzer as LegacyAnalyzer;
use crate::config::AnalysisConfig;
use crate::models::{IndexInfo, QueryStats, Recommendation, TableInfo};
use crate::traits::Analyzer;

/// Adapter for the existing QueryAnalyzer to implement the Analyzer trait
pub struct QueryAnalyzerAdapter {
    analyzer: LegacyAnalyzer,
}

impl QueryAnalyzerAdapter {
    /// Create a new QueryAnalyzerAdapter
    pub fn new() -> Self {
        Self {
            analyzer: LegacyAnalyzer::new(AnalysisConfig::default()),
        }
    }

    /// Create with default configuration (alias for new)
    pub fn with_defaults() -> Self {
        Self::new()
    }
}

impl Default for QueryAnalyzerAdapter {
    fn default() -> Self {
        Self::new()
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
        // The legacy analyzer has individual methods for different types of issues
        let mut recommendations = Vec::new();

        // Find slow queries
        recommendations.extend(self.analyzer.find_slow_queries(queries));

        // Find missing indexes
        recommendations.extend(self.analyzer.find_missing_indexes(queries, tables));

        // Find unused indexes
        recommendations.extend(self.analyzer.find_unused_indexes(indexes));

        // Find high-volume queries
        recommendations.extend(self.analyzer.find_high_volume_queries(queries));

        Ok(recommendations)
    }

    async fn analyze_single_query(
        &self,
        query: &QueryStats,
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>> {
        // Analyze a single query by passing it as a slice
        self.analyze_queries(&[query.clone()], indexes, tables)
            .await
    }
}
