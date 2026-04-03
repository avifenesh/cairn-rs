#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  worker-claim-next.sh worker-4 [--by NAME] [--force]
EOF
}

claimed_by=""
force=0

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 1
fi

worker="$(normalize_worker "$1")"
shift

while [[ $# -gt 0 ]]; do
  case "$1" in
    --by)
      claimed_by="${2:-}"
      shift 2
      ;;
    --force)
      force=1
      shift
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
if [[ "$force" != "1" && "$(claimed_count "$worker")" != "0" ]]; then
  echo "$worker already has a claimed task; complete or block it before claiming another" >&2
  print_worker_queue_snapshot "$worker"
  exit 1
fi

task="$(oldest_pending_task "$worker")"
if [[ -z "$task" ]]; then
  emit_event "manager" "queue_empty" "$worker" "-" "no pending task" "worker attempted claim with empty queue"
  echo "$worker has no pending tasks"
  exit 0
fi

task_id="$(task_id_from_path "$task")"
summary="$(task_summary "$task")"
append_task_metadata "$task" "Claimed-At" "$(iso_now)"
if [[ -n "$claimed_by" ]]; then
  append_task_metadata "$task" "Claimed-By" "$claimed_by"
fi
mv "$task" "$(worker_task_dir "$worker")/claimed/${task_id}.task"
emit_event "manager" "claimed" "$worker" "$task_id" "$summary" "${claimed_by:-worker} claimed task"
if [[ "$(pending_count "$worker")" == "0" ]]; then
  emit_event "manager" "queue_empty" "$worker" "-" "pending queue empty" "refill suggested after claim"
fi
printf 'claimed %s %s\n' "$task_id" "$summary"
print_worker_queue_snapshot "$worker"
