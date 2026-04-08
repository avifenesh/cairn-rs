//! SOUL.md Guardian — validates and gates proposed patches (GAP-007).
//!
//! Mirrors `cairn/internal/agent/soul_guard.go` and
//! `cairn/internal/memory/soul.go`.
//!
//! Three-tier decision for `validate_patch`:
//! 1. **Denied** — patch touches a `locked_field`. No amount of approval can
//!    override a locked field.
//! 2. **Requires approval** — patch adds/changes text under a personality or
//!    identity section (Voice, Values, Identity, Who I Am, Tone, Learning).
//! 3. **Allowed** — patch targets operational sections (Commands, Auto-execute,
//!    Tool Configs, Proactive Behaviors, etc.) or no recognisable section.
//!
//! Section detection is intentionally lightweight: the guard looks for
//! Markdown headings (`## Section Name`) in the patch text (case-insensitive).
//! A patch with no headings is treated as an operational addition.

use cairn_domain::soul::{SoulDocument, SoulPatchResult, OPERATIONAL_FIELDS, PERSONALITY_FIELDS};

/// Stateless soul patch validator.
///
/// Call `SoulGuard::default()` or `SoulGuard::new()`.
#[derive(Clone, Debug, Default)]
pub struct SoulGuard;

impl SoulGuard {
    pub fn new() -> Self {
        Self
    }

    /// Validate a proposed patch against the current soul document.
    ///
    /// `patch` is the text being proposed (append text, full replacement, or diff).
    /// The guard inspects Markdown headings in the patch to classify fields.
    pub fn validate_patch(&self, doc: &SoulDocument, patch: &str) -> SoulPatchResult {
        let sections = extract_sections(patch);

        // Rule 1: locked_fields → deny unconditionally.
        for locked in &doc.locked_fields {
            let locked_lower = locked.to_lowercase();
            if sections.iter().any(|s| s.contains(locked_lower.as_str())) {
                return SoulPatchResult::denied(format!(
                    "field '{}' is locked and cannot be modified",
                    locked
                ));
            }
        }

        // Rule 2: personality/identity sections → requires_approval.
        let personality_hit = sections
            .iter()
            .find(|s| PERSONALITY_FIELDS.iter().any(|pf| s.contains(*pf)));
        if let Some(hit) = personality_hit {
            return SoulPatchResult::requires_approval(format!(
                "patch modifies personality/identity field '{}'; operator approval required",
                hit
            ));
        }

        // Rule 3: operational or unrecognised sections → allowed.
        let op_reason = sections
            .iter()
            .find(|s| OPERATIONAL_FIELDS.iter().any(|of| s.contains(*of)))
            .map(|s| format!("patch targets operational field '{s}'"))
            .unwrap_or_else(|| {
                "patch contains no recognised section markers; treated as operational".to_owned()
            });

        SoulPatchResult::allowed(op_reason)
    }
}

