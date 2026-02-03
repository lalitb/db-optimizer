//! Index proof generation - validates that a recommended index actually improves query performance.
//!
//! This module:
//! - Creates candidate indexes with deterministic names
//! - Runs EXPLAIN before/after comparison
//! - Generates proof of improvement (or failure)
//! - Cleans up temporary indexes on validation failure
//!
//! **Safety Enforcement:**
//! - When OperationGuard is provided, all CREATE/DROP INDEX operations are gated
//! - Under read-only mode, validation runs in EXPLAIN-only mode (no candidate indexes)
//! - Kill switch blocks all validation that would create indexes

use super::explain::ExplainExecutor;
use super::models::{PlanDiff, ProofMode, ProofStrength, ValidationReasonCode, ValidationResult, ValidationStatus};
use crate::auto_apply::{BlockReason, OperationGuard};
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default minimum cost improvement to consider validation successful
const DEFAULT_MIN_IMPROVEMENT_PCT: f64 = 5.0;

/// Default timeout for index creation (5 minutes)
const DEFAULT_CREATE_INDEX_TIMEOUT_SECS: u64 = 300;

/// Prefix for validation candidate indexes
const CANDIDATE_INDEX_PREFIX: &str = "_vaanyze_candidate_";

/// Result of checking if destructive operations are allowed
#[derive(Debug, Clone)]
pub enum DestructiveCheckResult {
    /// Destructive operations are allowed
    Allowed,
    /// Kill switch is active - block everything including EXPLAIN
    KillSwitchActive,
    /// Read-only mode - allow EXPLAIN-only validation
    ReadOnly(ValidationReasonCode),
}

/// Generates and validates index proofs
pub struct ProofGenerator {
    pool: PgPool,
    explain_executor: ExplainExecutor,
    min_improvement_pct: f64,
    create_index_timeout: Duration,
    /// Optional operation guard for enforcing read-only mode
    operation_guard: Option<Arc<OperationGuard>>,
}

impl ProofGenerator {
    /// Create a new proof generator (no safety guard - for backward compatibility)
    pub fn new(pool: PgPool) -> Self {
        let explain_executor = ExplainExecutor::new(pool.clone());
        Self {
            pool,
            explain_executor,
            min_improvement_pct: DEFAULT_MIN_IMPROVEMENT_PCT,
            create_index_timeout: Duration::from_secs(DEFAULT_CREATE_INDEX_TIMEOUT_SECS),
            operation_guard: None,
        }
    }

    /// Create with custom settings (no safety guard)
    pub fn with_settings(
        pool: PgPool,
        min_improvement_pct: f64,
        create_index_timeout: Duration,
    ) -> Self {
        let explain_executor = ExplainExecutor::new(pool.clone());
        Self {
            pool,
            explain_executor,
            min_improvement_pct,
            create_index_timeout,
            operation_guard: None,
        }
    }

    /// Create with operation guard for safety enforcement
    pub fn with_guard(pool: PgPool, guard: Arc<OperationGuard>) -> Self {
        let explain_executor = ExplainExecutor::new(pool.clone());
        Self {
            pool,
            explain_executor,
            min_improvement_pct: DEFAULT_MIN_IMPROVEMENT_PCT,
            create_index_timeout: Duration::from_secs(DEFAULT_CREATE_INDEX_TIMEOUT_SECS),
            operation_guard: Some(guard),
        }
    }

    /// Create with custom settings and operation guard
    pub fn with_settings_and_guard(
        pool: PgPool,
        min_improvement_pct: f64,
        create_index_timeout: Duration,
        guard: Arc<OperationGuard>,
    ) -> Self {
        let explain_executor = ExplainExecutor::new(pool.clone());
        Self {
            pool,
            explain_executor,
            min_improvement_pct,
            create_index_timeout,
            operation_guard: Some(guard),
        }
    }

