# cairn-rs

**Self-hostable Rust control plane for production AI agent deployments.**

![Rust](https://img.shields.io/badge/rust-1.83%2B-orange?logo=rust)
![License](https://img.shields.io/badge/license-MIT-blue)
![Status](https://img.shields.io/badge/status-active-brightgreen)

---

## What is cairn-rs

cairn-rs is an open-source operator control plane that sits between your AI agents and your infrastructure. It handles the operational concerns — event sourcing, task orchestration, approval gates, provider routing, cost metering, and real-time observability — so your agent code stays focused on product logic.

The architecture is fully event-sourced: every agent action, LLM call, approval decision, and checkpoint is appended to an immutable log. The current state of any entity is derived by replaying its events. This gives you a complete audit trail, deterministic replay, and idempotent command handling out of the box.

cairn-rs is designed for teams that want the reliability of purpose-built infrastructure without the complexity of a hosted platform. It runs as a single binary, stores events in Postgres (or SQLite for local dev), and ships a React operator dashboard that works without additional configuration.

<!-- TODO: add screenshot -->

---

## Key features

- **Event-sourced runtime** — 56+ domain event types; append-only log with monotonically increasing positions; idempotent command dispatch via causation-id deduplication
- **Real-time SSE streaming** — live event feed at `GET /v1/stream`; reconnecting clients replay up to 1 000 missed events via `Last-Event-ID`; no polling required
- **Multi-tenant isolation** — tenant / workspace / project hierarchy; RBAC (Viewer, Member, Admin, Owner) per workspace; every query scoped by tenant
- **Approval workflows** — human-in-the-loop gates that block run or task progression until an operator resolves; full decision audit trail
- **LLM provider abstraction** — unified generation interface over OpenAI, Anthropic, Bedrock, OpenRouter, Azure, and any OpenAI-compatible endpoint; priority-ranked fallback chains
- **Built-in eval framework** — eval runs, scoring rubrics, locked baselines, regression detection, multi-armed bandit (EpsilonGreedy / UCB1) for live traffic steering
- **Operator dashboard** — embedded React + TypeScript UI served from the binary; sessions, runs, tasks, approvals, traces, costs, memory, and playground views
- **Local LLM support** — first-class Ollama integration; `OLLAMA_HOST` env var; `options.think=false` for Qwen3 chain-of-thought suppression
- **Cost tracking and token metering** — per-call token counts and USD micros; run-level and session-level aggregation; `GET /v1/costs` for operator-facing totals
- **Knowledge and memory retrieval** — document ingestion pipeline with chunking, deduplication, and multi-factor scoring (lexical relevance, freshness, credibility, graph proximity)

---

## Quick start

```bash
# Clone and run (in-memory, no config required)
git clone https://github.com/avifenesh/cairn-rs
cd cairn-rs
cargo run -p cairn-app

# Health check
curl http://localhost:3000/health
# {"ok":true}

# With Ollama for local LLM support
OLLAMA_HOST=http://localhost:11434 cargo run -p cairn-app
```

Default bearer token: `dev-admin-token`. Set `CAIRN_ADMIN_TOKEN` to override.

The embedded operator dashboard is available at **http://localhost:3000** — no separate frontend server needed.

---

## Architecture

cairn-rs is a 13-crate Rust workspace. Each crate owns one bounded context with no circular dependencies.

```
cairn-domain       pure domain types, events, lifecycle rules, RFC contracts
cairn-store        append-only event log + synchronous projections (InMemory / Postgres / SQLite)
cairn-runtime      service implementations: sessions, runs, tasks, approvals, routing, evals
cairn-api          HTTP types, SSE payloads, auth, bootstrap config, API error shapes
cairn-app          axum HTTP server, startup wiring, all route handlers, embedded React UI
cairn-memory       knowledge pipeline: ingest, chunking, retrieval, graph-backed expansion
cairn-graph        entity relationship graph: nodes, edges, traversal, proximity scoring
cairn-evals        eval runs, rubrics, baselines, scorecard matrices, bandit experiments
cairn-tools        tool invocation, plugin host (stdio JSON-RPC), capability verification
cairn-agent        agent orchestration loop, reflection, hook pipeline
cairn-signal       signal ingestion and routing between agents
cairn-channels     async message channels between agents
cairn-plugin-proto plugin wire protocol types and capability declarations
```

### Data flow

```
HTTP request
  └─► Command handler
        ├─► append(events) ──► InMemoryStore projections ──► read models
        │                  ──► Postgres event log          (when --db postgres://...)
        │                  ──► broadcast channel           ──► SSE subscribers
        └─► HTTP response  (returns latest projected state)
```

State is always derived from the log. Postgres stores events for durability and cursor-based replay; the in-memory store drives read models and the SSE broadcast. There is no separate synchronization step.

---

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `CAIRN_ADMIN_TOKEN` | `dev-admin-token` | Bearer token for the admin account. Required in team mode. |
| `OLLAMA_HOST` | _(unset)_ | Ollama base URL, e.g. `http://localhost:11434`. Enables local LLM endpoints. |

### CLI flags

```
cairn-app [OPTIONS]

  --addr   <addr>   Bind address              (default: 127.0.0.1)
  --port   <port>   Listen port               (default: 3000)
  --mode   team     Bind 0.0.0.0, require token, enable team features
  --db     <url>    postgres://... or sqlite:path.db
                    Omit to use ephemeral in-memory store (local dev)
```

### Persistence backends

| Backend | Flag | Notes |
|---------|------|-------|
| In-memory | _(default)_ | Resets on restart. Local development only. |
| SQLite | `--db cairn.db` | Durable single-file store. Single-writer. |
| Postgres | `--db postgres://user:pass@host/db` | Full durability. Concurrent writers. Schema migrations run on startup. |

---

## API overview

All `/v1/` routes require `Authorization: Bearer <token>`. `/health` and `/v1/stream` are public.

| Group | Endpoints |
|-------|-----------|
| **Health** | `GET /health`, `GET /v1/status`, `GET /v1/dashboard`, `GET /v1/overview` |
| **Sessions** | `GET/POST /v1/sessions`, `GET /v1/sessions/:id`, `GET /v1/sessions/:id/runs` |
| **Runs** | `GET/POST /v1/runs`, `GET /v1/runs/:id`, `POST /v1/runs/:id/pause`, `POST /v1/runs/:id/resume`, `GET /v1/runs/:id/events`, `GET /v1/runs/:id/cost` |
| **Tasks** | `GET /v1/tasks`, `POST /v1/tasks/:id/claim`, `POST /v1/tasks/:id/complete`, `POST /v1/tasks/:id/cancel`, `POST /v1/tasks/:id/release-lease` |
| **Approvals** | `GET /v1/approvals/pending`, `POST /v1/approvals/:id/resolve`, `POST /v1/approvals/:id/deny` |
| **Prompts** | `GET /v1/prompts/assets`, `GET /v1/prompts/releases`, `POST /v1/prompts/releases/:id/transition`, `POST /v1/prompts/releases/:id/activate` |
| **Events** | `GET /v1/events` (cursor replay), `POST /v1/events/append` (idempotent write), `GET /v1/stream` (SSE live feed) |
| **Providers** | `GET /v1/providers`, `GET /v1/providers/health`, `POST /v1/providers/connections`, `POST /v1/providers/bindings` |
| **Ollama** | `GET /v1/providers/ollama/models`, `POST /v1/providers/ollama/generate`, `POST /v1/providers/ollama/stream` |
| **Memory** | `POST /v1/memory/ingest`, `GET /v1/memory/search`, `POST /v1/memory/deep-search`, `POST /v1/memory/feedback` |
| **Evals** | `POST /v1/evals/runs`, `POST /v1/evals/runs/:id/start`, `POST /v1/evals/runs/:id/complete`, `GET /v1/evals/scorecard/:asset_id` |
| **Costs** | `GET /v1/costs`, `GET /v1/traces`, `GET /v1/sessions/:id/llm-traces` |
| **Admin** | `POST /v1/admin/tenants`, `GET /v1/settings`, `GET /v1/db/status` |

---

## Development

### Prerequisites

- Rust 1.83+ (`rustup update stable`)
- Node.js 20+ (for the UI only)

### Build and test

```bash
# Full workspace build
cargo build --workspace

# Run the server (in-memory, local mode)
cargo run -p cairn-app

# Run all tests (excluding cairn-app integration tests)
cargo test --workspace --exclude cairn-app

# Run the bootstrap integration tests
cargo test -p cairn-app --test bootstrap_server

# UI development server (proxies /v1/* to localhost:3000)
cd ui && npm install && npm run dev
# Opens at http://localhost:5173

# Rebuild UI and embed it in the binary
cd ui && npm run build
cargo build -p cairn-app   # picks up ui/dist/ via rust-embed
```

### Project structure

```
crates/
  cairn-app/       HTTP server binary + embedded React UI (ui/dist/)
  cairn-domain/    Domain types, events, RFC contracts
  cairn-store/     Event log, projections, Postgres/SQLite adapters
  cairn-runtime/   Service layer (sessions, runs, tasks, approvals, routing)
  cairn-api/       HTTP API types, SSE, auth, bootstrap
  cairn-memory/    Knowledge retrieval pipeline
  cairn-graph/     Entity graph (nodes, edges, traversal)
  cairn-evals/     Eval framework, rubrics, bandit experiments
  cairn-tools/     Tool invocation, plugin host
  cairn-agent/     Agent orchestration loop
  cairn-signal/    Signal routing
  cairn-channels/  Agent message channels
  cairn-plugin-proto/ Plugin wire protocol
ui/                React + TypeScript operator dashboard
docs/
  design/rfcs/     RFC specifications (002–014)
  api-reference.md Full endpoint reference
  deployment.md    Docker, Postgres, TLS, production hardening
```

### RFC compliance

Cairn's behaviour is specified by RFCs in `docs/design/rfcs/`. Each RFC has a corresponding integration test that serves as the compliance proof.

| RFC | Scope |
|-----|-------|
| 002 | Event-log durability, idempotency, SSE replay |
| 003 | Memory retrieval pipeline |
| 004 | Checkpoints, eval system, baselines |
| 005 | Approval blocking |
| 006 | Prompt release lifecycle |
| 007 | Provider connection health |
| 008 | Multi-tenant isolation, RBAC |
| 009 | Provider routing and cost tracking |
| 013 | Bundle import/export, eval rubrics |
| 014 | Commercial tiers and feature gating |

---

## License

MIT — see [LICENSE](./LICENSE).
