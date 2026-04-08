//! web_fetch — HTTP GET a URL and return the response body.
//!
//! ## Parameters
//! ```json
//! { "url": "https://example.com", "headers": {}, "timeout_ms": 10000 }
//! ```
//!
//! ## Output
//! ```json
//! { "status": 200, "content_type": "text/html", "body": "...", "truncated": false }
//! ```
//!
//! Response body is capped at **32 KB** (`truncated = true` when hit).
//! Default timeout is **10 s** (override with `timeout_ms`).

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

const MAX_BODY_BYTES: usize = 32 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// HTTP GET tool.  Tier: Registered (no approval required).
pub struct WebFetchTool {
    client: reqwest::Client,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(DEFAULT_TIMEOUT_MS))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("web_fetch: failed to build reqwest client");
        Self { client }
    }
}

#[async_trait]
impl ToolHandler for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Fetch a URL via HTTP GET and return the response body (capped at 32 KB)."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url":        { "type": "string", "description": "URL to fetch." },
                "headers":    { "type": "object",
                                "description": "Optional request headers.",
                                "additionalProperties": { "type": "string" } },
                "timeout_ms": { "type": "integer", "default": 10000,
                                "description": "Request timeout in milliseconds." }
            }
        })
    }

    // Network egress — monitored but no approval required.
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        // ── Validate URL ──────────────────────────────────────────────────────
        let url = args
            .get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "url".into(),
                message: "required".into(),
            })?
            .to_owned();

        let parsed = url
            .parse::<reqwest::Url>()
            .map_err(|e| ToolError::InvalidArgs {
                field: "url".into(),
                message: format!("invalid URL: {e}"),
            })?;

        if !["http", "https"].contains(&parsed.scheme()) {
            return Err(ToolError::InvalidArgs {
                field: "url".into(),
                message: format!(
                    "unsupported scheme '{}' — only http/https allowed",
                    parsed.scheme()
                ),
            });
        }

        // ── Build request ─────────────────────────────────────────────────────
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        let mut req = self
            .client
            .get(url.as_str())
            .timeout(std::time::Duration::from_millis(timeout_ms));

        if let Some(headers) = args.get("headers").and_then(|h| h.as_object()) {
            for (key, val) in headers {
                if let Some(s) = val.as_str() {
                    req = req.header(key.as_str(), s);
                }
            }
        }

        // ── Send ──────────────────────────────────────────────────────────────
        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ToolError::TimedOut
            } else {
                ToolError::Transient(e.to_string())
            }
        })?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();

        let bytes = response
            .bytes()
            .await
            .map_err(|e| ToolError::Transient(format!("read body: {e}")))?;

        let truncated = bytes.len() > MAX_BODY_BYTES;
        let body = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BODY_BYTES)]).into_owned();

        let output = serde_json::json!({
            "status":       status,
            "content_type": content_type,
            "body":         body,
            "truncated":    truncated,
        });

        Ok(if truncated {
            ToolResult::truncated(output)
        } else {
            ToolResult::ok(output)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[test]
    fn name_tier_class() {
        let t = WebFetchTool::default();
        assert_eq!(t.name(), "web_fetch");
        assert_eq!(t.tier(), ToolTier::Registered);
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    #[test]
    fn schema_requires_url() {
        let req = WebFetchTool::default().parameters_schema()["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(req.iter().any(|v| v.as_str() == Some("url")));
    }

    #[tokio::test]
    async fn missing_url_is_invalid_args() {
        let err = WebFetchTool::default()
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn bad_scheme_is_invalid_args() {
        let err = WebFetchTool::default()
            .execute(&project(), serde_json::json!({"url": "ftp://example.com"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn invalid_url_is_invalid_args() {
        let err = WebFetchTool::default()
            .execute(&project(), serde_json::json!({"url": "not-a-url"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
}
