use crate::models::*;
use crate::auto_apply::{executor, verifier};
use anyhow::Result;

pub struct Notifier {
    slack_webhook: String,
    client: reqwest::Client,
}

impl Notifier {
    pub fn new(slack_webhook: String) -> Self {
        Self {
            slack_webhook,
            client: reqwest::Client::new(),
        }
    }
    
    pub async fn send_approval_request(&self, rec: &ApplicableRecommendation) -> Result<()> {
        let message = format!(
            "🔔 *Manual Approval Required*\n\n\
            *Recommendation:* {}\n\
            *Risk Level:* {:?}\n\
            *Estimated Savings:* ${:.0}/month\n\n\
            Run `pg-query-advisor approve {}` to proceed",
            rec.recommendation.title,
            rec.risk_assessment.risk_level,
            rec.recommendation.estimated_cost_savings_usd_monthly.unwrap_or(0.0),
            rec.recommendation.id
        );
        
        self.send_slack_message(&message).await
    }
    
    pub async fn send_execution_start(&self, rec: &ApplicableRecommendation) -> Result<()> {
        let message = format!(
            "🚀 *Starting Optimization*\n\n\
            {}\n\
            Estimated duration: {} seconds",
            rec.recommendation.title,
            rec.estimated_duration_secs
        );
        
        self.send_slack_message(&message).await
    }
    
    pub async fn send_execution_failed(
        &self,
        rec: &ApplicableRecommendation,
        result: &executor::ExecutionResult,
    ) -> Result<()> {
        let message = format!(
            "❌ *Optimization Failed*\n\n\
            *Recommendation:* {}\n\
            *Error:* {}\n\n\
            No changes were applied.",
            rec.recommendation.title,
            result.error_message.as_deref().unwrap_or("Unknown error")
        );
        
        self.send_slack_message(&message).await
    }
    
    pub async fn send_success(
        &self,
        rec: &ApplicableRecommendation,
        verification: &verifier::VerificationResult,
    ) -> Result<()> {
        let message = format!(
            "✅ *Optimization Successful*\n\n\
            *Recommendation:* {}\n\
            *Improvement:* {:.1}%\n\
            *Est. Savings:* ${:.0}/month\n\n\
            Your database is now faster!",
            rec.recommendation.title,
            verification.improvement_pct,
            rec.recommendation.estimated_cost_savings_usd_monthly.unwrap_or(0.0)
        );
        
        self.send_slack_message(&message).await
    }
    
    pub async fn send_rollback(
        &self,
        rec: &ApplicableRecommendation,
        verification: &verifier::VerificationResult,
    ) -> Result<()> {
        let message = format!(
            "⚠️ *Optimization Rolled Back*\n\n\
            *Recommendation:* {}\n\
            *Improvement:* {:.1}% (below {:.0}% threshold)\n\n\
            Change was automatically rolled back. No impact to your database.",
            rec.recommendation.title,
            verification.improvement_pct,
            10.0 // TODO: Get from config
        );
        
        self.send_slack_message(&message).await
    }
    
    async fn send_slack_message(&self, message: &str) -> Result<()> {
        let payload = serde_json::json!({
            "text": message
        });
        
        let response = self.client
            .post(&self.slack_webhook)
            .json(&payload)
            .send()
            .await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Slack notification failed: {}", response.status());
        }
        
        Ok(())
    }
}
