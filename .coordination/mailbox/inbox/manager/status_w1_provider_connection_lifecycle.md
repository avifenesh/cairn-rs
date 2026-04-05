# STATUS: provider_connection_lifecycle

**Task:** RFC 007 provider connection lifecycle hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/provider_connection_lifecycle.rs`

Tests:
- `connection_registered_and_readable`
- `health_checked_healthy_updates_record`
- `full_lifecycle_register_healthy_degraded_recovered`
- `consecutive_failures_tracked_and_reset`
- `multiple_connections_tracked_independently`
- `degrade_without_prior_health_check_creates_record`
