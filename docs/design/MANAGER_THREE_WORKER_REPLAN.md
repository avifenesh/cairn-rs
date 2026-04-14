# Cairn Rust Rewrite: Manager + 3 Worker Replan

Status: active coordination plan  
Date: 2026-04-03  
Audience: manager, active implementers, reviewers

Supersedes for active execution:

- [`EIGHT_WORKER_EXECUTION_PLAN.md`](./EIGHT_WORKER_EXECUTION_PLAN.md)
- [`MILESTONE_BOARD_WEEKS_1_4.md`](./MILESTONE_BOARD_WEEKS_1_4.md)

Those documents remain useful as historical scaffolding and ownership history, but they are no longer the right live operating model for this repo.

## Why This Replan Exists

The repo is no longer in the phase that justified 8 parallel worker slices.

A fresh reality check shows:

- `cargo test --workspace --quiet` passes
- `./scripts/check-compat-inventory.sh` passes
- most crate-level work is present and green
- the remaining work is concentrated in a small number of cross-crate seams, generated-report truthfulness issues, and app-composition gaps

That means the execution problem has changed:

- less "parallel scaffolding"
- more "finish the seams honestly"
- more "keep docs, tests, and generated reports aligned"

## Full-Pass Findings

### Stable Now

- workspace-wide tests are green
- `cairn-app` now boots a composed Axum server/router instead of acting as a bootstrap placeholder
- preserved feed HTTP and `feed_update` SSE fixtures are aligned to the string-ID contract
- `assistant_tool_call` completed/failed runtime payloads preserve `taskId`, `toolName`, and `phase`
- runtime recovery no longer has the earlier placeholder-only implementation
- queue automation is paused and should stay non-authoritative

### Real Gaps Still Open

These are the live half-finished or still-explicit seams worth tracking.

1. `cairn-app` is a real composed entrypoint now, but the product surface is still split between the library router and binary-only routes.

   Evidence:

   - [`crates/cairn-app/src/lib.rs`](../../crates/cairn-app/src/lib.rs)
   - [`crates/cairn-app/src/main.rs`](../../crates/cairn-app/src/main.rs)

   The binary now starts the server and layers binary-specific routes on top of `AppBootstrap::build_catalog_routes()`. The remaining seam is surface truth living across both files, which makes route and composition drift easier to miss.

2. `MemoryApiImpl` is now composed through the app surface, but it still relies on the in-memory document store plus generated IDs/timestamps instead of a canonical durable backing path.

   Evidence:

   - [`crates/cairn-memory/src/api_impl.rs`](../../crates/cairn-memory/src/api_impl.rs)

   Search and the preserved `/v1/memories*` routes are now service/composition-backed, but create/list/accept/reject still depend on the in-memory document store and generated IDs/timestamps. That is useful scaffolding, but it is still half-work relative to the product contract.

3. Generated migration reports still need active refreshes when compatibility work lands.

   Evidence:

   - [`tests/fixtures/migration/phase0_http_endpoint_gap_report.md`](../../tests/fixtures/migration/phase0_http_endpoint_gap_report.md)
   - [`crates/cairn-api/tests/http_boundary_alignment.rs`](../../crates/cairn-api/tests/http_boundary_alignment.rs)
   - [`crates/cairn-api/tests/migration_report_consistency.rs`](../../crates/cairn-api/tests/migration_report_consistency.rs)

   These reports are coordination artifacts, not passive notes. When a preserved contract claim changes, the generated reports and their consistency tests need to move in the same cut.

4. `task_update` and `approval_required` still have a split truth:

   - exact dedicated builders exist
   - generic runtime-event mapping is still thinner

   Evidence:

   - [`crates/cairn-api/tests/sse_payload_alignment.rs`](../../crates/cairn-api/tests/sse_payload_alignment.rs)
   - [`tests/fixtures/migration/phase0_sse_publisher_gap_report.md`](../../tests/fixtures/migration/phase0_sse_publisher_gap_report.md)

   This is an honest open seam. The contract is not "missing entirely"; it is "exact path exists, generic runtime path still thinner".

## Active Operating Model

The active execution model is now:

- 1 manager
- 3 workers

This is not a reduction in ambition. It is a better fit for the remaining shape of the work.

## Role Split

### Manager

Owns:

- repo-wide reality checks
- drift detection between code, tests, and generated reports
- coordination docs and mailbox truth
- acceptance gates for seam-closing work
- deciding when a seam is still open versus merely documented as open

The manager does not generate fake backlog. The manager keeps the system honest.

### Worker A: Surface And Contract Truth

Primary surface:

- `cairn-api`
- `cairn-app`
- `tests/compat`
- `tests/fixtures`
- generated migration reports

Owns:

- preserved HTTP/SSE contract truth
- app/bootstrap composition
- SSE/HTTP report truthfulness
- fixture alignment
- deciding whether the exact builder path or the generic runtime path is the contract we actually want to preserve

Current focus:

- reconcile generated reports with the code/tests that already pass
- keep the now-composed preserved memory routes and `memory_proposed` path reflected honestly in reports and fixtures
- keep the dedicated assistant streaming families honest as the live app surface grows into them

### Worker B: Runtime And Durable Core

Primary surface:

- `cairn-domain`
- `cairn-store`
- `cairn-runtime`
- `cairn-tools`

Owns:

- durable runtime truth
- store/read-model support for enriched API surfaces
- tool lifecycle semantics that the API layer should surface
- making sure exact API/SSE builders are backed by honest core data rather than placeholders

Current focus:

- support exact `task_update` / `approval_required` enrichment from runtime/store truth
- expose any additional durable tool result/error detail needed for `assistant_tool_call`
- avoid broadening core contracts unless the product surface truly requires it

### Worker C: Knowledge, Memory, And Agent Surfaces

Primary surface:

- `cairn-memory`
- `cairn-graph`
- `cairn-agent`
- `cairn-evals`

Owns:

- retrieval and memory honesty
- graph/provenance support
- streaming and agent-facing composition support
- converting temporary memory-side seams into durable product seams where needed

Current focus:

- replace temporary memory CRUD/state shortcuts with a more honest backing path
- keep the composed memory contract honest while durable backing replaces the in-memory shortcut
- keep assistant streaming/eval/graph surfaces stable while Worker A finalizes product contract truth

## How We Decide Work Now

We are no longer optimizing for "everyone must always have a task".

We are optimizing for:

- no stale manager stories
- no generated-report drift
- no test-green / contract-false state
- no fake progress loops

That means a worker can be in support mode if:

- their surface is currently green
- the next real seam belongs elsewhere
- inventing more work would create churn instead of delivery

## Acceptance Standard

A seam is considered truly closed only when all of these agree:

- code
- executable tests
- generated reports
- coordination/docs

If one of those still disagrees, the seam is still open.

## Active Mailbox Model

The active mailboxes are now:

- [`../../.coordination/mailbox/manager.md`](../../.coordination/mailbox/manager.md)
- [`../../.coordination/mailbox/worker-surface.md`](../../.coordination/mailbox/worker-surface.md)
- [`../../.coordination/mailbox/worker-core.md`](../../.coordination/mailbox/worker-core.md)
- [`../../.coordination/mailbox/worker-knowledge.md`](../../.coordination/mailbox/worker-knowledge.md)

The older `worker-1.md` through `worker-8.md` files are preserved as execution history, not as the active coordination surface.
