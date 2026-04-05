# STATUS: approval_blocking

**Task:** RFC 005 approval blocking hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/approval_blocking.rs`

Tests:
- `run_in_waiting_approval_state_shows_pending`
- `multiple_approvals_for_same_run_tracked_independently`
- `approval_policy_record_is_linkable`
- `approvals_are_isolated_per_project`
- `approval_version_increments_on_resolve`
- `rejected_approval_increments_version_and_leaves_pending`
