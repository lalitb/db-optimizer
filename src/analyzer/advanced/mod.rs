// Advanced Index Intelligence Module
// Provides intelligent index type selection, composite index design, and partial index recommendations

pub mod column_stats;
pub mod index_type_selector;

use crate::models::Recommendation;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedIndexRecommendation {
    pub priority: i32,
    pub impact: ImpactLevel,
    pub single_column_indexes: Vec<index_type_selector::IndexRecommendation>,
    pub estimated_total_improvement: f64,
    pub estimated_total_size_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImpactLevel {
    Critical, // >90% improvement
    High,     // 70-90% improvement
    Medium,   // 40-70% improvement
    Low,      // <40% improvement
}

impl ImpactLevel {
    pub fn as_str(&self) -> &str {
        match self {
            ImpactLevel::Critical => "CRITICAL",
            ImpactLevel::High => "HIGH",
            ImpactLevel::Medium => "MEDIUM",
            ImpactLevel::Low => "LOW",
        }
    }
}

pub struct AdvancedIndexAnalyzer {
    pool: PgPool,
}

impl AdvancedIndexAnalyzer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Analyze a table and generate advanced index recommendations
    pub async fn analyze_table(
        &mut self,
        schema: &str,
        table: &str,
    ) -> Result<AdvancedIndexRecommendation, Box<dyn std::error::Error>> {
        let mut stats_collector = column_stats::ColumnStatsCollector::new(self.pool.clone());
        let type_selector = index_type_selector::IndexTypeSelector::new();

        // Get all columns that need indexing
        let columns = self.get_columns_needing_indexes(schema, table).await?;

        let mut single_column_recs = Vec::new();

        for column in columns {
            // Collect comprehensive statistics
            let stats = stats_collector
                .collect_column_stats(schema, table, &column)
                .await?;

            // Select optimal index type
            let predicates = Vec::new(); // TODO: Extract from pg_stat_statements
            let rec = type_selector.select_index_type(&stats, &predicates);
            single_column_recs.push(rec);
        }

        // Calculate aggregates
        let total_improvement = single_column_recs
            .iter()
            .map(|r| r.estimated_improvement_pct)
            .fold(0.0, f64::max);

        let total_size = single_column_recs
            .iter()
            .map(|r| r.estimated_size_mb)
            .sum::<f64>();

        Ok(AdvancedIndexRecommendation {
            priority: Self::calculate_priority(total_improvement),
            impact: Self::classify_impact(total_improvement),
            single_column_indexes: single_column_recs,
            estimated_total_improvement: total_improvement,
            estimated_total_size_mb: total_size,
        })
    }

    async fn get_columns_needing_indexes(
        &self,
        schema: &str,
        table: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        // Find columns that appear in WHERE clauses without indexes
        let query = r#"
            SELECT DISTINCT 
                c.column_name
            FROM information_schema.columns c
            WHERE c.table_schema = $1
              AND c.table_name = $2
              AND NOT EXISTS (
                  SELECT 1 
                  FROM pg_indexes i
                  WHERE i.schemaname = c.table_schema
                    AND i.tablename = c.table_name
                    AND i.indexdef LIKE '%' || c.column_name || '%'
              )
            LIMIT 10
        "#;

        let rows = sqlx::query(query)
            .bind(schema)
            .bind(table)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .iter()
            .map(|row| row.get::<String, _>("column_name"))
            .collect())
    }

    fn calculate_priority(improvement: f64) -> i32 {
        match improvement {
            i if i > 90.0 => 1,
            i if i > 70.0 => 2,
            i if i > 40.0 => 3,
            _ => 4,
        }
    }

    fn classify_impact(improvement: f64) -> ImpactLevel {
        match improvement {
            i if i > 90.0 => ImpactLevel::Critical,
            i if i > 70.0 => ImpactLevel::High,
            i if i > 40.0 => ImpactLevel::Medium,
            _ => ImpactLevel::Low,
        }
    }
}

/// Convert advanced recommendations to standard Recommendation format
pub fn convert_to_recommendations(advanced: AdvancedIndexRecommendation) -> Vec<Recommendation> {
    advanced
        .single_column_indexes
        .into_iter()
        .map(|idx_rec| Recommendation {
            id: format!(
                "advanced_index_{}_{}",
                idx_rec.columns.join("_"),
                idx_rec.index_type.as_str()
            ),
            issue_type: crate::models::IssueType::MissingIndex,
            severity: match advanced.impact {
                ImpactLevel::Critical => crate::models::Severity::Critical,
                ImpactLevel::High => crate::models::Severity::High,
                ImpactLevel::Medium => crate::models::Severity::Medium,
                ImpactLevel::Low => crate::models::Severity::Low,
            },
            title: format!(
                "Create {} index on {}",
                idx_rec.index_type.as_str(),
                idx_rec.columns.join(", ")
            ),
            description: idx_rec.reasoning.clone(),
            affected_queries_count: None,
            estimated_time_savings_ms: Some(idx_rec.estimated_improvement_pct * 10.0),
            estimated_cost_savings_usd_monthly: Some(
                (idx_rec.estimated_improvement_pct / 100.0) * 500.0,
            ),
            migration_sql: idx_rec.ddl.clone(),
            rollback_sql: None,
            safety_notes: format!(
                "Index type: {}. Estimated size: {:.1}MB. Confidence: {:.0}%",
                idx_rec.index_type.as_str(),
                idx_rec.estimated_size_mb,
                idx_rec.confidence_score * 100.0
            ),
            affected_table: None,
            affected_query: None,
        })
        .collect()
}
