#!/usr/bin/env bash
set -euo pipefail

ROOT="${COORD_ROOT_OVERRIDE:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
QUEUE_ROOT="${COORD_QUEUE_ROOT:-$ROOT/.coordination/queue}"
TASK_ROOT="$QUEUE_ROOT/tasks"
EVENT_ROOT="$QUEUE_ROOT/events"
STATE_ROOT="$QUEUE_ROOT/state"
LISTENER_ROOT="$STATE_ROOT/listeners"
LISTENER_PID_ROOT="$LISTENER_ROOT/pids"
LISTENER_LOG_ROOT="$LISTENER_ROOT/logs"

normalize_worker() {
  local raw="${1:-}"
  case "$raw" in
    worker-[1-8]) printf '%s\n' "$raw" ;;
    [1-8]) printf 'worker-%s\n' "$raw" ;;
    *)
      echo "invalid worker id: $raw (expected worker-1..worker-8 or 1..8)" >&2
      return 1
      ;;
  esac
}

ensure_queue_layout() {
  mkdir -p "$TASK_ROOT" "$EVENT_ROOT" "$STATE_ROOT" "$LISTENER_PID_ROOT" "$LISTENER_LOG_ROOT"
  local n
  for n in 1 2 3 4 5 6 7 8; do
    mkdir -p \
      "$TASK_ROOT/worker-$n/pending" \
      "$TASK_ROOT/worker-$n/claimed" \
      "$TASK_ROOT/worker-$n/done" \
      "$EVENT_ROOT/worker-$n"
  done
  mkdir -p "$EVENT_ROOT/manager"
}

worker_task_dir() {
  local worker
  worker="$(normalize_worker "$1")"
  printf '%s\n' "$TASK_ROOT/$worker"
}

worker_event_dir() {
  local worker
  worker="$(normalize_worker "$1")"
  printf '%s\n' "$EVENT_ROOT/$worker"
}

manager_event_dir() {
  printf '%s\n' "$EVENT_ROOT/manager"
}

iso_now() {
  date '+%Y-%m-%dT%H:%M:%S%z'
}

id_now() {
  date '+%Y%m%dT%H%M%S%N'
}

task_summary() {
  local file="$1"
  [[ -f "$file" ]] || return 0
  sed -n 's/^Summary: //p' "$file" 2>/dev/null | head -n 1 || true
}

task_id_from_path() {
  basename "$1" .task
}

event_field() {
  local key="$1"
  local file="$2"
  [[ -f "$file" ]] || return 0
  sed -n "s/^${key}: //p" "$file" 2>/dev/null | head -n 1 || true
}

write_task_file() {
  local worker="$1"
  local summary="$2"
  local queued_by="$3"
  local id file tmp
  id="$(id_now)"
  file="$TASK_ROOT/$worker/pending/$id.task"
  tmp="$file.tmp"
  {
    printf 'Summary: %s\n' "$summary"
    printf 'Worker: %s\n' "$worker"
    printf 'Queued-By: %s\n' "$queued_by"
    printf 'Queued-At: %s\n' "$(iso_now)"
  } > "$tmp"
  mv "$tmp" "$file"
  printf '%s\n' "$file"
}

append_task_metadata() {
  local file="$1"
  local key="$2"
  local value="$3"
  printf '%s: %s\n' "$key" "$value" >> "$file"
}

emit_event() {
  local recipient="$1"
  local kind="$2"
  local worker="$3"
  local task_id="$4"
  local summary="$5"
  local message="${6:-}"
  local dir file tmp event_id
  if [[ "$recipient" == "manager" ]]; then
    dir="$(manager_event_dir)"
  else
    dir="$(worker_event_dir "$recipient")"
  fi
  event_id="$(id_now)"
  file="$dir/${event_id}__${kind}.event"
  tmp="$file.tmp"
  {
    printf 'Kind: %s\n' "$kind"
    printf 'Worker: %s\n' "$worker"
    printf 'Task-Id: %s\n' "$task_id"
    printf 'At: %s\n' "$(iso_now)"
    printf 'Summary: %s\n' "$summary"
    printf 'Message: %s\n' "$message"
  } > "$tmp"
  mv "$tmp" "$file"
}

pending_count() {
  local worker
  worker="$(normalize_worker "$1")"
  find "$TASK_ROOT/$worker/pending" -maxdepth 1 -type f -name '*.task' | wc -l | tr -d ' '
}

claimed_count() {
  local worker
  worker="$(normalize_worker "$1")"
  find "$TASK_ROOT/$worker/claimed" -maxdepth 1 -type f -name '*.task' | wc -l | tr -d ' '
}

done_count() {
  local worker
  worker="$(normalize_worker "$1")"
  find "$TASK_ROOT/$worker/done" -maxdepth 1 -type f -name '*.task' | wc -l | tr -d ' '
}

oldest_pending_task() {
  local worker
  worker="$(normalize_worker "$1")"
  find "$TASK_ROOT/$worker/pending" -maxdepth 1 -type f -name '*.task' | sort | head -n 1
}

list_tasks_table() {
  local worker state dir
  worker="$(normalize_worker "$1")"
  state="${2:-pending}"
  dir="$TASK_ROOT/$worker/$state"
  find "$dir" -maxdepth 1 -type f -name '*.task' | sort | while read -r file; do
    [[ -z "$file" ]] && continue
    printf '%s\t%s\n' "$(task_id_from_path "$file")" "$(task_summary "$file")"
  done
}

listener_state_file() {
  local name="$1"
  printf '%s\n' "$STATE_ROOT/${name}.state"
}

listener_pid_file() {
  local name="$1"
  printf '%s\n' "$LISTENER_PID_ROOT/${name}.pid"
}

listener_log_file() {
  local name="$1"
  printf '%s\n' "$LISTENER_LOG_ROOT/${name}.log"
}

listener_is_running() {
  local name="$1"
  local pid_file pid
  pid_file="$(listener_pid_file "$name")"
  [[ -f "$pid_file" ]] || return 1
  pid="$(cat "$pid_file")"
  [[ -n "$pid" ]] || return 1
  kill -0 "$pid" >/dev/null 2>&1
}

remove_stale_listener_pid() {
  local name="$1"
  local pid_file
  pid_file="$(listener_pid_file "$name")"
  if [[ -f "$pid_file" ]] && ! listener_is_running "$name"; then
    rm -f "$pid_file"
  fi
}

read_last_seen_event() {
  local state_file="$1"
  if [[ -f "$state_file" ]]; then
    cat "$state_file"
  fi
}

write_last_seen_event() {
  local state_file="$1"
  local last_seen="$2"
  printf '%s\n' "$last_seen" > "$state_file"
}

print_worker_queue_snapshot() {
  local worker id summary shown=0
  worker="$(normalize_worker "$1")"
  printf '%s pending=%s claimed=%s done=%s\n' \
    "$worker" \
    "$(pending_count "$worker")" \
    "$(claimed_count "$worker")" \
    "$(done_count "$worker")"

  while IFS=$'\t' read -r id summary; do
    [[ -n "$id" ]] || continue
    printf '  PENDING %s %s\n' "$id" "$summary"
    shown=$(( shown + 1 ))
    if (( shown >= 10 )); then
      break
    fi
  done < <(list_tasks_table "$worker" pending)
}
