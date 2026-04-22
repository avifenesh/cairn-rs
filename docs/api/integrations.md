# Integrations & Webhooks

Integration plugin CRUD, per-integration overrides, webhook receivers (generic and GitHub-specific), and GitHub installation / scan / queue-management endpoints.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 19**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/integrations` | Preserve |  |
| `POST` | `/v1/integrations` | Preserve |  |
| `DELETE` | `/v1/integrations/:integration_id` | Preserve |  |
| `GET` | `/v1/integrations/:integration_id` | Preserve |  |
| `DELETE` | `/v1/integrations/:integration_id/overrides` | Preserve |  |
| `GET` | `/v1/integrations/:integration_id/overrides` | Preserve |  |
| `PUT` | `/v1/integrations/:integration_id/overrides` | Preserve |  |
| `POST` | `/v1/webhooks/:integration_id` | Preserve |  |
| `GET` | `/v1/webhooks/github/actions` | Preserve |  |
| `PUT` | `/v1/webhooks/github/actions` | Preserve |  |
| `GET` | `/v1/webhooks/github/installations` | Preserve |  |
| `GET` | `/v1/webhooks/github/queue` | Preserve |  |
| `POST` | `/v1/webhooks/github/queue/:issue/retry` | Preserve |  |
| `POST` | `/v1/webhooks/github/queue/:issue/skip` | Preserve |  |
| `PUT` | `/v1/webhooks/github/queue/concurrency` | Preserve |  |
| `POST` | `/v1/webhooks/github/queue/pause` | Preserve |  |
| `POST` | `/v1/webhooks/github/queue/resume` | Preserve |  |
| `POST` | `/v1/webhooks/github/scan` | Preserve |  |
| `POST` | `/v1/webhooks/github/webhook` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
