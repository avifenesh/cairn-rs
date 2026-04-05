# Cairn API Reference

This document describes every HTTP endpoint exposed by `cairn-app`.
All examples assume the server is running on `localhost:3000` (the default).

---

## Quick start

```bash
# Start the server (local mode — no CAIRN_ADMIN_TOKEN required)
cargo run -p cairn-app

# Default dev token in local mode
export CAIRN_TOKEN="dev-admin-token"

# Verify the server is up
curl http://localhost:3000/health
```

In **team mode** you must set `CAIRN_ADMIN_TOKEN` before starting:

```bash
export CAIRN_ADMIN_TOKEN="$(openssl rand -hex 32)"
cargo run -p cairn-app -- --mode team --addr 0.0.0.0
```

---

## Authentication

All `/v1/*` routes require a bearer token **except** `/v1/stream` (SSE clients
cannot set custom headers via the browser `EventSource` API).

```
Authorization: Bearer <token>
```

| Status | Meaning |
|--------|---------|
| `401 Unauthorized` | Token missing or invalid |
| `200 / 201` | Token accepted |

**Error body** (all error responses):

```json
{
  "code": "unauthorized",
  "message": "missing Authorization: Bearer <token> header"
}
```

---

## Base URL

```
http://<host>:<port>
```

Default: `http://127.0.0.1:3000`

---

## Pagination

List endpoints support `limit` (default `50`) and `offset` query parameters.

```
GET /v1/runs?limit=20&offset=40
```

---

## Endpoints

### `GET /health`

Liveness probe. **No auth required.** Used by load balancers and health checks.

**Response `200`**

```json
{ "ok": true }
```

```bash
curl http://localhost:3000/health
```

---

### `GET /v1/status`

Runtime and store health. Returns uptime and internal component status.

**Auth required:** yes

**Response `200`**

```json
{
  "runtime_ok": true,
  "store_ok": true,
  "uptime_secs": 142
}
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/status
```

---

### `GET /v1/dashboard`

Operator overview: active run/task counts, system health flag, and
observability metrics (latency percentiles, error rate, degraded components).

**Auth required:** yes

**Response `200`**

```json
{
  "active_runs": 3,
  "active_tasks": 12,
  "pending_approvals": 1,
  "failed_runs_24h": 0,
  "system_healthy": true,
  "latency_p50_ms": 142,
  "latency_p95_ms": 890,
  "error_rate_24h": 0.02,
  "degraded_components": [],
  "recent_critical_events": [],
  "active_providers": 2,
  "active_plugins": 0,
  "memory_doc_count": 0,
  "eval_runs_today": 4
}
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/dashboard
```

---

### `GET /v1/runs`

List all runs. Supports `limit` and `offset` pagination.

**Auth required:** yes

**Query params**

| Param | Default | Description |
|-------|---------|-------------|
| `limit` | `50` | Maximum number of runs to return |
| `offset` | `0` | Skip this many runs |

**Response `200`** — array of `RunRecord`

```json
[
  {
    "run_id": "run_abc123",
    "session_id": "sess_xyz",
    "project": {
      "tenant_id": "acme",
      "workspace_id": "engineering",
      "project_id": "agent-v2"
    },
    "state": "running",
    "created_at": 1712345600000,
    "updated_at": 1712345612000
  }
]
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     "http://localhost:3000/v1/runs?limit=10&offset=0"
```

---

### `GET /v1/runs/:id`

Get a single run by its ID.

**Auth required:** yes

**Path params**

| Param | Description |
|-------|-------------|
| `id` | The `run_id` string |

**Response `200`** — `RunRecord` (same shape as list item above)

**Response `404`**

```json
{ "code": "not_found", "message": "run xyz not found" }
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/runs/run_abc123
```

---

### `GET /v1/sessions`

List active sessions, most recent first. Supports `limit` and `offset`.

**Auth required:** yes

**Query params**

| Param | Default | Description |
|-------|---------|-------------|
| `limit` | `50` | Maximum sessions to return |
| `offset` | `0` | Skip this many sessions |

**Response `200`** — array of `SessionRecord`

```json
[
  {
    "session_id": "sess_xyz",
    "project": {
      "tenant_id": "acme",
      "workspace_id": "engineering",
      "project_id": "agent-v2"
    },
    "state": "active",
    "created_at": 1712345590000
  }
]
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/sessions
```

---

### `GET /v1/approvals/pending`

List pending (unresolved) approvals. Optionally scoped to a specific project.

**Auth required:** yes

**Query params**