/// Extract lowercased section names from Markdown `##` headings in `text`.
/// Returns an empty vec if no headings are found (patch has no section markers).
pub fn extract_sections(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Match `##` or `###` headings (skip `#` = document title).
            if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
                let heading = trimmed.trim_start_matches('#').trim();
                Some(heading.to_lowercase())
            } else {
                None
            }
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::soul::SoulDocument;

    fn doc() -> SoulDocument {
        SoulDocument::new(include_str_or_default())
    }

    fn doc_with_locked(locked: &[&str]) -> SoulDocument {
        SoulDocument::new("").with_locked_fields(locked.iter().copied())
    }

    fn include_str_or_default() -> &'static str {
        "# Cairn\n\n## Who I Am\nI am Cairn.\n\n## Auto-execute\nAll local commands.\n"
    }

    // ── Personality field patches ─────────────────────────────────────────

    /// Patch that targets `## Voice` requires approval.
    #[test]
    fn personality_field_voice_requires_approval() {
        let guard = SoulGuard::new();
        let patch = "## Voice\n\nAlways be empathetic and acknowledge feelings before solving.";
        let result = guard.validate_patch(&doc(), patch);
        assert!(
            result.allowed,
            "personality patch must be allowed (pending approval)"
        );
        assert!(
            result.requires_approval,
            "voice section must require approval"
        );
        assert!(
            result.reason.contains("approval"),
            "reason must mention approval"
        );
    }

    /// Patch that modifies `## Values` requires approval.
    #[test]
    fn personality_field_values_requires_approval() {
        let guard = SoulGuard::new();
        let patch = "## Values (immutable)\n\n- Be honest always.\n- Protect user data.";
        let result = guard.validate_patch(&doc(), patch);
        assert!(
            result.requires_approval,
            "values section must require approval"
        );
    }

    /// Patch changing `## Who I Am` requires approval.
    #[test]
    fn personality_field_identity_requires_approval() {
        let guard = SoulGuard::new();
        let patch = "## Who I Am\n\nI am Cairn, a revised self.";
        let result = guard.validate_patch(&doc(), patch);
        assert!(result.requires_approval);
    }

    // ── Operational field patches ─────────────────────────────────────────

    /// Patch that adds a command to `## Auto-execute` is allowed without approval.
    #[test]
    fn operational_field_auto_execute_allowed() {
        let guard = SoulGuard::new();
        let patch = "## Auto-execute\n\n- Run integration tests before merging.";
        let result = guard.validate_patch(&doc(), patch);
        assert!(result.allowed, "operational patch must be allowed");
        assert!(
            !result.requires_approval,
            "operational patch must not require approval"
        );
    }

    /// Patch updating `## Commands` is allowed.
    #[test]
    fn operational_field_commands_allowed() {
        let guard = SoulGuard::new();
        let patch = "## Commands\n\n```\ncargo test --workspace\n```";
        let result = guard.validate_patch(&doc(), patch);
        assert!(result.allowed);
        assert!(!result.requires_approval);
    }

    /// Patch with no section markers is treated as operational (allowed).
    #[test]
    fn no_section_markers_treated_as_operational() {
        let guard = SoulGuard::new();
        let patch = "Always prefer explicit error messages over silent failures.";
        let result = guard.validate_patch(&doc(), patch);
        assert!(result.allowed);
        assert!(!result.requires_approval);
    }

    // ── Locked field patches ──────────────────────────────────────────────

    /// Patch targeting a locked field is denied regardless of section type.
    #[test]
    fn locked_field_is_denied() {
        let guard = SoulGuard::new();
        let d = doc_with_locked(&["Values"]);
        let patch = "## Values\n\n- New value added by LLM drift.";
        let result = guard.validate_patch(&d, patch);
        assert!(!result.allowed, "locked field must be denied");
        assert!(
            !result.requires_approval,
            "denied patches must not set requires_approval"
        );
        assert!(
            result.reason.contains("locked"),
            "reason must mention 'locked'"
        );
    }

    /// Locking an operational field also denies patches to it.
    #[test]
    fn locked_operational_field_is_denied() {
        let guard = SoulGuard::new();
        let d = doc_with_locked(&["Auto-execute"]);
        let patch = "## Auto-execute\n\n- Run all tests.";
        let result = guard.validate_patch(&d, patch);
        assert!(!result.allowed, "locked operational field must be denied");
    }

    // ── Multiple sections ─────────────────────────────────────────────────

    /// A patch that mixes personality and operational sections is gated by the
    /// first personality hit — requires approval.
    #[test]
    fn mixed_patch_personality_wins() {
        let guard = SoulGuard::new();
        let patch = "## Auto-execute\n\n- cargo build\n\n## Voice\n\nBe more terse.";
        let result = guard.validate_patch(&doc(), patch);
        assert!(
            result.requires_approval,
            "mixed patch containing personality field must require approval"
        );
    }

    // ── Section extraction ────────────────────────────────────────────────

    #[test]
    fn extract_sections_finds_h2_and_h3() {
        let text = "## Voice\n### Sub-section\n# Title ignored\n## Values";
        let sections = super::extract_sections(text);
        assert!(sections.contains(&"voice".to_owned()));
        assert!(sections.contains(&"sub-section".to_owned()));
        assert!(sections.contains(&"values".to_owned()));
        assert!(
            !sections.iter().any(|s| s == "title ignored"),
            "H1 must be ignored"
        );
    }

    #[test]
    fn extract_sections_empty_for_no_headings() {
        let text = "Some plain text without any headings.";
        assert!(super::extract_sections(text).is_empty());
    }
}
