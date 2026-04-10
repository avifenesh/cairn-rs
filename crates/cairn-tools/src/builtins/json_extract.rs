//! json_extract — dot-notation path extraction from JSON data.
use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::recovery::RetrySafety;
use cairn_domain::ProjectKey;
use serde_json::Value;

pub struct JsonExtractTool;
impl Default for JsonExtractTool {
    fn default() -> Self {
        Self
    }
}

fn extract(data: &Value, path: &str) -> Option<Value> {
    if path.is_empty() {
        return Some(data.clone());
    }
    let mut cur = data;
    let segments: Vec<&str> = path.split('.').collect();
    for seg in &segments {
        cur = match cur {
            Value::Object(m) => m.get(*seg)?,
            Value::Array(a) => {
                let i: usize = seg.parse().ok()?;
                a.get(i)?
            }
            _ => return None,
        };
    }
    Some(cur.clone())
}

#[async_trait]
impl ToolHandler for JsonExtractTool {
    fn name(&self) -> &str {
        "json_extract"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Extract a value from JSON data using dot-notation path. \
         Supports nested objects and array index access."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type":"object","required":["data","path"],
            "properties":{
                "data":{"description":"JSON data to extract from (object, array, or JSON string)"},
                "path":{"type":"string","description":"Dot-notation path e.g. 'user.address.city' or 'items.0.name'"},
                "default":{"description":"Value to return if path not found (default: null)"}
            }
        })
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        if args.get("data").is_none() {
            return Err(ToolError::InvalidArgs {
                field: "data".into(),
                message: "required".into(),
            });
        }
        let path = args["path"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "path".into(),
                message: "required string".into(),
            })?;

        let data = match &args["data"] {
            Value::String(s) => serde_json::from_str(s).map_err(|e| ToolError::InvalidArgs {
                field: "data".into(),
                message: format!("invalid JSON: {e}"),
            })?,
            other => other.clone(),
        };

        let found_value = extract(&data, path);
        let found = found_value.is_some();
        let value =
            found_value.unwrap_or_else(|| args.get("default").cloned().unwrap_or(Value::Null));
        Ok(ToolResult::ok(
            serde_json::json!({ "value": value, "found": found, "path": path }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }
    fn tool() -> JsonExtractTool {
        JsonExtractTool
    }

    #[tokio::test]
    async fn extract_nested_key() {
        let r = tool()
            .execute(
                &p(),
                serde_json::json!({"data":{"user":{"name":"Alice"}},"path":"user.name"}),
            )
            .await
            .unwrap();
        assert_eq!(r.output["value"], "Alice");
        assert_eq!(r.output["found"], true);
    }
    #[tokio::test]
    async fn extract_array_index() {
        let r = tool()
            .execute(
                &p(),
                serde_json::json!({"data":{"items":["a","b","c"]},"path":"items.1"}),
            )
            .await
            .unwrap();
        assert_eq!(r.output["value"], "b");
    }
    #[tokio::test]
    async fn path_not_found_returns_default() {
        let r = tool()
            .execute(
                &p(),
                serde_json::json!({"data":{"x":1},"path":"missing","default":"fallback"}),
            )
            .await
            .unwrap();
        assert_eq!(r.output["value"], "fallback");
        assert_eq!(r.output["found"], false);
    }
    #[tokio::test]
    async fn path_not_found_null_default() {
        let r = tool()
            .execute(&p(), serde_json::json!({"data":{},"path":"no.such.path"}))
            .await
            .unwrap();
        assert!(r.output["value"].is_null());
        assert_eq!(r.output["found"], false);
    }
    #[tokio::test]
    async fn json_string_input_parsed() {
        let r = tool()
            .execute(&p(), serde_json::json!({"data":"{\"k\":42}","path":"k"}))
            .await
            .unwrap();
        assert_eq!(r.output["value"], 42);
    }
    #[tokio::test]
    async fn invalid_json_string_err() {
        let err = tool()
            .execute(&p(), serde_json::json!({"data":"not-json","path":"x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn missing_data_err() {
        let err = tool()
            .execute(&p(), serde_json::json!({"path":"x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn empty_path_returns_whole() {
        let r = tool()
            .execute(&p(), serde_json::json!({"data":{"a":1},"path":""}))
            .await
            .unwrap();
        assert_eq!(r.output["value"]["a"], 1);
    }
}
