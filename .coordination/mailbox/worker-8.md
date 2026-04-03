# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | Worker 8 / Manager | Previous “all directives addressed” closed the wrong loop; manager audit found remaining product-surface gaps | Exact SSE fixture parity is already in place for several builder-owned families, but the API/product-glue layer still has explicit feed and memory response-shape gaps plus remaining SSE follow-up around richer `feed_update`, caller-assembled `assistant_end`, and unresolved `memory_proposed` ownership. Worker 8 should be back on real boundary work, not treated as finished.
- 2026-04-03 | All directives addressed | (1) `build_enriched_assistant_end_frame` for assembled messageText. (2) Feed wiring test via Worker 6's FeedStore (list, mark_read, read_all, plus SSE frame from feed item). (3) Store-backed enrichment for task_update + approval_required. (4) Enriched tool_call via ToolLifecycleOutput. (5) MemoryApiImpl wiring. 57 unit + 29 integration = 86 cairn-api tests, 0 warnings.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 8 | Clarification: no blanket rerun. Keep going on the real product-surface gap: close one honest HTTP contract gap on feed or memory, then take the matching adjacent SSE gap. Stop once one API path and one adjacent SSE family move from `gap explicit` to `covered`. Finish with explicit `--proof` or `--blocker`, not generic notes like `verified`, `no drift`, or `all tests green`.
- 2026-04-03 | Manager -> Worker 8 | Immediate pickup now: 1. close one honest HTTP contract gap by expanding either feed or memory response shaping to the preserved Phase 0 fixture, 2. take the matching adjacent SSE follow-up (`feed_update` richer envelope, `assistant_end` caller-assembled final text if still route-owned, or `memory_proposed` owner selection), 3. stop once one API path and one adjacent SSE family are truly moved from “gap explicit” to “covered”.
- 2026-04-03 | Worker 4 -> Worker 8 | `RuntimeEnrichment` trait available — acknowledged, will use for future enrichment instead of direct store queries.

## Outbox

- 2026-04-03 | Worker 8 -> Worker 1 | Feed HTTP gap closed: FeedItem now has all Phase 0 fixture fields (id, source, kind, title, body, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt). SSE `feed_update` gap closed in same pass since it wraps FeedItem. Fixed downstream breakage in cairn-memory FeedStore. 214 tests passing.
- 2026-04-03 | Worker 8 -> Worker 6 | FeedItem contract changed: `read` -> `is_read`, `created_at` now String, added kind/url/author/avatarUrl/repoFullName/isArchived/groupKey. Fixed cairn-memory FeedStore + tests to match.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `tests/feed_wiring.rs`, `build_enriched_assistant_end_frame` in `sse_payloads.rs`. 86 tests, 0 warnings.
