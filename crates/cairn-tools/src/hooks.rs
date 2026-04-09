//! Hook/middleware system for the Cairn agent lifecycle.
//!
//! Hooks intercept events (pre/post tool use, model turns, stop, error) and can
//! block, modify input, or continue normally.
//!
//! Adopted from Cersei (MIT, pacifio/cersei).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Hook trait ──────────────────────────────────────────────────────────────

/// Implement this trait to intercept agent lifecycle events.
#[async_trait]
pub trait Hook: Send + Sync {
    /// Which events this hook handles.
    fn events(&self) -> &[HookEvent];

    /// Called when a matching event fires. Returns an action to control flow.
    async fn on_event(&self, ctx: &HookContext) -> HookAction;

    /// Optional name for logging/debugging.
    fn name(&self) -> &str {
        "unnamed-hook"
    }
}

// ── Hook events ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PreModelTurn,
    PostModelTurn,
    Stop,
    Error,
}

// ── Hook context ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HookContext {
    pub event: HookEvent,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub tool_result: Option<String>,
    pub tool_is_error: Option<bool>,
    pub turn: u32,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

// ── Hook actions ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum HookAction {
    /// Continue normally.
    Continue,
    /// Block the operation (PreToolUse only). Includes reason.
    Block(String),
    /// Replace the tool input with modified data (PreToolUse only).
    ModifyInput(Value),
}

// ── Shell hook ──────────────────────────────────────────────────────────────

/// A hook that runs a shell command. The command receives hook context via
/// the `CAIRN_HOOK_CONTEXT` environment variable (JSON).
pub struct ShellHook {
    pub command: String,
    pub hook_events: Vec<HookEvent>,
    pub blocking: bool,
    hook_name: String,
}

impl ShellHook {
    pub fn new(command: impl Into<String>, events: &[HookEvent], blocking: bool) -> Self {
        let cmd = command.into();
        let name = format!("shell:{}", cmd.chars().take(40).collect::<String>());
        Self {
            command: cmd,
            hook_events: events.to_vec(),
            blocking,
            hook_name: name,
        }
    }
}

#[async_trait]
impl Hook for ShellHook {
    fn events(&self) -> &[HookEvent] {
        &self.hook_events
    }

    fn name(&self) -> &str {
        &self.hook_name
    }

    async fn on_event(&self, ctx: &HookContext) -> HookAction {
        let sh = if cfg!(windows) { "cmd" } else { "sh" };
        let flag = if cfg!(windows) { "/C" } else { "-c" };

        let ctx_json = serde_json::to_string(&serde_json::json!({
            "event": format!("{:?}", ctx.event),
            "tool_name": ctx.tool_name,
            "turn": ctx.turn,
            "session_id": ctx.session_id,
            "run_id": ctx.run_id,
        }))
        .unwrap_or_default();

        let output = match std::process::Command::new(sh)
            .args([flag, &self.command])
            .env("CAIRN_HOOK_CONTEXT", &ctx_json)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
        {
            Ok(o) => o,
            Err(_) => return HookAction::Continue,
        };

        if output.status.success() {
            return HookAction::Continue;
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let body = if !stderr.trim().is_empty() {
            stderr.to_string()
        } else {
            stdout.to_string()
        };

        if self.blocking {
            HookAction::Block(format!("Hook '{}' failed: {}", self.command, body.trim()))
        } else {
            HookAction::Continue
        }
    }
}

// ── Hook runner ─────────────────────────────────────────────────────────────

/// Execute all matching hooks for a given event, returning the first non-Continue action.
pub async fn run_hooks(hooks: &[std::sync::Arc<dyn Hook>], ctx: &HookContext) -> HookAction {
    for hook in hooks {
        if hook.events().contains(&ctx.event) {
            let action = hook.on_event(ctx).await;
            match &action {
                HookAction::Continue => continue,
                _ => return action,
            }
        }
    }
    HookAction::Continue
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct BlockingHook;

    #[async_trait]
    impl Hook for BlockingHook {
        fn events(&self) -> &[HookEvent] {
            &[HookEvent::PreToolUse]
        }
        fn name(&self) -> &str {
            "blocker"
        }
        async fn on_event(&self, _ctx: &HookContext) -> HookAction {
            HookAction::Block("blocked by test".into())
        }
    }

    struct PassthroughHook;

    #[async_trait]
    impl Hook for PassthroughHook {
        fn events(&self) -> &[HookEvent] {
            &[HookEvent::PreToolUse, HookEvent::PostToolUse]
        }
        fn name(&self) -> &str {
            "passthrough"
        }
        async fn on_event(&self, _ctx: &HookContext) -> HookAction {
            HookAction::Continue
        }
    }

    fn test_ctx(event: HookEvent) -> HookContext {
        HookContext {
            event,
            tool_name: Some("test_tool".into()),
            tool_input: None,
            tool_result: None,
            tool_is_error: None,
            turn: 1,
            session_id: None,
            run_id: None,
        }
    }

    #[tokio::test]
    async fn no_hooks_returns_continue() {
        let hooks: Vec<Arc<dyn Hook>> = vec![];
        let action = run_hooks(&hooks, &test_ctx(HookEvent::PreToolUse)).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn blocking_hook_stops_execution() {
        let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(BlockingHook)];
        let action = run_hooks(&hooks, &test_ctx(HookEvent::PreToolUse)).await;
        assert!(matches!(action, HookAction::Block(_)));
    }

    #[tokio::test]
    async fn passthrough_then_blocker() {
        let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(PassthroughHook), Arc::new(BlockingHook)];
        let action = run_hooks(&hooks, &test_ctx(HookEvent::PreToolUse)).await;
        assert!(matches!(action, HookAction::Block(_)));
    }

    #[tokio::test]
    async fn hook_only_fires_for_matching_events() {
        let hooks: Vec<Arc<dyn Hook>> = vec![Arc::new(BlockingHook)];
        // BlockingHook only handles PreToolUse, not PostToolUse
        let action = run_hooks(&hooks, &test_ctx(HookEvent::PostToolUse)).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn shell_hook_continues_on_success() {
        let hook = ShellHook::new("true", &[HookEvent::PreToolUse], true);
        let action = hook.on_event(&test_ctx(HookEvent::PreToolUse)).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn shell_hook_blocks_on_failure() {
        let hook = ShellHook::new("false", &[HookEvent::PreToolUse], true);
        let action = hook.on_event(&test_ctx(HookEvent::PreToolUse)).await;
        assert!(matches!(action, HookAction::Block(_)));
    }

    #[tokio::test]
    async fn non_blocking_shell_hook_continues_on_failure() {
        let hook = ShellHook::new("false", &[HookEvent::PreToolUse], false);
        let action = hook.on_event(&test_ctx(HookEvent::PreToolUse)).await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[test]
    fn hook_event_serde_roundtrip() {
        let events = vec![
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::PreModelTurn,
            HookEvent::PostModelTurn,
            HookEvent::Stop,
            HookEvent::Error,
        ];
        let json = serde_json::to_string(&events).unwrap();
        let parsed: Vec<HookEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(events, parsed);
    }
}
