#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/.coordination/WORKER_SLICE_HEALTH.md"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

declare -a entries=(
  "Worker 2|cairn-domain"
  "Worker 3|cairn-store"
  "Worker 4|cairn-runtime"
  "Worker 5|cairn-tools"
  "Worker 5|cairn-plugin-proto"
  "Worker 6|cairn-memory"
  "Worker 6|cairn-graph"
  "Worker 7|cairn-agent"
  "Worker 7|cairn-evals"
  "Worker 8|cairn-signal"
  "Worker 8|cairn-channels"
  "Worker 8|cairn-api"
  "Worker 8|cairn-app"
)

status_row() {
  local worker="$1"
  local crate="$2"
  local log="$TMP_DIR/$crate.log"
  local status
  local summary

  if (cd "$ROOT" && cargo test -p "$crate" >"$log" 2>&1); then
    status='`pass`'
    summary='All crate tests passed in isolation.'
  else
    status='`fail`'
    if grep -Fq 'selector_resolver.rs' "$log"; then
      summary='Selector resolver has `PromptReleaseState` import/type ambiguity; release service also has borrow-check failures.'
    elif grep -Fq 'release_service.rs' "$log"; then
      summary='Release service has overlapping mutable-borrow failures during transition/rollback flows.'
    else
      summary="$(head -n 5 "$log" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g' | sed 's/|/\\|/g')"
    fi
  fi

  printf '| %s | `%s` | %s | %s |\n' "$worker" "$crate" "$status" "$summary"
}

{
  printf '# Worker Slice Health\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: keep manager-level crate health visible while workers land slices in parallel.\n\n'
  printf 'Interpretation:\n\n'
  printf -- '- this report runs `cargo test -p <crate>` per owned crate instead of only relying on workspace-wide status\n'
  printf -- '- a red workspace with mostly green slice tests usually means one worker has a concentrated integration issue rather than broad drift\n\n'
  printf '## Current Slice Status\n\n'
  printf '| Worker | Crate | Status | Notes |\n'
  printf '|---|---|---|---|\n'

  for entry in "${entries[@]}"; do
    worker="${entry%%|*}"
    crate="${entry##*|}"
    status_row "$worker" "$crate"
  done

  printf '\n## Manager Read\n\n'
  printf -- '- if all rows except one pass, treat the red build as a focused blocker and keep unrelated workers moving\n'
  printf -- '- if several adjacent rows fail together, stop and look for shared-contract drift before more code lands\n'
} > "$OUT"

echo "generated $OUT"
