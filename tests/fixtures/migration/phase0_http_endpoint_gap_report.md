# Phase 0 HTTP Endpoint Gap Report

Status: generated  
Purpose: show how the current Rust API endpoint surface relates to the preserved Phase 0 HTTP contract.

Current reading:

- this report is based on the current `cairn-api/src/endpoints.rs` service boundary plus the preserved Phase 0 HTTP requirement set
- it complements the route catalog and fixture checks by showing which preserved routes already have explicit Rust-side endpoint boundaries and which still exist only as compatibility inventory + fixtures
- Worker 1 should use this report to keep API-surface drift visible while Worker 8 expands product endpoints intentionally

Interpretation:

- `read_endpoint_trait_present`: a Rust-side read endpoint/service seam already exists for the preserved route family
- `dedicated_endpoint_trait_present`: a dedicated preserved-route endpoint or mutation trait exists outside the generic runtime read boundary
- `dedicated_endpoint_trait_present_followup_remaining`: the route seam exists, but the current serialized request or response shape is still thinner than the preserved fixture contract
- `stream_publisher_present_followup_remaining`: the stream surface exists, but compatibility work remains before it is locked
- `no_explicit_api_boundary_yet`: preserved route exists in the catalog and fixtures, but no dedicated Rust-side endpoint/mutation seam is visible yet

## Phase 0 HTTP Status

| Requirement | Current Status | Notes | Next Step |
|---|---|---|---|
| `GET /v1/feed?limit=20&unread=true` | `dedicated_endpoint_trait_present` | `FeedEndpoints::list` plus read-marking boundaries exist in `feed.rs`, and the current `ListResponse<FeedItem>` shape matches the preserved feed fixture contract. | `keep_contract_stable` |
| `GET /v1/tasks?status=running&type=agent` | `read_endpoint_trait_present` | `RuntimeReadEndpoints::list_tasks` exists and already uses the shared `ListQuery` boundary. | `keep_contract_stable` |
| `GET /v1/approvals?status=pending` | `read_endpoint_trait_present` | `RuntimeReadEndpoints::list_approvals` exists and already uses the shared `ListQuery` boundary. | `keep_contract_stable` |
| `GET /v1/memories/search?q=test&limit=10` | `dedicated_endpoint_trait_present` | `MemoryEndpoints::search` exists in `memory_api.rs`, and `cairn-app` now composes the preserved `/v1/memories*` routes through a live `MemoryApiImpl`, preserving the `source`, `confidence`, and ISO-string `createdAt` response fields exercised by the fixture-alignment tests. | `keep_contract_stable` |
| `POST /v1/assistant/message body={message,mode?,sessionId?}` | `dedicated_endpoint_trait_present` | `AssistantEndpoints::send_message` exists in `assistant.rs`, so the preserved assistant-message mutation now has an explicit Rust-side command boundary. | `keep_contract_stable` |
| `POST /v1/assistant/message body={message,mode?}` | `dedicated_endpoint_trait_present` | `AssistantEndpoints::send_message` exists in `assistant.rs`, so the preserved assistant-message mutation now has an explicit Rust-side command boundary. | `keep_contract_stable` |
| `GET /v1/stream?lastEventId=<id>` | `stream_publisher_present_followup_remaining` | `SsePublisher`, `build_sse_frame`, and `parse_last_event_id` exist, but preserved SSE payload-shape alignment is still an explicit follow-up. | `align_sse_payload_shape` |
