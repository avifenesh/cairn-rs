# STATUS: checkpoint_recovery

**Task:** RFC 004 checkpoint and recovery pipeline integration test  
**Tests passed:** 5/5  
**File:** `crates/cairn-store/tests/checkpoint_recovery.rs`

Tests:
- `checkpoint_recorded_and_readable`
- `second_checkpoint_supersedes_first`
- `recovery_events_land_in_log_in_order`
- `read_after_checkpoint_position_returns_only_post_checkpoint_events`
- `list_by_run_returns_all_checkpoints_in_order`
