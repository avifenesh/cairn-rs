# Store Backfill and Migration Assumptions

Status: living document  
Owner: Worker 3 (Store)  
Consumers: Worker 4 (Runtime), Worker 6 (Memory/Graph), Worker 8 (API/SSE)

## Purpose

Documents assumptions that API/SSE consumers must not violate when
reading from store projections. These constraints ensure that
projection rebuild, event replay, and cross-backend parity remain
boring and predictable.

## Nullable Fields

### TaskRecord

- `title`: nullable. Added in V015. Pre-V015 tasks have `NULL` title.
  SSE/API consumers must handle `None` gracefully (e.g. use task_id
  as fallback display text).
- `description`: nullable. Same as title.
- `failure_class`: nullable. Only set when task reaches a terminal
  failure state (Failed, DeadLettered). Never set for Completed.
- `lease_owner`, `lease_expires_at`: nullable. Only set after a
  TaskLeaseClaimed event. Cleared on terminal state but field values
  may persist in the record.

### ApprovalRecord

- `title`: nullable. Added in V015. Same handling as TaskRecord.
- `description`: nullable. Same handling.
- `decision`: nullable. NULL means pending (no decision yet).
  Non-null means resolved.

### ToolInvocationRecord (cairn-domain)

- `session_id`, `run_id`, `task_id`: all nullable. Tool invocations
  may be linked to any combination of these runtime entities.
- `outcome`: nullable. Only set on terminal states (Completed, Failed,
  Canceled). Must be non-null when `finished_at_ms` is set.
- `error_message`: nullable. Must be set when outcome is a failure
  kind, must be null when outcome is Success.
- `started_at_ms`, `finished_at_ms`: nullable. Started is set on
  the Started transition. Finished is set on terminal transitions.

## Ordering Guarantees

### Event Log

- `position` is monotonically increasing within a backend.
- `read_stream(after, limit)` returns events in position order.
- `event_id` is unique (enforced by UNIQUE constraint in both
  Postgres and SQLite schemas).

### Read Model Lists

All list queries return results sorted by `(created_at ASC, id ASC)`:

- `SessionReadModel::list_by_project` — by (created_at, session_id)
- `RunReadModel::list_by_session` — by (created_at, run_id)
- `TaskReadModel::list_by_state` — by (created_at, task_id)
- `ApprovalReadModel::list_pending` — by (created_at, approval_id)
- `ToolInvocationReadModel::list_by_run` — by (requested_at_ms, invocation_id)

This ordering is enforced in both InMemoryStore and SQL backends.
Consumers must not assume insertion order or HashMap iteration order.

## Projection Rebuild Rules

- `ProjectionRebuilder::rebuild_all()` truncates all projection
  tables and replays the full event log.
- `ProjectionRebuilder::rebuild_from(position)` replays from a
  cursor without truncation (incremental catchup).
- Rebuild is idempotent for create events (upsert semantics).
- Rebuild may produce different `updated_at` timestamps than the
  original write, since `updated_at` uses wall-clock time. Do not
  use `updated_at` as a stable ordering key across rebuilds.
- `version` is monotonically increasing per entity and is stable
  across rebuilds (it reflects event count, not wall-clock).
- List ordering is deterministic across rebuilds because it depends
  on `created_at` and entity ID, both of which are preserved in
  events. Tool invocations use `requested_at_ms` instead of
  `created_at` — this is also event-stable.
- Audit-only events (ExternalWorkerReported, SubagentSpawned,
  RecoveryAttempted, RecoveryCompleted) do not produce projection
  rows. They appear in the event stream but are silently skipped
  by the projection applier during rebuild. This is intentional.

## Cross-Backend Parity

- InMemoryStore and SQLite produce the same event positions (1-based,
  monotonic) for the same event sequence.
- Stream reads return events in the same order.
- Cursor-based replay produces the same tail.
- Read model list queries produce the same deterministic ordering.
- SQLite uses `json_extract()` for entity-filtered reads; Postgres
  uses `payload->>`. The filter behavior is equivalent but the
  JSON path syntax differs.

## Migration Assumptions

- Migrations are versioned V001-V015 (as of this writing).
- Migration runner applies migrations in version order within a
  single transaction.
- Migration history is tracked in `_cairn_migrations` table.
- `ALTER TABLE ADD COLUMN IF NOT EXISTS` is used for additive
  schema changes (V015). Consumers must handle NULL values for
  columns added after initial table creation.
