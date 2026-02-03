//! EXPLAIN plan execution and parsing.
//!
//! This module handles:
//! - Running EXPLAIN (FORMAT JSON) against PostgreSQL
//! - Parsing the JSON output into structured PlanNode trees
//! - Extracting key metrics (cost, rows, scan types)

use super::models::{ExplainPlan, PlanNode};
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::time::Duration;
use tracing::debug;

/// Default timeout for EXPLAIN queries (30 seconds)
const DEFAULT_EXPLAIN_TIMEOUT_SECS: u64 = 30;

/// Executor for running EXPLAIN queries
pub struct ExplainExecutor {
    pool: PgPool,
    timeout: Duration,
}

impl ExplainExecutor {
    /// Create a new EXPLAIN executor
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            timeout: Duration::from_secs(DEFAULT_EXPLAIN_TIMEOUT_SECS),
        }
    }

    /// Create with custom timeout
    pub fn with_timeout(pool: PgPool, timeout: Duration) -> Self {
        Self { pool, timeout }
    }

    /// Run EXPLAIN (FORMAT JSON) for a query
    ///
    /// Returns the parsed ExplainPlan or an error.
    /// Note: This does NOT execute the query, only plans it.
    pub async fn explain(&self, query: &str) -> Result<ExplainPlan> {
        // Set statement timeout for safety
        let timeout_ms = self.timeout.as_millis() as i64;
        sqlx::query(&format!("SET LOCAL statement_timeout = {}", timeout_ms))
            .execute(&self.pool)
            .await
            .context("Failed to set statement timeout")?;

        let explain_query = format!("EXPLAIN (FORMAT JSON, COSTS, VERBOSE) {}", query);
        debug!("Running EXPLAIN: {}", explain_query);

        let row = sqlx::query(&explain_query)
            .fetch_one(&self.pool)
            .await
            .context("EXPLAIN query failed")?;

        // PostgreSQL returns EXPLAIN JSON as a single text column
        let plan_json: String = row.try_get(0).context("Failed to get EXPLAIN result")?;

        PlanParser::parse_explain_json(&plan_json)
    }

    /// Run EXPLAIN ANALYZE (actually executes the query!)
    ///
    /// WARNING: This executes the query. Use with caution.
    /// Only use for SELECT queries in validation scenarios.
    pub async fn explain_analyze(&self, query: &str) -> Result<ExplainPlan> {
        // Validate this is a SELECT query to prevent accidental mutations
        let query_upper = query.trim().to_uppercase();
        if !query_upper.starts_with("SELECT") && !query_upper.starts_with("WITH") {
            anyhow::bail!("EXPLAIN ANALYZE only allowed for SELECT/WITH queries");
        }

        let timeout_ms = self.timeout.as_millis() as i64;
        sqlx::query(&format!("SET LOCAL statement_timeout = {}", timeout_ms))
            .execute(&self.pool)
            .await
            .context("Failed to set statement timeout")?;

        let explain_query = format!("EXPLAIN (ANALYZE, FORMAT JSON, COSTS, VERBOSE) {}", query);
        debug!("Running EXPLAIN ANALYZE: {}", explain_query);

        let row = sqlx::query(&explain_query)
            .fetch_one(&self.pool)
            .await
            .context("EXPLAIN ANALYZE query failed")?;

        let plan_json: String = row.try_get(0).context("Failed to get EXPLAIN result")?;

        PlanParser::parse_explain_json(&plan_json)
    }

    /// Check if a query can be explained (is a valid DML query)
    pub fn can_explain(query: &str) -> bool {
        let query_upper = query.trim().to_uppercase();
        query_upper.starts_with("SELECT")
            || query_upper.starts_with("INSERT")
            || query_upper.starts_with("UPDATE")
            || query_upper.starts_with("DELETE")
            || query_upper.starts_with("WITH")
    }
}

/// Parser for EXPLAIN JSON output
pub struct PlanParser;

