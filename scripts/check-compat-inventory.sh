#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPAT_DIR="$ROOT/tests/compat"
FIXTURE_DIR="$ROOT/tests/fixtures"

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
}

require_dir() {
  local dir="$1"
  if [[ ! -d "$dir" ]]; then
    echo "missing required directory: $dir" >&2
    exit 1
  fi
}

require_file "$COMPAT_DIR/http_routes.tsv"
require_file "$COMPAT_DIR/sse_events.tsv"
require_file "$COMPAT_DIR/phase0_required_http.txt"
require_file "$COMPAT_DIR/phase0_required_sse.txt"
require_file "$COMPAT_DIR/MIGRATION_HARNESS.md"
require_dir "$FIXTURE_DIR/http"
require_dir "$FIXTURE_DIR/sse"
require_dir "$FIXTURE_DIR/migration"

awk -F '\t' 'NR == 1 { next } NF != 5 { exit 1 }' "$COMPAT_DIR/http_routes.tsv" || {
  echo "http_routes.tsv must contain 5 tab-separated columns on every data row" >&2
  exit 1
}

awk -F '\t' 'NR == 1 { next } NF != 3 { exit 1 }' "$COMPAT_DIR/sse_events.tsv" || {
  echo "sse_events.tsv must contain 3 tab-separated columns on every data row" >&2
  exit 1
}

while IFS= read -r route; do
  [[ -z "$route" ]] && continue
  path="$(printf '%s\n' "$route" | awk '{print $2}')"
  base="${path%%\?*}"
  if ! grep -Fq "$base" "$COMPAT_DIR/http_routes.tsv"; then
    echo "missing required HTTP inventory entry for: $route" >&2
    exit 1
  fi
done < "$COMPAT_DIR/phase0_required_http.txt"

while IFS= read -r event_name; do
  [[ -z "$event_name" ]] && continue
  if ! grep -Fq "${event_name}" "$COMPAT_DIR/sse_events.tsv"; then
    echo "missing required SSE inventory entry for: $event_name" >&2
    exit 1
  fi
done < "$COMPAT_DIR/phase0_required_sse.txt"

echo "compatibility inventory looks structurally valid"
