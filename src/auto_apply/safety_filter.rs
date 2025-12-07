use crate::models::*;

pub struct SafetyFilter {
    config: AutoApplyConfig,
}

impl SafetyFilter {
    pub fn new(config: AutoApplyConfig) -> Self {
        Self { config }
    }

    /// Assess risk and determine if recommendation can be auto-applied
    pub fn assess(&self, rec: &Recommendation) -> ApplicableRecommendation {
        let risk_assessment = self.assess_risk(rec);
        let apply_strategy = self.determine_strategy(rec);
        let estimated_duration = self.estimate_duration(rec, &apply_strategy);

        let can_auto_apply = self.can_auto_apply(rec, &risk_assessment);
        let requires_approval = self.requires_manual_approval(rec, &risk_assessment);

        ApplicableRecommendation {
            recommendation: rec.clone(),
            risk_assessment,
            apply_strategy,
            estimated_duration_secs: estimated_duration,
            can_auto_apply,
            requires_approval,
        }
    }

    fn assess_risk(&self, rec: &Recommendation) -> RiskAssessment {
        let mut risk_factors = Vec::new();
        let mut mitigation_steps = Vec::new();

        let risk_level = match rec.issue_type {
            IssueType::MissingIndex => {
                // Check table size
                if let Some(table) = &rec.affected_table {
                    if self.estimate_table_size(table) > 10_000_000.0 {
                        risk_factors.push("Large table (>10M rows)".to_string());
                        risk_factors.push("Index creation may take >10 minutes".to_string());
                        mitigation_steps
                            .push("Using CONCURRENTLY to avoid table locks".to_string());
                        RiskLevel::Medium
                    } else {
                        mitigation_steps
                            .push("Using CONCURRENTLY for non-blocking index creation".to_string());
                        RiskLevel::Low
                    }
                } else {
                    RiskLevel::Low
                }
            }

            IssueType::UnusedIndex => {
                // Dropping index is low risk
                mitigation_steps.push("Can be recreated instantly if needed".to_string());
                RiskLevel::Low
            }

            IssueType::SlowQuery => {
                // Query rewrites are higher risk
                risk_factors.push("Query modification may change behavior".to_string());
                risk_factors.push("Requires application deployment".to_string());
                RiskLevel::High
            }

            IssueType::HighVolumeQuery => {
                risk_factors.push("High traffic - impact is multiplied".to_string());
                RiskLevel::Medium
            }

            // Connection-related issues - these are monitoring recommendations, not auto-applicable
            IssueType::IdleInTransaction => {
                risk_factors
                    .push("Requires application-level connection handling fixes".to_string());
                mitigation_steps
                    .push("Review application connection pooling configuration".to_string());
                RiskLevel::High
            }

            IssueType::ConnectionPoolExhaustion => {
                risk_factors.push("May require infrastructure changes".to_string());
                mitigation_steps
                    .push("Increase max_connections or optimize connection usage".to_string());
                RiskLevel::Medium
            }

            IssueType::LockContention => {
                risk_factors.push("Requires query or transaction optimization".to_string());
                mitigation_steps
                    .push("Review blocking queries and transaction duration".to_string());
                RiskLevel::High
            }
        };

        let rollback_complexity = match rec.issue_type {
            IssueType::MissingIndex => RollbackComplexity::Simple, // DROP INDEX
            IssueType::UnusedIndex => RollbackComplexity::Moderate, // Recreate index
            IssueType::SlowQuery => RollbackComplexity::Irreversible, // Code change
            IssueType::HighVolumeQuery => RollbackComplexity::Moderate,
            // Connection-related issues don't have database-level rollback
            IssueType::IdleInTransaction => RollbackComplexity::Irreversible, // Application change
            IssueType::ConnectionPoolExhaustion => RollbackComplexity::Irreversible, // Config change
            IssueType::LockContention => RollbackComplexity::Irreversible, // Application/query change
        };

        RiskAssessment {
            risk_level,
            risk_factors,
            mitigation_steps,
            rollback_complexity,
        }
    }

