#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/tests/fixtures/migration/phase0_sse_publisher_gap_report.md"
REQ_FILE="$ROOT/tests/compat/phase0_required_sse.txt"
PUBLISHER_FILE="$ROOT/crates/cairn-api/src/sse_publisher.rs"

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
}

require_file "$REQ_FILE"
require_file "$PUBLISHER_FILE"

if ! grep -Fq 'crate::sse_payloads::shape_event_payload(&stored.envelope.payload)' "$PUBLISHER_FILE"; then
  echo "expected shaped payload path not found in sse_publisher.rs" >&2
  exit 1
fi

status_for_event() {
  case "$1" in
    ready)
      printf 'supported_via_ready_frame'
      ;;
    task_update|approval_required|assistant_tool_call|agent_progress)
      printf 'mapped_with_shaped_payload_followup_remaining'
      ;;
    feed_update|poll_completed|assistant_delta|assistant_end|assistant_reasoning|memory_proposed)
      printf 'no_runtime_publisher_mapping_yet'
      ;;
    *)
      printf 'unclassified'
      ;;
  esac
}

notes_for_event() {
  case "$1" in
    ready)
      printf '`build_ready_frame()` covers connection bootstrap with `{ clientId }`.'
      ;;
    task_update)
      printf 'Name is mapped and payload shaping exists, but current `sse_payloads` output is still narrower than the preserved fixture contract (`taskId/state/eventType` instead of the fuller `{ task: { id, type, status, title, description, progress, createdAt, updatedAt } }` shape).'
      ;;
    approval_required)
      printf 'Name is mapped and payload shaping exists, but current `sse_payloads` output only carries `approvalId/runId/taskId` instead of the fuller preserved `{ approval: { id, type, status, title, description, context, createdAt } }` shape.'
      ;;
    assistant_tool_call)
      printf 'Name is mapped and payload shaping exists, but current `sse_payloads` output still needs preserved-field alignment across phases (for example completed/failed events currently collapse toward invocation identifiers instead of preserving the frontend tool-call envelope consistently).'
      ;;
    agent_progress)
      printf 'Name is mapped and payload shaping exists, but current builder still needs frontend-contract tightening for subagent/runtime progress semantics beyond the minimal `{ agentId, message }` fields.'
      ;;
    feed_update)
      printf 'Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`.'
      ;;
    poll_completed)
      printf 'Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`.'
      ;;
    assistant_delta)
      printf 'Required by preserved frontend SSE contract; streaming token events are not yet represented by the current runtime-event publisher mapping.'
      ;;
    assistant_end)
      printf 'Required by preserved frontend SSE contract; final assistant text event is not yet represented by the current runtime-event publisher mapping.'
      ;;
    assistant_reasoning)
      printf 'Required by preserved frontend SSE contract; reasoning trace event is not yet represented by the current runtime-event publisher mapping.'
      ;;
    memory_proposed)
      printf 'Required by preserved frontend SSE contract; no runtime publisher mapping is visible yet in `sse_publisher.rs`.'
      ;;
    *)
      printf 'No note recorded.'
      ;;
  esac
}

next_step_for_event() {
  case "$1" in
    ready)
      printf 'keep'
      ;;
    task_update|approval_required|assistant_tool_call|agent_progress)
      printf 'expand_shaped_payload_to_preserved_fixture'
      ;;
    feed_update|poll_completed|assistant_delta|assistant_end|assistant_reasoning|memory_proposed)
      printf 'decide_runtime_or_non_runtime_publisher_owner'
      ;;
    *)
      printf 'classify'
      ;;
  esac
}

{
  printf '# Phase 0 SSE Publisher Gap Report\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: show how the current Rust SSE publisher surface relates to the preserved Phase 0 SSE contract.\n\n'
  printf 'Current reading:\n\n'
  printf -- '- this report is based on the current `cairn-api/src/sse_publisher.rs` implementation plus the preserved Phase 0 SSE requirement set\n'
  printf -- '- it does not claim backend parity with the legacy Go app; it highlights where the Rust-side runtime publisher is already aligned and where it still needs explicit compatibility work\n'
  printf -- '- Worker 1 should use this report to keep payload-shape drift visible while Worker 8 tightens the SSE publisher\n\n'
  printf 'Interpretation:\n\n'
  printf -- '- `supported_via_ready_frame`: already covered by a dedicated publisher path\n'
  printf -- '- `mapped_with_shaped_payload_followup_remaining`: event name is present and the publisher now uses `sse_payloads`, but the emitted field set still needs alignment with the preserved frontend fixture contract\n'
  printf -- '- `no_runtime_publisher_mapping_yet`: preserved SSE event exists in the frontend contract, but no equivalent runtime-publisher mapping is visible yet in the current Rust source\n\n'

  printf '## Phase 0 SSE Status\n\n'
  printf '| Event | Current Status | Notes | Next Step |\n'
  printf '|---|---|---|---|\n'
  while IFS= read -r event_name; do
    [[ -z "$event_name" ]] && continue
    printf '| `%s` | `%s` | %s | `%s` |\n' \
      "$event_name" \
      "$(status_for_event "$event_name")" \
      "$(notes_for_event "$event_name")" \
      "$(next_step_for_event "$event_name")"
  done < "$REQ_FILE"
} > "$OUT"

echo "generated $OUT"
