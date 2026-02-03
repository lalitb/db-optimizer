//! State machine for recommendation lifecycle.
//!
//! States:
//!   NEW → VALIDATED → APPLIED → VERIFYING → STABLE
//!   
//! Failure states:
//!   FAILED (validation/apply error)
//!   ROLLED_BACK (verification failed, index dropped)
//!   REJECTED (manually rejected by user)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// States in the recommendation lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommendationState {
    /// Initial state - recommendation generated but not validated
    New,
    /// EXPLAIN proof completed - index improves query plan
    Validated,
    /// Index has been created in the database
    Applied,
    /// In verification window - monitoring for regressions
    Verifying,
    /// Verification passed - index is stable and in use
    Stable,
    /// Validation or apply failed
    Failed,
    /// Rolled back after verification found issues
    RolledBack,
    /// Manually rejected by user
    Rejected,
}

impl RecommendationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "NEW",
            Self::Validated => "VALIDATED",
            Self::Applied => "APPLIED",
            Self::Verifying => "VERIFYING",
            Self::Stable => "STABLE",
            Self::Failed => "FAILED",
            Self::RolledBack => "ROLLED_BACK",
            Self::Rejected => "REJECTED",
        }
    }

    /// Check if this state allows transition to the target state
    pub fn can_transition_to(&self, target: RecommendationState) -> bool {
        match self {
            Self::New => matches!(target, Self::Validated | Self::Failed | Self::Rejected),
            Self::Validated => matches!(target, Self::Applied | Self::Failed | Self::Rejected),
            Self::Applied => matches!(target, Self::Verifying | Self::Failed | Self::RolledBack),
            Self::Verifying => matches!(target, Self::Stable | Self::RolledBack | Self::Failed),
            // Terminal states - no transitions out
            Self::Stable | Self::Failed | Self::RolledBack | Self::Rejected => false,
        }
    }

    /// Check if this is a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Stable | Self::Failed | Self::RolledBack | Self::Rejected
        )
    }

    /// Check if the index exists in this state
    pub fn has_index(&self) -> bool {
        matches!(self, Self::Applied | Self::Verifying | Self::Stable)
    }
}

/// Tracks the full state of a recommendation through its lifecycle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationLifecycle {
    /// Recommendation ID
    pub recommendation_id: String,

    /// Current state
    pub state: RecommendationState,

    /// State transition history
    pub transitions: Vec<StateTransition>,

    /// When the recommendation was created
    pub created_at: DateTime<Utc>,

    /// When the state last changed
    pub updated_at: DateTime<Utc>,

    /// Validation result (if validated)
    pub validation_id: Option<String>,

    /// Index name (if applied)
    pub index_name: Option<String>,

    /// Verification window start (if verifying)
    pub verification_started_at: Option<DateTime<Utc>>,

    /// Verification window duration in seconds
    pub verification_window_secs: Option<u64>,

    /// Failure reason (if failed or rolled back)
    pub failure_reason: Option<String>,

    /// Rollback reason (if rolled back)
    pub rollback_reason: Option<RollbackReason>,
}

impl RecommendationLifecycle {
    /// Create a new lifecycle for a recommendation
    pub fn new(recommendation_id: String) -> Self {
        let now = Utc::now();
        Self {
            recommendation_id,
            state: RecommendationState::New,
            transitions: vec![StateTransition {
                from: None,
                to: RecommendationState::New,
                timestamp: now,
                reason: "Recommendation created".to_string(),
                metadata: None,
            }],
            created_at: now,
            updated_at: now,
            validation_id: None,
            index_name: None,
            verification_started_at: None,
            verification_window_secs: None,
            failure_reason: None,
            rollback_reason: None,
        }
    }

    /// Attempt to transition to a new state
    pub fn transition(
        &mut self,
        new_state: RecommendationState,
        reason: String,
    ) -> Result<(), StateTransitionError> {
        if !self.state.can_transition_to(new_state) {
            return Err(StateTransitionError::InvalidTransition {
                from: self.state,
                to: new_state,
            });
        }

        let transition = StateTransition {
            from: Some(self.state),
            to: new_state,
            timestamp: Utc::now(),
            reason,
            metadata: None,
        };

        self.transitions.push(transition);
        self.state = new_state;
        self.updated_at = Utc::now();

        Ok(())
    }

    /// Mark as validated
    pub fn mark_validated(&mut self, validation_id: String) -> Result<(), StateTransitionError> {
        self.transition(
            RecommendationState::Validated,
            "EXPLAIN proof passed".to_string(),
        )?;
        self.validation_id = Some(validation_id);
        Ok(())
    }

    /// Mark as applied
    pub fn mark_applied(&mut self, index_name: String) -> Result<(), StateTransitionError> {
        self.transition(
            RecommendationState::Applied,
            format!("Index '{}' created", index_name),
        )?;
        self.index_name = Some(index_name);
        Ok(())
    }