    /// Check if destructive operations (CREATE/DROP INDEX) are allowed.
    /// Returns detailed result indicating what mode should be used.
    pub async fn check_destructive_allowed(&self) -> DestructiveCheckResult {
        if let Some(guard) = &self.operation_guard {
            let check = guard.check_destructive_operation().await;
            if !check.allowed {
                return match check.block_reason {
                    Some(BlockReason::KillSwitchActive) => DestructiveCheckResult::KillSwitchActive,
                    Some(BlockReason::ReadOnlyMode) => {
                        DestructiveCheckResult::ReadOnly(ValidationReasonCode::ReadOnlyMode)
                    }
                    Some(BlockReason::ManagedEnvironment) => {
                        DestructiveCheckResult::ReadOnly(ValidationReasonCode::ReadOnlyMode)
                    }
                    Some(BlockReason::MissingPermission) => {
                        DestructiveCheckResult::ReadOnly(ValidationReasonCode::PermissionDenied)
                    }
                    Some(BlockReason::EnvironmentUnknown) => {
                        DestructiveCheckResult::ReadOnly(ValidationReasonCode::ReadOnlyMode)
                    }
                    None => DestructiveCheckResult::ReadOnly(ValidationReasonCode::ReadOnlyMode),
                };
            }
        }
        DestructiveCheckResult::Allowed
    }

    /// Check if kill switch is specifically active
    pub async fn is_kill_switch_active(&self) -> bool {
        if let Some(guard) = &self.operation_guard {
            guard.is_kill_switch_active()
        } else {
            false
        }
    }

