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

/// Build an allow-all hook. The closure is cheap to clone into any number
/// of session configs.
pub fn build_cairn_hook() -> PermissionHook {
    Arc::new(|_query: PermissionQuery| Box::pin(async move { PermissionDecision::Allow }))
}
