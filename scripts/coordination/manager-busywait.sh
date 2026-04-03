#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/lib.sh"

usage() {
  cat <<'EOF'
Usage:
  manager-busywait.sh [--interval 1] [--threshold 6]

Keeps a standing pending-task buffer for every worker by enqueueing
low-risk evergreen follow-up tasks whenever pending count drops below
the threshold.
EOF
}

interval=1
threshold=6

while [[ $# -gt 0 ]]; do
  case "$1" in
    --interval)
      interval="${2:-1}"
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

lock_dir="$STATE_ROOT/manager-busywait.lock"
if ! mkdir "$lock_dir" 2>/dev/null; then
  echo "manager busywait already running" >&2
  exit 0
fi
cleanup_lock() {
  rmdir "$lock_dir" 2>/dev/null || true
}
trap cleanup_lock EXIT INT TERM

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

recent_done_task_exists() {
  local worker summary limit file
  worker="$(normalize_worker "$1")"
  summary="$2"
  limit="${3:-0}"
  (( limit > 0 )) || return 1

  while read -r file; do
    [[ -f "$file" ]] || continue
    if [[ "$(task_summary "$file")" == "$summary" ]]; then
      return 0
    fi
  done < <(find "$TASK_ROOT/$worker/done" -maxdepth 1 -type f -name '*.task' | sort | tail -n "$limit")
  return 1
}

next_index_file() {
  local worker
  worker="$(normalize_worker "$1")"
  printf '%s\n' "$STATE_ROOT/${worker}.next-index"
}

worker_cycle_file() {
  local worker
  worker="$(normalize_worker "$1")"
  printf '%s\n' "$STATE_ROOT/${worker}.cycle"
}

worker_threshold() {
  local worker
  worker="$(normalize_worker "$1")"
  case "$worker" in
    worker-3) printf '%s\n' 10 ;;
    worker-4) printf '%s\n' 10 ;;
    worker-5) printf '%s\n' 30 ;;
    worker-8) printf '%s\n' 80 ;;
    *) printf '%s\n' "$threshold" ;;
  esac
}

worker_recent_done_limit() {
  local worker
  worker="$(normalize_worker "$1")"
  case "$worker" in
    worker-1|worker-2) printf '%s\n' 1 ;;
    *) printf '%s\n' 0 ;;
  esac
}

worker_task_pool() {
  local worker
  worker="$(normalize_worker "$1")"
  case "$worker" in
    worker-1)
      cat <<'EOF'
Turn one current Phase 0 HTTP or SSE report gap into an executable cairn-api test or explicit generated artifact update
Re-run compatibility generation and capture the exact files that changed, then narrow that drift to one concrete owner or test gap
Audit the current migration README owner map and generated reports for stale prose, then update one concrete mismatch instead of only noting it
Add one compatibility guard that would fail if feed, memory, or overview route shaping drifts from the preserved Phase 0 contract
Compare the newest API/SSE tests against generated migration artifacts and tighten whichever side is lagging behind
Take one explicit response-shape or payload-shape gap from the generated reports and convert it into a reproducible assertion or fixture refresh
Refresh worker slice health only after checking which concrete crate or seam changed, then record that exact change instead of a general status sweep
Audit the queue system only if it hides compatibility signal; otherwise spend the pass on a concrete preserved-surface truthfulness check
EOF
      ;;
    worker-2)
      cat <<'EOF'
