// src/ebpf/correlator.rs - Correlate database and system metrics

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

use crate::traits::{
    MetricsCorrelator, UnifiedQueryMetrics, SystemMetrics, RootCauseAnalysis, RootCause,
};
use crate::models::QueryStats;

/// Default implementation of MetricsCorrelator
/// 
/// Correlates database metrics (from pg_stat_statements) with system-level
/// metrics (from eBPF) to provide root cause analysis of performance issues.
pub struct DefaultMetricsCorrelator {
    /// Thresholds for identifying issues
    config: CorrelatorConfig,
}

#[derive(Debug, Clone)]
pub struct CorrelatorConfig {
    /// CPU time threshold (microseconds) above which to flag as CPU-bound
    pub cpu_bound_threshold_us: u64,
    
    /// IO wait threshold (microseconds) above which to flag as IO-bound
    pub io_bound_threshold_us: u64,
    
    /// Page cache miss threshold above which to flag memory pressure
    pub page_cache_miss_threshold: u32,
    
    /// Lock wait threshold (microseconds) above which to flag lock contention
    pub lock_wait_threshold_us: u64,
    
    /// Confidence threshold (0.0-1.0) for reporting root causes
    pub min_confidence: f64,
}

impl Default for CorrelatorConfig {
    fn default() -> Self {
        Self {
            cpu_bound_threshold_us: 50_000,      // 50ms
            io_bound_threshold_us: 100_000,      // 100ms
            page_cache_miss_threshold: 100,
            lock_wait_threshold_us: 10_000,      // 10ms
            min_confidence: 0.6,
        }
    }
}

impl DefaultMetricsCorrelator {
    /// Create a new correlator with default config
    pub fn new() -> Self {
        Self::with_config(CorrelatorConfig::default())
    }
    
    /// Create a new correlator with custom config
    pub fn with_config(config: CorrelatorConfig) -> Self {
        Self { config }
    }
    
    /// Analyze system metrics to determine dominant resource bottleneck
    fn analyze_system_metrics(&self, metrics: &SystemMetrics) -> Vec<(RootCause, f64)> {
        let mut causes = Vec::new();
        
        // Check CPU time
        if metrics.cpu_time_us > self.config.cpu_bound_threshold_us {
            let severity = (metrics.cpu_time_us as f64 / self.config.cpu_bound_threshold_us as f64).min(2.0);
            let confidence = (severity - 1.0).max(0.0).min(1.0);
            causes.push((RootCause::CpuBound, confidence));
        }
        
        // Check IO wait time
        if metrics.io_wait_time_us > self.config.io_bound_threshold_us {
            let severity = (metrics.io_wait_time_us as f64 / self.config.io_bound_threshold_us as f64).min(2.0);
            let confidence = (severity - 1.0).max(0.0).min(1.0);
            causes.push((RootCause::IoSaturation, confidence));
        }
        
        // Check page cache misses
        if metrics.page_cache_misses > self.config.page_cache_miss_threshold {
            let severity = (metrics.page_cache_misses as f64 / self.config.page_cache_miss_threshold as f64).min(2.0);
            let confidence = (severity - 1.0).max(0.0).min(1.0);
            causes.push((RootCause::MemoryPressure, confidence));
        }
        
        // Check lock wait time
        if let Some(lock_wait) = metrics.lock_wait_time_us {
            if lock_wait > self.config.lock_wait_threshold_us {
                let severity = (lock_wait as f64 / self.config.lock_wait_threshold_us as f64).min(2.0);
                let confidence = (severity - 1.0).max(0.0).min(1.0);
                causes.push((RootCause::LockContention, confidence));
            }
        }
        
        causes
    }
    
