use crate::models::*;
use sqlx::PgPool;
use anyhow::Result;
use chrono::Utc;

pub struct BaselineCollector {
    pool: PgPool,
}

impl BaselineCollector {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    /// Capture performance baseline before applying change
    pub async fn capture(
        &self,
        rec: &ApplicableRecommendation,
    ) -> Result<PerformanceBaseline> {
        match &rec.apply_strategy {
            ApplyStrategy::CreateIndexConcurrently { table_name, .. } => {
                self.capture_query_baseline(rec, table_name).await
            }
            
            ApplyStrategy::DropIndex { index_name, .. } => {
                self.capture_index_baseline(index_name).await
            }
            
            _ => {
                // Generic baseline for other strategies
                Ok(self.empty_baseline())
            }
        }
    }
    
    async fn capture_query_baseline(
        &self,
        rec: &ApplicableRecommendation,
        table_name: &str,
    ) -> Result<PerformanceBaseline> {
        // Get current query statistics
        let query_stats = if let Some(query) = &rec.recommendation.affected_query {
            self.get_query_stats_by_pattern(query).await?
        } else {
            None
        };
        
        // Get query plan for affected queries
        let query_plan = if let Some(query) = &rec.recommendation.affected_query {
            self.get_query_plan(query).await.ok()
        } else {
            None
        };
        
        // Get table size
        let table_size = self.get_table_size(table_name).await?;
        
        let mean_exec_time = query_stats.as_ref().map(|s| s.mean_exec_time).unwrap_or(0.0);
        let calls_per_minute = query_stats.as_ref().map(|s| s.calls as f64 / 60.0).unwrap_or(0.0);
        
        Ok(PerformanceBaseline {
            captured_at: Utc::now(),
            query_id: query_stats.as_ref().and_then(|s| s.queryid),
            table_name: Some(table_name.to_string()),
            mean_exec_time_ms: mean_exec_time,
            p95_exec_time_ms: 0.0, // TODO: Calculate from distribution
            p99_exec_time_ms: 0.0,
            calls_per_minute,
            query_plan: query_plan.clone(),
            plan_cost: query_plan.as_ref().and_then(|p| self.extract_cost(p)),
            table_size_mb: Some(table_size),
            index_size_mb: None,
            total_queries_analyzed: 1,
        })
    }
    
    async fn capture_index_baseline(&self, index_name: &str) -> Result<PerformanceBaseline> {
        let index_size = self.get_index_size(index_name).await.unwrap_or(0.0);
        
        Ok(PerformanceBaseline {
            captured_at: Utc::now(),
            query_id: None,
            table_name: None,
            mean_exec_time_ms: 0.0,
            p95_exec_time_ms: 0.0,
            p99_exec_time_ms: 0.0,
            calls_per_minute: 0.0,
            query_plan: None,
            plan_cost: None,
            table_size_mb: None,
            index_size_mb: Some(index_size),
            total_queries_analyzed: 0,
        })
    }
    
    async fn get_query_stats_by_pattern(&self, query_pattern: &str) -> Result<Option<QueryStatsSnapshot>> {
        // Query pg_stat_statements for queries matching pattern
        let row = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<f64>, Option<f64>)>(
            r#"
            SELECT 
                queryid,
                calls,
                mean_exec_time,
                total_exec_time
            FROM pg_stat_statements
            WHERE query LIKE $1
            ORDER BY calls DESC
            LIMIT 1
            "#
        )
        .bind(format!("%{}%", query_pattern))
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(row.map(|(queryid, calls, mean_exec_time, total_exec_time)| QueryStatsSnapshot {
            queryid,
            calls: calls.unwrap_or(0),
            mean_exec_time: mean_exec_time.unwrap_or(0.0),
            total_exec_time: total_exec_time.unwrap_or(0.0),
        }))
    }
    
    async fn get_query_plan(&self, query: &str) -> Result<String> {
        // Get EXPLAIN output for the query
        let plan = sqlx::query_scalar::<_, String>(
            &format!("EXPLAIN (FORMAT JSON) {}", query)
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(plan)
    }
    
    async fn get_table_size(&self, table_name: &str) -> Result<f64> {
        let size: Option<i64> = sqlx::query_scalar(
            "SELECT pg_total_relation_size($1::regclass)"
        )
        .bind(table_name)
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(size.unwrap_or(0) as f64 / (1024.0 * 1024.0)) // Convert to MB
    }
    
    async fn get_index_size(&self, index_name: &str) -> Result<f64> {
        let size: Option<i64> = sqlx::query_scalar(
            "SELECT pg_relation_size($1::regclass)"
        )
        .bind(index_name)
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(size.unwrap_or(0) as f64 / (1024.0 * 1024.0))
    }
    
    fn extract_cost(&self, plan_json: &str) -> Option<f64> {
        // Parse JSON and extract "Total Cost"
        // This is a simplified version - in production we'd use serde_json
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(plan_json) {
            value.get("Plan")
                .and_then(|p| p.get("Total Cost"))
                .and_then(|c| c.as_f64())
        } else {
            None
        }
    }
    
    fn empty_baseline(&self) -> PerformanceBaseline {
        PerformanceBaseline {
            captured_at: Utc::now(),
            query_id: None,
            table_name: None,
            mean_exec_time_ms: 0.0,
            p95_exec_time_ms: 0.0,
            p99_exec_time_ms: 0.0,
            calls_per_minute: 0.0,
            query_plan: None,
            plan_cost: None,
            table_size_mb: None,
            index_size_mb: None,
            total_queries_analyzed: 0,
        }
    }
}

#[derive(Debug)]
struct QueryStatsSnapshot {
    queryid: Option<i64>,
    calls: i64,
    mean_exec_time: f64,
    total_exec_time: f64,
}
