// src/azure/metadata.rs - Azure Instance Metadata Service integration

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Azure instance metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureMetadata {
    pub subscription_id: String,
    pub resource_group: String,
    pub vm_id: String,
    pub region: String,
    pub vm_size: String,
    pub zone: Option<String>,
    
    // Postgres-specific metadata (if applicable)
    pub postgres_server_name: Option<String>,
    pub postgres_sku: Option<String>,
    pub postgres_tier: Option<String>,
    pub postgres_version: Option<String>,
}

/// Azure IMDS response structure
#[derive(Debug, Deserialize)]
struct ImdsCompute {
    #[serde(rename = "subscriptionId")]
    subscription_id: String,
    #[serde(rename = "resourceGroupName")]
    resource_group: String,
    #[serde(rename = "vmId")]
    vm_id: String,
    location: String,
    #[serde(rename = "vmSize")]
    vm_size: String,
    zone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImdsResponse {
    compute: ImdsCompute,
}

/// Collector for Azure metadata
pub struct AzureMetadataCollector {
    client: reqwest::Client,
    imds_endpoint: String,
}

impl AzureMetadataCollector {
    /// Create a new Azure metadata collector
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            imds_endpoint: "http://169.254.169.254/metadata/instance".to_string(),
        }
    }
    
    /// Check if running in Azure
    pub async fn is_azure(&self) -> bool {
        self.client
            .get(&format!("{}?api-version=2021-02-01", self.imds_endpoint))
            .header("Metadata", "true")
            .send()
            .await
            .is_ok()
    }
    
    /// Collect metadata from Azure IMDS
    pub async fn collect(&self) -> Result<AzureMetadata, Box<dyn std::error::Error>> {
        let response = self.client
            .get(&format!("{}?api-version=2021-02-01", self.imds_endpoint))
            .header("Metadata", "true")
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(format!("IMDS returned status: {}", response.status()).into());
        }
        
        let imds: ImdsResponse = response.json().await?;
        
        // Try to detect if this is a Postgres Flexible Server
        let (postgres_server_name, postgres_sku, postgres_tier, postgres_version) = 
            self.detect_postgres_metadata(&imds.compute.resource_group).await;
        
        Ok(AzureMetadata {
            subscription_id: imds.compute.subscription_id,
            resource_group: imds.compute.resource_group,
            vm_id: imds.compute.vm_id,
            region: imds.compute.location,
            vm_size: imds.compute.vm_size,
            zone: imds.compute.zone,
            postgres_server_name,
            postgres_sku,
            postgres_tier,
            postgres_version,
        })
    }
    
    /// Detect Postgres-specific metadata
    async fn detect_postgres_metadata(
        &self,
        resource_group: &str,
    ) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
        // This would require Azure SDK or REST API calls
        // For now, return None - can be enhanced with proper Azure authentication
        (None, None, None, None)
    }
    
    /// Get metadata with fallback values
    pub async fn collect_or_default(&self) -> AzureMetadata {
        self.collect().await.unwrap_or_else(|_| AzureMetadata {
            subscription_id: "unknown".to_string(),
            resource_group: "unknown".to_string(),
            vm_id: "unknown".to_string(),
            region: "unknown".to_string(),
            vm_size: "unknown".to_string(),
            zone: None,
            postgres_server_name: None,
            postgres_sku: None,
            postgres_tier: None,
            postgres_version: None,
        })
    }
}

impl Default for AzureMetadataCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_metadata_collector_creation() {
        let collector = AzureMetadataCollector::new();
        assert_eq!(collector.imds_endpoint, "http://169.254.169.254/metadata/instance");
    }
    
    #[tokio::test]
    async fn test_metadata_default() {
        let collector = AzureMetadataCollector::new();
        let metadata = collector.collect_or_default().await;
        
        // Should return default values when not in Azure
        assert!(!metadata.subscription_id.is_empty());
    }
}