    fn determine_strategy(&self, rec: &Recommendation) -> ApplyStrategy {
        match rec.issue_type {
            IssueType::MissingIndex => {
                // Parse migration SQL to extract index details
                let (index_name, table_name, columns) = self.parse_create_index(&rec.migration_sql);

                ApplyStrategy::CreateIndexConcurrently {
                    index_name,
                    table_name,
                    columns,
                }
            }

            IssueType::UnusedIndex => {
                let (schema, index_name) = self.parse_drop_index(&rec.migration_sql);

                ApplyStrategy::DropIndex { index_name, schema }
            }

            IssueType::SlowQuery => {
                // Would require query rewriting logic
                ApplyStrategy::QueryRewrite {
                    original_query: rec.affected_query.clone().unwrap_or_default(),
                    optimized_query: String::new(),
                }
            }

            IssueType::HighVolumeQuery => {
                // Similar to slow query
                ApplyStrategy::QueryRewrite {
                    original_query: rec.affected_query.clone().unwrap_or_default(),
                    optimized_query: String::new(),
                }
            }

            // Connection-related issues - these would require configuration changes
            IssueType::IdleInTransaction
            | IssueType::ConnectionPoolExhaustion
            | IssueType::LockContention => {
                // These are monitoring/alerting recommendations, not auto-applicable
                // Return a configuration change strategy as a placeholder
                ApplyStrategy::ConfigurationChange {
                    parameter: "connection_monitoring".to_string(),
                    current_value: "N/A".to_string(),
                    new_value: "N/A".to_string(),
                }
            }
        }
    }

    fn estimate_duration(&self, _rec: &Recommendation, strategy: &ApplyStrategy) -> u64 {
        match strategy {
            ApplyStrategy::CreateIndexConcurrently { table_name, .. } => {
                // Estimate based on table size
                let table_size_mb = self.estimate_table_size(table_name);

                // Rough heuristic: 1 second per 100MB
                ((table_size_mb / 100.0) as u64).max(5).min(600) // 5s to 10min
            }

            ApplyStrategy::DropIndex { .. } => {
                5 // Usually instant, buffer 5s
            }

            ApplyStrategy::QueryRewrite { .. } => {
                0 // Application-level change
            }

            ApplyStrategy::ConfigurationChange { .. } => {
                30 // Config reload
            }
        }
    }

    fn can_auto_apply(&self, rec: &Recommendation, risk: &RiskAssessment) -> bool {
        // Check if auto-apply is enabled
        if !self.config.enabled {
            return false;
        }

        // Check risk level
        if risk.risk_level > self.config.max_risk_level {
            return false;
        }

        // Check savings threshold
        if let Some(savings) = rec.estimated_cost_savings_usd_monthly {
            if savings < self.config.min_savings_threshold {
                return false;
            }
        } else {
            return false;
        }

        // Check rollback complexity
        if risk.rollback_complexity == RollbackComplexity::Irreversible {
            return false;
        }

        true
    }

    fn requires_manual_approval(&self, rec: &Recommendation, risk: &RiskAssessment) -> bool {
        // Always require approval for high/critical risk
        if risk.risk_level >= RiskLevel::High {
            return true;
        }

        // Check config for specific types
        self.config
            .require_manual_approval
            .contains(&rec.issue_type)
    }

    // Helper methods
    fn estimate_table_size(&self, _table: &str) -> f64 {
        // TODO: Query actual table size from database
        1000.0 // Placeholder: 1000MB
    }

    fn parse_create_index(&self, sql: &str) -> (String, String, Vec<String>) {
        // Simple parsing of CREATE INDEX statement
        // Example: CREATE INDEX CONCURRENTLY idx_users_email ON users(email);

        let sql_upper = sql.to_uppercase();

        // Extract index name
        let index_name = if let Some(idx_pos) = sql_upper.find("INDEX") {
            let after_index = sql[idx_pos + 5..].trim();
            let name_part =
                if let Some(concurrently_pos) = after_index.to_uppercase().find("CONCURRENTLY") {
                    &after_index[concurrently_pos + 12..]
                } else {
                    after_index
                };

            name_part
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("idx_placeholder")
                .to_string()
        } else {
            "idx_placeholder".to_string()
        };

        // Extract table name
        let table_name = if let Some(on_pos) = sql_upper.find(" ON ") {
            let after_on = &sql[on_pos + 4..].trim();
            after_on
                .split('(')
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string()
        } else {
            "unknown".to_string()
        };

        // Extract columns
        let columns = if let Some(start) = sql.find('(') {
            if let Some(end) = sql.find(')') {
                sql[start + 1..end]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect()
            } else {
                vec!["unknown".to_string()]
            }
        } else {
            vec!["unknown".to_string()]
        };

        (index_name, table_name, columns)
    }