Scan runtime, store, tools, and API for one concrete duplicate of a cairn-domain helper or invariant and either remove it or record the exact blocker
Add or tighten one cairn-domain regression test for a lifecycle, tool-invocation, selector, or envelope invariant that downstream crates now depend on
Audit one newly touched downstream seam for domain-boundary leakage and capture the exact helper, type, or validator that should move or stay put
Check the latest API/runtime/store changes for envelope or ownership assembly outside cairn-domain and identify one concrete remaining offender
Review one prompt, provider, or selector path end-to-end and verify the shared domain contract is still the only source of truth
Take one downstream contract ambiguity surfaced by current worktree changes and resolve it into either a domain test, a helper move, or a written blocker
Inspect one current callsite that still re-derives domain semantics locally and replace or flag that exact spot instead of doing another broad audit
Verify one recent domain-facing integration uses cairn-domain builders or validators directly, and if not, name the exact missing adoption point
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
Confirm one worker-facing API consumer can still rely on store ordering without re-sorting above the backend seam
Check one projection rebuild edge case around completion or approval state and capture only the smallest backend-specific guard
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
Confirm one progress or approval enrichment payload still composes correctly when the store-backed path is exercised repeatedly
Audit one external-worker-facing runtime seam for drift under the latest API consumption and capture only the smallest corrective guard
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
Confirm one tool lifecycle payload still holds up under repeated claim/complete churn from fast API-facing consumers
Check one assistant tool path for idempotent operator-facing shaping after the latest downstream completions
Audit one runtime-to-tools seam for accidental duplication in API or SSE tests and stop at the first concrete example
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
Check one memory-backed operator read for stable provenance attachment after the latest Worker 8 integration passes
Verify one feed or poll-facing path still consumes memory services directly instead of rebuilding shape in the route layer
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
Verify one API-facing evaluation read still composes through the expected ownership seam after rapid downstream churn
Audit one graph-projection consumer for subtle drift from direct eval semantics and capture only the smallest needed guard
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
Check one feed or overview-facing composed route for drift after the latest runtime and memory completions
Verify one approval or progress SSE payload still reflects the authoritative service seam under rapid downstream churn
Audit one operator-facing surface for repeated no-op proof work and capture the next smallest real integration check instead
Confirm one memory-backed or eval-backed route still uses direct service composition rather than test-only shaping
Bundle one composed app check plus one SSE check together and stop once both are proven or one concrete mismatch is found
Take one operator-facing read path and one adjacent SSE family together so the next pass covers composition instead of a single micro-proof
Verify one memory-backed route plus one overview or feed route still align after the latest downstream changes using only existing service seams
Check one approval or progress SSE family together with its nearest operator-facing read so drift is caught as a pair rather than in isolation
Audit one runtime-fed API path plus one memory-fed API path in the same pass and capture only the smallest shared boundary issue
Confirm one end-to-end product-glue slice remains coherent across route composition and SSE emission without adding any new breadth
EOF
      ;;
  esac
}

queue_next_from_pool() {
  local worker idx_file cycle_file idx cycle count summary rendered queued="0" recent_done_limit
  local -a pool=()

  worker="$(normalize_worker "$1")"
  mapfile -t pool < <(worker_task_pool "$worker")
  count="${#pool[@]}"
  (( count > 0 )) || return 1
  recent_done_limit="$(worker_recent_done_limit "$worker")"

  idx_file="$(next_index_file "$worker")"
  cycle_file="$(worker_cycle_file "$worker")"
  idx=0
  cycle=1
  if [[ -f "$idx_file" ]]; then
    idx="$(cat "$idx_file" 2>/dev/null || printf '0')"
  fi
  if [[ -f "$cycle_file" ]]; then
    cycle="$(cat "$cycle_file" 2>/dev/null || printf '1')"
  fi
  [[ "$idx" =~ ^[0-9]+$ ]] || idx=0
  [[ "$cycle" =~ ^[0-9]+$ ]] || cycle=1
  idx=$(( idx % count ))

  local attempts=0
  while (( attempts < count )); do
    summary="${pool[$idx]}"
    idx=$(( (idx + 1) % count ))
    if (( idx == 0 )); then
      cycle=$(( cycle + 1 ))
    fi
    attempts=$(( attempts + 1 ))
    rendered="$summary"
    if [[ "$worker" == "worker-5" || "$worker" == "worker-8" ]]; then
      rendered="Batch ${cycle}: $summary"
    fi
    if ! task_exists "$worker" "$rendered" && ! recent_done_task_exists "$worker" "$rendered" "$recent_done_limit"; then
      "$script_dir/queue-worker-tasks.sh" --from manager "$worker" "$rendered" >/dev/null
      queued="1"
      break
    fi
  done

  printf '%s\n' "$idx" > "$idx_file"
  printf '%s\n' "$cycle" > "$cycle_file"
  [[ "$queued" == "1" ]]
}

refill_worker() {
  local worker pending target
  worker="$(normalize_worker "$1")"
  target="$(worker_threshold "$worker")"

  while true; do
    pending="$(pending_count "$worker")"
    if (( pending >= target )); then
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
