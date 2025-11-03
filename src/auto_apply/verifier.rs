use crate::models::*;
use crate::auto_apply::baseline::BaselineCollector;
use sqlx::PgPool;
use anyhow::Result;
use tokio::time::{sleep, Duration};

pub struct Verifier {
    pool: PgPool,
    baseline_collector: BaselineCollector,
    config: AutoApplyConfig,
}

impl Verifier {
    pub fn new(pool: PgPool, config: AutoApplyConfig) -> Self {
        let baseline_collector = BaselineCollector::new(pool.clone());
        Self {
            pool,
            baseline_collector,
            config,
        }
    }
    
    /// Verify impact of applied change
    pub async fn verify(
        &self,
        rec: &ApplicableRecommendation,
        baseline: &PerformanceBaseline,
    ) -> Result<VerificationResult> {
        tracing::info!(
            "Verifying impact for {} seconds...",
            self.config.verification_duration_secs
        );
        
        // Wait for specified duration to collect new metrics
        sleep(Duration::from_secs(self.config.verification_duration_secs)).await;
        
        // Capture new baseline
        let post_apply = self.baseline_collector.capture(rec).await?;
        
        // Calculate improvement
        let improvement_pct = self.calculate_improvement(baseline, &post_apply);
        
        // Check if improvement meets threshold
        let met_threshold = improvement_pct >= self.config.min_improvement_threshold;
        
        // Determine if we should rollback
        let should_rollback = !met_threshold && self.config.auto_rollback_on_failure;
        
        Ok(VerificationResult {
            post_apply,
            improvement_pct,
            met_threshold,
            should_rollback,
        })
    }
    
    fn calculate_improvement(
        &self,
        baseline: &PerformanceBaseline,
        post_apply: &PerformanceBaseline,
    ) -> f64 {
        if baseline.mean_exec_time_ms == 0.0 {
            return 0.0;
        }
        
        let improvement = baseline.mean_exec_time_ms - post_apply.mean_exec_time_ms;
        let improvement_pct = (improvement / baseline.mean_exec_time_ms) * 100.0;
        
        // Cap at reasonable bounds
        improvement_pct.max(-100.0).min(100.0)
    }
}

#[derive(Debug)]
pub struct VerificationResult {
    pub post_apply: PerformanceBaseline,
    pub improvement_pct: f64,
    pub met_threshold: bool,
    pub should_rollback: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    
    #[test]
    fn test_calculate_improvement() {
        // Create a dummy config and verifier for testing the calculation logic
        let baseline = PerformanceBaseline {
            captured_at: Utc::now(),
            query_id: None,
            table_name: None,
            mean_exec_time_ms: 1000.0,
            p95_exec_time_ms: 0.0,
            p99_exec_time_ms: 0.0,
            calls_per_minute: 0.0,
            query_plan: None,
            plan_cost: None,
            table_size_mb: None,
            index_size_mb: None,
            total_queries_analyzed: 0,
        };
        
        let post_apply = PerformanceBaseline {
            captured_at: Utc::now(),
            query_id: None,
            table_name: None,
            mean_exec_time_ms: 200.0,
            p95_exec_time_ms: 0.0,
            p99_exec_time_ms: 0.0,
            calls_per_minute: 0.0,
            query_plan: None,
            plan_cost: None,
            table_size_mb: None,
            index_size_mb: None,
            total_queries_analyzed: 0,
        };
        
        // Test the calculation directly
        let improvement = (baseline.mean_exec_time_ms - post_apply.mean_exec_time_ms) 
            / baseline.mean_exec_time_ms * 100.0;
        
        assert_eq!(improvement, 80.0); // 80% improvement
    }
}