    /// Validate an index recommendation.
    /// 
    /// Behavior depends on OperationGuard state:
    /// - **Destructive allowed**: Full validation with candidate index (CREATE/DROP INDEX)
    /// - **Read-only mode**: EXPLAIN-only validation (no DDL, weaker proof)
    /// - **Kill switch active**: Returns error immediately (no queries executed)
    ///
    /// Returns a ValidationResult with:
    /// - VALIDATED if index is used and cost improves >= min threshold (CandidateIndex mode only)
    /// - SKIPPED with EXPLAIN-only data if in read-only mode
    /// - FAILED with reason code otherwise
    pub async fn validate_index(
        &self,
        recommendation_id: &str,
        query: &str,
        table_name: &str,
        columns: &[String],
        index_type: Option<&str>,
    ) -> Result<ValidationResult> {
        // Generate deterministic candidate index name
        let candidate_index_name =
            self.generate_candidate_index_name(table_name, columns);

        // STEP 0: Check operation guard FIRST to determine proof mode
        let proof_mode = match self.check_destructive_allowed().await {
            DestructiveCheckResult::Allowed => ProofMode::CandidateIndex,
            DestructiveCheckResult::KillSwitchActive => {
                // Kill switch blocks EVERYTHING - don't even run EXPLAIN
                warn!(
                    recommendation_id = %recommendation_id,
                    "Validation blocked: kill switch active - no queries will be executed"
                );
                let now = chrono::Utc::now();
                return Ok(ValidationResult {
                    recommendation_id: recommendation_id.to_string(),
                    query_id: None,
                    query_text: query.to_string(),
                    index_name: candidate_index_name,
                    table_name: table_name.to_string(),
                    columns: columns.to_vec(),
                    before_plan: None,
                    after_plan: None,
                    diff_summary: None,
                    status: ValidationStatus::Skipped,
                    reason_code: ValidationReasonCode::KillSwitchActive,
                    reason_message: "Kill switch is active. No queries executed.".to_string(),
                    started_at: now,
                    completed_at: Some(now),
                    duration_ms: Some(0),
                    proof_mode: ProofMode::ExplainOnly,
                    proof_strength: ProofStrength::NoProof,
                    guidance: Some("Kill switch is active. Disable it to run validation.".to_string()),
                });
            }
            DestructiveCheckResult::ReadOnly(reason) => {
                info!(
                    recommendation_id = %recommendation_id,
                    reason = %reason.as_str(),
                    "Validation in EXPLAIN-only mode: destructive operations blocked"
                );
                ProofMode::ExplainOnly
            }
        };

        // Create pending result based on mode
        let result = if proof_mode == ProofMode::ExplainOnly {
            ValidationResult::pending_explain_only(
                recommendation_id.to_string(),
                query.to_string(),
                candidate_index_name.clone(),
                table_name.to_string(),
                columns.to_vec(),
            )
        } else {
            ValidationResult::pending(
                recommendation_id.to_string(),
                query.to_string(),
                candidate_index_name.clone(),
                table_name.to_string(),
                columns.to_vec(),
            )
        };

        // Check if query can be explained
        if !ExplainExecutor::can_explain(query) {
            return Ok(result.failed(
                ValidationReasonCode::QueryNotExplainable,
                format!("Query cannot be explained: {}", &query[..query.len().min(50)]),
            ));
        }

        // Step 1: Get EXPLAIN plan BEFORE index (always allowed - read-only operation)
        info!(
            "Validating index on {}.({}) for recommendation {}",
            table_name,
            columns.join(", "),
            recommendation_id
        );

        let before_plan = match self.explain_executor.explain(query).await {
            Ok(plan) => plan,
            Err(e) => {
                let reason = if e.to_string().contains("permission denied") {
                    ValidationReasonCode::PermissionDenied
                } else if e.to_string().contains("timeout") || e.to_string().contains("cancel") {
                    ValidationReasonCode::Timeout
                } else if e.to_string().contains("does not exist") {
                    ValidationReasonCode::InvalidQuery
                } else {
                    ValidationReasonCode::Error(e.to_string())
                };
                return Ok(result.failed(reason, format!("Failed to get before plan: {}", e)));
            }
        };

        debug!(
            "Before plan - cost: {:.2}, rows: {:.0}",
            before_plan.total_cost, before_plan.estimated_rows
        );

        // EXPLAIN-ONLY MODE: If we're in read-only, return with before plan only
        if proof_mode == ProofMode::ExplainOnly {
            info!(
                recommendation_id = %recommendation_id,
                "Returning EXPLAIN-only result: no candidate index created"
            );
            
            let guidance = format!(
                "System is in read-only mode. Cannot create candidate index to prove effectiveness. \
                Current query cost: {:.2}. To get full proof, enable safe_auto mode or disable kill switch. \
                Proposed index: {} ON {} ({})",
                before_plan.total_cost,
                candidate_index_name,
                table_name,
                columns.join(", ")
            );
            
            return Ok(result.explain_only(before_plan, guidance));
        }

        // CANDIDATE INDEX MODE: Full validation with DDL
        // Step 2: Create candidate index (GUARDED - destructive operation)
        let create_result = self
            .create_candidate_index(&candidate_index_name, table_name, columns, index_type)
            .await;

        if let Err(e) = create_result {
            let reason = if e.to_string().contains("permission denied") {
                ValidationReasonCode::PermissionDenied
            } else if e.to_string().contains("timeout") || e.to_string().contains("cancel") {
                ValidationReasonCode::Timeout
            } else {
                ValidationReasonCode::Error(e.to_string())
            };
            return Ok(result.failed(reason, format!("Failed to create candidate index: {}", e)));
        }

        // Step 3: Get EXPLAIN plan AFTER index (in a closure to ensure cleanup)
        let after_plan_result = self.explain_executor.explain(query).await;

        // Step 4: Analyze results and decide
        let validation_result = match after_plan_result {
            Ok(after_plan) => {
                debug!(
                    "After plan - cost: {:.2}, rows: {:.0}",
                    after_plan.total_cost, after_plan.estimated_rows
                );

                // Generate diff
                let diff =
                    PlanDiff::from_plans(&before_plan, &after_plan, table_name, &candidate_index_name);

                info!(
                    "Validation diff: cost {:.2} → {:.2} ({:.1}% change), index_used: {}",
                    diff.before_cost,
                    diff.after_cost,
                    diff.cost_improvement_pct,
                    diff.index_used
                );

                if diff.is_meaningful_improvement(self.min_improvement_pct) {
                    // Success! Index is used and improves cost
                    result.validated(before_plan, after_plan, diff)
                } else if !diff.index_used {
                    result.failed(
                        ValidationReasonCode::NotUsed,
                        format!(
                            "Index '{}' was not used by the query planner. {}",
                            candidate_index_name, diff.summary
                        ),
                    )
                } else {
                    result.failed(
                        ValidationReasonCode::NoCostImprovement,
                        format!(
                            "Cost improvement ({:.1}%) below threshold ({:.1}%). {}",
                            diff.cost_improvement_pct, self.min_improvement_pct, diff.summary
                        ),
                    )
                }
            }
            Err(e) => result.failed(
                ValidationReasonCode::Error(e.to_string()),
                format!("Failed to get after plan: {}", e),
            ),
        };

        // Step 5: Cleanup - always drop candidate index after validation
        // (Real index will be created by apply step if validation passes)
        if let Err(e) = self.drop_candidate_index(&candidate_index_name).await {
            warn!(
                "Failed to drop candidate index '{}': {}. Manual cleanup may be required.",
                candidate_index_name, e
            );
        }

        Ok(validation_result)
    }

