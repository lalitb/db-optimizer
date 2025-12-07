use crate::analyzer::HealthCalculator;
use crate::collector::DatabaseCollector;
use crate::config::AnalysisConfig;
use crate::fleet::models::{DatabaseInstance, FleetSummary, InstanceMetrics, InstanceStatus};
use crate::models::ReportSummary;
use crate::recommendations::RecommendationEngine;
use crate::remote_agent::client::AgentClient;
use anyhow::Result;
use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct FleetManager {
    instances: Arc<RwLock<HashMap<String, DatabaseInstance>>>,
}

impl FleetManager {
    pub fn new() -> Self {
        Self {
            instances: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_instance(&self, instance: DatabaseInstance) -> Result<String> {
        let mut instances = self.instances.write().await;
        let id = instance.id.clone();
        instances.insert(id.clone(), instance);
        Ok(id)
    }

    pub async fn remove_instance(&self, id: &str) -> Result<bool> {
        let mut instances = self.instances.write().await;
        Ok(instances.remove(id).is_some())
    }

    pub async fn get_instance(&self, id: &str) -> Option<DatabaseInstance> {
        let instances = self.instances.read().await;
        instances.get(id).cloned()
    }

    pub async fn list_instances(&self) -> Vec<DatabaseInstance> {
        let instances = self.instances.read().await;
        instances.values().cloned().collect()
    }

    pub async fn get_summary(&self) -> FleetSummary {
        let instances = self.instances.read().await;
        let total = instances.len();
        let mut healthy = 0;
        let mut warning = 0;
        let mut critical = 0;
        let mut unreachable = 0;
        let mut total_score = 0.0;
        let mut scored_count = 0;

        for instance in instances.values() {
            match instance.status {
                InstanceStatus::Healthy => healthy += 1,
                InstanceStatus::Warning => warning += 1,
                InstanceStatus::Critical => critical += 1,
                InstanceStatus::Unreachable => unreachable += 1,
                InstanceStatus::Unknown => {}
            }

            if let Some(score) = instance.health_score {
                total_score += score as f64;
                scored_count += 1;
            }
        }

        FleetSummary {
            total_databases: total,
            healthy_count: healthy,
            warning_count: warning,
            critical_count: critical,
            unreachable_count: unreachable,
            average_health_score: if scored_count > 0 {
                Some(total_score / scored_count as f64)
            } else {
                None
            },
        }
    }

    /// Check health of a specific instance
    pub async fn check_instance_health(&self, id: &str) -> Result<DatabaseInstance> {
        let instance_opt = {
            let instances = self.instances.read().await;
            instances.get(id).cloned()
        };

        let mut instance = match instance_opt {
            Some(i) => i,
            None => anyhow::bail!("Instance not found"),
        };
        // Connect and analyze
        match self
            .analyze_database(&instance.connection_string, instance.agent_url.as_deref())
            .await
        {
            Ok(analysis) => {
                instance.health_score = Some(analysis.score);
                instance.version = Some(analysis.version);
                instance.last_check_at = Some(Utc::now());
                instance.metrics = Some(analysis.metrics);
                instance.summary = Some(analysis.summary);

                instance.status = if analysis.score >= 90 {
                    InstanceStatus::Healthy
                } else if analysis.score >= 70 {
                    InstanceStatus::Warning
                } else {
                    InstanceStatus::Critical
                };
            }
            Err(e) => {
                tracing::error!("Failed to check health for {}: {}", instance.name, e);
                instance.status = InstanceStatus::Unreachable;
                instance.last_check_at = Some(Utc::now());
            }
        }

        // Update in map
        {
            let mut instances = self.instances.write().await;
            instances.insert(instance.id.clone(), instance.clone());
        }

        Ok(instance)
    }

    /// Run health check on all instances
    pub async fn check_all_instances(&self) {
        let ids: Vec<String> = {
            let instances = self.instances.read().await;
            instances.keys().cloned().collect()
        };

        for id in ids {
            if let Err(e) = self.check_instance_health(&id).await {
                tracing::error!("Failed to check instance {}: {}", id, e);
            }
        }
    }

    async fn analyze_database(
        &self,
        connection_string: &str,
        agent_url: Option<&str>,
    ) -> Result<InstanceAnalysis> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(connection_string)
            .await?;

        let collector = DatabaseCollector::new(pool);

        // Collect data
        let tables = collector.collect_table_info().await?;
        let indexes = collector.collect_index_info().await?;
        let connection_stats = collector.collect_connection_stats().await.ok();
        let connections = collector.collect_connections().await.ok();
        let queries = collector.collect_query_stats(50).await?; // Top 50 queries
        let (_, version) = collector.get_database_info().await?;

        // Collect eBPF data if agent is configured
        let ebpf_lock_summary = if let Some(url) = agent_url {
            match AgentClient::new(url.to_string()).get_metrics().await {
                Ok(metrics) => metrics.lock_summary,
                Err(e) => {
                    tracing::warn!("Failed to fetch agent metrics from {}: {}", url, e);
                    None
                }
            }
        } else {
            None
        };

        // Calculate health score
        let total_db_size_mb: f64 = tables.iter().map(|t| t.total_size_mb).sum();
        let score = HealthCalculator::calculate(
            &tables,
            &indexes,
            connection_stats.as_ref(),
            total_db_size_mb,
        );

        // Run recommendation engine to get summary
        let engine = RecommendationEngine::new().with_analysis_config(AnalysisConfig::default());

        let recommendations = engine.generate_recommendations(
            &queries,
            &tables,
            &indexes,
            connection_stats.as_ref(),
            connections.as_deref(),
        );

        let summary = RecommendationEngine::generate_summary(&recommendations);

        // Build metrics
        let metrics = InstanceMetrics {
            total_size_mb: total_db_size_mb,
            table_count: tables.len(),
            index_count: indexes.len(),
            active_connections: connection_stats
                .as_ref()
                .map(|s| s.total_connections)
                .unwrap_or(0),
            max_connections: connection_stats
                .as_ref()
                .map(|s| s.max_connections)
                .unwrap_or(0),
            connection_usage_pct: connection_stats
                .as_ref()
                .map(|s| s.usage_percentage())
                .unwrap_or(0.0),
            idle_in_transaction_count: connection_stats
                .as_ref()
                .map(|s| s.idle_in_transaction_count)
                .unwrap_or(0),
            locked_connections_count: connection_stats
                .as_ref()
                .map(|s| s.locked_connections_count)
                .unwrap_or(0),
            ebpf_lock_summary,
        };

        Ok(InstanceAnalysis {
            score,
            version,
            metrics,
            summary,
        })
    }
}

struct InstanceAnalysis {
    score: u8,
    version: String,
    metrics: InstanceMetrics,
    summary: ReportSummary,
}
