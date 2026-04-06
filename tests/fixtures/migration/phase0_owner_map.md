# Phase 0 Compatibility Owner Map

Status: generated  
Purpose: keep preserved Phase 0 HTTP and SSE compatibility gaps routed to the right worker/module owners instead of turning Worker 1 reports into orphaned TODOs.

## HTTP Surfaces

| Surface | Current Boundary Signal | Likely Owner | Notes |
|---|---|---|---|
| `GET /v1/tasks?status=running&type=agent` | `cairn-api` route catalog + runtime/store read-model seam present | `Worker 8` with `Worker 4` + `Worker 3` support | Runtime-facing read endpoint; preserved contract mostly depends on operator/read-model shaping. |
| `GET /v1/approvals?status=pending` | `cairn-api` route catalog + runtime/store read-model seam present | `Worker 8` with `Worker 4` + `Worker 3` support | Approval inbox/read-model shaping, not a new storage contract. |
| `GET /v1/stream?lastEventId=<id>` | `cairn-api::sse_publisher` present | `Worker 8` with `Worker 4` / `Worker 6` / `Worker 7` depending on event family | Compatibility gap is mainly preserved payload shaping and event-family ownership. |
| `GET /v1/feed?limit=20&unread=true` | `FeedEndpoints` trait present in `feed.rs` | `Worker 8` + `Worker 6` | Feed/list surface now has an explicit API seam; the remaining work is backing it with signal/channel aggregation and read-model data. |
| `GET /v1/memories/search?q=test&limit=10` | `MemoryEndpoints::search` present in `memory_api.rs` | `Worker 6` + `Worker 8` | Retrieval/search boundary is explicit now; the remaining work is wiring it cleanly to `cairn-memory` retrieval services. |
| `POST /v1/assistant/message` (with session) | `AssistantEndpoints::send_message` present in `assistant.rs` | `Worker 8` + `Worker 4` + `Worker 7` | API seam exists; the remaining work is runtime/agent execution backing and later streaming output families. |
| `POST /v1/assistant/message` (without session) | `AssistantEndpoints::send_message` present in `assistant.rs` | `Worker 8` + `Worker 4` + `Worker 7` | Same as above, but bootstrap/create-session behavior still needs to stay explicit at the API/runtime seam. |

## SSE Surfaces

| Event | Current Boundary Signal | Likely Owner | Notes |
|---|---|---|---|
| `ready` | dedicated `build_ready_frame()` path exists | `Worker 8` | Already covered; preserve as-is. |
| `task_update` | exact dedicated builder present; current-state helper available; runtime event mapping still thinner | `Worker 8` + `Worker 4` | `build_enriched_task_update_frame(...)` already matches the preserved payload shape, and `build_sse_frame_with_current_state(...)` can hydrate read-model fields, but the generic runtime-event mapping still lacks read-model-backed task metadata. |
| `approval_required` | exact dedicated builder present; current-state helper available; runtime event mapping still thinner | `Worker 8` + `Worker 4` | `build_enriched_approval_frame(...)` already matches the preserved payload shape, and `build_sse_frame_with_current_state(...)` can hydrate read-model fields, but the generic runtime-event mapping still lacks read-model-backed approval metadata/context. |
| `assistant_tool_call` | exact start-phase payload plus enriched builder present; completed/failed runtime identity is preserved and richer result/error semantics still open | `Worker 8` + `Worker 4` + `Worker 5` | Tool lifecycle is runtime-owned; start is preserved, and completed/failed runtime mapping now preserves `taskId`, `toolName`, and `phase`, but still lacks richer result/error detail. |
| `agent_progress` | runtime event mapped via `sse_payloads`; current minimal fixture contract is exact | `Worker 8` + `Worker 4` + `Worker 7` | Minimal `{ agentId, message }` shape is already preserved; any richer agent/subagent progress semantics are later product work, not current compatibility drift. |
| `feed_update` | dedicated non-runtime builder present | `Worker 8` + `Worker 6` | Non-runtime publisher seam exists and the preserved feed item envelope now matches the current string-ID fixture contract. |
| `poll_completed` | dedicated non-runtime builder present | `Worker 8` + `Worker 6` | Non-runtime publisher seam exists and is already builder-backed; preserve as-is while wiring it to signal/source polling output. |
| `assistant_delta` | dedicated assistant-streaming builder present | `Worker 8` + `Worker 7` | Streaming assistant output ownership is explicit now; preserve the dedicated builder path and keep payload alignment stable. |
| `assistant_end` | dedicated assistant-streaming builder present; assembled final text handoff still open | `Worker 8` + `Worker 7` | Final assistant message event is builder-backed, but the caller still needs to supply the assembled final reply text instead of the current empty placeholder. |
| `assistant_reasoning` | dedicated assistant-streaming builder present | `Worker 8` + `Worker 7` | Reasoning stream/event family is builder-backed now; preserve the current owner boundary. |
| `memory_proposed` | dedicated non-runtime builder present; wired to SSE broadcast via `SseMemoryProposalHook` | `Worker 8` + `Worker 6` | `build_memory_proposed_frame(...)` is wired to the SSE broadcast channel in `cairn-app`; preserve as-is. |
