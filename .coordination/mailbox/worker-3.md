# Worker 3 Mailbox

Owner: Store, Event Log, Synchronous Projections

## Current Status

- 2026-04-03 | Week 1 assigned | Scaffold `cairn-store`, migration layout, event-log interfaces, and synchronous projection boundaries.
- 2026-04-03 | Worker 3 / Manager | `cairn-store` scaffold complete | Event-log trait, sync projection traits, DB adapter seam, migration runner interface, and per-entity read-model traits (session, run, task, approval, checkpoint, mailbox, tool_invocation) are in repo with passing tests.
- 2026-04-03 | Week 2 assigned | Implement initial Postgres schema and migration runner, implement sync projections for runtime-critical entities.
- 2026-04-03 | Worker 3 / Manager | Week 2 complete | Postgres schema (13 migrations), PgAdapter, PgMigrationRunner, PgEventLog, PgSyncProjection. Feature-gated behind `postgres`. 12 tests passing.
- 2026-04-03 | Week 3 assigned | Implement replay/rebuild support for projections, add SQLite local-mode support.
- 2026-04-03 | Worker 3 / Manager | Week 3 complete | `ProjectionRebuilder`, SQLite backend, V014 FTS migration. `sqlite` feature flag. 12 tests passing.
- 2026-04-03 | Week 4 assigned | Stabilize migrations and projection correctness, document backfill assumptions.
- 2026-04-03 | Worker 3 / Manager | Week 4 complete | Full lifecycle integration test (13-event sequence covering session/run/task/approval/checkpoint/mailbox with lease, supersession, and recovery detection). Migration validation tests (ordering, non-empty SQL, DDL presence for all 14 migrations). Expired lease detection test for recovery sweeps. 17 tests passing.

## Blocked By

- none

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 3 | Week 1 focus: storage crate scaffold and DB/projection interfaces aligned with Worker 2 domain contracts.
- 2026-04-03 | Worker 1 -> Worker 3 | Priority order: migrations layout, event-log interfaces, sync projection boundaries, DB adapter seams. Worker 4/6/8 need these early.
- 2026-04-03 | Worker 2 -> Worker 3 | `cairn-domain` now exposes stable ownership keys, lifecycle enums, and event/command envelopes. You can start store and sync-projection interfaces against those types.
- 2026-04-03 | Worker 1 / Manager -> Worker 3 | Current next focus: take the Week 4 store hardening pass. Stabilize projection rebuild correctness across Postgres/SQLite, document any backfill assumptions, and tighten migration-check tooling without widening the store surface.

## Outbox

- 2026-04-03 | Worker 3 -> Worker 4 | `cairn-store` now exposes: `EventLog` trait (append/read/replay), sync projection traits, and per-entity read-model traits (`SessionReadModel`, `RunReadModel`, `TaskReadModel`, `ApprovalReadModel`, `CheckpointReadModel`, `MailboxReadModel`, `ToolInvocationReadModel`). You can code runtime service boundaries against these interfaces.
- 2026-04-03 | Worker 3 -> Worker 6 | `cairn-store` event-log and projection boundaries are available. Graph persistence should treat store interfaces as the write-side contract for durable state.
- 2026-04-03 | Worker 3 -> Worker 8 | Per-entity read-model traits are in `cairn-store::projections`. API read endpoints can depend on `SessionReadModel`, `RunReadModel`, `TaskReadModel`, etc. for query interfaces.
- 2026-04-03 | Worker 3 -> Worker 4 | Week 2: Postgres schema is ready (13 migrations). `PgEventLog` implements append/read with cursor-based replay. `PgSyncProjection::apply_async` handles all 20 RuntimeEvent variants within a transaction. Runtime can persist end-to-end through the store.
- 2026-04-03 | Worker 3 -> Worker 6 | Week 2: Document (V010) and chunk (V011) tables are in the migration set. Graph node (V012) and edge (V013) tables are ready. `cairn-memory` and `cairn-graph` can use `postgres` feature flag to access persistence.

## Ready For Review

- 2026-04-03 | Worker 3 | Review `crates/cairn-store/*` for Week 1 store scaffold: event-log trait, sync projections, DB adapter seam, migration runner, and all entity read-model traits.
- 2026-04-03 | Worker 3 | Review `crates/cairn-store/src/pg/*` and `crates/cairn-store/migrations/V001-V013` for Week 2 Postgres implementation: adapter, migration runner, event log, sync projections. 12 tests passing.
