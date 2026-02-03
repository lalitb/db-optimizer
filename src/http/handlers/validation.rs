//! HTTP handlers for index validation, apply, and rollback operations.
//!
//! Provides REST API endpoints for:
//! - Validating index recommendations with EXPLAIN proof
//! - Applying validated recommendations
//! - Rolling back applied indexes
//! - Viewing validation/apply status

use crate::analyzer::validation::{ProofGenerator, ValidationResult, ValidationStatus};
use crate::auto_apply::audit::{AuditLog, AuditEvent, AuditEventType};
use crate::auto_apply::state_machine::{
    RecommendationLifecycle, RecommendationState, RollbackReason,
};
use crate::auto_apply::{BlockReason, OperationGuard};
use crate::http::{
    error::{ApiError, ApiResult},
    models::ApiResponse,
};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Extended lifecycle with index metadata
#[derive(Debug, Clone)]
pub struct ExtendedLifecycle {
    pub lifecycle: RecommendationLifecycle,
    pub table_name: String,
    pub columns: Vec<String>,
    pub index_type: String,
    pub validation_result: Option<ValidationResult>,
}

/// Shared state for validation handlers
pub struct ValidationHandlerState {
    pub db_pool: PgPool,
    pub lifecycles: Arc<RwLock<std::collections::HashMap<String, ExtendedLifecycle>>>,
    pub audit_log: Arc<RwLock<AuditLog>>,
    /// Centralized operation guard for mode enforcement
    pub operation_guard: Arc<OperationGuard>,
}

/// Request to validate an index recommendation
#[derive(Debug, Deserialize)]
pub struct ValidateRequest {
    /// The query to validate (in normalized form)
    pub query: String,
    /// Table name
    pub table_name: String,
    /// Columns to index
    pub columns: Vec<String>,
    /// Optional: index type (btree, hash, gin, gist)
    #[serde(default = "default_index_type")]
    pub index_type: String,
    /// Optional: minimum cost improvement threshold (0.0-100.0 as percentage)
    pub min_improvement_pct: Option<f64>,
}

fn default_index_type() -> String {
    "btree".to_string()
}

/// Response from validation
#[derive(Debug, Serialize)]
pub struct ValidateResponse {
    /// Unique ID for this validation/recommendation
    pub recommendation_id: String,
    /// Current state
    pub state: String,
    /// Whether validation passed
    pub validated: bool,
    /// Human-readable reason
    pub reason: String,
    /// Cost improvement percentage (positive = better)
    pub cost_improvement_pct: Option<f64>,
    /// Whether index was used in the new plan
    pub index_used: bool,
    /// Candidate index name that was tested
    pub candidate_index_name: String,
    /// Production index name if applied
    pub production_index_name: String,
    /// SQL that would be executed to create the index
    pub create_index_sql: String,
    /// Plan diff summary
    pub diff_summary: Option<String>,
}

/// Request to apply a validated recommendation
#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    /// Recommendation ID from validation
    pub recommendation_id: String,
    /// User triggering the apply
    #[serde(default)]
    pub triggered_by: Option<String>,
    /// Verification window in minutes (default: 30)
    #[serde(default = "default_verification_minutes")]
    pub verification_minutes: u64,
}

fn default_verification_minutes() -> u64 {
    30
}

/// Response from apply
#[derive(Debug, Serialize)]
pub struct ApplyResponse {
    pub recommendation_id: String,
    pub state: String,
    pub index_name: String,
    pub message: String,
    pub verification_until: Option<String>,
}

/// Request to rollback an applied index
#[derive(Debug, Deserialize)]
pub struct RollbackRequest {
    /// Recommendation ID
    pub recommendation_id: String,
    /// Reason for rollback
    pub reason: String,
    /// User triggering the rollback
    #[serde(default)]
    pub triggered_by: Option<String>,
}

/// Response from rollback
#[derive(Debug, Serialize)]
pub struct RollbackResponse {
    pub recommendation_id: String,
    pub state: String,
    pub message: String,
}

/// Response for status query
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub recommendation_id: String,
    pub state: String,
    pub index_name: Option<String>,
    pub table_name: String,
    pub columns: Vec<String>,
    pub created_at: String,
    pub state_changed_at: String,
    pub validation_result: Option<ValidationSummary>,
    pub recent_events: Vec<AuditEventSummary>,
}

#[derive(Debug, Serialize)]
pub struct ValidationSummary {
    pub validated: bool,
    pub reason: String,
    pub cost_improvement_pct: Option<f64>,
    pub index_used: bool,
}

#[derive(Debug, Serialize)]
pub struct AuditEventSummary {
    pub timestamp: String,
    pub event_type: String,
    pub message: String,
}

