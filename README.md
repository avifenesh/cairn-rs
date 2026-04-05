# Cairn

**Self-hostable control plane for production AI agents.**

Cairn gives your team the infrastructure layer that sits between your LLM calls and your users: event-sourced task orchestration, multi-provider routing, operator approval workflows, real-time streaming, and full cost accountability — all in a single self-hosted binary written in Rust.

---

## Why Cairn

Running AI agents in production means more than calling an LLM. You need:

- **Audit trails** — every decision, tool call, and approval in an immutable event log
- **Human-in-the-loop** — approval gates that block task progression until an operator acts
- **Multi-provider resilience** — fallback chains across OpenAI, Anthropic, Bedrock, and OpenRouter
- **Cost visibility** — per-run token counts and USD micros tracked to the call level
- **Real-time observability** — operators see live run state via SSE without polling
- **Replay durability** — reconnecting clients pick up missed events from a durable log

Cairn is the control plane that handles all of this. Your agents focus on product logic; Cairn handles the infrastructure contract.

---

## Quick start

### Cargo (local dev — zero config)

```bash
git clone https://github.com/your-org/cairn-rs
cd cairn-rs
cargo run -p cairn-app
```

The server starts on `http://127.0.0.1:3000`. Data is in-memory and resets on restart. The default bearer token is `dev-admin-token`.

```bash
# Health check (no auth required)
curl http://localhost:3000/health
# → {"ok":true}

# Operator status
curl -H "Authorization: Bearer dev-admin-token" \
     http://localhost:3000/v1/status
# → {"runtime_ok":true,"store_ok":true,"uptime_secs":4}
```

### Docker Compose (recommended)

```bash
docker compose up --build
```

This starts Cairn with a Postgres backend on port 3000. Schema migrations run automatically on first boot.

### Postgres persistence

```bash
export CAIRN_ADMIN_TOKEN="$(openssl rand -hex 32)"

cargo run -p cairn-app -- \
  --mode team \
  --db postgres://cairn:cairn@localhost/cairn \
  --addr 0.0.0.0 \
  --port 3000
```

On first start, Cairn applies all schema migrations and confirms readiness at `GET /v1/db/status`.

---

## Features

### Event-sourced runtime (RFC 002)
Every run, task, approval, and provider call is appended to an immutable, append-only log with monotonically increasing positions. Commands carry a `causation_id` so retries are idempotent — re-delivering the same command returns the original position without re-appending. Operators can replay any window of the log via `GET /v1/events?after=<position>`.

### Multi-provider routing (RFC 009)
Provider bindings are ranked by priority. The resolver walks the fallback chain, checks capability requirements (streaming, structured output, tool use), and records a durable `RouteDecisionRecord` with `fallback_used: true` when the primary was unavailable. Cost is tracked per binding in USD micros at call granularity.

### Approval workflows (RFC 005)
Runs and tasks can be gated on operator approval before proceeding. Pending approvals surface in `GET /v1/approvals/pending`. A `POST /v1/approvals/:id/resolve` with `"approved"` or `"rejected"` unblocks the run atomically. The approval record version-increments on each decision for optimistic-concurrency safety.

### Real-time SSE stream (RFC 002)
`GET /v1/stream` delivers live events to operators and frontends without polling. On connect, a `connected` event carries the current head position. Reconnecting clients send `Last-Event-ID` and receive a replay of up to 1 000 missed events before rejoining the live feed. Auth-exempt so browser `EventSource` works without custom headers.

### Cost tracking (RFC 009)
Every `ProviderCallCompleted` event accumulates token counts and USD micros at the run level. `GET /v1/costs` returns the aggregate across all runs. Per-binding cost stats support provider comparison dashboards. Zero-cost and `None`-cost calls count toward call totals but never inflate cost figures — the arithmetic is pure integer (no floating-point loss).

### Knowledge retrieval (RFC 003)
Documents are ingested through a chunking, scoring, and deduplication pipeline. Retrieval scores results across lexical relevance, semantic relevance, freshness decay, staleness penalty, source credibility, corroboration, and graph proximity. The diagnostics surface exposes per-source quality metrics and index status for operator review.

### Eval and baselines (RFC 004, RFC 013)
Prompt releases are evaluated against rubrics and scored across `task_success_rate`, `latency_p50_ms`, and `cost_per_run`. Baselines lock the best-known metrics. Regressions beyond a 5 % tolerance are automatically flagged. Multi-armed bandit experimentation (EpsilonGreedy and UCB1) steers live traffic toward the best-performing release.

### Commercial feature gating (RFC 014)
Features are classified as `GeneralAvailability`, `Preview`, or `EntitlementGated`. Unknown feature names always return `Denied` (fail-closed — an unrecognized name is never silently allowed). The three product tiers — `LocalEval`, `TeamSelfHosted`, `EnterpriseSelfHosted` — control which gated capabilities are accessible.

### Checkpoint and recovery (RFC 004)
Agents record checkpoints at safe replay points. The recovery pipeline detects expired leases and re-queues tasks. `RecoveryAttempted` and `RecoveryCompleted` events form a complete audit trail. The latest checkpoint per run is surfaced for operator inspection and automated recovery targeting.

---

## Architecture

Cairn is a **12-crate Rust workspace**. Each crate owns a single bounded context with no circular dependencies.

