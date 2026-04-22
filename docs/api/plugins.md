# Plugins

Plugin lifecycle: list/detail, install/uninstall, credentials, eval-score, verify, per-project activation, capabilities, health, logs, metrics, pending-signals, tool inventory, and catalog search.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 17**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/plugins` | Preserve | { items } |
| `POST` | `/v1/plugins` | Preserve |  |
| `DELETE` | `/v1/plugins/:id` | Preserve |  |
| `GET` | `/v1/plugins/:id` | Preserve |  |
| `GET` | `/v1/plugins/:id/capabilities` | Preserve |  |
| `POST` | `/v1/plugins/:id/credentials` | Preserve |  |
| `POST` | `/v1/plugins/:id/eval-score` | Preserve |  |
| `GET` | `/v1/plugins/:id/health` | Preserve |  |
| `POST` | `/v1/plugins/:id/install` | Preserve |  |
| `GET` | `/v1/plugins/:id/logs` | Preserve |  |
| `GET` | `/v1/plugins/:id/metrics` | Preserve |  |
| `GET` | `/v1/plugins/:id/pending-signals` | Preserve |  |
| `GET` | `/v1/plugins/:id/tools` | Preserve |  |
| `DELETE` | `/v1/plugins/:id/uninstall` | Preserve |  |
| `POST` | `/v1/plugins/:id/verify` | Preserve |  |
| `GET` | `/v1/plugins/catalog` | Preserve |  |
| `GET` | `/v1/plugins/tools/search` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
