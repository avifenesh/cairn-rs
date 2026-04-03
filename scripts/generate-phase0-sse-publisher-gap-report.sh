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
    task_update|approval_required)
      printf 'runtime_mapping_followup_remaining_exact_dedicated_builder_present'
      ;;
    assistant_tool_call)
      printf 'runtime_mapping_followup_remaining_enriched_builder_present'
      ;;
    agent_progress)
      printf 'mapped_with_shaped_payload_exact_current_contract'
      ;;
    poll_completed|assistant_delta|assistant_reasoning)
      printf 'supported_via_dedicated_builder'
      ;;
    assistant_end)
      printf 'supported_via_dedicated_builder_followup_remaining'
      ;;
    feed_update|poll_completed|assistant_delta|assistant_reasoning)
      printf 'supported_via_dedicated_builder'
      ;;
    memory_proposed)
      printf 'no_runtime_or_dedicated_publisher_mapping_yet'
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
      printf 'An exact dedicated task-update builder exists (`build_enriched_task_update_frame(...)`), but the runtime-event path currently published through `shape_event_payload(...)` is still thinner than the preserved fixture contract because it lacks task metadata/read-model fields like `type`, `title`, `description`, `progress`, `createdAt`, and `updatedAt`.'
      ;;
    approval_required)
      printf 'An exact dedicated approval builder exists (`build_enriched_approval_frame(...)`), but the runtime-event path currently published through `shape_event_payload(...)` is still thinner than the preserved fixture contract because it lacks approval metadata/read-model fields like `type`, `title`, `description`, `context`, and `createdAt`.'
      ;;
    assistant_tool_call)
      printf 'The exact start-phase payload exists and there is an enriched builder path (`build_enriched_tool_call_frame(...)`), and the runtime-event completed/failed paths in `shape_event_payload(...)` now preserve `taskId`, `toolName`, and `phase`; the remaining follow-up is richer result/error payload semantics.'
      ;;
    agent_progress)
      printf 'Name is mapped and the current `sse_payloads` builder already matches the preserved minimal `{ agentId, message }` contract used by the frontend fixture; richer progress semantics are a later product concern, not a current contract mismatch.'
      ;;
    feed_update)
      printf 'Covered by `build_feed_update_frame(...)` in `sse_payloads.rs`; the preserved feed item envelope now matches the current string-ID fixture contract.'
      ;;
    poll_completed)
      printf 'Covered by `build_poll_completed_frame(...)` in `sse_payloads.rs`; this SSE family is available through the dedicated polling publisher path rather than runtime-event mapping.'
      ;;
    assistant_delta)
      printf 'Covered by `build_streaming_sse_frame(StreamingOutput::AssistantDelta, ...)`; streaming token updates are available through the dedicated assistant-streaming builder path.'
      ;;
    assistant_end)
      printf 'Covered by `build_streaming_sse_frame(StreamingOutput::AssistantEnd, ...)`, but the builder still emits an empty `messageText` placeholder unless the caller supplies the assembled final reply text.'
      ;;
    assistant_reasoning)
      printf 'Covered by `build_streaming_sse_frame(StreamingOutput::AssistantReasoning, ...)`; reasoning trace updates are available through the dedicated assistant-streaming builder path.'
      ;;
    memory_proposed)
      printf 'Required by preserved frontend SSE contract, but no runtime-event mapping or dedicated non-runtime builder is visible yet in the current Rust API slice.'
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
    task_update|approval_required|assistant_tool_call)
      printf 'expand_shaped_payload_to_preserved_fixture'
      ;;
    agent_progress)
      printf 'keep'
      ;;
    poll_completed|assistant_delta|assistant_reasoning)
      printf 'keep'
      ;;
    feed_update)
      printf 'keep'
      ;;
    assistant_end)
      printf 'pass_assembled_final_message_text_into_streaming_builder'
      ;;
    memory_proposed)
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
  printf -- '- `supported_via_dedicated_builder`: covered by a dedicated non-runtime builder path (feed, poll, or assistant streaming)\n'
  printf -- '- `supported_via_dedicated_builder_followup_remaining`: a dedicated builder path exists, but one preserved payload field still needs to be populated correctly\n'
  printf -- '- `runtime_mapping_followup_remaining_exact_dedicated_builder_present`: an exact dedicated builder already exists, but the generic runtime-event mapping path is still thinner than the preserved fixture contract\n'
  printf -- '- `runtime_mapping_followup_remaining_enriched_builder_present`: an enriched builder exists for this family, but the generic runtime-event mapping path still drops preserved phase/result detail\n'
  printf -- '- `mapped_with_shaped_payload_exact_current_contract`: event name is mapped and the current shaped payload already matches the preserved contract that the Phase 0 fixtures exercise today\n'
  printf -- '- `no_runtime_or_dedicated_publisher_mapping_yet`: preserved SSE event exists in the frontend contract, but no equivalent runtime or dedicated non-runtime publisher mapping is visible yet in the current Rust source\n\n'

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
