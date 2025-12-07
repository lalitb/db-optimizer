use super::models::{AgentMetrics, AgentStatus};
use anyhow::Result;
use reqwest::Client;
use std::time::Duration;

#[derive(Clone)]
pub struct AgentClient {
    base_url: String,
    client: Client,
}

impl AgentClient {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    pub async fn get_status(&self) -> Result<AgentStatus> {
        let url = format!("{}/v1/health", self.base_url);
        let status = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<AgentStatus>()
            .await?;
        Ok(status)
    }

    pub async fn get_metrics(&self) -> Result<AgentMetrics> {
        let url = format!("{}/v1/metrics", self.base_url);
        let metrics = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<AgentMetrics>()
            .await?;
        Ok(metrics)
    }
}