/// POST /api/v1/index/validate - Validate an index recommendation with EXPLAIN
pub async fn validate(
    State(state): State<Arc<ValidationHandlerState>>,
    Json(req): Json<ValidateRequest>,
) -> ApiResult<Json<ApiResponse<ValidateResponse>>> {
    // Generate recommendation ID
    let recommendation_id = uuid::Uuid::new_v4().to_string();

    // Log validation started
    {
        let mut audit = state.audit_log.write().await;
        audit.log_validation_started(&recommendation_id, &req.query);
    }

    let start = std::time::Instant::now();

    // Create proof generator with custom settings if provided
    let min_improvement = req.min_improvement_pct.unwrap_or(5.0);
    let generator = ProofGenerator::with_settings(
        state.db_pool.clone(),
        min_improvement,
        std::time::Duration::from_secs(300), // 5 min timeout
    );

    // Generate index names
    let candidate_index_name = generator.generate_candidate_index_name(&req.table_name, &req.columns);
    let production_index_name = generator.generate_production_index_name(&req.table_name, &req.columns);

    // Run validation
    let result = generator
        .validate_index(
            &recommendation_id,
            &req.query,
            &req.table_name,
            &req.columns,
            Some(&req.index_type),
        )
        .await;

    let duration_ms = start.elapsed().as_millis() as i64;

    // Create lifecycle tracker
    let mut lifecycle = RecommendationLifecycle::new(recommendation_id.clone());

    let response = match result {
        Ok(validation_result) => {
            let validated = validation_result.status == ValidationStatus::Validated;
            let diff_summary = validation_result.diff_summary.as_ref().map(|d| d.summary.clone());
            let cost_improvement_pct = validation_result.diff_summary.as_ref().map(|d| d.cost_improvement_pct);
            let index_used = validation_result.diff_summary.as_ref().map(|d| d.index_used).unwrap_or(false);

            // Update state
            if validated {
                let _ = lifecycle.mark_validated(validation_result.recommendation_id.clone());
                let mut audit = state.audit_log.write().await;
                audit.log_validation_succeeded(
                    &recommendation_id,
                    &production_index_name,
                    &req.table_name,
                    cost_improvement_pct.unwrap_or(0.0),
                    duration_ms,
                );
            } else {
                let _ = lifecycle.mark_failed(validation_result.reason_message.clone());
                let mut audit = state.audit_log.write().await;
                audit.log_validation_failed(
                    &recommendation_id,
                    &validation_result.reason_message,
                    duration_ms,
                );
            }

            let create_sql = format!(
                "CREATE INDEX CONCURRENTLY {} ON {} USING {} ({})",
                production_index_name,
                req.table_name,
                req.index_type,
                req.columns.join(", ")
            );

            // Store extended lifecycle
            {
                let mut lifecycles = state.lifecycles.write().await;
                lifecycles.insert(recommendation_id.clone(), ExtendedLifecycle {
                    lifecycle,
                    table_name: req.table_name.clone(),
                    columns: req.columns.clone(),
                    index_type: req.index_type.clone(),
                    validation_result: Some(validation_result.clone()),
                });
            }

            ValidateResponse {
                recommendation_id,
                state: if validated { "VALIDATED" } else { "FAILED" }.to_string(),
                validated,
                reason: validation_result.reason_message,
                cost_improvement_pct,
                index_used,
                candidate_index_name,
                production_index_name,
                create_index_sql: create_sql,
                diff_summary,
            }
        }
        Err(e) => {
            let _ = lifecycle.mark_failed(e.to_string());
            let mut audit = state.audit_log.write().await;
            audit.log_validation_failed(&recommendation_id, &e.to_string(), duration_ms);

            // Store failed lifecycle
            {
                let mut lifecycles = state.lifecycles.write().await;
                lifecycles.insert(recommendation_id.clone(), ExtendedLifecycle {
                    lifecycle,
                    table_name: req.table_name.clone(),
                    columns: req.columns.clone(),
                    index_type: req.index_type.clone(),
                    validation_result: None,
                });
            }

            ValidateResponse {
                recommendation_id,
                state: "FAILED".to_string(),
                validated: false,
                reason: format!("Error: {}", e),
                cost_improvement_pct: None,
                index_used: false,
                candidate_index_name,
                production_index_name,
                create_index_sql: String::new(),
                diff_summary: None,
            }
        }
    };

    Ok(Json(ApiResponse::ok(response)))
}

