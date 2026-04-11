//! Provider error types.

pub(crate) fn truncate_raw_response(raw: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut truncated = String::new();
    for (idx, ch) in raw.chars().enumerate() {
        if idx >= MAX_CHARS {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

/// Errors that can occur when interacting with LLM providers.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP transport: {0}")]
    Http(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("provider returned error: {0}")]
    Provider(String),

    #[error("rate limited")]
    RateLimited,

    #[error("response parse error: {message} (raw: {raw_response})")]
    ResponseFormat {
        message: String,
        raw_response: String,
    },

    #[error("JSON error: {0}")]
    Json(String),

    #[error("tool config error: {0}")]
    ToolConfig(String),

    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl From<reqwest::Error> for ProviderError {
    fn from(err: reqwest::Error) -> Self {
        Self::Http(err.to_string())
    }
}

impl From<serde_json::Error> for ProviderError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(format!("{err} at line {} col {}", err.line(), err.column()))
    }
}

/// Convert from the existing cairn-domain ProviderAdapterError.
impl From<ProviderError> for String {
    fn from(err: ProviderError) -> Self {
        err.to_string()
    }
}
