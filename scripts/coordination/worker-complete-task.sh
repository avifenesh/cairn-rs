#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  worker-complete-task.sh worker-4 <task-id> (--proof TEXT... | --blocker TEXT...) [--note TEXT] [--by NAME]
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
status="completed"
declare -a proofs=()
declare -a blockers=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --proof)
      proofs+=("${2:-}")
      shift 2
      ;;
    --blocker)
      blockers+=("${2:-}")
      shift 2
      ;;
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

trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s\n' "$value"
}

is_generic_detail() {
  local value lowered
  value="$(trim "$1")"
  lowered="$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]')"
  case "$lowered" in
    ""|done|completed|investigated|verified|no\ drift|standing\ order\ scope|all\ tests\ green)
      return 0
      ;;
    verified:\ all\ tests\ green*|investigated.\ standing\ order\ scope*|no\ drift*|done\ in*|looks\ good*)
      return 0
      ;;
  esac
  if [[ ${#value} -lt 16 ]]; then
    return 0
  fi
  return 1
}

validate_details() {
  local kind="$1"
  shift
  local item trimmed
  for item in "$@"; do
    trimmed="$(trim "$item")"
    if [[ -z "$trimmed" ]]; then
      echo "$kind cannot be empty" >&2
      return 1
    fi
    if is_generic_detail "$trimmed"; then
      echo "$kind is too generic: $trimmed" >&2
      echo "provide concrete evidence like a file path, command, test, or exact blocker seam" >&2
      return 1
    fi
  done
}

if (( ${#proofs[@]} > 0 && ${#blockers[@]} > 0 )); then
  echo "use either --proof or --blocker for a completion, not both" >&2
  exit 1
fi

if (( ${#proofs[@]} == 0 && ${#blockers[@]} == 0 )); then
  echo "completion requires at least one --proof or --blocker entry" >&2
  exit 1
fi

if (( ${#proofs[@]} > 0 )); then
  validate_details "proof" "${proofs[@]}"
  status="completed"
fi

if (( ${#blockers[@]} > 0 )); then
  validate_details "blocker" "${blockers[@]}"
  status="blocked"
fi

summary="$(task_summary "$file")"
append_task_metadata "$file" "Completed-At" "$(iso_now)"
if [[ -n "$completed_by" ]]; then
  append_task_metadata "$file" "Completed-By" "$completed_by"
fi
append_task_metadata "$file" "Completion-Status" "$status"
for proof in "${proofs[@]}"; do
  append_task_metadata "$file" "Completion-Proof" "$(trim "$proof")"
done
for blocker in "${blockers[@]}"; do
  append_task_metadata "$file" "Completion-Blocker" "$(trim "$blocker")"
done
if [[ -n "$note" ]]; then
  append_task_metadata "$file" "Completion-Note" "$note"
fi
mv "$file" "$(worker_task_dir "$worker")/done/${task_id}.task"
emit_event "manager" "$status" "$worker" "$task_id" "$summary" "${note:-task $status}"
if [[ "$(pending_count "$worker")" == "0" ]]; then
  emit_event "manager" "queue_empty" "$worker" "-" "pending queue empty" "refill suggested after completion"
fi
printf '%s %s %s\n' "$status" "$task_id" "$summary"
print_worker_queue_snapshot "$worker"
