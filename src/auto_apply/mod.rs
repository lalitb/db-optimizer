pub mod safety_filter;
pub mod baseline;
pub mod executor;
pub mod verifier;
pub mod notifier;

use crate::models::*;
use anyhow::Result;
use sqlx::PgPool;
use chrono::Utc;

use safety_filter::SafetyFilter;
use baseline::BaselineCollector;
use executor::Executor;
use verifier::Verifier;
use notifier::Notifier;

pub struct AutoApplyOrchestrator {
    pool: PgPool,
    config: AutoApplyConfig,
    safety_filter: SafetyFilter,
    baseline_collector: BaselineCollector,
    executor: Executor,
    verifier: Verifier,
    notifier: Option<Notifier>,
}

impl AutoApplyOrchestrator {
    pub fn new(pool: PgPool, config: AutoApplyConfig) -> Self {
        let safety_filter = SafetyFilter::new(config.clone());
        let baseline_collector = BaselineCollector::new(pool.clone());
        let executor = Executor::new(pool.clone(), config.clone());
        let verifier = Verifier::new(pool.clone(), config.clone());
        let notifier = config.slack_webhook.as_ref().map(|webhook| {
            Notifier::new(webhook.clone())
        });
        
        Self {
            pool,
            config,
            safety_filter,
            baseline_collector,
            executor,
            verifier,
            notifier,
        }
    }
    
    /// Process and optionally apply a single recommendation
    pub async fn process_recommendation(
        &self,
        rec: &Recommendation,
    ) -> Result<ApplyResult> {
        tracing::info!("Processing recommendation: {}", rec.title);
        
        // Phase 1: Safety assessment
        let applicable = self.safety_filter.assess(rec);
        
        if !applicable.can_auto_apply {
            tracing::info!("Skipping: Cannot auto-apply (risk or config)");
            return Ok(self.create_skipped_result(rec, "Risk or configuration prevents auto-apply"));
        }
        
        if applicable.requires_approval {
            tracing::info!("Requires manual approval");
            self.notify_approval_required(&applicable).await?;
            return Ok(self.create_pending_result(rec));
        }
        
        let started_at = Utc::now();
        
        // Phase 2: Capture baseline
        tracing::info!("Capturing baseline metrics...");
        let baseline = self.baseline_collector.capture(&applicable).await?;
        
        // Phase 3: Execute
        tracing::info!("Executing recommendation...");
        self.notify_execution_start(&applicable).await?;
        
        let exec_result = self.executor.execute(&applicable).await?;
        
        if exec_result.status != executor::ExecutionStatus::Success {
            tracing::error!("Execution failed: {:?}", exec_result.error_message);
            self.notify_execution_failed(&applicable, &exec_result).await?;
            
            return Ok(ApplyResult {
                recommendation_id: rec.id.clone(),
                started_at,
                completed_at: Utc::now(),
                duration_secs: (Utc::now() - started_at).num_seconds() as f64,
                status: ApplyStatus::Failed,
                error_message: exec_result.error_message,
                baseline: baseline.clone(),
                post_apply: None,
                actual_improvement_pct: None,
                met_threshold: false,
                rollback_executed: false,
            });
        }
        
        // Phase 4: Verify impact
        tracing::info!("Verifying impact...");
        let verification = self.verifier.verify(&applicable, &baseline).await?;
        
        // Phase 5: Decision
        let mut rollback_executed = false;
        let final_status = if verification.should_rollback {
            tracing::warn!(
                "Improvement ({:.1}%) below threshold ({:.1}%). Rolling back...",
                verification.improvement_pct,
                self.config.min_improvement_threshold
            );
            
            self.executor.rollback(&applicable).await?;
            rollback_executed = true;
            self.notify_rollback(&applicable, &verification).await?;
            
            ApplyStatus::RolledBack
        } else {
            tracing::info!(
                "Success! Improvement: {:.1}%",
                verification.improvement_pct
            );
            self.notify_success(&applicable, &verification).await?;
            
            ApplyStatus::Success
        };
        
        let completed_at = Utc::now();
        
        Ok(ApplyResult {
            recommendation_id: rec.id.clone(),
            started_at,
            completed_at,
            duration_secs: (completed_at - started_at).num_seconds() as f64,
            status: final_status,
            error_message: None,
            baseline,
            post_apply: Some(verification.post_apply),
            actual_improvement_pct: Some(verification.improvement_pct),
            met_threshold: verification.met_threshold,
            rollback_executed,
        })
    }
    
