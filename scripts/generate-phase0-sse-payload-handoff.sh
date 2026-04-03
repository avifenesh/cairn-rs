#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/tests/fixtures/migration/phase0_sse_payload_handoff.md"
FIXTURE_DIR="$ROOT/tests/fixtures/sse"
PUBLISHER_FILE="$ROOT/crates/cairn-api/src/sse_publisher.rs"

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
}

require_file "$PUBLISHER_FILE"
for file in \
  "$FIXTURE_DIR/ready__connected.json" \
  "$FIXTURE_DIR/task_update__running_task.json" \
  "$FIXTURE_DIR/approval_required__pending.json" \
  "$FIXTURE_DIR/assistant_tool_call__start.json" \
  "$FIXTURE_DIR/agent_progress__message.json" \
  "$FIXTURE_DIR/feed_update__single_item.json" \
  "$FIXTURE_DIR/poll_completed__source_done.json" \
  "$FIXTURE_DIR/assistant_delta__incremental_reply.json" \
  "$FIXTURE_DIR/assistant_end__complete_reply.json" \
  "$FIXTURE_DIR/assistant_reasoning__round_1.json" \
  "$FIXTURE_DIR/memory_proposed__proposal.json"; do
  require_file "$file"
done

runtime_source_for_event() {
  case "$1" in
    ready)
      printf '`build_ready_frame()`'
      ;;
    task_update)
      printf '`TaskCreated | TaskStateChanged | TaskLeaseClaimed | TaskLeaseHeartbeated`'
      ;;
    approval_required)
      printf '`ApprovalRequested`'
      ;;
    assistant_tool_call)
      printf '`ToolInvocationStarted | ToolInvocationCompleted | ToolInvocationFailed`'
      ;;
    agent_progress)
      printf '`ExternalWorkerReported | SubagentSpawned`'
      ;;
    feed_update)
      printf '`no_runtime_mapping_yet`'
      ;;
    poll_completed)
      printf '`no_runtime_mapping_yet`'
      ;;
    assistant_delta)
      printf '`no_runtime_mapping_yet`'
      ;;
    assistant_end)
      printf '`no_runtime_mapping_yet`'
      ;;
    assistant_reasoning)
      printf '`no_runtime_mapping_yet`'
      ;;
    memory_proposed)
      printf '`no_runtime_mapping_yet`'
      ;;
    *)
      printf '`unknown`'
      ;;
  esac
}

expected_payload_shape_for_event() {
  case "$1" in
    ready)
      printf '`{ clientId }`'
      ;;
    task_update)
      printf '`{ task: { id, type, status, title, description, progress, createdAt, updatedAt } }`'
      ;;
    approval_required)
      printf '`{ approval: { id, type, status, title, description, context, createdAt } }`'
      ;;
    assistant_tool_call)
      printf '`{ taskId, toolName, phase, args?, result? }`'
      ;;
    agent_progress)
      printf '`{ agentId, message }`'
      ;;
    feed_update)
      printf '`{ item: { id, source, kind, title, body, url, author, avatarUrl, repoFullName, isRead, isArchived, groupKey, createdAt } }`'
      ;;
    poll_completed)
      printf '`{ source, newCount }`'
      ;;
    assistant_delta)
      printf '`{ taskId, deltaText }`'
      ;;
    assistant_end)
      printf '`{ taskId, messageText }`'
      ;;
    assistant_reasoning)
      printf '`{ taskId, round, thought }`'
      ;;
    memory_proposed)
      printf '`{ memory: { id, category, status, content, source, confidence, createdAt } }`'
      ;;
    *)
      printf '`unknown`'
      ;;
  esac
}