/// POST /api/v1/index/apply - Apply a validated index recommendation
pub async fn apply(
    State(state): State<Arc<ValidationHandlerState>>,
    Json(req): Json<ApplyRequest>,
) -> ApiResult<Json<ApiResponse<ApplyResponse>>> {
    // ENFORCE: Check operation guard BEFORE any destructive action
    let check = state.operation_guard.check_destructive_operation().await;
    if !check.allowed {
        let reason = check.block_reason.unwrap_or(BlockReason::ReadOnlyMode);
        
        // Log the blocked attempt
        {
            let mut audit = state.audit_log.write().await;
            let event = AuditEvent::new(
                AuditEventType::ApplyFailed,
                format!("Apply blocked: {}", reason),
            )
            .with_recommendation(&req.recommendation_id)
            .with_metadata(serde_json::json!({
                "block_reason": format!("{:?}", reason),
                "effective_mode": format!("{}", check.effective_mode),
                "is_forced": check.is_forced,
            }));
            audit.log(event);
        }
        
        return Err(ApiError::Forbidden(format!(
            "Operation blocked: {}. Effective mode: {}{}",
            reason,
            check.effective_mode,
            if check.is_forced { " (forced)" } else { "" }
        )));
    }

    // Get lifecycle
    let mut lifecycles = state.lifecycles.write().await;
    let extended = lifecycles.get_mut(&req.recommendation_id).ok_or_else(|| {
        ApiError::NotFound(format!(
            "Recommendation {} not found",
            req.recommendation_id
        ))
    })?;

    // Check state allows apply
    if extended.lifecycle.state != RecommendationState::Validated {
        return Err(ApiError::BadRequest(format!(
            "Cannot apply recommendation in state '{}'. Must be 'VALIDATED'.",
            extended.lifecycle.state.as_str()
        )));
    }

    // Generate index name
    let generator = ProofGenerator::new(state.db_pool.clone());
    let index_name = generator.generate_production_index_name(&extended.table_name, &extended.columns);

    // Log apply started
    {
        let mut audit = state.audit_log.write().await;
        let create_sql = format!(
            "CREATE INDEX CONCURRENTLY {} ON {} USING {} ({})",
            index_name,
            extended.table_name,
            extended.index_type,
            extended.columns.join(", ")
        );
        audit.log_apply_started(&req.recommendation_id, &index_name, &extended.table_name, &create_sql);
    }

    let start = std::time::Instant::now();

    // Create the index
    let create_sql = format!(
        "CREATE INDEX CONCURRENTLY IF NOT EXISTS {} ON {} USING {} ({})",
        index_name,
        extended.table_name,
        extended.index_type,
        extended.columns.join(", ")
    );

    match sqlx::query(&create_sql).execute(&state.db_pool).await {
        Ok(_) => {
            let duration_ms = start.elapsed().as_millis() as i64;

            // Calculate verification end time
            let verification_until =
                chrono::Utc::now() + chrono::Duration::minutes(req.verification_minutes as i64);

            // Update lifecycle
            let _ = extended.lifecycle.mark_applied(index_name.clone());
            let _ = extended.lifecycle.start_verification(req.verification_minutes * 60);

            // Log success
            {
                let mut audit = state.audit_log.write().await;
                audit.log_apply_succeeded(
                    &req.recommendation_id,
                    &index_name,
                    &extended.table_name,
                    duration_ms,
                );
            }

            Ok(Json(ApiResponse::ok(ApplyResponse {
                recommendation_id: req.recommendation_id,
                state: extended.lifecycle.state.as_str().to_string(),
                index_name,
                message: "Index created successfully. Entering verification window.".to_string(),
                verification_until: Some(verification_until.to_rfc3339()),
            })))
        }
        Err(e) => {
            let duration_ms = start.elapsed().as_millis() as i64;

            // Log failure
            {
                let mut audit = state.audit_log.write().await;
                audit.log_apply_failed(&req.recommendation_id, &index_name, &e.to_string(), duration_ms);
            }

            // Update lifecycle
            let _ = extended.lifecycle.mark_failed(e.to_string());

            Err(ApiError::Internal(format!(
                "Failed to create index: {}",
                e
            )))
        }
    }
}

