# Compatibility Route and SSE Catalog

Status: draft  
Reference surface: local `../cairn/frontend/src/lib/api/client.ts` and `../cairn/frontend/src/lib/stores/sse.svelte.ts`  
Purpose: make Phase 0 compatibility concrete enough for parallel runtime/API workers

## Rule

This catalog covers the UI-referenced route and SSE surface from the current Cairn frontend.

Each surface is tagged:

- preserve
- transitional
- intentionally break

Preserve means preserve the operator-facing contract closely enough for the existing UI to function during migration.

This document is intentionally based on the current local Cairn frontend contract, not a fetched alternate branch.

## HTTP Route Catalog

| Method | Route | Classification | Minimum contract relied on by current UI |
|---|---|---|---|
| `GET` | `/health` | Preserve | `{ ok: boolean }` |
| `GET` | `/v1/dashboard` | Preserve | dashboard payload used by overview |
| `GET` | `/v1/feed` | Preserve | `{ items, hasMore }` |
| `POST` | `/v1/feed/:id/read` | Preserve | `{ ok }` |
| `POST` | `/v1/feed/read-all` | Preserve | `{ changed }` |
| `GET` | `/v1/tasks` | Preserve | `{ items, hasMore }` |
| `POST` | `/v1/tasks/:id/cancel` | Preserve | `{ ok }` |
| `GET` | `/v1/approvals` | Preserve | `{ items, hasMore }` |
| `POST` | `/v1/approvals/:id/approve` | Preserve | `{ ok }` |
| `POST` | `/v1/approvals/:id/deny` | Preserve | `{ ok }` |
| `GET` | `/v1/assistant/sessions` | Preserve | `{ items }` |
| `GET` | `/v1/assistant/sessions/:sessionId` | Preserve | `{ items }` chat messages |
| `POST` | `/v1/assistant/message` | Preserve | request `{ message, mode?, sessionId? }`, response `{ taskId }` |
| `POST` | `/v1/assistant/voice` | Transitional | multipart upload, returns `{ taskId, transcript }` |
| `GET` | `/v1/memories` | Preserve | `{ items, hasMore }` |
| `GET` | `/v1/memories/search` | Preserve | `{ items }` |
| `POST` | `/v1/memories` | Preserve | create memory body `{ content, category }` |
| `POST` | `/v1/memories/:id/accept` | Preserve | `{ ok }` |
| `POST` | `/v1/memories/:id/reject` | Preserve | `{ ok }` |
| `GET` | `/v1/fleet` | Transitional | `{ agents, summary }` |
| `GET` | `/v1/skills` | Preserve | `{ items, summary, currentlyActive? }` |
| `GET` | `/v1/soul` | Transitional | current singleton asset wrapper |
| `PUT` | `/v1/soul` | Transitional | request `{ content }`, response `{ ok, sha }` |
| `GET` | `/v1/soul/history` | Transitional | `{ items }` |
| `GET` | `/v1/soul/patches` | Transitional | `{ items }` |
| `GET` | `/v1/costs` | Preserve | cost summary payload |
| `GET` | `/v1/metrics` | Preserve | metrics read model |
| `GET` | `/v1/status` | Preserve | runtime/system status |
| `POST` | `/v1/poll/run` | Preserve | `{ ok }` |
| `GET` | `/v1/stream` | Preserve | SSE stream with `token` and `lastEventId` support |

## SSE Event Catalog

Current UI-referenced event names from `/v1/stream`:

| Event | Classification | Minimum payload contract relied on by current UI |
|---|---|---|
| `ready` | Preserve | `{ clientId }` |
| `feed_update` | Preserve | `{ item }` or feed item object |
| `poll_completed` | Preserve | `{ source, newCount }` |
| `task_update` | Preserve | `{ task }` or task object |
| `approval_required` | Preserve | `{ approval }` or approval object |
| `assistant_delta` | Preserve | `{ taskId, deltaText }` |
| `assistant_end` | Preserve | `{ taskId, messageText }` |
| `assistant_reasoning` | Preserve | `{ taskId, round, thought }` |
| `assistant_tool_call` | Preserve | `{ taskId, toolName, phase, args?, result? }` |
| `memory_proposed` | Preserve | current UI only needs `memory.content` for notification text |
| `memory_accepted` | Preserve | no payload currently consumed |
| `soul_updated` | Transitional | `{ sha }` |
| `digest_ready` | Preserve | no payload currently consumed |
| `coding_session_event` | Transitional | event presence only, no payload currently consumed |
| `agent_progress` | Preserve | `{ agentId, message }` |
| `skill_activated` | Transitional | `{ skillName }` |

## Compatibility Notes

- Preserve semantics first, not exact handler internals.
- The current UI does not rely on fetched-branch events like `session_event`, `subagent_*`, `pr_update`, or `mcp_connection`; those are not part of this local compatibility contract.
- Transitional surfaces must either:
  - have a compatibility wrapper in Rust, or
  - have a documented UI migration path before removal.
- The scoped-asset transition must keep current singleton-asset UIs working long enough for operators to migrate content into tenant/workspace/project assets.

## Known Follow-Ups

- Add golden request/response fixtures for:
  - `GET /v1/dashboard`
  - `GET /v1/tasks`
  - `POST /v1/assistant/message`
  - `GET /v1/memories/search`
  - `GET /v1/status`
- Add golden SSE payload fixtures for:
  - `feed_update`
  - `task_update`
  - `approval_required`
  - `assistant_delta`
  - `assistant_end`
  - `agent_progress`
