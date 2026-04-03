#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  requeue-extra-claims.sh [worker-4] [--keep 1]

Moves all but the oldest N claimed tasks back to pending.
Use this when a worker shell accidentally claims multiple tasks and
we want to restore one-active-task discipline.
EOF
}

worker=""
keep=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    worker-*|[1-8])
      worker="$(normalize_worker "$1")"
      shift
      ;;
    --keep)
      keep="${2:-1}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

ensure_queue_layout

workers=()
if [[ -n "$worker" ]]; then
  workers=("$worker")
else
  workers=(worker-1 worker-2 worker-3 worker-4 worker-5 worker-6 worker-7 worker-8)
fi

for w in "${workers[@]}"; do
  count=0
  while read -r file; do
    [[ -f "$file" ]] || continue
    count=$((count + 1))
    if (( count <= keep )); then
      continue
    fi
    task_id="$(task_id_from_path "$file")"
    summary="$(task_summary "$file")"
    append_task_metadata "$file" "Requeued-At" "$(iso_now)"
    append_task_metadata "$file" "Requeued-By" "manager"
    mv "$file" "$(worker_task_dir "$w")/pending/${task_id}.task"
    emit_event "$w" "requeued" "$w" "$task_id" "$summary" "manager requeued extra claimed task"
    emit_event "manager" "requeued" "$w" "$task_id" "$summary" "manager requeued extra claimed task"
    printf 'requeued %s %s\n' "$w" "$task_id"
  done < <(find "$TASK_ROOT/$w/claimed" -maxdepth 1 -type f -name '*.task' | sort)
done
