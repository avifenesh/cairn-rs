# cairn-rs Handover

## What is cairn-rs

A self-hostable Rust control plane for production AI agent deployments. Event-sourced architecture, multi-tenant, operator-focused. Single binary serves both the API and a React operator dashboard.

## Project Stats (2026-04-07)

| Metric | Count |
|--------|-------|
| Git commits | 430+ |
| Rust code | 170,000+ lines across 14 crates |
| TypeScript/TSX | 27,005 lines |
| UI pages | 35 |
| UI components | 22 |
| UI hooks | 8 |
| Total tests | 2,700+ (0 failures) |
| API routes | 56+ production, 366 bootstrap |
| RFCs implemented | 14/14 |

## Architecture

```
┌─────────────┐    ┌──────────────┐    ┌─────────────┐
│  React SPA  │───>│  axum HTTP   │───>│  Runtime     │
│  (embedded) │    │  + SSE + WS  │    │  Services    │
└─────────────┘    └──────────────┘    └──────┬──────┘
                                              │
                   ┌──────────────┐    ┌──────┴──────┐
                   │   Ollama     │    │  Event Log  │
                   │  (LLM/embed) │    │  + Store    │
                   └──────────────┘    └─────────────┘
                   ┌──────────────┐
                   │ OpenAI-compat│
                   │ (agntic.gdn) │
                   └──────────────┘
```

**Crates:**
- `cairn-domain` — types, events, commands, state machines
- `cairn-store` — EventLog trait, InMemory/SQLite/Postgres backends
- `cairn-runtime` — services (session, run, task, approval, checkpoint, plugin, provider routing, entitlements)
- `cairn-api` — bootstrap config, SSE, catalog
- `cairn-app` — HTTP server binary (main.rs) + test harness (lib.rs)
- `cairn-memory` — document store, retrieval, embedding, ingest pipeline
- `cairn-evals` — eval framework, rubric scoring, baseline comparison
- `cairn-graph` — knowledge graph, edge queries
- `cairn-tools` — tool invocation
- `cairn-signal` — signal event processing
- `cairn-channels` — notification channels
- `cairn-plugin-proto` — JSON-RPC plugin protocol
- `cairn-orchestrator` — GATHER → DECIDE → EXECUTE loop, GatherPhase/DecidePhase/ExecutePhase traits, OrchestratorLoop with checkpoint hook

## RFC Coverage

All 14 RFCs fully implemented:

| RFC | Feature |
|-----|---------|
| 001 | Product boundary (scope-only) |
| 002 | Runtime event model — 56+ event types, event log, projections |
| 003 | Owned retrieval — vector search, format parsers, hybrid mode, embedding pipeline |
| 004 | Graph + eval matrix — rubric scoring, baseline comparison, graph queries |
| 005 | Task lifecycle — pause/resume, recovery sweep, subagent linkage, checkpoints |
| 006 | Prompt release — version diffing, rollout percentage, A/B routing, approval gate |
| 007 | Plugin protocol — JSON-RPC, health monitoring, capability discovery, event subscriptions |
| 008 | Tenant/workspace — quota enforcement, workspace isolation, usage tracking |
| 009 | Provider routing — fallback chains, capability matching, cost-aware routing, health tracking |
| 010 | Operator control plane — 35-page React dashboard |
| 011 | Deployment shape — API-only, Worker-only, All-in-one roles |
| 012 | Onboarding — starter templates (chatbot, code-reviewer, data-analyst) |
| 013 | Artifact import/export — JSON/YAML bundles, conflict resolution |
| 014 | Entitlements — plan gating (Free/Pro/Enterprise), usage metering, enforcement |

## Running

```bash
# Quick start
CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app -- --addr 0.0.0.0 --port 3000

# With Ollama
OLLAMA_HOST=http://localhost:11434 CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app -- --addr 0.0.0.0 --port 3000

# Docker
docker compose up --build

# Development
make dev        # run server
make ui-dev     # vite dev server
make test       # full test suite
make smoke      # API smoke test (65 checks)
make check      # cargo check + tsc
```