| Param | Description |
|-------|-------------|
| `tenant_id` | Filter to this tenant (required together with workspace_id and project_id) |
| `workspace_id` | Filter to this workspace |
| `project_id` | Filter to this project |
| `limit` | Default `50` |
| `offset` | Default `0` |

If `tenant_id`, `workspace_id`, and `project_id` are all provided, only approvals
for that project are returned. Otherwise all pending approvals across all
projects in the store are returned.

**Response `200`** — array of `ApprovalRecord`

```json
[
  {
    "approval_id": "appr_001",
    "project": {
      "tenant_id": "acme",
      "workspace_id": "engineering",
      "project_id": "agent-v2"
    },
    "run_id": "run_abc123",
    "task_id": null,
    "requirement": "required",
    "decision": null,
    "title": "Approve GitHub write action",
    "description": "Agent wants to create a pull request.",
    "version": 1,
    "created_at": 1712345700000,
    "updated_at": 1712345700000
  }
]
```

```bash
# All pending approvals
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/approvals/pending

# Scoped to one project
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     "http://localhost:3000/v1/approvals/pending?tenant_id=acme&workspace_id=engineering&project_id=agent-v2"
```

---

### `POST /v1/approvals/:id/resolve`

Approve or reject a pending approval. Transitions the approval to a terminal
state and unblocks the waiting run (RFC 005).

**Auth required:** yes

**Path params**

| Param | Description |
|-------|-------------|
| `id` | The `approval_id` to resolve |

**Request body**

```json
{ "decision": "approved" }
```

| Field | Values |
|-------|--------|
| `decision` | `"approved"` or `"rejected"` |

**Response `200`** — updated `ApprovalRecord` with `decision` set

```json
{
  "approval_id": "appr_001",
  "decision": "approved",
  "version": 2,
  "updated_at": 1712345800000
}
```

**Response `400`** — unknown decision value

```json
{ "code": "bad_request", "message": "unknown decision: maybe; use 'approved' or 'rejected'" }
```

**Response `404`** — approval not found

```json
{ "code": "not_found", "message": "approval appr_001 not found" }
```

```bash
# Approve
curl -X POST \
     -H "Authorization: Bearer $CAIRN_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{"decision": "approved"}' \
     http://localhost:3000/v1/approvals/appr_001/resolve

# Reject
curl -X POST \
     -H "Authorization: Bearer $CAIRN_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{"decision": "rejected"}' \
     http://localhost:3000/v1/approvals/appr_001/resolve
```

---

### `GET /v1/prompts/assets`

List all prompt assets (RFC 006). Supports `limit` and `offset` pagination.

**Auth required:** yes

**Response `200`** — array of `PromptAssetRecord`

```json
[
  {
    "prompt_asset_id": "asset_abc",
    "name": "System Prompt",
    "kind": "system",
    "project": { "tenant_id": "acme", "workspace_id": "eng", "project_id": "agent" },
    "status": "published",
    "created_at": 1712340000000
  }
]
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/prompts/assets
```

---

### `GET /v1/prompts/releases`

List all prompt releases across all projects. Supports `limit` and `offset`.

**Auth required:** yes

**Response `200`** — array of `PromptReleaseRecord`

```json
[
  {
    "prompt_release_id": "rel_v3",
    "prompt_asset_id": "asset_abc",
    "prompt_version_id": "ver_007",
    "project": { "tenant_id": "acme", "workspace_id": "eng", "project_id": "agent" },
    "state": "active",
    "rollout_percent": null,
    "created_at": 1712341000000,
    "updated_at": 1712341500000
  }
]
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/prompts/releases
```

---

### `GET /v1/costs`

Aggregate cost summary across all runs in the store (RFC 009).
Totals all `RunCostUpdated` events appended to the log.

**Auth required:** yes

**Response `200`**

```json
{
  "total_provider_calls": 142,
  "total_tokens_in": 84300,
  "total_tokens_out": 31200,
  "total_cost_micros": 1250000
}
```

