//! Data models for EXPLAIN-based validation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Proof mode determines how validation runs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofMode {
    /// Full validation with candidate index (CREATE/DROP INDEX)
    /// - Creates temporary candidate index
    /// - Runs EXPLAIN before and after
    /// - Drops candidate index
    /// - Provides strong proof of index usefulness
    CandidateIndex,
    
    /// EXPLAIN-only validation (no DDL)
    /// - Runs EXPLAIN on existing schema only
    /// - Cannot prove proposed index would help
    /// - Used when destructive operations are blocked
    ExplainOnly,
}

impl ProofMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CandidateIndex => "candidate_index",
            Self::ExplainOnly => "explain_only",
        }
    }
}

/// Proof strength indicates how reliable the validation result is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofStrength {
    /// Strong proof - candidate index was created and tested
    CandidateIndexTested,
    /// Weak proof - only EXPLAIN on existing indexes, cannot prove proposed index helps
    ExplainOnlyExistingIndexes,
    /// No proof - validation could not run
    NoProof,
}

impl ProofStrength {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CandidateIndexTested => "candidate_index_tested",
            Self::ExplainOnlyExistingIndexes => "explain_only_existing_indexes",
            Self::NoProof => "no_proof",
        }
    }
}

/// Status of a validation attempt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationStatus {
    /// Validation pending (not yet run)
    Pending,
    /// Validation passed - index is used and cost improved
    Validated,
    /// Validation failed - see reason code
    Failed,
    /// Validation skipped (e.g., read-only mode)
    Skipped,
}

impl ValidationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Validated => "VALIDATED",
            Self::Failed => "FAILED",
            Self::Skipped => "SKIPPED",
        }
    }
}

/// Reason code explaining validation outcome
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationReasonCode {
    /// Index was used in the new plan - validation passed
    IndexUsed,
    /// Index was NOT used in the new plan
    NotUsed,
    /// Cost did not improve meaningfully (below threshold)
    NoCostImprovement,
    /// Query cannot be explained (DDL, utility, etc.)
    QueryNotExplainable,
    /// Insufficient permissions to create index or run EXPLAIN
    PermissionDenied,
    /// EXPLAIN or CREATE INDEX timed out
    Timeout,
    /// Query references non-existent table/column
    InvalidQuery,
    /// Operation blocked by read-only mode
    ReadOnlyMode,
    /// Operation blocked by kill switch
    KillSwitchActive,
    /// Other error occurred
    Error(String),
}

impl ValidationReasonCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::IndexUsed => "INDEX_USED",
            Self::NotUsed => "NOT_USED",
            Self::NoCostImprovement => "NO_COST_IMPROVEMENT",
            Self::QueryNotExplainable => "QUERY_NOT_EXPLAINABLE",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::Timeout => "TIMEOUT",
            Self::InvalidQuery => "INVALID_QUERY",
            Self::ReadOnlyMode => "READ_ONLY_MODE",
            Self::KillSwitchActive => "KILL_SWITCH_ACTIVE",
            Self::Error(_) => "ERROR",
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::IndexUsed)
    }

    /// Returns true if this reason indicates the operation was blocked by safety controls
    pub fn is_safety_blocked(&self) -> bool {
        matches!(self, Self::ReadOnlyMode | Self::KillSwitchActive)
    }
}

/// Result of validating a recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Recommendation ID being validated
    pub recommendation_id: String,

    /// Query ID (from pg_stat_statements) if available
    pub query_id: Option<i64>,

    /// Normalized query text used for validation
    pub query_text: String,

    /// Index name that was tested (temporary or candidate)
    pub index_name: String,

    /// Table the index is on
    pub table_name: String,

    /// Columns in the index
    pub columns: Vec<String>,

    /// Validation status
    pub status: ValidationStatus,

    /// Reason code explaining the outcome
    pub reason_code: ValidationReasonCode,

    /// Human-readable reason message
    pub reason_message: String,

    /// EXPLAIN plan before index creation
    pub before_plan: Option<ExplainPlan>,

    /// EXPLAIN plan after index creation
    pub after_plan: Option<ExplainPlan>,

    /// Diff summary (human readable)
    pub diff_summary: Option<PlanDiff>,

    /// When validation started
    pub started_at: DateTime<Utc>,

    /// When validation completed
    pub completed_at: Option<DateTime<Utc>>,

    /// Duration in milliseconds
    pub duration_ms: Option<i64>,

    /// The proof mode used for this validation
    #[serde(default = "default_proof_mode")]
    pub proof_mode: ProofMode,

    /// Strength of the proof provided
    #[serde(default = "default_proof_strength")]
    pub proof_strength: ProofStrength,

    /// Guidance for user when proof is incomplete
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

fn default_proof_mode() -> ProofMode {
    ProofMode::CandidateIndex
}

