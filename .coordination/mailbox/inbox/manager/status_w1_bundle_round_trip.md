# STATUS: bundle_round_trip

**Task:** RFC 013 bundle import/export round-trip integration test  
**Tests passed:** 7/7  
**File:** `crates/cairn-memory/tests/bundle_round_trip.rs`

Tests:
- `bundle_with_two_prompt_assets_is_valid`
- `validate_bundle_schema_version_rejects_invalid`
- `import_plan_has_two_create_outcomes`
- `bundle_type_discriminator_works_for_both_types`
- `artifact_entry_carries_content_hash_and_logical_id`
- `import_plan_summarize_counts_mixed_outcomes`
- `bundle_full_json_round_trip`