    /// Start verification window
    pub fn start_verification(&mut self, window_secs: u64) -> Result<(), StateTransitionError> {
        self.transition(
            RecommendationState::Verifying,
            format!("Starting {}s verification window", window_secs),
        )?;
        self.verification_started_at = Some(Utc::now());
        self.verification_window_secs = Some(window_secs);
        Ok(())
    }

    /// Check if verification window has elapsed
    pub fn is_verification_window_complete(&self) -> bool {
        if self.state != RecommendationState::Verifying {
            return false;
        }

        match (self.verification_started_at, self.verification_window_secs) {
            (Some(started), Some(window)) => {
                let elapsed = (Utc::now() - started).num_seconds() as u64;
                elapsed >= window
            }
            _ => false,
        }
    }

    /// Mark as stable (verification passed)
    pub fn mark_stable(&mut self) -> Result<(), StateTransitionError> {
        self.transition(
            RecommendationState::Stable,
            "Verification passed - index is stable".to_string(),
        )
    }

    /// Mark as failed
    pub fn mark_failed(&mut self, reason: String) -> Result<(), StateTransitionError> {
        self.transition(RecommendationState::Failed, reason.clone())?;
        self.failure_reason = Some(reason);
        Ok(())
    }

    /// Mark as rolled back
    pub fn mark_rolled_back(&mut self, reason: RollbackReason) -> Result<(), StateTransitionError> {
        let reason_str = reason.to_string();
        self.transition(RecommendationState::RolledBack, reason_str)?;
        self.rollback_reason = Some(reason);
        Ok(())
    }

    /// Mark as rejected (manual rejection)
    pub fn mark_rejected(&mut self, reason: String) -> Result<(), StateTransitionError> {
        self.transition(RecommendationState::Rejected, reason)
    }

    /// Get time in current state
    pub fn time_in_state(&self) -> chrono::Duration {
        Utc::now() - self.updated_at
    }

    /// Get total lifecycle duration
    pub fn total_duration(&self) -> chrono::Duration {
        Utc::now() - self.created_at
    }
}

/// A state transition record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    /// Previous state (None if initial)
    pub from: Option<RecommendationState>,

    /// New state
    pub to: RecommendationState,

    /// When the transition occurred
    pub timestamp: DateTime<Utc>,

    /// Reason for the transition
    pub reason: String,

    /// Optional metadata (JSON)
    pub metadata: Option<serde_json::Value>,
}

/// Reasons for rolling back an index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RollbackReason {
    /// Index was not used during verification window
    IndexNotUsed {
        idx_scan_count: i64,
        expected_min: i64,
    },

    /// Performance regressed after index creation
    PerformanceRegression {
        metric_name: String,
        before_value: f64,
        after_value: f64,
        regression_pct: f64,
    },

    /// Manual rollback requested
    ManualRequest { requested_by: Option<String> },

    /// Error during verification
    VerificationError { error: String },
}

impl std::fmt::Display for RollbackReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IndexNotUsed {
                idx_scan_count,
                expected_min,
            } => {
                write!(
                    f,
                    "Index unused: {} scans (expected >= {})",
                    idx_scan_count, expected_min
                )
            }
            Self::PerformanceRegression {
                metric_name,
                before_value,
                after_value,
                regression_pct,
            } => {
                write!(
                    f,
                    "{} regressed: {:.2} → {:.2} ({:.1}% worse)",
                    metric_name, before_value, after_value, regression_pct
                )
            }
            Self::ManualRequest { requested_by } => {
                if let Some(user) = requested_by {
                    write!(f, "Manual rollback by {}", user)
                } else {
                    write!(f, "Manual rollback requested")
                }
            }
            Self::VerificationError { error } => {
                write!(f, "Verification error: {}", error)
            }
        }
    }
}

/// Error when attempting an invalid state transition
#[derive(Debug, Clone)]
pub enum StateTransitionError {
    InvalidTransition {
        from: RecommendationState,
        to: RecommendationState,
    },
}

impl std::fmt::Display for StateTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { from, to } => {
                write!(
                    f,
                    "Invalid state transition: {} → {}",
                    from.as_str(),
                    to.as_str()
                )
            }
        }
    }
}