**Token:** `cairn-demo-token` (set via CAIRN_ADMIN_TOKEN)

**Dashboard:** http://localhost:3000 (set token in browser: `localStorage.setItem('cairn_token', 'cairn-demo-token')`)

**Swagger UI:** http://localhost:3000/v1/docs

## UI Pages (35)

**Operations:** Dashboard, Sessions, Runs, Tasks, Approvals, Prompts, Workers, Orchestration, Workspaces
**Observability:** Traces, Memory, Sources, Costs, Evals (+ comparison), Graph, Audit Log, Logs, Metrics
**Infrastructure:** Providers, Plugins, Credentials, Channels, Deployment, Playground, API Docs, Settings, Test Harness, Cost Calculator, Profile
**Detail views:** RunDetail, SessionDetail, EvalComparison, ProjectDashboard
**System:** Login, 404 NotFound

## Key Features

- **Real-time SSE** with 10K event replay buffer + Last-Event-ID reconnection
- **WebSocket** transport alternative with toggle in Settings
- **LLM Playground** — streaming chat, conversation history, model comparison, markdown rendering
- **Command palette** (Cmd+K) with fuzzy search + keyboard shortcuts
- **Dark/light theme** with system preference detection
- **Responsive layout** — mobile-friendly with collapsible sidebar
- **Data tables** — sort, filter, CSV export, pagination, virtual scroll
- **Compact mode** toggle for dense information display
- **i18n** foundation (en, es, de, ja, zh)
- **Service worker** for offline caching
- **Rate limiting** — per-IP and per-token
- **OpenAPI spec** + Swagger UI
- **Docker** + docker-compose with Ollama
- **CI/CD** — GitHub Actions (5-job CI + release pipeline)
- **SDKs** — Python + TypeScript clients

## Key Endpoints

```
GET  /health                          — health check
GET  /v1/status                       — runtime status
GET  /v1/stats                        — aggregate statistics
GET  /v1/dashboard                    — dashboard data
GET  /v1/health/detailed              — subsystem health
GET  /v1/system/info                  — version, features, environment
GET  /v1/openapi.json                 — OpenAPI spec
GET  /v1/docs                         — Swagger UI

POST /v1/sessions                     — create session
POST /v1/runs                         — start run
GET  /v1/runs/:id                     — get run
POST /v1/tasks/:id/claim              — claim task
POST /v1/tasks/:id/complete           — complete task
POST /v1/approvals/:id/approve        — approve
POST /v1/approvals/:id/reject         — reject

GET  /v1/events/stream                — SSE event stream
GET  /v1/events/recent                — recent events (JSON)
GET  /v1/providers/ollama/models      — list models
POST /v1/providers/ollama/generate    — generate text
POST /v1/providers/ollama/stream      — streaming generation

GET  /v1/templates                    — starter templates
GET  /v1/entitlements                 — plan + usage
POST /v1/bundles/export               — export project bundle
POST /v1/bundles/import               — import bundle
GET  /v1/admin/audit-log              — audit trail
POST /v1/admin/rebuild-projections    — replay events
POST /v1/admin/snapshot               — export store state

POST /v1/runs/:id/orchestrate         — trigger GATHER → DECIDE → EXECUTE loop
                                        body: { goal, model_id, max_iterations, timeout_ms }
                                        returns: { termination, summary|reason|approval_id }
GET  /v1/settings/defaults/all        — bulk read all stored DefaultSettings
PUT  /v1/settings/defaults/:scope/:scope_id/:key  — hot-reload any setting (model names, URLs)
```

## Session 2026-04-07: Orchestrator + Persistence

### Orchestrator (cairn-orchestrator crate — new)

