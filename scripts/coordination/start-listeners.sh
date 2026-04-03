#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  start-listeners.sh --all
  start-listeners.sh manager worker-4 worker-8
  start-listeners.sh worker-6 --interval 2
EOF
}

interval=2
targets=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all)
      targets=(manager worker-1 worker-2 worker-3 worker-4 worker-5 worker-6 worker-7 worker-8)
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
    manager)
      targets+=("manager")
      shift
      ;;
    worker-[1-8]|[1-8])
      targets+=("$(normalize_worker "$1")")
      shift
      ;;
    *)
      echo "unknown target: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ ${#targets[@]} -eq 0 ]]; then
  usage >&2
  exit 1
fi

ensure_queue_layout

for target in "${targets[@]}"; do
  remove_stale_listener_pid "$target"
  if listener_is_running "$target"; then
    printf '%s already running pid=%s log=%s\n' \
      "$target" \
      "$(cat "$(listener_pid_file "$target")")" \
      "$(listener_log_file "$target")"
    continue
  fi

  log_file="$(listener_log_file "$target")"
  pid_file="$(listener_pid_file "$target")"

  if [[ "$target" == "manager" ]]; then
    nohup "$script_dir/manager-listen.sh" --interval "$interval" >>"$log_file" 2>&1 &
  else
    nohup "$script_dir/worker-listen.sh" "$target" --interval "$interval" >>"$log_file" 2>&1 &
  fi
  pid=$!
  printf '%s\n' "$pid" > "$pid_file"
  printf 'started %s pid=%s log=%s\n' "$target" "$pid" "$log_file"
done
