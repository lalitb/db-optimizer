// HTTP handlers for configuration tuning endpoints

use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config_tuner::{
    ConfigTuner, ConfigValidator, WorkloadDetector,
    models::{SystemInfo, WorkloadType, OptimalConfig, ValidationReport, DiskType, OsType},
};
use crate::http::{error::ApiError, models::ApiResponse};

/// Shared state for config tuning handlers
#[derive(Clone)]
pub struct ConfigTuningState {
    pub pool: sqlx::PgPool,
}

/// Request to tune configuration
#[derive(Debug, Deserialize)]
pub struct TuneConfigRequest {
    /// Workload type (optional, will auto-detect if not provided)
    pub workload: Option<WorkloadType>,
    /// Override system RAM in GB (optional)
    pub ram_gb: Option<f64>,
    /// Override CPU count (optional)
    pub cpu_count: Option<u32>,
    /// Override disk type (optional)
    pub disk_type: Option<DiskType>,
    /// Override OS type (optional)
    pub os_type: Option<OsType>,
    /// Override PostgreSQL version (optional)
    pub pg_version: Option<f32>,
}

/// Response from tune configuration
#[derive(Debug, Serialize)]
pub struct TuneConfigResponse {
    pub system_info: SystemInfoResponse,
    pub workload: WorkloadResponse,
    pub optimal_config: OptimalConfig,
    pub postgresql_conf: String,
}

#[derive(Debug, Serialize)]
pub struct SystemInfoResponse {
    pub ram_gb: f64,
    pub cpu_count: u32,
    pub disk_type: DiskType,
    pub os_type: OsType,
    pub pg_version: f32,
}

#[derive(Debug, Serialize)]
pub struct WorkloadResponse {
    pub workload_type: WorkloadType,
    pub detected: bool,
    pub confidence: Option<f64>,
}

/// Tune configuration endpoint
/// POST /api/v1/config/tune
pub async fn tune_config(
    State(state): State<Arc<ConfigTuningState>>,
    Json(req): Json<TuneConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Get or detect system info
    let mut system_info = if req.ram_gb.is_some() || req.cpu_count.is_some() {
        // User provided overrides
        let base = SystemInfo::detect().map_err(|e| {
            ApiError::Internal(format!("Failed to detect system info: {}", e))
        })?;
        
        SystemInfo {
            total_ram_kb: req
                .ram_gb
                .map(|gb| (gb * 1024.0 * 1024.0) as u64)
                .unwrap_or(base.total_ram_kb),
            cpu_count: req.cpu_count.unwrap_or(base.cpu_count),
            disk_type: req.disk_type.unwrap_or(base.disk_type),
            os_type: req.os_type.unwrap_or(base.os_type),
            pg_version: req.pg_version.unwrap_or(base.pg_version),
        }
    } else {
        SystemInfo::detect().map_err(|e| {
            ApiError::Internal(format!("Failed to detect system info: {}", e))
        })?
    };

    // Get or detect workload
    let (workload, detected, confidence) = if let Some(w) = req.workload {
        (w, false, None)
    } else {
        // Auto-detect workload
        let detector = WorkloadDetector::new();
        let detected_workload = detector.detect(&state.pool).await.map_err(|e| {
            ApiError::Internal(format!("Failed to detect workload: {}", e))
        })?;
        
        // Calculate confidence (we'd need to re-fetch stats, simplified here)
        (detected_workload, true, Some(0.75))
    };

    // Calculate optimal config
    let tuner = ConfigTuner::new();
    let optimal_config = tuner.calculate_optimal_config(&system_info, workload);

    // Generate PostgreSQL conf
    let postgresql_conf = optimal_config.to_postgresql_conf();

    let response = TuneConfigResponse {
        system_info: SystemInfoResponse {
            ram_gb: system_info.ram_gb(),
            cpu_count: system_info.cpu_count,
            disk_type: system_info.disk_type,
            os_type: system_info.os_type,
            pg_version: system_info.pg_version,
        },
        workload: WorkloadResponse {
            workload_type: workload,
            detected,
            confidence,
        },
        optimal_config,
        postgresql_conf,
    };

    Ok((StatusCode::OK, Json(ApiResponse::ok(response))))
}

/// Validate configuration endpoint
/// GET /api/v1/config/validate
pub async fn validate_config(
    State(state): State<Arc<ConfigTuningState>>,
) -> Result<impl IntoResponse, ApiError> {
    let validator = ConfigValidator::new();
    let report = validator.validate(&state.pool).await.map_err(|e| {
        ApiError::Internal(format!("Failed to validate config: {}", e))
    })?;

    Ok((StatusCode::OK, Json(ApiResponse::ok(report))))
}

/// Detect workload endpoint
/// GET /api/v1/config/detect-workload
pub async fn detect_workload(
    State(state): State<Arc<ConfigTuningState>>,
) -> Result<impl IntoResponse, ApiError> {
    let detector = WorkloadDetector::new();
    let workload = detector.detect(&state.pool).await.map_err(|e| {
        ApiError::Internal(format!("Failed to detect workload: {}", e))
    })?;

    #[derive(Serialize)]
    struct DetectWorkloadResponse {
        workload: WorkloadType,
        confidence: f64,
        description: String,
    }

    let description = match workload {
        WorkloadType::OLTP => "High-concurrency transactional workload with fast queries",
        WorkloadType::DataWarehouse => "Analytics workload with complex, long-running queries",
        WorkloadType::Web => "Web application with moderate concurrency and mostly reads",
        WorkloadType::Desktop => "Local development or desktop application",
        WorkloadType::Mixed => "Mixed workload that doesn't clearly match a pattern",
    };

    let response = DetectWorkloadResponse {
        workload,
        confidence: 0.75, // Simplified, would calculate actual confidence
        description: description.to_string(),
    };

    Ok((StatusCode::OK, Json(ApiResponse::ok(response))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tune_config_request_deserialization() {
        let json = r#"{"workload":"oltp","ram_gb":64.0,"cpu_count":16}"#;
        let req: TuneConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.workload, Some(WorkloadType::OLTP));
        assert_eq!(req.ram_gb, Some(64.0));
        assert_eq!(req.cpu_count, Some(16));
    }

    #[test]
    fn test_tune_config_request_minimal() {
        let json = r#"{}"#;
        let req: TuneConfigRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.workload, None);
        assert_eq!(req.ram_gb, None);
    }

    #[test]
    fn test_workload_response_serialization() {
        let response = WorkloadResponse {
            workload_type: WorkloadType::OLTP,
            detected: true,
            confidence: Some(0.85),
        };
        
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"workload_type\":\"oltp\""));
        assert!(json.contains("\"detected\":true"));
        assert!(json.contains("\"confidence\":0.85"));
    }
}