fn default_proof_strength() -> ProofStrength {
    ProofStrength::NoProof
}

impl ValidationResult {
    /// Create a new pending validation result
    pub fn pending(recommendation_id: String, query_text: String, index_name: String, table_name: String, columns: Vec<String>) -> Self {
        Self {
            recommendation_id,
            query_id: None,
            query_text,
            index_name,
            table_name,
            columns,
            status: ValidationStatus::Pending,
            reason_code: ValidationReasonCode::Error("Not yet validated".to_string()),
            reason_message: "Validation pending".to_string(),
            before_plan: None,
            after_plan: None,
            diff_summary: None,
            started_at: Utc::now(),
            completed_at: None,
            duration_ms: None,
            proof_mode: ProofMode::CandidateIndex,
            proof_strength: ProofStrength::NoProof,
            guidance: None,
        }
    }

    /// Create a pending result for explain-only mode
    pub fn pending_explain_only(recommendation_id: String, query_text: String, index_name: String, table_name: String, columns: Vec<String>) -> Self {
        Self {
            recommendation_id,
            query_id: None,
            query_text,
            index_name,
            table_name,
            columns,
            status: ValidationStatus::Pending,
            reason_code: ValidationReasonCode::Error("Not yet validated".to_string()),
            reason_message: "Validation pending (EXPLAIN-only mode)".to_string(),
            before_plan: None,
            after_plan: None,
            diff_summary: None,
            started_at: Utc::now(),
            completed_at: None,
            duration_ms: None,
            proof_mode: ProofMode::ExplainOnly,
            proof_strength: ProofStrength::ExplainOnlyExistingIndexes,
            guidance: None,
        }
    }

    /// Mark validation as successful
    pub fn validated(mut self, before: ExplainPlan, after: ExplainPlan, diff: PlanDiff) -> Self {
        self.status = ValidationStatus::Validated;
        self.reason_code = ValidationReasonCode::IndexUsed;
        self.reason_message = format!(
            "Index '{}' is used by query planner. {}",
            self.index_name,
            diff.summary
        );
        self.before_plan = Some(before);
        self.after_plan = Some(after);
        self.diff_summary = Some(diff);
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds());
        self.proof_mode = ProofMode::CandidateIndex;
        self.proof_strength = ProofStrength::CandidateIndexTested;
        self.guidance = None;
        self
    }

    /// Mark validation as failed
    pub fn failed(mut self, reason: ValidationReasonCode, message: String) -> Self {
        self.status = ValidationStatus::Failed;
        self.reason_code = reason;
        self.reason_message = message;
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds());
        self
    }

    /// Mark validation as skipped
    pub fn skipped(mut self, reason: &str) -> Self {
        self.status = ValidationStatus::Skipped;
        self.reason_code = ValidationReasonCode::Error(reason.to_string());
        self.reason_message = reason.to_string();
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds());
        self
    }

    /// Mark as explain-only result (no DDL was performed)
    pub fn explain_only(mut self, before_plan: ExplainPlan, guidance: String) -> Self {
        self.status = ValidationStatus::Skipped;
        self.reason_code = ValidationReasonCode::ReadOnlyMode;
        self.reason_message = "Cannot prove index effectiveness without creating candidate index. EXPLAIN-only mode.".to_string();
        self.before_plan = Some(before_plan);
        self.after_plan = None;
        self.diff_summary = None;
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds());
        self.proof_mode = ProofMode::ExplainOnly;
        self.proof_strength = ProofStrength::ExplainOnlyExistingIndexes;
        self.guidance = Some(guidance);
        self
    }
}

/// Parsed EXPLAIN plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainPlan {
    /// Raw JSON from EXPLAIN (FORMAT JSON)
    pub raw_json: serde_json::Value,

    /// Root node of the plan tree
    pub root_node: PlanNode,

    /// Total estimated cost (startup + total)
    pub total_cost: f64,

    /// Estimated rows
    pub estimated_rows: f64,

    /// Planning time (if available)
    pub planning_time_ms: Option<f64>,

    /// Execution time (if ANALYZE was used)
    pub execution_time_ms: Option<f64>,
}

impl ExplainPlan {
    /// Check if plan uses a specific index
    pub fn uses_index(&self, index_name: &str) -> bool {
        self.root_node.uses_index(index_name)
    }

    /// Get all index names used in the plan
    pub fn get_used_indexes(&self) -> Vec<String> {
        self.root_node.collect_indexes()
    }

    /// Get the primary scan type for a table
    pub fn get_scan_type_for_table(&self, table_name: &str) -> Option<ScanType> {
        self.root_node.find_scan_for_table(table_name)
    }
}

