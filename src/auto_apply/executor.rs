use crate::auto_apply::operation_guard::OperationGuard;
use crate::models::*;
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

/// Error type for executor operations
#[derive(Debug)]
pub enum ExecutorError {
    /// Operation blocked by safety guard
    OperationBlocked(String),
    /// Execution failed
    ExecutionFailed(String),
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OperationBlocked(reason) => write!(f, "Operation blocked: {}", reason),
            Self::ExecutionFailed(msg) => write!(f, "Execution failed: {}", msg),
        }
    }
}

impl std::error::Error for ExecutorError {}

pub struct Executor {
    pool: PgPool,
    config: AutoApplyConfig,
    /// Optional operation guard for safety enforcement
    guard: Option<Arc<OperationGuard>>,
}

impl Executor {
    pub fn new(pool: PgPool, config: AutoApplyConfig) -> Self {
        Self {
            pool,
            config,
            guard: None,
        }
    }

    /// Create executor with operation guard for safety enforcement
    pub fn with_guard(pool: PgPool, config: AutoApplyConfig, guard: Arc<OperationGuard>) -> Self {
        Self {
            pool,
            config,
            guard: Some(guard),
        }
    }

    /// Check if destructive operation is allowed
    async fn check_guard(&self) -> Result<(), ExecutorError> {
        if let Some(guard) = &self.guard {
            let check = guard.check_destructive_operation().await;
            if !check.allowed {
                let reason = check.block_reason
                    .map(|r| format!("{:?}", r))
                    .unwrap_or_else(|| "Operation not allowed in current mode".to_string());
                return Err(ExecutorError::OperationBlocked(reason));
            }
        }
        Ok(())
    }

    /// Execute a recommendation
    pub async fn execute(&self, rec: &ApplicableRecommendation) -> Result<ExecutionResult> {
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

        // ENFORCE: Check guard before any destructive operation
        if let Err(e) = self.check_guard().await {
            return Ok(ExecutionResult {
                started_at,
                completed_at: Utc::now(),
                status: ExecutionStatus::Blocked,
                rows_affected: None,
                error_message: Some(e.to_string()),
            });
        }

        // Execute based on strategy
        let result = match &rec.apply_strategy {
            ApplyStrategy::CreateIndexConcurrently { .. } => self.execute_create_index(rec).await,

            ApplyStrategy::DropIndex { .. } => self.execute_drop_index(rec).await,

            ApplyStrategy::QueryRewrite { .. } => {
                // Cannot auto-execute query rewrites (requires code changes)
                Err(anyhow::anyhow!(
                    "Query rewrites require manual implementation"
                ))
            }

            ApplyStrategy::ConfigurationChange { .. } => self.execute_config_change(rec).await,
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
            sqlx::query(sql).execute(&self.pool).await
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
        if let ApplyStrategy::ConfigurationChange {
            parameter,
            new_value,
            ..
        } = &rec.apply_strategy
        {
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
        // ENFORCE: Check guard before rollback (which can be DROP INDEX)
        if let Err(e) = self.check_guard().await {
            return Err(anyhow::anyhow!(e.to_string()));
        }

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
    /// Operation was blocked by safety guard
    Blocked,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_apply::BlockReason;
    use crate::analyzer::environment::{DatabaseEnvironment, EnvironmentInfo};

    fn create_guard_with_kill_switch() -> Arc<OperationGuard> {
        Arc::new(OperationGuard::with_settings("safe_auto", true))
    }

    #[tokio::test]
    async fn test_guard_blocks_with_kill_switch() {
        // Create a guard with kill switch active
        let guard = create_guard_with_kill_switch();
        
        // Verify the guard is blocking
        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::KillSwitchActive));
    }

    #[tokio::test]
    async fn test_guard_blocks_in_read_only_mode() {
        // Create a guard in read-only mode
        let guard = Arc::new(OperationGuard::with_settings("observe_only", false));
        
        // Set a permissive environment (so only the mode blocks)
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::SelfManaged,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: true,
            is_db_owner: true,
            warnings: vec![],
            recommended_mode: "safe_auto".to_string(),
        };
        guard.set_environment(env).await;
        
        // Verify the guard is blocking due to read-only mode
        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::ReadOnlyMode));
    }

    #[tokio::test]
    async fn test_guard_blocks_in_managed_environment() {
        // Create a guard that would allow operations by config
        let guard = Arc::new(OperationGuard::with_settings("safe_auto", false));
        
        // But set an RDS environment which should force read-only
        let env = EnvironmentInfo {
            environment: DatabaseEnvironment::AwsRds,
            pg_version: "16.0".to_string(),
            has_pg_stat_statements: true,
            can_create_index: true,
            can_drop_index: true,
            is_superuser: false,
            is_db_owner: false,
            warnings: vec![],
            recommended_mode: "observe_only".to_string(),
        };
        guard.set_environment(env).await;
        
        // Verify the guard is blocking due to managed environment
        let check = guard.check_destructive_operation().await;
        assert!(!check.allowed);
        assert_eq!(check.block_reason, Some(BlockReason::ManagedEnvironment));
    }

    #[tokio::test]
    async fn test_executor_error_display() {
        let blocked_err = ExecutorError::OperationBlocked("Kill switch active".to_string());
        assert_eq!(
            blocked_err.to_string(),
            "Operation blocked: Kill switch active"
        );
        
        let failed_err = ExecutorError::ExecutionFailed("Connection lost".to_string());
        assert_eq!(
            failed_err.to_string(),
            "Execution failed: Connection lost"
        );
    }

    #[test]
    fn test_execution_status_blocked_variant() {
        // Ensure Blocked variant exists and is distinct
        let blocked = ExecutionStatus::Blocked;
        assert_eq!(blocked, ExecutionStatus::Blocked);
        assert_ne!(blocked, ExecutionStatus::Failed);
        assert_ne!(blocked, ExecutionStatus::Success);
        assert_ne!(blocked, ExecutionStatus::Simulated);
    }
}
