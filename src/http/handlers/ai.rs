// AI-powered endpoints for natural language database assistance

use crate::ai::{AiAdvisor, AiConfig};
use crate::http::{error::ApiError, models::ApiResponse, AppState};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

/// Request to ask AI a question
#[derive(Debug, Deserialize)]
pub struct AskRequest {
    /// The question to ask
    pub question: String,
    /// Context about the database fleet
    pub context: String,
}

/// Response from AI
#[derive(Debug, Serialize)]
pub struct AskResponse {
    /// The AI's answer
    pub answer: String,
    /// LLM provider used
    pub provider: String,
    /// Model used
    pub model: String,
}

/// POST /api/v1/ai/ask - Ask AI a natural language question
pub async fn ask_ai(
    State(_state): State<AppState>,
    Json(req): Json<AskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ai_config = AiConfig::from_env();

    if !ai_config.enabled {
        return Err(ApiError::BadRequest(
            "AI features not enabled. Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or configure Ollama"
                .to_string(),
        ));
    }

    let advisor = AiAdvisor::new(ai_config.clone())
        .map_err(|e| ApiError::Internal(format!("Failed to initialize AI advisor: {}", e)))?;

    let answer = advisor
        .ask(&req.question, &req.context)
        .await
        .map_err(|e| ApiError::Internal(format!("AI query failed: {}", e)))?;

    let response = AskResponse {
        answer,
        provider: ai_config.provider.to_string(),
        model: ai_config.model.unwrap_or_else(|| "default".to_string()),
    };

    Ok((StatusCode::OK, Json(ApiResponse::ok(response))))
}

/// Request to optimize a query with AI
#[derive(Debug, Deserialize)]
pub struct OptimizeQueryRequest {
    /// The query to optimize
    pub query: String,
    /// Optional: Database context (tables, indexes)
    pub context: Option<String>,
}

/// Response with optimized query
#[derive(Debug, Serialize)]
pub struct OptimizeQueryResponse {
    /// Original query
    pub original: String,
    /// Optimized query
    pub optimized: String,
    /// Explanation of changes
    pub explanation: String,
    /// Estimated improvement percentage
    pub estimated_improvement_pct: f64,
    /// Provider used
    pub provider: String,
}

/// POST /api/v1/ai/optimize - Optimize a query using AI
pub async fn optimize_query(
    State(_state): State<AppState>,
    Json(req): Json<OptimizeQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ai_config = AiConfig::from_env();

    if !ai_config.enabled {
        return Err(ApiError::BadRequest(
            "AI features not enabled. Set OPENAI_API_KEY or configure AI provider".to_string(),
        ));
    }

    let advisor = AiAdvisor::new(ai_config.clone())
        .map_err(|e| ApiError::Internal(format!("Failed to initialize AI advisor: {}", e)))?;

    // Build prompt for optimization
    let context = req.context.unwrap_or_default();
    let prompt = format!(
        "Optimize this PostgreSQL query for better performance:\n\n\
        Query: {}\n\n\
        {}\n\n\
        Provide:\n\
        1. Optimized query\n\
        2. Explanation of changes\n\
        3. Estimated improvement percentage\n\n\
        Respond in JSON format:\n\
        {{\n\
          \"optimized_query\": \"...\",\n\
          \"explanation\": \"...\",\n\
          \"estimated_improvement\": 50\n\
        }}",
        req.query, context
    );

    let ai_response = advisor
        .ask(&prompt, &context)
        .await
        .map_err(|e| ApiError::Internal(format!("AI optimization failed: {}", e)))?;

    // Parse JSON response
    let parsed: serde_json::Value = serde_json::from_str(&ai_response)
        .or_else(|_| {
            // Try to extract JSON from markdown
            if let Some(start) = ai_response.find("```json") {
                if let Some(end) = ai_response[start + 7..].find("```") {
                    return serde_json::from_str(&ai_response[start + 7..start + 7 + end]);
                }
            }
            // If no JSON found, create a basic response
            Ok(serde_json::json!({
                "optimized_query": req.query.clone(),
                "explanation": ai_response.clone(),
                "estimated_improvement": 0
            }))
        })
        .map_err(|e| {
            ApiError::Internal(format!(
                "Failed to parse AI response: {}. Response: {}",
                e, ai_response
            ))
        })?;

    let response = OptimizeQueryResponse {
        original: req.query,
        optimized: parsed["optimized_query"].as_str().unwrap_or("").to_string(),
        explanation: parsed["explanation"]
            .as_str()
            .unwrap_or("No explanation provided")
            .to_string(),
        estimated_improvement_pct: parsed["estimated_improvement"].as_f64().unwrap_or(0.0),
        provider: ai_config.provider.to_string(),
    };

    Ok((StatusCode::OK, Json(ApiResponse::ok(response))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ask_request_deserialization() {
        let json = r#"{"question":"Why is my database slow?","context":"Fleet: 5 databases"}"#;
        let req: AskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.question, "Why is my database slow?");
        assert_eq!(req.context, "Fleet: 5 databases");
    }

    #[test]
    fn test_optimize_request_deserialization() {
        let json = r#"{"query":"SELECT * FROM users"}"#;
        let req: OptimizeQueryRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "SELECT * FROM users");
        assert!(req.context.is_none());
    }
}
