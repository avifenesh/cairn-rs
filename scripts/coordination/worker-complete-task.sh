#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  worker-complete-task.sh worker-4 <task-id> [--note TEXT] [--by NAME]
EOF
}

if [[ $# -lt 2 ]]; then
  usage >&2
  exit 1
fi

worker="$(normalize_worker "$1")"
task_id="$2"
shift 2

note=""
completed_by=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --note)
      note="${2:-}"
      shift 2
      ;;
    --by)
      completed_by="${2:-}"
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
file="$(worker_task_dir "$worker")/claimed/${task_id}.task"
if [[ ! -f "$file" ]]; then
  echo "claimed task not found: $worker $task_id" >&2
  exit 1
fi

summary="$(task_summary "$file")"
append_task_metadata "$file" "Completed-At" "$(iso_now)"
if [[ -n "$completed_by" ]]; then
  append_task_metadata "$file" "Completed-By" "$completed_by"
fi
if [[ -n "$note" ]]; then
  append_task_metadata "$file" "Completion-Note" "$note"
fi
mv "$file" "$(worker_task_dir "$worker")/done/${task_id}.task"
emit_event "manager" "completed" "$worker" "$task_id" "$summary" "${note:-task completed}"
if [[ "$(pending_count "$worker")" == "0" ]]; then
  emit_event "manager" "queue_empty" "$worker" "-" "pending queue empty" "refill suggested after completion"
fi
printf 'completed %s %s\n' "$task_id" "$summary"
print_worker_queue_snapshot "$worker"
