# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | Worker 8 / Manager | Current product-surface gaps are now very narrow and still honest in the generated reports | Memory HTTP shape is already aligned and should stay closed. The live remaining API/SSE decisions are the `FeedItem.id` numeric-vs-string preserved contract and the `memory_proposed` publisher owner. The generated migration reports still name those two gaps explicitly, and that is correct today.
- 2026-04-03 | --proof | Feed HTTP gap closed: `FeedItem` expanded to all Phase 0 fixture fields (kind, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt as ISO string). `feed_update` SSE gap closed in same pass. Downstream cairn-memory FeedStore fixed. Test: `feed_response_gap_is_still_explicit` in `http_boundary_alignment.rs` now asserts field-by-field match against fixture.
- 2026-04-03 | --proof | Memory search HTTP gap closed: `MemoryItem` expanded with `source` (Option), `confidence` (Option<f64>), `created_at` as ISO string. `memory_proposed` SSE builder added: `build_memory_proposed_frame` wraps full MemoryItem. Test: `memory_search_response_matches_fixture` in `http_boundary_alignment.rs`.
- 2026-04-03 | --proof | `assistant_end` caller-assembled path made real: `build_streaming_sse_frame` returns `None` for AssistantEnd (forces enriched builder). `assistant_end_caller_assembled_text_composition` test proves delta accumulation -> enriched end frame. Test: in `product_surface_composition.rs`.
- 2026-04-03 | --proof | 219 tests passing across cairn-api + cairn-tools + cairn-plugin-proto + cairn-memory. 0 warnings. InMemoryStore `list_by_state` stub added to unblock workspace.

## Blocked By

- 2026-04-03 | --blocker | `FeedItem.id` type mismatch: preserved fixture models `id` as numeric (`101`), our API serializes as string (`"101"`). Changing to numeric would break all downstream string-based ID handling (FeedStore, tests, cairn-memory). Options: (a) change `id` to `serde_json::Value` and accept mixed-type handling, (b) change fixture to string if frontend treats IDs as opaque, (c) add `#[serde(deserialize_with)]` for flexible parsing. Decision needed from Worker 1 (fixture authority) or architecture owner.
- 2026-04-03 | --blocker | `memory_proposed` SSE publisher ownership undecided. Builder exists (`build_memory_proposed_frame` in cairn-api), but no code path decides WHEN to emit it. Candidate owners: (a) Worker 6's cairn-memory proposal flow emits it when a memory is proposed, (b) Worker 4's runtime emits it as a RuntimeEvent. Needs Worker 6 + architecture owner decision. Builder is ready — just needs a caller.

## Inbox

- 2026-04-03 | Manager -> Worker 8 | Packed next cut: 1. resolve the `FeedItem.id` contract one way or the other by making HTTP/SSE/tests/reports agree on either numeric preservation or string-as-opaque truth, 2. keep `feed_update` and the migration reports consistent with that decision, 3. then pair with Worker 6 on the smallest real `memory_proposed` publisher-owner decision and leave a blocker if ownership is still not settled.
- 2026-04-03 | Manager -> Worker 8 | Packed next cut: 1. decide the preserved feed `id` contract and either implement the API/SSE side to match it or update the compatibility reports/tests truthfully, 2. keep the resulting `feed_update` story consistent across `http_boundary_alignment`, `sse_payload_alignment`, and the generated migration reports, 3. then pair with Worker 6 on the smallest real `memory_proposed` owner/builder decision or leave that gap explicitly open with proof.
- 2026-04-03 | Manager -> Worker 8 | Correction after code check: memory HTTP shape is already aligned, so do not spend time reopening that. The honest remaining product-surface gaps are narrower now: 1. the preserved feed fixture still models `FeedItem.id` as numeric while our API/SSE path serializes it as a string, and 2. `memory_proposed` still lacks a decided publisher owner. New concrete direction: close the feed id contract mismatch if we intend to preserve the frontend fixture literally; otherwise update the generated reports/tests truthfully and leave a precise blocker. After that, pair with Worker 6 on the smallest real `memory_proposed` ownership decision or keep that gap explicit.
- 2026-04-03 | Manager -> Worker 8 | Clarification: no blanket rerun. Keep going on the real product-surface gap: close one honest HTTP contract gap on feed or memory, then take the matching adjacent SSE gap. Stop once one API path and one adjacent SSE family move from `gap explicit` to `covered`. Finish with explicit `--proof` or `--blocker`, not generic notes like `verified`, `no drift`, or `all tests green`.
- 2026-04-03 | Manager -> Worker 8 | Immediate pickup now: 1. close one honest HTTP contract gap by expanding either feed or memory response shaping to the preserved Phase 0 fixture, 2. take the matching adjacent SSE follow-up (`feed_update` richer envelope, `assistant_end` caller-assembled final text if still route-owned, or `memory_proposed` owner selection), 3. stop once one API path and one adjacent SSE family are truly moved from “gap explicit” to “covered”.
- 2026-04-03 | Worker 4 -> Worker 8 | `RuntimeEnrichment` trait available — acknowledged, will use for future enrichment instead of direct store queries.

## Outbox

- 2026-04-03 | Worker 8 -> Worker 1 | Feed HTTP gap closed: FeedItem now has all Phase 0 fixture fields (id, source, kind, title, body, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt). SSE `feed_update` gap closed in same pass since it wraps FeedItem. Fixed downstream breakage in cairn-memory FeedStore. 214 tests passing.
- 2026-04-03 | Worker 8 -> Worker 6 | FeedItem contract changed: `read` -> `is_read`, `created_at` now String, added kind/url/author/avatarUrl/repoFullName/isArchived/groupKey. Fixed cairn-memory FeedStore + tests to match.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `tests/feed_wiring.rs`, `build_enriched_assistant_end_frame` in `sse_payloads.rs`. 86 tests, 0 warnings.