/// POST /api/v1/index/rollback - Rollback an applied index
pub async fn rollback(
    State(state): State<Arc<ValidationHandlerState>>,
    Json(req): Json<RollbackRequest>,
) -> ApiResult<Json<ApiResponse<RollbackResponse>>> {
    // ENFORCE: Check operation guard BEFORE any destructive action
    let check = state.operation_guard.check_destructive_operation().await;
    if !check.allowed {
        let reason = check.block_reason.unwrap_or(BlockReason::ReadOnlyMode);
        
        // Log the blocked attempt
        {
            let mut audit = state.audit_log.write().await;
            let event = AuditEvent::new(
                AuditEventType::RollbackFailed,
                format!("Rollback blocked: {}", reason),
            )
            .with_recommendation(&req.recommendation_id)
            .with_metadata(serde_json::json!({
                "block_reason": format!("{:?}", reason),
                "effective_mode": format!("{}", check.effective_mode),
                "is_forced": check.is_forced,
            }));
            audit.log(event);
        }
        
        return Err(ApiError::Forbidden(format!(
            "Operation blocked: {}. Effective mode: {}{}",
            reason,
            check.effective_mode,
            if check.is_forced { " (forced)" } else { "" }
        )));
    }

    // Get lifecycle
    let mut lifecycles = state.lifecycles.write().await;
    let extended = lifecycles.get_mut(&req.recommendation_id).ok_or_else(|| {
        ApiError::NotFound(format!(
            "Recommendation {} not found",
            req.recommendation_id
        ))
    })?;

    // Check state allows rollback
    let can_rollback = matches!(
        extended.lifecycle.state,
        RecommendationState::Applied | RecommendationState::Verifying
    );

    if !can_rollback {
        return Err(ApiError::BadRequest(format!(
            "Cannot rollback recommendation in state '{}'. Must be 'APPLIED' or 'VERIFYING'.",
            extended.lifecycle.state.as_str()
        )));
    }

    let index_name = extended.lifecycle.index_name.clone().unwrap_or_else(|| {
        let generator = ProofGenerator::new(state.db_pool.clone());
        generator.generate_production_index_name(&extended.table_name, &extended.columns)
    });

    // Drop the index
    let drop_sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {}", index_name);

    match sqlx::query(&drop_sql).execute(&state.db_pool).await {
        Ok(_) => {
            // Update lifecycle
            let rollback_reason = RollbackReason::ManualRequest {
                requested_by: req.triggered_by.clone(),
            };
            let _ = extended.lifecycle.mark_rolled_back(rollback_reason);

            // Log success
            {
                let mut audit = state.audit_log.write().await;
                audit.log_rollback(
                    &req.recommendation_id,
                    &index_name,
                    &extended.table_name,
                    &req.reason,
                    true,
                );
            }

            Ok(Json(ApiResponse::ok(RollbackResponse {
                recommendation_id: req.recommendation_id,
                state: extended.lifecycle.state.as_str().to_string(),
                message: format!("Index '{}' dropped successfully", index_name),
            })))
        }
        Err(e) => {
            // Log failure
            {
                let mut audit = state.audit_log.write().await;
                audit.log_rollback(
                    &req.recommendation_id,
                    &index_name,
                    &extended.table_name,
                    &e.to_string(),
                    false,
                );
            }

            Err(ApiError::Internal(format!(
                "Failed to drop index: {}",
                e
            )))
        }
    }
}

/// GET /api/v1/index/status/:id - Get status of a recommendation
pub async fn status(
    State(state): State<Arc<ValidationHandlerState>>,
    Path(recommendation_id): Path<String>,
) -> ApiResult<Json<ApiResponse<StatusResponse>>> {
    // Get lifecycle
    let lifecycles = state.lifecycles.read().await;
    let extended = lifecycles.get(&recommendation_id).ok_or_else(|| {
        ApiError::NotFound(format!(
            "Recommendation {} not found",
            recommendation_id
        ))
    })?;

    // Get recent audit events
    let audit = state.audit_log.read().await;
    let events: Vec<AuditEventSummary> = audit
        .get_events_for_recommendation(&recommendation_id)
        .iter()
        .take(10)
        .map(|e| AuditEventSummary {
            timestamp: e.timestamp.to_rfc3339(),
            event_type: e.event_type.as_str().to_string(),
            message: e.message.clone(),
        })
        .collect();

    let validation_summary = extended.validation_result.as_ref().map(|v| {
        let diff = v.diff_summary.as_ref();
        ValidationSummary {
            validated: v.status == ValidationStatus::Validated,
            reason: v.reason_message.clone(),
            cost_improvement_pct: diff.map(|d| d.cost_improvement_pct),
            index_used: diff.map(|d| d.index_used).unwrap_or(false),
        }
    });

    Ok(Json(ApiResponse::ok(StatusResponse {
        recommendation_id: recommendation_id.clone(),
        state: extended.lifecycle.state.as_str().to_string(),
        index_name: extended.lifecycle.index_name.clone(),
        table_name: extended.table_name.clone(),
        columns: extended.columns.clone(),
        created_at: extended.lifecycle.created_at.to_rfc3339(),
        state_changed_at: extended.lifecycle.updated_at.to_rfc3339(),
        validation_result: validation_summary,
        recent_events: events,
    })))
}

/// GET /api/v1/index/audit - Get audit log
pub async fn audit_log(
    State(state): State<Arc<ValidationHandlerState>>,
) -> ApiResult<Json<ApiResponse<serde_json::Value>>> {
    let audit = state.audit_log.read().await;
    let events: Vec<AuditEventSummary> = audit
        .get_recent_events(100)
        .iter()
        .map(|e| AuditEventSummary {
            timestamp: e.timestamp.to_rfc3339(),
            event_type: e.event_type.as_str().to_string(),
            message: e.message.clone(),
        })
        .collect();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "events": events,
        "total": audit.len(),
    }))))
}
