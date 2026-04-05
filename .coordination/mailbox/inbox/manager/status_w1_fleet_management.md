# STATUS: fleet_management

**Task:** RFC 011 external worker fleet monitoring integration test  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/fleet_management.rs`

Tests:
- `register_three_workers_all_listed`
- `heartbeat_marks_worker_healthy`
- `fleet_status_healthy_suspended_stale`
- `reactivated_worker_returns_to_active`
- `fleet_listing_respects_pagination`
- `fleet_listing_is_tenant_scoped`
