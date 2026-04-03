//! Prompt asset, version, and release boundaries per RFC 006.
//!
//! Prompts are first-class product assets with:
//! - Library-owned assets and immutable versions (tenant/workspace scoped)
//! - Project-scoped releases with approval-gated lifecycle
//! - Auditable release actions for rollout/rollback

pub mod assets;
pub mod releases;
pub mod versions;

pub use assets::*;
pub use releases::*;
pub use versions::*;
