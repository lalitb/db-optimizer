// src/auto_apply/enhanced_verifier.rs - Enhanced verification with system metrics

use anyhow::Result;
use chrono::{DateTime, Utc, Duration};

use crate::models::{QueryStats, ApplyResult, ApplyStatus, PerformanceBaseline};
use crate::traits::{SystemMetrics, MetricsCollector, SystemMetricsCollector};
use crate::config::SuccessCriterion;

/// Enhanced verifier that considers both database and system metrics
pub struct EnhancedVerifier {
    /// Minimum duration to wait before verification
    stabilization_period_seconds: u64,
    
    /// Success criteria configuration
    success_criteria: Vec<SuccessCriterion>,
}

impl EnhancedVerifier {
    /// Create a new enhanced verifier
    pub fn new(
        stabilization_period_seconds: u64,
        success_criteria: Vec<SuccessCriterion>,
    ) -> Self {
        Self {
            stabilization_period_seconds,
            success_criteria,
        }
    }
    
    /// Verify the impact of an applied change
    pub async fn verify_impact(
        &self,
        recommendation_id: &str,
        baseline_metrics: &BaselineMetrics,
        db_collector: &dyn MetricsCollector,
        system_collector: Option<&dyn SystemMetricsCollector>,
    ) -> Result<VerificationResult> {
        tracing::info!("Verifying impact for recommendation: {}", recommendation_id);
        
        // Wait for stabilization
        tracing::info!(
            "Waiting {} seconds for metrics to stabilize...",
            self.stabilization_period_seconds
        );
        tokio::time::sleep(tokio::time::Duration::from_secs(
            self.stabilization_period_seconds
        )).await;
        
        // Collect current metrics
        let current_db_metrics = self.collect_current_db_metrics(
            db_collector,
            &baseline_metrics.query_ids
        ).await?;
        
        let current_system_metrics = if let Some(collector) = system_collector {
            Some(self.collect_current_system_metrics(collector).await?)
        } else {
            None
        };
        
        // Compare against baseline
        let db_comparison = self.compare_db_metrics(
            &baseline_metrics.db_metrics,
            &current_db_metrics,
        )?;
        
        let system_comparison = if let (Some(baseline), Some(current)) = 
            (&baseline_metrics.system_metrics, &current_system_metrics) {
            Some(self.compare_system_metrics(baseline, current)?)
        } else {
            None
        };
        
        // Evaluate success criteria
        let evaluation = self.evaluate_success_criteria(
            &db_comparison,
            system_comparison.as_ref(),
        )?;
        
        Ok(VerificationResult {
            recommendation_id: recommendation_id.to_string(),
            verified_at: Utc::now(),
            db_comparison,
            system_comparison,
            meets_criteria: evaluation.success,
            confidence: evaluation.confidence,
            details: evaluation.details,
        })
    }
    
    /// Collect current database metrics for affected queries
    async fn collect_current_db_metrics(
        &self,
        collector: &dyn MetricsCollector,
        query_ids: &[i64],
    ) -> Result<Vec<QueryStats>> {
        // Collect all current query stats
        let all_stats = collector.collect_query_stats(1000).await?;
        
        // Filter to affected queries
        let affected_stats: Vec<QueryStats> = all_stats
            .into_iter()
            .filter(|stat| {
                stat.queryid
                    .map(|id| query_ids.contains(&id))
                    .unwrap_or(false)
            })
            .collect();
        
        tracing::info!("Collected {} current query metrics", affected_stats.len());
        Ok(affected_stats)
    }
    
    /// Collect current system metrics
    async fn collect_current_system_metrics(
        &self,
        collector: &dyn SystemMetricsCollector,
    ) -> Result<SystemMetrics> {
        collector.collect_metrics(None).await
    }
    
