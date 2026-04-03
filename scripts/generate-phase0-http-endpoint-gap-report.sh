#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/tests/fixtures/migration/phase0_http_endpoint_gap_report.md"
REQ_FILE="$ROOT/tests/compat/phase0_required_http.txt"
ENDPOINTS_FILE="$ROOT/crates/cairn-api/src/endpoints.rs"
FEED_FILE="$ROOT/crates/cairn-api/src/feed.rs"
MEMORY_FILE="$ROOT/crates/cairn-api/src/memory_api.rs"
ASSISTANT_FILE="$ROOT/crates/cairn-api/src/assistant.rs"
SSE_PUBLISHER_FILE="$ROOT/crates/cairn-api/src/sse_publisher.rs"

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
}

require_file "$REQ_FILE"
require_file "$ENDPOINTS_FILE"
require_file "$FEED_FILE"
require_file "$MEMORY_FILE"
require_file "$ASSISTANT_FILE"
require_file "$SSE_PUBLISHER_FILE"

status_for_requirement() {
  case "$1" in
    "GET /v1/tasks?status=running&type=agent")
      printf 'read_endpoint_trait_present'
      ;;
    "GET /v1/approvals?status=pending")
      printf 'read_endpoint_trait_present'
      ;;
    "GET /v1/feed?limit=20&unread=true")
      printf 'dedicated_endpoint_trait_present'
      ;;
    "GET /v1/memories/search?q=test&limit=10")
      printf 'dedicated_endpoint_trait_present_followup_remaining'
      ;;
    "POST /v1/assistant/message body={message,mode?,sessionId?}"|"POST /v1/assistant/message body={message,mode?}")
      printf 'dedicated_endpoint_trait_present'
      ;;
    "GET /v1/stream?lastEventId=<id>")
      printf 'stream_publisher_present_followup_remaining'
      ;;
    *)
      printf 'unclassified'
      ;;
  esac
}

notes_for_requirement() {
  case "$1" in
    "GET /v1/tasks?status=running&type=agent")
      printf '`RuntimeReadEndpoints::list_tasks` exists and already uses the shared `ListQuery` boundary.'
      ;;
    "GET /v1/approvals?status=pending")
      printf '`RuntimeReadEndpoints::list_approvals` exists and already uses the shared `ListQuery` boundary.'
      ;;
    "GET /v1/stream?lastEventId=<id>")
      printf '`SsePublisher`, `build_sse_frame`, and `parse_last_event_id` exist, but preserved SSE payload-shape alignment is still an explicit follow-up.'
      ;;
    "GET /v1/feed?limit=20&unread=true")
      printf '`FeedEndpoints::list` plus read-marking boundaries exist in `feed.rs`, and the current `ListResponse<FeedItem>` shape matches the preserved feed fixture contract.'
      ;;
    "GET /v1/memories/search?q=test&limit=10")
      printf '`MemoryEndpoints::search` exists in `memory_api.rs`, but the current `MemoryItem` response shape is still thinner than the preserved fixture contract (missing preserved `source` / `confidence`, and `createdAt` is not yet aligned to the fixture format).'
      ;;
    "POST /v1/assistant/message body={message,mode?,sessionId?}")
      printf '`AssistantEndpoints::send_message` exists in `assistant.rs`, so the preserved assistant-message mutation now has an explicit Rust-side command boundary.'
      ;;
    "POST /v1/assistant/message body={message,mode?}")
      printf '`AssistantEndpoints::send_message` exists in `assistant.rs`, so the preserved assistant-message mutation now has an explicit Rust-side command boundary.'
      ;;
    *)
      printf 'No note recorded.'
      ;;
  esac
}

next_step_for_requirement() {
  case "$1" in
    "GET /v1/tasks?status=running&type=agent"|"GET /v1/approvals?status=pending")
      printf 'keep_contract_stable'
      ;;
    "POST /v1/assistant/message body={message,mode?,sessionId?}"|"POST /v1/assistant/message body={message,mode?}")
      printf 'keep_contract_stable'
      ;;
    "GET /v1/feed?limit=20&unread=true")
      printf 'keep_contract_stable'
      ;;
    "GET /v1/memories/search?q=test&limit=10")
      printf 'expand_memory_search_response_shape_to_preserved_fixture'
      ;;
    "GET /v1/stream?lastEventId=<id>")
      printf 'align_sse_payload_shape'
      ;;
    *)
      printf 'classify'
      ;;
  esac
}

{
  printf '# Phase 0 HTTP Endpoint Gap Report\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: show how the current Rust API endpoint surface relates to the preserved Phase 0 HTTP contract.\n\n'
  printf 'Current reading:\n\n'
  printf -- '- this report is based on the current `cairn-api/src/endpoints.rs` service boundary plus the preserved Phase 0 HTTP requirement set\n'
  printf -- '- it complements the route catalog and fixture checks by showing which preserved routes already have explicit Rust-side endpoint boundaries and which still exist only as compatibility inventory + fixtures\n'
  printf -- '- Worker 1 should use this report to keep API-surface drift visible while Worker 8 expands product endpoints intentionally\n\n'
  printf 'Interpretation:\n\n'
  printf -- '- `read_endpoint_trait_present`: a Rust-side read endpoint/service seam already exists for the preserved route family\n'
  printf -- '- `dedicated_endpoint_trait_present`: a dedicated preserved-route endpoint or mutation trait exists outside the generic runtime read boundary\n'
  printf -- '- `dedicated_endpoint_trait_present_followup_remaining`: the route seam exists, but the current serialized request or response shape is still thinner than the preserved fixture contract\n'
  printf -- '- `stream_publisher_present_followup_remaining`: the stream surface exists, but compatibility work remains before it is locked\n'
  printf -- '- `no_explicit_api_boundary_yet`: preserved route exists in the catalog and fixtures, but no dedicated Rust-side endpoint/mutation seam is visible yet\n\n'

  printf '## Phase 0 HTTP Status\n\n'
  printf '| Requirement | Current Status | Notes | Next Step |\n'
  printf '|---|---|---|---|\n'
  while IFS= read -r requirement; do
    [[ -z "$requirement" ]] && continue
    printf '| `%s` | `%s` | %s | `%s` |\n' \
      "$requirement" \
      "$(status_for_requirement "$requirement")" \
      "$(notes_for_requirement "$requirement")" \
      "$(next_step_for_requirement "$requirement")"
  done < "$REQ_FILE"
} > "$OUT"

echo "generated $OUT"
