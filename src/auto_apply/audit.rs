//! Audit log for tracking all recommendation lifecycle events.
//!
//! Records:
//! - Validation attempts/success/failure
//! - Apply attempts/success/failure
//! - Rollback triggered/success/failure
//! - Manual actions (approve, reject)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Maximum number of events to keep in memory
const MAX_AUDIT_EVENTS: usize = 10000;

/// Audit event types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    // Validation events
    ValidationStarted,
    ValidationSucceeded,
    ValidationFailed,
    ValidationSkipped,

    // Apply events
    ApplyStarted,
    ApplySucceeded,
    ApplyFailed,

    // Verification events
    VerificationStarted,
    VerificationSucceeded,
    VerificationFailed,

    // Rollback events
    RollbackStarted,
    RollbackSucceeded,
    RollbackFailed,

    // Manual actions
    ManualApproval,
    ManualRejection,
    ManualRollback,

    // System events
    ConfigChanged,
    SystemError,
    OperationBlocked,
}

impl AuditEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ValidationStarted => "VALIDATION_STARTED",
            Self::ValidationSucceeded => "VALIDATION_SUCCEEDED",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::ValidationSkipped => "VALIDATION_SKIPPED",
            Self::ApplyStarted => "APPLY_STARTED",
            Self::ApplySucceeded => "APPLY_SUCCEEDED",
            Self::ApplyFailed => "APPLY_FAILED",
            Self::VerificationStarted => "VERIFICATION_STARTED",
            Self::VerificationSucceeded => "VERIFICATION_SUCCEEDED",
            Self::VerificationFailed => "VERIFICATION_FAILED",
            Self::RollbackStarted => "ROLLBACK_STARTED",
            Self::RollbackSucceeded => "ROLLBACK_SUCCEEDED",
            Self::RollbackFailed => "ROLLBACK_FAILED",
            Self::ManualApproval => "MANUAL_APPROVAL",
            Self::ManualRejection => "MANUAL_REJECTION",
            Self::ManualRollback => "MANUAL_ROLLBACK",
            Self::ConfigChanged => "CONFIG_CHANGED",
            Self::SystemError => "SYSTEM_ERROR",
            Self::OperationBlocked => "OPERATION_BLOCKED",
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(
            self,
            Self::ValidationSucceeded
                | Self::ApplySucceeded
                | Self::VerificationSucceeded
                | Self::RollbackSucceeded
                | Self::ManualApproval
        )
    }

    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::ValidationFailed
                | Self::ApplyFailed
                | Self::VerificationFailed
                | Self::RollbackFailed
        )
    }
}

/// A single audit event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Unique event ID
    pub id: String,

    /// When the event occurred
    pub timestamp: DateTime<Utc>,

    /// Event type
    pub event_type: AuditEventType,

    /// Associated recommendation ID (if applicable)
    pub recommendation_id: Option<String>,

    /// Index name involved (if applicable)
    pub index_name: Option<String>,

    /// Table name involved (if applicable)
    pub table_name: Option<String>,

    /// SQL executed (redacted if needed)
    pub sql: Option<String>,

    /// Outcome message
    pub message: String,

    /// Duration in milliseconds (for operations)
    pub duration_ms: Option<i64>,

    /// User who triggered the event (if manual)
    pub triggered_by: Option<String>,

    /// Additional metadata
    pub metadata: Option<serde_json::Value>,
}

