use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Query statistics from pg_stat_statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStats {
    pub queryid: Option<i64>,
    pub query: String,
    pub calls: i64,
    pub total_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub rows: i64,
}

/// Index information from pg_stat_user_indexes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub schema_name: String,
    pub table_name: String,
    pub index_name: String,
    pub index_size_mb: f64,
    pub index_scans: i64,
    pub is_unique: bool,
    pub is_primary: bool,
}

/// Table information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub schema_name: String,
    pub table_name: String,
    pub row_count: i64,
    pub total_size_mb: f64,
    pub table_size_mb: f64,
    pub index_size_mb: f64,
}

/// Issue types we detect
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IssueType {
    SlowQuery,
    MissingIndex,
    UnusedIndex,
    HighVolumeQuery,
}

/// Severity levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Default for Severity {
    fn default() -> Self {
        Self::Low
    }
}

impl Severity {
    pub fn as_str(&self) -> &str {
        match self {
            Severity::Low => "LOW",
            Severity::Medium => "MEDIUM",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }
}

/// A recommendation with actionable SQL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub id: String,
    pub title: String,
    pub description: String,
    pub issue_type: IssueType,
    pub severity: Severity,
    
    // Impact metrics
    pub affected_queries_count: Option<i64>,
    pub estimated_time_savings_ms: Option<f64>,
    pub estimated_cost_savings_usd_monthly: Option<f64>,
    
    // Actionable SQL
    pub migration_sql: String,
    pub rollback_sql: Option<String>,
    pub safety_notes: String,
    
    // Context
    pub affected_table: Option<String>,
    pub affected_query: Option<String>,
}

/// Full analysis report
#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub database_name: String,
    pub postgres_version: String,
    pub analyzed_at: DateTime<Utc>,
    
    pub summary: ReportSummary,
    pub recommendations: Vec<Recommendation>,
    
    // Raw stats for reference
    pub total_queries_analyzed: usize,
    pub total_tables: usize,
    pub total_indexes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReportSummary {
    pub issues_found: usize,
    pub critical_issues: usize,
    pub high_priority_issues: usize,
    
    pub slow_queries: usize,
    pub missing_indexes_suggested: usize,
    pub unused_indexes_found: usize,
    
    pub total_estimated_savings_usd_monthly: f64,
}

// ============================================================================
// AUTO-APPLY FEATURE MODELS
// ============================================================================

/// Configuration for auto-apply behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoApplyConfig {
    /// Whether auto-apply is enabled
    pub enabled: bool,
    
    /// Dry-run mode (simulate without executing)
    pub dry_run: bool,
    
    /// Minimum estimated monthly savings to auto-approve (USD)
    pub min_savings_threshold: f64,
    
    /// Maximum risk level to auto-approve
    pub max_risk_level: RiskLevel,
    
    /// Duration to monitor impact after applying (seconds)
    pub verification_duration_secs: u64,
    
    /// Minimum improvement % to consider successful
    pub min_improvement_threshold: f64,
    
    /// Auto-rollback if no improvement detected
    pub auto_rollback_on_failure: bool,
    
    /// Timeout for SQL execution (seconds)
    pub execution_timeout_secs: u64,
    
    /// Require manual approval for certain operations
    pub require_manual_approval: Vec<IssueType>,
    
    /// Slack webhook for notifications
    pub slack_webhook: Option<String>,
}

impl Default for AutoApplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dry_run: true,
            min_savings_threshold: 50.0,
            max_risk_level: RiskLevel::Low,
            verification_duration_secs: 300, // 5 minutes
            min_improvement_threshold: 10.0, // 10% improvement required
            auto_rollback_on_failure: true,
            execution_timeout_secs: 600, // 10 minutes
            require_manual_approval: vec![IssueType::SlowQuery],
            slack_webhook: None,
        }
    }
}

/// Risk level for a recommendation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    /// Safe operations: CREATE INDEX CONCURRENTLY, DROP unused index
    Low,
    
    /// Moderate risk: ALTER TABLE (non-blocking), configuration changes
    Medium,
    
    /// High risk: Query rewrites, schema changes with locks
    High,
    
    /// Critical: Operations requiring downtime or data migration
    Critical,
}

/// Baseline metrics captured before applying a change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceBaseline {
    pub captured_at: DateTime<Utc>,
    pub query_id: Option<i64>,
    pub table_name: Option<String>,
    
    // Performance metrics
    pub mean_exec_time_ms: f64,
    pub p95_exec_time_ms: f64,
    pub p99_exec_time_ms: f64,
    pub calls_per_minute: f64,
    
    // Query plan
    pub query_plan: Option<String>, // EXPLAIN ANALYZE output
    pub plan_cost: Option<f64>,
    
    // System metrics
    pub table_size_mb: Option<f64>,
    pub index_size_mb: Option<f64>,
    pub total_queries_analyzed: u64,
}

/// Result of applying a recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub recommendation_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration_secs: f64,
    
    pub status: ApplyStatus,
    pub error_message: Option<String>,
    
    pub baseline: PerformanceBaseline,
    pub post_apply: Option<PerformanceBaseline>,
    
    pub actual_improvement_pct: Option<f64>,
    pub met_threshold: bool,
    pub rollback_executed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApplyStatus {
    /// Simulation only (dry-run)
    Simulated,
    
    /// Waiting for manual approval
    PendingApproval,
    
    /// Currently executing
    InProgress,
    
    /// Successfully applied and verified
    Success,
    
    /// Applied but improvement below threshold, rolled back
    RolledBack,
    
    /// Failed during execution
    Failed,
    
    /// Skipped due to risk or configuration
    Skipped,
}

/// Extended recommendation with auto-apply metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicableRecommendation {
    pub recommendation: Recommendation,
    pub risk_assessment: RiskAssessment,
    pub apply_strategy: ApplyStrategy,
    pub estimated_duration_secs: u64,
    pub can_auto_apply: bool,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub risk_level: RiskLevel,
    pub risk_factors: Vec<String>,
    pub mitigation_steps: Vec<String>,
    pub rollback_complexity: RollbackComplexity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RollbackComplexity {
    /// Simple: DROP INDEX (instant)
    Simple,
    
    /// Moderate: Index rebuild required
    Moderate,
    
    /// Complex: May require downtime or data migration
    Complex,
    
    /// Irreversible: Cannot be automatically rolled back
    Irreversible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApplyStrategy {
    /// Create index concurrently (non-blocking)
    CreateIndexConcurrently {
        index_name: String,
        table_name: String,
        columns: Vec<String>,
    },
    
    /// Drop unused index
    DropIndex {
        index_name: String,
        schema: String,
    },
    
    /// Rewrite query (application-level change)
    QueryRewrite {
        original_query: String,
        optimized_query: String,
    },
    
    /// Configuration change (postgresql.conf)
    ConfigurationChange {
        parameter: String,
        current_value: String,
        new_value: String,
    },
}
