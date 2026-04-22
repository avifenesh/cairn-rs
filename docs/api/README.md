# Cairn HTTP API — Per-Subject Reference

Cairn's HTTP surface is grouped into subject-area files. The canonical source for the full route inventory is [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv), which is kept in sync with the live router by the `compat_catalog_sync` test in `cairn-api`.

Request/response body shapes are still being backfilled per subject. Each row currently lists method, path, classification, and the minimum-contract note from the TSV.

## Subjects

| File | Routes | Description |
|---|---:|---|
| [`admin.md`](admin.md) | 58 | Tenant, workspace, license, credential, retention, audit-log, snapshot/restore, capability, and notification administration — plus bearer-token lifecycle under `/v1/auth/tokens/*`. |
| [`approvals.md`](approvals.md) | 10 | Human-in-the-loop approval queue: list pending, approve/deny/reject/delegate/resolve, and policy management. |
| [`channels.md`](channels.md) | 19 | Outbound messaging: notification channels (Slack/email/webhook), mailbox, signals subscriptions, operator notifications, and the activity feed. |
| [`decisions.md`](decisions.md) | 10 | RFC 019 decision pipeline: evaluate, cache inspection, invalidate (by id / bulk / by rule), and the policy engine (`/v1/policies*`). |
| [`evals.md`](evals.md) | 29 | Eval datasets, rubrics, runs, baselines, scorecards, matrices (guardrail, memory-quality, permissions, prompt-comparison, provider-routing, skill-health), and dashboard. |
| [`events.md`](events.md) | 11 | Server-sent event streams (`/v1/stream`, `/v1/streams/runtime`, `/v1/sqeq/events`), raw event log reads and append, WebSocket upgrade, SQ/EQ command plane (RFC 021), and A2A task submission/status (RFC 021). |
| [`graph.md`](graph.md) | 10 | Provenance and execution-graph queries: dependency-path, execution-trace, multi-hop, prompt-provenance, retrieval-provenance, subgraph trace, and raw request traces. |
| [`health.md`](health.md) | 23 | Liveness, readiness, Prometheus scrape targets, OpenAPI/Swagger JSON, version, system info, and other unauthenticated or meta-surface endpoints. |
| [`integrations.md`](integrations.md) | 19 | Integration plugin CRUD, per-integration overrides, webhook receivers (generic and GitHub-specific), and GitHub installation / scan / queue-management endpoints. |
| [`memory.md`](memory.md) | 30 | Knowledge-plane surface: memory documents, vector search, provenance, versions, deep-search, embed, feedback, ingest jobs, source connections, chunks, quality, refresh scheduling, and the UI-facing `/v1/memories` collection. |
| [`platform.md`](platform.md) | 51 | Deployment-wide settings, operator configuration, assistant/chat, bundles (import/export/apply/plan/validate), import/export, onboarding, dashboards, overview, skills, templates, agent-templates, soul, fleet, checkpoints, poll, test, and costs. |
| [`plugins.md`](plugins.md) | 17 | Plugin lifecycle: list/detail, install/uninstall, credentials, eval-score, verify, per-project activation, capabilities, health, logs, metrics, pending-signals, tool inventory, and catalog search. |
| [`projects.md`](projects.md) | 17 | Project-scoped sub-resources: repos, run-templates, triggers (enable/disable/resume), and plugin activation. |
| [`prompts.md`](prompts.md) | 16 | Prompt asset registry, versioning, render, release lifecycle (rollout / activate / rollback / transition / approval), and history. |
| [`providers.md`](providers.md) | 35 | LLM provider connections, bindings, pools, policies, budget, health scheduling, Ollama adapter, and cost-ranking. |
| [`runs.md`](runs.md) | 40 | Agent run lifecycle: create/list/detail, orchestration, checkpoint, resume, cancel, SLA, cost alerts, intervention, replay, diagnose, and queue visibility (stalled / escalated / resume-due / sla-breached). |
| [`sessions.md`](sessions.md) | 11 | Session aggregate: list/detail, active-runs, activity, cost, events, export, LLM traces, and child runs. |
| [`tasks.md`](tasks.md) | 16 | Unit-of-work queue used by runs and workers. |
| [`tools.md`](tools.md) | 6 | Tool-call audit surface: list, detail, progress, create, cancel, complete. |
| [`workers.md`](workers.md) | 8 | Worker registration, claim, heartbeat, report, suspend, and reactivate. |

**Total routes:** 436

## Classification legend

- **Preserve** — stable contract, covered by compat tests, safe for clients.
- **Transitional** — shipping but may evolve before 1.0; breaking changes announced in CHANGELOG.
- **IntentionallyBroken** — deprecated / stub; still routed to avoid 404 churn but not part of the supported surface.

## Coverage

The test `cairn-api::tests::api_docs_coverage` asserts that every route in `http_routes.tsv` is documented in exactly one file under `docs/api/`. See `crates/cairn-api/tests/api_docs_coverage.rs`.
