//! `http_request` built-in tool — full HTTP client (GET/POST/PUT/DELETE/PATCH).

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;

use super::{PermissionLevel, ToolCategory, ToolError, ToolHandler, ToolResult, ToolTier};

/// Send an HTTP request to any URL.
///
/// # Schema
///
/// ```json
/// { "method": "POST", "url": "https://…", "headers": {}, "body": "…", "timeout_secs": 30 }
/// ```
pub struct HttpRequestTool;

impl Default for HttpRequestTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl ToolHandler for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Send an HTTP request (GET, POST, PUT, DELETE, PATCH) to any URL. \
         Returns status code, response headers, and response body."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["method", "url"],
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"],
                    "description": "HTTP method"
                },
                "url": {
                    "type": "string",
                    "description": "Full URL to request"
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers as key-value pairs",
                    "additionalProperties": { "type": "string" }
                },
                "body": {
                    "description": "Request body — string or JSON object",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "object" }
                    ]
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default 30, max 120)",
                    "default": 30,
                    "minimum": 1,
                    "maximum": 120
                }
            }
        })
    }

    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SandboxedProcess
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "method".into(),
                message: "required string".into(),
            })?
            .to_uppercase();

        let url =
            args.get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "url".into(),
                    message: "required string".into(),
                })?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ToolError::InvalidArgs {
                field: "url".into(),
                message: "URL must start with http:// or https://".into(),
            });
        }

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(120))
            .unwrap_or(30);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        let mut req = match method.as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "DELETE" => client.delete(url),
            "PATCH" => client.patch(url),
            "HEAD" => client.head(url),
            other => {
                return Err(ToolError::InvalidArgs {
                    field: "method".into(),
                    message: format!("unsupported method: {other}"),
                })
            }
        };

        // Apply custom headers.
        if let Some(headers) = args.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in headers {
                if let Some(val) = v.as_str() {
                    req = req.header(k.as_str(), val);
                }
            }
        }

        // Apply body.
        if let Some(body) = args.get("body") {
            match body {
                Value::String(s) => req = req.body(s.clone()),
                Value::Object(_) | Value::Array(_) => {
                    req = req.header("Content-Type", "application/json").body(
                        serde_json::to_string(body).map_err(|e| ToolError::InvalidArgs {
                            field: "body".into(),
                            message: e.to_string(),
                        })?,
                    );
                }
                _ => {}
            }
        }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let headers: serde_json::Map<String, Value> = resp
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| {
                        v.to_str()
                            .ok()
                            .map(|s| (k.as_str().to_owned(), Value::String(s.to_owned())))
                    })
                    .collect();
                let body_text = resp
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("<read error: {e}>"));
                let body_value: Value =
                    serde_json::from_str(&body_text).unwrap_or(Value::String(body_text.clone()));

                Ok(ToolResult::ok(serde_json::json!({
                    "status":  status,
                    "ok":      (200..300).contains(&status),
                    "headers": headers,
                    "body":    body_value,
                })))
            }
            Err(e) if e.is_timeout() => Err(ToolError::TimedOut),
            Err(e) => Err(ToolError::Transient(format!("request failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[tokio::test]
    async fn missing_method_is_invalid() {
        let err = HttpRequestTool
            .execute(
                &project(),
                serde_json::json!({
                    "url": "https://example.com"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn missing_url_is_invalid() {
        let err = HttpRequestTool
            .execute(
                &project(),
                serde_json::json!({
                    "method": "GET"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn non_http_url_is_invalid() {
        let err = HttpRequestTool
            .execute(
                &project(),
                serde_json::json!({
                    "method": "GET",
                    "url": "ftp://example.com/file"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn unsupported_method_is_invalid() {
        let err = HttpRequestTool
            .execute(
                &project(),
                serde_json::json!({
                    "method": "CONNECT",
                    "url": "https://example.com"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[test]
    fn tier_is_registered() {
        assert_eq!(HttpRequestTool.tier(), ToolTier::Registered);
    }

    #[test]
    fn schema_requires_method_and_url() {
        let schema = HttpRequestTool.parameters_schema();
        let required: Vec<String> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        assert!(required.contains(&"method".to_owned()));
        assert!(required.contains(&"url".to_owned()));
    }
}
