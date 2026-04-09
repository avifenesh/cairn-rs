//! Bedrock GenerationProvider — calls AWS Bedrock Converse API via aws CLI.
//!
//! Uses `aws bedrock-runtime converse` subprocess for auth (SigV4 handled by AWS CLI).

use async_trait::async_trait;
use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};
use serde_json::Value;

pub struct BedrockProvider {
    model_id: String,
    region: String,
}

impl BedrockProvider {
    pub fn new(model_id: impl Into<String>, region: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            region: region.into(),
        }
    }

    /// Create from environment variables.
    /// Requires AWS credentials configured for `aws` CLI.
    /// BEDROCK_MODEL_ID defaults to `minimax.minimax-m2.5`.
    /// AWS_REGION defaults to `us-west-2`.
    pub fn from_env() -> Option<Self> {
        let output = std::process::Command::new("aws")
            .args(["sts", "get-caller-identity", "--output", "json"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }

        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-west-2".to_owned());
        let model_id =
            std::env::var("BEDROCK_MODEL_ID").unwrap_or_else(|_| "minimax.minimax-m2.5".to_owned());

        Some(Self::new(model_id, region))
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

        let messages_json = serde_json::to_string(&bedrock_messages)
            .map_err(|e| ProviderAdapterError::TransportFailure(e.to_string()))?;

        let effective_model = if model_id.is_empty() {
            &self.model_id
        } else {
            model_id
        };

        let mut args = vec![
            "bedrock-runtime",
            "converse",
            "--model-id",
            effective_model,
            "--messages",
            &messages_json,
            "--region",
            &self.region,
            "--output",
            "json",
        ];

        let system_json;
        if let Some(sys) = &system_prompt {
            system_json =
                serde_json::to_string(&[serde_json::json!({"text": sys})]).unwrap_or_default();
            args.push("--system");
            args.push(&system_json);
        }

        let output = tokio::process::Command::new("aws")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(format!("aws CLI spawn: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("ThrottlingException") || stderr.contains("TooManyRequests") {
                return Err(ProviderAdapterError::RateLimited);
            }
            return Err(ProviderAdapterError::ProviderError(format!(
                "Bedrock converse failed: {stderr}"
            )));
        }

        let resp: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| ProviderAdapterError::TransportFailure(format!("parse response: {e}")))?;

        // Extract text from: { "output": { "message": { "content": [{"text": "..."}] } } }
        let text = resp["output"]["message"]["content"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
                    .into()
            })
            .unwrap_or_default();

        let input_tokens = resp["usage"]["inputTokens"].as_u64().map(|n| n as u32);
        let output_tokens = resp["usage"]["outputTokens"].as_u64().map(|n| n as u32);

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
        let p = BedrockProvider::new("minimax.minimax-m2.5", "us-west-2");
        assert_eq!(p.model_id(), "minimax.minimax-m2.5");
        assert_eq!(p.region(), "us-west-2");
    }
}
