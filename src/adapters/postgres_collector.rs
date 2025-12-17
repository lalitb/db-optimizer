// src/adapters/postgres_collector.rs - PostgreSQL implementation of MetricsCollector trait

use anyhow::Result;
use async_trait::async_trait;

use crate::collector::DatabaseCollector;
use crate::models::{IndexInfo, QueryStats, TableInfo};
use crate::traits::MetricsCollector;

/// PostgreSQL implementation of the MetricsCollector trait
pub struct PostgresMetricsCollector {
    collector: DatabaseCollector,
}

impl PostgresMetricsCollector {
    /// Create a new PostgresMetricsCollector
    pub fn new(collector: DatabaseCollector) -> Self {
        Self { collector }
    }

    /// Create from database URL
    pub async fn from_url(database_url: &str) -> Result<Self> {
        use sqlx::postgres::PgPoolOptions;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        let collector = DatabaseCollector::new(pool);
        Ok(Self::new(collector))
    }
}

#[async_trait]
impl MetricsCollector for PostgresMetricsCollector {
    async fn collect_query_stats(&self, limit: i64) -> Result<Vec<QueryStats>> {
        self.collector.collect_query_stats(limit).await
    }

    async fn collect_index_info(&self) -> Result<Vec<IndexInfo>> {
        self.collector.collect_index_info().await
    }

    async fn collect_table_info(&self) -> Result<Vec<TableInfo>> {
        self.collector.collect_table_info().await
    }

    async fn get_database_name(&self) -> Result<String> {
        let (db_name, _) = self.collector.get_database_info().await?;
        Ok(db_name)
    }

    async fn get_database_version(&self) -> Result<String> {
        let (_, version) = self.collector.get_database_info().await?;
        Ok(version)
    }

    async fn test_connection(&self) -> Result<()> {
        // Test by trying to get database info
        self.collector.get_database_info().await?;
        Ok(())
    }
}
