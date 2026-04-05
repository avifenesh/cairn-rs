# Worker Core Mailbox

Owner: domain, store, runtime, tools

## Current Status

- 2026-04-05 | Worker Core | Fixing workspace compile errors from misplaced/missing `ProviderCallCompleted` fields in `crates/cairn-api/src/sse_publisher.rs`, `crates/cairn-store/src/in_memory.rs`, `crates/cairn-store/src/pg/projections.rs`, and `crates/cairn-app/src/lib.rs`. Blocker: none.
- 2026-04-03 | Worker Core | Primary surfaces are `cairn-domain`, `cairn-store`, `cairn-runtime`, and `cairn-tools`.
- 2026-04-03 | Worker Core | Core crates are green. The remaining job is supporting honest enriched API/SSE surfaces with durable runtime/store truth, not broadening the state model.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker Core | The active seam here is runtime/store support for exact `task_update`, `approval_required`, and richer `assistant_tool_call` payload semantics when the surface worker chooses to close those gaps.

## Outbox

- 2026-04-05 | Worker Core -> Manager | Checked the requested `ProviderCallCompleted` / `TaskRecord` seam in `crates/cairn-api/src/sse_publisher.rs` and `crates/cairn-app/src/lib.rs`; current literals are already correct. Workspace compile is still blocked by unrelated test compile errors in `crates/cairn-memory/tests/bundle_round_trip.rs`, `crates/cairn-store/tests/rfc_compliance_summary.rs`, and `crates/cairn-store/tests/prompt_asset_scoping.rs`, outside the requested field-only edit scope.

## Ready For Review

- [`docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
- [`crates/cairn-runtime/src/services/recovery_impl.rs`](../../crates/cairn-runtime/src/services/recovery_impl.rs)
- [`crates/cairn-store/src/projections`](../../crates/cairn-store/src/projections)
