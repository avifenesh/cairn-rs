# Admin & Auth

Tenant, workspace, license, credential, retention, audit-log, snapshot/restore, capability, and notification administration — plus bearer-token lifecycle under `/v1/auth/tokens/*`. Most routes here require the admin role (bearer token resolving to `role: admin`).

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 58**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/admin/audit-log` | Preserve | audit log entries |
| `GET` | `/v1/admin/audit-log/:resource_type/:resource_id` | Preserve |  |
| `POST` | `/v1/admin/backup` | Preserve |  |
| `GET` | `/v1/admin/capabilities` | Preserve | feature capability map |
| `GET` | `/v1/admin/entitlements` | Preserve | tenant entitlement set |
| `GET` | `/v1/admin/event-count` | Preserve |  |
| `GET` | `/v1/admin/event-log` | Preserve |  |
| `GET` | `/v1/admin/license` | Preserve | active license record |
| `POST` | `/v1/admin/license/activate` | Preserve | body: { license_key }; { ok } |
| `POST` | `/v1/admin/license/override` | Preserve |  |
| `GET` | `/v1/admin/logs` | Preserve | admin logs |
| `GET` | `/v1/admin/models` | Preserve |  |
| `DELETE` | `/v1/admin/models/:id` | Preserve |  |
| `GET` | `/v1/admin/models/:id` | Preserve |  |
| `PUT` | `/v1/admin/models/:id` | Preserve |  |
| `POST` | `/v1/admin/models/import-litellm` | Preserve |  |
| `POST` | `/v1/admin/notifications/:id/retry` | Preserve |  |
| `GET` | `/v1/admin/notifications/failed` | Preserve | failed notification records |
| `GET` | `/v1/admin/operators/:id/notifications` | Preserve |  |
| `POST` | `/v1/admin/operators/:id/notifications` | Preserve |  |
| `POST` | `/v1/admin/rebuild-projections` | Preserve |  |
| `POST` | `/v1/admin/restore` | Preserve |  |
| `POST` | `/v1/admin/rotate-token` | Preserve |  |
| `POST` | `/v1/admin/rotate-waitpoint-hmac` | Preserve |  |
| `POST` | `/v1/admin/snapshot` | Preserve |  |
| `GET` | `/v1/admin/tenants` | Preserve | { items } |
| `POST` | `/v1/admin/tenants` | Preserve |  |
| `GET` | `/v1/admin/tenants/:id` | Preserve |  |
| `POST` | `/v1/admin/tenants/:id/compact-event-log` | Preserve |  |
| `GET` | `/v1/admin/tenants/:id/overview` | Preserve |  |
| `POST` | `/v1/admin/tenants/:id/restore` | Preserve |  |
| `POST` | `/v1/admin/tenants/:id/snapshot` | Preserve |  |
| `GET` | `/v1/admin/tenants/:id/snapshots` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/apply-retention` | Preserve |  |
| `GET` | `/v1/admin/tenants/:tenant_id/credentials` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/credentials` | Preserve |  |
| `DELETE` | `/v1/admin/tenants/:tenant_id/credentials/:id` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/credentials/rotate-key` | Preserve |  |
| `GET` | `/v1/admin/tenants/:tenant_id/operator-profiles` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/operator-profiles` | Preserve |  |
| `GET` | `/v1/admin/tenants/:tenant_id/quota` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/quota` | Preserve |  |
| `GET` | `/v1/admin/tenants/:tenant_id/retention-policy` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/retention-policy` | Preserve |  |
| `GET` | `/v1/admin/tenants/:tenant_id/workspaces` | Preserve |  |
| `POST` | `/v1/admin/tenants/:tenant_id/workspaces` | Preserve |  |
| `DELETE` | `/v1/admin/tenants/:tenant_id/workspaces/:workspace_id` | Preserve | Soft-delete: archives workspace + cascades archival to children (PR #225). |
| `GET` | `/v1/admin/workspaces` | Preserve | { items } |
| `GET` | `/v1/admin/workspaces/:id/shares` | Preserve |  |
| `POST` | `/v1/admin/workspaces/:id/shares` | Preserve |  |
| `DELETE` | `/v1/admin/workspaces/:id/shares/:share_id` | Preserve |  |
| `GET` | `/v1/admin/workspaces/:workspace_id/members` | Preserve |  |
| `POST` | `/v1/admin/workspaces/:workspace_id/members` | Preserve |  |
| `DELETE` | `/v1/admin/workspaces/:workspace_id/members/:id` | Preserve |  |
| `GET` | `/v1/admin/workspaces/:workspace_id/projects` | Preserve |  |
| `POST` | `/v1/admin/workspaces/:workspace_id/projects` | Preserve |  |
| `GET` | `/v1/auth/tokens` | Preserve |  |
| `POST` | `/v1/auth/tokens` | Preserve |  |
| `DELETE` | `/v1/auth/tokens/:id` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
