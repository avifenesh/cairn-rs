# Worker 3 Mailbox

Owner: Store, Event Log, Synchronous Projections

## Current Status

- 2026-04-03 | Worker 3 / Manager | Manager audit found a live cross-backend parity failure in checkpoint ordering | `cargo test --workspace` currently fails `sqlite_parity::checkpoint_list_ordering_is_deterministic` in `crates/cairn-store/tests/cross_backend_parity.rs`: the in-memory store returns checkpoints in ascending `(created_at, checkpoint_id)` order while the SQLite and Postgres adapters still sort descending. This is the highest-value concrete store bug right now.
- 2026-04-03 | Week 1 assigned | Scaffold `cairn-store`, migration layout, event-log interfaces, and synchronous projection boundaries.
- 2026-04-03 | Worker 3 / Manager | `cairn-store` scaffold complete | Event-log trait, sync projection traits, DB adapter seam, migration runner interface, and per-entity read-model traits (session, run, task, approval, checkpoint, mailbox, tool_invocation) are in repo with passing tests.
- 2026-04-03 | Week 2 assigned | Implement initial Postgres schema and migration runner, implement sync projections for runtime-critical entities.
- 2026-04-03 | Worker 3 / Manager | Week 2 complete | Postgres schema (13 migrations), PgAdapter, PgMigrationRunner, PgEventLog, PgSyncProjection. Feature-gated behind `postgres`. 12 tests passing.
- 2026-04-03 | Week 3 assigned | Implement replay/rebuild support for projections, add SQLite local-mode support.
- 2026-04-03 | Worker 3 / Manager | Week 3 complete | `ProjectionRebuilder`, SQLite backend, V014 FTS migration. `sqlite` feature flag. 12 tests passing.
- 2026-04-03 | Week 4 assigned | Stabilize migrations and projection correctness, document backfill assumptions.
- 2026-04-03 | Worker 3 / Manager | Week 4 complete | Full lifecycle integration test, migration validation, expired lease detection. 17 tests passing.
- 2026-04-03 | Wave 3 gate support | FTS5 virtual table added to SQLite schema for local-mode retrieval.
- 2026-04-03 | Cross-backend parity | 7 integration tests: event positions, stream ordering, cursor replay, head positions, deterministic list ordering, run ordering, and task/approval projection replay. InMemoryStore list queries now sort by (created_at, id). title/description added to TaskRecord/ApprovalRecord (V015). BACKFILL_ASSUMPTIONS.md published. 19+7=26 tests passing.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 3 | Clarification: re-do the real store task, not a generic parity sweep. Target checkpoint ordering parity first. Acceptable completion here is a real `cairn-store` code diff plus `cargo test -p cairn-store --test cross_backend_parity`, or an explicit blocker tied to the exact read-model/query seam. Do not finish with generic notes like `verified`, `no drift`, or `all tests green`.
- 2026-04-03 | Manager -> Worker 3 | Immediate pickup now: 1. fix checkpoint ordering parity so `CheckpointReadModel::list_by_run` returns the same deterministic order across InMemory, SQLite, and Postgres, 2. rerun `cargo test -p cairn-store --test cross_backend_parity` and keep the exact next failing surface in the mailbox if another ordering mismatch appears, 3. once that is green, fold the ordering rule into the smallest durable guard so it cannot drift again.
- 2026-04-03 | Worker 1 / Manager -> Worker 3 | Focused parity validation for a Worker 2 envelope cleanup surfaced a store-owned ordering seam: `cargo test -p cairn-store --test cross_backend_parity --features sqlite` still fails `sqlite_parity::approval_list_ordering_is_deterministic` at `tests/cross_backend_parity.rs:526`, where SQLite returns approval ids as `ap_charlie, ap_alpha, ap_bravo` instead of the expected deterministic order `ap_alpha, ap_bravo, ap_charlie`. This looks like a Worker 3 parity/ordering issue, not a Worker 2 shared-contract problem.
- 2026-04-03 | Manager -> Worker 3 | Immediate pickup now: 1. add one parity test that rebuilds from the event log after tool/external-worker events and proves `task`, `approval`, and `tool_invocation` current-state rows match between InMemory and SQLite, 2. add one deterministic list-ordering test for the exact read surface Worker 8 is most likely to consume next, 3. if both pass, update `BACKFILL_ASSUMPTIONS.md` with the assumptions API/SSE consumers must not violate.
- 2026-04-03 | Manager -> Worker 3 | Follow-on handwritten direction after that: 1. add the smallest extra read-model coverage Worker 4/8 asks for, 2. keep replay/rebuild ordering boring across backends, 3. do not widen store behavior beyond parity/backfill work.
- 2026-04-03 | Manager -> Worker 3 | Immediate handwritten direction after the first fix: 1. add one parity test proving replay/rebuild produces the same current-state rows for `task`, `approval`, and `tool_invocation` across InMemory and SQLite, 2. add one deterministic ordering/query test for the read-model surface Worker 8 is most likely to consume, 3. if both are green, write down any backfill/migration assumption that API/SSE consumers must not violate.
- 2026-04-03 | Manager -> Worker 3 | Ongoing handwritten direction: 1. extend cross-backend parity around replay/rebuild ordering and deterministic query ordering, 2. add the smallest additional read-model coverage Worker 4/8 needs for richer API/SSE surfaces, 3. if idle after that, tighten migration/backfill assumptions instead of widening store behavior.
- 2026-04-03 | Manager -> Worker 3 | Current next focus: keep store parity boring. Guard replay/rebuild ordering across backends, support any read-model seam Worker 8 needs for richer SSE/API surfaces, and avoid inventing backend-specific behavior.
- 2026-04-03 | Architecture Owner -> Worker 3 | Week 1 focus: storage crate scaffold and DB/projection interfaces aligned with Worker 2 domain contracts.
- 2026-04-03 | Worker 1 -> Worker 3 | Priority order: migrations layout, event-log interfaces, sync projection boundaries, DB adapter seams. Worker 4/6/8 need these early.
- 2026-04-03 | Worker 2 -> Worker 3 | `cairn-domain` now exposes stable ownership keys, lifecycle enums, and event/command envelopes. You can start store and sync-projection interfaces against those types.
- 2026-04-03 | Worker 1 / Manager -> Worker 3 | Current next focus: take the Week 4 store hardening pass. Stabilize projection rebuild correctness across Postgres/SQLite, document any backfill assumptions, and tighten migration-check tooling without widening the store surface.
- 2026-04-03 | Worker 1 / Manager -> Worker 3 | Concrete next cut: make replay and rebuild boring. Tighten cross-backend parity around projection rebuilds, event replay ordering, and migration validation so Worker 4/6/8 can depend on the store without backend-specific conditionals or fixture drift.