    /// Compare database metrics
    fn compare_db_metrics(
        &self,
        baseline: &[QueryStats],
        current: &[QueryStats],
    ) -> Result<DbMetricsComparison> {
        if baseline.is_empty() || current.is_empty() {
            return Ok(DbMetricsComparison::default());
        }
        
        // Calculate aggregated metrics
        let baseline_p95 = calculate_percentile(
            &baseline.iter().map(|s| s.mean_exec_time_ms).collect::<Vec<_>>(),
            0.95
        );
        let current_p95 = calculate_percentile(
            &current.iter().map(|s| s.mean_exec_time_ms).collect::<Vec<_>>(),
            0.95
        );
        
        let baseline_mean: f64 = baseline.iter().map(|s| s.mean_exec_time_ms).sum::<f64>()
            / baseline.len() as f64;
        let current_mean: f64 = current.iter().map(|s| s.mean_exec_time_ms).sum::<f64>()
            / current.len() as f64;
        
        let baseline_total_calls: i64 = baseline.iter().map(|s| s.calls).sum();
        let current_total_calls: i64 = current.iter().map(|s| s.calls).sum();
        
        Ok(DbMetricsComparison {
            p95_latency_improvement_pct: calculate_improvement_pct(baseline_p95, current_p95),
            mean_latency_improvement_pct: calculate_improvement_pct(baseline_mean, current_mean),
            throughput_change_pct: calculate_change_pct(baseline_total_calls as f64, current_total_calls as f64),
            error_rate_change_pct: 0.0, // TODO: Track error rates
            baseline_p95_ms: baseline_p95,
            current_p95_ms: current_p95,
            baseline_mean_ms: baseline_mean,
            current_mean_ms: current_mean,
        })
    }
    
    /// Compare system metrics
    fn compare_system_metrics(
        &self,
        baseline: &SystemMetrics,
        current: &SystemMetrics,
    ) -> Result<SystemMetricsComparison> {
        Ok(SystemMetricsComparison {
            cpu_time_improvement_pct: calculate_improvement_pct(
                baseline.cpu_time_us as f64,
                current.cpu_time_us as f64
            ),
            io_wait_improvement_pct: calculate_improvement_pct(
                baseline.io_wait_time_us as f64,
                current.io_wait_time_us as f64
            ),
            page_cache_miss_improvement_pct: calculate_improvement_pct(
                baseline.page_cache_misses as f64,
                current.page_cache_misses as f64
            ),
            context_switch_change_pct: calculate_change_pct(
                baseline.context_switches as f64,
                current.context_switches as f64
            ),
        })
    }
    
    /// Evaluate against success criteria
    fn evaluate_success_criteria(
        &self,
        db_comparison: &DbMetricsComparison,
        system_comparison: Option<&SystemMetricsComparison>,
    ) -> Result<CriteriaEvaluation> {
        use crate::config::SuccessMetric;
        
        let mut passed_criteria = 0;
        let mut total_criteria = self.success_criteria.len();
        let mut details = Vec::new();
        
        for criterion in &self.success_criteria {
            let met = match criterion.metric {
                SuccessMetric::P95Latency => {
                    db_comparison.p95_latency_improvement_pct >= criterion.min_improvement_pct
                }
                SuccessMetric::MeanLatency => {
                    db_comparison.mean_latency_improvement_pct >= criterion.min_improvement_pct
                }
                SuccessMetric::Throughput => {
                    db_comparison.throughput_change_pct >= criterion.min_improvement_pct
                }
                SuccessMetric::ErrorRate => {
                    // Negative change is improvement for error rate
                    -db_comparison.error_rate_change_pct >= criterion.min_improvement_pct
                }
                SuccessMetric::CpuTime => {
                    system_comparison
                        .map(|sc| sc.cpu_time_improvement_pct >= criterion.min_improvement_pct)
                        .unwrap_or(false)
                }
                SuccessMetric::IoWait => {
                    system_comparison
                        .map(|sc| sc.io_wait_improvement_pct >= criterion.min_improvement_pct)
                        .unwrap_or(false)
                }
            };
            
            if met {
                passed_criteria += 1;
            }
            
            details.push(format!(
                "{:?}: {} (target: {}%)",
                criterion.metric,
                if met { "✓ PASS" } else { "✗ FAIL" },
                criterion.min_improvement_pct
            ));
        }
        
        // All criteria must pass
        let success = passed_criteria == total_criteria;
        
        // Calculate confidence based on sample size and consistency
        let confidence = if success {
            0.8 + (passed_criteria as f64 / total_criteria as f64) * 0.2
        } else {
            0.3 + (passed_criteria as f64 / total_criteria as f64) * 0.3
        };
        
        Ok(CriteriaEvaluation {
            success,
            confidence: confidence.min(1.0),
            details,
        })
    }
}

