// src/adapters/output_reporter.rs - Reporter trait implementation

use anyhow::Result;
use async_trait::async_trait;

use crate::models::AnalysisReport;
use crate::output::OutputFormatter;
use crate::traits::Reporter;

/// Adapter for OutputFormatter to implement the Reporter trait
pub struct OutputReporterAdapter {
    // OutputFormatter is stateless, so we don't need to store it
}

impl OutputReporterAdapter {
    /// Create a new OutputReporterAdapter
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for OutputReporterAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for OutputReporterAdapter {
    fn generate_report(
        &self,
        queries: &[crate::models::QueryStats],
        recommendations: &[crate::models::Recommendation],
        database_name: &str,
        postgres_version: &str,
    ) -> Result<AnalysisReport> {
        use crate::recommendations::RecommendationEngine;

        let summary = RecommendationEngine::generate_summary(recommendations);

        Ok(AnalysisReport {
            database_name: database_name.to_string(),
            postgres_version: postgres_version.to_string(),
            analyzed_at: chrono::Utc::now(),
            total_queries_analyzed: queries.len(),
            total_tables: 0,  // Not provided in this context
            total_indexes: 0, // Not provided in this context
            summary,
            recommendations: recommendations.to_vec(),
            connection_stats: None, // Not available in this adapter context
            health_score: 100,      // Default to 100 when not calculated
        })
    }

    fn format_terminal(&self, report: &AnalysisReport) -> Result<String> {
        // OutputFormatter methods are static
        Ok(OutputFormatter::format_terminal(report))
    }

    fn format_json(&self, report: &AnalysisReport) -> Result<String> {
        OutputFormatter::format_json(report)
    }

    fn format_markdown(&self, report: &AnalysisReport) -> Result<String> {
        Ok(OutputFormatter::format_markdown(report))
    }
}
