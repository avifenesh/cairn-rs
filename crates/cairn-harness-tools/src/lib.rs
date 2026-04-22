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
//! read-before-edit state. v1 uses a process-global `InMemoryLedger` —
//! adequate for single-run cairn-app invocations. A future PR can scope
//! ledgers to session boundaries via `ToolContext`.

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
