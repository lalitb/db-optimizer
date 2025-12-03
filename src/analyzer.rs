use crate::models::*;
use crate::config::AnalysisConfig;
use regex::Regex;
use std::collections::HashMap;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

pub mod advanced;
pub mod hypothetical;

pub struct QueryAnalyzer {
    config: AnalysisConfig,
}

impl QueryAnalyzer {
    pub fn new(config: AnalysisConfig) -> Self {
        Self { config }
    }
    
    /// Find slow queries that need attention
    pub fn find_slow_queries(&self, stats: &[QueryStats]) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();
        
        for stat in stats {
            // Heuristic 1: Mean execution time > threshold
            if stat.mean_exec_time_ms > self.config.thresholds.slow_query_ms {
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
                idx.index_size_mb > self.config.thresholds.unused_index_size_mb
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
                stat.calls > self.config.thresholds.high_volume_calls && stat.mean_exec_time_ms > 20.0
            })
            .map(|stat| self.create_high_volume_recommendation(stat))
            .collect()
    }
    
    // ===== Helper Methods (Simple String/Regex Matching) =====
    
    /// Extract table name from query using sqlparser
    fn extract_table_name(&self, query: &str) -> Option<String> {
        let dialect = PostgreSqlDialect {};
        let ast = Parser::parse_sql(&dialect, query).ok()?;
        
        // Get first statement
        if let Some(statement) = ast.first() {
            match statement {
                sqlparser::ast::Statement::Query(query) => {
                    if let sqlparser::ast::SetExpr::Select(select) = &*query.body {
                        // Look at FROM
                        for table_with_joins in &select.from {
                            if let sqlparser::ast::TableFactor::Table { name, .. } = &table_with_joins.relation {
                                return Some(name.to_string());
                            }
                        }
                    }
                }
                sqlparser::ast::Statement::Update { table, .. } => {
                    if let sqlparser::ast::TableFactor::Table { name, .. } = &table.relation {
                        return Some(name.to_string());
                    }
                }
                sqlparser::ast::Statement::Delete { from, .. } => {
                    for table_with_joins in from {
                        if let sqlparser::ast::TableFactor::Table { name, .. } = &table_with_joins.relation {
                            return Some(name.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        
        // Fallback to regex if parser fails or structure is complex
        let query_lower = query.to_lowercase();
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

// ============================================================================
// CONNECTION ANALYZER
// ============================================================================

pub struct ConnectionAnalyzer {
    /// Threshold for idle in transaction warning (seconds)
    idle_in_transaction_threshold_secs: i64,
    /// Threshold for connection pool usage warning (percentage)
    connection_pool_warning_threshold: f64,
    /// Threshold for lock wait warning (seconds)
    lock_wait_threshold_secs: i64,
}

impl ConnectionAnalyzer {
    pub fn new() -> Self {
        Self {
            idle_in_transaction_threshold_secs: 30,  // 30 seconds default
            connection_pool_warning_threshold: 80.0,  // 80% usage
            lock_wait_threshold_secs: 5,              // 5 seconds
        }
    }

    /// Create analyzer with custom thresholds
    pub fn with_thresholds(
        idle_threshold: i64,
        pool_threshold: f64,
        lock_threshold: i64,
    ) -> Self {
        Self {
            idle_in_transaction_threshold_secs: idle_threshold,
            connection_pool_warning_threshold: pool_threshold,
            lock_wait_threshold_secs: lock_threshold,
        }
    }

    /// Analyze connection statistics and generate recommendations
    pub fn analyze_connections(
        &self,
        stats: &ConnectionStats,
        connections: &[ConnectionInfo],
    ) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();

        // Check for connection pool exhaustion
        if let Some(rec) = self.check_connection_pool_exhaustion(stats) {
            recommendations.push(rec);
        }

        // Check for idle in transaction issues
        if let Some(rec) = self.check_idle_in_transaction(stats, connections) {
            recommendations.push(rec);
        }

        // Check for lock contention
        if let Some(rec) = self.check_lock_contention(stats, connections) {
            recommendations.push(rec);
        }

        recommendations
    }

    /// Check if connection pool is near exhaustion
    fn check_connection_pool_exhaustion(&self, stats: &ConnectionStats) -> Option<Recommendation> {
        if !stats.is_near_limit(self.connection_pool_warning_threshold) {
            return None;
        }

        let usage_pct = stats.usage_percentage();
        let severity = if usage_pct >= 95.0 {
            Severity::Critical
        } else if usage_pct >= 90.0 {
            Severity::High
        } else {
            Severity::Medium
        };

        Some(Recommendation {
            id: "connection_pool_exhaustion".to_string(),
            title: format!(
                "Connection pool usage at {:.1}%",
                usage_pct
            ),
            description: format!(
                "Current connections: {} / {} ({:.1}% used). High connection usage can lead to \
                application errors when the pool is exhausted. Consider increasing max_connections, \
                optimizing connection pooling, or investigating connection leaks.",
                stats.total_connections,
                stats.max_connections,
                usage_pct
            ),
            issue_type: IssueType::ConnectionPoolExhaustion,
            severity,

            affected_queries_count: Some(stats.total_connections as i64),
            estimated_time_savings_ms: None,
            estimated_cost_savings_usd_monthly: None,

            migration_sql: format!(
                "-- Option 1: Increase max_connections (requires restart)\n\
                -- WARNING: Each connection consumes memory (~10MB)\n\
                ALTER SYSTEM SET max_connections = {};\n\
                -- Then restart PostgreSQL\n\n\
                -- Option 2: Implement connection pooling (pgBouncer recommended)\n\
                -- See: https://www.pgbouncer.org/\n\n\
                -- Option 3: Investigate connection leaks in application\n\
                SELECT * FROM pg_stat_activity WHERE state = 'idle' ORDER BY state_change LIMIT 20;",
                (stats.max_connections as f64 * 1.5) as i32
            ),
            rollback_sql: Some(format!(
                "ALTER SYSTEM SET max_connections = {};\n-- Then restart PostgreSQL",
                stats.max_connections
            )),
            safety_notes: format!(
                "Increasing max_connections increases memory usage (~10MB per connection). \
                Current setting: {}. Recommended maximum based on available RAM. \
                Consider implementing connection pooling (pgBouncer, PgPool II) instead of \
                increasing max_connections.",
                stats.max_connections
            ),

            affected_table: None,
            affected_query: None,
        })
    }

    /// Check for idle in transaction connections
    fn check_idle_in_transaction(
        &self,
        stats: &ConnectionStats,
        connections: &[ConnectionInfo],
    ) -> Option<Recommendation> {
        if stats.idle_in_transaction_count == 0 {
            return None;
        }

        let long_idle: Vec<&ConnectionInfo> = connections
            .iter()
            .filter(|c| {
                c.state.is_problematic() &&
                c.state_duration_secs()
                    .map_or(false, |d| d > self.idle_in_transaction_threshold_secs)
            })
            .collect();

        if long_idle.is_empty() {
            return None;
        }

        let severity = if stats.idle_in_transaction_count > 10 {
            Severity::Critical
        } else if stats.idle_in_transaction_count > 5 {
            Severity::High
        } else {
            Severity::Medium
        };

        let example_queries: Vec<String> = long_idle
            .iter()
            .take(3)
            .map(|c| format!("  PID {}: {} ({}s)", c.pid, truncate_query(&c.query, 80), c.state_duration_secs().unwrap_or(0)))
            .collect();

        Some(Recommendation {
            id: "idle_in_transaction".to_string(),
            title: format!(
                "{} idle in transaction connection{}",
                stats.idle_in_transaction_count,
                if stats.idle_in_transaction_count > 1 { "s" } else { "" }
            ),
            description: format!(
                "Detected {} connection(s) in 'idle in transaction' state. Longest: {}s. \
                Idle transactions hold locks, preventing VACUUM from cleaning up dead rows and can \
                cause table bloat. This is usually caused by:\n\
                1. Application not committing/rolling back transactions\n\
                2. Connection pooling misconfiguration\n\
                3. Long-running application logic between queries\n\n\
                Example connections:\n{}",
                stats.idle_in_transaction_count,
                stats.oldest_idle_transaction_secs.unwrap_or(0),
                example_queries.join("\n")
            ),
            issue_type: IssueType::IdleInTransaction,
            severity,

            affected_queries_count: Some(stats.idle_in_transaction_count as i64),
            estimated_time_savings_ms: None,
            estimated_cost_savings_usd_monthly: None,

            migration_sql: format!(
                "-- Option 1: Set statement_timeout to auto-abort long transactions\n\
                ALTER SYSTEM SET idle_in_transaction_session_timeout = '30s';\n\
                -- Restart required\n\n\
                -- Option 2: Kill specific idle transactions\n\
                SELECT pg_terminate_backend(pid)\n\
                FROM pg_stat_activity\n\
                WHERE state = 'idle in transaction'\n\
                  AND (now() - state_change) > interval '30 seconds';\n\n\
                -- Option 3: View all idle transactions\n\
                SELECT pid, usename, application_name, state, query, \n\
                       now() - state_change as idle_duration\n\
                FROM pg_stat_activity\n\
                WHERE state LIKE 'idle in transaction%'\n\
                ORDER BY state_change;"
            ),
            rollback_sql: Some(
                "ALTER SYSTEM SET idle_in_transaction_session_timeout = '0';\n-- Then restart PostgreSQL".to_string()
            ),
            safety_notes: format!(
                "Idle transactions block VACUUM and can cause severe table bloat. \
                Investigate application code to ensure transactions are committed/rolled back promptly. \
                Consider setting idle_in_transaction_session_timeout to automatically terminate \
                idle transactions after a timeout (e.g., 30 seconds)."
            ),

            affected_table: None,
            affected_query: Some(format!("Example: {}", long_idle.first().map(|c| &c.query).unwrap_or(&String::from("N/A")))),
        })
    }

    /// Check for lock contention issues
    fn check_lock_contention(
        &self,
        stats: &ConnectionStats,
        connections: &[ConnectionInfo],
    ) -> Option<Recommendation> {
        if stats.locked_connections_count == 0 {
            return None;
        }

        let long_locked: Vec<&ConnectionInfo> = connections
            .iter()
            .filter(|c| {
                c.is_locked() &&
                c.query_duration_secs()
                    .map_or(false, |d| d > self.lock_wait_threshold_secs)
            })
            .collect();

        if long_locked.is_empty() {
            return None;
        }

        let severity = if stats.locked_connections_count > 20 {
            Severity::Critical
        } else if stats.locked_connections_count > 10 {
            Severity::High
        } else {
            Severity::Medium
        };

        let example_queries: Vec<String> = long_locked
            .iter()
            .take(3)
            .map(|c| format!("  PID {}: {} (waiting {}s)", c.pid, truncate_query(&c.query, 80), c.query_duration_secs().unwrap_or(0)))
            .collect();

        Some(Recommendation {
            id: "lock_contention".to_string(),
            title: format!(
                "{} connection{} waiting for locks",
                stats.locked_connections_count,
                if stats.locked_connections_count > 1 { "s" } else { "" }
            ),
            description: format!(
                "Detected {} connection(s) waiting for locks. Longest wait: {}s. \
                Lock contention occurs when multiple transactions try to modify the same data simultaneously. \
                Common causes:\n\
                1. Missing indexes causing full table scans with locks\n\
                2. Long-running transactions\n\
                3. Inefficient queries updating many rows\n\n\
                Waiting connections:\n{}",
                stats.locked_connections_count,
                stats.oldest_locked_query_secs.unwrap_or(0),
                example_queries.join("\n")
            ),
            issue_type: IssueType::LockContention,
            severity,

            affected_queries_count: Some(stats.locked_connections_count as i64),
            estimated_time_savings_ms: stats.oldest_locked_query_secs.map(|s| s as f64 * 1000.0),
            estimated_cost_savings_usd_monthly: None,

            migration_sql: format!(
                "-- View blocking queries and what they're blocking\n\
                SELECT\n\
                    blocked_activity.pid AS blocked_pid,\n\
                    blocked_activity.query AS blocked_query,\n\
                    blocking_activity.pid AS blocking_pid,\n\
                    blocking_activity.query AS blocking_query,\n\
                    now() - blocked_activity.query_start AS wait_duration\n\
                FROM pg_catalog.pg_locks blocked_locks\n\
                JOIN pg_catalog.pg_stat_activity blocked_activity ON blocked_activity.pid = blocked_locks.pid\n\
                JOIN pg_catalog.pg_locks blocking_locks\n\
                    ON blocking_locks.locktype = blocked_locks.locktype\n\
                    AND blocking_locks.pid != blocked_locks.pid\n\
                JOIN pg_catalog.pg_stat_activity blocking_activity ON blocking_activity.pid = blocking_locks.pid\n\
                WHERE NOT blocked_locks.granted\n\
                ORDER BY wait_duration DESC;\n\n\
                -- To manually kill a blocking query (use with caution):\n\
                -- SELECT pg_terminate_backend(<blocking_pid>);"
            ),
            rollback_sql: None,
            safety_notes: format!(
                "Lock contention indicates queries waiting for data access. Investigate blocking queries using \
                pg_locks and pg_stat_activity. Consider:\n\
                1. Adding indexes to speed up locking queries\n\
                2. Reducing transaction scope\n\
                3. Using FOR UPDATE SKIP LOCKED for queue-like patterns\n\
                4. Partitioning large tables\n\
                Never kill blocking queries in production without understanding the impact."
            ),

            affected_table: None,
            affected_query: Some(format!("Example: {}", long_locked.first().map(|c| &c.query).unwrap_or(&String::from("N/A")))),
        })
    }
}

impl Default for ConnectionAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod connection_analyzer_tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_connection(
        pid: i32,
        state: ConnectionState,
        wait_event_type: WaitEventType,
        state_change_secs_ago: Option<i64>,
        query_start_secs_ago: Option<i64>,
    ) -> ConnectionInfo {
        let now = chrono::Utc::now();
        ConnectionInfo {
            pid,
            database: "testdb".to_string(),
            username: "testuser".to_string(),
            application_name: Some("testapp".to_string()),
            client_addr: Some("127.0.0.1".to_string()),
            state,
            query: "SELECT * FROM users".to_string(),
            query_start: query_start_secs_ago.map(|secs| now - chrono::Duration::seconds(secs)),
            state_change: state_change_secs_ago.map(|secs| now - chrono::Duration::seconds(secs)),
            wait_event_type,
            wait_event: None,
            backend_type: "client backend".to_string(),
        }
    }

    #[test]
    fn test_connection_pool_exhaustion_detection() {
        let analyzer = ConnectionAnalyzer::new();

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("active".to_string(), 92);

        let stats = ConnectionStats {
            total_connections: 92,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 0,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: None,
        };

        let result = analyzer.check_connection_pool_exhaustion(&stats);
        assert!(result.is_some());

        let recommendation = result.unwrap();
        assert_eq!(recommendation.issue_type, IssueType::ConnectionPoolExhaustion);
        assert_eq!(recommendation.severity, Severity::High);
        assert!(recommendation.title.contains("92"));
    }

    #[test]
    fn test_connection_pool_no_alert_when_usage_low() {
        let analyzer = ConnectionAnalyzer::new();

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("active".to_string(), 50);

        let stats = ConnectionStats {
            total_connections: 50,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 0,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: None,
        };

        let result = analyzer.check_connection_pool_exhaustion(&stats);
        assert!(result.is_none());
    }

    #[test]
    fn test_idle_in_transaction_detection() {
        let analyzer = ConnectionAnalyzer::new();

        let connections = vec![
            create_test_connection(1001, ConnectionState::IdleInTransaction, WaitEventType::None, Some(60), Some(60)),
            create_test_connection(1002, ConnectionState::IdleInTransaction, WaitEventType::None, Some(120), Some(120)),
            create_test_connection(1003, ConnectionState::Active, WaitEventType::None, Some(10), Some(10)),
        ];

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("idle in transaction".to_string(), 2);
        connections_by_state.insert("active".to_string(), 1);

        let stats = ConnectionStats {
            total_connections: 3,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 2,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: Some(120),
            oldest_locked_query_secs: None,
        };

        let result = analyzer.check_idle_in_transaction(&stats, &connections);
        assert!(result.is_some());

        let recommendation = result.unwrap();
        assert_eq!(recommendation.issue_type, IssueType::IdleInTransaction);
        assert!(recommendation.title.contains("2 idle in transaction"));
        assert!(recommendation.description.contains("120s"));
    }

    #[test]
    fn test_idle_in_transaction_no_alert_when_none() {
        let analyzer = ConnectionAnalyzer::new();

        let connections = vec![
            create_test_connection(1001, ConnectionState::Active, WaitEventType::None, Some(10), Some(10)),
            create_test_connection(1002, ConnectionState::Idle, WaitEventType::None, Some(5), Some(5)),
        ];

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("active".to_string(), 1);
        connections_by_state.insert("idle".to_string(), 1);

        let stats = ConnectionStats {
            total_connections: 2,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 0,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: None,
        };

        let result = analyzer.check_idle_in_transaction(&stats, &connections);
        assert!(result.is_none());
    }

    #[test]
    fn test_lock_contention_detection() {
        let analyzer = ConnectionAnalyzer::new();

        let connections = vec![
            create_test_connection(1001, ConnectionState::Active, WaitEventType::Lock, Some(30), Some(30)),
            create_test_connection(1002, ConnectionState::Active, WaitEventType::Lock, Some(45), Some(45)),
            create_test_connection(1003, ConnectionState::Active, WaitEventType::None, Some(10), Some(10)),
        ];

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("active".to_string(), 3);

        let stats = ConnectionStats {
            total_connections: 3,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 0,
            locked_connections_count: 2,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: Some(45),
        };

        let result = analyzer.check_lock_contention(&stats, &connections);
        assert!(result.is_some());

        let recommendation = result.unwrap();
        assert_eq!(recommendation.issue_type, IssueType::LockContention);
        assert!(recommendation.title.contains("2 connection"));
        assert!(recommendation.description.contains("45s"));
    }

    #[test]
    fn test_analyze_connections_multiple_issues() {
        let analyzer = ConnectionAnalyzer::new();

        let connections = vec![
            create_test_connection(1001, ConnectionState::IdleInTransaction, WaitEventType::None, Some(60), Some(60)),
            create_test_connection(1002, ConnectionState::Active, WaitEventType::Lock, Some(30), Some(30)),
        ];

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("idle in transaction".to_string(), 1);
        connections_by_state.insert("active".to_string(), 1);

        let stats = ConnectionStats {
            total_connections: 92,  // Changed to 92% to trigger pool exhaustion
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 1,
            locked_connections_count: 1,
            oldest_idle_transaction_secs: Some(60),
            oldest_locked_query_secs: Some(30),
        };

        let recommendations = analyzer.analyze_connections(&stats, &connections);

        // Should detect: pool exhaustion, idle in transaction, lock contention
        assert_eq!(recommendations.len(), 3);

        let issue_types: Vec<IssueType> = recommendations.iter()
            .map(|r| r.issue_type.clone())
            .collect();

        assert!(issue_types.contains(&IssueType::ConnectionPoolExhaustion));
        assert!(issue_types.contains(&IssueType::IdleInTransaction));
        assert!(issue_types.contains(&IssueType::LockContention));
    }

    #[test]
    fn test_severity_scaling_with_idle_transaction_count() {
        let analyzer = ConnectionAnalyzer::new();

        // Test with 3 idle transactions (should be Medium)
        let connections_medium: Vec<ConnectionInfo> = (0..3)
            .map(|i| create_test_connection(1000 + i, ConnectionState::IdleInTransaction, WaitEventType::None, Some(60), Some(60)))
            .collect();

        let mut connections_by_state = HashMap::new();
        connections_by_state.insert("idle in transaction".to_string(), 3);

        let stats_medium = ConnectionStats {
            total_connections: 3,
            max_connections: 100,
            connections_by_state: connections_by_state.clone(),
            idle_in_transaction_count: 3,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: Some(60),
            oldest_locked_query_secs: None,
        };

        let result_medium = analyzer.check_idle_in_transaction(&stats_medium, &connections_medium);
        assert_eq!(result_medium.unwrap().severity, Severity::Medium);

        // Test with 7 idle transactions (should be High)
        let connections_high: Vec<ConnectionInfo> = (0..7)
            .map(|i| create_test_connection(1000 + i, ConnectionState::IdleInTransaction, WaitEventType::None, Some(60), Some(60)))
            .collect();

        connections_by_state.insert("idle in transaction".to_string(), 7);

        let stats_high = ConnectionStats {
            total_connections: 7,
            max_connections: 100,
            connections_by_state: connections_by_state.clone(),
            idle_in_transaction_count: 7,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: Some(60),
            oldest_locked_query_secs: None,
        };

        let result_high = analyzer.check_idle_in_transaction(&stats_high, &connections_high);
        assert_eq!(result_high.unwrap().severity, Severity::High);

        // Test with 12 idle transactions (should be Critical)
        let connections_critical: Vec<ConnectionInfo> = (0..12)
            .map(|i| create_test_connection(1000 + i, ConnectionState::IdleInTransaction, WaitEventType::None, Some(60), Some(60)))
            .collect();

        connections_by_state.insert("idle in transaction".to_string(), 12);

        let stats_critical = ConnectionStats {
            total_connections: 12,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 12,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: Some(60),
            oldest_locked_query_secs: None,
        };

        let result_critical = analyzer.check_idle_in_transaction(&stats_critical, &connections_critical);
        assert_eq!(result_critical.unwrap().severity, Severity::Critical);
    }
}


/// Calculator for overall database health score (0-100)
pub struct HealthCalculator;

impl HealthCalculator {
    pub fn calculate(
        tables: &[TableInfo],
        indexes: &[IndexInfo],
        connections: Option<&ConnectionStats>,
        _db_size_mb: f64,
    ) -> u8 {
        let mut score = 100.0;
        
        // 1. Cache Hit Ratio (proxy via index usage vs seq scans)
        // Ideally we'd use pg_stat_database.blks_hit / (blks_hit + blks_read)
        // For now, we'll use index scan ratio as a proxy if available
        let total_scans: i64 = indexes.iter().map(|i| i.index_scans).sum();
        if total_scans == 0 && !indexes.is_empty() {
            score -= 10.0; // Suspiciously low index usage
        }

        // 2. Unused Indexes Penalty
        let unused_indexes = indexes.iter().filter(|i| i.index_scans == 0 && !i.is_primary && !i.is_unique).count();
        let total_indexes = indexes.len();
        if total_indexes > 0 {
            let unused_ratio = unused_indexes as f64 / total_indexes as f64;
            if unused_ratio > 0.5 {
                score -= 15.0 * unused_ratio;
            }
        }

        // 3. Connection Saturation
        if let Some(conn_stats) = connections {
            let utilization = conn_stats.total_connections as f64 / conn_stats.max_connections as f64;
            if utilization > 0.9 {
                score -= 30.0; // Critical saturation
            } else if utilization > 0.75 {
                score -= 15.0; // Warning saturation
            }

            // Idle in transaction penalty
            if conn_stats.idle_in_transaction_count > 0 {
                score -= (conn_stats.idle_in_transaction_count * 5) as f64;
            }
            
            // Locked connections penalty
            if conn_stats.locked_connections_count > 0 {
                score -= (conn_stats.locked_connections_count * 10) as f64;
            }
        }

        // 4. Bloat / Table Health (Proxy)
        // If we had pg_stat_user_tables.n_dead_tup, we'd use that.
        // For now, check if we have huge tables without indexes
        for table in tables {
            if table.table_size_mb > 1000.0 && table.index_size_mb < 10.0 {
                score -= 5.0; // Large table with minimal indexing
            }
        }

        // Clamp score between 0 and 100
        score.max(0.0).min(100.0) as u8
    }
}
