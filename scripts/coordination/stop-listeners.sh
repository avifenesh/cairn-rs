#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  stop-listeners.sh --all
  stop-listeners.sh manager worker-4 worker-8
EOF
}

targets=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --all)
      targets=(manager worker-1 worker-2 worker-3 worker-4 worker-5 worker-6 worker-7 worker-8)
      shift
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
  pid_file="$(listener_pid_file "$target")"
  remove_stale_listener_pid "$target"
  if [[ ! -f "$pid_file" ]]; then
    printf '%s not running\n' "$target"
    continue
  fi
  pid="$(cat "$pid_file")"
  if kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid"
    printf 'stopped %s pid=%s\n' "$target" "$pid"
  else
    printf '%s had stale pid=%s\n' "$target" "$pid"
  fi
  rm -f "$pid_file"
done
