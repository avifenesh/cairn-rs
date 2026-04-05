# STATUS: provider_routing_e2e

**Task:** RFC 009 multi-provider fallback chain E2E test  
**Tests passed:** 4/4  
**File:** `crates/cairn-runtime/tests/provider_routing_e2e.rs`

Tests:
- `primary_selected_when_all_capabilities_available`
- `fallback_used_when_primary_degraded`
- `route_decision_record_is_persisted`
- `no_viable_route_when_all_candidates_vetoed`
