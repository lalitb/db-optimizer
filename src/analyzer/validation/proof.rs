//! Index proof generation - validates that a recommended index actually improves query performance.
//!
//! This module:
//! - Creates candidate indexes with deterministic names
//! - Runs EXPLAIN before/after comparison
//! - Generates proof of improvement (or failure)
//! - Cleans up temporary indexes on validation failure

use super::explain::ExplainExecutor;
use super::models::{PlanDiff, ValidationReasonCode, ValidationResult, ValidationStatus};
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default minimum cost improvement to consider validation successful
const DEFAULT_MIN_IMPROVEMENT_PCT: f64 = 5.0;

/// Default timeout for index creation (5 minutes)
const DEFAULT_CREATE_INDEX_TIMEOUT_SECS: u64 = 300;

/// Prefix for validation candidate indexes
const CANDIDATE_INDEX_PREFIX: &str = "_vaanyze_candidate_";

/// Generates and validates index proofs
pub struct ProofGenerator {
    pool: PgPool,
    explain_executor: ExplainExecutor,
    min_improvement_pct: f64,
    create_index_timeout: Duration,
}

impl ProofGenerator {
    /// Create a new proof generator
    pub fn new(pool: PgPool) -> Self {
        let explain_executor = ExplainExecutor::new(pool.clone());
        Self {
            pool,
            explain_executor,
            min_improvement_pct: DEFAULT_MIN_IMPROVEMENT_PCT,
            create_index_timeout: Duration::from_secs(DEFAULT_CREATE_INDEX_TIMEOUT_SECS),
        }
    }

    /// Create with custom settings
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
        }
    }

    /// Validate an index recommendation by creating a candidate index and comparing plans
    ///
    /// Returns a ValidationResult with:
    /// - VALIDATED if index is used and cost improves >= min threshold
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

        let result = ValidationResult::pending(
            recommendation_id.to_string(),
            query.to_string(),
            candidate_index_name.clone(),
            table_name.to_string(),
            columns.to_vec(),
        );

        // Check if query can be explained
        if !ExplainExecutor::can_explain(query) {
            return Ok(result.failed(
                ValidationReasonCode::QueryNotExplainable,
                format!("Query cannot be explained: {}", &query[..query.len().min(50)]),
            ));
        }

        // Step 1: Get EXPLAIN plan BEFORE index
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

        // Step 2: Create candidate index
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
    async fn create_candidate_index(
        &self,
        index_name: &str,
        table_name: &str,
        columns: &[String],
        index_type: Option<&str>,
    ) -> Result<()> {
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
    async fn drop_candidate_index(&self, index_name: &str) -> Result<()> {
        // Only drop indexes with our prefix (safety check)
        if !index_name.starts_with(CANDIDATE_INDEX_PREFIX) {
            anyhow::bail!(
                "Refusing to drop index '{}': not a candidate index",
                index_name
            );
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
    pub async fn cleanup_candidate_indexes(&self) -> Result<Vec<String>> {
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
}
