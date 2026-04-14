#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UPSTREAM_ROOT="/mnt/c/users/avife/cairn"
OUT="$ROOT/tests/fixtures/migration/phase0_upstream_contract_report.md"

CLIENT_TS="$UPSTREAM_ROOT/frontend/src/lib/api/client.ts"
SSE_TS="$UPSTREAM_ROOT/frontend/src/lib/stores/sse.svelte.ts"
BRIEF_MD="$UPSTREAM_ROOT/docs/design/FRONTEND_AGENT_BRIEF.md"
SERVER_PROTO_MD="$UPSTREAM_ROOT/docs/design/pieces/09-server-protocols.md"
HTTP_REQ="$ROOT/tests/compat/phase0_required_http.txt"
SSE_REQ="$ROOT/tests/compat/phase0_required_sse.txt"

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
}

require_file "$CLIENT_TS"
require_file "$SSE_TS"
require_file "$BRIEF_MD"
require_file "$SERVER_PROTO_MD"
require_file "$HTTP_REQ"
require_file "$SSE_REQ"

has_literal() {
  local needle="$1"
  local file="$2"
  grep -Fq "$needle" "$file"
}

collect_http_evidence() {
  local route="$1"
  local evidence=()
  if has_literal "$route" "$CLIENT_TS"; then
    evidence+=("frontend_client")
  fi
  if has_literal "$route" "$BRIEF_MD"; then
    evidence+=("frontend_brief")
  fi
  if has_literal "$route" "$SERVER_PROTO_MD"; then
    evidence+=("server_protocol_doc")
  fi
  if ((${#evidence[@]} == 0)); then
    printf 'missing_upstream_evidence'
  else
    local joined=""
    local item
    for item in "${evidence[@]}"; do
      if [[ -n "$joined" ]]; then
        joined+=", "
      fi
      joined+="$item"
    done
    printf '%s' "$joined"
  fi
}

collect_sse_evidence() {
  local event_name="$1"
  local evidence=()
  if has_literal "'$event_name'" "$SSE_TS"; then
    evidence+=("frontend_sse_store")
  fi
  if has_literal "\`$event_name\`" "$BRIEF_MD"; then
    evidence+=("frontend_brief")
  fi
  if ((${#evidence[@]} == 0)); then
    printf 'missing_upstream_evidence'
  else
    local joined=""
    local item
    for item in "${evidence[@]}"; do
      if [[ -n "$joined" ]]; then
        joined+=", "
      fi
      joined+="$item"
    done
    printf '%s' "$joined"
  fi
}

{
  printf '# Phase 0 Upstream Contract Report\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: confirm that the preserved Phase 0 HTTP/SSE fixture set is backed by the upstream frontend and protocol contract even when direct server-side captures are not available locally.\n\n'
  printf 'Current reading:\n\n'
  printf -- '- the local `../cairn-sdk` checkout currently exposes preserved `/v1/*` contract evidence through frontend usage and protocol docs\n'
  printf -- '- Worker 1 did not find a concrete legacy backend handler surface for these routes/events in the local checkout, so this report is intentionally protocol-backed\n'
  printf -- '- if direct handler captures become available later, they should tighten these fixtures rather than replace the compatibility contract casually\n\n'

  printf '## HTTP Evidence\n\n'
  printf '| Requirement | Base Route | Upstream Evidence |\n'
  printf '|---|---|---|\n'
  while IFS= read -r requirement; do
    [[ -z "$requirement" ]] && continue
    method="${requirement%% *}"
    rest="${requirement#* }"
    path="${rest%% *}"
    base="${path%%\?*}"
    evidence="$(collect_http_evidence "$base")"
    printf '| `%s` | `%s` | `%s` |\n' "$requirement" "$method $base" "$evidence"
  done < "$HTTP_REQ"

  printf '\n## SSE Evidence\n\n'
  printf '| Event | Upstream Evidence |\n'
  printf '|---|---|\n'
  while IFS= read -r event_name; do
    [[ -z "$event_name" ]] && continue
    evidence="$(collect_sse_evidence "$event_name")"
    printf '| `%s` | `%s` |\n' "$event_name" "$evidence"
  done < "$SSE_REQ"
} > "$OUT"

if grep -Fq 'missing_upstream_evidence' "$OUT"; then
  echo "one or more required Phase 0 surfaces are missing upstream contract evidence" >&2
  exit 1
fi

echo "generated $OUT"
