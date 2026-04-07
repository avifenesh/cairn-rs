//! summarize_text — meta-tool calling the worker LLM to compress text.
use std::sync::Arc;
use async_trait::async_trait;
use cairn_domain::{ProjectKey, providers::{GenerationProvider, ProviderAdapterError, ProviderBindingSettings, GenerationResponse}};
use serde_json::Value;
use super::{ToolError, ToolHandler, ToolResult, ToolTier};

struct NoopProvider;
#[async_trait]
impl GenerationProvider for NoopProvider {
    async fn generate(&self, _: &str, _: Vec<Value>, _: &ProviderBindingSettings) -> Result<GenerationResponse, ProviderAdapterError> {
        Ok(GenerationResponse { text: "Summary not available (no LLM provider configured).".into(), input_tokens: None, output_tokens: None, model_id: "none".into(), tool_calls: vec![] })
    }
}

pub struct SummarizeTextTool { provider: Arc<dyn GenerationProvider>, model_id: String }

impl SummarizeTextTool {
    pub fn new(provider: Arc<dyn GenerationProvider>, model_id: impl Into<String>) -> Self {
        Self { provider, model_id: model_id.into() }
    }
    pub fn stub() -> Self { Self::new(Arc::new(NoopProvider), "none") }
}
impl Default for SummarizeTextTool { fn default() -> Self { Self::stub() } }

#[async_trait]
impl ToolHandler for SummarizeTextTool {
    fn name(&self) -> &str { "summarize_text" }
    fn tier(&self) -> ToolTier { ToolTier::Registered }
    fn description(&self) -> &str {
        "Compress text using the worker LLM. \
         Use when context is too long to pass directly to the brain."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type":"object","required":["text"],
            "properties":{
                "text":{"type":"string","description":"Text to summarize (max 50000 chars)"},
                "max_words":{"type":"integer","default":100,"minimum":10,"maximum":1000},
                "style":{"type":"string","enum":["bullet","paragraph"],"default":"paragraph"},
                "focus":{"type":"string","description":"Aspect to emphasize in the summary"}
            }
        })
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let text = args["text"].as_str()
            .ok_or_else(|| ToolError::InvalidArgs { field:"text".into(), message:"required string".into() })?;
        if text.trim().is_empty() {
            return Err(ToolError::InvalidArgs { field:"text".into(), message:"must not be empty".into() });
        }
        let trimmed = if text.len() > 50_000 { &text[..50_000] } else { text };
        let max_words = args["max_words"].as_u64().unwrap_or(100).clamp(10, 1000);
        let style = args["style"].as_str().unwrap_or("paragraph");
        let focus = args["focus"].as_str().map(|f| format!(" Focus on: {f}.")).unwrap_or_default();

        let user_msg = format!(
            "Summarize in approximately {max_words} words in {style} format.{focus}\n\nText:\n{trimmed}"
        );
        let messages = vec![
            serde_json::json!({"role":"system","content":"You are a precise text summarizer. Respond only with the summary, no preamble."}),
            serde_json::json!({"role":"user","content":user_msg}),
        ];
        let settings = ProviderBindingSettings { max_output_tokens: Some(512), ..Default::default() };
        let resp = self.provider.generate(&self.model_id, messages, &settings).await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        let word_count = resp.text.split_whitespace().count();
        Ok(ToolResult::ok(serde_json::json!({
            "summary": resp.text, "word_count": word_count, "style": style
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p() -> ProjectKey { ProjectKey::new("t","w","p") }

    #[test] fn tier_is_registered() { assert_eq!(SummarizeTextTool::stub().tier(), ToolTier::Registered); }
    #[test] fn schema_has_required_text() {
        let s = SummarizeTextTool::stub().parameters_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("text")));
    }
    #[tokio::test] async fn stub_returns_summary() {
        let r = SummarizeTextTool::stub().execute(&p(), serde_json::json!({"text":"hello world"})).await.unwrap();
        assert!(!r.output["summary"].as_str().unwrap().is_empty());
        assert!(r.output["word_count"].as_u64().unwrap() > 0);
    }
    #[tokio::test] async fn empty_text_err() {
        let err = SummarizeTextTool::stub().execute(&p(), serde_json::json!({"text":"  "})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test] async fn missing_text_err() {
        let err = SummarizeTextTool::stub().execute(&p(), serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test] async fn style_and_max_words_passed() {
        let r = SummarizeTextTool::stub().execute(&p(), serde_json::json!({"text":"test","style":"bullet","max_words":50})).await.unwrap();
        assert_eq!(r.output["style"],"bullet");
    }
}