builder_direction_for_event() {
  case "$1" in
    ready)
      printf '`keep_existing_ready_builder`'
      ;;
    task_update)
      printf '`expand_existing_task_update_shaper_fields`'
      ;;
    approval_required)
      printf '`expand_existing_approval_required_shaper_fields`'
      ;;
    assistant_tool_call)
      printf '`expand_existing_assistant_tool_call_shaper_fields`'
      ;;
    agent_progress)
      printf '`tighten_existing_agent_progress_shaper_fields`'
      ;;
    feed_update)
      printf '`decide_non_runtime_or_signal_owner_then_add_builder`'
      ;;
    poll_completed)
      printf '`decide_non_runtime_or_signal_owner_then_add_builder`'
      ;;
    assistant_delta)
      printf '`decide_agent_stream_owner_then_add_builder`'
      ;;
    assistant_end)
      printf '`decide_agent_stream_owner_then_add_builder`'
      ;;
    assistant_reasoning)
      printf '`decide_agent_stream_owner_then_add_builder`'
      ;;
    memory_proposed)
      printf '`decide_memory_owner_then_add_builder`'
      ;;
    *)
      printf '`classify`'
      ;;
  esac
}

status_for_event() {
  case "$1" in
    ready)
      printf '`covered`'
      ;;
    task_update|approval_required|assistant_tool_call|agent_progress)
      printf '`shaped_builder_present_fixture_alignment_remaining`'
      ;;
    feed_update|poll_completed|assistant_delta|assistant_end|assistant_reasoning|memory_proposed)
      printf '`owner_and_builder_missing`'
      ;;
    *)
      printf '`unknown`'
      ;;
  esac
}

exact_followup_for_event() {
  case "$1" in
    ready)
      printf '`none`'
      ;;
    task_update)
      printf '`populate task.type/title/description/progress/createdAt/updatedAt from runtime-owned task metadata or read model`'
      ;;
    approval_required)
      printf '`populate approval.type/title/description/context/createdAt from approval requirement + read-model context`'
      ;;
    assistant_tool_call)
      printf '`preserve args on start, then add stable completed/failed payload semantics for taskId/result/error detail instead of collapsing to invocation id`'
      ;;
    agent_progress)
      printf '`keep minimal agentId/message shape, but replace placeholder subagent text with stable runtime/agent progress semantics`'
      ;;
    feed_update)
      printf '`wire feed publisher to feed/signal aggregation source that can emit full item payloads`'
      ;;
    poll_completed)
      printf '`wire signal/source polling completion path to emit source + newCount through the explicit publisher seam`'
      ;;
    assistant_delta|assistant_end|assistant_reasoning)
      printf '`wait for Worker 7 streaming/output seam, then add explicit publisher builder for the preserved payload family`'
      ;;
    memory_proposed)
      printf '`decide whether this is a memory-service publisher or proposal workflow publisher, then emit the full memory envelope`'
      ;;
    *)
      printf '`classify`'
      ;;
  esac
}

{
  printf '# Phase 0 SSE Payload Handoff\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: give Worker 8 a concrete event-by-event handoff from current runtime publisher inputs to the preserved SSE payload shapes Worker 1 is guarding.\n\n'
  printf 'Current implementation note:\n\n'
  printf -- '- `cairn-api/src/sse_publisher.rs` now routes mapped runtime events through `crate::sse_payloads::shape_event_payload(&stored.envelope.payload)`.\n'
  printf -- '- that is a real compatibility step forward: event names and wrapper families now exist for the mapped runtime surfaces.\n'
  printf -- '- the remaining work is field-level alignment with the preserved frontend fixtures, not raw-event serialization removal.\n\n'
  printf '## Event Handoff Table\n\n'
  printf '| Event | Current Runtime Source | Expected Preserved Payload Shape | Current Status | Suggested Builder Direction | Exact Follow-up |\n'
  printf '|---|---|---|---|---|---|\n'
  for event_name in \
    ready \
    task_update \
    approval_required \
    assistant_tool_call \
    agent_progress \
    feed_update \
    poll_completed \
    assistant_delta \
    assistant_end \
    assistant_reasoning \
    memory_proposed; do
    printf '| `%s` | %s | %s | %s | %s | %s |\n' \
      "$event_name" \
      "$(runtime_source_for_event "$event_name")" \
      "$(expected_payload_shape_for_event "$event_name")" \
      "$(status_for_event "$event_name")" \
      "$(builder_direction_for_event "$event_name")" \
      "$(exact_followup_for_event "$event_name")"
  done
} > "$OUT"

echo "generated $OUT"