/// Node in an EXPLAIN plan tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanNode {
    /// Node type (e.g., "Seq Scan", "Index Scan", "Nested Loop")
    pub node_type: String,

    /// Relation name (table name) if applicable
    pub relation_name: Option<String>,

    /// Index name if this is an index scan
    pub index_name: Option<String>,

    /// Index condition (for index scans)
    pub index_cond: Option<String>,

    /// Filter condition
    pub filter: Option<String>,

    /// Startup cost
    pub startup_cost: f64,

    /// Total cost
    pub total_cost: f64,

    /// Estimated rows
    pub plan_rows: f64,

    /// Estimated row width
    pub plan_width: i32,

    /// Child nodes
    pub children: Vec<PlanNode>,

    /// Actual rows (only if ANALYZE was used)
    pub actual_rows: Option<f64>,

    /// Actual loops
    pub actual_loops: Option<i64>,
}

impl PlanNode {
    /// Check if this node or any child uses a specific index
    pub fn uses_index(&self, index_name: &str) -> bool {
        if let Some(ref idx) = self.index_name {
            if idx == index_name {
                return true;
            }
        }
        self.children.iter().any(|c| c.uses_index(index_name))
    }

    /// Collect all index names used in this subtree
    pub fn collect_indexes(&self) -> Vec<String> {
        let mut indexes = Vec::new();
        if let Some(ref idx) = self.index_name {
            indexes.push(idx.clone());
        }
        for child in &self.children {
            indexes.extend(child.collect_indexes());
        }
        indexes
    }

    /// Find the scan type for a specific table
    pub fn find_scan_for_table(&self, table_name: &str) -> Option<ScanType> {
        if let Some(ref rel) = self.relation_name {
            if rel == table_name {
                return Some(ScanType::from_node_type(&self.node_type));
            }
        }
        for child in &self.children {
            if let Some(scan) = child.find_scan_for_table(table_name) {
                return Some(scan);
            }
        }
        None
    }

    /// Check if this is a sequential scan
    pub fn is_seq_scan(&self) -> bool {
        self.node_type == "Seq Scan"
    }

    /// Check if this is any kind of index scan
    pub fn is_index_scan(&self) -> bool {
        matches!(
            self.node_type.as_str(),
            "Index Scan" | "Index Only Scan" | "Bitmap Index Scan" | "Bitmap Heap Scan"
        )
    }
}

/// Type of table scan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanType {
    SeqScan,
    IndexScan,
    IndexOnlyScan,
    BitmapIndexScan,
    BitmapHeapScan,
    Other,
}

impl ScanType {
    pub fn from_node_type(node_type: &str) -> Self {
        match node_type {
            "Seq Scan" => Self::SeqScan,
            "Index Scan" => Self::IndexScan,
            "Index Only Scan" => Self::IndexOnlyScan,
            "Bitmap Index Scan" => Self::BitmapIndexScan,
            "Bitmap Heap Scan" => Self::BitmapHeapScan,
            _ => Self::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SeqScan => "Seq Scan",
            Self::IndexScan => "Index Scan",
            Self::IndexOnlyScan => "Index Only Scan",
            Self::BitmapIndexScan => "Bitmap Index Scan",
            Self::BitmapHeapScan => "Bitmap Heap Scan",
            Self::Other => "Other",
        }
    }

    /// Check if this scan type uses an index
    pub fn uses_index(&self) -> bool {
        matches!(
            self,
            Self::IndexScan | Self::IndexOnlyScan | Self::BitmapIndexScan
        )
    }
}

/// Diff between two EXPLAIN plans
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDiff {
    /// Human-readable summary
    pub summary: String,

    /// Before scan type (for the relevant table)
    pub before_scan_type: Option<ScanType>,

    /// After scan type
    pub after_scan_type: Option<ScanType>,

    /// Before total cost
    pub before_cost: f64,

    /// After total cost
    pub after_cost: f64,

    /// Cost improvement percentage (positive = better)
    pub cost_improvement_pct: f64,

    /// Before estimated rows
    pub before_rows: f64,

    /// After estimated rows
    pub after_rows: f64,

    /// Whether the new index is used
    pub index_used: bool,

    /// Name of the index being tested
    pub index_name: String,
}

impl PlanDiff {
    /// Create a diff from two plans
    pub fn from_plans(
        before: &ExplainPlan,
        after: &ExplainPlan,
        table_name: &str,
        index_name: &str,
    ) -> Self {
        let before_scan = before.get_scan_type_for_table(table_name);
        let after_scan = after.get_scan_type_for_table(table_name);
        let index_used = after.uses_index(index_name);

        let cost_improvement_pct = if before.total_cost > 0.0 {
            ((before.total_cost - after.total_cost) / before.total_cost) * 100.0
        } else {
            0.0
        };

        let summary = Self::generate_summary(
            before_scan,
            after_scan,
            before.total_cost,
            after.total_cost,
            cost_improvement_pct,
            index_used,
            index_name,
        );

        Self {
            summary,
            before_scan_type: before_scan,
            after_scan_type: after_scan,
            before_cost: before.total_cost,
            after_cost: after.total_cost,
            cost_improvement_pct,
            before_rows: before.estimated_rows,
            after_rows: after.estimated_rows,
            index_used,
            index_name: index_name.to_string(),
        }
    }

