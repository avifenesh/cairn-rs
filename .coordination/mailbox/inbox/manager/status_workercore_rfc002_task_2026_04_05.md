# Status Update — Worker Core

## Task: task_state_machine (RFC 002)
- **Tests**: 12/12 pass
- **Files created**: crates/cairn-store/tests/task_state_machine.rs
- **Files changed**: none
- **Issues**: async closure lifetime error on first compile — replaced with macro for inline assertions
- **Adaptations**:
  - "WaitingInput" does not exist in TaskState. Used WaitingApproval (the human-input gate). WaitingDependency is the dependency-waiting gate.
  - Real path is Queued→Leased→Running (store does not validate; runtime enforces). Tests use the real valid transitions.
  - list_by_run = TaskReadModel::list_by_parent_run (parent_run_id-scoped query)
- **Notable**: RetryableFailed increments retry_count (not a terminal state — task re-queues); version increments on every transition verified step-by-step
