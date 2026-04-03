#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  audit-completions.sh [worker-4] [--limit 20]

Reports recent completed tasks that are missing proof/blocker metadata
or still carry generic completion notes from before completion hardening.
EOF
}

worker=""
limit=20

while [[ $# -gt 0 ]]; do
  case "$1" in
    worker-*|[1-8])
      worker="$(normalize_worker "$1")"
      shift
      ;;
    --limit)
      limit="${2:-20}"
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

generic_note() {
  local note lowered
  note="$(sed -n 's/^Completion-Note: //p' "$1" | head -n 1)"
  lowered="$(printf '%s' "$note" | tr '[:upper:]' '[:lower:]')"
  case "$lowered" in
    verified:\ all\ tests\ green*|investigated.\ standing\ order\ scope*|no\ drift*|done|completed|verified|investigated)
      return 0
      ;;
  esac
  return 1
}

found=0
for w in "${workers[@]}"; do
  echo "### $w"
  count=0
  while read -r file; do
    [[ -f "$file" ]] || continue
    count=$((count + 1))
    if (( count > limit )); then
      break
    fi
    has_proof="$(sed -n 's/^Completion-Proof: //p' "$file" | head -n 1)"
    has_blocker="$(sed -n 's/^Completion-Blocker: //p' "$file" | head -n 1)"
    status="$(sed -n 's/^Completion-Status: //p' "$file" | head -n 1)"
    if [[ -z "$has_proof" && -z "$has_blocker" ]]; then
      echo "MISSING_EVIDENCE $(basename "$file") :: $(task_summary "$file")"
      found=1
      continue
    fi
    if generic_note "$file"; then
      echo "GENERIC_NOTE $(basename "$file") :: $(task_summary "$file")"
      found=1
      continue
    fi
    if [[ "$status" == "blocked" && -z "$has_blocker" ]]; then
      echo "BLOCKED_WITHOUT_BLOCKER $(basename "$file") :: $(task_summary "$file")"
      found=1
      continue
    fi
  done < <(find "$TASK_ROOT/$w/done" -maxdepth 1 -type f -name '*.task' | sort -r)
done

if (( found == 0 )); then
  echo "no suspicious completions found"
fi
