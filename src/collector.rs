use crate::models::{
    ConnectionInfo, ConnectionState, ConnectionStats, IndexInfo, QueryStats, TableInfo,
    WaitEventType,
};
use anyhow::Result;
use sqlx::{PgPool, Row};
use std::collections::HashMap;

pub struct DatabaseCollector {
    pool: PgPool,
}

impl DatabaseCollector {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Check if pg_stat_statements extension is enabled
    pub async fn check_pg_stat_statements(&self) -> Result<bool> {
        let result: (bool,) = sqlx::query_as(
            "SELECT EXISTS (
                SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements'
            )",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0)
    }

    /// Collect top queries by total execution time
    pub async fn collect_query_stats(&self, limit: i64) -> Result<Vec<QueryStats>> {
        let rows = sqlx::query(
            r#"
            SELECT 
                queryid,
                query,
                calls,
                total_exec_time,
                mean_exec_time,
                rows,
                shared_blks_read,
                shared_blks_hit
            FROM pg_stat_statements
            WHERE query NOT LIKE '%pg_%'
              AND query NOT LIKE 'EXPLAIN%'
              AND query NOT LIKE 'SELECT pg_%'
              AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
            ORDER BY total_exec_time DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let queries: Vec<QueryStats> = rows
            .into_iter()
            .map(|row| QueryStats {
                queryid: row.try_get("queryid").ok(),
                query: row.get("query"),
                calls: row.get("calls"),
                total_exec_time_ms: row.get("total_exec_time"),
                mean_exec_time_ms: row.get("mean_exec_time"),
                rows: row.get("rows"),
                shared_blks_read: row.get("shared_blks_read"),
                shared_blks_hit: row.get("shared_blks_hit"),
            })
            .collect();

        Ok(queries)
    }

    /// Collect index usage statistics
    pub async fn collect_index_info(&self) -> Result<Vec<IndexInfo>> {
        let rows = sqlx::query(
            r#"
            SELECT 
                psi.schemaname as schema_name,
                psi.relname as table_name,
                psi.indexrelname as index_name,
                CAST(pg_relation_size(psi.indexrelid) AS DOUBLE PRECISION) / (1024.0 * 1024.0) as index_size_mb,
                psi.idx_scan as index_scans,
                pi.indisunique as is_unique,
                pi.indisprimary as is_primary
            FROM pg_stat_user_indexes psi
            JOIN pg_index pi ON psi.indexrelid = pi.indexrelid
            WHERE psi.schemaname NOT IN ('pg_catalog', 'information_schema')
            ORDER BY psi.idx_scan ASC, pg_relation_size(psi.indexrelid) DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        let indexes: Vec<IndexInfo> = rows
            .into_iter()
            .map(|row| IndexInfo {
                schema_name: row.get("schema_name"),
                table_name: row.get("table_name"),
                index_name: row.get("index_name"),
                index_size_mb: row.get("index_size_mb"),
                index_scans: row.get("index_scans"),
                is_unique: row.get("is_unique"),
                is_primary: row.get("is_primary"),
            })
            .collect();

        Ok(indexes)
    }

    /// Collect table statistics
    pub async fn collect_table_info(&self) -> Result<Vec<TableInfo>> {
        let rows = sqlx::query(
            r#"
            SELECT 
                schemaname as schema_name,
                relname as table_name,
                n_live_tup as row_count,
                CAST(pg_total_relation_size(schemaname||'.'||relname) AS DOUBLE PRECISION) / (1024.0 * 1024.0) as total_size_mb,
                CAST(pg_relation_size(schemaname||'.'||relname) AS DOUBLE PRECISION) / (1024.0 * 1024.0) as table_size_mb,
                CAST((pg_total_relation_size(schemaname||'.'||relname) - pg_relation_size(schemaname||'.'||relname)) AS DOUBLE PRECISION) / (1024.0 * 1024.0) as index_size_mb
            FROM pg_stat_user_tables
            WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
            ORDER BY pg_total_relation_size(schemaname||'.'||relname) DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;

        let tables: Vec<TableInfo> = rows
            .into_iter()
            .map(|row| TableInfo {
                schema_name: row.get("schema_name"),
                table_name: row.get("table_name"),
                row_count: row.get("row_count"),
                total_size_mb: row.get("total_size_mb"),
                table_size_mb: row.get("table_size_mb"),
                index_size_mb: row.get("index_size_mb"),
            })
            .collect();

        Ok(tables)
    }

    /// Get database name and version
    pub async fn get_database_info(&self) -> Result<(String, String)> {
        let result: (String, String) =
            sqlx::query_as("SELECT current_database() as db_name, version() as pg_version")
                .fetch_one(&self.pool)
                .await?;

        Ok(result)
    }

    // ============================================================================
    // CONNECTION MONITORING METHODS
    // ============================================================================

    /// Collect all active connections from pg_stat_activity
    pub async fn collect_connections(&self) -> Result<Vec<ConnectionInfo>> {
        let rows = sqlx::query(
            r#"
            SELECT
                pid,
                datname as database,
                usename as username,
                application_name,
                client_addr::text,
                state,
                query,
                query_start,
                state_change,
                wait_event_type,
                wait_event,
                backend_type
            FROM pg_stat_activity
            WHERE pid != pg_backend_pid()  -- Exclude our own connection
              AND backend_type = 'client backend'
            ORDER BY state_change ASC  -- Oldest first
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let connections: Vec<ConnectionInfo> = rows
            .into_iter()
            .map(|row| {
                let state_str: String = row.get("state");
                let wait_event_type_opt: Option<String> = row.try_get("wait_event_type").ok();

                ConnectionInfo {
                    pid: row.get("pid"),
                    database: row.get("database"),
                    username: row.get("username"),
                    application_name: row.try_get("application_name").ok(),
                    client_addr: row.try_get("client_addr").ok(),
                    state: ConnectionState::from_str(&state_str),
                    query: row.get("query"),
                    query_start: row.try_get("query_start").ok(),
                    state_change: row.try_get("state_change").ok(),
                    wait_event_type: WaitEventType::from_str_opt(wait_event_type_opt.as_deref()),
                    wait_event: row.try_get("wait_event").ok(),
                    backend_type: row.get("backend_type"),
                }
            })
            .collect();

        Ok(connections)
    }

    /// Get aggregated connection statistics
    pub async fn collect_connection_stats(&self) -> Result<ConnectionStats> {
        // Get max_connections setting
        let max_connections: (i32,) =
            sqlx::query_as("SELECT setting::int FROM pg_settings WHERE name = 'max_connections'")
                .fetch_one(&self.pool)
                .await?;

        let connections = self.collect_connections().await?;
        let total_connections = connections.len() as i32;

        // Group by state
        let mut connections_by_state: HashMap<String, i32> = HashMap::new();
        let mut idle_in_transaction_count = 0;
        let mut locked_connections_count = 0;
        let mut oldest_idle_transaction_secs: Option<i64> = None;
        let mut oldest_locked_query_secs: Option<i64> = None;

        for conn in &connections {
            // Count by state
            let state_key = conn.state.as_str().to_string();
            *connections_by_state.entry(state_key).or_insert(0) += 1;

            // Track idle in transaction
            if conn.state.is_problematic() {
                idle_in_transaction_count += 1;
                if let Some(duration) = conn.state_duration_secs() {
                    oldest_idle_transaction_secs = Some(
                        oldest_idle_transaction_secs
                            .map(|old| old.max(duration))
                            .unwrap_or(duration),
                    );
                }
            }

            // Track locked queries
            if conn.is_locked() {
                locked_connections_count += 1;
                if let Some(duration) = conn.query_duration_secs() {
                    oldest_locked_query_secs = Some(
                        oldest_locked_query_secs
                            .map(|old| old.max(duration))
                            .unwrap_or(duration),
                    );
                }
            }
        }

        Ok(ConnectionStats {
            total_connections,
            max_connections: max_connections.0,
            connections_by_state,
            idle_in_transaction_count,
            locked_connections_count,
            oldest_idle_transaction_secs,
            oldest_locked_query_secs,
        })
    }

    /// Get connections that are idle in transaction for longer than threshold_secs
    pub async fn get_long_idle_transactions(
        &self,
        threshold_secs: i64,
    ) -> Result<Vec<ConnectionInfo>> {
        let connections = self.collect_connections().await?;

        let long_idle: Vec<ConnectionInfo> = connections
            .into_iter()
            .filter(|conn| {
                conn.state.is_problematic()
                    && conn
                        .state_duration_secs()
                        .map_or(false, |d| d > threshold_secs)
            })
            .collect();

        Ok(long_idle)
    }

    /// Get connections that are waiting for locks
    pub async fn get_locked_connections(&self) -> Result<Vec<ConnectionInfo>> {
        let connections = self.collect_connections().await?;

        let locked: Vec<ConnectionInfo> = connections
            .into_iter()
            .filter(|conn| conn.is_locked())
            .collect();

        Ok(locked)
    }

    /// Collect general database metrics (TPS, Cache Hit Ratio, etc.)
    pub async fn collect_database_metrics(&self) -> Result<crate::models::DatabaseMetrics> {
        let row = sqlx::query(
            r#"
            SELECT 
                sum(xact_commit + xact_rollback) as total_xact,
                sum(tup_fetched) as total_fetched,
                sum(tup_returned) as total_returned,
                sum(blks_hit) as total_blks_hit,
                sum(blks_read) as total_blks_read,
                sum(deadlocks) as total_deadlocks,
                sum(temp_files) as total_temp_files,
                sum(temp_bytes) as total_temp_bytes
            FROM pg_stat_database
            WHERE datname = current_database()
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        let total_xact: i64 = row.try_get("total_xact").unwrap_or(0);
        let total_fetched: i64 = row.try_get("total_fetched").unwrap_or(0);
        let total_returned: i64 = row.try_get("total_returned").unwrap_or(0);
        let blks_hit: i64 = row.try_get("total_blks_hit").unwrap_or(0);
        let blks_read: i64 = row.try_get("total_blks_read").unwrap_or(0);
        let deadlocks: i64 = row.try_get("total_deadlocks").unwrap_or(0);
        let temp_files: i64 = row.try_get("total_temp_files").unwrap_or(0);
        let temp_bytes: i64 = row.try_get("total_temp_bytes").unwrap_or(0);

        // Calculate Cache Hit Ratio
        let total_blks = blks_hit + blks_read;
        let buffer_cache_hit_ratio = if total_blks > 0 {
            (blks_hit as f64 / total_blks as f64) * 100.0
        } else {
            100.0
        };

        // Note: TPS and Rows/sec are cumulative since reset.
        // For real-time rates, we would need to sample twice.
        // For now, we return the raw counters or derived ratios where possible.
        // In a real agent, we'd diff these against the previous sample.

        Ok(crate::models::DatabaseMetrics {
            transactions_per_sec: total_xact as f64, // Placeholder: This is total, not per sec
            rows_fetched_per_sec: total_fetched as f64,
            rows_returned_per_sec: total_returned as f64,
            buffer_cache_hit_ratio,
            deadlocks,
            temp_files,
            temp_bytes,
        })
    }
}
