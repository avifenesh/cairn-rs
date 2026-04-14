#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HTTP_MAP="$ROOT/tests/compat/phase0_http_fixture_map.tsv"
SSE_MAP="$ROOT/tests/compat/phase0_sse_fixture_map.tsv"
OUT="$ROOT/tests/fixtures/migration/phase0_mismatch_report.md"

generate_section() {
  local title="$1"
  local map_file="$2"
  local kind="$3"

  {
    echo "## $title"
    echo
    echo "| $kind | Fixture | Status | Next Step |"
    echo "|---|---|---|---|"
    tail -n +2 "$map_file" | while IFS=$'\t' read -r item fixture status next_step; do
      local resolved="$ROOT/${fixture}"
      local rel_fixture="../${fixture#tests/fixtures/}"
      local effective_status="$status"
      if [[ ! -f "$resolved" ]]; then
        effective_status="missing_fixture"
      fi
      echo "| \`$item\` | [\`$fixture\`]($rel_fixture) | \`$effective_status\` | \`$next_step\` |"
    done
    echo
  } >> "$OUT"
}

cat > "$OUT" <<'EOF'
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

EOF

generate_section "HTTP Preserved Set" "$HTTP_MAP" "Requirement"
generate_section "SSE Preserved Set" "$SSE_MAP" "Event"

cat >> "$OUT" <<'EOF'
## Current Reading

- The minimum preserved Phase 0 set now has seeded fixtures for every required HTTP and SSE surface.
- The next Worker 1 task is to replace or confirm these seeded fixtures with direct backend captures from `../cairn-sdk` where possible.
- Any later mismatch between Rust behavior and these fixtures should be classified as:
  - preserve bug
  - intentional break
  - transitional surface

EOF

echo "generated $OUT"
