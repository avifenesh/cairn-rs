#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  manager-listen.sh [--once] [--interval 2]

Runs until interrupted unless --once is set.
EOF
}

once=0
interval=2

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
state_file="$(listener_state_file manager.listener)"
event_dir="$(manager_event_dir)"

echo "listening for manager refill events"
for n in 1 2 3 4 5 6 7 8; do
  print_worker_queue_snapshot "worker-$n"
done

process_new_events() {
  local last_seen file base kind worker task_id summary message pending
  last_seen="$(read_last_seen_event "$state_file")"
  find "$event_dir" -maxdepth 1 -type f -name '*.event' | sort | while read -r file; do
    [[ -z "$file" ]] && continue
    base="$(basename "$file")"
    if [[ -n "$last_seen" && ( "$base" < "$last_seen" || "$base" == "$last_seen" ) ]]; then
      continue
    fi
    kind="$(event_field "Kind" "$file")"
    worker="$(event_field "Worker" "$file")"
    task_id="$(event_field "Task-Id" "$file")"
    summary="$(event_field "Summary" "$file")"
    message="$(event_field "Message" "$file")"
    pending="$(pending_count "$worker")"
    printf '\a[manager] %s %s %s %s pending=%s\n' "$worker" "$kind" "$task_id" "$summary" "$pending"
    if [[ -n "$message" ]]; then
      printf '  %s\n' "$message"
    fi
    if [[ "$kind" == "queue_empty" || "$pending" == "0" ]]; then
      printf '  refill-suggested %s\n' "$worker"
    fi
    write_last_seen_event "$state_file" "$base"
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