impl AuditEvent {
    /// Create a new audit event
    pub fn new(event_type: AuditEventType, message: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            event_type,
            recommendation_id: None,
            index_name: None,
            table_name: None,
            sql: None,
            message,
            duration_ms: None,
            triggered_by: None,
            metadata: None,
        }
    }

    pub fn with_recommendation(mut self, recommendation_id: &str) -> Self {
        self.recommendation_id = Some(recommendation_id.to_string());
        self
    }

    pub fn with_index(mut self, index_name: &str, table_name: &str) -> Self {
        self.index_name = Some(index_name.to_string());
        self.table_name = Some(table_name.to_string());
        self
    }

    pub fn with_sql(mut self, sql: &str) -> Self {
        // Redact potential sensitive data (simple redaction)
        let redacted = redact_sql(sql);
        self.sql = Some(redacted);
        self
    }

    pub fn with_duration(mut self, duration_ms: i64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    pub fn with_user(mut self, user: &str) -> Self {
        self.triggered_by = Some(user.to_string());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Simple SQL redaction (remove string literals and numbers)
fn redact_sql(sql: &str) -> String {
    // Replace string literals with '?'
    let re_strings = regex::Regex::new(r"'[^']*'").unwrap();
    let result = re_strings.replace_all(sql, "'?'");

    // Replace numbers with ?
    let re_numbers = regex::Regex::new(r"\b\d+\b").unwrap();
    re_numbers.replace_all(&result, "?").to_string()
}

/// In-memory audit log with bounded size
#[derive(Debug, Default)]
pub struct AuditLog {
    events: VecDeque<AuditEvent>,
    max_events: usize,
}

impl AuditLog {
    /// Create a new audit log
    pub fn new() -> Self {
        Self {
            events: VecDeque::new(),
            max_events: MAX_AUDIT_EVENTS,
        }
    }

    /// Create with custom max events
    pub fn with_capacity(max_events: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(max_events.min(MAX_AUDIT_EVENTS)),
            max_events,
        }
    }

    /// Add an event to the log
    pub fn log(&mut self, event: AuditEvent) {
        // Remove oldest if at capacity
        while self.events.len() >= self.max_events {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    /// Log a validation started event
    pub fn log_validation_started(&mut self, recommendation_id: &str, query: &str) {
        let event = AuditEvent::new(
            AuditEventType::ValidationStarted,
            format!("Starting EXPLAIN validation for recommendation {}", recommendation_id),
        )
        .with_recommendation(recommendation_id)
        .with_sql(query);
        self.log(event);
    }

    /// Log a validation success event
    pub fn log_validation_succeeded(
        &mut self,
        recommendation_id: &str,
        index_name: &str,
        table_name: &str,
        improvement_pct: f64,
        duration_ms: i64,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ValidationSucceeded,
            format!(
                "Validation passed: index '{}' improves query cost by {:.1}%",
                index_name, improvement_pct
            ),
        )
        .with_recommendation(recommendation_id)
        .with_index(index_name, table_name)
        .with_duration(duration_ms)
        .with_metadata(serde_json::json!({
            "improvement_pct": improvement_pct
        }));
        self.log(event);
    }

    /// Log a validation failure event
    pub fn log_validation_failed(
        &mut self,
        recommendation_id: &str,
        reason: &str,
        duration_ms: i64,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ValidationFailed,
            format!("Validation failed: {}", reason),
        )
        .with_recommendation(recommendation_id)
        .with_duration(duration_ms)
        .with_metadata(serde_json::json!({
            "failure_reason": reason
        }));
        self.log(event);
    }

    /// Log a validation skipped event (due to read-only mode or kill switch)
    pub fn log_validation_skipped(
        &mut self,
        recommendation_id: &str,
        reason: &str,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ValidationSkipped,
            format!("Validation skipped: {}", reason),
        )
        .with_recommendation(recommendation_id)
        .with_metadata(serde_json::json!({
            "skip_reason": reason
        }));
        self.log(event);
    }

    /// Log an apply started event
    pub fn log_apply_started(
        &mut self,
        recommendation_id: &str,
        index_name: &str,
        table_name: &str,
        sql: &str,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApplyStarted,
            format!("Creating index '{}' on table '{}'", index_name, table_name),
        )
        .with_recommendation(recommendation_id)
        .with_index(index_name, table_name)
        .with_sql(sql);
        self.log(event);
    }

    /// Log an apply success event
    pub fn log_apply_succeeded(
        &mut self,
        recommendation_id: &str,
        index_name: &str,
        table_name: &str,
        duration_ms: i64,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApplySucceeded,
            format!("Index '{}' created successfully", index_name),
        )
        .with_recommendation(recommendation_id)
        .with_index(index_name, table_name)
        .with_duration(duration_ms);
        self.log(event);
    }

    /// Log an apply failure event
    pub fn log_apply_failed(
        &mut self,
        recommendation_id: &str,
        index_name: &str,
        error: &str,
        duration_ms: i64,
    ) {
        let event = AuditEvent::new(
            AuditEventType::ApplyFailed,
            format!("Failed to create index '{}': {}", index_name, error),
        )
        .with_recommendation(recommendation_id)
        .with_duration(duration_ms)
        .with_metadata(serde_json::json!({
            "error": error
        }));
        self.log(event);
    }

    /// Log a rollback event
    pub fn log_rollback(
        &mut self,
        recommendation_id: &str,
        index_name: &str,
        table_name: &str,
        reason: &str,
        success: bool,
    ) {
        let event_type = if success {
            AuditEventType::RollbackSucceeded
        } else {
            AuditEventType::RollbackFailed
        };

        let message = if success {
            format!("Rolled back index '{}': {}", index_name, reason)
        } else {
            format!("Failed to rollback index '{}': {}", index_name, reason)
        };

        let event = AuditEvent::new(event_type, message)
            .with_recommendation(recommendation_id)
            .with_index(index_name, table_name)
            .with_metadata(serde_json::json!({
                "rollback_reason": reason
            }));
        self.log(event);
    }

    /// Get all events (newest first)
    pub fn get_events(&self) -> Vec<&AuditEvent> {
        self.events.iter().rev().collect()
    }

    /// Get events for a specific recommendation
    pub fn get_events_for_recommendation(&self, recommendation_id: &str) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.recommendation_id.as_deref() == Some(recommendation_id))
            .rev()
            .collect()
    }

    /// Get recent events (up to limit)
    pub fn get_recent_events(&self, limit: usize) -> Vec<&AuditEvent> {
        self.events.iter().rev().take(limit).collect()
    }

    /// Get events by type
    pub fn get_events_by_type(&self, event_type: AuditEventType) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.event_type == event_type)
            .rev()
            .collect()
    }

    /// Get failure events only
    pub fn get_failures(&self) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.event_type.is_failure())
            .rev()
            .collect()
    }

    /// Count events by type
    pub fn count_by_type(&self, event_type: AuditEventType) -> usize {
        self.events.iter().filter(|e| e.event_type == event_type).count()
    }

    /// Get total event count
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Clear all events
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Export to JSON
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.events.iter().collect::<Vec<_>>()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::new(
            AuditEventType::ValidationStarted,
            "Test message".to_string(),
        )
        .with_recommendation("rec_123")
        .with_index("idx_test", "users")
        .with_sql("SELECT * FROM users WHERE email = 'test@example.com'")
        .with_duration(100);

        assert_eq!(event.event_type, AuditEventType::ValidationStarted);
        assert_eq!(event.recommendation_id, Some("rec_123".to_string()));
        assert_eq!(event.index_name, Some("idx_test".to_string()));
        assert_eq!(event.duration_ms, Some(100));

        // SQL should be redacted
        let sql = event.sql.unwrap();
        assert!(sql.contains("'?'")); // String literal redacted
        assert!(!sql.contains("test@example.com"));
    }

    #[test]
    fn test_sql_redaction() {
        let sql = "SELECT * FROM users WHERE id = 123 AND email = 'test@example.com'";
        let redacted = redact_sql(sql);

        assert!(redacted.contains("id = ?"));
        assert!(redacted.contains("'?'"));
        assert!(!redacted.contains("123"));
        assert!(!redacted.contains("test@example.com"));
    }

    #[test]
    fn test_audit_log_capacity() {
        let mut log = AuditLog::with_capacity(5);

        for i in 0..10 {
            log.log(AuditEvent::new(
                AuditEventType::ValidationStarted,
                format!("Event {}", i),
            ));
        }

        assert_eq!(log.len(), 5);

        // Should have events 5-9 (newest)
        let events = log.get_recent_events(10);
        assert_eq!(events.len(), 5);
        assert!(events[0].message.contains("Event 9"));
    }

    #[test]
    fn test_audit_log_filtering() {
        let mut log = AuditLog::new();

        log.log_validation_started("rec_1", "SELECT * FROM users");
        log.log_validation_succeeded("rec_1", "idx_users", "users", 50.0, 100);
        log.log_validation_started("rec_2", "SELECT * FROM orders");
        log.log_validation_failed("rec_2", "Index not used", 50);

        // Filter by recommendation
        let rec1_events = log.get_events_for_recommendation("rec_1");
        assert_eq!(rec1_events.len(), 2);

        // Filter by type
        let started = log.get_events_by_type(AuditEventType::ValidationStarted);
        assert_eq!(started.len(), 2);

        // Get failures
        let failures = log.get_failures();
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn test_event_type_classification() {
        assert!(AuditEventType::ValidationSucceeded.is_success());
        assert!(AuditEventType::ApplySucceeded.is_success());
        assert!(!AuditEventType::ValidationFailed.is_success());

        assert!(AuditEventType::ValidationFailed.is_failure());
        assert!(AuditEventType::ApplyFailed.is_failure());
        assert!(!AuditEventType::ValidationSucceeded.is_failure());
    }
}
