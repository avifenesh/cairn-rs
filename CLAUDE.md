# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.
AGENTS.md is a symlink to this file — one source of truth for all AI agents.

## Hard Rules

These are non-negotiable. Every instance MUST follow them.

1. **Production quality only.** This product hits real teams in production. Never cut corners. Go above and beyond on every piece — from error messages to test coverage to operator UX.
2. **Nothing ships without tests.** Integration tests are required for every feature. The project is stable only when covered by E2E tests. Write them proactively.
3. **Fix everything you touch.** Urgent to minor, in-scope or old broken code out of scope. If something is fragile or broken, fix it immediately. Don't wait to be asked.
4. **Verify subagent work.** YOU own results from any agent you spawn. Never assume they succeeded. Read their output, check the files, run the tests.
5. **Ask before designing.** Always validate design decisions with the user before implementing. Ask questions to confirm alignment on direction, then execute.
6. **Be proactive.** Check what can be improved. Propose enhancements. Find the gaps. But confirm direction before large changes.

*WHY: Cairn serves engineering teams who need a single all-in-one control plane for agent operations. Every feature must serve that vision — integrate everything teams need in one place. Quality is the product.*

## Source of Truth Order

When there is ambiguity, resolve in this order:

1. The relevant RFCs under `docs/design/rfcs/`
2. Compatibility docs under `docs/design/`
3. This file (CLAUDE.md)
4. The current Go implementation in `../cairn-sdk` only where preserved behavior or fixtures need to be checked

If the docs disagree, fix the docs before inventing local behavior.

## Core Project Rules

- One codebase and one product binary
- Local mode and self-hosted team mode are first-class in v1
- Managed cloud and hybrid are later motions, not v1 foundations
- Do not introduce a separate enterprise architecture fork
- Do not move canonical runtime truth into queues, plugins, or transient workers
- Do not bypass tenant/workspace/project scoping
- Do not re-open preserved route or SSE contracts casually

## Build & Run

```bash
cargo build --workspace                   # Full workspace build
cargo run -p cairn-app                    # Local dev (Postgres via DATABASE_URL, or in-memory fallback)

# With Postgres (recommended):
DATABASE_URL=postgres://cairn:pass@localhost:5432/cairn \
CAIRN_ADMIN_TOKEN=dev-admin-token cargo run -p cairn-app

# Explicit in-memory (dev only — all data lost on restart):
cargo run -p cairn-app -- --db memory

# Docker (Postgres included in docker-compose.yml):
docker compose up --build
```

Cairn is provider-agnostic — connect any LLM endpoint (OpenAI, Anthropic, Bedrock, Vertex, OpenRouter, Ollama, Groq, DeepSeek, xAI, Google Gemini, MiniMax, etc.) via env vars or `POST /v1/providers/connections`. 13 provider backends supported.

Default admin token: `dev-admin-token`. Dashboard: `http://localhost:3000`. Swagger: `/v1/docs`.

Auth token: `CAIRN_ADMIN_TOKEN` env var, or `CAIRN_ADMIN_TOKEN_FILE` for Docker secrets. Rotatable at runtime via `POST /v1/admin/rotate-token`.

CLI: `cairn-app --addr 0.0.0.0 --port 3000 --mode team --db postgres://user:pass@host/db`

Log rotation: set `CAIRN_LOG_DIR=/var/log/cairn` for daily-rotating log files.

## Tests

```bash
cargo test --workspace                                  # All ~3300+ tests
cargo test -p cairn-app --lib                           # App unit tests (72)
cargo test -p cairn-app --test full_workspace_suite     # E2E integration (6 workflows)
CAIRN_TOKEN=dev-admin-token ./scripts/smoke-test.sh     # 81-check HTTP smoke test
cd ui && npx tsc --noEmit                               # UI type check
cd ui && npm run build                                  # UI build (must succeed before cargo build)
cd ui && npx playwright test                            # 72 browser E2E tests (operator journeys)
```

Single test: `cargo test -p cairn-domain -- test_name`

## UI Development

```bash
cd ui && npm install && npm run dev    # Dev server :5173, proxies /v1/* → :3000
cd ui && npm run build                 # Production → ui/dist/ (embedded via rust-embed)
```

After `npm run build`, `cargo build -p cairn-app` embeds the new UI assets.

## Architecture

20-crate Rust workspace. Each crate owns one bounded context. No circular dependencies.

```
domain → store → runtime → {memory, graph, evals, tools, agent, signal, channels} → api/plugin-proto → app
```

