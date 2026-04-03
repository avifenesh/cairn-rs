# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | All work complete | All 9 preserved SSE families have fixture-aligned payloads and tests. External worker endpoint wired. Streaming SSE wired from Worker 7. Unused imports cleaned. MemoryApiImpl wiring documented in cairn-app bootstrap. cairn-app depends on cairn-memory for router wiring. 71 cairn-api tests + cairn-app builds clean, 0 warnings.
- 2026-04-03 | Manager quality hold | Primary implementation slice is complete. Remaining value is API/product-glue polish: keep memory/provenance surfaces honest, watch for seam drift from Worker 5/6/7, and only reopen the API slice for real integration gaps.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 8 | Immediate pickup order for idle time: 1. replace documented bootstrap wiring with one executable composed-app/router test that exercises `MemoryApiImpl`, `FeedEndpoints`, and preserved HTTP surfaces together, 2. upgrade exactly one SSE family (`assistant_tool_call` or `task_update`) to use richer read-model-backed data where available, 3. if both are green, stay on seam-watch duty and only reopen the API slice for real drift from Workers 4/5/6/7.
- 2026-04-03 | Manager -> Worker 8 | Continuous queue: 1. replace documented bootstrap wiring with executable app/router coverage for `MemoryApiImpl`, `FeedEndpoints`, and preserved HTTP boundary tests, 2. take one representative read-model-backed SSE enrichment pass on either `assistant_tool_call` or `task_update`, 3. if both are green, stay on seam-watch duty and only reopen the API slice for real integration drift.
- 2026-04-03 | Manager -> Worker 8 | Next pacing cut: move from documented wiring to executable product-glue proof. Replace the current “wiring documented in bootstrap” stance with real router/app coverage that exercises `MemoryApiImpl`, `FeedEndpoints`, and the existing preserved HTTP boundary tests through composed application wiring.
- 2026-04-03 | Manager -> Worker 8 | After that, take one higher-fidelity SSE enrichment pass on exactly one family (`assistant_tool_call` or `task_update`) using real read-model data where available. Keep it representative; do not reopen the whole SSE catalog at once.
- 2026-04-03 | Manager -> Worker 8 | Current next focus: stay on API quality/support duty. Keep the slice green, pair with Worker 6 if memory/provenance wiring still needs real router coverage, and be ready to take the first store-backed SSE enrichment pass if runtime/store seams settle enough to support it.
- 2026-04-03 | Manager -> Worker 8 | Do not broaden API surface area. The only acceptable new work from this seat is integration-hardening, seam cleanup, or higher-fidelity read-model-backed SSE payloads.
- 2026-04-03 | Worker 6 -> Worker 8 | Ready for router wiring: `cairn_memory::services::InMemoryServices::new()` gives you a fully composed bundle with `.memory` (MemoryEndpoints), `.feed` (FeedEndpoints), `.ingest` (IngestService), `.deep_search` (DeepSearchService), `.diagnostics` (DiagnosticsService). All share one document store. Also: TaskRecord/ApprovalRecord now have `title`/`description` fields for SSE enrichment (V015 migration).

## Outbox

- 2026-04-03 | Worker 8 -> Worker 1 | All 9 preserved SSE families fixture-aligned. External worker and streaming SSE wired. Unused imports cleaned. Bootstrap documents service wiring.
- 2026-04-03 | Worker 8 -> Worker 6 | MemoryApiImpl wiring documented in cairn-app. cairn-app depends on cairn-memory.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `cairn-api` / `cairn-app` for preserved API/SSE compatibility plus product-glue wiring. 71 cairn-api tests, workspace green, 0 warnings in the API slice.
