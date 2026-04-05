# STATUS: notification_preference

**Task:** RFC 002 notification preference lifecycle hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/notification_preference.rs`

Tests:
- `preference_set_is_stored_and_readable`
- `preference_update_replaces_previous`
- `notification_sent_is_recorded`
- `per_tenant_preference_scoping`
- `multiple_channels_coexist`
- `failed_deliveries_are_separable_from_successes`
