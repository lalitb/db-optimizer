//! AI/LLM integration for intelligent database recommendations
//!
//! Supports multiple LLM providers:
//! - OpenAI (GPT-4, GPT-3.5)
//! - Anthropic (Claude)
//! - Ollama (local models)

mod providers;

pub use providers::{AnthropicClient, LlmClient, LlmProvider, OllamaClient, OpenAiClient};

use crate::models::{IndexInfo, IssueType, QueryStats, Recommendation, Severity, TableInfo};
use anyhow::{Context, Result};
use serde::Deserialize;

/// Configuration for AI features
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiConfig {
    /// Enable AI-powered recommendations
    #[serde(default)]
    pub enabled: bool,

    /// LLM provider to use
    #[serde(default)]
    pub provider: LlmProvider,

    /// API key (for cloud providers)
    #[serde(default)]
    pub api_key: Option<String>,

    /// Model name to use
    #[serde(default)]
    pub model: Option<String>,

    /// Base URL (for Ollama or custom endpoints)
    #[serde(default)]
    pub base_url: Option<String>,

    /// Maximum queries to analyze with AI (to control costs)
    #[serde(default = "default_max_queries")]
    pub max_queries_to_analyze: usize,

    /// Only analyze queries slower than this threshold (ms)
    #[serde(default = "default_slow_threshold")]
    pub slow_query_threshold_ms: f64,
}

fn default_max_queries() -> usize {
    5
}

fn default_slow_threshold() -> f64 {
    100.0
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: LlmProvider::OpenAi,
            api_key: None,
            model: None,
            base_url: None,
            max_queries_to_analyze: 5,
            slow_query_threshold_ms: 100.0,
        }
    }
}

impl AiConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let provider = std::env::var("DBO_AI_PROVIDER")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(LlmProvider::OpenAi);

        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .or_else(|| std::env::var("DBO_AI_API_KEY").ok());

        let enabled = api_key.is_some()
            || provider == LlmProvider::Ollama
            || std::env::var("DBO_AI_ENABLED")
                .ok()
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);

        Self {
            enabled,
            provider,
            api_key,
            model: std::env::var("DBO_AI_MODEL").ok(),
            base_url: std::env::var("DBO_AI_BASE_URL")
                .ok()
                .or_else(|| std::env::var("OLLAMA_HOST").ok()),
            max_queries_to_analyze: std::env::var("DBO_AI_MAX_QUERIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            slow_query_threshold_ms: std::env::var("DBO_AI_SLOW_THRESHOLD_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
        }
    }

    /// Build an LLM client from this config
    pub fn build_client(&self) -> Result<Box<dyn LlmClient>> {
        match self.provider {
            LlmProvider::OpenAi => {
                let api_key = self
                    .api_key
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                    .context("OpenAI API key required (set OPENAI_API_KEY or api_key in config)")?;
                let model = self
                    .model
                    .clone()
                    .unwrap_or_else(|| "gpt-4o-mini".to_string());
                Ok(Box::new(OpenAiClient::new(api_key, model)))
            }
            LlmProvider::Anthropic => {
                let api_key = self
                    .api_key
                    .clone()
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                    .context(
                        "Anthropic API key required (set ANTHROPIC_API_KEY or api_key in config)",
                    )?;
                let model = self
                    .model
                    .clone()
                    .unwrap_or_else(|| "claude-3-haiku-20240307".to_string());
                Ok(Box::new(AnthropicClient::new(api_key, model)))
            }
            LlmProvider::Ollama => {
                let base_url = self
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                let model = self.model.clone().unwrap_or_else(|| "llama3.2".to_string());
                Ok(Box::new(OllamaClient::new(base_url, model)))
            }
        }
    }
}

/// AI-powered query advisor
pub struct AiAdvisor {
    client: Box<dyn LlmClient>,
    config: AiConfig,
}

impl AiAdvisor {
    pub fn new(config: AiConfig) -> Result<Self> {
        let client = config.build_client()?;
        Ok(Self { client, config })
    }

    /// Analyze slow queries and generate AI-powered recommendations
    pub async fn analyze_queries(
        &self,
        queries: &[QueryStats],
        tables: &[TableInfo],
        indexes: &[IndexInfo],
    ) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();

        // Filter to slow queries only, limit by config
        let slow_queries: Vec<_> = queries
            .iter()
            .filter(|q| q.mean_exec_time_ms >= self.config.slow_query_threshold_ms)
            .take(self.config.max_queries_to_analyze)
            .collect();

        for query in slow_queries {
            match self.analyze_single_query(query, tables, indexes).await {
                Ok(Some(rec)) => recommendations.push(rec),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("AI analysis failed for query: {}", e);
                }
            }
        }

        recommendations
    }

    /// Analyze a single query
    async fn analyze_single_query(
        &self,
        query: &QueryStats,
        tables: &[TableInfo],
        indexes: &[IndexInfo],
    ) -> Result<Option<Recommendation>> {
        let prompt = build_analysis_prompt(query, tables, indexes);
        let response = self.client.complete(&prompt).await?;

        parse_ai_response(&response, query)
    }

    /// Generate a natural language explanation for a recommendation
    pub async fn explain_recommendation(&self, rec: &Recommendation) -> Result<String> {
        let prompt = format!(
            "Explain this PostgreSQL optimization recommendation in plain English for a developer:\n\n\
            Title: {}\n\
            Description: {}\n\
            SQL to apply: {}\n\n\
            Provide a clear, concise explanation of:\n\
            1. What the issue is\n\
            2. Why this fix helps\n\
            3. Any risks to be aware of\n\
            Keep it under 200 words.",
            rec.title, rec.description, rec.migration_sql
        );

        self.client.complete(&prompt).await
    }

    /// Answer a natural language question about the database
    pub async fn ask(&self, question: &str, context: &str) -> Result<String> {
        let prompt = format!(
            "You are a PostgreSQL expert assistant.\n\n\
            Database Context:\n{}\n\n\
            Question: {}\n\n\
            Provide a concise, actionable answer.",
            context, question
        );

        self.client.complete(&prompt).await
    }
}

