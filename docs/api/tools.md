# Tool Invocations

Tool-call audit surface: list, detail, progress, create, cancel, complete. Tool definitions themselves live under `/v1/plugins/*/tools` — see `plugins.md`.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 6**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/tool-invocations` | Preserve | query: run_id?, limit?; { items } |
| `POST` | `/v1/tool-invocations` | Preserve |  |
| `GET` | `/v1/tool-invocations/:id` | Preserve |  |
| `POST` | `/v1/tool-invocations/:id/cancel` | Preserve |  |
| `POST` | `/v1/tool-invocations/:id/complete` | Preserve |  |
| `GET` | `/v1/tool-invocations/:id/progress` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