> Cost is in **USD micros** (1 USD = 1,000,000 micros).
> `total_cost_micros: 1250000` = $1.25.

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/costs
```

---

### `GET /v1/providers`

List all provider bindings registered in the store (RFC 007).
Supports `limit` and `offset` pagination.

**Auth required:** yes

**Response `200`** — array of `ProviderBindingRecord`

```json
[
  {
    "provider_binding_id": "bind_openai_gen",
    "project": { "tenant_id": "acme", "workspace_id": "eng", "project_id": "agent" },
    "provider_connection_id": "conn_openai",
    "provider_model_id": "gpt-4o",
    "operation_kind": "generate",
    "active": true,
    "created_at": 1712300000000
  }
]
```

```bash
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/providers
```

---

### `GET /v1/events`

Cursor-based replay of the global event log (RFC 002).
Returns event summaries (type + position) for operator inspection.

**Auth required:** yes

**Query params**

| Param | Default | Description |
|-------|---------|-------------|
| `after` | _(none)_ | Return only events with position strictly greater than this value |
| `limit` | `100` | Maximum events to return (hard cap: 500) |

**Response `200`** — array of event summaries

```json
[
  { "position": 1, "stored_at": 1712345600000, "event_type": "session_created" },
  { "position": 2, "stored_at": 1712345601000, "event_type": "run_created" },
  { "position": 3, "stored_at": 1712345602000, "event_type": "task_created" }
]
```

```bash
# All events from the beginning
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     http://localhost:3000/v1/events

# Page forward from position 50
curl -H "Authorization: Bearer $CAIRN_TOKEN" \
     "http://localhost:3000/v1/events?after=50&limit=100"
```

---

### `POST /v1/events/append`

Write events directly to the event log (RFC 002).
Implements the idempotency contract: events tagged with a `causation_id`
that already exists in the log are **not** re-appended — the original
position is returned instead.

**Auth required:** yes

**Request body** — JSON array of `EventEnvelope<RuntimeEvent>`

Each envelope must be a JSON object with the following fields:

| Field | Required | Description |
|-------|----------|-------------|
| `event_id` | yes | Stable string identifier for this event |
| `source` | yes | `{"source_type": "runtime"}` |
| `ownership` | yes | Project scope (see below) |
| `payload` | yes | The `RuntimeEvent` payload (internally tagged with `"event"`) |
| `causation_id` | no | If set, enables idempotency guard |
| `correlation_id` | no | Optional tracing correlation ID |

**Ownership object:**

```json
{
  "scope": "project",
  "tenant_id": "acme",
  "workspace_id": "engineering",
  "project_id": "agent-v2"
}
```

**Example: append a `SessionCreated` event**

```json
[
  {
    "event_id": "evt_001",
    "source": { "source_type": "runtime" },
    "ownership": {
      "scope": "project",
      "tenant_id": "acme",
      "workspace_id": "engineering",
      "project_id": "agent-v2"
    },
    "causation_id": "cmd_create_session_42",
    "correlation_id": null,
    "payload": {
      "event": "session_created",
      "project": {
        "tenant_id": "acme",
        "workspace_id": "engineering",
        "project_id": "agent-v2"
      },
      "session_id": "sess_42"
    }
  }
]
```

**Response `201`** — array of append results (same order as input)

```json
[
  {
    "event_id": "evt_001",
    "position": 7,
    "appended": true
  }
]
```

**Response `200`** — empty array (empty input batch)

| Field | Description |
|-------|-------------|
| `event_id` | Echoed from the envelope |
| `position` | Log position assigned (or existing position if idempotent) |
| `appended` | `true` = newly written; `false` = idempotent duplicate, original position returned |

**Idempotency example:**

```bash
# First call — appended=true, position=7
curl -X POST \
     -H "Authorization: Bearer $CAIRN_TOKEN" \
     -H "Content-Type: application/json" \
     -d '[{"event_id":"evt_001","source":{"source_type":"runtime"},"ownership":{"scope":"project","tenant_id":"acme","workspace_id":"eng","project_id":"p1"},"causation_id":"cmd_42","correlation_id":null,"payload":{"event":"session_created","project":{"tenant_id":"acme","workspace_id":"eng","project_id":"p1"},"session_id":"sess_1"}}]' \
     http://localhost:3000/v1/events/append

# Second call with same causation_id — appended=false, same position=7
curl -X POST \
     -H "Authorization: Bearer $CAIRN_TOKEN" \
     -H "Content-Type: application/json" \
     -d '[{"event_id":"evt_002","source":{"source_type":"runtime"},"ownership":{"scope":"project","tenant_id":"acme","workspace_id":"eng","project_id":"p1"},"causation_id":"cmd_42","correlation_id":null,"payload":{"event":"session_created","project":{"tenant_id":"acme","workspace_id":"eng","project_id":"p1"},"session_id":"sess_1_retry"}}]' \
     http://localhost:3000/v1/events/append