    fn generate_summary(
        before_scan: Option<ScanType>,
        after_scan: Option<ScanType>,
        before_cost: f64,
        after_cost: f64,
        improvement_pct: f64,
        index_used: bool,
        index_name: &str,
    ) -> String {
        let mut parts = Vec::new();

        // Scan type change
        if let (Some(before), Some(after)) = (before_scan, after_scan) {
            if before != after {
                parts.push(format!(
                    "Plan changed: {} → {}",
                    before.as_str(),
                    after.as_str()
                ));
            }
        }

        // Cost change
        parts.push(format!(
            "Estimated cost: {:.0} → {:.0} ({:.1}% {})",
            before_cost,
            after_cost,
            improvement_pct.abs(),
            if improvement_pct > 0.0 {
                "improvement"
            } else {
                "regression"
            }
        ));

        // Index usage
        if index_used {
            parts.push(format!("Index '{}' is used", index_name));
        } else {
            parts.push(format!("Index '{}' is NOT used", index_name));
        }

        parts.join(". ")
    }

    /// Check if this diff indicates a meaningful improvement
    pub fn is_meaningful_improvement(&self, min_improvement_pct: f64) -> bool {
        self.index_used && self.cost_improvement_pct >= min_improvement_pct
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_status_as_str() {
        assert_eq!(ValidationStatus::Pending.as_str(), "PENDING");
        assert_eq!(ValidationStatus::Validated.as_str(), "VALIDATED");
        assert_eq!(ValidationStatus::Failed.as_str(), "FAILED");
    }

    #[test]
    fn test_reason_code_is_success() {
        assert!(ValidationReasonCode::IndexUsed.is_success());
        assert!(!ValidationReasonCode::NotUsed.is_success());
        assert!(!ValidationReasonCode::NoCostImprovement.is_success());
    }

    #[test]
    fn test_scan_type_uses_index() {
        assert!(!ScanType::SeqScan.uses_index());
        assert!(ScanType::IndexScan.uses_index());
        assert!(ScanType::IndexOnlyScan.uses_index());
        assert!(ScanType::BitmapIndexScan.uses_index());
        assert!(!ScanType::BitmapHeapScan.uses_index());
    }

    #[test]
    fn test_plan_diff_is_meaningful_improvement() {
        let diff = PlanDiff {
            summary: "test".to_string(),
            before_scan_type: Some(ScanType::SeqScan),
            after_scan_type: Some(ScanType::IndexScan),
            before_cost: 1000.0,
            after_cost: 100.0,
            cost_improvement_pct: 90.0,
            before_rows: 10000.0,
            after_rows: 100.0,
            index_used: true,
            index_name: "idx_test".to_string(),
        };

        assert!(diff.is_meaningful_improvement(5.0));
        assert!(diff.is_meaningful_improvement(50.0));
        assert!(!diff.is_meaningful_improvement(95.0));

        // Not meaningful if index not used
        let diff_not_used = PlanDiff {
            index_used: false,
            ..diff.clone()
        };
        assert!(!diff_not_used.is_meaningful_improvement(5.0));
    }

    #[test]
    fn test_plan_node_uses_index() {
        let node = PlanNode {
            node_type: "Index Scan".to_string(),
            relation_name: Some("users".to_string()),
            index_name: Some("idx_users_email".to_string()),
            index_cond: Some("(email = 'test@example.com')".to_string()),
            filter: None,
            startup_cost: 0.0,
            total_cost: 10.0,
            plan_rows: 1.0,
            plan_width: 100,
            children: vec![],
            actual_rows: None,
            actual_loops: None,
        };

        assert!(node.uses_index("idx_users_email"));
        assert!(!node.uses_index("idx_other"));
    }

    #[test]
    fn test_validation_result_lifecycle() {
        let result = ValidationResult::pending(
            "rec_123".to_string(),
            "SELECT * FROM users WHERE email = $1".to_string(),
            "idx_users_email".to_string(),
            "users".to_string(),
            vec!["email".to_string()],
        );

        assert_eq!(result.status, ValidationStatus::Pending);

        let failed = result.clone().failed(
            ValidationReasonCode::NotUsed,
            "Index not used by planner".to_string(),
        );
        assert_eq!(failed.status, ValidationStatus::Failed);
        assert_eq!(failed.reason_code, ValidationReasonCode::NotUsed);
    }
}
