pub mod agent;
pub mod models;
pub mod collector;
pub mod analyzer;
pub mod cost_estimator;
pub mod recommendations;
pub mod output;
pub mod config;
pub mod auto_apply;
pub mod query_rewriter;
pub mod persistence;
pub mod traits;
pub mod adapters;
pub mod http;
pub mod ebpf;
pub mod config_tuner;
pub mod azure;
pub mod events;
#[cfg(feature = "cnpg")]
pub mod cnpg_plugin;
pub mod privacy;

pub use models::*;
pub use collector::DatabaseCollector;
pub use analyzer::QueryAnalyzer;
pub use cost_estimator::CostEstimator;
pub use recommendations::RecommendationEngine;
pub use output::OutputFormatter;
pub use auto_apply::AutoApplyOrchestrator;

// Re-export all config types for convenient access
pub use config::{
    Config, 
    OperationMode, 
    DatabaseConfig,
    AnalysisConfig,
    SafetyConfig,
    MaintenanceWindow,
    EbpfConfig,
    AutoApplyConfigExtended,
    PersistenceConfig,
    PersistenceBackend,
    PrivacyConfig,
    QueryTextStorage,
    HttpServerConfig,
    NotificationConfig,
    SuccessCriterion,
    SuccessMetric,
    ThresholdConfig,
};