```
cairn-domain      — pure domain types, events, lifecycle rules, RFC contracts
cairn-store       — append-only event log + synchronous projections (InMemory + Postgres)
cairn-runtime     — service implementations: runs, tasks, sessions, approvals, routing
cairn-api         — HTTP types, SSE payloads, auth, bootstrap config, API error shapes
cairn-app         — executable: axum HTTP server, startup wiring, all route handlers
cairn-memory      — knowledge pipeline: ingest, chunking, retrieval, diagnostics
cairn-graph       — entity relationship graph: nodes, edges, proximity scoring
cairn-evals       — eval runs, scoring rubrics, baselines, bandit experiment matrices
cairn-tools       — tool invocation contracts, plugin capability verification
cairn-signal      — signal ingestion and routing between agents
cairn-channels    — async message channels between agents
cairn-plugin-proto — plugin protocol types and capability declarations
```

### Event log and projections

Cairn's entire state is derived from the event log. Every mutation is an append; every read is a projection query. The same `apply_projection` function that populates in-memory read models also drives the Postgres synchronous projection applier — there is no dual-implementation drift.

```
append(events)
  ├──► apply_projection ──► current-state read models
  ├──► persist to Postgres (when --db postgres://... is set)
  └──► broadcast channel  ──► live SSE stream
```

When Postgres is configured, cairn-app dual-writes: each append goes to both Postgres (durability) and the in-memory store (read models, SSE broadcast). The Postgres event log serves cursor-based replay for `GET /v1/events`.

### Durability classes (RFC 002)

| Entity | Class | Reason |
|--------|-------|--------|
| Session, Run, Task | `FullHistory` | Core state machines require full replay |
| Approval, Checkpoint | `CurrentStatePlusAudit` | Current state + audit trail is sufficient |
| All other entities | `CurrentStatePlusAudit` | Operational visibility only |

---

## Test suite

The test suite is the executable specification of the RFC contracts.

```
796 lib tests         — 0 failures  (cargo test --workspace --exclude cairn-app --lib)
~230 integration tests — 0 failures  (store, runtime, memory, evals, api, domain)
```

```bash
# Full suite (excludes cairn-app lib due to pre-existing in-progress handlers)
cargo test --workspace --exclude cairn-app

# Workspace build
cargo build --workspace
```

The integration tests include an explicit RFC compliance summary (`crates/cairn-store/tests/rfc_compliance_summary.rs`) with one test per RFC verifying the core MUST requirement against the real store backend.

---

## API reference

Full endpoint documentation: **[docs/api-reference.md](./docs/api-reference.md)**

Includes: method, path, auth requirements, query params, request/response shapes, curl examples, error codes, and server configuration reference.

### Route summary

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `GET` | `/health` | No | Liveness probe |
| `GET` | `/v1/stream` | No | Real-time SSE event stream |
| `GET` | `/v1/status` | Yes | Runtime + store health |
| `GET` | `/v1/dashboard` | Yes | Operator overview (runs, tasks, costs) |
| `GET` | `/v1/runs` | Yes | List runs (paginated) |
| `GET` | `/v1/runs/:id` | Yes | Get run by ID |
| `GET` | `/v1/sessions` | Yes | List active sessions |
| `GET` | `/v1/approvals/pending` | Yes | List pending approvals |
| `POST` | `/v1/approvals/:id/resolve` | Yes | Approve or reject |
| `GET` | `/v1/prompts/assets` | Yes | List prompt assets |
| `GET` | `/v1/prompts/releases` | Yes | List prompt releases |
| `GET` | `/v1/costs` | Yes | Aggregate cost summary |
| `GET` | `/v1/providers` | Yes | List provider bindings |
| `GET` | `/v1/events` | Yes | Replay event log (cursor-based) |
| `POST` | `/v1/events/append` | Yes | Append events (idempotent) |
| `GET` | `/v1/db/status` | Yes | Postgres health + migration state |

---

## Deployment

Full guide: **[docs/deployment.md](./docs/deployment.md)**

Covers Docker Compose, Postgres setup, environment variables, team vs. local mode, TLS configuration, and production hardening.

### CLI flags

```
cairn-app [OPTIONS]

  --mode  team       Bind to 0.0.0.0 (requires CAIRN_ADMIN_TOKEN)
  --port  <port>     Listen port (default: 3000)
  --addr  <addr>     Bind address (default: 127.0.0.1)
  --db    <url>      postgres://... or path/to/db.sqlite
                     Omit for in-memory (local dev only)
```

### Key environment variables

| Variable | Description |
|----------|-------------|
| `CAIRN_ADMIN_TOKEN` | Bearer token for the admin account. Required in team mode. Defaults to `dev-admin-token` in local mode. |

---

## RFCs

Cairn's behaviour is defined by RFCs in [`docs/design/rfcs/`](./docs/design/rfcs/README.md). Each RFC specifies a contract that the implementation must satisfy; the integration test suite provides the compliance proof.

| RFC | Scope | Status |
|-----|-------|--------|
| RFC 002 | Event-log durability, idempotency, SSE replay | ✓ |
| RFC 003 | Memory retrieval pipeline | ✓ |
| RFC 004 | Checkpoint, eval system, baselines | ✓ |
| RFC 005 | Approval blocking | ✓ |
| RFC 006 | Prompt release lifecycle | ✓ |
| RFC 007 | Provider connection health | ✓ |
| RFC 008 | Multi-tenant isolation | ✓ |
| RFC 009 | Provider routing and cost tracking | ✓ |
| RFC 013 | Bundle import/export, eval rubrics | ✓ |
| RFC 014 | Commercial tiers and feature gating | ✓ |

---

## Contributing

This repository uses a manager + worker coordination model. Active work is tracked in [`.coordination/`](./.coordination/). See [AGENTS.md](./AGENTS.md) for the coordination protocol.

---

## License

Proprietary. All rights reserved.
