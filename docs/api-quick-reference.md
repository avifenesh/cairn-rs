# Cairn API Quick Reference

All routes are prefixed with the server base URL. Routes marked **†** are transitional and may change before v1 GA.

## System

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness check — returns `{ ok: true }` |
| `GET` | `/v1/status` | Runtime and system status |
| `GET` | `/v1/metrics` | Aggregated runtime metrics |
| `GET` | `/v1/dashboard` | Operator dashboard payload |
| `GET` | `/v1/overview` | Canonical operator entry point (RFC 010) |

## Streaming & Polling

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/stream` | SSE event stream with replay support (`?lastEventId=`) |
| `POST` | `/v1/poll/run` | Long-poll for run state changes |

## Feed

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/feed` | Activity feed for the current operator |
| `POST` | `/v1/feed/:id/read` | Mark a single feed item as read |
| `POST` | `/v1/feed/read-all` | Mark all feed items as read |

## Assistant

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/assistant/sessions` | List assistant sessions |
| `GET` | `/v1/assistant/sessions/:sessionId` | Get session messages |
| `POST` | `/v1/assistant/message` | Send a message (text) |
| `POST` | `/v1/assistant/voice` | Send a voice message (multipart) **†** |

## Runs

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/runs` | List runs (`?session_id=`, `?limit=`) |
| `GET` | `/v1/runs/:id` | Get run detail with linked tasks |
| `POST` | `/v1/runs/process-scheduled-resumes` | Trigger scheduled run resumes |
| `GET` | `/v1/sessions/:id/llm-traces` | LLM call traces for a session |

## Tasks

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/tasks` | List tasks (`?status=`, `?type=`) |
| `POST` | `/v1/tasks/:id/cancel` | Cancel a task |
| `POST` | `/v1/tasks/expire-leases` | Expire stale task leases (internal) |

## Approvals

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/approvals` | Approval inbox (`?status=`) |
| `POST` | `/v1/approvals/:id/approve` | Approve a pending approval |
| `POST` | `/v1/approvals/:id/deny` | Deny a pending approval |
| `POST` | `/v1/approval-policies` | Create an approval policy |

## Memory / Knowledge

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/memories` | List knowledge memories |
| `GET` | `/v1/memories/search` | Semantic search (`?q=`, `?limit=`) |
| `POST` | `/v1/memories` | Create a memory |
| `POST` | `/v1/memories/:id/accept` | Accept a suggested memory |
| `POST` | `/v1/memories/:id/reject` | Reject a suggested memory |
| `POST` | `/v1/memory/ingest` | Submit content for ingestion |
| `POST` | `/v1/memory/deep-search` | Deep search across corpora |
| `GET` | `/v1/sources` | List signal sources |
| `POST` | `/v1/sources` | Register a source |
| `POST` | `/v1/sources/process-refresh` | Trigger source refresh cycle |
| `GET` | `/v1/channels` | List channels |
| `POST` | `/v1/channels` | Create a channel |

## Ingest Jobs

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/ingest/jobs` | Submit an ingest job |

## Prompts

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/prompts/assets` | List prompt assets |
| `POST` | `/v1/prompts/assets` | Create a prompt asset |
| `GET` | `/v1/prompts/releases` | List prompt releases |
| `POST` | `/v1/prompts/releases` | Create a prompt release |

## Evals

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/evals/runs` | List eval runs |
| `POST` | `/v1/evals/runs` | Start an eval run |
| `GET` | `/v1/evals/datasets` | List eval datasets |
| `POST` | `/v1/evals/datasets` | Create an eval dataset |
| `POST` | `/v1/evals/baselines` | Set an eval baseline |
| `POST` | `/v1/evals/rubrics` | Create an eval rubric |

## Graph & Policies

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/graph/trace` | Execution graph trace (`?root_id=`, `?kind=`) |
| `GET` | `/v1/policies/decisions` | Recent policy decisions |

## Providers

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/providers/health` | Provider connection health |
| `POST` | `/v1/providers/bindings` | Register a provider binding |
| `POST` | `/v1/providers/connections` | Register a provider connection |
| `POST` | `/v1/providers/pools` | Create a provider connection pool |
| `POST` | `/v1/providers/budget` | Set a provider budget |
| `POST` | `/v1/providers/policies` | Create a route policy |
| `POST` | `/v1/providers/run-health-checks` | Trigger provider health checks |

## Tool Invocations & Plugins

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/tool-invocations` | Record a tool invocation |
| `POST` | `/v1/plugins` | Register a plugin |

## Soul (Agent Identity) **†**

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/soul` | Get current agent soul |
| `PUT` | `/v1/soul` | Update agent soul |
| `GET` | `/v1/soul/history` | Soul patch history |
| `GET` | `/v1/soul/patches` | Pending soul patches |
| `GET` | `/v1/skills` | List available skills |
| `GET` | `/v1/fleet` | Worker fleet summary **†** |

## Settings & Config

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/settings` | Deployment settings |
| `GET` | `/v1/config` | All config key-value pairs |
| `GET` | `/v1/config/:key` | Get a single config value |
| `PUT` | `/v1/config/:key` | Set a config value |
| `DELETE` | `/v1/config/:key` | Delete a config value |

## Costs

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/costs` | Cost summary |

## Import / Export (RFC 013)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/import/validate` | Validate a bundle without applying it |
| `POST` | `/v1/import/preview` | Preview the import plan |
| `POST` | `/v1/import/apply` | Apply an import plan |
| `GET` | `/v1/import/reports` | List import reports |
| `GET` | `/v1/export/:format` | Export artifacts (`format`: `json`, `zip`) |

## Onboarding

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/onboarding/template` | Bootstrap from a starter template |

## Admin (RFC 010 / RFC 014)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/admin/tenants` | List tenants |
| `POST` | `/v1/admin/tenants` | Create a tenant |
| `GET` | `/v1/admin/workspaces` | List workspaces |
| `GET` | `/v1/admin/entitlements` | Tenant entitlement set (RFC 014) |
| `GET` | `/v1/admin/capabilities` | Feature capability map (RFC 014) |
| `GET` | `/v1/admin/license` | Active license record (RFC 014) |
| `POST` | `/v1/admin/license/activate` | Activate a license key (RFC 014) |
| `POST` | `/v1/admin/license/override` | Set a per-tenant feature override (RFC 014) |

---

> **†** Transitional — shape may change before v1 GA.  
> All `POST`/`PUT`/`DELETE` requests require `Content-Type: application/json`.  
> Authentication: `Authorization: Bearer <token>` on all `/v1/*` routes.
