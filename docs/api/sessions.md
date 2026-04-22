# Sessions

Session aggregate: list/detail, active-runs, activity, cost, events, export, LLM traces, and child runs. A session groups a series of runs under one conversation or workflow.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 11**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/sessions` | Preserve |  |
| `POST` | `/v1/sessions` | Preserve |  |
| `GET` | `/v1/sessions/:id` | Preserve |  |
| `GET` | `/v1/sessions/:id/active-runs` | Preserve |  |
| `GET` | `/v1/sessions/:id/activity` | Preserve |  |
| `GET` | `/v1/sessions/:id/cost` | Preserve |  |
| `GET` | `/v1/sessions/:id/events` | Preserve |  |
| `GET` | `/v1/sessions/:id/export` | Preserve |  |
| `GET` | `/v1/sessions/:id/llm-traces` | Preserve | path param: id; LLM call traces for session |
| `GET` | `/v1/sessions/:id/runs` | Preserve |  |
| `POST` | `/v1/sessions/import` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
