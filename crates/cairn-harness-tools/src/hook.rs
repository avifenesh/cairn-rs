//! v1 permission hook — delegates to cairn's executor pre-check.
//!
//! Cairn's executor already gates tool calls by role / tenant / approval
//! before dispatching. If a harness tool is invoked, cairn has already
//! decided the call is allowed — so the hook returns `Allow`.
//!
//! Future work (#228): per-domain allowlists for webfetch, per-skill
//! trust levels, operator-facing `Ask` routing.

use std::sync::Arc;

use harness_core::permissions::PermissionQuery;
use harness_core::{PermissionDecision, PermissionHook};
use once_cell::sync::Lazy;

/// Singleton allow-all hook. The outer `Arc` is cheap to clone into any
/// number of session configs; the inner closure is shared across every
/// tool invocation in the process — no per-call allocation.
static ALLOW_ALL_HOOK: Lazy<PermissionHook> = Lazy::new(|| {
    Arc::new(|_query: PermissionQuery| Box::pin(async move { PermissionDecision::Allow }))
});

/// Clone the shared allow-all hook for a new session config.
pub fn build_cairn_hook() -> PermissionHook {
    ALLOW_ALL_HOOK.clone()
}
