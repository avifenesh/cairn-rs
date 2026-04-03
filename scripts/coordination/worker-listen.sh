#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  worker-listen.sh worker-4 [--once] [--interval 2]

Runs until interrupted unless --once is set.
EOF
}

once=0
interval=2

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 1
fi

worker="$(normalize_worker "$1")"
shift

while [[ $# -gt 0 ]]; do
  case "$1" in
    --once)
      once=1
      shift
      ;;
    --interval)
      interval="${2:-2}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

ensure_queue_layout
state_file="$(listener_state_file "${worker}.listener")"
event_dir="$(worker_event_dir "$worker")"

echo "listening for $worker"
print_worker_queue_snapshot "$worker"

process_new_events() {
  local last_seen file base kind task_id summary message
  last_seen="$(read_last_seen_event "$state_file")"
  find "$event_dir" -maxdepth 1 -type f -name '*.event' | sort | while read -r file; do
    [[ -z "$file" ]] && continue
    base="$(basename "$file")"
    if [[ -n "$last_seen" && ( "$base" < "$last_seen" || "$base" == "$last_seen" ) ]]; then
      continue
    fi
    kind="$(event_field "Kind" "$file")"
    task_id="$(event_field "Task-Id" "$file")"
    summary="$(event_field "Summary" "$file")"
    message="$(event_field "Message" "$file")"
    printf '\a[%s] %s %s %s\n' "$worker" "$kind" "$task_id" "$summary"
    if [[ -n "$message" ]]; then
      printf '  %s\n' "$message"
    fi
    write_last_seen_event "$state_file" "$base"
    print_worker_queue_snapshot "$worker"
  done
}

if [[ $once -eq 1 ]]; then
  process_new_events
  exit 0
fi

if command -v inotifywait >/dev/null 2>&1; then
  process_new_events
  while inotifywait -qq -e create,moved_to "$event_dir"; do
    process_new_events
  done
else
  while true; do
    process_new_events
    sleep "$interval"
  done
fi
