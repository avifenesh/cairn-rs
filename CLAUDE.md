# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Hard Rules

These are non-negotiable. Every instance MUST follow them.

1. **Production quality only.** This product hits real teams in production. Never cut corners. Go above and beyond on every piece — from error messages to test coverage to operator UX.
2. **Nothing ships without tests.** Integration tests are required for every feature. The project is stable only when covered by E2E tests. Write them proactively.
3. **Fix everything you touch.** Urgent to minor, in-scope or old broken code out of scope. If something is fragile or broken, fix it immediately. Don't wait to be asked.
4. **Verify subagent work.** YOU own results from any agent you spawn. Never assume they succeeded. Read their output, check the files, run the tests.
5. **Ask before designing.** Always validate design decisions with the user before implementing. Ask questions to confirm alignment on direction, then execute.
6. **Be proactive.** Check what can be improved. Propose enhancements. Find the gaps. But confirm direction before large changes.

*WHY: Cairn serves engineering teams who need a single all-in-one control plane for agent operations. Every feature must serve that vision — integrate everything teams need in one place. Quality is the product.*

## Build & Run

```bash
cargo build --workspace                   # Full workspace build
cargo run -p cairn-app                    # Local dev (in-memory store)

# With any OpenAI-compatible provider
CAIRN_BRAIN_URL=https://your-provider/v1 \
OPENAI_COMPAT_API_KEY=<key> \
CAIRN_ADMIN_TOKEN=dev-admin-token cargo run -p cairn-app

# Docker
docker compose up --build
```

Cairn is provider-agnostic — connect any LLM endpoint (OpenAI, Anthropic, Bedrock, Vertex, OpenRouter, Ollama, Groq, etc.) via env vars or `POST /v1/providers/connections`.

Default admin token: `dev-admin-token`. Dashboard: `http://localhost:3000`. Swagger: `/v1/docs`.

CLI: `cairn-app --addr 0.0.0.0 --port 3000 --mode team --db postgres://user:pass@host/db`

## Tests

```bash
cargo test --workspace                                  # All ~2700 tests
cargo test -p cairn-app --lib                           # App unit tests (49)
cargo test -p cairn-app --test full_workspace_suite     # E2E integration (6 workflows)
CAIRN_TOKEN=dev-admin-token ./scripts/smoke-test.sh     # 81-check HTTP smoke test
cd ui && npx tsc --noEmit                               # UI type check
cd ui && npm run build                                  # UI build (must succeed before cargo build)
```

Single test: `cargo test -p cairn-domain -- test_name`

## UI Development

```bash
cd ui && npm install && npm run dev    # Dev server :5173, proxies /v1/* → :3000
cd ui && npm run build                 # Production → ui/dist/ (embedded via rust-embed)
```

After `npm run build`, `cargo build -p cairn-app` embeds the new UI assets.

## Architecture

14-crate Rust workspace. Each crate owns one bounded context. No circular dependencies.

```
domain → store → runtime → {memory, graph, evals, tools, agent, signal, channels} → api/plugin-proto → app
```

| Crate | Owns |
|-------|------|
| `cairn-domain` | Pure types: IDs, commands (30+), events (56+), state machines, policy. No IO. |
| `cairn-store` | Append-only event log + sync projections. Backends: InMemory / Postgres / SQLite (feature-gated). |
| `cairn-runtime` | Service layer: sessions, runs, tasks, approvals, checkpoints, mailbox, recovery. |
| `cairn-app` | Axum HTTP server. `lib.rs` has route catalog + middleware. `main.rs` has binary wiring + embedded UI. |
| `cairn-memory` | Knowledge pipeline: ingest → chunk → embed → index → score → rerank → retrieve. |
| `cairn-graph` | Entity/provenance graph: nodes, edges, 6 query families, graph-backed expansion. |
| `cairn-evals` | Prompt registry, version/release lifecycle, scorecards, bandit experiments. |
| `cairn-tools` | Tool invocation, stdio JSON-RPC plugin host, permission gates, concurrency limits. |
| `cairn-orchestrator` | Agent orchestration loop, step execution, event emission. |

### Event Sourcing

All state derives from an immutable event log. Flow: `RuntimeCommand` → handler → `RuntimeEvent` → `EventLog::append()` → `SyncProjection` updates read models. SSE at `GET /v1/stream` with Last-Event-ID replay.

### Multi-Tenancy

Every entity scoped by `ProjectKey { tenant_id, workspace_id, project_id }`. All queries filter by scope. UI stores active scope in localStorage and injects via `withScope()` in the API client.

### Auth

`auth_middleware` in `lib.rs`: checks `Authorization: Bearer <token>` header. Fallback: `?token=` query param (for SSE EventSource). Static UI paths and `/health` are exempt — the React LoginPage handles token collection client-side.

### Provider Abstraction

Split-tier: Brain (heavy generation) + Worker (everyday + embeddings). Provider-agnostic — users connect their own endpoints. Supports any OpenAI-compatible API, plus native adapters for Bedrock, Vertex, and Ollama. All configured via env vars or `POST /v1/providers/connections`. Models hot-reloadable via `PUT /v1/settings/defaults/system/<key>`.

### UI

React 19 + TypeScript + Tailwind v4 + TanStack Query. 30 operator pages. Embedded in binary via `rust-embed`. Dark/light/system theme. API client: `ui/src/lib/api.ts`.

## Key Patterns

- **List responses** are inconsistent: some endpoints return `T[]`, others `{items: T[], hasMore}`. The UI `getList()` helper in `api.ts` normalizes both. Always use `getList()` for list endpoints.
- **Health endpoints**: `/health` returns `{status: "healthy", store_ok, ...}`. `/v1/status` returns `{status: "ok", components: [...]}`. Use `isRuntimeHealthy()` / `isStoreHealthy()` from `ui/src/lib/types.ts` — never access `runtime_ok` or `store_ok` directly.
- **Store backends**: Feature-gated via Cargo features (`postgres`, `sqlite`). Default is in-memory (ephemeral — all data lost on restart).
- **`unsafe_code = "forbid"`** at workspace level. No exceptions.
- **RFCs**: Behavior is specified by RFCs in `docs/design/rfcs/`. Each RFC has integration tests as compliance proof.

## Reminders

- Every API change must update the OpenAPI spec at `crates/cairn-app/src/openapi_spec.rs`.
- TypeScript types in `ui/src/lib/types.ts` MUST match actual API response shapes. When the API changes, update both sides.
- The smoke test at `scripts/smoke-test.sh` is the release gatekeeper. It must pass before any deploy.
- In-memory store loses all data on restart. For persistent dogfood testing, use `--db cairn.db`.