| Crate | Owns |
|-------|------|
| `cairn-domain` | Pure types: IDs, commands (30+), events (56+), state machines, policy. No IO. |
| `cairn-store` | Append-only event log + sync projections. Backends: Postgres (default via `DATABASE_URL`) / SQLite / InMemory (`--db memory`). |
| `cairn-runtime` | Service layer: sessions, runs, tasks, approvals, checkpoints, mailbox, recovery. |
| `cairn-app` | Axum HTTP server. `router.rs` has route catalog, `middleware.rs` has auth/rate-limit, `bootstrap.rs` has CLI parsing. `main.rs` has binary wiring + embedded UI. |
| `cairn-memory` | Knowledge pipeline: ingest → chunk → embed → index → score → rerank → retrieve. |
| `cairn-graph` | Entity/provenance graph: nodes, edges, 6 query families, graph-backed expansion. |
| `cairn-evals` | Prompt registry, version/release lifecycle, scorecards, bandit experiments. |
| `cairn-tools` | Tool invocation, stdio JSON-RPC plugin host, permission gates, concurrency limits. |
| `cairn-orchestrator` | Agent orchestration loop, step execution, event emission. |
| `cairn-agent` | Higher-level agent patterns: ReAct, reflection, streaming, subagent spawning. |
| `cairn-workspace` | Sandbox workspace primitive (RFC 016): repo store, clone cache, sandbox lifecycle. |
| `cairn-integrations` | Integration plugin framework: GitHub, Linear, Notion, Obsidian, generic webhook. |
| `cairn-providers` | Unified LLM provider abstraction: 12+ backends (OpenAI, Bedrock, Vertex, Ollama, etc.). |
| `cairn-github` | GitHub App client: JWT auth, installation tokens, REST API, webhook verification. |
| `cairn-plugin-catalog` | Plugin marketplace and catalog (RFC 015): discovery, publishing, reviews. |
| `cairn-api` | Extracted HTTP route handlers (admin, evals, graph, memory, triggers, etc.). |
| `cairn-signal` | Signal detection and routing for event-driven automation (RFC 022). |
| `cairn-channels` | Notification channels: Slack, email, webhook delivery. |

### Event Sourcing

All state derives from an immutable event log. Flow: `RuntimeCommand` → handler → `RuntimeEvent` → `EventLog::append()` → `SyncProjection` updates read models. SSE at `GET /v1/stream` with Last-Event-ID replay.

### Multi-Tenancy

Every entity scoped by `ProjectKey { tenant_id, workspace_id, project_id }`. All queries filter by scope. UI stores active scope in localStorage and injects via `withScope()` in the API client.

### Auth

`auth_middleware` in `middleware.rs`: checks `Authorization: Bearer <token>` header. Fallback: `?token=` query param (for SSE EventSource). Static UI paths and `/health` are exempt — the React LoginPage handles token collection client-side.

### Provider Abstraction

Split-tier: Brain (heavy generation) + Worker (everyday + embeddings). Provider-agnostic — users connect their own endpoints. Supports any OpenAI-compatible API, plus native adapters for Bedrock, Vertex, and Ollama. All configured via env vars or `POST /v1/providers/connections`. Models hot-reloadable via `PUT /v1/settings/defaults/system/<key>`.

### UI

React 19 + TypeScript + Tailwind v4 + TanStack Query. 30 operator pages. Embedded in binary via `rust-embed`. Dark/light/system theme. API client: `ui/src/lib/api.ts`.

## Key Patterns

- **List responses** are inconsistent: some endpoints return `T[]`, others `{items: T[], hasMore}`. The UI `getList()` helper in `api.ts` normalizes both. Always use `getList()` for list endpoints.
- **Health endpoints**: `/health` returns `{status: "healthy", store_ok, ...}`. `/v1/status` returns `{status: "ok", components: [...]}`. Use `isRuntimeHealthy()` / `isStoreHealthy()` from `ui/src/lib/types.ts` — never access `runtime_ok` or `store_ok` directly.
- **Store backends**: Feature-gated via Cargo features (`postgres`, `sqlite`). Default is Postgres when `DATABASE_URL` is set. Use `--db memory` for explicit in-memory (ephemeral, with startup warning).
- **`unsafe_code = "forbid"`** at workspace level. No exceptions.
- **RFCs**: Behavior is specified by RFCs in `docs/design/rfcs/`. Each RFC has integration tests as compliance proof.

## Reminders

- Every API change must update the OpenAPI spec at `crates/cairn-app/src/openapi_spec.rs`.
- TypeScript types in `ui/src/lib/types.ts` MUST match actual API response shapes. When the API changes, update both sides.
- The smoke test at `scripts/smoke-test.sh` is the release gatekeeper. It must pass before any deploy.
- In-memory store (`--db memory`) loses all data on restart. Set `DATABASE_URL` for persistent storage (Postgres recommended, SQLite for single-node).

# CLAUDE.md

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.
