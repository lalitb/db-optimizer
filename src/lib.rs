// Allow some clippy lints that are too noisy or require significant refactoring
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::redundant_guards)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::if_same_then_else)]

pub mod adapters;
pub mod agent;
pub mod ai;
pub mod analyzer;
pub mod auto_apply;
pub mod azure;
pub mod cnpg_plugin;
pub mod collector;
pub mod config;
pub mod config_tuner;
pub mod cost_estimator;
pub mod ebpf;
pub mod events;
pub mod fleet;
pub mod http;
pub mod models;
pub mod output;
pub mod persistence;
pub mod query_rewriter;
pub mod recommendations;
pub mod remote_agent;
pub mod traits;

pub use analyzer::QueryAnalyzer;
pub use auto_apply::AutoApplyOrchestrator;
pub use collector::DatabaseCollector;
pub use cost_estimator::{CostEstimator, PricingConfig};
pub use models::*;
pub use output::OutputFormatter;
pub use recommendations::RecommendationEngine;

// Re-export AI types
pub use ai::{AiAdvisor, AiConfig, LlmProvider};

// Re-export all config types for convenient access
pub use config::{
    AnalysisConfig, AutoApplyConfigExtended, Config, DatabaseConfig, EbpfConfig, HttpServerConfig,
    MaintenanceWindow, NotificationConfig, OperationMode, PersistenceBackend, PersistenceConfig,
    SafetyConfig, SuccessCriterion, SuccessMetric, ThresholdConfig,
};