    /// Generate a deterministic, unique index name for validation
    ///
    /// Format: _vaanyze_candidate_<hash>
    /// The hash is based on table name and columns for idempotency
    pub fn generate_candidate_index_name(&self, table_name: &str, columns: &[String]) -> String {
        let mut hasher = DefaultHasher::new();
        table_name.hash(&mut hasher);
        for col in columns {
            col.hash(&mut hasher);
        }
        let hash = hasher.finish();
        format!("{}{:016x}", CANDIDATE_INDEX_PREFIX, hash)
    }

    /// Generate a deterministic production index name
    ///
    /// Format: idx_<table>_<columns>_<short_hash>
    pub fn generate_production_index_name(&self, table_name: &str, columns: &[String]) -> String {
        let mut hasher = DefaultHasher::new();
        table_name.hash(&mut hasher);
        for col in columns {
            col.hash(&mut hasher);
        }
        let hash = hasher.finish();
        let short_hash = format!("{:08x}", hash as u32);

        // Truncate table/columns if too long (PostgreSQL limit is 63 chars)
        let table_part = if table_name.len() > 20 {
            &table_name[..20]
        } else {
            table_name
        };

        let cols_part: String = columns
            .iter()
            .map(|c| if c.len() > 10 { &c[..10] } else { c })
            .collect::<Vec<_>>()
            .join("_");

        let cols_part = if cols_part.len() > 20 {
            &cols_part[..20]
        } else {
            &cols_part
        };

        format!("idx_{}_{}_{}",  table_part, cols_part, short_hash)
    }

    /// Create a candidate index for validation
    /// 
    /// IMPORTANT: Caller MUST verify proof_mode == CandidateIndex before calling.
    /// This method assumes the guard check has already been done.
    async fn create_candidate_index(
        &self,
        index_name: &str,
        table_name: &str,
        columns: &[String],
        index_type: Option<&str>,
    ) -> Result<()> {
        // Double-check guard (defense in depth)
        match self.check_destructive_allowed().await {
            DestructiveCheckResult::Allowed => {}
            other => {
                anyhow::bail!(
                    "Cannot create candidate index: operation blocked ({:?})",
                    other
                );
            }
        }

        // Set statement timeout
        let timeout_ms = self.create_index_timeout.as_millis() as i64;
        sqlx::query(&format!("SET LOCAL statement_timeout = {}", timeout_ms))
            .execute(&self.pool)
            .await
            .context("Failed to set statement timeout")?;

        // Build CREATE INDEX statement
        let index_type_clause = index_type
            .map(|t| format!(" USING {}", t))
            .unwrap_or_default();

        let columns_clause = columns.join(", ");

        // Note: We use CONCURRENTLY for candidate indexes too, to avoid blocking
        // This is slower but safer in production environments
        let create_sql = format!(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS {} ON {}{} ({})",
            index_name, table_name, index_type_clause, columns_clause
        );

        debug!("Creating candidate index: {}", create_sql);

        sqlx::query(&create_sql)
            .execute(&self.pool)
            .await
            .context("Failed to create candidate index")?;

        info!("Created candidate index: {}", index_name);
        Ok(())
    }

    /// Drop a candidate index after validation
    /// 
    /// IMPORTANT: Caller MUST verify proof_mode == CandidateIndex before calling.
    /// This method assumes the guard check has already been done.
    async fn drop_candidate_index(&self, index_name: &str) -> Result<()> {
        // Only drop indexes with our prefix (safety check)
        if !index_name.starts_with(CANDIDATE_INDEX_PREFIX) {
            anyhow::bail!(
                "Refusing to drop index '{}': not a candidate index",
                index_name
            );
        }

        // Double-check guard (defense in depth)
        match self.check_destructive_allowed().await {
            DestructiveCheckResult::Allowed => {}
            other => {
                anyhow::bail!(
                    "Cannot drop candidate index: operation blocked ({:?})",
                    other
                );
            }
        }

        let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index_name);
        debug!("Dropping candidate index: {}", drop_sql);

        sqlx::query(&drop_sql)
            .execute(&self.pool)
            .await
            .context("Failed to drop candidate index")?;

