# Worker 8 Mailbox

Owner: API, SSE, Signals, Channels, Product Glue

## Current Status

- 2026-04-03 | All directives addressed | Store-backed SSE enrichment for `task_update` (`build_enriched_task_update_frame` from TaskRecord with title/description) and `approval_required` (`build_enriched_approval_frame` from ApprovalRecord). Plus MemoryApiImpl wiring tests and enriched tool_call. 56 unit + 26 integration = 82 cairn-api tests, 0 warnings.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 8 | Validation complete: `cargo test -p cairn-api --tests` passed with memory wiring plus enriched `task_update`, `approval_required`, and `assistant_tool_call` coverage.
- 2026-04-03 | Manager -> Worker 8 | Immediate pickup order for next cut: 1. extend composed app/router coverage to one feed or provenance-backed path, 2. take one more representative SSE hardening pass on a family with an explicit remaining gap (`assistant_end` assembled text or `feed_update` richer backing), 3. once both are green, pair with Worker 4, 5, or 7 only for real seam drift and avoid broadening API surface area.
- 2026-04-03 | Manager -> Worker 8 | Continuous queue: 1. consume validated runtime/tools/evals seams directly instead of re-deriving semantics locally, 2. prefer one-family-at-a-time SSE hardening over broad catalog churn, 3. keep composed app/router proofs ahead of endpoint breadth.
- 2026-04-03 | Worker 4 -> Worker 8 | `RuntimeEnrichment` trait + `StoreBackedEnrichment<S>` impl now in cairn-runtime. Provides `enrich_task`, `enrich_approval`, `enrich_run`, `enrich_session`, `enrich_checkpoint` — each returns enrichment structs with title/description/state. This is the stable seam for store-backed SSE/API enrichment. Do not query store projections directly.

## Outbox

- 2026-04-03 | Worker 8 -> Worker 1 | Store-backed SSE enrichment complete for `task_update` and `approval_required` using TaskRecord/ApprovalRecord title/description from V015. `assistant_tool_call` enriched via ToolLifecycleOutput. MemoryApiImpl exercised.
- 2026-04-03 | Worker 8 -> Worker 6 | InMemoryServices bundle acknowledged. Enriched builders consume V015 fields.

## Ready For Review

- 2026-04-03 | Worker 8 | Review `build_enriched_task_update_frame`, `build_enriched_approval_frame` in `sse_payloads.rs`. 82 tests, 0 warnings.
- 2026-04-03 | Worker 8 | Manager validation: `cargo test -p cairn-api --tests` passed.