```

---

### `GET /v1/stream`

Real-time SSE event stream. **No auth required** (browsers cannot set
custom headers via `EventSource`). RFC 002 replay window is supported
via the standard `Last-Event-ID` header.

**Protocol:**

1. Server immediately emits `event: connected` with the current head position.
2. If `Last-Event-ID` is present, replays all stored events after that position
   (up to 1,000 events) before entering the live stream.
3. Live events are pushed as they are appended to the store.
4. Each SSE frame carries `id: <position>` so the browser can resume on
   reconnect using the native `EventSource` retry mechanism.
5. A keepalive comment (`: heartbeat`) is sent every 15 seconds.

**SSE wire format:**

```
event: connected
data: {"head_position":42}

id: 43
event: session_created
data: {"event_id":"evt_abc","type":"session_created","payload":{...}}

id: 44
event: run_created
data: {"event_id":"evt_def","type":"run_created","payload":{...}}

: heartbeat
```

**Browser example:**

```js
const es = new EventSource('http://localhost:3000/v1/stream');

es.addEventListener('connected', e => {
  console.log('head position:', JSON.parse(e.data).head_position);
});

es.addEventListener('session_created', e => {
  const payload = JSON.parse(e.data);
  console.log('new session:', payload.payload.session_id);
});

// Reconnect with cursor (native EventSource behaviour):
// the browser automatically sends Last-Event-ID on reconnect.
```

**curl example (print first 5 events):**

```bash
curl -N -H "Accept: text/event-stream" \
     http://localhost:3000/v1/stream | head -30

# Resume from a known position
curl -N -H "Accept: text/event-stream" \
     -H "Last-Event-ID: 42" \
     http://localhost:3000/v1/stream
```

---

## Error responses

All errors follow the same shape:

```json
{
  "code": "not_found",
  "message": "run xyz not found"
}
```

| HTTP status | `code` | When |
|-------------|--------|------|
| `400` | `bad_request` | Invalid request body (e.g. unknown approval decision) |
| `401` | `unauthorized` | Missing or invalid `Authorization: Bearer` token |
| `404` | `not_found` | Resource with the given ID does not exist |
| `500` | `internal_error` | Unexpected server-side error |

---

## Server configuration

```
cairn-app [OPTIONS]

Options:
  --mode  team       Self-hosted team mode (binds 0.0.0.0, requires CAIRN_ADMIN_TOKEN)
  --port  <port>     Listen port (default: 3000)
  --addr  <addr>     Bind address (default: 127.0.0.1)
  --db    <path|url> Storage backend:
                       postgres://...  → PostgreSQL
                       my_data.db      → SQLite
                       (omit)          → In-memory (local dev only)
  --encryption-key-env <VAR>  Read encryption key from this env var
```

**Environment variables:**

| Variable | Description |
|----------|-------------|
| `CAIRN_ADMIN_TOKEN` | Bearer token for the admin account. **Required in team mode.** Defaults to `dev-admin-token` in local mode. |

**Examples:**

```bash
# Local dev (default)
cargo run -p cairn-app

# Team mode on port 8080 with SQLite persistence
export CAIRN_ADMIN_TOKEN="$(openssl rand -hex 32)"
cargo run -p cairn-app -- --mode team --port 8080 --db cairn.db

# Team mode with PostgreSQL
export CAIRN_ADMIN_TOKEN="$(openssl rand -hex 32)"
cargo run -p cairn-app -- --mode team --db postgres://user:pass@localhost/cairn
```

---

## Route summary

| Method | Path | Auth | RFC | Description |
|--------|------|------|-----|-------------|
| `GET` | `/health` | No | — | Liveness probe |
| `GET` | `/v1/stream` | No | 002 | Real-time SSE event stream |
| `GET` | `/v1/status` | Yes | — | Runtime + store health |
| `GET` | `/v1/dashboard` | Yes | — | Operator overview |
| `GET` | `/v1/runs` | Yes | — | List runs (paginated) |
| `GET` | `/v1/runs/:id` | Yes | — | Get run by ID |
| `GET` | `/v1/sessions` | Yes | — | List active sessions |
| `GET` | `/v1/approvals/pending` | Yes | 005 | List pending approvals |
| `POST` | `/v1/approvals/:id/resolve` | Yes | 005 | Approve or reject |
| `GET` | `/v1/prompts/assets` | Yes | 006 | List prompt assets |
| `GET` | `/v1/prompts/releases` | Yes | 006 | List prompt releases |
| `GET` | `/v1/costs` | Yes | 009 | Aggregate cost summary |
| `GET` | `/v1/providers` | Yes | 007 | List provider bindings |
| `GET` | `/v1/events` | Yes | 002 | Replay event log (paginated) |
| `POST` | `/v1/events/append` | Yes | 002 | Append events (idempotent) |