        info!("Dropped candidate index: {}", index_name);
        Ok(())
    }

    /// Clean up any leftover candidate indexes (maintenance)
    ///
    /// **Note:** This requires destructive operations to be allowed.
    /// Returns empty list if blocked by read-only mode.
    pub async fn cleanup_candidate_indexes(&self) -> Result<Vec<String>> {
        // SAFETY CHECK: Cleanup requires DROP operations
        match self.check_destructive_allowed().await {
            DestructiveCheckResult::Allowed => {}
            other => {
                warn!(
                    reason = ?other,
                    "Cannot cleanup candidate indexes: operation blocked"
                );
                return Ok(Vec::new());
            }
        }

        let rows = sqlx::query(
            r#"
            SELECT indexname
            FROM pg_indexes
            WHERE indexname LIKE $1
            "#,
        )
        .bind(format!("{}%", CANDIDATE_INDEX_PREFIX))
        .fetch_all(&self.pool)
        .await?;

        let mut dropped = Vec::new();
        for row in rows {
            let index_name: String = row.try_get("indexname")?;
            if let Err(e) = self.drop_candidate_index(&index_name).await {
                warn!("Failed to cleanup candidate index {}: {}", index_name, e);
            } else {
                dropped.push(index_name);
            }
        }

        if !dropped.is_empty() {
            info!("Cleaned up {} candidate indexes", dropped.len());
        }

        Ok(dropped)
    }
}

/// Result of index proof generation
#[derive(Debug)]
pub struct IndexProof {
    /// The validation result
    pub validation: ValidationResult,

    /// SQL to create the production index (if validated)
    pub create_index_sql: Option<String>,

    /// SQL to drop the index (for rollback)
    pub drop_index_sql: Option<String>,
}

impl IndexProof {
    /// Create a successful proof
    pub fn success(
        validation: ValidationResult,
        production_index_name: &str,
        table_name: &str,
        columns: &[String],
        index_type: Option<&str>,
    ) -> Self {
        let index_type_clause = index_type
            .map(|t| format!(" USING {}", t))
            .unwrap_or_default();
        let columns_clause = columns.join(", ");

        let create_sql = format!(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS {} ON {}{} ({})",
            production_index_name, table_name, index_type_clause, columns_clause
        );

        let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", production_index_name);

        Self {
            validation,
            create_index_sql: Some(create_sql),
            drop_index_sql: Some(drop_sql),
        }
    }

    /// Create a failed proof
    pub fn failure(validation: ValidationResult) -> Self {
        Self {
            validation,
            create_index_sql: None,
            drop_index_sql: None,
        }
    }

