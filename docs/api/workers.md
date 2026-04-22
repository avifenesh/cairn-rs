# External Workers

Worker registration, claim, heartbeat, report, suspend, and reactivate. Used by out-of-process workers to lease tasks.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 8**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/workers` | Preserve |  |
| `GET` | `/v1/workers/:id` | Preserve |  |
| `POST` | `/v1/workers/:id/claim` | Preserve |  |
| `POST` | `/v1/workers/:id/heartbeat` | Preserve |  |
| `POST` | `/v1/workers/:id/reactivate` | Preserve |  |
| `POST` | `/v1/workers/:id/report` | Preserve |  |
| `POST` | `/v1/workers/:id/suspend` | Preserve |  |
| `POST` | `/v1/workers/register` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
