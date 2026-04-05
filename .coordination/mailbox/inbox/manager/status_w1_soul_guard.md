# STATUS: soul_guard

**Task:** GAP-007 SOUL.md guardian integration test  
**Tests passed:** 10/10  
**File:** `crates/cairn-runtime/tests/soul_guard.rs`  
**Note:** SoulGuard lives in cairn-runtime (not cairn-domain) so test is in cairn-runtime/tests/.
Made extract_sections() pub to enable integration testing.

Tests:
- `soul_document_created_from_content`
- `personality_section_patch_requires_approval`
- `operational_section_patch_allowed_without_approval`
- `locked_field_patch_is_denied`
- `locked_field_denied_before_personality_check`
- `extract_sections_parses_h2_and_h3_headings`
- `extract_sections_empty_for_no_headings`
- `extract_sections_lowercases_output`
- `personality_and_operational_fields_are_disjoint`
- `mixed_patch_personality_takes_precedence_over_operational`
