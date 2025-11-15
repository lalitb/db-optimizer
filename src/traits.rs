// src/traits.rs - Core trait abstractions for db-optimizer

use anyhow::Result;
use async_trait::async_trait;
use crate::models::{QueryStats, IndexInfo, TableInfo, Recommendation, AnalysisReport};

/// Trait for collecting metrics from databases
#[async_trait]
pub trait MetricsCollector: Send + Sync {
    /// Collect query statistics
    async fn collect_query_stats(&self, limit: i64) -> Result<Vec<QueryStats>>;
    
    /// Collect index information
    async fn collect_index_info(&self) -> Result<Vec<IndexInfo>>;
    
    /// Collect table information
    async fn collect_table_info(&self) -> Result<Vec<TableInfo>>;
    
    /// Get database name
    async fn get_database_name(&self) -> Result<String>;
    
    /// Get database version
    async fn get_database_version(&self) -> Result<String>;
    
    /// Test database connection
    async fn test_connection(&self) -> Result<()>;
}

/// Trait for analyzing query performance and generating insights
#[async_trait]
pub trait Analyzer: Send + Sync {
    /// Analyze query statistics and identify issues
    async fn analyze_queries(
        &self,
        queries: &[QueryStats],
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>>;
    
    /// Analyze a single query
    async fn analyze_single_query(
        &self,
        query: &QueryStats,
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>>;
}

/// Trait for generating recommendations from analysis results
#[async_trait]
pub trait RecommendationGenerator: Send + Sync {
    /// Generate recommendations based on analysis
    async fn generate_recommendations(
        &self,
        queries: &[QueryStats],
        indexes: &[IndexInfo],
        tables: &[TableInfo],
    ) -> Result<Vec<Recommendation>>;
    
    /// Filter recommendations by severity or other criteria
    fn filter_recommendations(
        &self,
        recommendations: Vec<Recommendation>,
        min_severity: crate::models::Severity,
    ) -> Vec<Recommendation>;
    
    /// Sort recommendations by priority
    fn sort_by_priority(&self, recommendations: &mut [Recommendation]);
}

/// Trait for executing and managing auto-apply operations
#[async_trait]
pub trait AutoApplyOrchestrator: Send + Sync {
    /// Evaluate if a recommendation should be auto-applied
    async fn should_auto_apply(&self, recommendation: &Recommendation) -> Result<bool>;
    
    /// Execute a recommendation
    async fn execute_recommendation(
        &self,
        recommendation: &Recommendation,
    ) -> Result<crate::models::ApplyResult>;
    
    /// Verify the impact of an applied recommendation
    async fn verify_impact(
        &self,
        recommendation_id: &str,
    ) -> Result<crate::models::ApplyResult>;
    
    /// Rollback a recommendation
    async fn rollback_recommendation(&self, recommendation_id: &str) -> Result<()>;
}

/// Unified metrics combining database and system-level data
#[derive(Debug, Clone)]
pub struct UnifiedQueryMetrics {
    /// Basic query stats from database
    pub query_stats: QueryStats,
    
    /// System-level metrics (from eBPF or other sources)
    pub system_metrics: Option<SystemMetrics>,
}

/// System-level metrics (CPU, IO, memory)
#[derive(Debug, Clone)]
pub struct SystemMetrics {
    /// CPU time in microseconds
    pub cpu_time_us: u64,
    
    /// IO wait time in microseconds
    pub io_wait_time_us: u64,
    
    /// Number of context switches
    pub context_switches: u32,
    
    /// Page cache misses
    pub page_cache_misses: u32,
    
    /// Memory allocated in bytes
    pub memory_bytes: Option<u64>,
    
    /// Lock wait time in microseconds
    pub lock_wait_time_us: Option<u64>,
}

/// Trait for collecting system-level metrics (eBPF, OS stats, etc.)
#[async_trait]
pub trait SystemMetricsCollector: Send + Sync {
    /// Collect system metrics for a specific query or process
    async fn collect_metrics(&self, query_id: Option<i64>) -> Result<SystemMetrics>;
    
    /// Start collecting metrics in the background
    async fn start_collection(&mut self) -> Result<()>;
    
    /// Stop collecting metrics
    async fn stop_collection(&mut self) -> Result<()>;
    
    /// Get collected metrics for a time range
    async fn get_metrics_range(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<SystemMetrics>>;
}

/// Trait for correlating database and system metrics
#[async_trait]
pub trait MetricsCorrelator: Send + Sync {
    /// Correlate database query stats with system metrics
    async fn correlate(
        &self,
        db_metrics: Vec<QueryStats>,
        system_metrics: Vec<SystemMetrics>,
    ) -> Result<Vec<UnifiedQueryMetrics>>;
    
    /// Identify root causes based on unified metrics
    async fn identify_root_causes(
        &self,
        unified_metrics: &[UnifiedQueryMetrics],
    ) -> Result<Vec<RootCauseAnalysis>>;
}

/// Root cause analysis result
#[derive(Debug, Clone)]
pub struct RootCauseAnalysis {
    /// Query ID
    pub query_id: i64,
    
    /// Identified root cause
    pub root_cause: RootCause,
    
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
    
    /// Supporting evidence
    pub evidence: Vec<String>,
    
    /// Recommended actions
    pub recommendations: Vec<String>,
}

/// Possible root causes for performance issues
#[derive(Debug, Clone, PartialEq)]
pub enum RootCause {
    /// CPU-bound processing (e.g., complex computations, JSON parsing)
    CpuBound,
    
    /// IO saturation (high read/write latency)
    IoSaturation,
    
    /// Memory pressure (page cache misses, swapping)
    MemoryPressure,
    
    /// Lock contention
    LockContention,
    
    /// Missing or inefficient index
    MissingIndex,
    
    /// Table bloat or fragmentation
    TableBloat,
    
    /// Network latency
    NetworkLatency,
    
    /// Multiple factors
    MultipleCauses(Vec<RootCause>),
}

impl std::fmt::Display for RootCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CpuBound => write!(f, "CPU-bound processing"),
            Self::IoSaturation => write!(f, "I/O saturation"),
            Self::MemoryPressure => write!(f, "Memory pressure"),
            Self::LockContention => write!(f, "Lock contention"),
            Self::MissingIndex => write!(f, "Missing index"),
            Self::TableBloat => write!(f, "Table bloat"),
            Self::NetworkLatency => write!(f, "Network latency"),
            Self::MultipleCauses(causes) => {
                write!(f, "Multiple causes: ")?;
                for (i, cause) in causes.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", cause)?;
                }
                Ok(())
            }
        }
    }
}

