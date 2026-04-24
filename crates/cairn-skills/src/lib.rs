//! `cairn-skills` — thin adapter over published `harness-skill`.
//!
//! ## What this crate does
//!
//! Exposes the `skill` tool (agentskills.io activation primitive) as a
//! cairn builtin. The tool is implemented by wrapping
//! `harness-skill::skill()` — we do NOT re-implement SKILL.md parsing,
//! frontmatter validation, progressive disclosure, trust gating, or
//! fence logic. All of that lives in `harness-skill` 0.1.0 on crates.io.
//!
//! ## Layout
//!
//! ```text
//! HarnessSkill         — empty type implementing `cairn_harness_tools::HarnessTool`.
//!                        Registered in cairn-app as `HarnessBuiltin::<HarnessSkill>::new()`.
//! activated_set_for()  — per-session `ActivatedSet` cache, keyed by
//!                        `(tenant, workspace, project, session_id)` so dedupe
//!                        state is isolated between sessions + tenants.
//! skill_roots_for()    — resolves skill discovery roots from the tool
//!                        context's working directory:
//!                          `<cwd>/.cairn/skills` (project-level)
//!                          `<cwd>/skills`        (workspace-level fallback)
//! ```
//!
//! ## Scope (BP-8 v1)
//!
//! - Filesystem-backed discovery (`FilesystemSkillRegistry`) only.
//! - Default trust policy (`SkillTrustPolicy::default()`): untrusted project
//!   skills fall through the hook, which in v1 is allow-all (matches
//!   `cairn-harness-tools::hook::build_cairn_hook`). Approval-gate wiring is
//!   a follow-up once the catalog UI ships (see research doc §6.4).
//! - No catalog injection into the LLM system prompt yet — the model sees
//!   the `skill` tool in its tool list and discovers names by invoking it
//!   with an unknown name (the `not_found` result returns fuzzy siblings).
//!   Catalog-block injection is planned for BP-9.
//! - No `SkillTrustGranted` / `SkillActivated` domain events yet — activation
//!   flows through the standard `ToolInvocationNodeData` graph emission that
//!   every builtin produces. Dedicated events are a follow-up.

pub mod skill_tool;

pub use skill_tool::{evict_session, HarnessSkill};

/// Internal helpers exposed only under the `test-utils` cargo feature.
///
/// `__clear_activated_sets_for_tests` lets integration tests reset the
/// process-wide activation cache between cases; `skill_roots_for` exposes
/// the root-resolution helper for parity testing. Neither is part of the
/// stable public API and production consumers must not depend on them.
#[cfg(feature = "test-utils")]
#[doc(hidden)]
pub use skill_tool::{__clear_activated_sets_for_tests, skill_roots_for};
