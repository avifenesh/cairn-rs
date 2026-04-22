//! Harness-tools adapter — bridges `@agent-sh/harness-*` Rust crates into
//! cairn's `cairn_tools::ToolHandler` surface.
//!
//! ## Layout
//!
//! ```text
//! HarnessTool        — shared associated-type trait (one impl per tool).
//! HarnessBuiltin<H>  — wrapper that implements cairn's `ToolHandler` for any
//!                      `HarnessTool`. Register as `Arc::new(HarnessBuiltin::<H>::new())`.
//! build_cairn_hook() — v1 permission hook: delegates to cairn's executor
//!                      pre-check (allow-all at the harness layer).
//! default_sensitive_patterns() — baseline deny list for permission policy.
//! From<harness_core::ToolError> for cairn_tools::ToolError — pass-through mapping.
//! ```
//!
//! ## Ledger lifetime
//!
//! `harness-write` requires a `Ledger` implementation that tracks
//! read-before-edit state. Ledgers are keyed by
//! `(tenant_id, workspace_id, project_id, session_id, run_id)` from the
//! `ToolContext` passed on every invocation, so a read in one run never
//! satisfies `NOT_READ_THIS_SESSION` for another run — cross-tenant and
//! cross-run coupling are both prevented. The ledger map grows unbounded
//! over a cairn-app process lifetime; eviction on run-finalize is a
//! follow-up (tracked alongside #228).

pub mod adapter;
pub mod error;
pub mod hook;
pub mod sensitive;
pub mod tools;

pub use adapter::{HarnessBuiltin, HarnessTool};
pub use hook::build_cairn_hook;
pub use sensitive::default_sensitive_patterns;
pub use tools::{
    HarnessBash, HarnessBashKill, HarnessBashOutput, HarnessEdit, HarnessGlob, HarnessGrep,
    HarnessMultiEdit, HarnessRead, HarnessWebFetch, HarnessWrite,
};