- SQLite schema is applied as a single DDL block (not versioned
  migrations). The SQLite schema must be kept in sync with the
  cumulative effect of all Postgres migrations.

## Tool Invocation Read Model Contract

- `ToolInvocationReadModel::list_by_run` orders by
  `(requested_at_ms ASC, invocation_id ASC)`. This matches the
  SQL ORDER BY in both Postgres and SQLite adapters, and is
  enforced in InMemoryStore.
- Tool invocations may be in states: Requested, Started, Completed,
  Failed, Canceled. Only terminal states have `outcome` and
  `finished_at_ms` set.
- `target` is stored as JSON (JSONB in Postgres, TEXT in SQLite).
  Consumers should deserialize via `ToolInvocationTarget` enum.
- `execution_class` is stored as snake_case string. Consumers
  should deserialize via `ExecutionClass` enum.

## External Worker Events

- `ExternalWorkerReported` events are recorded in the event log
  for audit/replay but do NOT update projection tables.
- `SubagentSpawned` events are similarly audit-only in projections.
- Both event types are visible through `read_stream` and
  `read_by_entity` but will not appear in any read-model list query.
- Recovery events (`RecoveryAttempted`, `RecoveryCompleted`) follow
  the same audit-only pattern.

## Parent-Run Child-Task Query Semantics

`TaskReadModel::list_by_parent_run(parent_run_id, limit)` returns
child tasks spawned by a parent run, sorted by `(created_at ASC,
task_id ASC)`. Used by `RecoveryService::resolve_stale_dependencies()`
to check whether all child tasks of a `WaitingDependency` run have
completed.

`TaskReadModel::any_non_terminal_children(parent_run_id)` returns
`true` if any child task of the given parent run is in a non-terminal
state (`Queued`, `Leased`, `Running`, `WaitingApproval`, `Paused`,
`WaitingDependency`, `RetryableFailed`). Terminal states are
`Completed`, `Failed`, `Canceled`, `DeadLettered`.

Semantics are narrower than full dependency lineage:
- Only direct children (tasks with `parent_run_id` matching the run)
  are checked — not grandchildren or transitive dependencies.
- Subagent spawns create child tasks that link back via `parent_run_id`,
  so this query covers the primary subagent dependency path.
- If deeper lineage tracking is needed later, a separate graph-based
  query through `cairn-graph` should be used.

## API Consumer Ordering Guarantee

API and SSE consumers can rely on store read-model ordering directly
without re-sorting above the backend seam. Every list query returns
results in a deterministic order defined by the store contract:

- Sessions: `(created_at ASC, session_id ASC)`
- Runs: `(created_at ASC, run_id ASC)`
- Tasks: `(created_at ASC, task_id ASC)`
- Approvals: `(created_at ASC, approval_id ASC)`
- Checkpoints: `(created_at ASC, checkpoint_id ASC)`
- Mailbox messages: `(created_at ASC, message_id ASC)`
- Tool invocations: `(requested_at_ms ASC, invocation_id ASC)`
- Expired leases: `(lease_expires_at ASC, task_id ASC)`

This ordering is enforced identically in InMemoryStore, SQLite, and
Postgres backends. API consumers must not apply their own sorting
on top — doing so risks inconsistency if the sort keys differ.

Event stream reads (`read_stream`, `read_by_entity`) return events
in `position ASC` order. Cursors are stable across later appends —
a cursor obtained from batch N remains valid after batches N+1, N+2
are appended and returns exactly the events after the cursor position.

Single-value queries used by session state derivation:

- `latest_root_run(session_id)`: returns the most recently created
  root run (no `parent_run_id`). Used to derive session outcome.
- `any_non_terminal(session_id)`: returns true if any run in the
  session is non-terminal. Used to keep session Open.

Both are backend-agnostic and tested in cross-backend parity.

## What Consumers Must Not Do

- Assume `title` or `description` are always present on tasks/approvals.
- Assume `updated_at` is stable across projection rebuilds.
- Assume read model list order matches HashMap insertion order.
- Assume tool invocation `outcome` is set before terminal state.
- Write directly to projection tables without going through the
  event log (breaks rebuild invariant).
- Depend on specific `event_id` format (they're opaque strings).
- Assume external-worker or subagent events produce projection rows.
- Assume tool invocation `run_id` is always set (it's nullable).
