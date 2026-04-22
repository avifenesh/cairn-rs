# Platform (Settings, Config, Assistant, Bundles, Onboarding, Misc)

Deployment-wide settings, operator configuration, assistant/chat, bundles (import/export/apply/plan/validate), import/export, onboarding, dashboards, overview, skills, templates, agent-templates, soul, fleet, checkpoints, poll, and test. Catch-all for surfaces that don't fit a single domain bucket.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 51**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/agent-templates` | Preserve |  |
| `POST` | `/v1/agent-templates/:id/instantiate` | Preserve |  |
| `POST` | `/v1/assistant/message` | Preserve | body: { message, mode?, sessionId? }; { taskId } |
| `GET` | `/v1/assistant/sessions` | Preserve | { items } |
| `GET` | `/v1/assistant/sessions/:sessionId` | Preserve | path param: sessionId; { items } chat messages |
| `POST` | `/v1/assistant/voice` | Transitional | multipart: audio, mode?, sessionId?; { taskId, transcript } |
| `POST` | `/v1/bundles/apply` | Preserve |  |
| `GET` | `/v1/bundles/export` | Preserve |  |
| `POST` | `/v1/bundles/export-filtered` | Preserve |  |
| `GET` | `/v1/bundles/export/prompts` | Preserve |  |
| `POST` | `/v1/bundles/import` | Preserve |  |
| `POST` | `/v1/bundles/plan` | Preserve |  |
| `POST` | `/v1/bundles/validate` | Preserve |  |
| `POST` | `/v1/chat/stream` | Preserve |  |
| `GET` | `/v1/checkpoints` | Preserve | query: run_id; { items } |
| `GET` | `/v1/checkpoints/:id` | Preserve |  |
| `POST` | `/v1/checkpoints/:id/restore` | Preserve |  |
| `GET` | `/v1/config` | Preserve | configuration key-value pairs |
| `DELETE` | `/v1/config/:key` | Preserve | path param: key; { ok } |
| `GET` | `/v1/config/:key` | Preserve | path param: key; single config value |
| `PUT` | `/v1/config/:key` | Preserve | path param: key, body: value; { ok } |
| `GET` | `/v1/dashboard` | Preserve | query: limit?, source?; dashboard payload used by overview |
| `GET` | `/v1/dashboard/activity` | Preserve |  |
| `GET` | `/v1/entitlements` | Preserve |  |
| `GET` | `/v1/entitlements/usage` | Preserve |  |
| `GET` | `/v1/export/:format` | Preserve | path param: format; exported bundle |
| `GET` | `/v1/fleet` | Transitional | { agents, summary } |
| `POST` | `/v1/import/apply` | Preserve | body: import payload; { report } |
| `POST` | `/v1/import/preview` | Preserve | body: import payload; { plan, conflicts[] } |
| `GET` | `/v1/import/reports` | Preserve | query: limit?; { items } |
| `POST` | `/v1/import/validate` | Preserve | body: import payload; { valid, errors[] } |
| `GET` | `/v1/onboarding/status` | Preserve | onboarding progress |
| `POST` | `/v1/onboarding/template` | Preserve |  |
| `GET` | `/v1/onboarding/templates` | Preserve | { items } |
| `GET` | `/v1/overview` | Preserve | operator overview dashboard |
| `POST` | `/v1/poll/run` | Preserve | { ok } |
| `GET` | `/v1/settings` | Preserve | deployment settings |
| `DELETE` | `/v1/settings/defaults/:scope/:scope_id/:key` | Preserve |  |
| `PUT` | `/v1/settings/defaults/:scope/:scope_id/:key` | Preserve |  |
| `GET` | `/v1/settings/defaults/all` | Preserve |  |
| `GET` | `/v1/settings/defaults/resolve/:key` | Preserve |  |
| `GET` | `/v1/settings/tls` | Preserve | TLS config |
| `GET` | `/v1/skills` | Preserve | { items, summary, currentlyActive? } |
| `GET` | `/v1/soul` | Transitional | current singleton asset wrapper |
| `PUT` | `/v1/soul` | Transitional | body: { content }; { ok, sha } |
| `GET` | `/v1/soul/history` | Transitional | { items } |
| `GET` | `/v1/soul/patches` | Transitional | { items } |
| `GET` | `/v1/templates` | Preserve |  |
| `GET` | `/v1/templates/:id` | Preserve |  |
| `POST` | `/v1/templates/:id/apply` | Preserve |  |
| `POST` | `/v1/test/webhook` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
