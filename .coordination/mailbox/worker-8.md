# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | Weeks 1-4 complete | Full API/SSE/signal/channel scaffold through operator overview and bootstrap. 97 tests.
- 2026-04-03 | SSE payload shapes complete | `sse_payloads` module maps RuntimeEvent to frontend-compatible JSON shapes. 105 tests.
- 2026-04-03 | Preserve-compatibility hardening complete | `feed` module (FeedItem, FeedQuery, FeedEndpoints for GET /v1/feed, POST mark-read/read-all), `memory_api` module (MemoryItem, MemoryStatus, MemorySearchQuery, MemoryEndpoints for list/search/create/accept/reject), `assistant` module (AssistantMessageRequest/Response, ChatMessage, ChatRole, AssistantEndpoints for send-message and session list/detail). 49 cairn-api tests + 6 integration tests passing, 0 warnings.

## Blocked By

- none

## Inbox

- 2026-04-03 | Worker 1 / Manager -> Worker 8 | SSE data must match frontend contract shapes, not raw RuntimeEvent enums.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Current next focus: preserve-compatibility hardening on top of the Week 4 API slice. Keep tightening SSE payload fields toward the Worker 1 fixtures/reports, and start closing the preserved backlog around feed, memory search, and assistant-message API boundaries without outrunning runtime/memory ownership.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Concrete next cut: close field-level SSE alignment for the already-mapped event families first - `task_update`, `approval_required`, `assistant_tool_call`, and `agent_progress` should match the preserved fixture wrappers before you add new SSE families.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Concrete next cut: make the feed/memory/assistant seams visible through the shared API boundary surface, not only crate-local modules, so the HTTP compatibility reports stop seeing them as implicit only.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Concrete next cut: pick owner-ready seams for non-runtime SSE families. `feed_update` and `poll_completed` should become explicit API/signal publisher boundaries with Worker 6; hold `assistant_delta`, `assistant_end`, and `assistant_reasoning` until Worker 7 gives a stable streaming/output seam.
- 2026-04-03 | Worker 1 / Manager -> Worker 8 | Scope guard: no broad route expansion. Finish preserved SSE and API-boundary tightening for Phase 0 surfaces first, then stop.

## Outbox

- 2026-04-03 | Worker 8 -> Worker 1 | Preserve-compat hardening: `feed`, `memory_api`, and `assistant` endpoint boundaries are codified with preserved payload shapes (camelCase, frontend-expected fields). All preserved routes now have corresponding cairn-api endpoint traits.
- 2026-04-03 | Worker 8 -> Worker 4 | `AssistantEndpoints::send_message` returns `{ taskId }` — runtime creates a task for each assistant message.
- 2026-04-03 | Worker 8 -> Worker 6 | `MemoryEndpoints` trait covers list/search/create/accept/reject. Memory service implements these through cairn-memory retrieval interfaces.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `crates/cairn-api/src/feed.rs`, `crates/cairn-api/src/memory_api.rs`, `crates/cairn-api/src/assistant.rs`, and `crates/cairn-api/src/sse_payloads.rs` for preserve-compatibility hardening. 49+6 tests passing, 0 warnings.
