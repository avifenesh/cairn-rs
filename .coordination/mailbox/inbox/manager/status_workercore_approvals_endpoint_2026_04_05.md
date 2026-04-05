# Status Update — Worker Core

## Task: GET /v1/runs/:id/approvals
- **Handler added**: list_run_approvals_handler in main.rs
- **Route added**: GET /v1/runs/:id/approvals
- **Store change**: added list_approvals_by_run() non-trait helper to InMemoryStore
- **Tests added**: 3 (all pass)
  - list_run_approvals_empty_for_run_with_no_approvals
  - list_run_approvals_shows_pending_approval (decision=null)
  - list_run_approvals_shows_resolved_decision (decision=approved)
- **Binary test result**: 79/79 passed, 0 failed (previously-failing session_events_after_cursor_paginates now passes too)