/// Trait for reporting and outputting analysis results
pub trait Reporter: Send + Sync {
    /// Generate a full analysis report
    fn generate_report(
        &self,
        queries: &[QueryStats],
        recommendations: &[Recommendation],
        database_name: &str,
        postgres_version: &str,
    ) -> Result<AnalysisReport>;
    
    /// Format report for terminal output
    fn format_terminal(&self, report: &AnalysisReport) -> Result<String>;
    
    /// Format report as JSON
    fn format_json(&self, report: &AnalysisReport) -> Result<String>;
    
    /// Format report as Markdown
    fn format_markdown(&self, report: &AnalysisReport) -> Result<String>;
}

/// Configuration for the optimizer pipeline
#[derive(Debug, Clone)]
pub struct OptimizerConfig {
    /// Database connection configuration
    pub database_config: crate::config::DatabaseConfig,
    
    /// Operation mode
    pub operation_mode: crate::config::OperationMode,
    
    /// Analysis configuration
    pub analysis_config: crate::config::AnalysisConfig,
    
    /// Safety configuration
    pub safety_config: crate::config::SafetyConfig,
    
    /// Enable system metrics collection
    pub enable_system_metrics: bool,
}

/// The main optimizer pipeline that coordinates all components
pub struct OptimizerPipeline {
    metrics_collector: Box<dyn MetricsCollector>,
    analyzer: Box<dyn Analyzer>,
    recommendation_generator: Box<dyn RecommendationGenerator>,
    reporter: Box<dyn Reporter>,
    system_metrics_collector: Option<Box<dyn SystemMetricsCollector>>,
    auto_apply: Option<Box<dyn AutoApplyOrchestrator>>,
    config: OptimizerConfig,
}

impl OptimizerPipeline {
    /// Create a new optimizer pipeline
    pub fn new(
        metrics_collector: Box<dyn MetricsCollector>,
        analyzer: Box<dyn Analyzer>,
        recommendation_generator: Box<dyn RecommendationGenerator>,
        reporter: Box<dyn Reporter>,
        config: OptimizerConfig,
    ) -> Self {
        Self {
            metrics_collector,
            analyzer,
            recommendation_generator,
            reporter,
            system_metrics_collector: None,
            auto_apply: None,
            config,
        }
    }
    
    /// Add system metrics collection
    pub fn with_system_metrics(mut self, collector: Box<dyn SystemMetricsCollector>) -> Self {
        self.system_metrics_collector = Some(collector);
        self
    }
    
    /// Add auto-apply orchestrator
    pub fn with_auto_apply(mut self, orchestrator: Box<dyn AutoApplyOrchestrator>) -> Self {
        self.auto_apply = Some(orchestrator);
        self
    }
    
    /// Run the full analysis pipeline
    pub async fn run_analysis(&self) -> Result<AnalysisReport> {
        tracing::info!("Starting optimizer pipeline");
        
        // Step 1: Collect metrics
        tracing::info!("Collecting database metrics...");
        let queries = self.metrics_collector.collect_query_stats(self.config.analysis_config.query_limit).await?;
        let indexes = self.metrics_collector.collect_index_info().await?;
        let tables = self.metrics_collector.collect_table_info().await?;
        let db_name = self.metrics_collector.get_database_name().await?;
        let db_version = self.metrics_collector.get_database_version().await?;
        
        tracing::info!("Collected {} queries, {} indexes, {} tables", 
            queries.len(), indexes.len(), tables.len());
        
        // Step 2: Analyze and generate recommendations
        tracing::info!("Analyzing queries...");
        let recommendations = self.recommendation_generator
            .generate_recommendations(&queries, &indexes, &tables)
            .await?;
        
        tracing::info!("Generated {} recommendations", recommendations.len());
        
        // Step 3: Generate report
        let report = self.reporter.generate_report(&queries, &recommendations, &db_name, &db_version)?;
        
        tracing::info!("Pipeline complete");
        Ok(report)
    }
}
