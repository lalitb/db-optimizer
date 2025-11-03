use crate::models::*;
use regex::Regex;
use std::collections::HashMap;

pub mod advanced;

pub struct QueryAnalyzer;

impl QueryAnalyzer {
    pub fn new() -> Self {
        Self
    }
    
    /// Find slow queries that need attention
    pub fn find_slow_queries(&self, stats: &[QueryStats]) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();
        
        for stat in stats {
            // Heuristic 1: Mean execution time > 1000ms
            if stat.mean_exec_time_ms > 1000.0 {
                recommendations.push(self.create_slow_query_recommendation(stat, "high_mean_time"));
            }
            // Heuristic 2: Total time > 60 seconds (high volume slow query)
            else if stat.total_exec_time_ms > 60_000.0 && stat.mean_exec_time_ms > 100.0 {
                recommendations.push(self.create_slow_query_recommendation(stat, "high_total_time"));
            }
        }
        
        recommendations
    }
    
    /// Suggest missing indexes based on simple heuristics
    pub fn find_missing_indexes(
        &self,
        stats: &[QueryStats],
        tables: &[TableInfo],
    ) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();
        
        for stat in stats {
            // Only analyze queries called frequently
            if stat.calls < 100 {
                continue;
            }
            
            // Heuristic: Look for WHERE clauses on large tables
            if let Some(table_name) = self.extract_table_name(&stat.query) {
                if let Some(table) = tables.iter().find(|t| t.table_name == table_name) {
                    // Only suggest for tables with significant data
                    if table.row_count > 10_000 && stat.mean_exec_time_ms > 50.0 {
                        if let Some(column) = self.extract_where_column(&stat.query) {
                            // Check if this pattern appears frequently
                            recommendations.push(
                                self.create_missing_index_recommendation(
                                    table,
                                    &column,
                                    stat,
                                )
                            );
                        }
                    }
                }
            }
        }
        
        // Deduplicate recommendations for same table+column
        self.deduplicate_index_recommendations(recommendations)
    }
    
    /// Find unused indexes wasting storage
    pub fn find_unused_indexes(&self, indexes: &[IndexInfo]) -> Vec<Recommendation> {
        indexes
            .iter()
            .filter(|idx| {
                // Don't recommend dropping primary keys or unique constraints
                !idx.is_primary && !idx.is_unique &&
                // Must have zero scans
                idx.index_scans == 0 &&
                // Must be wasting meaningful space
                idx.index_size_mb > 1.0
            })
            .map(|idx| self.create_unused_index_recommendation(idx))
            .collect()
    }
    
    /// Find high-volume queries that could benefit from optimization
    pub fn find_high_volume_queries(&self, stats: &[QueryStats]) -> Vec<Recommendation> {
        stats
            .iter()
            .filter(|stat| {
                // Very high call count with moderate execution time
                stat.calls > 10_000 && stat.mean_exec_time_ms > 20.0
            })
            .map(|stat| self.create_high_volume_recommendation(stat))
            .collect()
    }
    
    // ===== Helper Methods (Simple String/Regex Matching) =====
    
    /// Extract table name from query (simple heuristic)
    fn extract_table_name(&self, query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();
        
        // Match: FROM table_name or JOIN table_name
        let re = Regex::new(r"(?:from|join)\s+([a-z_][a-z0-9_]*)").ok()?;
        re.captures(&query_lower)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
    }
    
    /// Extract column from WHERE clause (simple heuristic)
    fn extract_where_column(&self, query: &str) -> Option<String> {
        let query_lower = query.to_lowercase();
        
        // Match: WHERE column = or WHERE column IN
        let re = Regex::new(r"where\s+([a-z_][a-z0-9_]*)\s*(?:=|in|>|<)").ok()?;
        re.captures(&query_lower)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
    }
    
    /// Create recommendation for slow query
    fn create_slow_query_recommendation(&self, stat: &QueryStats, reason: &str) -> Recommendation {
        let severity = if stat.mean_exec_time_ms > 5000.0 {
            Severity::Critical
        } else if stat.mean_exec_time_ms > 2000.0 {
            Severity::High
        } else if stat.mean_exec_time_ms > 1000.0 {
            Severity::Medium
        } else {
            Severity::Low
        };
        
        let description = match reason {
            "high_mean_time" => format!(
                "This query has an average execution time of {:.0}ms. Queries taking over 1 second can significantly impact user experience.",
                stat.mean_exec_time_ms
            ),
            "high_total_time" => format!(
                "This query is called {} times with average execution time of {:.0}ms, consuming {:.1} seconds of database time. Optimizing it could have a significant impact.",
                stat.calls, stat.mean_exec_time_ms, stat.total_exec_time_ms / 1000.0
            ),
            _ => format!(
                "This query needs optimization. Average execution time: {:.0}ms, called {} times.",
                stat.mean_exec_time_ms, stat.calls
            ),
        };
        
        Recommendation {
            id: format!("slow_query_{}", stat.queryid.unwrap_or(0)),
            title: format!("Slow query detected ({:.0}ms average)", stat.mean_exec_time_ms),
            description,
            issue_type: IssueType::SlowQuery,
            severity,
            
            affected_queries_count: Some(stat.calls),
            estimated_time_savings_ms: Some(stat.mean_exec_time_ms * 0.5), // Assume 50% improvement possible
            estimated_cost_savings_usd_monthly: None, // Calculated later
            
            migration_sql: format!(
                "-- Run EXPLAIN ANALYZE to understand query execution:\nEXPLAIN (ANALYZE, BUFFERS) {};",
                truncate_query(&stat.query, 200)
            ),
            rollback_sql: None,
            safety_notes: "Use EXPLAIN ANALYZE to identify bottlenecks. Consider adding indexes, rewriting the query, or caching results.".to_string(),
            
            affected_table: self.extract_table_name(&stat.query),
            affected_query: Some(truncate_query(&stat.query, 200)),
        }
    }
    
    /// Create recommendation for missing index
    fn create_missing_index_recommendation(
        &self,
        table: &TableInfo,
        column: &str,
        stat: &QueryStats,
    ) -> Recommendation {
        let index_name = format!("idx_{}_{}", table.table_name, column);
        
        let severity = if stat.mean_exec_time_ms > 500.0 {
            Severity::High
        } else if stat.mean_exec_time_ms > 200.0 {
            Severity::Medium
        } else {
            Severity::Low
        };
        
        Recommendation {
            id: format!("missing_index_{}_{}", table.table_name, column),
            title: format!("Add index on {}.{}", table.table_name, column),
            description: format!(
                "Queries filtering by this column are executed {} times with average time of {:.0}ms. \
                Table has {} rows. An index could significantly improve performance.",
                stat.calls, stat.mean_exec_time_ms, table.row_count
            ),
            issue_type: IssueType::MissingIndex,
            severity,
            
            affected_queries_count: Some(stat.calls),
            estimated_time_savings_ms: Some(stat.mean_exec_time_ms * 0.7), // Assume 70% improvement
            estimated_cost_savings_usd_monthly: None, // Calculated later
            
            migration_sql: format!(
                "CREATE INDEX CONCURRENTLY {} ON {} ({});",
                index_name, table.table_name, column
            ),
            rollback_sql: Some(format!("DROP INDEX IF EXISTS {};", index_name)),
            safety_notes: "Using CONCURRENTLY means this operation won't lock the table. Safe to run in production.".to_string(),
            
            affected_table: Some(table.table_name.clone()),
            affected_query: Some(truncate_query(&stat.query, 100)),
        }
    }
    
    /// Create recommendation for unused index
    fn create_unused_index_recommendation(&self, index: &IndexInfo) -> Recommendation {
        let severity = if index.index_size_mb > 100.0 {
            Severity::High
        } else if index.index_size_mb > 10.0 {
            Severity::Medium
        } else {
            Severity::Low
        };
        
        Recommendation {
            id: format!("unused_index_{}", index.index_name),
            title: format!("Drop unused index {}", index.index_name),
            description: format!(
                "Index {} on table {} has never been scanned and is consuming {:.1} MB of storage. \
                Removing it will free up disk space and speed up write operations.",
                index.index_name, index.table_name, index.index_size_mb
            ),
            issue_type: IssueType::UnusedIndex,
            severity,
            
            affected_queries_count: None,
            estimated_time_savings_ms: None,
            estimated_cost_savings_usd_monthly: Some(index.index_size_mb * 0.1), // $0.10 per GB-month = $0.0001 per MB
            
            migration_sql: format!("DROP INDEX CONCURRENTLY IF EXISTS {};", index.index_name),
            rollback_sql: None, // Can't easily recreate without knowing the index definition
            safety_notes: format!(
                "Using CONCURRENTLY means this operation won't lock the table. \
                Free up {:.1} MB of storage. Ensure this index is truly unused before dropping.",
                index.index_size_mb
            ),
            
            affected_table: Some(format!("{}.{}", index.schema_name, index.table_name)),
            affected_query: None,
        }
    }
    
    /// Create recommendation for high-volume query
    fn create_high_volume_recommendation(&self, stat: &QueryStats) -> Recommendation {
        let severity = if stat.calls > 100_000 && stat.mean_exec_time_ms > 100.0 {
            Severity::High
        } else if stat.calls > 50_000 {
            Severity::Medium
        } else {
            Severity::Low
        };
        
        Recommendation {
            id: format!("high_volume_query_{}", stat.queryid.unwrap_or(0)),
            title: format!("High-volume query ({} calls)", stat.calls),
            description: format!(
                "This query is executed {} times with average execution time of {:.0}ms. \
                Consider caching results, using materialized views, or optimizing the query.",
                stat.calls, stat.mean_exec_time_ms
            ),
            issue_type: IssueType::HighVolumeQuery,
            severity,
            
            affected_queries_count: Some(stat.calls),
            estimated_time_savings_ms: Some(stat.mean_exec_time_ms * 0.3), // Conservative 30% improvement
            estimated_cost_savings_usd_monthly: None, // Calculated later
            
            migration_sql: format!(
                "-- Consider caching or using a materialized view:\n-- CREATE MATERIALIZED VIEW cached_result AS\n-- {};",
                truncate_query(&stat.query, 200)
            ),
            rollback_sql: None,
            safety_notes: "Consider application-level caching (Redis, Memcached) or database-level materialized views. Profile query execution plan first.".to_string(),
            
            affected_table: self.extract_table_name(&stat.query),
            affected_query: Some(truncate_query(&stat.query, 200)),
        }
    }
    
    /// Deduplicate index recommendations (same table+column)
    fn deduplicate_index_recommendations(&self, recs: Vec<Recommendation>) -> Vec<Recommendation> {
        let mut seen: HashMap<String, Recommendation> = HashMap::new();
        
        for rec in recs {
            let key = rec.id.clone();
            
            // Keep the highest severity recommendation for each unique table+column
            seen.entry(key.clone())
                .and_modify(|existing| {
                    if rec.severity > existing.severity {
                        *existing = rec.clone();
                    }
                })
                .or_insert(rec);
        }
        
        seen.into_values().collect()
    }
}

/// Truncate query for display
fn truncate_query(query: &str, max_len: usize) -> String {
    let cleaned = query.trim().replace('\n', " ").replace("  ", " ");
    if cleaned.len() <= max_len {
        cleaned
    } else {
        format!("{}...", &cleaned[..max_len])
    }
}
