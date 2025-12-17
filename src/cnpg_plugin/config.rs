use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct PluginConfig {
    pub api_key: Option<String>,
    pub enable_auto_optimize: bool,
    pub health_threshold: u8,
    pub check_interval_seconds: u64,
    pub auto_apply_config: AutoApplyConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct AutoApplyConfig {
    pub enabled: bool,
    pub min_savings_threshold: f64,
    pub max_risk_level: String,
    pub verification_duration_secs: u64,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            enable_auto_optimize: false,
            health_threshold: 70,
            check_interval_seconds: 300, // 5 minutes
            auto_apply_config: AutoApplyConfig::default(),
        }
    }
}

impl Default for AutoApplyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_savings_threshold: 50.0,
            max_risk_level: "Medium".to_string(),
            verification_duration_secs: 300,
        }
    }
}

pub fn load_plugin_config(path: Option<PathBuf>) -> Result<PluginConfig> {
    if let Some(p) = path {
        if p.exists() {
            let content = std::fs::read_to_string(p)?;
            return Ok(serde_yaml::from_str(&content)?);
        }
    }

    // Fallback to environment variables or default
    Ok(PluginConfig::default())
}
