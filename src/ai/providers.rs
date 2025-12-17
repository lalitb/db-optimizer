//! LLM provider implementations

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// LLM provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    OpenAi,
    Anthropic,
    Ollama,
}

impl FromStr for LlmProvider {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "openai" | "gpt" => Ok(Self::OpenAi),
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "ollama" | "local" => Ok(Self::Ollama),
            _ => anyhow::bail!(
                "Unknown LLM provider: {}. Use: openai, anthropic, or ollama",
                s
            ),
        }
    }
}

impl std::fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAi => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Ollama => write!(f, "ollama"),
        }
    }
}

/// Trait for LLM interactions
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Complete a prompt and return the response
    async fn complete(&self, prompt: &str) -> Result<String>;

    /// Get the provider name
    fn provider_name(&self) -> &'static str;

    /// Get the model being used
    fn model_name(&self) -> &str;
}

// ============================================================================
// OpenAI Implementation
// ============================================================================

pub struct OpenAiClient {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
        }
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn complete(&self, prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Message<'a> {
            role: &'a str,
            content: &'a str,
        }

        #[derive(Serialize)]
        struct Request<'a> {
            model: &'a str,
            messages: Vec<Message<'a>>,
            temperature: f32,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: ResponseMessage,
        }

        #[derive(Deserialize)]
        struct ResponseMessage {
            content: String,
        }

        #[derive(Deserialize)]
        struct Response {
            choices: Vec<Choice>,
        }

        let request = Request {
            model: &self.model,
            messages: vec![
                Message {
                    role: "system",
                    content: "You are a PostgreSQL performance expert. Provide concise, actionable recommendations.",
                },
                Message {
                    role: "user",
                    content: prompt,
                },
            ],
            temperature: 0.0,
        };

        let response: Response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to OpenAI")?
            .error_for_status()
            .context("OpenAI API error")?
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .context("No response from OpenAI")
    }

    fn provider_name(&self) -> &'static str {
        "OpenAI"
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// ============================================================================
// Anthropic (Claude) Implementation
// ============================================================================

pub struct AnthropicClient {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
        }
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Message<'a> {
            role: &'a str,
            content: &'a str,
        }

        #[derive(Serialize)]
        struct Request<'a> {
            model: &'a str,
            max_tokens: u32,
            messages: Vec<Message<'a>>,
        }

        #[derive(Deserialize)]
        struct ContentBlock {
            text: String,
        }

        #[derive(Deserialize)]
        struct Response {
            content: Vec<ContentBlock>,
        }

        let content = format!(
            "You are a PostgreSQL performance expert. Provide concise, actionable recommendations.\n\n{}",
            prompt
        );
        let request = Request {
            model: &self.model,
            max_tokens: 1024,
            messages: vec![Message {
                role: "user",
                content: &content,
            }],
        };

        let response: Response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Anthropic")?
            .error_for_status()
            .context("Anthropic API error")?
            .json()
            .await
            .context("Failed to parse Anthropic response")?;

        response
            .content
            .into_iter()
            .next()
            .map(|c| c.text)
            .context("No response from Anthropic")
    }

    fn provider_name(&self) -> &'static str {
        "Anthropic"
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// ============================================================================
// Ollama (Local) Implementation
// ============================================================================

pub struct OllamaClient {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
    }
}

#[async_trait]
impl LlmClient for OllamaClient {
    async fn complete(&self, prompt: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Request<'a> {
            model: &'a str,
            prompt: &'a str,
            stream: bool,
        }

        #[derive(Deserialize)]
        struct Response {
            response: String,
        }

        let full_prompt = format!(
            "You are a PostgreSQL performance expert. Provide concise, actionable recommendations.\n\n{}",
            prompt
        );

        let request = Request {
            model: &self.model,
            prompt: &full_prompt,
            stream: false,
        };

        let response: Response = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Ollama")?
            .error_for_status()
            .context("Ollama API error")?
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        Ok(response.response)
    }

    fn provider_name(&self) -> &'static str {
        "Ollama"
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_from_str() {
        assert_eq!(
            LlmProvider::from_str("openai").unwrap(),
            LlmProvider::OpenAi
        );
        assert_eq!(LlmProvider::from_str("GPT").unwrap(), LlmProvider::OpenAi);
        assert_eq!(
            LlmProvider::from_str("anthropic").unwrap(),
            LlmProvider::Anthropic
        );
        assert_eq!(
            LlmProvider::from_str("claude").unwrap(),
            LlmProvider::Anthropic
        );
        assert_eq!(
            LlmProvider::from_str("ollama").unwrap(),
            LlmProvider::Ollama
        );
        assert_eq!(LlmProvider::from_str("local").unwrap(), LlmProvider::Ollama);
        assert!(LlmProvider::from_str("invalid").is_err());
    }

    #[test]
    fn test_provider_display() {
        assert_eq!(LlmProvider::OpenAi.to_string(), "openai");
        assert_eq!(LlmProvider::Anthropic.to_string(), "anthropic");
        assert_eq!(LlmProvider::Ollama.to_string(), "ollama");
    }
}
