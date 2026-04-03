#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  show-worker-queue.sh worker-4
  show-worker-queue.sh --all
EOF
}

ensure_queue_layout

if [[ "${1:-}" == "--all" ]]; then
  for n in 1 2 3 4 5 6 7 8; do
    print_worker_queue_snapshot "worker-$n"
  done
  exit 0
fi

if [[ $# -ne 1 ]]; then
  usage >&2
  exit 1
fi

print_worker_queue_snapshot "$1"
