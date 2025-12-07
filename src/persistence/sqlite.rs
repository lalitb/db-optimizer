// src/persistence/sqlite.rs - SQLite persistence backend

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::str::FromStr;

use super::{
    schema, BaselineSnapshot, BaselineType, ChangeEvent, ChangeEventType, PersistenceStore,
    QuerySample, RecommendationRecord, RecommendationStatus, WinSummary,
};

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Create a new SQLite store
    pub async fn new(path: &str) -> Result<Self> {
        // Expand tilde in path
        let expanded_path = if path.starts_with("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            path.replacen("~", &home, 1)
        } else {
            path.to_string()
        };

        // Create parent directory if it doesn't exist
        if let Some(parent) = std::path::Path::new(&expanded_path).parent() {
            std::fs::create_dir_all(parent).context("Failed to create persistence directory")?;
        }

        // Create connection options
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", expanded_path))?
            .create_if_missing(true);

        // Create connection pool
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("Failed to connect to SQLite database")?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl PersistenceStore for SqliteStore {
    async fn initialize(&self) -> Result<()> {
        // Execute schema
        let schema_sql = schema::get_schema();

        // Split by semicolon and execute each statement
        for statement in schema_sql.split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement)
                    .execute(&self.pool)
                    .await
                    .context("Failed to execute schema statement")?;
            }
        }

        tracing::info!("SQLite persistence store initialized");
        Ok(())
    }

    async fn store_query_sample(&self, sample: &QuerySample) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO query_samples (
                captured_at, query_id, query_text,
                mean_exec_time_ms, p95_exec_time_ms, p99_exec_time_ms,
                calls_per_min, total_calls,
                cpu_time_us, io_wait_time_us, page_cache_misses, context_switches
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(query_id, captured_at) DO UPDATE SET
                query_text = excluded.query_text,
                mean_exec_time_ms = excluded.mean_exec_time_ms,
                p95_exec_time_ms = excluded.p95_exec_time_ms,
                p99_exec_time_ms = excluded.p99_exec_time_ms,
                calls_per_min = excluded.calls_per_min,
                total_calls = excluded.total_calls,
                cpu_time_us = excluded.cpu_time_us,
                io_wait_time_us = excluded.io_wait_time_us,
                page_cache_misses = excluded.page_cache_misses,
                context_switches = excluded.context_switches
            "#,
        )
        .bind(sample.captured_at)
        .bind(sample.query_id)
        .bind(&sample.query_text)
        .bind(sample.mean_exec_time_ms)
        .bind(sample.p95_exec_time_ms)
        .bind(sample.p99_exec_time_ms)
        .bind(sample.calls_per_min)
        .bind(sample.total_calls)
        .bind(sample.cpu_time_us.map(|v| v as i64))
        .bind(sample.io_wait_time_us.map(|v| v as i64))
        .bind(sample.page_cache_misses.map(|v| v as i32))
        .bind(sample.context_switches.map(|v| v as i32))
        .execute(&self.pool)
        .await
        .context("Failed to store query sample")?;

        Ok(())
    }

    async fn store_recommendation(&self, rec: &RecommendationRecord) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO recommendations (
                id, created_at, issue_type, severity, title, description,
                estimated_savings_usd, status, applied_at, rolled_back_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                applied_at = excluded.applied_at,
                rolled_back_at = excluded.rolled_back_at
            "#,
        )
        .bind(&rec.id)
        .bind(rec.created_at)
        .bind(&rec.issue_type)
        .bind(&rec.severity)
        .bind(&rec.title)
        .bind(&rec.description)
        .bind(rec.estimated_savings_usd)
        .bind(rec.status.as_str())
        .bind(rec.applied_at)
        .bind(rec.rolled_back_at)
        .execute(&self.pool)
        .await
        .context("Failed to store recommendation")?;

        Ok(())
    }

    async fn update_recommendation_status(
        &self,
        id: &str,
        status: RecommendationStatus,
    ) -> Result<()> {
        let now = Utc::now();

        // Update status and set appropriate timestamp
        let query = match status {
            RecommendationStatus::Applied => {
                sqlx::query("UPDATE recommendations SET status = ?, applied_at = ? WHERE id = ?")
                    .bind(status.as_str())
                    .bind(now)
                    .bind(id)
            }
            RecommendationStatus::RolledBack => sqlx::query(
                "UPDATE recommendations SET status = ?, rolled_back_at = ? WHERE id = ?",
            )
            .bind(status.as_str())
            .bind(now)
            .bind(id),
            _ => sqlx::query("UPDATE recommendations SET status = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(id),
        };

        query
            .execute(&self.pool)
            .await
            .context("Failed to update recommendation status")?;

        Ok(())
    }

    async fn store_change_event(&self, event: &ChangeEvent) -> Result<()> {
        let metadata_str = serde_json::to_string(&event.metadata)?;

        sqlx::query(
            "INSERT INTO change_events (recommendation_id, event_type, timestamp, metadata) VALUES (?, ?, ?, ?)"
        )
        .bind(&event.recommendation_id)
        .bind(event.event_type.as_str())
        .bind(event.timestamp)
        .bind(metadata_str)
        .execute(&self.pool)
        .await
        .context("Failed to store change event")?;

        Ok(())
    }

    async fn store_baseline(&self, baseline: &BaselineSnapshot) -> Result<()> {
        let metrics_str = serde_json::to_string(&baseline.metrics)?;

        sqlx::query(
            "INSERT INTO baselines (recommendation_id, baseline_type, captured_at, metrics) VALUES (?, ?, ?, ?)"
        )
        .bind(&baseline.recommendation_id)
        .bind(baseline.baseline_type.as_str())
        .bind(baseline.captured_at)
        .bind(metrics_str)
        .execute(&self.pool)
        .await
        .context("Failed to store baseline")?;

        Ok(())
    }

    async fn get_query_samples(&self, query_id: i64, limit: usize) -> Result<Vec<QuerySample>> {
        let rows = sqlx::query(
            r#"
            SELECT captured_at, query_id, query_text,
                   mean_exec_time_ms, p95_exec_time_ms, p99_exec_time_ms,
                   calls_per_min, total_calls,
                   cpu_time_us, io_wait_time_us, page_cache_misses, context_switches
            FROM query_samples
            WHERE query_id = ?
            ORDER BY captured_at DESC
            LIMIT ?
            "#,
        )
        .bind(query_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch query samples")?;

        let samples = rows
            .into_iter()
            .map(|row| QuerySample {
                captured_at: row.get("captured_at"),
                query_id: row.get("query_id"),
                query_text: row.get("query_text"),
                mean_exec_time_ms: row.get("mean_exec_time_ms"),
                p95_exec_time_ms: row.get("p95_exec_time_ms"),
                p99_exec_time_ms: row.get("p99_exec_time_ms"),
                calls_per_min: row.get("calls_per_min"),
                total_calls: row.get("total_calls"),
                cpu_time_us: row.get::<Option<i64>, _>("cpu_time_us").map(|v| v as u64),
                io_wait_time_us: row
                    .get::<Option<i64>, _>("io_wait_time_us")
                    .map(|v| v as u64),
                page_cache_misses: row
                    .get::<Option<i32>, _>("page_cache_misses")
                    .map(|v| v as u32),
                context_switches: row
                    .get::<Option<i32>, _>("context_switches")
                    .map(|v| v as u32),
            })
            .collect();

        Ok(samples)
    }

    async fn get_recommendations_by_status(
        &self,
        status: RecommendationStatus,
    ) -> Result<Vec<RecommendationRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, created_at, issue_type, severity, title, description,
                   estimated_savings_usd, status, applied_at, rolled_back_at
            FROM recommendations
            WHERE status = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch recommendations")?;

        let recommendations = rows
            .into_iter()
            .map(|row| RecommendationRecord {
                id: row.get("id"),
                created_at: row.get("created_at"),
                issue_type: row.get("issue_type"),
                severity: row.get("severity"),
                title: row.get("title"),
                description: row.get("description"),
                estimated_savings_usd: row.get("estimated_savings_usd"),
                status: RecommendationStatus::from_str(&row.get::<String, _>("status"))
                    .unwrap_or(RecommendationStatus::Pending),
                applied_at: row.get("applied_at"),
                rolled_back_at: row.get("rolled_back_at"),
            })
            .collect();

        Ok(recommendations)
    }

    async fn get_change_history(&self, recommendation_id: &str) -> Result<Vec<ChangeEvent>> {
        let rows = sqlx::query(
            "SELECT recommendation_id, event_type, timestamp, metadata FROM change_events WHERE recommendation_id = ? ORDER BY timestamp"
        )
        .bind(recommendation_id)
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch change history")?;

        let events = rows
            .into_iter()
            .filter_map(|row| {
                let event_type_str: String = row.get("event_type");
                let event_type = match event_type_str.as_str() {
                    "PROPOSED" => ChangeEventType::Proposed,
                    "APPLIED" => ChangeEventType::Applied,
                    "VERIFIED" => ChangeEventType::Verified,
                    "ROLLED_BACK" => ChangeEventType::RolledBack,
                    _ => return None,
                };

                let metadata_str: String = row.get("metadata");
                let metadata = serde_json::from_str(&metadata_str).ok()?;

                Some(ChangeEvent {
                    recommendation_id: row.get("recommendation_id"),
                    event_type,
                    timestamp: row.get("timestamp"),
                    metadata,
                })
            })
            .collect();

        Ok(events)
    }

    async fn get_baselines(
        &self,
        recommendation_id: &str,
    ) -> Result<(Option<BaselineSnapshot>, Option<BaselineSnapshot>)> {
        let rows = sqlx::query(
            "SELECT recommendation_id, baseline_type, captured_at, metrics FROM baselines WHERE recommendation_id = ?"
        )
        .bind(recommendation_id)
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch baselines")?;

        let mut pre_apply = None;
        let mut post_apply = None;

        for row in rows {
            let baseline_type_str: String = row.get("baseline_type");
            let baseline_type = match baseline_type_str.as_str() {
                "PRE_APPLY" => BaselineType::PreApply,
                "POST_APPLY" => BaselineType::PostApply,
                _ => continue,
            };

            let metrics_str: String = row.get("metrics");
            let metrics = serde_json::from_str(&metrics_str).unwrap_or_default();

            let snapshot = BaselineSnapshot {
                recommendation_id: row.get("recommendation_id"),
                baseline_type,
                captured_at: row.get("captured_at"),
                metrics,
            };

            match baseline_type {
                BaselineType::PreApply => pre_apply = Some(snapshot),
                BaselineType::PostApply => post_apply = Some(snapshot),
            }
        }

        Ok((pre_apply, post_apply))
    }

    async fn get_top_wins(&self, limit: usize) -> Result<Vec<WinSummary>> {
        let rows = sqlx::query(
            r#"
            SELECT 
                r.id as recommendation_id,
                r.title,
                r.issue_type,
                r.applied_at,
                r.estimated_savings_usd,
                COALESCE(
                    (SELECT json_extract(b.metrics, '$.actual_improvement_pct')
                     FROM baselines b
                     WHERE b.recommendation_id = r.id AND b.baseline_type = 'POST_APPLY'
                     LIMIT 1),
                    0
                ) as improvement_pct
            FROM recommendations r
            WHERE r.status = 'VERIFIED' AND r.applied_at IS NOT NULL
            ORDER BY improvement_pct DESC, r.estimated_savings_usd DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch top wins")?;

        let wins = rows
            .into_iter()
            .map(|row| WinSummary {
                recommendation_id: row.get("recommendation_id"),
                title: row.get("title"),
                issue_type: row.get("issue_type"),
                applied_at: row.get("applied_at"),
                improvement_pct: row.get::<f64, _>("improvement_pct"),
                estimated_savings_usd: row.get("estimated_savings_usd"),
            })
            .collect();

        Ok(wins)
    }

    async fn cleanup_old_records(&self, retention_days: u32) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);

        // Delete old query samples
        let result = sqlx::query("DELETE FROM query_samples WHERE captured_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .context("Failed to cleanup old query samples")?;

        let deleted = result.rows_affected();

        tracing::info!(
            "Cleaned up {} old query samples (retention: {} days)",
            deleted,
            retention_days
        );

        Ok(deleted)
    }
}
