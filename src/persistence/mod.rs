// src/persistence/mod.rs - Persistence layer for tracking history and trends

pub mod schema;
pub mod sqlite;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::config::PersistenceBackend;
use crate::models::{ApplyResult, QueryStats, Recommendation};

/// Trait for persistence backends
#[async_trait]
pub trait PersistenceStore: Send + Sync {
    /// Initialize the persistence store (create tables, run migrations, etc.)
    async fn initialize(&self) -> Result<()>;

    /// Store a query sample
    async fn store_query_sample(&self, sample: &QuerySample) -> Result<()>;

    /// Store a recommendation
    async fn store_recommendation(&self, rec: &RecommendationRecord) -> Result<()>;

    /// Update recommendation status
    async fn update_recommendation_status(
        &self,
        id: &str,
        status: RecommendationStatus,
    ) -> Result<()>;

    /// Store a change event (proposed, applied, verified, rolled back)
    async fn store_change_event(&self, event: &ChangeEvent) -> Result<()>;

    /// Store a baseline snapshot
    async fn store_baseline(&self, baseline: &BaselineSnapshot) -> Result<()>;

    /// Get query samples for a specific query ID
    async fn get_query_samples(&self, query_id: i64, limit: usize) -> Result<Vec<QuerySample>>;

    /// Get recommendations by status
    async fn get_recommendations_by_status(
        &self,
        status: RecommendationStatus,
    ) -> Result<Vec<RecommendationRecord>>;

    /// Get change history for a recommendation
    async fn get_change_history(&self, recommendation_id: &str) -> Result<Vec<ChangeEvent>>;

    /// Get baseline snapshots for a recommendation
    async fn get_baselines(
        &self,
        recommendation_id: &str,
    ) -> Result<(Option<BaselineSnapshot>, Option<BaselineSnapshot>)>;

    /// Get top wins (successful recommendations) with highest impact
    async fn get_top_wins(&self, limit: usize) -> Result<Vec<WinSummary>>;

    /// Clean up old records based on retention policy
    async fn cleanup_old_records(&self, retention_days: u32) -> Result<u64>;
}

/// Factory for creating persistence stores
pub async fn create_store(
    backend: PersistenceBackend,
    config: &crate::config::PersistenceConfig,
) -> Result<Box<dyn PersistenceStore>> {
    match backend {
        PersistenceBackend::None => Ok(Box::new(NullStore)),
        PersistenceBackend::Sqlite => {
            let store = sqlite::SqliteStore::new(&config.sqlite_path).await?;
            store.initialize().await?;
            Ok(Box::new(store))
        }
        PersistenceBackend::Postgres => {
            // TODO: Implement Postgres backend in Phase 4
            anyhow::bail!("Postgres persistence backend not yet implemented")
        }
    }
}

/// Query sample stored in persistence layer
#[derive(Debug, Clone)]
pub struct QuerySample {
    pub captured_at: DateTime<Utc>,
    pub query_id: i64,
    pub query_text: String,
    pub mean_exec_time_ms: f64,
    pub p95_exec_time_ms: f64,
    pub p99_exec_time_ms: f64,
    pub calls_per_min: f64,
    pub total_calls: i64,

    // eBPF metrics (optional)
    pub cpu_time_us: Option<u64>,
    pub io_wait_time_us: Option<u64>,
    pub page_cache_misses: Option<u32>,
    pub context_switches: Option<u32>,
}

impl QuerySample {
    /// Create from QueryStats
    pub fn from_query_stats(stats: &QueryStats) -> Self {
        Self {
            captured_at: Utc::now(),
            query_id: stats.queryid.unwrap_or(0),
            query_text: stats.query.clone(),
            mean_exec_time_ms: stats.mean_exec_time_ms,
            p95_exec_time_ms: stats.mean_exec_time_ms * 1.5, // Approximation
            p99_exec_time_ms: stats.mean_exec_time_ms * 2.0, // Approximation
            calls_per_min: stats.calls as f64 / 60.0,        // Rough estimate
            total_calls: stats.calls,
            cpu_time_us: None,
            io_wait_time_us: None,
            page_cache_misses: None,
            context_switches: None,
        }
    }
}

/// Recommendation record with status tracking
#[derive(Debug, Clone)]
pub struct RecommendationRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub issue_type: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub estimated_savings_usd: f64,
    pub status: RecommendationStatus,
    pub applied_at: Option<DateTime<Utc>>,
    pub rolled_back_at: Option<DateTime<Utc>>,
}

