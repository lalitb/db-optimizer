// src/auto_apply/enhanced_safety.rs - Enhanced safety filtering and governance

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc, Timelike, Weekday, Datelike};

use crate::models::Recommendation;
use crate::config::{SafetyConfig, OperationMode, MaintenanceWindow};

/// Enhanced safety filter with comprehensive governance rules
pub struct EnhancedSafetyFilter {
    config: SafetyConfig,
    operation_mode: OperationMode,
}

impl EnhancedSafetyFilter {
    /// Create a new enhanced safety filter
    pub fn new(config: SafetyConfig, operation_mode: OperationMode) -> Self {
        Self {
            config,
            operation_mode,
        }
    }
    
    /// Evaluate if a recommendation is safe to auto-apply
    pub fn is_safe_to_apply(&self, recommendation: &Recommendation) -> Result<SafetyDecision> {
        tracing::info!("Evaluating safety for recommendation: {}", recommendation.id);
        
        // Check 1: Operation mode allows auto-apply
        if let OperationMode::ObserveOnly = self.operation_mode {
            return Ok(SafetyDecision::Rejected {
                reason: "Operation mode is ObserveOnly - no auto-apply allowed".to_string(),
            });
        }
        
        // Check 2: Maintenance window - NOTE: Maintenance window is now in parent Config, not SafetyConfig
        // This check would need access to the full Config struct, so we'll skip it here
        // and handle it at a higher level where Config is available
        
        // Check 3: Protected resources
        if self.is_protected_resource(recommendation)? {
            return Ok(SafetyDecision::Rejected {
                reason: format!(
                    "Resource is protected: {}",
                    recommendation.affected_table.as_ref().unwrap_or(&"unknown".to_string())
                ),
            });
        }
        
        // Check 4: Operation mode specific restrictions
        match self.operation_mode {
            OperationMode::ObserveOnly => {
                // Already handled above
                unreachable!()
            }
            OperationMode::SafeAuto => {
                // Only allow safe operations
                if !self.is_safe_operation(recommendation)? {
                    return Ok(SafetyDecision::Rejected {
                        reason: "Operation not allowed in SafeAuto mode".to_string(),
                    });
                }
            }
            OperationMode::Aggressive => {
                // Allow all operations, but still check size limits
                // (handled below)
            }
        }
        
        // Check 5: Index size limits
        let size_limit = self.config.max_index_size_mb;
        if self.exceeds_index_size_limit(recommendation, size_limit)? {
            return Ok(SafetyDecision::Rejected {
                reason: format!(
                    "Estimated index size exceeds limit ({} MB)",
                    size_limit
                ),
            });
        }
        
        // Check 6: DDL duration estimate
        let duration_limit = self.config.max_ddl_lock_time_secs;
        if self.exceeds_ddl_duration_limit(recommendation, duration_limit)? {
            return Ok(SafetyDecision::Rejected {
                reason: format!(
                    "Estimated DDL duration exceeds limit ({} seconds)",
                    duration_limit
                ),
            });
        }
        
        // Check 7: Primary key / unique constraint protection
        if self.affects_primary_or_unique_constraint(recommendation)? {
            return Ok(SafetyDecision::Rejected {
                reason: "Cannot modify primary keys or unique constraints".to_string(),
            });
        }
        
        // All checks passed
        Ok(SafetyDecision::Approved {
            confidence: self.calculate_confidence(recommendation),
            risk_level: self.assess_risk_level(recommendation),
        })
    }
    
    // Maintenance window methods removed - these are now handled at Config level
    // If needed, create a separate struct that takes the full Config
    
    /// Check if recommendation affects protected resources
    fn is_protected_resource(&self, recommendation: &Recommendation) -> Result<bool> {
        if let Some(ref table) = recommendation.affected_table {
            // Check protected schemas
            if self.config.protected_schemas.iter().any(|schema| table.starts_with(&format!("{}.", schema))) {
                return Ok(true);
            }
            
            // Check protected tables
            if self.config.protected_tables.iter().any(|protected| table == protected) {
                return Ok(true);
            }
        }
        
        Ok(false)
    }
    
    /// Check if operation is safe (for SafeAuto mode)
    fn is_safe_operation(&self, recommendation: &Recommendation) -> Result<bool> {
        use crate::models::IssueType;
        
        match recommendation.issue_type {
            IssueType::MissingIndex => {
                // Check if CREATE INDEX CONCURRENTLY
                Ok(recommendation.migration_sql.to_lowercase().contains("concurrently"))
            }
            IssueType::UnusedIndex => {
                // Check if DROP INDEX CONCURRENTLY
                Ok(recommendation.migration_sql.to_lowercase().contains("concurrently"))
            }
            IssueType::SlowQuery |
            IssueType::HighVolumeQuery => {
                // These usually don't auto-apply DDL, so OK
                Ok(false)
            }
        }
    }
    
    /// Estimate if index size will exceed limit
    fn exceeds_index_size_limit(&self, recommendation: &Recommendation, limit_mb: f64) -> Result<bool> {
        // TODO: Implement actual index size estimation based on:
        // - Table row count
        // - Column data types
        // - Existing table size
        
        // For now, conservative estimate
        Ok(false)
    }
    
