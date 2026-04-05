//! GAP-007 SOUL.md guardian integration tests.
//!
//! Validates the three-tier SoulGuard pipeline:
//! - Personality/identity section patches require operator approval.
//! - Operational section patches are allowed immediately.
//! - Locked field patches are denied unconditionally.
//! - extract_sections() correctly parses Markdown ## headings.
//!
//! Note: SoulGuard lives in cairn-runtime (depends on cairn-domain types).
//! These tests use the public API: SoulGuard + SoulDocument from cairn-domain.

use cairn_domain::soul::{
    SoulDocument, OPERATIONAL_FIELDS, PERSONALITY_FIELDS,
};
use cairn_runtime::soul_guard::{SoulGuard, extract_sections};

// ── helpers ───────────────────────────────────────────────────────────────────

/// A realistic SOUL.md document that mirrors what a deployed agent would carry.
fn sample_soul_doc() -> SoulDocument {
    SoulDocument::new(
        "# Cairn Agent\n\
         \n\
         ## Who I Am\n\
         I am Cairn, an AI assistant built to help software engineers.\n\
         \n\
         ## Voice\n\
         Direct, precise, and collaborative. No unnecessary filler.\n\
         \n\
         ## Values\n\
         Correctness, transparency, and user autonomy above all.\n\
         \n\
         ## Auto-execute\n\
         - Run tests before committing.\n\
         - Format code with `cargo fmt`.\n\
         \n\
         ## Learned Patterns\n\
         - Prefer small, atomic commits.\n\
         - Ask before deleting files.\n",
    )
}

