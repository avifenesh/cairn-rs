//! Bedrock GenerationProvider — calls AWS Bedrock Converse API via HTTP.
//!
//! Auth: Bearer token (BEDROCK_API_KEY env var) or AWS CLI fallback.

use async_trait::async_trait;
use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};
use serde_json::Value;

pub struct BedrockProvider {
    model_id: String,
    region: String,
    api_key: String,
}

impl BedrockProvider {
    pub fn new(
        model_id: impl Into<String>,
        region: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            region: region.into(),
            api_key: api_key.into(),
        }
    }

    /// Create from environment variables.
    ///
    /// `BEDROCK_API_KEY` — bearer token for Bedrock HTTP API.
    /// `AWS_BEARER_TOKEN_BEDROCK` — alternative env var name (Claude Code compat).
    /// `BEDROCK_MODEL_ID` — defaults to `minimax.minimax-m2.5`.
    /// `AWS_REGION` — defaults to `us-west-2`.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("BEDROCK_API_KEY")
            .or_else(|_| std::env::var("AWS_BEARER_TOKEN_BEDROCK"))
            .ok()?;

        if api_key.is_empty() {
            return None;
        }

        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-west-2".to_owned());
        let model_id =
            std::env::var("BEDROCK_MODEL_ID").unwrap_or_else(|_| "minimax.minimax-m2.5".to_owned());

        Some(Self::new(model_id, region, api_key))
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn region(&self) -> &str {
        &self.region
    }
}

#[async_trait]
impl GenerationProvider for BedrockProvider {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<Value>,
        _settings: &ProviderBindingSettings,
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let effective_model = if model_id.is_empty() {
            &self.model_id
        } else {
            model_id
        };

        // Convert OpenAI-format messages to Bedrock Converse format
        let bedrock_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .map(|m| {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or("");
                serde_json::json!({
                    "role": role,
                    "content": [{"text": content}]
                })
            })
            .collect();

        let system_prompt: Option<String> = messages
            .iter()
            .find(|m| m["role"].as_str() == Some("system"))
            .and_then(|m| m["content"].as_str())
            .map(|s| s.to_owned());

        let url = format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/converse",
            self.region, effective_model
        );

        let mut body = serde_json::json!({ "messages": bedrock_messages });
        if let Some(sys) = &system_prompt {
            body["system"] = serde_json::json!([{"text": sys}]);
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(format!("Bedrock HTTP: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 429 {
                return Err(ProviderAdapterError::RateLimited);
            }
            return Err(ProviderAdapterError::ProviderError(format!(
                "Bedrock {status}: {body}"
            )));
        }

        let resp_body: Value = resp
            .json()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(format!("parse: {e}")))?;

        // Extract text from: { "output": { "message": { "content": [{"text": "..."}] } } }
        let text = resp_body["output"]["message"]["content"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        let input_tokens = resp_body["usage"]["inputTokens"].as_u64().map(|n| n as u32);
        let output_tokens = resp_body["usage"]["outputTokens"]
            .as_u64()
            .map(|n| n as u32);

        Ok(GenerationResponse {
            text,
            input_tokens,
            output_tokens,
            model_id: effective_model.to_owned(),
            tool_calls: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_id_and_region() {
        let p = BedrockProvider::new("minimax.minimax-m2.5", "us-west-2", "test-key");
        assert_eq!(p.model_id(), "minimax.minimax-m2.5");
        assert_eq!(p.region(), "us-west-2");
    }
}
