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

// ============================================================================
// CONNECTION MONITORING MODELS
// ============================================================================

/// Connection state from pg_stat_activity
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    Active,
    Idle,
    IdleInTransaction,
    IdleInTransactionAborted,
    FastpathFunctionCall,
    Disabled,
}

impl ConnectionState {
    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "idle" => Self::Idle,
            "idle in transaction" => Self::IdleInTransaction,
            "idle in transaction (aborted)" => Self::IdleInTransactionAborted,
            "fastpath function call" => Self::FastpathFunctionCall,
            "disabled" => Self::Disabled,
            _ => Self::Idle,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::IdleInTransaction => "idle in transaction",
            Self::IdleInTransactionAborted => "idle in transaction (aborted)",
            Self::FastpathFunctionCall => "fastpath function call",
            Self::Disabled => "disabled",
        }
    }

    /// Returns true if this state indicates a problematic connection
    pub fn is_problematic(&self) -> bool {
        matches!(
            self,
            Self::IdleInTransaction | Self::IdleInTransactionAborted
        )
    }
}

/// Wait event type from pg_stat_activity
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WaitEventType {
    Lock,
    LWLock,
    BufferPin,
    Activity,
    Client,
    Extension,
    IPC,
    Timeout,
    IO,
    None,
}

impl WaitEventType {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s {
            Some("Lock") => Self::Lock,
            Some("LWLock") => Self::LWLock,
            Some("BufferPin") => Self::BufferPin,
            Some("Activity") => Self::Activity,
            Some("Client") => Self::Client,
            Some("Extension") => Self::Extension,
            Some("IPC") => Self::IPC,
            Some("Timeout") => Self::Timeout,
            Some("IO") => Self::IO,
            _ => Self::None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Lock => "Lock",
            Self::LWLock => "LWLock",
            Self::BufferPin => "BufferPin",
            Self::Activity => "Activity",
            Self::Client => "Client",
            Self::Extension => "Extension",
            Self::IPC => "IPC",
            Self::Timeout => "Timeout",
            Self::IO => "IO",
            Self::None => "None",
        }
    }
}

/// Active connection information from pg_stat_activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub pid: i32,
    pub database: String,
    pub username: String,
    pub application_name: Option<String>,
    pub client_addr: Option<String>,
    pub state: ConnectionState,
    pub query: String,
    pub query_start: Option<DateTime<Utc>>,
    pub state_change: Option<DateTime<Utc>>,
    pub wait_event_type: WaitEventType,
    pub wait_event: Option<String>,
    pub backend_type: String,
}

impl ConnectionInfo {
    /// Returns the duration since the state changed
    pub fn state_duration_secs(&self) -> Option<i64> {
        self.state_change.map(|sc| {
            let now = Utc::now();
            (now - sc).num_seconds()
        })
    }

    /// Returns the duration since query started
    pub fn query_duration_secs(&self) -> Option<i64> {
        self.query_start.map(|qs| {
            let now = Utc::now();
            (now - qs).num_seconds()
        })
    }

    /// Returns true if this connection is waiting for a lock
    pub fn is_locked(&self) -> bool {
        self.wait_event_type == WaitEventType::Lock
    }
}

/// Aggregated connection statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStats {
    pub total_connections: i32,
    pub max_connections: i32,
    pub connections_by_state: std::collections::HashMap<String, i32>,
    pub idle_in_transaction_count: i32,
    pub locked_connections_count: i32,
    pub oldest_idle_transaction_secs: Option<i64>,
    pub oldest_locked_query_secs: Option<i64>,
}

impl ConnectionStats {
    /// Returns connection usage percentage
    pub fn usage_percentage(&self) -> f64 {
        (self.total_connections as f64 / self.max_connections as f64) * 100.0
    }

    /// Returns true if connections are near limit
    pub fn is_near_limit(&self, threshold_percent: f64) -> bool {
        self.usage_percentage() >= threshold_percent
    }
}

