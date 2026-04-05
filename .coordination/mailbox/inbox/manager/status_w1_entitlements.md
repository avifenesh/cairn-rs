# STATUS: entitlements

**Task:** RFC 014 commercial entitlements integration test  
**Tests passed:** 10/10  
**File:** `crates/cairn-store/tests/entitlements.rs`

Tests:
- `license_activated_is_stored_and_queryable`
- `no_license_when_none_activated`
- `entitlement_override_is_stored_and_queryable`
- `multiple_overrides_all_listed`
- `unknown_feature_is_denied_fail_closed`
- `ga_features_always_allowed`
- `entitlement_gated_feature_allowed_with_entitlement`
- `entitlement_gated_feature_denied_without_entitlement`
- `incremental_entitlement_grants_access`
- `full_pipeline_license_to_gate_decision`
