# Durable Recovery and Readiness (RFC 020)

Operator-facing guide to how cairn-app survives crashes, how it signals readiness
during startup, and what's live today versus coming in a future release. Summarises
[RFC 020](../design/rfcs/020-durable-recovery.md); the RFC is the source of truth.

## Summary

cairn-app is designed to survive crashes. The event log in the canonical store
(Postgres for team mode, SQLite for dev/edge) is the source of truth — every
state change is an append-only event, and on restart cairn rebuilds its
projections and in-memory state from the log. The process exposes two health
endpoints so orchestrators can distinguish "alive" from "accepting traffic"
during recovery.

As of RFC 020 rev 3 (see
[RFC 020](../design/rfcs/020-durable-recovery.md)), Tracks 1–4 are all live:
the readiness gate (`GET /health/ready` with a progress-shaped body),
run-level recovery via `RecoveryService`, tool-call idempotency with
deterministic `ToolCallId`s and a consulted `ToolCallResultCache`, and dual
checkpoint (`Intent` / `Result`) per orchestrator iteration. Decision cache
entries now survive restart. Sandbox recovery emits
`SandboxLost` / `SandboxAllowlistRevoked` / `SandboxBaseRevisionDrift`
for the preserved scenarios. See [Live today](#live-today) for the mapping
from behaviour to shipping PR.

## Health endpoints

cairn-app exposes a Kubernetes-style split between liveness and readiness.

### `GET /health` — liveness

Returns `200 OK` as soon as the process is running, even mid-recovery. Use this
for process-level checks: systemd `ExecStartPost`, Kubernetes `livenessProbe`,
"is the binary up" scripts. Do not use it to decide whether to route user traffic.

```bash
curl -sf http://localhost:3000/health
# → {"status":"healthy","version":"...","uptime_secs":42,"store_ok":true,...}
```

Response codes:

| Code | Meaning | Operator action |
|---|---|---|
| 200 | Process is healthy or degraded but responsive. | None. |
| 503 | Store is unreachable from the running process. | Check store availability, inspect logs; the process itself is alive but cannot serve work. |

### `GET /health/ready` — readiness

Returns `200 OK` once cairn-app has finished startup and is ready to accept
application traffic. Returns `503 Service Unavailable` with a progress JSON body
while recovery is still in flight. Use this for load-balancer health checks,
Kubernetes `readinessProbe`, or any gate that decides whether to send real
requests.

```bash
curl -s http://localhost:3000/health/ready | jq
```

Progress JSON shape (from `StartupProgress` /`StartupBranches` in
`crates/cairn-runtime/src/startup.rs`, aligned with [RFC 020 §Startup order](../design/rfcs/020-durable-recovery.md)):

```json
{
  "status": "ready",
  "step": "6",
  "branches": {
    "event_log":         { "state": "complete", "count": 15234 },
    "tool_result_cache": { "state": "complete", "count": 42 },
    "decision_cache":    { "state": "complete", "count": 87 },
    "memory":            { "state": "complete", "count": 3401 },
    "graph":             { "state": "complete", "count": 892 },
    "evals":             { "state": "complete", "count": 14 },
    "repo_store":        { "state": "complete", "count": 3 },
    "plugin_host":       { "state": "complete", "count": 1 },
    "providers":         { "state": "complete", "count": 2 },
    "sandboxes":         { "state": "complete", "count": 4 },
    "webhook_dedup":     { "state": "complete", "count": 156 },
    "triggers":          { "state": "complete", "count": 5 },
    "runs":              { "state": "complete", "count": 7 }
  },
  "started_at": 1775759896876,
  "elapsed_ms": 2340
}
```

Each branch reports `"pending"` → `"in_progress"` → `"complete"` (or
`"failed"` with a `detail` string) as it advances. `count` populates when the
branch finishes and records a branch-specific quantity (events replayed, chunks
indexed, sandboxes reattached, etc.). Readiness flips to `200` only when every
branch reports `complete`.

Consumers parsing this endpoint should tolerate unknown branches (future
recovery steps may add new ones) and treat any non-`complete` state as
not-ready.

Response codes:

| Code | Meaning | Operator action |
|---|---|---|
| 200 | Ready for traffic. | Route requests normally. |
| 503 | Recovery in progress. Body shows which branches are still pending. | Wait; check logs if recovery stays pending for minutes. |

### Using both endpoints together

A typical Kubernetes pod spec points `livenessProbe` at `/health` and
`readinessProbe` at `/health/ready`. The pod stays in the service only while
readiness is 200; the kubelet restarts the pod only if liveness fails. This
means cairn-app can spend minutes replaying a large event log without being
killed, and traffic doesn't reach it until replay finishes.

## Startup sequence

At a high level, cairn-app startup proceeds through the following phases:

1. Bind the HTTP listener. `/health` (liveness) starts responding immediately.
   `/health/ready` (readiness) returns 503 until step 6.
2. Replay the event log into projections (run, task, approval, session, mailbox,
   decision cache, tool-result cache, memory, graph, eval scorecards,
   webhook-delivery dedup).
3. Parallel recovery branches (run concurrently where independent):
   repo-clone cache, plugin host descriptor revalidation, provider pool warmup.
4. Sandbox reconciliation — `SandboxService::recover_all` reattaches or prunes
   sandboxes against the now-known run state.
5. Run-level recovery — `RecoveryService` enumerates non-terminal runs and
   applies the RFC 020 recovery matrix, emitting `RecoveryAttempted` /
   `RecoveryCompleted` per run.
6. Emit `RecoverySummary` event (one per boot, with aggregated counts);
   flip `/health/ready` to 200; open non-health routes.

Expected timing: sub-second for small deployments; seconds-to-minutes for large
event logs. cairn-app logs a progress line every 5 seconds while recovery is in
flight so an operator tailing `journalctl -u cairn -f` can watch progress.

For the complete dependency graph (which steps are parallel, which barriers
separate them, and why), see
[RFC 020 §Startup order](../design/rfcs/020-durable-recovery.md).

## Store requirements

cairn-app supports three storage backends, with different durability and
deployment characteristics. See also [postgres-team-mode.md](../postgres-team-mode.md)
for team-mode setup.

| Backend | Team mode | Dev/local | Durability on restart |
|---|---|---|---|
| Postgres | Supported (required) | Supported | Full — event log survives, projections rebuild. |
| SQLite | Not supported — cairn-app refuses to start. | Supported | Full on the single node; no replication or multi-writer. |
| In-memory (`--db memory`) | Not supported. | Dev only. | None — all data lost on restart. |

### Why Postgres is required for team mode

Team-mode deployments assume durability, concurrent writes, and standard
operational tooling (backup, PITR, replication). SQLite cannot support the
team-mode story in production: its WAL is single-writer, its durability
configuration is fsync-dependent, and it has no replication primitive. cairn-app
refuses to start in `--mode team` with a SQLite DSN; this is intentional, not a
warning — operators must provide Postgres.

### SQLite schema parity caveat

cairn-store ships a schema parity test that enumerates `CREATE TABLE` statements
from Postgres migrations and SQLite schema and asserts the table sets match. Run
it manually:

```bash
cargo test -p cairn-store --test schema_parity -- --ignored
```

As of this writing the test is marked `#[ignore]` because the shipped schemas
are drifted: several tables (routing policies, workspace members, tenants,
projects, and related RBAC surface) exist only in Postgres migrations. SQLite is
fine for single-operator development and for edge deployments that don't use
the features those tables back, but **SQLite is not a supported production
backend in v1**. If you are running cairn-app against SQLite, expect some
team-mode features (routing, multi-tenant RBAC) to be unavailable.

When this gap closes, the `#[ignore]` attribute will be removed and schema
parity will become a fail-on-merge gate.

### Portability posture

cairn-store uses portable SQL in service code and keeps backend-specific DDL in
per-backend migration files. Cairn avoids Postgres-only features (advisory
locks, `LISTEN/NOTIFY`, JSONB operators, array columns) in the query path so
the storage surface stays compatible with other SQL databases if the project
later adopts additional backends. Postgres is the v1 production target, not a
hard lock-in.

## What survives a crash today

The following state is durable across a SIGKILL and restart in the current
build:

- **Event log.** Every appended event is durable in the store's WAL before the
  append returns. On restart, projections are rebuilt from the log.
- **Sync projections** (runs, tasks, approvals, sessions, mailboxes) are written
  in the same transaction as the event append, so the projection and the log
  advance together.
- **Sandbox filesystems.** `SandboxService::recover_all` runs on startup,
  reattaches sandboxes against their base revisions, preserves sandboxes that
  drifted, and prunes orphans (see [RFC 016](../design/rfcs/016-sandbox-workspace-primitive.md)).
- **FF lease-history cursors.** The bridge subscriber records its stream
  position in a dedicated projection so it resumes from the correct offset
  across restarts.
- **Pending approvals.** A run in `WaitingApproval` at crash time is still in
  `WaitingApproval` after restart, because that state is derived from events in
  the log.
- **Decision cache entries.** The decision cache is a projection over
  `DecisionEvent`s, so cached decisions from before the crash are available
  after recovery with no re-approval required.
- **Engine-side operational state** (leases, execution deadlines, flow edges)
  is owned by FlowFabric's background scanners — not by cairn-app's startup
  path. If Valkey is lost, FF's scanners rebuild the operational state; cairn's
  durability guarantees do not depend on engine-backing-store survival.

### Tool-call side-effect contract

Side-effecting tools (shell commands, HTTP with side effects, git merges) are
now classified by `RetrySafety`:

- `IdempotentSafe` — safe to re-dispatch on resume; the cache may also serve a
  hit for free.
- `AuthorResponsible` — re-dispatched with the same `ToolCallId`; the tool's
  implementation is responsible for its own idempotency.
- `DangerousPause` — the orchestrator pauses on resume and waits for operator
  confirmation before re-dispatch. This replaces the pre-Track-3 risk of
  silent double-execution.

## Live today

Every item below corresponds to a shipped PR. Source of truth for the full
contract is [RFC 020](../design/rfcs/020-durable-recovery.md) rev 3.

- **Track 1 — RecoveryService (run-level recovery).** Startup pass enumerates
  non-terminal runs, applies the RFC 020 recovery matrix, and emits
  `RecoveryAttempted` / `RecoveryCompleted` events before readiness flips to
  200. (#75)
- **Track 2 — Readiness gate.** `GET /health/ready` returns the progress
  JSON shape above, flipping to 200 only when every branch is complete. (#73)
- **Track 3 — Tool-call idempotency.** Deterministic `ToolCallId`,
  `ToolCallResultCache` projection consulted on every dispatch, `RetrySafety`
  three-tier enforcement, and atomic batched append of
  `ToolInvocationRequested` + `ToolInvocationCompleted`. (#82)
- **Track 4 — Dual checkpoint per iteration.** `Intent` checkpoint before
  tool dispatch and `Result` after; `RecoverySummary` emitted once per boot
  with recovered-run / recovered-task / preserved-sandbox / warmed-decision
  counters. (#84)
- **Decision cache durability.** Cached decisions survive restart without
  re-approval via `DecisionEvent` projection replay. (#85)
- **Sandbox recovery — preserved scenarios** emit distinct events so
  operators can tell preservation cases apart from lost sandboxes:
  `SandboxLost` (#83), `SandboxAllowlistRevoked` (#86), and
  `SandboxBaseRevisionDrift` (#89).
- **Schema parity check (CI).** `cargo test -p cairn-store --test schema_parity`
  audits that Postgres and SQLite declare the same tables. Ignored today
  because 10 Postgres-only tables are known drifts; becomes a fail-on-merge
  gate when the gap closes. (#76)

See RFC 020 rev 3 for the `RecoveryEvent` enum, the 12 durability
invariants (all closed as of this session), and compliance tests 1–15.

## Operator playbook

### "cairn-app is stuck returning 503 on /health/ready"

cairn-app is still in recovery. Inspect the progress JSON:

```bash
curl -s http://localhost:3000/health/ready | jq '.branches'
```

The body names each branch and its state. Recovery logs a progress line every
5 seconds; check `journalctl -u cairn -f` (systemd) or `docker compose logs -f
cairn` (Docker). If a specific branch is pending for minutes:

- `event_log` branch stuck: Postgres is unreachable or slow. Check
  connectivity and WAL size.
- `sandboxes` branch stuck: a sandbox reattach is hanging on a filesystem
  operation. Check `docker ps`, underlying mounts, disk availability.
- `providers` branch stuck: a configured LLM provider is unreachable at warmup.
  Providers should fail fast; if one hangs, check the provider endpoint.

Liveness (`/health`) stays 200 throughout — do not restart the process just
because readiness is 503; restart only if liveness fails or the process is
hung.

### "cairn-app refuses to start in team mode"

Team mode requires Postgres. If you started cairn-app with `--mode team --db /path/to/file.db`,
it exits with a clear error directing you to switch to a Postgres DSN:

```
cairn-app \
  --mode team \
  --db postgres://cairn:password@host:5432/cairn \
  --tls-cert /etc/cairn/tls/cert.pem \
  --tls-key  /etc/cairn/tls/key.pem
```

See [postgres-team-mode.md](../postgres-team-mode.md) for a full team-mode
setup walkthrough.

### "schema drift warning when running the schema-parity test"

If you run `cargo test -p cairn-store --test schema_parity -- --ignored` and
see missing tables in SQLite, this is expected today: routing policies,
workspace members, tenants, projects, and related RBAC tables are Postgres-only
in v1. The test names every missing table so you can check whether your
deployment actually uses those features. For local dev on SQLite, ignore the
drift; for production, use Postgres.

### "How do I safely restart cairn-app?"

`SIGTERM` triggers a graceful shutdown: in-flight HTTP requests drain, open
SSE streams close cleanly, and the process exits. `SIGKILL` is also safe from a
durability standpoint — the event log is durable before every append returns,
and on restart cairn replays the log to rebuild state. The only difference is
that SIGTERM is cleaner for connected clients; SIGKILL is indistinguishable
from a crash from cairn's perspective.

During restart, `/health/ready` will return 503 with the progress body until
recovery completes. Load balancers should route away from the instance based on
readiness, not liveness, so traffic stops reaching it during the 503 window.

### "How do I tell whether cairn-app recovered cleanly?"

Cairn emits a single `RecoverySummary` event per boot with counts (recovered
runs, recovered tasks, preserved sandboxes, decision-cache entries warmed).
Additional signals:

- The `/health/ready` progress body transitioning from 503 with per-branch
  state to 200.
- The startup log banner printing its `boot_id` (a UUID v7 minted per process
  start).
- `SandboxService::recover_all` logs sandbox-level outcomes (reattached,
  preserved with a `SandboxAllowlistRevoked` / `SandboxBaseRevisionDrift`
  cause, orphaned as `SandboxLost`, or pruned).

## See also

- [RFC 020 — Durable Recovery and Tool-Call Idempotency](../design/rfcs/020-durable-recovery.md)
  — the source of truth.
- [RFC 011 — Deployment shape](../design/rfcs/011-deployment-shape.md) — why
  Postgres is the team-mode target.
- [RFC 016 — Sandbox workspace primitive](../design/rfcs/016-sandbox-workspace-primitive.md)
  — sandbox reconciliation on startup.
- [postgres-team-mode.md](../postgres-team-mode.md) — team-mode setup walkthrough.
- [deployment.md](../deployment.md) — Docker, systemd, TLS setup.
