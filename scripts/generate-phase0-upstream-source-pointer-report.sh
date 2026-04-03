#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UPSTREAM_ROOT="/mnt/c/users/avife/cairn"
OUT="$ROOT/tests/fixtures/migration/phase0_upstream_source_pointers.md"

CLIENT_TS="$UPSTREAM_ROOT/frontend/src/lib/api/client.ts"
SSE_TS="$UPSTREAM_ROOT/frontend/src/lib/stores/sse.svelte.ts"
TYPES_TS="$UPSTREAM_ROOT/frontend/src/lib/types.ts"
BRIEF_MD="$UPSTREAM_ROOT/docs/design/FRONTEND_AGENT_BRIEF.md"
SERVER_PROTO_MD="$UPSTREAM_ROOT/docs/design/pieces/09-server-protocols.md"
FRONTEND_PIECE_MD="$UPSTREAM_ROOT/docs/design/pieces/10-frontend.md"
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
require_file "$TYPES_TS"
require_file "$BRIEF_MD"
require_file "$SERVER_PROTO_MD"
require_file "$FRONTEND_PIECE_MD"
require_file "$HTTP_REQ"
require_file "$SSE_REQ"

relative_to_upstream() {
  local path="$1"
  printf '%s' "${path#"$UPSTREAM_ROOT/"}"
}

collect_literal_matches() {
  local literal="$1"
  shift
  local matches=()
  local file
  for file in "$@"; do
    while IFS= read -r line; do
      [[ -z "$line" ]] && continue
      local lineno="${line%%:*}"
      matches+=("<code>$(relative_to_upstream "$file"):${lineno}</code>")
    done < <(grep -Fn "$literal" "$file" || true)
  done

  if ((${#matches[@]} == 0)); then
    printf '<code>missing</code>'
  else
    local joined=""
    local item
    for item in "${matches[@]}"; do
      if [[ -n "$joined" ]]; then
        joined+='<br>'
      fi
      joined+="$item"
    done
    printf '%s' "$joined"
  fi
}

http_pointer_set() {
  local route="$1"
  collect_literal_matches "$route" "$CLIENT_TS" "$BRIEF_MD" "$SERVER_PROTO_MD"
}

sse_pointer_set() {
  local event_name="$1"
  local joined=""

  local store_matches
  local brief_matches
  local types_matches
  local frontend_piece_matches

  store_matches="$(collect_literal_matches "'$event_name'" "$SSE_TS")"
  brief_matches="$(collect_literal_matches "\`$event_name\`" "$BRIEF_MD")"
  types_matches="$(collect_literal_matches "'$event_name'" "$TYPES_TS")"
  frontend_piece_matches="$(collect_literal_matches "'$event_name'" "$FRONTEND_PIECE_MD")"

  for item in "$store_matches" "$brief_matches" "$types_matches" "$frontend_piece_matches"; do
    if [[ "$item" == '<code>missing</code>' ]]; then
      continue
    fi
    if [[ -n "$joined" ]]; then
      joined+='<br>'
    fi
    joined+="$item"
  done

  if [[ -z "$joined" ]]; then
    printf '<code>missing</code>'
  else
    printf '%s' "$joined"
  fi
}

{
  printf '# Phase 0 Upstream Source Pointers\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: pin the preserved Phase 0 compatibility contract to exact upstream files and line numbers in `../cairn`, so Worker 1 evidence stays auditable even without direct legacy backend handler captures.\n\n'
  printf 'Current reading:\n\n'
  printf -- '- these pointers intentionally reference frontend and protocol sources because the local upstream checkout still does not expose concrete handler implementations for the preserved `/v1/*` surfaces\n'
  printf -- '- if direct backend capture becomes available later, it should supplement these pointers rather than erase the preserved frontend contract lineage\n\n'

  printf '## HTTP Source Pointers\n\n'
  printf '| Requirement | Base Route | Upstream Source Pointers |\n'
  printf '|---|---|---|\n'
  while IFS= read -r requirement; do
    [[ -z "$requirement" ]] && continue
    method="${requirement%% *}"
    rest="${requirement#* }"
    path="${rest%% *}"
    base="${path%%\?*}"
    printf '| `%s` | `%s` | %s |\n' \
      "$requirement" \
      "$method $base" \
      "$(http_pointer_set "$base")"
  done < "$HTTP_REQ"

  printf '\n## SSE Source Pointers\n\n'
  printf '| Event | Upstream Source Pointers |\n'
  printf '|---|---|\n'
  while IFS= read -r event_name; do
    [[ -z "$event_name" ]] && continue
    printf '| `%s` | %s |\n' \
      "$event_name" \
      "$(sse_pointer_set "$event_name")"
  done < "$SSE_REQ"
} > "$OUT"

if grep -Fq '<code>missing</code>' "$OUT"; then
  echo "one or more required Phase 0 surfaces are missing upstream source pointers" >&2
  exit 1
fi

echo "generated $OUT"
