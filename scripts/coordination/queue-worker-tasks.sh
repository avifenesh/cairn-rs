#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  queue-worker-tasks.sh [--from NAME] worker-4 "task one" "task two"
  printf '%s\n' "task one" "task two" | queue-worker-tasks.sh [--from NAME] worker-4
EOF
}

queued_by="manager"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from)
      queued_by="${2:-}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 1
fi

ensure_queue_layout
worker="$(normalize_worker "$1")"
shift

tasks=()
if [[ $# -gt 0 ]]; then
  tasks=("$@")
else
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    tasks+=("$line")
  done
fi

if [[ ${#tasks[@]} -eq 0 ]]; then
  echo "no tasks provided" >&2
  exit 1
fi

for summary in "${tasks[@]}"; do
  file="$(write_task_file "$worker" "$summary" "$queued_by")"
  task_id="$(task_id_from_path "$file")"
  emit_event "$worker" "queued" "$worker" "$task_id" "$summary" "queued by $queued_by"
  printf 'queued %s %s\n' "$task_id" "$summary"
done

print_worker_queue_snapshot "$worker"
