#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/tests/fixtures/migration/phase0_http_endpoint_gap_report.md"
REQ_FILE="$ROOT/tests/compat/phase0_required_http.txt"
ENDPOINTS_FILE="$ROOT/crates/cairn-api/src/endpoints.rs"
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
require_file "$SSE_PUBLISHER_FILE"

status_for_requirement() {
  case "$1" in
    "GET /v1/tasks?status=running&type=agent")
      printf 'read_endpoint_trait_present'
      ;;
    "GET /v1/approvals?status=pending")
      printf 'read_endpoint_trait_present'
      ;;
    "GET /v1/stream?lastEventId=<id>")
      printf 'stream_publisher_present_followup_remaining'
      ;;
    "GET /v1/feed?limit=20&unread=true"|"GET /v1/memories/search?q=test&limit=10"|"POST /v1/assistant/message body={message,mode?,sessionId?}"|"POST /v1/assistant/message body={message,mode?}")
      printf 'no_explicit_api_boundary_yet'
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
      printf 'Preserved route and fixture exist, but no dedicated Rust-side feed endpoint/service boundary is visible yet in `endpoints.rs`.'
      ;;
    "GET /v1/memories/search?q=test&limit=10")
      printf 'Preserved route and fixture exist, but no dedicated Rust-side memory search endpoint/service boundary is visible yet in `endpoints.rs`.'
      ;;
    "POST /v1/assistant/message body={message,mode?,sessionId?}")
      printf 'Preserved mutation route and fixture exist, but no explicit Rust-side assistant message command boundary is visible yet in `endpoints.rs`.'
      ;;
    "POST /v1/assistant/message body={message,mode?}")
      printf 'Preserved mutation route and fixture exist, but no explicit Rust-side assistant message command boundary is visible yet in `endpoints.rs`.'
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
    "GET /v1/stream?lastEventId=<id>")
      printf 'align_sse_payload_shape'
      ;;
    "GET /v1/feed?limit=20&unread=true"|"GET /v1/memories/search?q=test&limit=10")
      printf 'define_read_service_boundary'
      ;;
    "POST /v1/assistant/message body={message,mode?,sessionId?}"|"POST /v1/assistant/message body={message,mode?}")
      printf 'define_mutation_command_boundary'
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
