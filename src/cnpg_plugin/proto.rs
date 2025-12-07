// CNPG-I (CloudNativePG Interface) Protocol Definitions
//
// This module defines the gRPC service interfaces for CloudNativePG plugin integration.
// In production, these would be generated from the official CNPG-I proto files using tonic-build.
//
// Reference: https://github.com/cloudnative-pg/cnpg-i

use tonic::{Request, Response, Status};

// ============================================================================
// Operator Lifecycle Service
// ============================================================================

/// Operator lifecycle service trait
#[tonic::async_trait]
pub trait OperatorService: Send + Sync + 'static {
    /// Called during cluster lifecycle events (create, update, delete)
    async fn lifecycle(
        &self,
        request: Request<OperatorLifecycleRequest>,
    ) -> Result<Response<OperatorLifecycleResponse>, Status>;

    /// Called to get plugin capabilities
    async fn get_capabilities(
        &self,
        request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<GetCapabilitiesResponse>, Status>;
}

/// Lifecycle request from CNPG operator
#[derive(Debug, Clone)]
pub struct OperatorLifecycleRequest {
    /// Cluster name
    pub cluster_name: String,
    /// Kubernetes namespace
    pub namespace: String,
    /// Instance role: "primary" or "replica"
    pub role: String,
    /// PostgreSQL connection string
    pub connection_string: String,
    /// Lifecycle operation type
    pub operation: OperationType,
    /// Cluster definition (JSON)
    pub cluster_definition: String,
}

/// Lifecycle operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    /// Cluster is being created
    Create,
    /// Cluster configuration is being updated
    Update,
    /// Cluster is being deleted
    Delete,
    /// Periodic health check
    HealthCheck,
    /// Failover event
    Failover,
    /// Switchover event
    Switchover,
}

/// Lifecycle response to CNPG operator
#[derive(Debug, Clone)]
pub struct OperatorLifecycleResponse {
    /// Status: "Healthy", "Warning", "Critical", "Skipped"
    pub status: String,
    /// Human-readable message
    pub message: String,
    /// Health score (0-100)
    pub health_score: u32,
    /// Recommendations (JSON array)
    pub recommendations: String,
    /// Whether to proceed with the operation
    pub proceed: bool,
}

/// Get capabilities request
#[derive(Debug, Clone)]
pub struct GetCapabilitiesRequest {}

/// Get capabilities response
#[derive(Debug, Clone)]
pub struct GetCapabilitiesResponse {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Supported operations
    pub capabilities: Vec<PluginCapability>,
}

/// Plugin capability
#[derive(Debug, Clone)]
pub struct PluginCapability {
    /// Capability type
    pub capability_type: CapabilityType,
    /// Whether this capability is enabled
    pub enabled: bool,
}

/// Capability type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityType {
    /// Health monitoring
    HealthMonitoring,
    /// Performance analysis
    PerformanceAnalysis,
    /// Index recommendations
    IndexRecommendations,
    /// Query rewriting
    QueryRewriting,
    /// Auto-optimization
    AutoOptimization,
    /// eBPF instrumentation
    EbpfInstrumentation,
    /// Cost analysis
    CostAnalysis,
}

// ============================================================================
// WAL Service (Write-Ahead Log)
// ============================================================================

/// WAL service trait for backup/restore hooks
#[tonic::async_trait]
pub trait WalService: Send + Sync + 'static {
    /// Called before WAL archive
    async fn pre_archive(
        &self,
        request: Request<WalArchiveRequest>,
    ) -> Result<Response<WalArchiveResponse>, Status>;

    /// Called after WAL archive
    async fn post_archive(
        &self,
        request: Request<WalArchiveRequest>,
    ) -> Result<Response<WalArchiveResponse>, Status>;
}

#[derive(Debug, Clone)]
pub struct WalArchiveRequest {
    pub cluster_name: String,
    pub namespace: String,
    pub wal_file: String,
    pub source_path: String,
}

#[derive(Debug, Clone)]
pub struct WalArchiveResponse {
    pub success: bool,
    pub message: String,
}

// ============================================================================
// Backup Service
// ============================================================================

/// Backup service trait
#[tonic::async_trait]
pub trait BackupService: Send + Sync + 'static {
    /// Called before backup
    async fn pre_backup(
        &self,
        request: Request<BackupRequest>,
    ) -> Result<Response<BackupResponse>, Status>;

    /// Called after backup
    async fn post_backup(
        &self,
        request: Request<BackupRequest>,
    ) -> Result<Response<BackupResponse>, Status>;
}

#[derive(Debug, Clone)]
pub struct BackupRequest {
    pub cluster_name: String,
    pub namespace: String,
    pub backup_name: String,
    pub backup_type: String,
}

#[derive(Debug, Clone)]
pub struct BackupResponse {
    pub success: bool,
    pub message: String,
    /// Metrics captured during backup
    pub metrics: Option<BackupMetrics>,
}

#[derive(Debug, Clone)]
pub struct BackupMetrics {
    /// Backup size in bytes
    pub size_bytes: u64,
    /// Backup duration in seconds
    pub duration_secs: f64,
    /// Throughput in MB/s
    pub throughput_mbps: f64,
}

// ============================================================================
// Reconciler Service
// ============================================================================

/// Reconciler service for configuration management
#[tonic::async_trait]
pub trait ReconcilerService: Send + Sync + 'static {
    /// Called during reconciliation to validate config
    async fn validate_config(
        &self,
        request: Request<ValidateConfigRequest>,
    ) -> Result<Response<ValidateConfigResponse>, Status>;

    /// Called to get recommended configuration
    async fn get_recommended_config(
        &self,
        request: Request<GetRecommendedConfigRequest>,
    ) -> Result<Response<GetRecommendedConfigResponse>, Status>;
}

#[derive(Debug, Clone)]
pub struct ValidateConfigRequest {
    pub cluster_name: String,
    pub namespace: String,
    pub config: String, // JSON
}

#[derive(Debug, Clone)]
pub struct ValidateConfigResponse {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GetRecommendedConfigRequest {
    pub cluster_name: String,
    pub namespace: String,
    pub workload_type: String,
    pub resource_constraints: String, // JSON
}

#[derive(Debug, Clone)]
pub struct GetRecommendedConfigResponse {
    pub config: String, // JSON
    pub explanation: String,
}

// ============================================================================
// Server wrappers (placeholders for tonic-build generated code)
// ============================================================================

/// Operator service server wrapper
pub struct OperatorServer<T> {
    inner: T,
}

impl<T: OperatorService> OperatorServer<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }
}

/// WAL service server wrapper
pub struct WalServer<T> {
    #[allow(dead_code)] // Required by server pattern
    inner: T,
}

impl<T: WalService> WalServer<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

/// Backup service server wrapper
pub struct BackupServer<T> {
    #[allow(dead_code)] // Required by server pattern
    inner: T,
}

impl<T: BackupService> BackupServer<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

/// Reconciler service server wrapper
pub struct ReconcilerServer<T> {
    #[allow(dead_code)] // Required by server pattern
    inner: T,
}

impl<T: ReconcilerService> ReconcilerServer<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}