    /// Generate evidence strings for root cause
    fn generate_evidence(
        &self,
        query: &QueryStats,
        metrics: &SystemMetrics,
        cause: &RootCause,
    ) -> Vec<String> {
        let mut evidence = Vec::new();
        
        match cause {
            RootCause::CpuBound => {
                evidence.push(format!(
                    "CPU time: {:.2}ms (threshold: {:.2}ms)",
                    metrics.cpu_time_us as f64 / 1000.0,
                    self.config.cpu_bound_threshold_us as f64 / 1000.0
                ));
                evidence.push(format!(
                    "Query mean execution time: {:.2}ms",
                    query.mean_exec_time_ms
                ));
                if metrics.context_switches > 50 {
                    evidence.push(format!(
                        "High context switches: {}",
                        metrics.context_switches
                    ));
                }
            }
            RootCause::IoSaturation => {
                evidence.push(format!(
                    "IO wait time: {:.2}ms (threshold: {:.2}ms)",
                    metrics.io_wait_time_us as f64 / 1000.0,
                    self.config.io_bound_threshold_us as f64 / 1000.0
                ));
                evidence.push(format!(
                    "Query returns {} rows per execution",
                    query.rows
                ));
            }
            RootCause::MemoryPressure => {
                evidence.push(format!(
                    "Page cache misses: {} (threshold: {})",
                    metrics.page_cache_misses,
                    self.config.page_cache_miss_threshold
                ));
                if let Some(mem) = metrics.memory_bytes {
                    evidence.push(format!(
                        "Memory allocated: {:.2} MB",
                        mem as f64 / (1024.0 * 1024.0)
                    ));
                }
            }
            RootCause::LockContention => {
                if let Some(lock_wait) = metrics.lock_wait_time_us {
                    evidence.push(format!(
                        "Lock wait time: {:.2}ms (threshold: {:.2}ms)",
                        lock_wait as f64 / 1000.0,
                        self.config.lock_wait_threshold_us as f64 / 1000.0
                    ));
                }
                evidence.push(format!(
                    "Query called {} times",
                    query.calls
                ));
            }
            RootCause::MissingIndex => {
                evidence.push("Database metrics indicate potential missing index".to_string());
                evidence.push(format!("Query execution time: {:.2}ms", query.mean_exec_time_ms));
            }
            _ => {}
        }
        
        evidence
    }
    
    /// Generate recommendations for root cause
    fn generate_recommendations(&self, cause: &RootCause, query: &QueryStats) -> Vec<String> {
        match cause {
            RootCause::CpuBound => vec![
                "Consider optimizing complex computations or JSON processing".to_string(),
                "Use EXPLAIN ANALYZE to identify CPU-intensive operations".to_string(),
                "Consider caching computed results".to_string(),
                format!("Review query: {}", truncate_query(&query.query, 100)),
            ],
            RootCause::IoSaturation => vec![
                "Add appropriate indexes to reduce disk reads".to_string(),
                "Consider using FILLFACTOR to reduce index bloat".to_string(),
                "Review table partitioning strategy".to_string(),
                "Check if table VACUUM is needed".to_string(),
            ],
            RootCause::MemoryPressure => vec![
                "Increase PostgreSQL shared_buffers if possible".to_string(),
                "Reduce work_mem for this query type".to_string(),
                "Consider query rewriting to reduce memory usage".to_string(),
                "Check for memory leaks in PostgreSQL extensions".to_string(),
            ],
            RootCause::LockContention => vec![
                "Reduce transaction duration".to_string(),
                "Use appropriate isolation levels".to_string(),
                "Consider table partitioning to reduce lock scope".to_string(),
                "Review application-level locking strategy".to_string(),
            ],
            RootCause::MissingIndex => vec![
                "Add indexes on frequently queried columns".to_string(),
                "Use covering indexes where appropriate".to_string(),
                "Consider partial indexes for conditional queries".to_string(),
            ],
            RootCause::TableBloat => vec![
                "Run VACUUM FULL on affected tables".to_string(),
                "Increase autovacuum frequency".to_string(),
                "Review update/delete patterns".to_string(),
            ],
            _ => vec!["Review query performance characteristics".to_string()],
        }
    }
}