impl RecommendationRecord {
    /// Create from Recommendation
    pub fn from_recommendation(rec: &Recommendation) -> Self {
        Self {
            id: rec.id.clone(),
            created_at: Utc::now(),
            issue_type: format!("{:?}", rec.issue_type),
            severity: rec.severity.as_str().to_string(),
            title: rec.title.clone(),
            description: rec.description.clone(),
            estimated_savings_usd: rec.estimated_cost_savings_usd_monthly.unwrap_or(0.0),
            status: RecommendationStatus::Pending,
            applied_at: None,
            rolled_back_at: None,
        }
    }
}

/// Status of a recommendation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendationStatus {
    Pending,
    Applied,
    Verified,
    RolledBack,
    Rejected,
}

impl RecommendationStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "PENDING",
            Self::Applied => "APPLIED",
            Self::Verified => "VERIFIED",
            Self::RolledBack => "ROLLED_BACK",
            Self::Rejected => "REJECTED",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "PENDING" => Some(Self::Pending),
            "APPLIED" => Some(Self::Applied),
            "VERIFIED" => Some(Self::Verified),
            "ROLLED_BACK" => Some(Self::RolledBack),
            "REJECTED" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// Change event tracking the lifecycle of a recommendation
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    pub recommendation_id: String,
    pub event_type: ChangeEventType,
    pub timestamp: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeEventType {
    Proposed,
    Applied,
    Verified,
    RolledBack,
}

impl ChangeEventType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Proposed => "PROPOSED",
            Self::Applied => "APPLIED",
            Self::Verified => "VERIFIED",
            Self::RolledBack => "ROLLED_BACK",
        }
    }
}

impl ChangeEvent {
    pub fn new(recommendation_id: String, event_type: ChangeEventType) -> Self {
        Self {
            recommendation_id,
            event_type,
            timestamp: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Baseline snapshot (pre or post apply)
#[derive(Debug, Clone)]
pub struct BaselineSnapshot {
    pub recommendation_id: String,
    pub baseline_type: BaselineType,
    pub captured_at: DateTime<Utc>,
    pub metrics: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineType {
    PreApply,
    PostApply,
}

impl BaselineType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::PreApply => "PRE_APPLY",
            Self::PostApply => "POST_APPLY",
        }
    }
}

impl BaselineSnapshot {
    pub fn from_apply_result(result: &ApplyResult, baseline_type: BaselineType) -> Self {
        let metrics = match baseline_type {
            BaselineType::PreApply => serde_json::to_value(&result.baseline).unwrap_or_default(),
            BaselineType::PostApply => serde_json::to_value(&result.post_apply).unwrap_or_default(),
        };

        Self {
            recommendation_id: result.recommendation_id.clone(),
            baseline_type,
            captured_at: Utc::now(),
            metrics,
        }
    }
}

/// Summary of a successful optimization (win)
#[derive(Debug, Clone)]
pub struct WinSummary {
    pub recommendation_id: String,
    pub title: String,
    pub issue_type: String,
    pub applied_at: DateTime<Utc>,
    pub improvement_pct: f64,
    pub estimated_savings_usd: f64,
}

/// Null store that does nothing (for when persistence is disabled)
struct NullStore;

#[async_trait]
impl PersistenceStore for NullStore {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }
    async fn store_query_sample(&self, _sample: &QuerySample) -> Result<()> {
        Ok(())
    }
    async fn store_recommendation(&self, _rec: &RecommendationRecord) -> Result<()> {
        Ok(())
    }
    async fn update_recommendation_status(
        &self,
        _id: &str,
        _status: RecommendationStatus,
    ) -> Result<()> {
        Ok(())
    }
    async fn store_change_event(&self, _event: &ChangeEvent) -> Result<()> {
        Ok(())
    }
    async fn store_baseline(&self, _baseline: &BaselineSnapshot) -> Result<()> {
        Ok(())
    }
    async fn get_query_samples(&self, _query_id: i64, _limit: usize) -> Result<Vec<QuerySample>> {
        Ok(vec![])
    }
    async fn get_recommendations_by_status(
        &self,
        _status: RecommendationStatus,
    ) -> Result<Vec<RecommendationRecord>> {
        Ok(vec![])
    }
    async fn get_change_history(&self, _recommendation_id: &str) -> Result<Vec<ChangeEvent>> {
        Ok(vec![])
    }
    async fn get_baselines(
        &self,
        _recommendation_id: &str,
    ) -> Result<(Option<BaselineSnapshot>, Option<BaselineSnapshot>)> {
        Ok((None, None))
    }
    async fn get_top_wins(&self, _limit: usize) -> Result<Vec<WinSummary>> {
        Ok(vec![])
    }
    async fn cleanup_old_records(&self, _retention_days: u32) -> Result<u64> {
        Ok(0)
    }
}
