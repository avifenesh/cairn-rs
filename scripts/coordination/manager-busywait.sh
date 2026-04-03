#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  manager-busywait.sh [--interval 2] [--threshold 6]

Keeps a standing pending-task buffer for every worker by enqueueing
low-risk evergreen follow-up tasks whenever pending count drops below
the threshold.
EOF
}

interval=2
threshold=6

while [[ $# -gt 0 ]]; do
  case "$1" in
    --interval)
      interval="${2:-2}"
      shift 2
      ;;
    --threshold)
      threshold="${2:-6}"
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

task_exists() {
  local worker summary file
  worker="$(normalize_worker "$1")"
  summary="$2"
  for file in "$TASK_ROOT/$worker"/pending/*.task "$TASK_ROOT/$worker"/claimed/*.task; do
    [[ -f "$file" ]] || continue
    if [[ "$(task_summary "$file")" == "$summary" ]]; then
      return 0
    fi
  done
  return 1
}

next_index_file() {
  local worker
  worker="$(normalize_worker "$1")"
  printf '%s\n' "$STATE_ROOT/${worker}.next-index"
}

worker_task_pool() {
  local worker
  worker="$(normalize_worker "$1")"
  case "$worker" in
    worker-1)
      cat <<'EOF'
Refresh worker slice health after the latest completions and route any new seam drift to the right owner
Re-run compatibility report generation after the latest API and SSE alignment changes and confirm phase0 reports stay in sync
Audit queue-bus manager noise for stale replay events and tighten reporting only if it hides real refill signals
Compare the queue-driven manager loop against mailbox guidance and note any mismatch before it becomes stale coordination debt
Check whether fast worker turnover is outrunning the current health-report cadence and document the next smallest manager fix if so
Review the newest cross-worker handoff points and flag only ownership drift that could confuse the next queue refill
Sanity-check whether current queue health still matches the latest repo state and capture only a concrete manager correction if not
Inspect the last wave of completions for repeated no-op work and suggest one small refinement to keep worker tasks sharper
EOF
      ;;
    worker-2)
      cat <<'EOF'
Review whether Worker 1 or Worker 8 surfaced any real shared-type blocker during the latest API and compatibility pass
Audit the newest downstream domain usage for convenience-only helpers creeping back in and push back if needed
If no seam is blocked stay in contract-freeze support mode and document the no-op outcome
Scan the latest runtime and API changes for domain helper duplication and only intervene if the contract boundary actually drifted
Review whether any new lifecycle or provider types should stay downstream instead of being pulled back into shared contracts
Check whether recent integrations introduced naming drift between domain contracts and API surfaces and capture only one concrete mismatch
Audit one recently touched domain seam for accidental policy leakage and stop once the boundary is confirmed
Review the latest downstream ownership of selectors, prompts, or provider types and only pull back what clearly belongs in shared contracts
EOF
      ;;
    worker-3)
      cat <<'EOF'
Add one more deterministic cross-backend ordering test for a Worker-8-facing read or query surface
Fold the newest rebuild and ordering assumptions into migration-check or parity tooling so regressions fail earlier
If parity stays green audit one store-backed surface touched by external-worker progress or mailbox reads for backend drift
Take one more narrow cross-backend regression around replay cursor stability or current-state rebuild ordering and stop at the first proven gap
If backend parity still holds add one lightweight store-facing assertion that helps Worker 8 trust read ordering without reading store internals
Check one newly touched projection or backfill path for backend-specific assumptions before it leaks into API-facing behavior
Verify one mailbox or external-worker read path still orders consistently across backends after the latest completions
Audit whether replay cursor or resume expectations changed under the latest store work and capture only the smallest needed test
EOF
      ;;
    worker-4)
      cat <<'EOF'
Stay on final seam-watch with Worker 8 and take only the next smallest runtime fix if store-backed enrichment drift appears
If no drift appears add one lightweight guard or doc-level contract check around RuntimeEnrichment consumption and stop
If Worker 8 reports progress or approval mismatch add only that exact replay-or-current-state regression
Check whether the latest store-backed runtime seams changed progress or approval payload expectations before API hardening drifts
Add one narrow regression around runtime enrichment lookup or replay recovery only if it protects an already-used API or SSE path
Verify one pause, resume, or timeout-facing runtime seam still matches the latest API expectations without widening lifecycle scope
Check whether one current-state read used by API or SSE is still runtime-owned rather than re-derived above the seam
Audit the newest runtime-facing enrichment use for hidden dependency on store internals and add only the smallest guard if needed
EOF
      ;;
    worker-5)
      cat <<'EOF'
Stay on tool-path seam watch and take only the next smallest runtime-to-tools-to-API handoff fix a downstream worker reports
Add one final guard that downstream API or SSE shaping does not bypass ToolLifecycleOutput semantics
If the seam stays green add one lightweight contract assertion for assistant_tool_call shaping and stop
Verify one negative-path assistant_tool_call case still preserves lifecycle and permission coherence after the newest API changes
Check one store-backed or replay-backed tool path for drift between runtime output and API-facing tool lifecycle shaping
If no regression appears add one focused test or assertion that protects tool outcome coherence without widening plugin scope
Check whether one permission or policy edge case still flows through the same tool lifecycle seam after the latest API work
Verify one non-happy-path tool outcome still surfaces the right operator-facing shape without bypassing runtime-owned semantics
EOF
      ;;
    worker-6)
      cat <<'EOF'
Keep MemoryApiImpl FeedEndpoints and provenance seams honest while Worker 8 consumes them in API paths
Add one representative provenance-or-search integration proof only if the current HTTP-facing read seam exposes a gap
If Worker 8 finds no drift stay in support mode and avoid widening the retrieval model
Check whether one more memory or provenance read path can be proven executable from the API surface without adding new retrieval concepts
If the seam is still stable add one narrow guard that feed or provenance shaping remains backed by real services rather than route-local shaping
Verify one feed-facing or deep-search-facing read still composes through real memory services after the latest router changes
Audit whether a recent provenance or retrieval touch introduced route-local shaping and capture only the smallest correction if so
Confirm one search or bundle-related read can still be exercised without adding new product-facing retrieval scope
EOF
      ;;
    worker-7)
      cat <<'EOF'
Stay in final agent-evals support mode and take only the next smallest API-facing release scorecard graph or streaming mismatch Worker 8 reports
If no mismatch appears add one lightweight guard against re-deriving prompt or eval semantics above the direct API seam and stop
Keep rollout and scorecard scope closed unless Worker 8 surfaces a concrete integration blocker
Check whether the latest API-facing graph or scorecard work drifted from direct eval ownership and add only the smallest guard if it did
Verify one prompt-release or selector-facing surface still lines up with eval scorecard expectations after the newest downstream changes
Audit one graph or scorecard-facing API seam for accidental duplication of eval logic and stop at the first concrete example
Check whether one streaming output path still lines up with agent or eval ownership after the latest API changes
Confirm one selector-driven release surface remains consistent with scorecard and graph expectations without widening rollout scope
EOF
      ;;
    worker-8)
      cat <<'EOF'
Take the next smallest API or SSE mismatch reported by Workers 4 5 6 or 7 using only existing service seams
If the product-glue pass stays green add one last operator-facing read or SSE consumption proof without widening API scope
Remain on integration-watch duty and stop before inventing new API breadth
Prove one more composed app surface stays wired through real services after the latest downstream completions and stop before adding new routes
Take one more narrow SSE enrichment or operator-facing read proof only if it uses already-stable runtime and memory seams
Check whether any latest worker completions changed API boundary assumptions and capture only the smallest needed alignment fix
Verify one operator-facing read or route composition still matches the newest runtime and memory seams without adding breadth
Audit one SSE family for hidden route-local shaping and add only the smallest correction if it drifted from service-backed data
EOF
      ;;
  esac
}

queue_next_from_pool() {
  local worker idx_file idx count summary queued="0"
  local -a pool=()

  worker="$(normalize_worker "$1")"
  mapfile -t pool < <(worker_task_pool "$worker")
  count="${#pool[@]}"
  (( count > 0 )) || return 1

  idx_file="$(next_index_file "$worker")"
  idx=0
  if [[ -f "$idx_file" ]]; then
    idx="$(cat "$idx_file" 2>/dev/null || printf '0')"
  fi
  [[ "$idx" =~ ^[0-9]+$ ]] || idx=0
  idx=$(( idx % count ))

  local attempts=0
  while (( attempts < count )); do
    summary="${pool[$idx]}"
    idx=$(( (idx + 1) % count ))
    attempts=$(( attempts + 1 ))
    if ! task_exists "$worker" "$summary"; then
      "$script_dir/queue-worker-tasks.sh" --from manager "$worker" "$summary" >/dev/null
      queued="1"
      break
    fi
  done

  printf '%s\n' "$idx" > "$idx_file"
  [[ "$queued" == "1" ]]
}

refill_worker() {
  local worker pending
  worker="$(normalize_worker "$1")"

  while true; do
    pending="$(pending_count "$worker")"
    if (( pending >= threshold )); then
      break
    fi
    if ! queue_next_from_pool "$worker"; then
      break
    fi
  done
}

echo "manager busywait active interval=${interval}s threshold=${threshold}"
while true; do
  for worker in worker-1 worker-2 worker-3 worker-4 worker-5 worker-6 worker-7 worker-8; do
    refill_worker "$worker"
  done
  sleep "$interval"
done
