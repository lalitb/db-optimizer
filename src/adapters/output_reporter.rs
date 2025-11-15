// src/adapters/output_reporter.rs - Reporter trait implementation

use anyhow::Result;

use crate::traits::Reporter;
use crate::models::{QueryStats, Recommendation, AnalysisReport, ReportSummary};
use crate::output::OutputFormatter;
use chrono::Utc;

/// Implementation of the Reporter trait using OutputFormatter
pub struct OutputReporter {
    formatter: OutputFormatter,
}

impl OutputReporter {
    /// Create a new OutputReporter
    pub fn new() -> Self {
        Self {
            formatter: OutputFormatter,
        }
    }
}

impl Default for OutputReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for OutputReporter {
    fn generate_report(
        &self,
        queries: &[QueryStats],
        recommendations: &[Recommendation],
        database_name: &str,
        postgres_version: &str,
    ) -> Result<AnalysisReport> {
        // Calculate summary statistics
        let issues_found = recommendations.len();
        let critical_issues = recommendations
            .iter()
            .filter(|r| matches!(r.severity, crate::models::Severity::Critical))
            .count();
        let high_priority_issues = recommendations
            .iter()
            .filter(|r| matches!(r.severity, crate::models::Severity::High))
            .count();
        
        let slow_queries = recommendations
            .iter()
            .filter(|r| matches!(r.issue_type, crate::models::IssueType::SlowQuery))
            .count();
        let missing_indexes_suggested = recommendations
            .iter()
            .filter(|r| matches!(r.issue_type, crate::models::IssueType::MissingIndex))
            .count();
        let unused_indexes_found = recommendations
            .iter()
            .filter(|r| matches!(r.issue_type, crate::models::IssueType::UnusedIndex))
            .count();
        
        let total_estimated_savings_usd_monthly = recommendations
            .iter()
            .filter_map(|r| r.estimated_cost_savings_usd_monthly)
            .sum();
        
        let summary = ReportSummary {
            issues_found,
            critical_issues,
            high_priority_issues,
            slow_queries,
            missing_indexes_suggested,
            unused_indexes_found,
            total_estimated_savings_usd_monthly,
        };
        
        Ok(AnalysisReport {
            database_name: database_name.to_string(),
            postgres_version: postgres_version.to_string(),
            analyzed_at: Utc::now(),
            summary,
            recommendations: recommendations.to_vec(),
            total_queries_analyzed: queries.len(),
            total_tables: 0, // Would need to pass this in
            total_indexes: 0, // Would need to pass this in
        })
    }
    
    fn format_terminal(&self, report: &AnalysisReport) -> Result<String> {
        Ok(self.formatter.format_terminal(report))
    }
    
    fn format_json(&self, report: &AnalysisReport) -> Result<String> {
        let json = serde_json::to_string_pretty(report)?;
        Ok(json)
    }
    
    fn format_markdown(&self, report: &AnalysisReport) -> Result<String> {
        Ok(self.formatter.format_markdown(report))
    }
}
