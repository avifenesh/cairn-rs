# Phase 0 Mismatch Report

Status: generated  
Purpose: track preserved-surface fixture readiness and the gap between seeded fixtures and direct backend captures

Interpretation:

- `seeded_fixture_present`
  - a preserved fixture exists, but it is still seeded from frontend/protocol contracts until direct backend capture confirms it
- `missing_fixture`
  - required compatibility coverage is absent and must be added before Phase 0 is complete

This report does not yet assert semantic parity with the Rust backend.

It tracks whether Worker 1 has a concrete comparison surface for the preserved Phase 0 HTTP and SSE set.

## HTTP Preserved Set

| Requirement | Fixture | Status | Next Step |
|---|---|---|---|
| `GET /v1/feed?limit=20&unread=true` | [`tests/fixtures/http/GET__v1_feed__limit20_unread_true.json`](../http/GET__v1_feed__limit20_unread_true.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `GET /v1/tasks?status=running&type=agent` | [`tests/fixtures/http/GET__v1_tasks__status_running_type_agent.json`](../http/GET__v1_tasks__status_running_type_agent.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `GET /v1/approvals?status=pending` | [`tests/fixtures/http/GET__v1_approvals__status_pending.json`](../http/GET__v1_approvals__status_pending.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `GET /v1/memories/search?q=test&limit=10` | [`tests/fixtures/http/GET__v1_memories_search__q_test_limit_10.json`](../http/GET__v1_memories_search__q_test_limit_10.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `POST /v1/assistant/message body={message,mode?,sessionId?}` | [`tests/fixtures/http/POST__v1_assistant_message__with_session.json`](../http/POST__v1_assistant_message__with_session.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `POST /v1/assistant/message body={message,mode?}` | [`tests/fixtures/http/POST__v1_assistant_message__without_session.json`](../http/POST__v1_assistant_message__without_session.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `GET /v1/stream?lastEventId=<id>` | [`tests/fixtures/http/GET__v1_stream__replay_from_last_event_id.json`](../http/GET__v1_stream__replay_from_last_event_id.json) | `seeded_fixture_present` | `confirm_against_direct_stream_resume_behavior` |

## SSE Preserved Set

| Event | Fixture | Status | Next Step |
|---|---|---|---|
| `ready` | [`tests/fixtures/sse/ready__connected.json`](../sse/ready__connected.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `feed_update` | [`tests/fixtures/sse/feed_update__single_item.json`](../sse/feed_update__single_item.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `poll_completed` | [`tests/fixtures/sse/poll_completed__source_done.json`](../sse/poll_completed__source_done.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `task_update` | [`tests/fixtures/sse/task_update__running_task.json`](../sse/task_update__running_task.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `approval_required` | [`tests/fixtures/sse/approval_required__pending.json`](../sse/approval_required__pending.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `assistant_delta` | [`tests/fixtures/sse/assistant_delta__incremental_reply.json`](../sse/assistant_delta__incremental_reply.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `assistant_end` | [`tests/fixtures/sse/assistant_end__complete_reply.json`](../sse/assistant_end__complete_reply.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `assistant_reasoning` | [`tests/fixtures/sse/assistant_reasoning__round_1.json`](../sse/assistant_reasoning__round_1.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `assistant_tool_call` | [`tests/fixtures/sse/assistant_tool_call__start.json`](../sse/assistant_tool_call__start.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `memory_proposed` | [`tests/fixtures/sse/memory_proposed__proposal.json`](../sse/memory_proposed__proposal.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |
| `agent_progress` | [`tests/fixtures/sse/agent_progress__message.json`](../sse/agent_progress__message.json) | `seeded_fixture_present` | `replace_or_confirm_with_direct_backend_capture` |

## Current Reading

- The minimum preserved Phase 0 set now has seeded fixtures for every required HTTP and SSE surface.
- The next Worker 1 task is to replace or confirm these seeded fixtures with direct backend captures from `../cairn` where possible.
- Any later mismatch between Rust behavior and these fixtures should be classified as:
  - preserve bug
  - intentional break
  - transitional surface

