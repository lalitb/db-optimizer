use crate::analyzer::{ConnectionAnalyzer, QueryAnalyzer};
use crate::config::AnalysisConfig;
use crate::cost_estimator::{CostEstimator, PricingConfig};
use crate::models::*;
pub struct RecommendationEngine {
    analyzer: QueryAnalyzer,
    connection_analyzer: ConnectionAnalyzer,
    cost_estimator: CostEstimator,
}

impl RecommendationEngine {
    pub fn new() -> Self {
        Self {
            analyzer: QueryAnalyzer::new(AnalysisConfig::default()),
            connection_analyzer: ConnectionAnalyzer::new(),
            cost_estimator: CostEstimator::new(PricingConfig::default()),
        }
    }

    pub fn with_analysis_config(mut self, config: AnalysisConfig) -> Self {
        self.analyzer = QueryAnalyzer::new(config);
        self
    }

    pub fn with_pricing_config(mut self, config: PricingConfig) -> Self {
        self.cost_estimator = CostEstimator::new(config);
        self
    }

    /// Generate all recommendations
    pub fn generate_recommendations(
        &self,
        queries: &[QueryStats],
        tables: &[TableInfo],
        indexes: &[IndexInfo],
        connection_stats: Option<&ConnectionStats>,
        connections: Option<&[ConnectionInfo]>,
    ) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();

        // Find slow queries
        recommendations.extend(self.analyzer.find_slow_queries(queries));

        // Find missing indexes
        recommendations.extend(self.analyzer.find_missing_indexes(queries, tables));

        // Find unused indexes
        recommendations.extend(self.analyzer.find_unused_indexes(indexes));

        // Find high-volume queries
        recommendations.extend(self.analyzer.find_high_volume_queries(queries));

        // Check connection issues (if connection data provided)
        if let (Some(stats), Some(conns)) = (connection_stats, connections) {
            recommendations.extend(self.connection_analyzer.analyze_connections(stats, conns));
        }

        // Estimate cost savings for each
        for rec in recommendations.iter_mut() {
            self.cost_estimator.estimate_savings(rec);
        }

        // Sort by severity then by cost savings
        recommendations.sort_by(|a, b| {
            b.severity.cmp(&a.severity).then_with(|| {
                let a_savings = a.estimated_cost_savings_usd_monthly.unwrap_or(0.0);
                let b_savings = b.estimated_cost_savings_usd_monthly.unwrap_or(0.0);
                b_savings
                    .partial_cmp(&a_savings)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        recommendations
    }

    /// Generate summary statistics
    pub fn generate_summary(recommendations: &[Recommendation]) -> ReportSummary {
        let issues_found = recommendations.len();
        let critical_issues = recommendations
            .iter()
            .filter(|r| r.severity == Severity::Critical)
            .count();
        let high_priority_issues = recommendations
            .iter()
            .filter(|r| r.severity == Severity::High)
            .count();

        let slow_queries = recommendations
            .iter()
            .filter(|r| r.issue_type == IssueType::SlowQuery)
            .count();
        let missing_indexes = recommendations
            .iter()
            .filter(|r| r.issue_type == IssueType::MissingIndex)
            .count();
        let unused_indexes = recommendations
            .iter()
            .filter(|r| r.issue_type == IssueType::UnusedIndex)
            .count();

        let total_savings: f64 = recommendations
            .iter()
            .filter_map(|r| r.estimated_cost_savings_usd_monthly)
            .sum();

        ReportSummary {
            issues_found,
            critical_issues,
            high_priority_issues,
            slow_queries,
            missing_indexes_suggested: missing_indexes,
            unused_indexes_found: unused_indexes,
            total_estimated_savings_usd_monthly: total_savings,
        }
    }
}

impl Default for RecommendationEngine {
    fn default() -> Self {
        Self::new()
    }
}
