// src/adapters/mod.rs - Adapters for existing components to implement new traits

pub mod output_reporter;
pub mod postgres_collector;
pub mod query_analyzer;
pub mod recommendation_engine;

pub use output_reporter::OutputReporterAdapter;
pub use postgres_collector::PostgresMetricsCollector;
pub use query_analyzer::QueryAnalyzerAdapter;
pub use recommendation_engine::RecommendationEngineAdapter;
