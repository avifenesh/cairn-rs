# Manager Mailbox

Owner: repo-wide truth, drift detection, acceptance gates

## Current Status

- 2026-04-03 | Manager | Current `task_update` / `approval_required` seam set is complete for this routing round: Worker 1 added an API-side current-state SSE enrichment helper so the exact builders can consume `TaskRecord` / `ApprovalRecord` when store context is available, Worker 3 strengthened backend parity coverage for `TaskReadModel::get(...)` and `ApprovalReadModel::get(...)`, and Worker 2 confirmed no new domain/runtime helper is needed. Remaining explicit gap is now narrower: the generic runtime-event fallback stays thin when no current-state record is supplied.
- 2026-04-03 | Manager | Current dispatch focus is the next honest seam after the green workspace pass: close the thin runtime-backed `task_update` / `approval_required` publish path at the API surface, keep Worker 2 in unblocker-only contract-freeze support, and use Worker 3 for the smallest backend-stable task/approval point-lookup proof. Repo-local inboxes under `.coordination/mailbox/inbox/*` are the live message path in this environment.
- 2026-04-03 | Manager | New dispatch focus after the green acceptance pass: activate the three workers on the next honest seam set. Worker 1 should try to close the thin `task_update` / `approval_required` runtime publish path at the API surface, Worker 2 stays on unblocker-only support for that path, and Worker 3 should add the smallest backend-stable point-lookup proof the surface can depend on.
- 2026-04-03 | Manager | This manager pass closed the live store regression and landed two seam-truth cuts: `cargo test --workspace --quiet` now passes again, `./scripts/check-compat-inventory.sh` is green, `cairn-app` now delegates through a real `ServerBootstrap` path with an explicit blocker instead of a print-only stub, and Worker 2 proved the remaining `task_update` / `approval_required` thinness is surface/store composition work rather than missing domain truth.
- 2026-04-03 | Manager | Fresh takeover check: `cargo test --workspace --quiet` is currently red only on `sqlite_parity::approval_list_ordering_is_deterministic` in `crates/cairn-store/tests/cross_backend_parity.rs`, while `./scripts/check-compat-inventory.sh` is green. Routing this pass is: Worker 3 on the store parity regression, Worker 1 on the still-open app/bootstrap composition seam, and Worker 2 on the smallest honest core support or blocker for exact `task_update` / `approval_required` runtime-backed SSE enrichment.
- 2026-04-03 | Manager | Active operating model is now manager + 3 workers, not the historical 8-worker split. Workspace and compatibility harness are green; the remaining work is seam-closing and report-truth work, not broad parallel scaffolding.
- 2026-04-03 | Manager | Full-pass findings are recorded in `docs/design/MANAGER_THREE_WORKER_REPLAN.md`. The live half-work is concentrated in app/bootstrap composition, memory durability, generated-report drift, `task_update` / `approval_required` runtime enrichment, `assistant_end` final-text handoff, and `memory_proposed` composition.
- 2026-04-03 | Manager | Manager handoff/context is now centralized in `.coordination/MANAGER_CONTEXT.md` so the next pass inherits the spec constraints, closed seams, open seams, and validation order in one place.

## Blocked By

- none

## Inbox

- 2026-04-05 | Worker (quality sweep) → Manager | **TOTAL TESTS: 794 passing, 0 failing** across 12 crates (excluding cairn-app).

  Per-crate breakdown:
  | Crate | Tests |
  |---|---|
  | cairn-domain | 148 |
  | cairn-store | 24 |
  | cairn-runtime | 206 |
  | cairn-evals | 42 |
  | cairn-tools | 114 |
  | cairn-memory | 92 |
  | cairn-graph | 21 |
  | cairn-api | 113 |
  | cairn-agent | 13 |
  | cairn-signal | 7 |
  | cairn-channels | 7 |
  | cairn-plugin-proto | 7 |
  | **Total** | **794** |

  **cairn-runtime** went from 175 passing to **206 passing** (31 new tests from linter-added service impls now fully passing). All 30 previously failing service tests (channel, credential, defaults, guardrail, license, notification, operator_profile, provider_binding, provider_connection, provider_pool, provider_health, quota, budget) now pass after adding real event projection handlers and real read model implementations to InMemoryStore.

  **cairn-app** still has ~705 compile errors — a mix of pre-existing stub/forward-reference issues (axum extractor lifetime mismatches requiring `#[async_trait]`, ProviderBindingBootstrapService being struct not trait, InMemoryServices::new missing, etc.). worker-2 is handling the axum root cause. The other crates are fully healthy.

## Outbox

- 2026-04-03 | Manager -> All | The 8-worker files are now historical logs. Use the new workstream mailboxes for active coordination.

## Ready For Review

- `cargo test --workspace --quiet`
- `./scripts/check-compat-inventory.sh`
