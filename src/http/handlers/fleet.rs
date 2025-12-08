use crate::fleet::{CreateInstanceRequest, DatabaseInstance, FleetSummary};
use crate::http::AppState;
use crate::http::{ApiError, ApiResult};
use axum::{
    extract::{Path, State},
    Json,
};

/// List all fleet instances
pub async fn list_instances(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<DatabaseInstance>>> {
    if let Some(manager) = &state.fleet_manager {
        let instances = manager.list_instances().await;
        Ok(Json(instances))
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// Get fleet summary
pub async fn get_summary(State(state): State<AppState>) -> ApiResult<Json<FleetSummary>> {
    if let Some(manager) = &state.fleet_manager {
        let summary = manager.get_summary().await;
        Ok(Json(summary))
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// Register a new database instance
pub async fn register_instance(
    State(state): State<AppState>,
    Json(req): Json<CreateInstanceRequest>,
) -> ApiResult<Json<DatabaseInstance>> {
    if let Some(manager) = &state.fleet_manager {
        let mut instance = DatabaseInstance::new(req.name, req.connection_string, req.environment);

        if let Some(tags) = req.tags {
            instance.tags = tags;
        }

        manager
            .add_instance(instance.clone())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        // Trigger initial health check in background
        let manager_clone = manager.clone();
        let id = instance.id.clone();
        tokio::spawn(async move {
            let _ = manager_clone.check_instance_health(&id).await;
        });

        Ok(Json(instance))
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// Get instance details
pub async fn get_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<DatabaseInstance>> {
    if let Some(manager) = &state.fleet_manager {
        if let Some(instance) = manager.get_instance(&id).await {
            Ok(Json(instance))
        } else {
            Err(ApiError::NotFound(format!("Instance {} not found", id)))
        }
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// Remove instance
pub async fn remove_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<bool>> {
    if let Some(manager) = &state.fleet_manager {
        let removed = manager
            .remove_instance(&id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(Json(removed))
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// Trigger health check for an instance
pub async fn check_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<DatabaseInstance>> {
    if let Some(manager) = &state.fleet_manager {
        let instance = manager
            .check_instance_health(&id)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(Json(instance))
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// Get detailed analysis for an instance (drill-down)
#[derive(serde::Serialize)]
pub struct InstanceDetailResponse {
    pub instance: DatabaseInstance,
    pub slow_queries: Vec<SlowQueryInfo>,
    pub recommendations: Vec<RecommendationInfo>,
    pub ai_insights: Vec<AiInsightInfo>,
}

#[derive(serde::Serialize)]
pub struct SlowQueryInfo {
    pub query_id: String,
    pub query_text: String,
    pub mean_time_ms: f64,
    pub calls: i64,
    pub total_time_ms: f64,
}

#[derive(serde::Serialize)]
pub struct RecommendationInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: String,
    pub estimated_savings_usd: Option<f64>,
    pub migration_sql: String,
}

#[derive(serde::Serialize)]
pub struct AiInsightInfo {
    pub id: String,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub recommendation: Option<String>,
}

/// GET /api/v1/fleet/:id/details - Get detailed drill-down for an instance
pub async fn get_instance_details(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<InstanceDetailResponse>> {
    if let Some(manager) = &state.fleet_manager {
        if let Some(instance) = manager.get_instance(&id).await {
            // Get slow queries from summary if available
            let slow_queries = vec![]; // TODO: Store queries in instance or fetch live

            // Get recommendations from summary
            let recommendations = vec![]; // TODO: Store recommendations in instance

            // Get AI insights from metrics
            let ai_insights = instance
                .metrics
                .as_ref()
                .and_then(|m| m.ai_insights.as_ref())
                .map(|insights| {
                    insights
                        .iter()
                        .map(|i| AiInsightInfo {
                            id: i.id.clone(),
                            category: format!("{:?}", i.category),
                            severity: format!("{:?}", i.severity),
                            title: i.title.clone(),
                            description: i.description.clone(),
                            recommendation: i.recommendation.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            Ok(Json(InstanceDetailResponse {
                instance,
                slow_queries,
                recommendations,
                ai_insights,
            }))
        } else {
            Err(ApiError::NotFound(format!("Instance {} not found", id)))
        }
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}

/// POST /api/v1/fleet/:id/ai-analyze - Trigger AI analysis for an instance
pub async fn ai_analyze_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    use crate::ai::{AiAdvisor, AiConfig};
    use crate::fleet::models::{AiInsight, InsightCategory, InsightSeverity};
    use chrono::Utc;
    use uuid::Uuid;

    if let Some(manager) = &state.fleet_manager {
        if let Some(mut instance) = manager.get_instance(&id).await {
            let ai_config = AiConfig::from_env();

            if !ai_config.enabled {
                return Err(ApiError::BadRequest(
                    "AI features not enabled. Set OPENAI_API_KEY or configure AI provider"
                        .to_string(),
                ));
            }

            // Build context from instance data
            let context = format!(
                "Database: {}\nEnvironment: {}\nHealth Score: {:?}\nSize: {} MB\nTables: {}\nIndexes: {}\n\
                Active Connections: {}\nIdle in Transaction: {}\nLock Contention: {}",
                instance.name,
                instance.environment,
                instance.health_score,
                instance.metrics.as_ref().map(|m| m.total_size_mb).unwrap_or(0.0),
                instance.metrics.as_ref().map(|m| m.table_count).unwrap_or(0),
                instance.metrics.as_ref().map(|m| m.index_count).unwrap_or(0),
                instance.metrics.as_ref().map(|m| m.active_connections).unwrap_or(0),
                instance.metrics.as_ref().map(|m| m.idle_in_transaction_count).unwrap_or(0),
                instance.metrics.as_ref().map(|m| m.locked_connections_count).unwrap_or(0),
            );

            let advisor = AiAdvisor::new(ai_config)
                .map_err(|e| ApiError::Internal(format!("Failed to initialize AI: {}", e)))?;

            let question = "Analyze this PostgreSQL database and provide 3-5 actionable insights for improving performance, reducing costs, or preventing issues. Focus on the most impactful recommendations.";

            let response = advisor
                .ask(question, &context)
                .await
                .map_err(|e| ApiError::Internal(format!("AI analysis failed: {}", e)))?;

            // Parse AI response into insights (simplified)
            let insight = AiInsight {
                id: Uuid::new_v4().to_string(),
                category: InsightCategory::Performance,
                severity: InsightSeverity::Info,
                title: "AI Performance Analysis".to_string(),
                description: response.clone(),
                recommendation: None,
                estimated_impact: None,
                created_at: Utc::now(),
            };

            // Update instance with AI insights
            if let Some(ref mut metrics) = instance.metrics {
                metrics.ai_insights = Some(vec![insight.clone()]);
            }

            // Save back to manager (would need a method for this)
            // manager.update_instance(instance).await?;

            Ok(Json(serde_json::json!({
                "success": true,
                "instance_id": id,
                "insights_generated": 1,
                "analysis": response,
            })))
        } else {
            Err(ApiError::NotFound(format!("Instance {} not found", id)))
        }
    } else {
        Err(ApiError::Internal(
            "Fleet manager not initialized".to_string(),
        ))
    }
}