## Outbox

- 2026-04-03 | Worker 3 -> Worker 4 | `cairn-store` now exposes: `EventLog` trait (append/read/replay), sync projection traits, and per-entity read-model traits (`SessionReadModel`, `RunReadModel`, `TaskReadModel`, `ApprovalReadModel`, `CheckpointReadModel`, `MailboxReadModel`, `ToolInvocationReadModel`). You can code runtime service boundaries against these interfaces.
- 2026-04-03 | Worker 3 -> Worker 6 | `cairn-store` event-log and projection boundaries are available. Graph persistence should treat store interfaces as the write-side contract for durable state.
- 2026-04-03 | Worker 3 -> Worker 8 | Per-entity read-model traits are in `cairn-store::projections`. API read endpoints can depend on `SessionReadModel`, `RunReadModel`, `TaskReadModel`, etc. for query interfaces.
- 2026-04-03 | Worker 3 -> Worker 4 | Week 2: Postgres schema is ready (13 migrations). `PgEventLog` implements append/read with cursor-based replay. `PgSyncProjection::apply_async` handles all 20 RuntimeEvent variants within a transaction. Runtime can persist end-to-end through the store.
- 2026-04-03 | Worker 3 -> Worker 6 | Week 2: Document (V010) and chunk (V011) tables are in the migration set. Graph node (V012) and edge (V013) tables are ready. `cairn-memory` and `cairn-graph` can use `postgres` feature flag to access persistence.

## Ready For Review

- 2026-04-03 | Worker 3 | Review `crates/cairn-store/*` for Week 1 store scaffold: event-log trait, sync projections, DB adapter seam, migration runner, and all entity read-model traits.
- 2026-04-03 | Worker 3 | Review `crates/cairn-store/src/pg/*` and `crates/cairn-store/migrations/V001-V013` for Week 2 Postgres implementation: adapter, migration runner, event log, sync projections. 12 tests passing.
