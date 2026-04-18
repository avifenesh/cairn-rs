# cairn-rs

**Self-hostable Rust control plane for production AI agent deployments.**

![Rust](https://img.shields.io/badge/rust-1.95-orange?logo=rust)
![License](https://img.shields.io/badge/license-BSL--1.1-blue)
![Status](https://img.shields.io/badge/status-active-brightgreen)

---

## What is cairn-rs

cairn-rs is a source-available operator control plane that sits between your AI agents and your infrastructure. It handles the operational concerns — event sourcing, task orchestration, approval gates, provider routing, cost metering, and real-time observability — so your agent code stays focused on product logic.

The architecture is fully event-sourced: every agent action, LLM call, approval decision, and checkpoint is appended to an immutable log. The current state of any entity is derived by replaying its events. This gives you a complete audit trail, deterministic replay, and idempotent command handling out of the box.

cairn-rs is designed for teams that want the reliability of purpose-built infrastructure without the complexity of a hosted platform. It runs as a single binary, stores events in Postgres (or SQLite for local dev), and ships a React operator dashboard that works without additional configuration.

<!-- TODO: add screenshot -->

---

## Key features

### Core Runtime
- **Event-sourced runtime** — 120+ domain event types; append-only log with monotonically increasing positions; idempotent command dispatch via causation-id deduplication
- **Real-time SSE streaming** — live event feed at `GET /v1/stream`; reconnecting clients replay via `Last-Event-ID`; no polling required
- **Multi-tenant isolation** — tenant / workspace / project hierarchy; RBAC per workspace; every query scoped by `ProjectKey`
- **Durable recovery** — dual checkpoint (Intent/Result) per orchestrator iteration; deterministic `ToolCallId` with call_index for parallel dispatch; `RetrySafety` three-tier classification (IdempotentSafe / DangerousPause / AuthorResponsible); parallel-where-independent startup dependency graph with per-branch readiness at `/health/ready`

### Plugin Marketplace
- **One-click plugin activation** — discover, install, credential wizard, enable per project; no code changes to cairn-rs core
- **Per-project scoping** — plugins enabled per project; tool visibility and signal routing filtered by `VisibilityContext`; agents only see what their project has turned on
- **Signal Knowledge Capture** — plugin signals auto-projected into cairn-graph (default on) and optionally ingested into cairn-memory (opt-in per signal type)
- **External binary model** — plugins are independent executables speaking JSON-RPC over stdio; not bundled, not embedded, not arg0

### Sandbox Workspace
- **Per-run isolated environments** — `cairn-workspace` crate with OverlayFS (Linux) and reflink-copy (macOS/Windows) providers
- **Immutable repo cache** — `RepoCloneCache` (tenant-scoped) + `ProjectRepoAccessService` (project-scoped access allowlist); disk-efficient sharing with per-project isolation
- **Resource limits** — disk, memory, wall-clock caps with three exhaustion modes (Destroy / PauseAwaitOperator / ReportOnly)
- **Drift detection** — overlay sandboxes check base_revision on recovery; reflink sandboxes are physically independent

### Agent Loop
- **Plan / Execute / Direct modes** — Plan mode restricts agents to Observational + Internal tools; produces a markdown plan artifact for human review before any external action
- **Guardian resolver** — LLM-based approval resolver with structured output, risk ceiling, fail-closed timeout; configurable per project with tenant inheritance
- **Inline context compaction** — automatic history summarization when context exceeds threshold; preserves recent steps, compresses older ones
- **30+ built-in tools** — all classified by ToolEffect (Observational/Internal/External) and RetrySafety; wired into the orchestrate registry with `tool_search` for deferred-tier discovery

### Unified Decision Layer
- **One truth per decision** — single `DecisionRecorded` event for both Allowed and Denied; 8-step evaluation (scope → visibility → guardrail → budget → cache → resolver → write → return)
- **Learned rules** — singleflight cache with Miss/Pending/Resolved states; operator-approved decisions auto-apply to future equivalent requests within TTL
- **Selective invalidation** — policy-rule reference index; editing a guardrail rule invalidates only decisions that referenced it

### Triggers
- **Signal-to-run binding** — declarative `Trigger` entity with condition DSL (Equals/Contains/Exists/Not) and `RunTemplate` for reusable run configuration
- **Durable fire ledger** — prevents duplicate runs on webhook retry or signal replay; separate from ingress dedup
- **Loop prevention** — `source_run_id` on signal envelope; chain-depth tracking with configurable cap
- **Runaway protection** — per-trigger rate limits + per-project trigger budgets, both backed by durable projections

### Protocols & Observability
- **SQ/EQ protocol** — versioned REST + SSE for external clients (IDE extensions, scripts, alternate dashboards); scope-bound transport sessions with `correlation_id` threading
- **A2A Agent Card** — `GET /.well-known/agent.json` per A2A v0.3; task submission at `POST /v1/a2a/tasks`
- **OTLP GenAI export** — every LLM call, tool invocation, and run exported as OTel spans with 2025 GenAI semantic conventions; works with Langfuse, Phoenix, Grafana Tempo, Jaeger, Datadog
- **Dynamic provider discovery** — `PluginCapability::GenerationProvider` lets plugins register as LLM providers at runtime

### Foundation (Phase 1)
- **LLM provider abstraction** — unified generation interface over OpenAI, Anthropic, Bedrock, OpenRouter, Azure, and any OpenAI-compatible endpoint; priority-ranked fallback chains
- **Approval workflows** — human-in-the-loop gates that block run or task progression until an operator resolves; full decision audit trail
- **Built-in eval framework** — eval runs, scoring rubrics, locked baselines, regression detection, multi-armed bandit for live traffic steering
- **Operator dashboard** — embedded React + TypeScript UI; sessions, runs, tasks, approvals, traces, costs, memory, and playground views
- **Cost tracking** — per-call token counts and USD micros; run-level and session-level aggregation
- **Knowledge and memory** — document ingestion pipeline with chunking, deduplication, and multi-factor scoring

---

## Quick start

### Cargo (local dev)

```bash
git clone https://github.com/avifenesh/cairn-rs
cd cairn-rs
cargo run -p cairn-app

# Health check
curl http://localhost:3000/health
# {"ok":true}

# With Ollama for local LLM support
OLLAMA_HOST=http://localhost:11434 cargo run -p cairn-app

# With the agntic.garden split inference API (brain + worker tiers)
CAIRN_BRAIN_URL=https://agntic.garden/inference/brain/v1 \
CAIRN_WORKER_URL=https://agntic.garden/inference/worker/v1 \
OPENAI_COMPAT_API_KEY=Cairn-Inference-2026! \
  cargo run -p cairn-app

# With any other OpenAI-compatible provider (legacy single-endpoint)
OPENAI_COMPAT_BASE_URL=https://your-server/v1 \
OPENAI_COMPAT_API_KEY=your-key \
  cargo run -p cairn-app

# With OpenRouter (free models available — great for testing)
CAIRN_BRAIN_URL=https://openrouter.ai/api/v1 \
OPENROUTER_API_KEY=sk-or-your-key-here \
  cargo run -p cairn-app
```

Default bearer token: `dev-admin-token`. Set `CAIRN_ADMIN_TOKEN` to override.

**Inference providers:** cairn-rs supports a split-tier inference API:
- **Brain** (`CAIRN_BRAIN_URL`): heavy generation — default model `cyankiwi/gemma-4-31B-it-AWQ-4bit`
- **Worker** (`CAIRN_WORKER_URL`): everyday generation + embeddings — default model `qwen3.5:9b`
- Both read `OPENAI_COMPAT_API_KEY` for auth; `CAIRN_BRAIN_KEY` / `CAIRN_WORKER_KEY` take precedence when set.
- Legacy `OPENAI_COMPAT_BASE_URL` still works and maps to the worker path.
- **[OpenRouter](https://openrouter.ai)** — set `OPENROUTER_API_KEY` (preferred) or any of the above key vars.
  Free-tier models include `qwen/qwen3-coder:free` (262K context), `deepseek/deepseek-chat:free`, and more.
  OpenRouter exposes a standard OpenAI-compatible endpoint so no code changes are required.

All model names are hot-reloadable via `PUT /v1/settings/defaults/system/<key>` — no restart required.

### Docker

```bash
# One command — starts cairn-app + Postgres + Ollama
docker compose up --build

# Background
docker compose up -d --build

# Override the admin token
echo 'CAIRN_ADMIN_TOKEN=my-secret-token' > .env
docker compose up -d
```

Schema migrations run automatically on first boot. Ollama models are cached in
a Docker volume; pull additional models with:

```bash
docker compose exec ollama ollama pull qwen3.5:9b        # worker/everyday generation
docker compose exec ollama ollama pull nomic-embed-text  # embeddings
```

### After startup

| URL | Description |
|-----|-------------|
| `http://localhost:3000` | Operator dashboard |
| `http://localhost:3000/v1/docs` | Interactive API explorer (Swagger UI) |
| `http://localhost:3000/v1/openapi.json` | OpenAPI 3.0 spec |
| `http://localhost:3000/health` | Liveness probe |

---

## Architecture

cairn-rs currently spans a 20-crate workspace plus `cairn-providers` as a
repo-local path crate consumed by the app/runtime surface. Each crate owns
one bounded context with no circular dependencies.

```
cairn-domain       pure domain types, events, lifecycle rules, RFC contracts
cairn-store        append-only event log + synchronous projections (InMemory / Postgres / SQLite)
cairn-runtime      service implementations: sessions, runs, tasks, approvals, routing, evals
cairn-providers    unified chat/completion/embedding provider abstraction
cairn-api          HTTP types, SSE payloads, auth, bootstrap config, API error shapes
cairn-app          axum HTTP server, startup wiring, all route handlers, embedded React UI
cairn-memory       knowledge pipeline: ingest, chunking, retrieval, graph-backed expansion
cairn-graph        entity relationship graph: nodes, edges, traversal, proximity scoring
cairn-evals        eval runs, rubrics, baselines, scorecard matrices, bandit experiments
cairn-tools        tool invocation, plugin host (stdio JSON-RPC), capability verification
cairn-tools-derive proc-macro helpers for built-in and plugin-exposed tools
cairn-agent        agent orchestration loop, reflection, hook pipeline
cairn-orchestrator gather/decide/execute loop runner over the runtime spine
cairn-signal       signal ingestion and routing between agents
cairn-channels     async message channels between agents
cairn-plugin-catalog bundled marketplace catalog descriptors for RFC 015
cairn-plugin-proto plugin wire protocol types and capability declarations
cairn-workspace    repo clone cache and sandbox lifecycle primitives
cairn-github       standalone GitHub App auth, webhook, and REST client SDK
cairn-integrations integration registry and per-service plugin surfaces
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
| `CAIRN_BRAIN_URL` | _(unset)_ | Heavy/generate provider base URL (e.g. `https://…/brain/v1`). Used for generation. |
| `CAIRN_BRAIN_KEY` | _(unset)_ | API key for the brain provider. |
| `CAIRN_WORKER_URL` | _(unset)_ | Light/embed provider base URL (e.g. `https://…/worker/v1`). Used for embedding. |
| `CAIRN_WORKER_KEY` | _(unset)_ | API key for the worker provider. |
| `OPENAI_COMPAT_BASE_URL` | _(unset)_ | Legacy: maps to both BRAIN and WORKER when set. Superseded by the split vars above. |
| `OPENAI_COMPAT_API_KEY` | _(unset)_ | Legacy: API key for the legacy single-endpoint provider. |
| `OPENROUTER_API_KEY` | _(unset)_ | API key for [OpenRouter](https://openrouter.ai). When set, `CAIRN_BRAIN_URL` can be pointed at `https://openrouter.ai/api/v1`. Free-tier models (e.g. `qwen/qwen3-coder:free`) work without a paid account. |

### CLI flags

```
cairn-app [OPTIONS]

  --addr   <addr>   Bind address              (default: 127.0.0.1)
  --port   <port>   Listen port               (default: 3000)
  --mode   team     Bind 0.0.0.0, require token, enable team features
  --db     <url>    postgres://... or sqlite:path.db or memory
                    Also reads DATABASE_URL env var (Postgres default)
```

### Persistence backends

| Backend | Config | Notes |
|---------|--------|-------|
| Postgres | `DATABASE_URL=postgres://...` _(default)_ | Full durability. Concurrent writers. Schema migrations run on startup. |
| SQLite | `--db sqlite:cairn.db` | Durable single-file store. Single-writer. |
| In-memory | `--db memory` | Resets on restart. Local development only. Startup warning emitted. |

---

## API overview

All `/v1/` routes require `Authorization: Bearer <token>`. `/health` and `/v1/stream` are public.

| Group | Endpoints |
|-------|-----------|
| **Health** | `GET /health`, `GET /v1/status`, `GET /v1/dashboard`, `GET /v1/overview` |
| **Sessions** | `GET/POST /v1/sessions`, `GET /v1/sessions/:id`, `GET /v1/sessions/:id/runs` |
| **Runs** | `GET/POST /v1/runs`, `GET /v1/runs/:id`, `POST /v1/runs/:id/pause`, `POST /v1/runs/:id/resume`, `GET /v1/runs/:id/events`, `GET /v1/runs/:id/cost` |
| **Tasks** | `GET /v1/tasks`, `POST /v1/tasks/:id/claim`, `POST /v1/tasks/:id/complete`, `POST /v1/tasks/:id/cancel`, `POST /v1/tasks/:id/release-lease` |
| **Approvals** | `POST /v1/approvals` (create gate), `GET /v1/approvals/pending`, `POST /v1/approvals/:id/approve`, `POST /v1/approvals/:id/reject`, `POST /v1/approvals/:id/resolve` |
| **Prompts** | `GET /v1/prompts/assets`, `GET /v1/prompts/releases`, `POST /v1/prompts/releases/:id/transition`, `POST /v1/prompts/releases/:id/activate` |
| **Events** | `GET /v1/events` (cursor replay), `POST /v1/events/append` (idempotent write), `GET /v1/stream` (SSE live feed) |
| **Providers** | `GET /v1/providers`, `GET /v1/providers/health`, `POST /v1/providers/connections`, `POST /v1/providers/bindings` |
| **Ollama** | `GET /v1/providers/ollama/models`, `POST /v1/providers/ollama/generate`, `POST /v1/providers/ollama/stream` |
| **Memory** | `POST /v1/memory/ingest`, `GET /v1/memory/search`, `GET /v1/memory/documents/:id`, `GET /v1/memory/diagnostics`, `POST /v1/memory/feedback` |
| **Sources** | `GET /v1/sources`, `GET /v1/sources/:id`, `POST /v1/sources` |
| **Bundles** | `POST /v1/bundles/import`, `POST /v1/bundles/export` |
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

# Run the server (Postgres via DATABASE_URL, or in-memory fallback)
cargo run -p cairn-app

# Run all 3,300+ tests
cargo test --workspace

# Run the end-to-end integration suite (6 full-workflow tests)
cargo test -p cairn-app --test full_workspace_suite

# Run the 81-check smoke test against a running server
CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app &
CAIRN_TOKEN=cairn-demo-token ./scripts/smoke-test.sh

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
  cairn-tools-derive/ Proc-macro helpers for tools
  cairn-agent/     Agent orchestration loop
  cairn-orchestrator/ Gather/decide/execute loop runner
  cairn-signal/    Signal routing
  cairn-channels/  Agent message channels
  cairn-plugin-proto/ Plugin wire protocol
  cairn-plugin-catalog/ Marketplace catalog descriptors
  cairn-workspace/ Repo clone cache and sandbox lifecycle
  cairn-github/    GitHub App auth, webhook, REST client
  cairn-integrations/ Integration registry and plugin surfaces
ui/                React + TypeScript operator dashboard
docs/
  design/rfcs/     RFC specifications (001–023)
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

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for prerequisites, development
workflow, testing instructions, code style requirements, and the pull-request
process.

---

## License

MIT — see [LICENSE](./LICENSE).