fn soul_doc_with_locked(locked: &[&str]) -> SoulDocument {
    sample_soul_doc().with_locked_fields(locked.iter().copied())
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Create SoulDocument from SOUL.md content.
#[test]
fn soul_document_created_from_content() {
    let doc = sample_soul_doc();

    assert!(!doc.content.is_empty(), "document content must be non-empty");
    assert_eq!(doc.version, 1, "fresh document must start at version 1");
    assert!(doc.locked_fields.is_empty(), "no locked fields by default");
    assert!(
        doc.content.contains("## Who I Am"),
        "document must contain Who I Am section"
    );
    assert!(
        doc.content.contains("## Auto-execute"),
        "document must contain operational section"
    );
}

/// (2) + (3) Propose a patch that changes personality section —
/// SoulGuard must require approval, not outright deny.
#[test]
fn personality_section_patch_requires_approval() {
    let guard = SoulGuard::new();
    let doc = sample_soul_doc();

    // Patch targeting "## Voice" — a personality field.
    let voice_patch = "## Voice\n\nBe warmer and more encouraging. Celebrate small wins.";
    let result = guard.validate_patch(&doc, voice_patch);

    assert!(
        result.allowed,
        "personality patch must be allowed (requires approval, not denied)"
    );
    assert!(
        result.requires_approval,
        "voice section change must require operator approval"
    );
    assert!(
        result.reason.contains("approval"),
        "reason must mention approval, got: {}",
        result.reason
    );

    // Patch targeting "## Who I Am" — also a personality field.
    let identity_patch = "## Who I Am\n\nI am Cairn v2, redesigned from scratch.";
    let identity_result = guard.validate_patch(&doc, identity_patch);
    assert!(identity_result.requires_approval, "who i am section must require approval");

    // Patch targeting "## Values" — personality field.
    let values_patch = "## Values\n\n- Efficiency over correctness (updated).";
    let values_result = guard.validate_patch(&doc, values_patch);
    assert!(values_result.requires_approval, "values section must require approval");
}

/// (4) + (5) Propose operational fact change — SoulGuard allows it without approval.
#[test]
fn operational_section_patch_allowed_without_approval() {
    let guard = SoulGuard::new();
    let doc = sample_soul_doc();

    // Patch to "## Auto-execute" — an operational field.
    let auto_exec_patch = "## Auto-execute\n\n- Run `cargo clippy` before each PR.";
    let result = guard.validate_patch(&doc, auto_exec_patch);

    assert!(result.allowed, "operational patch must be allowed");
    assert!(
        !result.requires_approval,
        "operational patch must NOT require approval"
    );

    // Patch to "## Learned Patterns" — also operational.
    let patterns_patch = "## Learned Patterns\n\n- Always use explicit type annotations in Rust.";
    let patterns_result = guard.validate_patch(&doc, patterns_patch);
    assert!(patterns_result.allowed);
    assert!(!patterns_result.requires_approval);

    // Patch with no section markers is treated as an operational addition.
    let bare_patch = "Always prefer `unwrap_or_else` over `unwrap_or` for lazy evaluation.";
    let bare_result = guard.validate_patch(&doc, bare_patch);
    assert!(bare_result.allowed, "headingless patch treated as operational");
    assert!(!bare_result.requires_approval);
}

/// (6) + (7) Propose locked field change — SoulGuard denies it unconditionally.
#[test]
fn locked_field_patch_is_denied() {
    let guard = SoulGuard::new();

    // Lock "Voice" — patch targeting it must be denied.
    let doc_locked_voice = soul_doc_with_locked(&["Voice"]);
    let voice_patch = "## Voice\n\nBe more casual and use emojis freely.";
    let result = guard.validate_patch(&doc_locked_voice, voice_patch);

    assert!(
        !result.allowed,
        "patch to locked field 'voice' must be denied"
    );
    assert!(
        !result.requires_approval,
        "denied patches must NOT set requires_approval"
    );
    assert!(
        result.reason.contains("locked"),
        "denial reason must mention 'locked', got: {}",
        result.reason
    );

    // Locking an operational field also denies it.
    let doc_locked_ops = soul_doc_with_locked(&["Auto-execute"]);
    let ops_patch = "## Auto-execute\n\n- Allow deleting files automatically.";
    let ops_result = guard.validate_patch(&doc_locked_ops, ops_patch);
    assert!(!ops_result.allowed, "locked operational field must also be denied");

    // Patch to a DIFFERENT field must still pass (only the locked field is gated).
    let other_patch = "## Learned Patterns\n\n- Prefer early returns.";
    let other_result = guard.validate_patch(&doc_locked_ops, other_patch);
    assert!(
        other_result.allowed,
        "patch to non-locked field must still be allowed"
    );
}

/// Locked field denial takes precedence over personality check.
#[test]
fn locked_field_denied_before_personality_check() {
    let guard = SoulGuard::new();

    // Lock "Values" (which is also a personality field).
    // The denied result must come from the lock rule, not the personality rule.
    let doc = soul_doc_with_locked(&["Values"]);
    let patch = "## Values\n\n- Updated: efficiency over accuracy.";
    let result = guard.validate_patch(&doc, patch);

    assert!(!result.allowed, "locked field must be denied even though it's also a personality field");
    assert!(!result.requires_approval, "denial must not ask for approval");
}

/// (8) Test extract_sections() parsing: ## and ### are captured, # is ignored.
#[test]
fn extract_sections_parses_h2_and_h3_headings() {
    let text = "# Document Title (H1 — ignored)\n\
                \n\
                ## Voice\n\
                Some content here.\n\
                \n\
                ### Sub-section Detail\n\
                More content.\n\
                \n\
                ## Values\n\
                - Item 1\n\
                \n\
                ## Auto-execute\n\
                - Run tests.\n";

    let sections = extract_sections(text);

    // H2 sections must be captured.
    assert!(sections.contains(&"voice".to_owned()), "## Voice must be extracted");
    assert!(sections.contains(&"values".to_owned()), "## Values must be extracted");
    assert!(sections.contains(&"auto-execute".to_owned()), "## Auto-execute must be extracted");

    // H3 sections must also be captured.
    assert!(
        sections.contains(&"sub-section detail".to_owned()),
        "### Sub-section Detail must be extracted"
    );

    // H1 document title must be IGNORED (only # prefix, not ## or ###).
    assert!(
        !sections.iter().any(|s| s.contains("document title")),
        "H1 titles must not appear in extracted sections"
    );
}

/// extract_sections() returns empty vec for plain text with no headings.
#[test]
fn extract_sections_empty_for_no_headings() {
    let plain = "Just plain text content with no markdown headings at all.";
    let sections = extract_sections(plain);
    assert!(
        sections.is_empty(),
        "extract_sections must return empty vec for headingless text"
    );
}

/// extract_sections() lowercases all section names.
#[test]
fn extract_sections_lowercases_output() {
    let text = "## WHO I AM\n## Auto-Execute\n## VOICE";
    let sections = extract_sections(text);

    assert!(sections.contains(&"who i am".to_owned()), "section names must be lowercased");
    assert!(sections.contains(&"auto-execute".to_owned()));
    assert!(sections.contains(&"voice".to_owned()));
    assert!(
        !sections.iter().any(|s| s.chars().any(|c| c.is_uppercase())),
        "no uppercase characters should appear in extracted sections"
    );
}

/// PERSONALITY_FIELDS and OPERATIONAL_FIELDS constants are populated
/// and non-overlapping (no section is both personality and operational).
#[test]
fn personality_and_operational_fields_are_disjoint() {
    for pf in PERSONALITY_FIELDS {
        assert!(
            !OPERATIONAL_FIELDS.contains(pf),
            "field '{}' appears in both PERSONALITY_FIELDS and OPERATIONAL_FIELDS",
            pf
        );
    }
}

/// Mixed patch (operational + personality section) — the personality check wins,
/// so approval is required even though an operational section is also present.
#[test]
fn mixed_patch_personality_takes_precedence_over_operational() {
    let guard = SoulGuard::new();
    let doc = sample_soul_doc();

    let mixed_patch = "## Auto-execute\n\n- cargo fmt\n\n## Voice\n\nBe more direct.";
    let result = guard.validate_patch(&doc, mixed_patch);

    // Voice is a personality field → must require approval despite auto-execute being present.
    assert!(
        result.requires_approval,
        "mixed patch with a personality field must require approval"
    );
}