/// Issue types we detect
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IssueType {
    SlowQuery,
    MissingIndex,
    UnusedIndex,
    HighVolumeQuery,

    // Connection issues
    IdleInTransaction,
    ConnectionPoolExhaustion,
    LockContention,
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

    // Connection monitoring stats (optional - only if connection monitoring is enabled)
    pub connection_stats: Option<ConnectionStats>,
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

#[cfg(test)]
mod connection_models_tests {
    use super::*;

    #[test]
    fn test_connection_state_from_str() {
        assert_eq!(ConnectionState::from_str("active"), ConnectionState::Active);
        assert_eq!(ConnectionState::from_str("idle"), ConnectionState::Idle);
        assert_eq!(ConnectionState::from_str("idle in transaction"), ConnectionState::IdleInTransaction);
        assert_eq!(ConnectionState::from_str("idle in transaction (aborted)"), ConnectionState::IdleInTransactionAborted);
        assert_eq!(ConnectionState::from_str("fastpath function call"), ConnectionState::FastpathFunctionCall);
        assert_eq!(ConnectionState::from_str("disabled"), ConnectionState::Disabled);
        assert_eq!(ConnectionState::from_str("unknown"), ConnectionState::Idle); // defaults to Idle
    }

    #[test]
    fn test_connection_state_as_str() {
        assert_eq!(ConnectionState::Active.as_str(), "active");
        assert_eq!(ConnectionState::Idle.as_str(), "idle");
        assert_eq!(ConnectionState::IdleInTransaction.as_str(), "idle in transaction");
        assert_eq!(ConnectionState::IdleInTransactionAborted.as_str(), "idle in transaction (aborted)");
    }

    #[test]
    fn test_connection_state_is_problematic() {
        assert!(ConnectionState::IdleInTransaction.is_problematic());
        assert!(ConnectionState::IdleInTransactionAborted.is_problematic());
        assert!(!ConnectionState::Active.is_problematic());
        assert!(!ConnectionState::Idle.is_problematic());
    }

    #[test]
    fn test_wait_event_type_from_str_opt() {
        assert_eq!(WaitEventType::from_str_opt(Some("Lock")), WaitEventType::Lock);
        assert_eq!(WaitEventType::from_str_opt(Some("LWLock")), WaitEventType::LWLock);
        assert_eq!(WaitEventType::from_str_opt(Some("IO")), WaitEventType::IO);
        assert_eq!(WaitEventType::from_str_opt(None), WaitEventType::None);
        assert_eq!(WaitEventType::from_str_opt(Some("Unknown")), WaitEventType::None);
    }

    #[test]
    fn test_connection_info_state_duration() {
        let now = Utc::now();
        let one_minute_ago = now - chrono::Duration::seconds(60);

        let conn = ConnectionInfo {
            pid: 1234,
            database: "testdb".to_string(),
            username: "testuser".to_string(),
            application_name: None,
            client_addr: None,
            state: ConnectionState::IdleInTransaction,
            query: "SELECT 1".to_string(),
            query_start: Some(now),
            state_change: Some(one_minute_ago),
            wait_event_type: WaitEventType::None,
            wait_event: None,
            backend_type: "client backend".to_string(),
        };

        let duration = conn.state_duration_secs();
        assert!(duration.is_some());
        // Should be approximately 60 seconds (allow 1 second tolerance for test execution time)
        let duration_val = duration.unwrap();
        assert!(duration_val >= 59 && duration_val <= 61, "Expected ~60 seconds, got {}", duration_val);
    }

    #[test]
    fn test_connection_info_query_duration() {
        let now = Utc::now();
        let two_minutes_ago = now - chrono::Duration::seconds(120);

        let conn = ConnectionInfo {
            pid: 1234,
            database: "testdb".to_string(),
            username: "testuser".to_string(),
            application_name: None,
            client_addr: None,
            state: ConnectionState::Active,
            query: "SELECT * FROM big_table".to_string(),
            query_start: Some(two_minutes_ago),
            state_change: Some(now),
            wait_event_type: WaitEventType::None,
            wait_event: None,
            backend_type: "client backend".to_string(),
        };

        let duration = conn.query_duration_secs();
        assert!(duration.is_some());
        let duration_val = duration.unwrap();
        assert!(duration_val >= 119 && duration_val <= 121, "Expected ~120 seconds, got {}", duration_val);
    }

    #[test]
    fn test_connection_info_is_locked() {
        let conn_locked = ConnectionInfo {
            pid: 1234,
            database: "testdb".to_string(),
            username: "testuser".to_string(),
            application_name: None,
            client_addr: None,
            state: ConnectionState::Active,
            query: "UPDATE users SET status = 'active'".to_string(),
            query_start: Some(Utc::now()),
            state_change: Some(Utc::now()),
            wait_event_type: WaitEventType::Lock,
            wait_event: Some("relation".to_string()),
            backend_type: "client backend".to_string(),
        };

        assert!(conn_locked.is_locked());

        let conn_not_locked = ConnectionInfo {
            pid: 1235,
            database: "testdb".to_string(),
            username: "testuser".to_string(),
            application_name: None,
            client_addr: None,
            state: ConnectionState::Active,
            query: "SELECT * FROM users".to_string(),
            query_start: Some(Utc::now()),
            state_change: Some(Utc::now()),
            wait_event_type: WaitEventType::None,
            wait_event: None,
            backend_type: "client backend".to_string(),
        };

        assert!(!conn_not_locked.is_locked());
    }

    #[test]
    fn test_connection_stats_usage_percentage() {
        let mut connections_by_state = std::collections::HashMap::new();
        connections_by_state.insert("active".to_string(), 75);

        let stats = ConnectionStats {
            total_connections: 75,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 0,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: None,
        };

        assert_eq!(stats.usage_percentage(), 75.0);
    }

    #[test]
    fn test_connection_stats_is_near_limit() {
        let mut connections_by_state = std::collections::HashMap::new();
        connections_by_state.insert("active".to_string(), 85);

        let stats_near_limit = ConnectionStats {
            total_connections: 85,
            max_connections: 100,
            connections_by_state: connections_by_state.clone(),
            idle_in_transaction_count: 0,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: None,
        };

        assert!(stats_near_limit.is_near_limit(80.0));
        assert!(!stats_near_limit.is_near_limit(90.0));

        connections_by_state.insert("active".to_string(), 50);
        let stats_not_near_limit = ConnectionStats {
            total_connections: 50,
            max_connections: 100,
            connections_by_state,
            idle_in_transaction_count: 0,
            locked_connections_count: 0,
            oldest_idle_transaction_secs: None,
            oldest_locked_query_secs: None,
        };

        assert!(!stats_not_near_limit.is_near_limit(80.0));
    }
}