Complete GATHER → DECIDE → EXECUTE loop with all 7 steps wired:
- **GatherPhase / StandardGatherPhase** — reads EventLog, RetrievalService, DefaultsReadModel, CheckpointReadModel, GraphQueryService
- **DecidePhase / LlmDecidePhase** — calls brain LLM (agntic.garden), parses structured JSON proposals, applies confidence calibration
- **ExecutePhase / RuntimeExecutePhase** — dispatches InvokeTool, SpawnSubagent, CompleteRun, EscalateToOperator, SendNotification, CreateMemory through real runtime services
- **OrchestratorLoop** — 7-step body with timeout, approval gate, checkpoint hook, step summaries, tracing
- **POST /v1/runs/:id/orchestrate** — wired with real services (StubExecutePhase removed)

### Persistence

- **SQLite + Postgres dual-write** — `InMemoryStore.set_secondary_log()` covers all 109 `store.append()` call sites; startup replay warms InMemoryStore from event log
- **V020 migration registered** — `checkpoints.data_json` column now created
- **Org hierarchy projections** — `TenantCreated/WorkspaceCreated/ProjectCreated` write to SQL tables

### Config + Settings

- **RuntimeConfig** — hot-reloadable model defaults: `CAIRN_BRAIN_URL`, `CAIRN_WORKER_URL`, `CAIRN_DEFAULT_GENERATE_MODEL`, etc. read from DefaultsService first, then env vars, then hardcoded defaults
- **GET /v1/settings/defaults/all** — bulk read endpoint for SettingsPage
- **Split inference API** — brain (heavy generation) / worker (everyday + embeddings)

### Outcome tracking + confidence calibration

- **OutcomeRecord** + **OutcomeReadModel** — evaluator-optimizer feedback loop (Go PR #1222)
- **ConfidenceCalibrator** — 7-day window, per-agent-type adjustment

### Other

- **Provider routing safety tests** — 5 tests covering Go PR #1237-1240 patterns
- **ReembedAll** — `InMemoryDocumentStore.re_embed_all()` + `embedding_model_id` on ChunkRecord
- **Dead code cleanup** — 1715 lines removed from cairn-app/main.rs (68 warnings → 0)

## Session 2026-04-06: Quality Hardening

All 6 market-readiness seams closed in this session:

- **Durable memory CRUD** — MemoryApiImpl backed by DocumentStore instead of volatile HashMap
- **Chunk quality scoring** — pipeline computes credibility_score during ingest
- **Corroboration scoring** — cross-source retrieval corroboration pass (lexical + embedding)
- **Recency-of-use tracking** — per-chunk retrieval timestamps with tiered decay
- **OpenAI-compatible provider** — GenerationProvider + EmbeddingProvider against any OpenAI-compat endpoint (agntic.garden wired)
- **Embedding pipeline active** — IngestPipeline wired with real embedder (qwen3-embedding:8b) when OPENAI_COMPAT_BASE_URL is set; vector and hybrid retrieval modes now functional

Additional fixes: cairn-store latest_root_run tiebreaker, cairn-runtime RunCostUpdated import, SDK provider connection methods.

Final per-crate sweep: **2,636 tests passed, 0 failed, 7 ignored** across all 12 crates.

## What's Next

1. **End-to-end orchestrator validation** — run `POST /v1/runs/:id/orchestrate` with a live brain provider (CAIRN_BRAIN_URL); smoke test section 21 currently skips when provider is offline (503)
2. **Postgres in production** — SQLite + dual-write is proven; Postgres path validated by W1; 13 of 18 projection tables have write handlers (route_decisions + provider_calls deferred)
3. **Multi-user auth** — proper user management beyond single admin token
4. **Plugin ecosystem** — build and connect real plugins
5. **Horizontal scaling** — test Worker-only + API-only deployment roles
6. **Load testing** — stress test with concurrent agent sessions + orchestrator loops
7. **Monitoring** — connect to Grafana/Prometheus via /v1/metrics/prometheus

## Repository

https://github.com/avifenesh/cairn-rs (private)
