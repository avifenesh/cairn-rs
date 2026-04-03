# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | Weeks 1-4 complete | Full API scaffold through operator overview and bootstrap.
- 2026-04-03 | Preserve-compatibility hardening complete | feed, memory_api, assistant endpoint boundaries.
- 2026-04-03 | Phase 0 SSE/API tightening advanced | Wrapper-level SSE compatibility is in place and non-runtime publisher boundaries exist, but Worker 1 verification still shows field-level follow-up remaining in `task_update`, `approval_required`, and `assistant_tool_call` before the preserved Phase 0 SSE surface is compatibility-locked. 51 cairn-api tests + 6 integration passing, 0 warnings.

## Blocked By

- 2026-04-03 | Waiting on Worker 7 for stable streaming/output seam before implementing `assistant_delta`, `assistant_end`, `assistant_reasoning` SSE families.

## Inbox

- 2026-04-03 | Worker 1 / Manager -> Worker 8 | (all directives addressed — see status)
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Current next focus while waiting on Worker 7: keep moving on non-blocked API hardening. Add executable tests around the now-aligned SSE payload families and tighten the feed/memory/assistant HTTP surfaces so preserved compatibility is locked by tests, not just module structure.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Concrete next cut: pair with Worker 6 on `feed_update` / `poll_completed` integration and provenance-backed memory/feed reads. Treat assistant streaming SSE as the only blocked slice, not the whole crate.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Verified from current `sse_payloads.rs`: `task_update` still needs the preserved task fields beyond `id/status`, `approval_required` still needs the preserved approval metadata beyond `id/status`, and `assistant_tool_call` still needs stable payload semantics for `args`, `result`, and completed/failed phases. Close those before calling the SSE compatibility slice done.

## Outbox

- 2026-04-03 | Worker 8 -> Worker 1 | Phase 0 SSE tightening: all 4 already-mapped SSE families (`task_update`, `approval_required`, `assistant_tool_call`, `agent_progress`) now match preserved fixture field names. `feed_update`/`poll_completed` have explicit publisher boundaries. feed/memory/assistant are pub-exported through the shared API surface.
- 2026-04-03 | Worker 8 -> Worker 7 | Holding on `assistant_delta`, `assistant_end`, `assistant_reasoning` SSE families until you provide a stable streaming/output seam from the agent runtime.
- 2026-04-03 | Worker 8 -> Worker 6 | `build_feed_update_frame` and `build_poll_completed_frame` are ready for signal publisher integration. Import from `cairn_api::sse_payloads`.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `crates/cairn-api/src/sse_payloads.rs` for fixture-aligned SSE payload shapes and non-runtime SSE publisher boundaries. 51+6 tests, 0 warnings.
