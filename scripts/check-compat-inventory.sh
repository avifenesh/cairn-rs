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

cargo_test_clean_env() {
  (
    cd "$ROOT"
    env -u LD_LIBRARY_PATH CARGO_TARGET_DIR="$ROOT/target" cargo test "$@"
  )
}

run_report_generator() {
  local script="$1"
  if [[ ! -f "$script" ]]; then
    echo "missing report generator: $script" >&2
    exit 1
  fi
  bash "$script"
}

require_file "$COMPAT_DIR/http_routes.tsv"
require_file "$COMPAT_DIR/sse_events.tsv"
require_file "$COMPAT_DIR/phase0_required_http.txt"
require_file "$COMPAT_DIR/phase0_required_sse.txt"
require_file "$COMPAT_DIR/phase0_http_fixture_map.tsv"
require_file "$COMPAT_DIR/phase0_sse_fixture_map.tsv"
require_file "$COMPAT_DIR/MIGRATION_HARNESS.md"
require_dir "$FIXTURE_DIR/http"
require_dir "$FIXTURE_DIR/sse"
require_dir "$FIXTURE_DIR/migration"

run_report_generator "$ROOT/scripts/generate-phase0-upstream-contract-report.sh"
run_report_generator "$ROOT/scripts/generate-phase0-http-endpoint-gap-report.sh"
run_report_generator "$ROOT/scripts/generate-phase0-sse-publisher-gap-report.sh"
run_report_generator "$ROOT/scripts/generate-phase0-sse-payload-handoff.sh"

awk -F '\t' 'NR == 1 { next } NF != 5 { exit 1 }' "$COMPAT_DIR/http_routes.tsv" || {
  echo "http_routes.tsv must contain 5 tab-separated columns on every data row" >&2
  exit 1
}

awk -F '\t' 'NR == 1 { next } NF != 3 { exit 1 }' "$COMPAT_DIR/sse_events.tsv" || {
  echo "sse_events.tsv must contain 3 tab-separated columns on every data row" >&2
  exit 1
}

awk -F '\t' 'NR == 1 { next } NF != 4 { exit 1 }' "$COMPAT_DIR/phase0_http_fixture_map.tsv" || {
  echo "phase0_http_fixture_map.tsv must contain 4 tab-separated columns on every data row" >&2
  exit 1
}

awk -F '\t' 'NR == 1 { next } NF != 4 { exit 1 }' "$COMPAT_DIR/phase0_sse_fixture_map.tsv" || {
  echo "phase0_sse_fixture_map.tsv must contain 4 tab-separated columns on every data row" >&2
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

require_file "$FIXTURE_DIR/HARVESTING_NOTES.md"
require_file "$FIXTURE_DIR/migration/README.md"
require_file "$FIXTURE_DIR/migration/phase0_mismatch_report.md"
require_file "$FIXTURE_DIR/migration/phase0_upstream_contract_report.md"
require_file "$FIXTURE_DIR/migration/phase0_http_endpoint_gap_report.md"
require_file "$FIXTURE_DIR/migration/phase0_sse_publisher_gap_report.md"
require_file "$FIXTURE_DIR/migration/phase0_sse_payload_handoff.md"
require_file "$FIXTURE_DIR/http/GET__v1_feed__limit20_unread_true.json"
require_file "$FIXTURE_DIR/http/GET__v1_tasks__status_running_type_agent.json"
require_file "$FIXTURE_DIR/http/GET__v1_approvals__status_pending.json"
require_file "$FIXTURE_DIR/http/GET__v1_memories_search__q_test_limit_10.json"
require_file "$FIXTURE_DIR/http/POST__v1_assistant_message__with_session.json"
require_file "$FIXTURE_DIR/http/POST__v1_assistant_message__without_session.json"
require_file "$FIXTURE_DIR/http/GET__v1_stream__replay_from_last_event_id.json"
require_file "$FIXTURE_DIR/sse/ready__connected.json"
require_file "$FIXTURE_DIR/sse/feed_update__single_item.json"
require_file "$FIXTURE_DIR/sse/poll_completed__source_done.json"
require_file "$FIXTURE_DIR/sse/task_update__running_task.json"
require_file "$FIXTURE_DIR/sse/approval_required__pending.json"
require_file "$FIXTURE_DIR/sse/assistant_delta__incremental_reply.json"
require_file "$FIXTURE_DIR/sse/assistant_end__complete_reply.json"
require_file "$FIXTURE_DIR/sse/assistant_reasoning__round_1.json"
require_file "$FIXTURE_DIR/sse/assistant_tool_call__start.json"
require_file "$FIXTURE_DIR/sse/memory_proposed__proposal.json"
require_file "$FIXTURE_DIR/sse/agent_progress__message.json"

cargo_test_clean_env -p cairn-api --test compat_catalog_sync --manifest-path "$ROOT/Cargo.toml"
cargo_test_clean_env -p cairn-api --test phase0_fixture_shapes --manifest-path "$ROOT/Cargo.toml"

echo "compatibility inventory looks structurally valid"