/// Baseline metrics captured before applying a change
#[derive(Debug, Clone)]
pub struct BaselineMetrics {
    pub captured_at: DateTime<Utc>,
    pub query_ids: Vec<i64>,
    pub db_metrics: Vec<QueryStats>,
    pub system_metrics: Option<SystemMetrics>,
}

/// Result of verification
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub recommendation_id: String,
    pub verified_at: DateTime<Utc>,
    pub db_comparison: DbMetricsComparison,
    pub system_comparison: Option<SystemMetricsComparison>,
    pub meets_criteria: bool,
    pub confidence: f64,
    pub details: Vec<String>,
}

impl VerificationResult {
    /// Convert to ApplyResult
    pub fn to_apply_result(&self, baseline: PerformanceBaseline, post_apply: PerformanceBaseline) -> ApplyResult {
        let status = if self.meets_criteria {
            ApplyStatus::Success
        } else {
            if self.confidence < 0.5 {
                ApplyStatus::Failed
            } else {
                ApplyStatus::RolledBack
            }
        };
        
        let error_message = if !self.meets_criteria {
            Some(format!(
                "Verification failed: P95 latency improved by {:.1}%, Mean latency improved by {:.1}%",
                self.db_comparison.p95_latency_improvement_pct,
                self.db_comparison.mean_latency_improvement_pct
            ))
        } else {
            None
        };
        
        ApplyResult {
            recommendation_id: self.recommendation_id.clone(),
            started_at: baseline.captured_at,
            completed_at: self.verified_at,
            duration_secs: (self.verified_at - baseline.captured_at).num_seconds() as f64,
            status,
            error_message,
            baseline,
            post_apply: Some(post_apply),
            actual_improvement_pct: Some(self.db_comparison.p95_latency_improvement_pct),
            met_threshold: self.meets_criteria,
            rollback_executed: !self.meets_criteria,
        }
    }
}

/// Database metrics comparison
#[derive(Debug, Clone, Default)]
pub struct DbMetricsComparison {
    pub p95_latency_improvement_pct: f64,
    pub mean_latency_improvement_pct: f64,
    pub throughput_change_pct: f64,
    pub error_rate_change_pct: f64,
    pub baseline_p95_ms: f64,
    pub current_p95_ms: f64,
    pub baseline_mean_ms: f64,
    pub current_mean_ms: f64,
}

/// System metrics comparison
#[derive(Debug, Clone)]
pub struct SystemMetricsComparison {
    pub cpu_time_improvement_pct: f64,
    pub io_wait_improvement_pct: f64,
    pub page_cache_miss_improvement_pct: f64,
    pub context_switch_change_pct: f64,
}

/// Criteria evaluation result
struct CriteriaEvaluation {
    success: bool,
    confidence: f64,
    details: Vec<String>,
}

/// Calculate percentile from sorted values
fn calculate_percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    let index = ((sorted.len() as f64) * percentile) as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// Calculate improvement percentage
fn calculate_improvement_pct(baseline: f64, current: f64) -> f64 {
    if baseline == 0.0 {
        return 0.0;
    }
    ((baseline - current) / baseline) * 100.0
}

/// Calculate change percentage
fn calculate_change_pct(baseline: f64, current: f64) -> f64 {
    if baseline == 0.0 {
        return 0.0;
    }
    ((current - baseline) / baseline) * 100.0
}