impl PlanParser {
    /// Parse EXPLAIN (FORMAT JSON) output into an ExplainPlan
    pub fn parse_explain_json(json_str: &str) -> Result<ExplainPlan> {
        let json: serde_json::Value =
            serde_json::from_str(json_str).context("Failed to parse EXPLAIN JSON")?;

        // EXPLAIN JSON is an array with one element containing the plan
        let plan_wrapper = json
            .as_array()
            .and_then(|arr| arr.first())
            .context("Invalid EXPLAIN JSON format: expected array")?;

        let plan_obj = plan_wrapper
            .get("Plan")
            .context("No 'Plan' key in EXPLAIN output")?;

        let root_node = Self::parse_plan_node(plan_obj)?;

        // Extract timing info if available (from ANALYZE)
        let planning_time_ms = plan_wrapper.get("Planning Time").and_then(|v| v.as_f64());
        let execution_time_ms = plan_wrapper.get("Execution Time").and_then(|v| v.as_f64());

        Ok(ExplainPlan {
            raw_json: json,
            total_cost: root_node.total_cost,
            estimated_rows: root_node.plan_rows,
            root_node,
            planning_time_ms,
            execution_time_ms,
        })
    }

    /// Parse a single plan node from JSON
    fn parse_plan_node(json: &serde_json::Value) -> Result<PlanNode> {
        let node_type = json
            .get("Node Type")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let relation_name = json
            .get("Relation Name")
            .and_then(|v| v.as_str())
            .map(String::from);

        let index_name = json
            .get("Index Name")
            .and_then(|v| v.as_str())
            .map(String::from);

        let index_cond = json
            .get("Index Cond")
            .and_then(|v| v.as_str())
            .map(String::from);

        let filter = json
            .get("Filter")
            .and_then(|v| v.as_str())
            .map(String::from);

        let startup_cost = json
            .get("Startup Cost")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let total_cost = json
            .get("Total Cost")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let plan_rows = json
            .get("Plan Rows")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let plan_width = json.get("Plan Width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        // Parse actual values if present (from ANALYZE)
        let actual_rows = json.get("Actual Rows").and_then(|v| v.as_f64());
        let actual_loops = json.get("Actual Loops").and_then(|v| v.as_i64());

        // Recursively parse child nodes
        let children = if let Some(plans) = json.get("Plans").and_then(|v| v.as_array()) {
            plans
                .iter()
                .filter_map(|p| Self::parse_plan_node(p).ok())
                .collect()
        } else {
            vec![]
        };

        Ok(PlanNode {
            node_type,
            relation_name,
            index_name,
            index_cond,
            filter,
            startup_cost,
            total_cost,
            plan_rows,
            plan_width,
            children,
            actual_rows,
            actual_loops,
        })
    }

    /// Extract all index names used in a plan
    pub fn extract_index_names(plan: &ExplainPlan) -> Vec<String> {
        plan.root_node.collect_indexes()
    }

    /// Check if a plan contains a sequential scan on a specific table
    pub fn has_seq_scan_on_table(plan: &ExplainPlan, table_name: &str) -> bool {
        Self::find_seq_scan_recursive(&plan.root_node, table_name)
    }

    fn find_seq_scan_recursive(node: &PlanNode, table_name: &str) -> bool {
        if node.node_type == "Seq Scan" {
            if let Some(ref rel) = node.relation_name {
                if rel == table_name {
                    return true;
                }
            }
        }
        node.children
            .iter()
            .any(|c| Self::find_seq_scan_recursive(c, table_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_explain() {
        assert!(ExplainExecutor::can_explain("SELECT * FROM users"));
        assert!(ExplainExecutor::can_explain("  select * from users  "));
        assert!(ExplainExecutor::can_explain("INSERT INTO users VALUES (1)"));
        assert!(ExplainExecutor::can_explain("UPDATE users SET name = 'x'"));
        assert!(ExplainExecutor::can_explain("DELETE FROM users"));
        assert!(ExplainExecutor::can_explain(
            "WITH cte AS (SELECT 1) SELECT * FROM cte"
        ));

        assert!(!ExplainExecutor::can_explain("CREATE TABLE foo (id int)"));
        assert!(!ExplainExecutor::can_explain("DROP TABLE foo"));
        assert!(!ExplainExecutor::can_explain("ALTER TABLE foo ADD col int"));
    }

    #[test]
    fn test_parse_explain_json_seq_scan() {
        let json = r#"[
          {
            "Plan": {
              "Node Type": "Seq Scan",
              "Relation Name": "users",
              "Startup Cost": 0.00,
              "Total Cost": 1234.56,
              "Plan Rows": 10000,
              "Plan Width": 100
            }
          }
        ]"#;

        let plan = PlanParser::parse_explain_json(json).unwrap();
        assert_eq!(plan.root_node.node_type, "Seq Scan");
        assert_eq!(plan.root_node.relation_name, Some("users".to_string()));
        assert!((plan.total_cost - 1234.56).abs() < 0.01);
        assert!((plan.estimated_rows - 10000.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_explain_json_index_scan() {
        let json = r#"[
          {
            "Plan": {
              "Node Type": "Index Scan",
              "Relation Name": "users",
              "Index Name": "idx_users_email",
              "Index Cond": "(email = 'test@example.com')",
              "Startup Cost": 0.29,
              "Total Cost": 8.31,
              "Plan Rows": 1,
              "Plan Width": 100
            }
          }
        ]"#;

        let plan = PlanParser::parse_explain_json(json).unwrap();
        assert_eq!(plan.root_node.node_type, "Index Scan");
        assert_eq!(
            plan.root_node.index_name,
            Some("idx_users_email".to_string())
        );
        assert!(plan.uses_index("idx_users_email"));
        assert!(!plan.uses_index("idx_other"));
    }

    #[test]
    fn test_parse_explain_json_nested() {
        let json = r#"[
          {
            "Plan": {
              "Node Type": "Nested Loop",
              "Startup Cost": 0.29,
              "Total Cost": 100.0,
              "Plan Rows": 10,
              "Plan Width": 200,
              "Plans": [
                {
                  "Node Type": "Index Scan",
                  "Relation Name": "orders",
                  "Index Name": "idx_orders_user_id",
                  "Startup Cost": 0.29,
                  "Total Cost": 8.31,
                  "Plan Rows": 1,
                  "Plan Width": 100
                },
                {
                  "Node Type": "Seq Scan",
                  "Relation Name": "products",
                  "Startup Cost": 0.0,
                  "Total Cost": 50.0,
                  "Plan Rows": 100,
                  "Plan Width": 100
                }
              ]
            }
          }
        ]"#;

        let plan = PlanParser::parse_explain_json(json).unwrap();
        assert_eq!(plan.root_node.node_type, "Nested Loop");
        assert_eq!(plan.root_node.children.len(), 2);

        // Check index usage
        assert!(plan.uses_index("idx_orders_user_id"));
        assert!(!plan.uses_index("idx_products"));

        // Check seq scan detection
        assert!(PlanParser::has_seq_scan_on_table(&plan, "products"));
        assert!(!PlanParser::has_seq_scan_on_table(&plan, "orders"));
    }

    #[test]
    fn test_parse_explain_analyze() {
        let json = r#"[
          {
            "Plan": {
              "Node Type": "Seq Scan",
              "Relation Name": "users",
              "Startup Cost": 0.00,
              "Total Cost": 1234.56,
              "Plan Rows": 10000,
              "Plan Width": 100,
              "Actual Rows": 9876,
              "Actual Loops": 1
            },
            "Planning Time": 0.123,
            "Execution Time": 45.678
          }
        ]"#;

        let plan = PlanParser::parse_explain_json(json).unwrap();
        assert_eq!(plan.root_node.actual_rows, Some(9876.0));
        assert_eq!(plan.root_node.actual_loops, Some(1));
        assert!((plan.planning_time_ms.unwrap() - 0.123).abs() < 0.001);
        assert!((plan.execution_time_ms.unwrap() - 45.678).abs() < 0.001);
    }

    #[test]
    fn test_collect_indexes() {
        let json = r#"[
          {
            "Plan": {
              "Node Type": "Nested Loop",
              "Startup Cost": 0.0,
              "Total Cost": 100.0,
              "Plan Rows": 10,
              "Plan Width": 200,
              "Plans": [
                {
                  "Node Type": "Index Scan",
                  "Index Name": "idx_a",
                  "Startup Cost": 0.0,
                  "Total Cost": 10.0,
                  "Plan Rows": 1,
                  "Plan Width": 100
                },
                {
                  "Node Type": "Index Only Scan",
                  "Index Name": "idx_b",
                  "Startup Cost": 0.0,
                  "Total Cost": 10.0,
                  "Plan Rows": 1,
                  "Plan Width": 100
                }
              ]
            }
          }
        ]"#;

        let plan = PlanParser::parse_explain_json(json).unwrap();
        let indexes = PlanParser::extract_index_names(&plan);
        assert!(indexes.contains(&"idx_a".to_string()));
        assert!(indexes.contains(&"idx_b".to_string()));
        assert_eq!(indexes.len(), 2);
    }
}
