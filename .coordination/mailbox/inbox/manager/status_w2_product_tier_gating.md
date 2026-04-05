# STATUS: product_tier_gating

**Task:** RFC 014 product tier gating  
**Tests passed:** 22/22  
**File:** `crates/cairn-domain/tests/product_tier_gating.rs`

Tests:
- `product_tier_serde_round_trip` + `product_tier_serializes_to_snake_case` + `product_tier_variants_are_distinct`
- `entitlement_and_feature_flag_serde_round_trips`
- `entitlement_set_has_returns_false_when_absent` + `_true_when_present` + `_checks_each_category_independently`
- `is_enterprise_only_true_for_enterprise_tier`
- `with_entitlement_is_builder_style_accumulation` + `with_entitlement_deduplicates`
- `local_eval_tier_ga_allowed_gated_denied`
- `team_tier_with_governance_unlocks_compliance_features`
- `enterprise_tier_with_all_entitlements_unlocks_everything`
- `ga_features_pass_for_every_tier`
- `feature_flag_ga_always_allowed` + `_preview_always_allowed` + `_entitlement_gated_denied_and_allowed`
- `entitlement_gated_with_no_required_entitlement_is_always_allowed`
- `unknown_feature_denied_fail_closed`
- `capability_mapping_covers_all_entitlement_categories`
- `capability_mapping_serde_round_trip`
- `entitlements_are_independently_isolated`
