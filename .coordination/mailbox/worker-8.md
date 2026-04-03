# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | All planned work + hardening + cross-worker integration largely complete | Assistant streaming SSE families are wired from Worker 7's `StreamingOutput` types (`assistant_delta`, `assistant_end`, `assistant_reasoning` via `build_streaming_sse_frame`). External worker API endpoint is wired to Worker 4's `ExternalWorkerService`. All 9 preserved SSE families that can be implemented now have fixture-aligned payload shapes and tests. `cargo test --workspace` is green, but `crates/cairn-api/src/external_workers.rs` still emits 2 unused-import warnings that need cleanup.

## Blocked By

- 2026-04-03 | SSE enrichment (title/description/progress/context) requires store read-model queries — implementor task, not struct-shape task.

## Inbox

- 2026-04-03 | Manager -> Worker 8 | Current next focus: close the last API quality seams. Wire `MemoryApiImpl<R>` into the HTTP router with Worker 6, clean the unused imports in `crates/cairn-api/src/external_workers.rs`, and then take the first store-backed pass on richer SSE enrichment if time remains.
- 2026-04-03 | Worker 4 -> Worker 8 | (addressed: ExternalWorkerEndpoints created)
- 2026-04-03 | Worker 7 -> Worker 8 | (addressed: streaming SSE families wired)
- 2026-04-03 | Worker 6 -> Worker 8 | `MemoryApiImpl<R>` in cairn_memory implements MemoryEndpoints. Wire into HTTP router.

## Outbox

- 2026-04-03 | Worker 8 -> Worker 7 | `build_streaming_sse_frame` maps all StreamingOutput variants to preserved SSE frames. `ToolResult` returns None (not in preserved catalog).
- 2026-04-03 | Worker 8 -> Worker 4 | `ExternalWorkerEndpoints::report` API boundary ready. Routes worker reports through runtime's ExternalWorkerService.
- 2026-04-03 | Worker 8 -> Worker 1 | All 9 implementable preserved SSE families now have fixture-aligned tests: task_update, approval_required, assistant_tool_call, agent_progress, poll_completed, feed_update, assistant_delta, assistant_end, assistant_reasoning.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `crates/cairn-api/src/sse_payloads.rs` (streaming SSE), `crates/cairn-api/src/external_workers.rs`, and `tests/sse_payload_alignment.rs` (9 fixture tests). 71 tests, workspace green, 2 unused-import warnings pending cleanup in `external_workers.rs`.
