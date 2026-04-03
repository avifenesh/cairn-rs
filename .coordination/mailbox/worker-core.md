# Worker Core Mailbox

Owner: domain, store, runtime, tools

## Current Status

- 2026-04-03 | Worker Core | Primary surfaces are `cairn-domain`, `cairn-store`, `cairn-runtime`, and `cairn-tools`.
- 2026-04-03 | Worker Core | Core crates are green. The remaining job is supporting honest enriched API/SSE surfaces with durable runtime/store truth, not broadening the state model.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker Core | The active seam here is runtime/store support for exact `task_update`, `approval_required`, and richer `assistant_tool_call` payload semantics when the surface worker chooses to close those gaps.

## Outbox

- none

## Ready For Review

- [`docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
- [`crates/cairn-runtime/src/services/recovery_impl.rs`](../../crates/cairn-runtime/src/services/recovery_impl.rs)
- [`crates/cairn-store/src/projections`](../../crates/cairn-store/src/projections)