    /// Check if this proof passed validation
    pub fn is_validated(&self) -> bool {
        self.validation.status == ValidationStatus::Validated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::validation::models::{ExplainPlan, PlanNode, ScanType};

    #[test]
    fn test_generate_candidate_index_name() {
        // We can't test the full ProofGenerator without a pool, but we can test the logic
        let mut hasher = DefaultHasher::new();
        "users".hash(&mut hasher);
        "email".to_string().hash(&mut hasher);
        let hash = hasher.finish();
        let expected = format!("{}{:016x}", CANDIDATE_INDEX_PREFIX, hash);

        // Verify the name starts with our prefix
        assert!(expected.starts_with(CANDIDATE_INDEX_PREFIX));

        // Same inputs should produce same hash (deterministic)
        let mut hasher2 = DefaultHasher::new();
        "users".hash(&mut hasher2);
        "email".to_string().hash(&mut hasher2);
        let hash2 = hasher2.finish();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_candidate_index_prefix() {
        // Verify prefix is what we expect
        assert_eq!(CANDIDATE_INDEX_PREFIX, "_vaanyze_candidate_");

        // Verify candidate indexes can be identified
        assert!("_vaanyze_candidate_abc123".starts_with(CANDIDATE_INDEX_PREFIX));
        assert!(!"idx_users_email".starts_with(CANDIDATE_INDEX_PREFIX));
    }

    #[test]
    fn test_index_proof_success() {
        let validation = ValidationResult::pending(
            "rec_1".to_string(),
            "SELECT * FROM users WHERE email = $1".to_string(),
            "idx_test".to_string(),
            "users".to_string(),
            vec!["email".to_string()],
        );

        let proof = IndexProof::success(
            validation,
            "idx_users_email",
            "users",
            &["email".to_string()],
            None,
        );

        assert!(proof.create_index_sql.is_some());
        assert!(proof.drop_index_sql.is_some());

        let create_sql = proof.create_index_sql.unwrap();
        assert!(create_sql.contains("CREATE INDEX CONCURRENTLY"));
        assert!(create_sql.contains("idx_users_email"));
        assert!(create_sql.contains("users"));
        assert!(create_sql.contains("email"));
    }

    #[test]
    fn test_index_proof_with_type() {
        let validation = ValidationResult::pending(
            "rec_1".to_string(),
            "SELECT * FROM logs WHERE data @> '{}'".to_string(),
            "idx_test".to_string(),
            "logs".to_string(),
            vec!["data".to_string()],
        );

        let proof = IndexProof::success(
            validation,
            "idx_logs_data_gin",
            "logs",
            &["data".to_string()],
            Some("GIN"),
        );

        let create_sql = proof.create_index_sql.unwrap();
        assert!(create_sql.contains("USING GIN"));
    }

    #[test]
    fn test_destructive_check_result_variants() {
        // Ensure all variants exist and can be matched
        let allowed = DestructiveCheckResult::Allowed;
        let kill_switch = DestructiveCheckResult::KillSwitchActive;
        let read_only = DestructiveCheckResult::ReadOnly(ValidationReasonCode::ReadOnlyMode);

        match allowed {
            DestructiveCheckResult::Allowed => {}
            _ => panic!("Expected Allowed"),
        }

        match kill_switch {
            DestructiveCheckResult::KillSwitchActive => {}
            _ => panic!("Expected KillSwitchActive"),
        }

        match read_only {
            DestructiveCheckResult::ReadOnly(reason) => {
                assert_eq!(reason, ValidationReasonCode::ReadOnlyMode);
            }
            _ => panic!("Expected ReadOnly"),
        }
    }

    #[test]
    fn test_proof_mode_as_str() {
        assert_eq!(ProofMode::CandidateIndex.as_str(), "candidate_index");
        assert_eq!(ProofMode::ExplainOnly.as_str(), "explain_only");
    }

    #[test]
    fn test_proof_strength_as_str() {
        assert_eq!(ProofStrength::CandidateIndexTested.as_str(), "candidate_index_tested");
        assert_eq!(ProofStrength::ExplainOnlyExistingIndexes.as_str(), "explain_only_existing_indexes");
        assert_eq!(ProofStrength::NoProof.as_str(), "no_proof");
    }

    #[test]
    fn test_validation_result_explain_only() {
        let result = ValidationResult::pending_explain_only(
            "rec_1".to_string(),
            "SELECT * FROM users WHERE id = 1".to_string(),
            "idx_test".to_string(),
            "users".to_string(),
            vec!["id".to_string()],
        );

        assert_eq!(result.proof_mode, ProofMode::ExplainOnly);
        assert_eq!(result.proof_strength, ProofStrength::ExplainOnlyExistingIndexes);
    }

    #[test]
    fn test_validation_result_explain_only_method() {
        let result = ValidationResult::pending(
            "rec_1".to_string(),
            "SELECT * FROM users WHERE id = 1".to_string(),
            "idx_test".to_string(),
            "users".to_string(),
            vec!["id".to_string()],
        );

        // Create a dummy explain plan for testing
        let before_plan = ExplainPlan {
            raw_json: serde_json::json!({}),
            root_node: PlanNode {
                node_type: "Seq Scan".to_string(),
                relation_name: Some("users".to_string()),
                index_name: None,
                startup_cost: 0.0,
                total_cost: 100.0,
                plan_rows: 1000.0,
                plan_width: 100,
                filter: None,
                index_cond: None,
                children: vec![],
                actual_rows: None,
                actual_loops: None,
            },
            total_cost: 100.0,
            estimated_rows: 1000.0,
            planning_time_ms: None,
            execution_time_ms: None,
        };

        let explain_result = result.explain_only(
            before_plan,
            "System is in read-only mode. Cannot prove index effectiveness.".to_string(),
        );

        assert_eq!(explain_result.status, ValidationStatus::Skipped);
        assert_eq!(explain_result.reason_code, ValidationReasonCode::ReadOnlyMode);
        assert_eq!(explain_result.proof_mode, ProofMode::ExplainOnly);
        assert_eq!(explain_result.proof_strength, ProofStrength::ExplainOnlyExistingIndexes);
        assert!(explain_result.guidance.is_some());
        assert!(explain_result.before_plan.is_some());
        assert!(explain_result.after_plan.is_none());
    }
}
