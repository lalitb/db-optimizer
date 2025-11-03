use crate::models::*;
use sqlx::PgPool;
use anyhow::{Result, Context};
use tokio::time::{timeout, Duration};
use chrono::Utc;

pub struct Executor {
    pool: PgPool,
    config: AutoApplyConfig,
}

impl Executor {
    pub fn new(pool: PgPool, config: AutoApplyConfig) -> Self {
        Self { pool, config }
    }
    
    /// Execute a recommendation
    pub async fn execute(
        &self,
        rec: &ApplicableRecommendation,
    ) -> Result<ExecutionResult> {
        let started_at = Utc::now();
        
        // Dry-run mode
        if self.config.dry_run {
            return Ok(ExecutionResult {
                started_at,
                completed_at: Utc::now(),
                status: ExecutionStatus::Simulated,
                rows_affected: None,
                error_message: None,
            });
        }
        
        // Execute based on strategy
        let result = match &rec.apply_strategy {
            ApplyStrategy::CreateIndexConcurrently { .. } => {
                self.execute_create_index(rec).await
            }
            
            ApplyStrategy::DropIndex { .. } => {
                self.execute_drop_index(rec).await
            }
            
            ApplyStrategy::QueryRewrite { .. } => {
                // Cannot auto-execute query rewrites (requires code changes)
                Err(anyhow::anyhow!("Query rewrites require manual implementation"))
            }
            
            ApplyStrategy::ConfigurationChange { .. } => {
                self.execute_config_change(rec).await
            }
        };
        
        let completed_at = Utc::now();
        
        match result {
            Ok(rows_affected) => Ok(ExecutionResult {
                started_at,
                completed_at,
                status: ExecutionStatus::Success,
                rows_affected: Some(rows_affected),
                error_message: None,
            }),
            Err(e) => Ok(ExecutionResult {
                started_at,
                completed_at,
                status: ExecutionStatus::Failed,
                rows_affected: None,
                error_message: Some(e.to_string()),
            }),
        }
    }
    
    async fn execute_create_index(&self, rec: &ApplicableRecommendation) -> Result<u64> {
        let sql = &rec.recommendation.migration_sql;
        
        tracing::info!("Creating index: {}", sql);
        
        // Execute with timeout
        let timeout_duration = Duration::from_secs(self.config.execution_timeout_secs);
        
        let result = timeout(timeout_duration, async {
            sqlx::query(sql)
                .execute(&self.pool)
                .await
        })
        .await
        .context("Index creation timed out")?
        .context("Failed to create index")?;
        
        Ok(result.rows_affected())
    }
    
    async fn execute_drop_index(&self, rec: &ApplicableRecommendation) -> Result<u64> {
        let sql = &rec.recommendation.migration_sql;
        
        tracing::info!("Dropping index: {}", sql);
        
        let result = sqlx::query(sql)
            .execute(&self.pool)
            .await
            .context("Failed to drop index")?;
        
        Ok(result.rows_affected())
    }
    
    async fn execute_config_change(&self, rec: &ApplicableRecommendation) -> Result<u64> {
        // Extract parameter and value from strategy
        if let ApplyStrategy::ConfigurationChange { parameter, new_value, .. } = &rec.apply_strategy {
            let sql = format!("ALTER SYSTEM SET {} = '{}'", parameter, new_value);
            
            tracing::info!("Changing configuration: {}", sql);
            
            let result = sqlx::query(&sql)
                .execute(&self.pool)
                .await
                .context("Failed to change configuration")?;
            
            // Reload configuration
            sqlx::query("SELECT pg_reload_conf()")
                .execute(&self.pool)
                .await
                .context("Failed to reload configuration")?;
            
            Ok(result.rows_affected())
        } else {
            Err(anyhow::anyhow!("Invalid apply strategy"))
        }
    }
    
    /// Rollback a change
    pub async fn rollback(&self, rec: &ApplicableRecommendation) -> Result<()> {
        if let Some(rollback_sql) = &rec.recommendation.rollback_sql {
            tracing::warn!("Rolling back: {}", rollback_sql);
            
            sqlx::query(rollback_sql)
                .execute(&self.pool)
                .await
                .context("Failed to rollback")?;
            
            Ok(())
        } else {
            Err(anyhow::anyhow!("No rollback SQL available"))
        }
    }
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub started_at: chrono::DateTime<Utc>,
    pub completed_at: chrono::DateTime<Utc>,
    pub status: ExecutionStatus,
    pub rows_affected: Option<u64>,
    pub error_message: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ExecutionStatus {
    Simulated,
    Success,
    Failed,
}
