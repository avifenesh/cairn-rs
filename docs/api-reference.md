# API Reference (moved)

The per-endpoint HTTP reference has moved to [`docs/api/`](./api/README.md), split by subject. The canonical route inventory is [`tests/compat/http_routes.tsv`](../tests/compat/http_routes.tsv) — 443 routes, kept in sync with the live router by `cargo test -p cairn-api --test compat_catalog_sync`.

See [`docs/api/README.md`](./api/README.md) for the full index of subject-area files:

- [`admin.md`](./api/admin.md) — Admin & Auth
- [`approvals.md`](./api/approvals.md) — Approvals
- [`channels.md`](./api/channels.md) — Channels, Mailbox, Signals, Feed, Notifications
- [`decisions.md`](./api/decisions.md) — Decisions & Policies
- [`evals.md`](./api/evals.md) — Evaluations
- [`events.md`](./api/events.md) — Events, SSE, and A2A
- [`graph.md`](./api/graph.md) — Graph & Traces
- [`health.md`](./api/health.md) — Health, Metrics, and Service Metadata
- [`integrations.md`](./api/integrations.md) — Integrations & Webhooks
- [`memory.md`](./api/memory.md) — Memory, Sources, and Ingestion
- [`platform.md`](./api/platform.md) — Platform (Settings, Config, Assistant, Bundles, Onboarding, Misc)
- [`plugins.md`](./api/plugins.md) — Plugins
- [`projects.md`](./api/projects.md) — Projects
- [`prompts.md`](./api/prompts.md) — Prompts
- [`providers.md`](./api/providers.md) — Providers
- [`runs.md`](./api/runs.md) — Runs
- [`sessions.md`](./api/sessions.md) — Sessions
- [`tasks.md`](./api/tasks.md) — Tasks
- [`tools.md`](./api/tools.md) — Tool Invocations
- [`workers.md`](./api/workers.md) — External Workers

Request/response body shapes are being backfilled per subject. For the authoritative machine-readable surface today, use `GET /openapi.json` on a running server.