    fn parse_drop_index(&self, sql: &str) -> (String, String) {
        // Example: DROP INDEX IF EXISTS public.idx_users_email;
        // or: DROP INDEX idx_users_email;

        let sql_upper = sql.to_uppercase();

        // Find the index name (after EXISTS or INDEX)
        let index_part = if let Some(exists_pos) = sql_upper.find("EXISTS") {
            sql[exists_pos + 6..].trim()
        } else if let Some(index_pos) = sql_upper.find("INDEX") {
            sql[index_pos + 5..].trim()
        } else {
            sql.trim()
        };

        // Remove semicolon
        let index_part = index_part.trim_end_matches(';').trim();

        // Split schema.index_name
        if let Some(dot_pos) = index_part.find('.') {
            let schema = index_part[..dot_pos].to_string();
            let index_name = index_part[dot_pos + 1..].to_string();
            (schema, index_name)
        } else {
            ("public".to_string(), index_part.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_create_index() {
        let config = AutoApplyConfig::default();
        let filter = SafetyFilter::new(config);

        let sql = "CREATE INDEX CONCURRENTLY idx_users_email ON users(email);";
        let (index_name, table_name, columns) = filter.parse_create_index(sql);

        assert_eq!(index_name, "idx_users_email");
        assert_eq!(table_name, "users");
        assert_eq!(columns, vec!["email"]);
    }

    #[test]
    fn test_parse_drop_index() {
        let config = AutoApplyConfig::default();
        let filter = SafetyFilter::new(config);

        let sql = "DROP INDEX IF EXISTS public.idx_users_email;";
        let (schema, index_name) = filter.parse_drop_index(sql);

        assert_eq!(schema, "public");
        assert_eq!(index_name, "idx_users_email");
    }

    #[test]
    fn test_risk_assessment_missing_index() {
        let config = AutoApplyConfig {
            enabled: true, // Enable auto-apply for test
            ..Default::default()
        };
        let filter = SafetyFilter::new(config);

        let rec = Recommendation {
            id: "test-1".to_string(),
            title: "Add missing index".to_string(),
            description: "Test".to_string(),
            issue_type: IssueType::MissingIndex,
            severity: Severity::High,
            affected_queries_count: Some(100),
            estimated_time_savings_ms: Some(5000.0),
            estimated_cost_savings_usd_monthly: Some(100.0),
            migration_sql: "CREATE INDEX CONCURRENTLY idx_test ON test(col);".to_string(),
            rollback_sql: Some("DROP INDEX idx_test;".to_string()),
            safety_notes: "Safe".to_string(),
            affected_table: Some("test".to_string()),
            affected_query: None,
        };

        let applicable = filter.assess(&rec);

        assert_eq!(applicable.risk_assessment.risk_level, RiskLevel::Low);
        assert!(applicable.can_auto_apply);
    }

    #[test]
    fn test_blocks_high_risk() {
        let config = AutoApplyConfig {
            max_risk_level: RiskLevel::Low,
            ..Default::default()
        };
        let filter = SafetyFilter::new(config);

        let rec = Recommendation {
            id: "test-2".to_string(),
            title: "Optimize slow query".to_string(),
            description: "Test".to_string(),
            issue_type: IssueType::SlowQuery,
            severity: Severity::High,
            affected_queries_count: Some(100),
            estimated_time_savings_ms: Some(5000.0),
            estimated_cost_savings_usd_monthly: Some(100.0),
            migration_sql: "-- Rewrite query".to_string(),
            rollback_sql: None,
            safety_notes: "Requires code changes".to_string(),
            affected_table: None,
            affected_query: Some("SELECT * FROM users".to_string()),
        };

        let applicable = filter.assess(&rec);

        assert_eq!(applicable.risk_assessment.risk_level, RiskLevel::High);
        assert!(!applicable.can_auto_apply);
        assert!(applicable.requires_approval);
    }
}