    /// Process multiple recommendations
    pub async fn process_all(
        &self,
        recommendations: &[Recommendation],
    ) -> Vec<ApplyResult> {
        let mut results = Vec::new();
        
        for rec in recommendations {
            match self.process_recommendation(rec).await {
                Ok(result) => results.push(result),
                Err(e) => {
                    tracing::error!("Failed to process recommendation {}: {}", rec.id, e);
                    results.push(self.create_failed_result(rec, &e.to_string()));
                }
            }
        }
        
        results
    }
    
    // Helper methods for creating results
    fn create_skipped_result(&self, rec: &Recommendation, reason: &str) -> ApplyResult {
        ApplyResult {
            recommendation_id: rec.id.clone(),
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_secs: 0.0,
            status: ApplyStatus::Skipped,
            error_message: Some(reason.to_string()),
            baseline: self.empty_baseline(),
            post_apply: None,
            actual_improvement_pct: None,
            met_threshold: false,
            rollback_executed: false,
        }
    }
    
    fn create_pending_result(&self, rec: &Recommendation) -> ApplyResult {
        ApplyResult {
            recommendation_id: rec.id.clone(),
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_secs: 0.0,
            status: ApplyStatus::PendingApproval,
            error_message: None,
            baseline: self.empty_baseline(),
            post_apply: None,
            actual_improvement_pct: None,
            met_threshold: false,
            rollback_executed: false,
        }
    }
    
    fn create_failed_result(&self, rec: &Recommendation, error: &str) -> ApplyResult {
        ApplyResult {
            recommendation_id: rec.id.clone(),
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_secs: 0.0,
            status: ApplyStatus::Failed,
            error_message: Some(error.to_string()),
            baseline: self.empty_baseline(),
            post_apply: None,
            actual_improvement_pct: None,
            met_threshold: false,
            rollback_executed: false,
        }
    }
    
    fn empty_baseline(&self) -> PerformanceBaseline {
        PerformanceBaseline {
            captured_at: Utc::now(),
            query_id: None,
            table_name: None,
            mean_exec_time_ms: 0.0,
            p95_exec_time_ms: 0.0,
            p99_exec_time_ms: 0.0,
            calls_per_minute: 0.0,
            query_plan: None,
            plan_cost: None,
            table_size_mb: None,
            index_size_mb: None,
            total_queries_analyzed: 0,
        }
    }
    
    // Notification helpers
    async fn notify_approval_required(&self, rec: &ApplicableRecommendation) -> Result<()> {
        if let Some(notifier) = &self.notifier {
            notifier.send_approval_request(rec).await?;
        }
        Ok(())
    }
    
    async fn notify_execution_start(&self, rec: &ApplicableRecommendation) -> Result<()> {
        if let Some(notifier) = &self.notifier {
            notifier.send_execution_start(rec).await?;
        }
        Ok(())
    }
    
    async fn notify_execution_failed(
        &self,
        rec: &ApplicableRecommendation,
        result: &executor::ExecutionResult,
    ) -> Result<()> {
        if let Some(notifier) = &self.notifier {
            notifier.send_execution_failed(rec, result).await?;
        }
        Ok(())
    }
    
    async fn notify_success(
        &self,
        rec: &ApplicableRecommendation,
        verification: &verifier::VerificationResult,
    ) -> Result<()> {
        if let Some(notifier) = &self.notifier {
            notifier.send_success(rec, verification).await?;
        }
        Ok(())
    }
    
    async fn notify_rollback(
        &self,
        rec: &ApplicableRecommendation,
        verification: &verifier::VerificationResult,
    ) -> Result<()> {
        if let Some(notifier) = &self.notifier {
            notifier.send_rollback(rec, verification).await?;
        }
        Ok(())
    }
}
