# Status Update — Worker Core

## Task: GET /v1/runs/:id/tasks + TraceLayer
- **Handler added**: list_run_tasks_handler in main.rs
- **Route added**: GET /v1/runs/:id/tasks
- **Tests added**: 3 (all pass)
  - list_run_tasks_returns_empty_for_run_with_no_tasks
  - list_run_tasks_returns_tasks_for_run (2 tasks, parent_run_id verified)
  - list_run_tasks_returns_404_for_unknown_run
- **Binary test result**: 71 passed, 1 pre-existing failure (session_events_after_cursor_paginates uses SessionState::Closed which does not exist in the domain — not caused by these changes)

## Changes summary
1. Cargo.toml: added tower-http trace feature, tracing, tracing-subscriber
2. main.rs: TraceLayer + tracing init (previous task)
3. main.rs: TaskReadModel/TaskId imports, list_run_tasks_handler, /v1/runs/:id/tasks route