fn build_analysis_prompt(
    query: &QueryStats,
    tables: &[TableInfo],
    indexes: &[IndexInfo],
) -> String {
    let mut prompt = String::with_capacity(2000);

    prompt.push_str("Analyze this slow PostgreSQL query and suggest optimizations.\n\n");
    prompt.push_str(&format!("QUERY:\n{}\n\n", truncate(&query.query, 500)));
    prompt.push_str(&format!(
        "STATS:\n- Mean execution time: {:.2}ms\n- Calls: {}\n- Total time: {:.2}ms\n\n",
        query.mean_exec_time_ms, query.calls, query.total_exec_time_ms
    ));

    if !tables.is_empty() {
        prompt.push_str("TABLES:\n");
        for t in tables.iter().take(10) {
            prompt.push_str(&format!(
                "- {} ({} rows, {:.1}MB)\n",
                t.table_name, t.row_count, t.table_size_mb
            ));
        }
        prompt.push('\n');
    }

    if !indexes.is_empty() {
        prompt.push_str("EXISTING INDEXES:\n");
        for idx in indexes.iter().take(20) {
            prompt.push_str(&format!("- {} on {}\n", idx.index_name, idx.table_name));
        }
        prompt.push('\n');
    }

    prompt.push_str(
        "Respond with ONLY valid JSON (no markdown):\n\
        {\n\
          \"needs_optimization\": true/false,\n\
          \"title\": \"Brief issue title\",\n\
          \"description\": \"What's wrong and why\",\n\
          \"severity\": \"LOW|MEDIUM|HIGH|CRITICAL\",\n\
          \"sql\": \"SQL statement to fix (CREATE INDEX, etc)\",\n\
          \"explanation\": \"Plain English explanation\"\n\
        }",
    );

    prompt
}

fn parse_ai_response(response: &str, query: &QueryStats) -> Result<Option<Recommendation>> {
    // Extract JSON from response (handle markdown code blocks)
    let json_str = extract_json(response);

    let parsed: AiAnalysisResponse =
        serde_json::from_str(json_str).context("Failed to parse AI response as JSON")?;

    if !parsed.needs_optimization {
        return Ok(None);
    }

    Ok(Some(Recommendation {
        id: format!("ai_{}", query.queryid.unwrap_or(0)),
        title: format!("🤖 {}", parsed.title),
        description: format!(
            "{}\n\n💡 AI Explanation: {}",
            parsed.description, parsed.explanation
        ),
        issue_type: IssueType::SlowQuery,
        severity: parse_severity(&parsed.severity),
        affected_queries_count: Some(query.calls),
        estimated_time_savings_ms: Some(query.mean_exec_time_ms * 0.3), // Estimate 30% improvement
        estimated_cost_savings_usd_monthly: None,
        migration_sql: parsed.sql,
        rollback_sql: None,
        safety_notes: "Generated by AI. Review carefully before applying.".to_string(),
        affected_table: None,
        affected_query: Some(truncate(&query.query, 200)),
    }))
}

fn extract_json(text: &str) -> &str {
    // Try to find JSON in markdown code block
    if let Some(start) = text.find("```json") {
        if let Some(end) = text[start + 7..].find("```") {
            return text[start + 7..start + 7 + end].trim();
        }
    }
    if let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            return text[start + 3..start + 3 + end].trim();
        }
    }
    // Try to find raw JSON object
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return &text[start..=end];
        }
    }
    text.trim()
}

fn parse_severity(s: &str) -> Severity {
    match s.to_uppercase().as_str() {
        "CRITICAL" => Severity::Critical,
        "HIGH" => Severity::High,
        "MEDIUM" => Severity::Medium,
        _ => Severity::Low,
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[derive(Debug, Deserialize)]
struct AiAnalysisResponse {
    needs_optimization: bool,
    title: String,
    description: String,
    severity: String,
    sql: String,
    explanation: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json() {
        let with_markdown = "Here's the analysis:\n```json\n{\"test\": true}\n```\nDone.";
        assert_eq!(extract_json(with_markdown), "{\"test\": true}");

        let raw_json = "{\"test\": true}";
        assert_eq!(extract_json(raw_json), "{\"test\": true}");

        let with_text = "The result is {\"test\": true} as shown.";
        assert_eq!(extract_json(with_text), "{\"test\": true}");
    }

    #[test]
    fn test_parse_severity() {
        assert_eq!(parse_severity("CRITICAL"), Severity::Critical);
        assert_eq!(parse_severity("high"), Severity::High);
        assert_eq!(parse_severity("Medium"), Severity::Medium);
        assert_eq!(parse_severity("low"), Severity::Low);
        assert_eq!(parse_severity("unknown"), Severity::Low);
    }

    #[test]
    fn test_ai_config_default() {
        let config = AiConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.provider, LlmProvider::OpenAi);
        assert_eq!(config.max_queries_to_analyze, 5);
    }
}
