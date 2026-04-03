#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/lib.sh"

ensure_queue_layout

for target in manager worker-1 worker-2 worker-3 worker-4 worker-5 worker-6 worker-7 worker-8; do
  remove_stale_listener_pid "$target"
  pid_file="$(listener_pid_file "$target")"
  log_file="$(listener_log_file "$target")"
  if listener_is_running "$target"; then
    printf '%s\tup\tpid=%s\tlog=%s\n' "$target" "$(cat "$pid_file")" "$log_file"
  else
    printf '%s\tdown\tpid=-\tlog=%s\n' "$target" "$log_file"
  fi
done
