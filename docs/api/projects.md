# Projects

Project-scoped sub-resources: repos, run-templates, triggers (enable/disable/resume), and plugin activation. Project CRUD itself lives under `/v1/admin/workspaces/:ws/projects` (see `admin.md`).

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 18**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `DELETE` | `/v1/projects/:proj/plugins/:id` | Preserve |  |
| `POST` | `/v1/projects/:proj/plugins/:id` | Preserve |  |
| `DELETE` | `/v1/projects/:project/local-paths` | Preserve | Detach a `host=local_fs` repo; body `{path}`. |
| `GET` | `/v1/projects/:project/repos` | Preserve |  |
| `POST` | `/v1/projects/:project/repos` | Preserve | `host` defaults to `"github"`; `local_fs` accepts an absolute path; `gitlab | gitea | confluence` return 501. |
| `DELETE` | `/v1/projects/:project/repos/:owner/:repo` | Preserve |  |
| `GET` | `/v1/projects/:project/repos/:owner/:repo` | Preserve |  |
| `GET` | `/v1/projects/:project/run-templates` | Preserve |  |
| `POST` | `/v1/projects/:project/run-templates` | Preserve |  |
| `DELETE` | `/v1/projects/:project/run-templates/:template_id` | Preserve |  |
| `GET` | `/v1/projects/:project/run-templates/:template_id` | Preserve |  |
| `GET` | `/v1/projects/:project/triggers` | Preserve |  |
| `POST` | `/v1/projects/:project/triggers` | Preserve |  |
| `DELETE` | `/v1/projects/:project/triggers/:trigger_id` | Preserve |  |
| `GET` | `/v1/projects/:project/triggers/:trigger_id` | Preserve |  |
| `POST` | `/v1/projects/:project/triggers/:trigger_id/disable` | Preserve |  |
| `POST` | `/v1/projects/:project/triggers/:trigger_id/enable` | Preserve |  |
| `POST` | `/v1/projects/:project/triggers/:trigger_id/resume` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