impl Default for DefaultMetricsCorrelator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MetricsCorrelator for DefaultMetricsCorrelator {
    async fn correlate(
        &self,
        db_metrics: Vec<QueryStats>,
        system_metrics: Vec<SystemMetrics>,
    ) -> Result<Vec<UnifiedQueryMetrics>> {
        // Simple correlation: match by index position
        // In a real implementation, this would correlate by:
        // - Query ID
        // - Timestamp
        // - PostgreSQL PID
        
        let mut unified = Vec::new();
        
        for (idx, query_stats) in db_metrics.iter().enumerate() {
            let system_metrics_opt = system_metrics.get(idx).cloned();
            
            unified.push(UnifiedQueryMetrics {
                query_stats: query_stats.clone(),
                system_metrics: system_metrics_opt,
            });
        }
        
        tracing::info!(
            "Correlated {} database metrics with {} system metrics",
            db_metrics.len(),
            system_metrics.len()
        );
        
        Ok(unified)
    }
    
    async fn identify_root_causes(
        &self,
        unified_metrics: &[UnifiedQueryMetrics],
    ) -> Result<Vec<RootCauseAnalysis>> {
        let mut analyses = Vec::new();
        
        for unified in unified_metrics {
            // Skip if no system metrics available
            let sys_metrics = match &unified.system_metrics {
                Some(m) => m,
                None => continue,
            };
            
            // Analyze system metrics to identify potential causes
            let causes = self.analyze_system_metrics(sys_metrics);
            
            // Filter by confidence threshold
            let high_confidence_causes: Vec<_> = causes
                .into_iter()
                .filter(|(_, confidence)| *confidence >= self.config.min_confidence)
                .collect();
            
            if high_confidence_causes.is_empty() {
                continue;
            }
            
            // Find the primary cause (highest confidence)
            let (primary_cause, confidence) = high_confidence_causes
                .iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap()
                .clone();
            
            // Generate evidence and recommendations
            let evidence = self.generate_evidence(
                &unified.query_stats,
                sys_metrics,
                &primary_cause,
            );
            
            let recommendations = self.generate_recommendations(
                &primary_cause,
                &unified.query_stats,
            );
            
            analyses.push(RootCauseAnalysis {
                query_id: unified.query_stats.queryid.unwrap_or(0),
                root_cause: primary_cause,
                confidence,
                evidence,
                recommendations,
            });
        }
        
        tracing::info!(
            "Identified {} root causes from {} unified metrics",
            analyses.len(),
            unified_metrics.len()
        );
        
        Ok(analyses)
    }
}

/// Truncate query for display
fn truncate_query(query: &str, max_len: usize) -> String {
    let cleaned = query.trim().replace('\n', " ").replace("  ", " ");
    if cleaned.len() <= max_len {
        cleaned
    } else {
        format!("{}...", &cleaned[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_identify_cpu_bound() {
        let correlator = DefaultMetricsCorrelator::new();
        
        let query = QueryStats {
            queryid: Some(1),
            query: "SELECT * FROM users".to_string(),
            calls: 1000,
            total_exec_time_ms: 100000.0,
            mean_exec_time_ms: 100.0,
            rows: 10000,
            shared_blks_read: 0,
            shared_blks_hit: 0,
        };
        
        let metrics = SystemMetrics {
            cpu_time_us: 150_000, // 150ms - above threshold
            io_wait_time_us: 10_000,
            context_switches: 10,
            page_cache_misses: 5,
            memory_bytes: Some(1_000_000),
            lock_wait_time_us: Some(0),
        };
        
        let unified = vec![UnifiedQueryMetrics {
            query_stats: query,
            system_metrics: Some(metrics),
        }];
        
        let analyses = correlator.identify_root_causes(&unified).await.unwrap();
        
        assert_eq!(analyses.len(), 1);
        assert_eq!(analyses[0].root_cause, RootCause::CpuBound);
        assert!(analyses[0].confidence > 0.6);
    }
}
