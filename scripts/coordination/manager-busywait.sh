#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  manager-busywait.sh [--interval 2] [--threshold 4]

Keeps a standing pending-task buffer for every worker by enqueueing
low-risk evergreen follow-up tasks whenever pending count drops below
the threshold.
EOF
}

interval=2
threshold=4

while [[ $# -gt 0 ]]; do
  case "$1" in
    --interval)
      interval="${2:-2}"
      shift 2
      ;;
    --threshold)
      threshold="${2:-4}"
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

queue_if_missing() {
  local worker summary
  worker="$(normalize_worker "$1")"
  summary="$2"
  if ! task_exists "$worker" "$summary"; then
    "$script_dir/queue-worker-tasks.sh" --from manager "$worker" "$summary" >/dev/null
  fi
}

refill_worker() {
  local worker pending
  worker="$(normalize_worker "$1")"
  pending="$(pending_count "$worker")"
  if (( pending >= threshold )); then
    return 0
  fi

  case "$worker" in
    worker-1)
      queue_if_missing "$worker" "Refresh worker slice health after the latest completions and route any new seam drift to the right owner"
      queue_if_missing "$worker" "Re-run compatibility report generation after the latest API and SSE alignment changes and confirm phase0 reports stay in sync"
      queue_if_missing "$worker" "Audit queue-bus manager noise for stale replay events and tighten reporting only if it hides real refill signals"
      queue_if_missing "$worker" "Compare the queue-driven manager loop against mailbox guidance and note any mismatch before it becomes stale coordination debt"
      queue_if_missing "$worker" "Check whether fast worker turnover is outrunning the current health-report cadence and document the next smallest manager fix if so"
      ;;
    worker-2)
      queue_if_missing "$worker" "Review whether Worker 1 or Worker 8 surfaced any real shared-type blocker during the latest API and compatibility pass"
      queue_if_missing "$worker" "Audit the newest downstream domain usage for convenience-only helpers creeping back in and push back if needed"
      queue_if_missing "$worker" "If no seam is blocked stay in contract-freeze support mode and document the no-op outcome"
      queue_if_missing "$worker" "Scan the latest runtime and API changes for domain helper duplication and only intervene if the contract boundary actually drifted"
      queue_if_missing "$worker" "Review whether any new lifecycle or provider types should stay downstream instead of being pulled back into shared contracts"
      ;;
    worker-3)
      queue_if_missing "$worker" "Add one more deterministic cross-backend ordering test for a Worker-8-facing read or query surface"
      queue_if_missing "$worker" "Fold the newest rebuild and ordering assumptions into migration-check or parity tooling so regressions fail earlier"
      queue_if_missing "$worker" "If parity stays green audit one store-backed surface touched by external-worker progress or mailbox reads for backend drift"
      queue_if_missing "$worker" "Take one more narrow cross-backend regression around replay cursor stability or current-state rebuild ordering and stop at the first proven gap"
      queue_if_missing "$worker" "If backend parity still holds add one lightweight store-facing assertion that helps Worker 8 trust read ordering without reading store internals"
      ;;
    worker-4)
      queue_if_missing "$worker" "Stay on final seam-watch with Worker 8 and take only the next smallest runtime fix if store-backed enrichment drift appears"
      queue_if_missing "$worker" "If no drift appears add one lightweight guard or doc-level contract check around RuntimeEnrichment consumption and stop"
      queue_if_missing "$worker" "If Worker 8 reports progress or approval mismatch add only that exact replay-or-current-state regression"
      queue_if_missing "$worker" "Check whether the latest store-backed runtime seams changed progress or approval payload expectations before API hardening drifts"
      queue_if_missing "$worker" "Add one narrow regression around runtime enrichment lookup or replay recovery only if it protects an already-used API or SSE path"
      ;;
    worker-5)
      queue_if_missing "$worker" "Stay on tool-path seam watch and take only the next smallest runtime-to-tools-to-API handoff fix a downstream worker reports"
      queue_if_missing "$worker" "Add one final guard that downstream API or SSE shaping does not bypass ToolLifecycleOutput semantics"
      queue_if_missing "$worker" "If the seam stays green add one lightweight contract assertion for assistant_tool_call shaping and stop"
      queue_if_missing "$worker" "Verify one negative-path assistant_tool_call case still preserves lifecycle and permission coherence after the newest API changes"
      queue_if_missing "$worker" "Check one store-backed or replay-backed tool path for drift between runtime output and API-facing tool lifecycle shaping"
      queue_if_missing "$worker" "If no regression appears add one focused test or assertion that protects tool outcome coherence without widening plugin scope"
      ;;
    worker-6)
      queue_if_missing "$worker" "Keep MemoryApiImpl FeedEndpoints and provenance seams honest while Worker 8 consumes them in API paths"
      queue_if_missing "$worker" "Add one representative provenance-or-search integration proof only if the current HTTP-facing read seam exposes a gap"
      queue_if_missing "$worker" "If Worker 8 finds no drift stay in support mode and avoid widening the retrieval model"
      queue_if_missing "$worker" "Check whether one more memory or provenance read path can be proven executable from the API surface without adding new retrieval concepts"
      queue_if_missing "$worker" "If the seam is still stable add one narrow guard that feed or provenance shaping remains backed by real services rather than route-local shaping"
      ;;
    worker-7)
      queue_if_missing "$worker" "Stay in final agent-evals support mode and take only the next smallest API-facing release scorecard graph or streaming mismatch Worker 8 reports"
      queue_if_missing "$worker" "If no mismatch appears add one lightweight guard against re-deriving prompt or eval semantics above the direct API seam and stop"
      queue_if_missing "$worker" "Keep rollout and scorecard scope closed unless Worker 8 surfaces a concrete integration blocker"
      queue_if_missing "$worker" "Check whether the latest API-facing graph or scorecard work drifted from direct eval ownership and add only the smallest guard if it did"
      queue_if_missing "$worker" "Verify one prompt-release or selector-facing surface still lines up with eval scorecard expectations after the newest downstream changes"
      ;;
    worker-8)
      queue_if_missing "$worker" "Take the next smallest API or SSE mismatch reported by Workers 4 5 6 or 7 using only existing service seams"
      queue_if_missing "$worker" "If the product-glue pass stays green add one last operator-facing read or SSE consumption proof without widening API scope"
      queue_if_missing "$worker" "Remain on integration-watch duty and stop before inventing new API breadth"
      queue_if_missing "$worker" "Prove one more composed app surface stays wired through real services after the latest downstream completions and stop before adding new routes"
      queue_if_missing "$worker" "Take one more narrow SSE enrichment or operator-facing read proof only if it uses already-stable runtime and memory seams"
      queue_if_missing "$worker" "Check whether any latest worker completions changed API boundary assumptions and capture only the smallest needed alignment fix"
      ;;
  esac
}

echo "manager busywait active interval=${interval}s threshold=${threshold}"
while true; do
  for worker in worker-1 worker-2 worker-3 worker-4 worker-5 worker-6 worker-7 worker-8; do
    refill_worker "$worker"
  done
  sleep "$interval"
done
