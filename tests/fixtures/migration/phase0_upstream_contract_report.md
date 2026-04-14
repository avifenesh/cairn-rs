# Phase 0 Upstream Contract Report

Status: generated  
Purpose: confirm that the preserved Phase 0 HTTP/SSE fixture set is backed by the upstream frontend and protocol contract even when direct server-side captures are not available locally.

Current reading:

- the local `../cairn-sdk` checkout currently exposes preserved `/v1/*` contract evidence through frontend usage and protocol docs
- Worker 1 did not find a concrete legacy backend handler surface for these routes/events in the local checkout, so this report is intentionally protocol-backed
- if direct handler captures become available later, they should tighten these fixtures rather than replace the compatibility contract casually

## HTTP Evidence

| Requirement | Base Route | Upstream Evidence |
|---|---|---|
| `GET /v1/feed?limit=20&unread=true` | `GET /v1/feed` | `frontend_client, frontend_brief, server_protocol_doc` |
| `GET /v1/tasks?status=running&type=agent` | `GET /v1/tasks` | `frontend_client, frontend_brief, server_protocol_doc` |
| `GET /v1/approvals?status=pending` | `GET /v1/approvals` | `frontend_client, frontend_brief, server_protocol_doc` |
| `GET /v1/memories/search?q=test&limit=10` | `GET /v1/memories/search` | `frontend_client, frontend_brief` |
| `POST /v1/assistant/message body={message,mode?,sessionId?}` | `POST /v1/assistant/message` | `frontend_client, frontend_brief, server_protocol_doc` |
| `POST /v1/assistant/message body={message,mode?}` | `POST /v1/assistant/message` | `frontend_client, frontend_brief, server_protocol_doc` |
| `GET /v1/stream?lastEventId=<id>` | `GET /v1/stream` | `frontend_brief, server_protocol_doc` |

## SSE Evidence

| Event | Upstream Evidence |
|---|---|
| `ready` | `frontend_sse_store, frontend_brief` |
| `feed_update` | `frontend_sse_store, frontend_brief` |
| `poll_completed` | `frontend_sse_store, frontend_brief` |
| `task_update` | `frontend_sse_store, frontend_brief` |
| `approval_required` | `frontend_sse_store, frontend_brief` |
| `assistant_delta` | `frontend_sse_store, frontend_brief` |
| `assistant_end` | `frontend_sse_store, frontend_brief` |
| `assistant_reasoning` | `frontend_sse_store, frontend_brief` |
| `assistant_tool_call` | `frontend_sse_store, frontend_brief` |
| `memory_proposed` | `frontend_sse_store, frontend_brief` |
| `agent_progress` | `frontend_sse_store, frontend_brief` |
