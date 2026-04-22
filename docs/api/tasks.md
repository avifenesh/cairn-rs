# Tasks

Unit-of-work queue used by runs and workers. Create, claim, complete/fail, heartbeat, lease release, priority, dependencies, batch cancel, and lease expiry sweep.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 16**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/tasks` | Preserve | query: status?, type?; { items, hasMore } |
| `POST` | `/v1/tasks` | Preserve |  |
| `GET` | `/v1/tasks/:id` | Preserve |  |
| `POST` | `/v1/tasks/:id/cancel` | Preserve | path param: id; { ok } |
| `POST` | `/v1/tasks/:id/claim` | Preserve |  |
| `POST` | `/v1/tasks/:id/complete` | Preserve |  |
| `GET` | `/v1/tasks/:id/dependencies` | Preserve |  |
| `POST` | `/v1/tasks/:id/dependencies` | Preserve |  |
| `POST` | `/v1/tasks/:id/fail` | Preserve |  |
| `POST` | `/v1/tasks/:id/heartbeat` | Preserve |  |
| `POST` | `/v1/tasks/:id/priority` | Preserve |  |
| `POST` | `/v1/tasks/:id/release-lease` | Preserve |  |
| `POST` | `/v1/tasks/:id/start` | Preserve |  |
| `POST` | `/v1/tasks/batch/cancel` | Preserve |  |
| `POST` | `/v1/tasks/expire-leases` | Preserve |  |
| `GET` | `/v1/tasks/expired` | Preserve | { items } |

<!-- TODO: contract bodies (tracked as follow-up) -->
