use sqlx::{PgPool, Row};
use crate::models::{QueryStats, IndexInfo, TableInfo};
use anyhow::Result;

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
            )"
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
                rows
            FROM pg_stat_statements
            WHERE query NOT LIKE '%pg_%'
              AND query NOT LIKE 'EXPLAIN%'
              AND query NOT LIKE 'SELECT pg_%'
              AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
            ORDER BY total_exec_time DESC
            LIMIT $1
            "#
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
        let result: (String, String) = sqlx::query_as(
            "SELECT current_database() as db_name, version() as pg_version"
        )
        .fetch_one(&self.pool)
        .await?;
        
        Ok(result)
    }
}
