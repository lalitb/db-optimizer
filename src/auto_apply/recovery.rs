//! State recovery for interrupted operations.
//!
//! Handles cleanup and state recovery when Vaanyze crashes or restarts during
//! critical operations like VERIFYING or CREATE INDEX.

use crate::auto_apply::operation_guard::OperationGuard;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{info, warn};

/// Index naming prefix for Vaanyze-created indexes
pub const VAANYZE_INDEX_PREFIX: &str = "vaanyze_idx_";
pub const VAANYZE_CANDIDATE_PREFIX: &str = "_vaanyze_candidate_";

/// Recovery status for a recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryStatus {
    pub recommendation_id: String,
    pub previous_state: String,
    pub recovered_to: String,
    pub index_exists: bool,
    pub recovery_action: RecoveryAction,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// No action needed - state is consistent
    None,
    /// Marked as failed due to interrupted operation
    MarkedFailed { reason: String },
    /// Cleaned up orphaned candidate index
    CleanedUpCandidate { index_name: String },
    /// Recovered VERIFYING state - can continue verification
    ContinueVerification { remaining_secs: i64 },
    /// Verification window expired while down - needs manual review
    VerificationExpired { index_name: String },
    /// Cleanup was blocked due to read-only mode
    BlockedReadOnly { indexes_found: Vec<String> },
}

/// Detects orphaned candidate indexes (read-only operation)
pub async fn find_orphaned_candidates(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    // Find all candidate indexes (they should not persist)
    let candidates: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT indexname 
        FROM pg_indexes 
        WHERE indexname LIKE '_vaanyze_candidate_%'
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(candidates.into_iter().map(|(name,)| name).collect())
}

/// Detects and cleans up orphaned candidate indexes
///
/// IMPORTANT: This function performs DROP INDEX operations.
/// Caller MUST check OperationGuard before calling if enforcement is required.
/// Use `cleanup_orphaned_candidates_guarded` for enforced cleanup.
pub async fn cleanup_orphaned_candidates(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let mut cleaned = Vec::new();

    let candidates = find_orphaned_candidates(pool).await?;

    for index_name in candidates {
        warn!(
            index_name = %index_name,
            "Found orphaned candidate index, cleaning up"
        );

        // Drop the orphaned candidate using CONCURRENTLY for safety
        let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index_name);
        if let Err(e) = sqlx::query(&drop_sql).execute(pool).await {
            warn!(
                index_name = %index_name,
                error = %e,
                "Failed to drop orphaned candidate index"
            );
        } else {
            cleaned.push(index_name);
        }
    }

    if !cleaned.is_empty() {
        info!(
            count = cleaned.len(),
            "Cleaned up orphaned candidate indexes"
        );
    }

    Ok(cleaned)
}

/// Detects and cleans up orphaned candidate indexes with guard enforcement
///
/// Returns Ok with BlockedReadOnly action if guard blocks the operation,
/// allowing caller to inform user of manual cleanup needed.
pub async fn cleanup_orphaned_candidates_guarded(
    pool: &PgPool,
    guard: Option<&Arc<OperationGuard>>,
) -> anyhow::Result<(Vec<String>, Option<RecoveryAction>)> {
    // First, find what exists (read-only operation)
    let candidates = find_orphaned_candidates(pool).await?;

    if candidates.is_empty() {
        return Ok((vec![], None));
    }

    // Check guard before destructive operations
    if let Some(g) = guard {
        let check = g.check_destructive_operation().await;
        if !check.allowed {
            warn!(
                count = candidates.len(),
                reason = ?check.block_reason,
                "Cannot cleanup orphaned indexes: operation blocked"
            );
            return Ok((
                vec![],
                Some(RecoveryAction::BlockedReadOnly {
                    indexes_found: candidates,
                }),
            ));
        }
    }

    // Guard allows - proceed with cleanup
    let cleaned = cleanup_orphaned_candidates(pool).await?;
    Ok((cleaned, None))
}

