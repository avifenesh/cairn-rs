//! Provider error types.
//!
//! All user-facing error strings constructed in this crate **must** flow
//! through [`redact_secrets`] before they cross a trust boundary (HTTP
//! response, tracing event, metrics label). Provider-layer errors are
//! routinely surfaced verbatim in the UI, so a key that slips into an
//! error payload becomes a key on an operator's screen.

use crate::redact::redact_secrets;

/// Hard ceiling on embedded raw-response payloads inside `ProviderError`
/// variants. Callers should prefer [`safe_raw_response`] which also scrubs
/// secrets.
pub(crate) const MAX_RAW_RESPONSE_CHARS: usize = 500;

/// Truncate a raw provider response body to [`MAX_RAW_RESPONSE_CHARS`]
/// characters.
///
/// **This does not redact secrets.** All in-crate callers that may see a
/// body sourced from a third party must use [`safe_raw_response`] instead.
/// The plain-truncate variant is retained only for use after the caller has
/// already performed redaction.
pub(crate) fn truncate_raw_response(raw: &str) -> String {
    let mut truncated = String::new();
    for (idx, ch) in raw.chars().enumerate() {
        if idx >= MAX_RAW_RESPONSE_CHARS {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

/// Redact secrets from `raw` and then truncate to the standard ceiling.
///
/// Preferred helper for building `ProviderError::ResponseFormat` /
/// `ProviderError::Provider` payloads, since upstream providers sometimes
/// echo the request's Authorization header back into error bodies.
///
/// Thin alias over [`crate::redact::redact_and_truncate`] — kept as a
/// crate-private convenience so internal callsites can import from the
/// same module as the error types they're building, but the single
/// source of truth for the pipeline lives in `redact.rs`.
pub(crate) fn safe_raw_response(raw: &str) -> String {
    crate::redact::redact_and_truncate(raw)
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

    #[error("upstream {status}: {message}")]
    ServerError { status: u16, message: String },

    #[error(
        "empty completion from {model_id} (prompt_tokens={prompt_tokens:?}, completion_tokens={completion_tokens:?})"
    )]
    EmptyResponse {
        model_id: String,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    },

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
        // reqwest::Error::to_string() embeds the request URL on connect
        // errors, which may carry an `?api_key=...` query param. Redact.
        Self::Http(redact_secrets(&err.to_string()))
    }
}

impl From<serde_json::Error> for ProviderError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(redact_secrets(&format!(
            "{err} at line {} col {}",
            err.line(),
            err.column()
        )))
    }
}

/// Convert from the existing cairn-domain ProviderAdapterError.
impl From<ProviderError> for String {
    fn from(err: ProviderError) -> Self {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_raw_response_redacts_and_truncates() {
        let body = "upstream error: Authorization: Bearer sk-ant-abcdef1234567890abcdefxyz denied";
        let out = safe_raw_response(body);
        assert!(!out.contains("sk-ant-abcdef"), "leaked: {out}");
    }
}
