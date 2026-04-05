# STATUS: eval_run_lifecycle

**Task:** RFC 004 eval run lifecycle test  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/eval_run_lifecycle.rs`

Tests:
- `eval_run_started_shows_running_state`
- `eval_run_completed_success_persists_state`
- `eval_run_failure_records_error_message`
- `list_by_project_returns_runs_in_order`
- `eval_run_links_to_prompt_asset_id`
- `eval_run_list_is_project_scoped`
