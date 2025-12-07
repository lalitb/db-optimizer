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
