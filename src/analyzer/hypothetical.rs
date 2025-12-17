use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

/// Analyzer for "What-If" scenarios using hypopg
pub struct HypotheticalAnalyzer {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct HypotheticalResult {
    pub baseline_cost: f64,
    pub new_cost: f64,
    pub improvement_pct: f64,
    pub index_used: bool,
    pub plan_summary: String,
}

impl HypotheticalAnalyzer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Check if hypopg extension is available
    pub async fn is_available(&self) -> bool {
        let result = sqlx::query("SELECT 1 FROM pg_extension WHERE extname = 'hypopg'")
            .fetch_optional(&self.pool)
            .await;

        matches!(result, Ok(Some(_)))
    }

    /// Evaluate a hypothetical index
    ///
    /// This creates a virtual index, runs EXPLAIN, and compares the cost.
    /// It runs inside a transaction that is rolled back, so no changes persist.
    pub async fn evaluate_index(&self, query: &str, index_def: &str) -> Result<HypotheticalResult> {
        let mut tx = self.pool.begin().await?;

        // 1. Get baseline cost (without index)
        let baseline_cost = self.get_query_cost(&mut tx, query).await?;

        // 2. Create hypothetical index
        // hypopg_create_index returns the OID of the new index
        // We ignore the result, just need it created for this session
        sqlx::query("SELECT hypopg_create_index($1)")
            .bind(index_def)
            .execute(&mut *tx)
            .await
            .context("Failed to create hypothetical index. Is hypopg installed?")?;

        // 3. Get new cost (with index)
        let new_cost = self.get_query_cost(&mut tx, query).await?;

        // 4. Get plan to see if index was used
        let plan = self.get_explain_text(&mut tx, query).await?;

        // hypopg indexes usually show up with their name or as "Index Scan using <hypothetical>"
        // We can also check if the cost went down significantly
        let index_used = new_cost < baseline_cost && plan.to_lowercase().contains("index");

        // 5. Rollback to clean up
        tx.rollback().await?;

        let improvement_pct = if baseline_cost > 0.0 {
            ((baseline_cost - new_cost) / baseline_cost) * 100.0
        } else {
            0.0
        };

        Ok(HypotheticalResult {
            baseline_cost,
            new_cost,
            improvement_pct,
            index_used,
            plan_summary: truncate_plan(&plan, 500),
        })
    }

    async fn get_query_cost(&self, tx: &mut Transaction<'_, Postgres>, query: &str) -> Result<f64> {
        // Use EXPLAIN (FORMAT JSON) to get precise cost
        let explain_sql = format!("EXPLAIN (FORMAT JSON) {}", query);

        let row = sqlx::query(&explain_sql)
            .fetch_one(&mut **tx)
            .await
            .context("Failed to run EXPLAIN")?;

        let json_val: Value = row.try_get(0)?;

        // Parse: [ { "Plan": { "Total Cost": 123.45, ... } } ]
        if let Some(array) = json_val.as_array() {
            if let Some(first) = array.first() {
                if let Some(plan) = first.get("Plan") {
                    if let Some(cost) = plan.get("Total Cost") {
                        return cost.as_f64().context("Total Cost is not a number");
                    }
                }
            }
        }

        Ok(0.0)
    }

    async fn get_explain_text(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        query: &str,
    ) -> Result<String> {
        let explain_sql = format!("EXPLAIN {}", query);

        // Fetch all rows of EXPLAIN output
        let rows = sqlx::query(&explain_sql).fetch_all(&mut **tx).await?;

        let plan: String = rows
            .iter()
            .map(|r| r.get::<String, _>(0))
            .collect::<Vec<String>>()
            .join("\n");

        Ok(plan)
    }
}

fn truncate_plan(plan: &str, max_len: usize) -> String {
    if plan.len() <= max_len {
        plan.to_string()
    } else {
        format!("{}...", &plan[..max_len])
    }
}
