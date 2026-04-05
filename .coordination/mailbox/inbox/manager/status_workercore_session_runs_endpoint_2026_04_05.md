# Status Update — Worker Core

## Task: GET /v1/sessions/:id/runs
- **Handler added**: list_session_runs_handler in main.rs
- **Route added**: GET /v1/sessions/:id/runs  
- **Tests added**: 4 (all pass)
  - list_session_runs_empty_for_session_with_no_runs → 200 []
  - list_session_runs_returns_two_runs → both run_ids + session_id verified
  - list_session_runs_shows_parent_run_id_for_subagent → parent_run_id + agent_role_id for each
  - list_session_runs_returns_404_for_unknown_session → 404
- **Binary test result**: 95/95 pass (up from 79)
- **Note**: compile error seen mid-task was a stale build artifact; clean rebuild succeeded
