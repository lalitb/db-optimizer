use serde::{Deserialize, Serialize};
use anyhow::Result;

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub database_url: String,
    pub query_limit: i64,
    pub cost_per_hour_usd: f64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL")?,
            query_limit: std::env::var("QUERY_LIMIT")
                .unwrap_or_else(|_| "100".to_string())
                .parse()?,
            cost_per_hour_usd: std::env::var("COST_PER_HOUR")
                .unwrap_or_else(|_| "0.07".to_string())
                .parse()?,
        })
    }
}