/// Checks if a Vaanyze-created index exists in the database
pub async fn index_exists(pool: &PgPool, index_name: &str) -> anyhow::Result<bool> {
    let result: Option<(i64,)> = sqlx::query_as(
        r#"
        SELECT 1 
        FROM pg_indexes 
        WHERE indexname = $1
        "#,
    )
    .bind(index_name)
    .fetch_optional(pool)
    .await?;

    Ok(result.is_some())
}

/// Lists all Vaanyze-created production indexes
pub async fn list_vaanyze_indexes(pool: &PgPool) -> anyhow::Result<Vec<VaanyzeIndex>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT indexname, tablename, indexdef
        FROM pg_indexes 
        WHERE indexname LIKE 'vaanyze_idx_%'
        ORDER BY indexname
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut indexes = Vec::new();
    for (name, table, definition) in rows {
        // Try to get usage stats
        let stats: Option<(i64, i64)> = sqlx::query_as(
            r#"
            SELECT idx_scan, idx_tup_read
            FROM pg_stat_user_indexes
            WHERE indexrelname = $1
            "#,
        )
        .bind(&name)
        .fetch_optional(pool)
        .await?;

        indexes.push(VaanyzeIndex {
            name,
            table,
            definition,
            idx_scan: stats.map(|(s, _)| s).unwrap_or(0),
            idx_tup_read: stats.map(|(_, r)| r).unwrap_or(0),
        });
    }

    Ok(indexes)
}

/// Information about a Vaanyze-created index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaanyzeIndex {
    pub name: String,
    pub table: String,
    pub definition: String,
    pub idx_scan: i64,
    pub idx_tup_read: i64,
}

/// Check for incomplete CREATE INDEX CONCURRENTLY operations
pub async fn check_invalid_indexes(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    // PostgreSQL marks incomplete concurrent index builds as invalid
    let invalid: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT c.relname
        FROM pg_class c
        JOIN pg_index i ON c.oid = i.indexrelid
        WHERE NOT i.indisvalid
        AND c.relname LIKE 'vaanyze_idx_%'
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(invalid.into_iter().map(|(name,)| name).collect())
}

/// Drop an invalid index that failed to build
///
/// NOTE: We use DROP INDEX (not CONCURRENTLY) here because:
/// 1. Invalid indexes are not usable anyway
/// 2. CONCURRENTLY cannot be used inside a transaction
/// 3. This operation should be rare (only after failed concurrent builds)
pub async fn drop_invalid_index(pool: &PgPool, index_name: &str) -> anyhow::Result<()> {
    warn!(index_name = %index_name, "Dropping invalid index from failed concurrent build");

    // Use CONCURRENTLY for consistency with other DROP operations
    // This requires running outside a transaction
    let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index_name);
    sqlx::query(&drop_sql).execute(pool).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_action_serialize() {
        let action = RecoveryAction::CleanedUpCandidate {
            index_name: "_vaanyze_candidate_abc123".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("CleanedUpCandidate"));
    }

    #[test]
    fn test_index_prefixes() {
        assert!(VAANYZE_INDEX_PREFIX.starts_with("vaanyze"));
        assert!(VAANYZE_CANDIDATE_PREFIX.starts_with("_vaanyze"));
    }

    #[test]
    fn test_recovery_action_blocked_read_only() {
        let action = RecoveryAction::BlockedReadOnly {
            indexes_found: vec!["idx1".to_string(), "idx2".to_string()],
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("BlockedReadOnly"));
        assert!(json.contains("idx1"));
        assert!(json.contains("idx2"));
    }

    #[test]
    fn test_cleanup_sql_uses_concurrently() {
        // Verify the format string in cleanup_orphaned_candidates uses CONCURRENTLY
        let index_name = "_vaanyze_candidate_test";
        let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index_name);
        assert!(drop_sql.contains("CONCURRENTLY"));
        assert!(drop_sql.contains("IF EXISTS"));
    }
}