impl std::error::Error for StateTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        // Valid transitions
        assert!(RecommendationState::New.can_transition_to(RecommendationState::Validated));
        assert!(RecommendationState::New.can_transition_to(RecommendationState::Failed));
        assert!(RecommendationState::New.can_transition_to(RecommendationState::Rejected));

        assert!(RecommendationState::Validated.can_transition_to(RecommendationState::Applied));
        assert!(RecommendationState::Validated.can_transition_to(RecommendationState::Rejected));

        assert!(RecommendationState::Applied.can_transition_to(RecommendationState::Verifying));
        assert!(RecommendationState::Applied.can_transition_to(RecommendationState::RolledBack));

        assert!(RecommendationState::Verifying.can_transition_to(RecommendationState::Stable));
        assert!(RecommendationState::Verifying.can_transition_to(RecommendationState::RolledBack));

        // Invalid transitions
        assert!(!RecommendationState::New.can_transition_to(RecommendationState::Applied));
        assert!(!RecommendationState::New.can_transition_to(RecommendationState::Stable));
        assert!(!RecommendationState::Validated.can_transition_to(RecommendationState::Stable));
        assert!(!RecommendationState::Stable.can_transition_to(RecommendationState::New));
        assert!(!RecommendationState::Failed.can_transition_to(RecommendationState::Validated));
    }

    #[test]
    fn test_terminal_states() {
        assert!(!RecommendationState::New.is_terminal());
        assert!(!RecommendationState::Validated.is_terminal());
        assert!(!RecommendationState::Applied.is_terminal());
        assert!(!RecommendationState::Verifying.is_terminal());

        assert!(RecommendationState::Stable.is_terminal());
        assert!(RecommendationState::Failed.is_terminal());
        assert!(RecommendationState::RolledBack.is_terminal());
        assert!(RecommendationState::Rejected.is_terminal());
    }

    #[test]
    fn test_has_index() {
        assert!(!RecommendationState::New.has_index());
        assert!(!RecommendationState::Validated.has_index());
        assert!(RecommendationState::Applied.has_index());
        assert!(RecommendationState::Verifying.has_index());
        assert!(RecommendationState::Stable.has_index());
        assert!(!RecommendationState::Failed.has_index());
        assert!(!RecommendationState::RolledBack.has_index());
    }

    #[test]
    fn test_lifecycle_happy_path() {
        let mut lifecycle = RecommendationLifecycle::new("rec_123".to_string());
        assert_eq!(lifecycle.state, RecommendationState::New);

        lifecycle.mark_validated("val_456".to_string()).unwrap();
        assert_eq!(lifecycle.state, RecommendationState::Validated);
        assert_eq!(lifecycle.validation_id, Some("val_456".to_string()));

        lifecycle
            .mark_applied("idx_users_email".to_string())
            .unwrap();
        assert_eq!(lifecycle.state, RecommendationState::Applied);
        assert_eq!(lifecycle.index_name, Some("idx_users_email".to_string()));

        lifecycle.start_verification(900).unwrap(); // 15 min
        assert_eq!(lifecycle.state, RecommendationState::Verifying);
        assert!(lifecycle.verification_started_at.is_some());

        lifecycle.mark_stable().unwrap();
        assert_eq!(lifecycle.state, RecommendationState::Stable);
        assert!(lifecycle.state.is_terminal());

        // Verify transition history
        assert_eq!(lifecycle.transitions.len(), 5);
    }

    #[test]
    fn test_lifecycle_validation_failure() {
        let mut lifecycle = RecommendationLifecycle::new("rec_123".to_string());

        lifecycle
            .mark_failed("Index not used by planner".to_string())
            .unwrap();
        assert_eq!(lifecycle.state, RecommendationState::Failed);
        assert_eq!(
            lifecycle.failure_reason,
            Some("Index not used by planner".to_string())
        );
    }

    #[test]
    fn test_lifecycle_rollback() {
        let mut lifecycle = RecommendationLifecycle::new("rec_123".to_string());
        lifecycle.mark_validated("val_1".to_string()).unwrap();
        lifecycle.mark_applied("idx_test".to_string()).unwrap();
        lifecycle.start_verification(900).unwrap();

        let reason = RollbackReason::IndexNotUsed {
            idx_scan_count: 0,
            expected_min: 10,
        };
        lifecycle.mark_rolled_back(reason.clone()).unwrap();

        assert_eq!(lifecycle.state, RecommendationState::RolledBack);
        assert!(lifecycle.rollback_reason.is_some());
    }

    #[test]
    fn test_invalid_transition() {
        let mut lifecycle = RecommendationLifecycle::new("rec_123".to_string());

        // Can't go directly from NEW to STABLE
        let result = lifecycle.transition(
            RecommendationState::Stable,
            "Trying invalid transition".to_string(),
        );
        assert!(result.is_err());
        assert_eq!(lifecycle.state, RecommendationState::New);
    }

    #[test]
    fn test_rollback_reason_display() {
        let unused = RollbackReason::IndexNotUsed {
            idx_scan_count: 0,
            expected_min: 10,
        };
        assert!(unused.to_string().contains("0 scans"));

        let regression = RollbackReason::PerformanceRegression {
            metric_name: "p99_latency".to_string(),
            before_value: 100.0,
            after_value: 150.0,
            regression_pct: 50.0,
        };
        assert!(regression.to_string().contains("regressed"));
        assert!(regression.to_string().contains("50.0% worse"));
    }
}
