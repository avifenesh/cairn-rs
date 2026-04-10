//! scratch_pad — ephemeral key-value store with TTL for agent working memory.
//!
//! Faster than memory_store for temporary data that doesn't need to survive
//! beyond a run. Keys expire automatically after their TTL.
//!
//! ## Parameters
//! ```json
//! { "action": "write", "key": "plan_step_2", "value": {"goal":"..."}, "ttl_minutes": 30 }
//! { "action": "read",  "key": "plan_step_2" }
//! { "action": "list" }
//! { "action": "delete", "key": "plan_step_2" }
//! ```
//!
//! ## Output (action=write)
//! ```json
//! { "key": "plan_step_2", "written": true, "expires_at_ms": 1234567890 }
//! ```

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

const DEFAULT_TTL_MINUTES: u64 = 30;

#[derive(Clone)]
struct Entry {
    value: Value,
    expires_at_ms: u64,
}

/// Thread-safe ephemeral key-value scratch pad with TTL.
pub struct ScratchPadTool {
    store: Arc<Mutex<HashMap<String, Entry>>>,
}

impl Default for ScratchPadTool {
    fn default() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl ScratchPadTool {
    pub fn new() -> Self {
        Self::default()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl ToolHandler for ScratchPadTool {
    fn name(&self) -> &str {
        "scratch_pad"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Internal
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Ephemeral key-value scratch pad with TTL for agent working memory during a run."
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action":      { "type": "string", "enum": ["write","read","list","delete"],
                                 "description": "Operation to perform." },
                "key":         { "type": "string", "description": "Key (required for write/read/delete)." },
                "value":       { "description": "Value to store (required for write)." },
                "ttl_minutes": { "type": "integer", "default": 30,
                                 "description": "Time-to-live in minutes (write only)." }
            }
        })
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let action =
            args.get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "action".into(),
                    message: "required".into(),
                })?;
        let now = now_ms();

        match action {
            "write" => {
                let key = require_key(&args)?;
                let value = args
                    .get("value")
                    .cloned()
                    .ok_or_else(|| ToolError::InvalidArgs {
                        field: "value".into(),
                        message: "required for write".into(),
                    })?;
                let ttl_ms = args
                    .get("ttl_minutes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_TTL_MINUTES)
                    * 60_000;
                let expires_at_ms = now + ttl_ms;
                self.store.lock().unwrap().insert(
                    key.clone(),
                    Entry {
                        value,
                        expires_at_ms,
                    },
                );
                Ok(ToolResult::ok(serde_json::json!({
                    "key": key, "written": true, "expires_at_ms": expires_at_ms,
                })))
            }
            "read" => {
                let key = require_key(&args)?;
                let mut map = self.store.lock().unwrap();
                match map.get(&key) {
                    Some(e) if e.expires_at_ms > now => Ok(ToolResult::ok(serde_json::json!({
                        "key": key, "value": e.value.clone(), "found": true,
                        "expires_at_ms": e.expires_at_ms,
                    }))),
                    Some(_) => {
                        map.remove(&key);
                        Ok(ToolResult::ok(
                            serde_json::json!({ "key": key, "found": false, "reason": "expired" }),
                        ))
                    }
                    None => Ok(ToolResult::ok(
                        serde_json::json!({ "key": key, "found": false }),
                    )),
                }
            }
            "list" => {
                let map = self.store.lock().unwrap();
                let entries: Vec<Value> = map
                    .iter()
                    .filter(|(_, e)| e.expires_at_ms > now)
                    .map(|(k, e)| {
                        serde_json::json!({
                            "key": k, "expires_at_ms": e.expires_at_ms,
                        })
                    })
                    .collect();
                let total = entries.len();
                Ok(ToolResult::ok(
                    serde_json::json!({ "keys": entries, "total": total }),
                ))
            }
            "delete" => {
                let key = require_key(&args)?;
                let removed = self.store.lock().unwrap().remove(&key).is_some();
                Ok(ToolResult::ok(
                    serde_json::json!({ "key": key, "deleted": removed }),
                ))
            }
            other => Err(ToolError::InvalidArgs {
                field: "action".into(),
                message: format!("unknown action '{other}'; expected write/read/list/delete"),
            }),
        }
    }
}

fn require_key(args: &Value) -> Result<String, ToolError> {
    let k = args
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs {
            field: "key".into(),
            message: "required".into(),
        })?;
    if k.trim().is_empty() {
        return Err(ToolError::InvalidArgs {
            field: "key".into(),
            message: "must not be empty".into(),
        });
    }
    Ok(k.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[test]
    fn name_tier_class() {
        let t = ScratchPadTool::new();
        assert_eq!(t.name(), "scratch_pad");
        assert_eq!(t.tier(), ToolTier::Registered);
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    #[tokio::test]
    async fn write_then_read() {
        let t = ScratchPadTool::new();
        t.execute(
            &project(),
            serde_json::json!({"action":"write","key":"k","value":{"x":1}}),
        )
        .await
        .unwrap();
        let r = t
            .execute(&project(), serde_json::json!({"action":"read","key":"k"}))
            .await
            .unwrap();
        assert_eq!(r.output["found"], true);
        assert_eq!(r.output["value"]["x"], 1);
    }

    #[tokio::test]
    async fn read_missing_key_returns_not_found() {
        let t = ScratchPadTool::new();
        let r = t
            .execute(&project(), serde_json::json!({"action":"read","key":"no"}))
            .await
            .unwrap();
        assert_eq!(r.output["found"], false);
    }

    #[tokio::test]
    async fn list_shows_active_keys() {
        let t = ScratchPadTool::new();
        t.execute(
            &project(),
            serde_json::json!({"action":"write","key":"a","value":1}),
        )
        .await
        .unwrap();
        t.execute(
            &project(),
            serde_json::json!({"action":"write","key":"b","value":2}),
        )
        .await
        .unwrap();
        let r = t
            .execute(&project(), serde_json::json!({"action":"list"}))
            .await
            .unwrap();
        assert_eq!(r.output["total"], 2);
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let t = ScratchPadTool::new();
        t.execute(
            &project(),
            serde_json::json!({"action":"write","key":"del","value":"v"}),
        )
        .await
        .unwrap();
        let r = t
            .execute(
                &project(),
                serde_json::json!({"action":"delete","key":"del"}),
            )
            .await
            .unwrap();
        assert_eq!(r.output["deleted"], true);
        let r2 = t
            .execute(&project(), serde_json::json!({"action":"read","key":"del"}))
            .await
            .unwrap();
        assert_eq!(r2.output["found"], false);
    }

    #[tokio::test]
    async fn delete_missing_key_returns_false() {
        let t = ScratchPadTool::new();
        let r = t
            .execute(
                &project(),
                serde_json::json!({"action":"delete","key":"ghost"}),
            )
            .await
            .unwrap();
        assert_eq!(r.output["deleted"], false);
    }

    #[tokio::test]
    async fn unknown_action_is_invalid() {
        let err = ScratchPadTool::new()
            .execute(&project(), serde_json::json!({"action":"nuke"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn write_returns_expires_at() {
        let t = ScratchPadTool::new();
        let r = t
            .execute(
                &project(),
                serde_json::json!({"action":"write","key":"k","value":1,"ttl_minutes":60}),
            )
            .await
            .unwrap();
        let exp = r.output["expires_at_ms"].as_u64().unwrap();
        let now = now_ms();
        assert!(
            exp > now + 50 * 60_000,
            "expires_at must be ~60 min from now"
        );
    }
}
