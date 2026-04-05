# STATUS: feature_gate_enforcement

**Task:** RFC 014 commercial gate enforcement hardening  
**Tests passed:** 7/7  
**File:** `crates/cairn-store/tests/feature_gate_enforcement.rs`

Tests:
- `ga_features_allowed_without_entitlements`
- `gated_features_denied_without_entitlement`
- `unknown_features_denied_fail_closed`
- `entitlement_override_event_unlocks_gated_feature`
- `multiple_overrides_grant_and_deny_independently`
- `capability_mapping_links_features_to_entitlements`
- `custom_capability_mapping_works_as_gate`
