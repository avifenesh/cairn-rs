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
Regenerate the SSE migration artifacts and remove any rows that still claim task_update approval_required assistant_tool_call or agent_progress need payload alignment when executable tests already prove exact fixture parity
Compare phase0_sse_publisher_gap_report.md phase0_sse_payload_handoff.md and phase0_owner_map.md against current sse_payload_alignment.rs assertions and land the smallest generator or report fix needed to make them truthful again
Keep the honest remaining SSE gaps explicit for feed_update assistant_end caller assembly and memory_proposed ownership instead of re-proving already-exact builder payloads
Refresh compatibility artifacts only after checking which exact generated row is stale relative to cairn-api tests and update that concrete mismatch instead of another broad report sweep
Take one preserved API or SSE contract gap that is still real and convert it into either a stronger executable guard or a corrected generated artifact, but do not queue another generic proof pass
Audit one migration report row against current executable tests and stop as soon as one stale claim or one still-open honest gap is made explicit
Add one compatibility guard only if it protects a real remaining feed memory or overview drift seam; otherwise spend the pass removing stale generated-report noise
Treat queue or mailbox work as secondary unless it directly hides a compatibility truthfulness issue; prioritize report accuracy over another status refresh
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
Fix CheckpointReadModel::list_by_run ordering so InMemory SQLite and Postgres all return checkpoints in the same deterministic order and keep the parity test green
After the checkpoint fix rerun cross_backend_parity and take the next concrete ordering mismatch only if one still exists; stop at the first real failing surface
Fold the checkpoint ordering rule into the smallest backend guard or comment needed so Worker 8 can trust read ordering without route-level re-sorting
If parity is green after the checkpoint fix take one more replay or rebuild ordering edge case that still affects API-facing reads and no broader store refactor
Check whether any current mailbox approval or tool_invocation ordering assumptions now differ from the fixed checkpoint rule and capture only one concrete follow-up if so
Audit one projection rebuild edge case around checkpoint latest-vs-list ordering and make the contract explicit where the adapters and in-memory store must agree
Use the next store pass on a failing or newly exposed parity seam only; avoid another generic parity sweep if the test suite is already honest
Take one API-facing read model that depends on deterministic order and prove it still works after the checkpoint parity fix without widening backend behavior
EOF
      ;;
    worker-4)
      cat <<'EOF'
Replace the recover_interrupted_runs placeholder with the smallest real scan-and-recover path supported by current run or checkpoint read models
Replace the resolve_stale_dependencies placeholder with either a real dependency-resolution pass or an explicit trait/query blocker that names the exact missing store seam
Add one focused integration test for interrupted-run recovery so the method stops returning scanned=0 with no action by default
Add one focused integration test for stale-dependency resolution or for the explicit blocker path so the runtime contract is honest instead of silent
If one of the recovery methods truly cannot be implemented yet capture the exact missing read-model query in code and mailbox form instead of leaving a generic placeholder
After recovery placeholders are closed recheck only the nearest API or SSE seam that consumes those runtime states; do not widen recovery behavior beyond that
Keep runtime seam-watch duty secondary until the placeholder methods are either implemented or turned into explicit blocked contracts
Take one timeout pause or resume regression only if it becomes the next failing surface after the recovery placeholder work is real
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
Implement the smallest real submit_pack path so knowledge-pack ingest stops returning not yet implemented and can flow through the existing RFC 013 bundle types
Make RetrievalMode behavior honest by either implementing the minimal vector or hybrid path now or tightening the service contract and tests so Hybrid no longer silently means lexical-only
If submit_pack is too large for one pass wire the first bundle parsing and pipeline handoff step and leave an explicit bounded blocker instead of a generic internal error
Check whether the current API-facing memory routes need their tests updated once submit_pack or retrieval mode behavior becomes real and stop at the first concrete seam
Keep provenance and route wiring secondary until the hard not-implemented paths in pipeline.rs and pg/retrieval.rs are either implemented or turned into explicit narrowed contracts
Take one feed or provenance follow-up only after the memory core stops hard-failing on knowledge packs and ambiguous retrieval modes
If vector work is still deferred make the exact fallback behavior explicit in diagnostics and tests rather than letting callers infer full hybrid support
Use the next pass on a real ingest or retrieval capability gap, not another generic support proof for routes that already exercise real services
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
Close one honest HTTP contract gap by expanding either feed or memory response shaping to match the preserved Phase 0 fixture exactly
After the HTTP gap pick the matching adjacent SSE follow-up: richer feed_update envelope, caller-assembled assistant_end final text, or memory_proposed ownership and builder path
Stop after moving one API path and one adjacent SSE family from explicit gap to explicit coverage; do not queue another generic proof pass
Use existing runtime memory and eval seams only; the next API pass should remove one real boundary gap, not add breadth
If feed is chosen close both the HTTP item shape and the feed_update envelope together enough that Worker 1 can remove that row from the generated gap report
If memory is chosen close the HTTP search item shape and then decide whether memory_proposed should be runtime-owned or dedicated-builder-owned so the report can stop saying owner missing
If assistant_end is the next smallest SSE follow-up make the caller-assembled final text path real in the API composition layer and keep the report/tests aligned
After one real contract gap is closed rerun only the nearest API and compatibility tests and stop at the first newly exposed mismatch
Avoid another composition-only proof unless it is tied to one remaining preserved response or payload gap
Treat product-glue work as close one explicit gap at a time until Worker 1’s generated reports shrink for real
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