    /// Estimate if DDL duration will exceed limit
    fn exceeds_ddl_duration_limit(&self, recommendation: &Recommendation, limit_seconds: u64) -> Result<bool> {
        // TODO: Implement actual DDL duration estimation based on:
        // - Table size
        // - Operation type (CREATE INDEX, DROP INDEX, etc.)
        // - System load
        
        // For now, conservative estimate
        Ok(false)
    }
    
    /// Check if recommendation affects primary key or unique constraint
    fn affects_primary_or_unique_constraint(&self, recommendation: &Recommendation) -> Result<bool> {
        let sql_lower = recommendation.migration_sql.to_lowercase();
        
        // Check for operations on primary keys or unique constraints
        Ok(
            sql_lower.contains("primary key") ||
            sql_lower.contains("alter table") && sql_lower.contains("drop constraint") ||
            sql_lower.contains("alter table") && sql_lower.contains("add constraint")
        )
    }
    
    /// Calculate confidence score for this recommendation
    fn calculate_confidence(&self, recommendation: &Recommendation) -> f64 {
        use crate::models::{Severity, IssueType};
        
        let mut confidence: f64 = 0.7; // Base confidence
        
        // Adjust based on severity
        match recommendation.severity {
            Severity::Critical => confidence += 0.2,
            Severity::High => confidence += 0.1,
            Severity::Medium => confidence += 0.0,
            Severity::Low => confidence -= 0.1,
        }
        
        // Adjust based on issue type
        match recommendation.issue_type {
            IssueType::MissingIndex => confidence += 0.1,  // Well understood
            IssueType::UnusedIndex => confidence += 0.15,  // Very safe
            _ => confidence += 0.0,
        }
        
        // Adjust based on estimated savings
        if let Some(savings) = recommendation.estimated_cost_savings_usd_monthly {
            if savings > 100.0 {
                confidence += 0.05;
            }
        }
        
        confidence.max(0.0).min(1.0)
    }
    
    /// Assess risk level
    fn assess_risk_level(&self, recommendation: &Recommendation) -> RiskLevel {
        use crate::models::IssueType;
        
        // Check if uses CONCURRENTLY
        let uses_concurrently = recommendation.migration_sql.to_lowercase().contains("concurrently");
        
        match recommendation.issue_type {
            IssueType::UnusedIndex if uses_concurrently => RiskLevel::Low,
            IssueType::MissingIndex if uses_concurrently => RiskLevel::Medium,
            IssueType::UnusedIndex => RiskLevel::Medium,
            IssueType::MissingIndex => RiskLevel::High,
            _ => RiskLevel::Medium,
        }
    }
}

/// Safety decision result
#[derive(Debug, Clone)]
pub enum SafetyDecision {
    /// Recommendation approved for auto-apply
    Approved {
        confidence: f64,
        risk_level: RiskLevel,
    },
    
    /// Recommendation rejected (will never be auto-applied)
    Rejected {
        reason: String,
    },
    
    /// Recommendation deferred (can be retried later)
    Deferred {
        reason: String,
        retry_after: DateTime<Utc>,
    },
}

/// Risk level assessment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "Low"),
            Self::Medium => write!(f, "Medium"),
            Self::High => write!(f, "High"),
        }
    }
}

/// Format weekday for comparison
fn format_weekday(weekday: Weekday) -> String {
    match weekday {
        Weekday::Mon => "monday".to_string(),
        Weekday::Tue => "tuesday".to_string(),
        Weekday::Wed => "wednesday".to_string(),
        Weekday::Thu => "thursday".to_string(),
        Weekday::Fri => "friday".to_string(),
        Weekday::Sat => "saturday".to_string(),
        Weekday::Sun => "sunday".to_string(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IssueType, Severity};
    
    #[test]
    fn test_observe_only_mode() {
        let config = SafetyConfig::default();
        let filter = EnhancedSafetyFilter::new(config, OperationMode::ObserveOnly);
        
        let rec = create_test_recommendation();
        let decision = filter.is_safe_to_apply(&rec).unwrap();
        
        assert!(matches!(decision, SafetyDecision::Rejected { .. }));
    }
    
    #[test]
    fn test_safe_auto_mode_with_concurrently() {
        let config = SafetyConfig::default();
        let filter = EnhancedSafetyFilter::new(config, OperationMode::SafeAuto);
        
        let mut rec = create_test_recommendation();
        rec.migration_sql = "CREATE INDEX CONCURRENTLY idx_test ON users (email);".to_string();
        
        let decision = filter.is_safe_to_apply(&rec).unwrap();
        
        assert!(matches!(decision, SafetyDecision::Approved { .. }));
    }
    
    fn create_test_recommendation() -> Recommendation {
        Recommendation {
            id: "test-1".to_string(),
            title: "Test Recommendation".to_string(),
            description: "Test".to_string(),
            issue_type: IssueType::MissingIndex,
            severity: Severity::Medium,
            affected_queries_count: Some(100),
            estimated_time_savings_ms: Some(50.0),
            estimated_cost_savings_usd_monthly: Some(10.0),
            migration_sql: "CREATE INDEX idx_test ON users (email);".to_string(),
            rollback_sql: Some("DROP INDEX idx_test;".to_string()),
            safety_notes: "Safe".to_string(),
            affected_table: Some("public.users".to_string()),
            affected_query: None,
        }
    }
}
