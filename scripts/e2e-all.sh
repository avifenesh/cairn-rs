#!/usr/bin/env bash
# =============================================================================
# e2e-all.sh — Master runner for all cairn-rs end-to-end test suites.
#
# Runs every e2e-*.sh script in priority order, collects results, and prints
# a unified report. Stops on first critical failure (P0) by default, or
# runs all with --continue-on-failure.
#
# Usage:
#   ./scripts/e2e-all.sh                          # stop on P0 failure
#   ./scripts/e2e-all.sh --continue-on-failure    # run everything
#   CAIRN_URL=http://localhost:3000 ./scripts/e2e-all.sh
#
# Exit code: 0 = all passed, 1 = one or more failures.
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CONTINUE_ON_FAILURE=false
[[ "${1:-}" == "--continue-on-failure" ]] && CONTINUE_ON_FAILURE=true

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"

# ── Colour ───────────────────────────────────────────────────────────────────
if [ -t 2 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'
  CYN='\033[0;36m'; BLD='\033[1m';   RST='\033[0m'
else
  GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''
fi

# ── Test suites in priority order ────────────────────────────────────────────
# P0: Core product value (must pass for first sale)
P0_SUITES=(
  "e2e-agent-workflow.sh:Agent lifecycle (existing)"
  "e2e-full-happy-path.sh:Solo dev happy path (UC-01)"
  "e2e-approval-gate.sh:Approval gate flow (UC-04)"
  "e2e-memory-to-agent.sh:Memory to agent pipeline (UC-06)"
  "e2e-multi-tool-orchestration.sh:Multi-tool orchestration (UC-03)"
)

# P1: Production readiness
P1_SUITES=(
  "e2e-persistence.sh:SQLite persistence (existing)"
  "e2e-multi-tenant.sh:Multi-tenant isolation (UC-08)"
  "e2e-provider-routing.sh:Provider routing (UC-10)"
  "e2e-fleet-monitoring.sh:Fleet monitoring (UC-14)"
  "e2e-checkpoint-recovery.sh:Checkpoint recovery (UC-07)"
)

# P2: Differentiation features
P2_SUITES=(
  "e2e-subagent.sh:Sub-agent spawning (UC-05)"
  "e2e-prompt-lifecycle.sh:Prompt versioning (UC-12)"
  "e2e-eval-pipeline.sh:Eval comparison (UC-13)"
  "e2e-sse-streaming.sh:SSE streaming (UC-20)"
)

# P3: Operational maturity
P3_SUITES=(
  "e2e-cost-tracking.sh:Cost tracking (UC-15)"
  "e2e-audit-trail.sh:Audit trail (UC-19)"
  "e2e-orchestrator-mock.sh:Orchestrator plumbing (existing)"
  "e2e-memory-pipeline.sh:Memory pipeline (existing)"
)

# ── Runner ───────────────────────────────────────────────────────────────────

TOTAL=0; PASSED=0; FAILED=0; SKIPPED=0
FAILURES=()
START_TIME=$(date +%s)

run_suite() {
  local script="$1" label="$2" priority="$3"
  local path="${SCRIPT_DIR}/${script}"

  TOTAL=$(( TOTAL + 1 ))

  if [ ! -f "$path" ]; then
    echo -e "  ${YLW}SKIP${RST}  ${label}  ${YLW}(${script} not found)${RST}" >&2
    SKIPPED=$(( SKIPPED + 1 ))
    return 0
  fi

  if [ ! -x "$path" ]; then
    chmod +x "$path"
  fi

  echo -e "\n${BLD}${CYN}━━━ [${priority}] ${label} ━━━${RST}" >&2

  set +e
  CAIRN_URL="$BASE" CAIRN_TOKEN="$TOKEN" bash "$path" 2>&1
  local rc=$?
  set -e

  if [ $rc -eq 0 ]; then
    echo -e "  ${GRN}PASS${RST}  ${label}" >&2
    PASSED=$(( PASSED + 1 ))
  else
    echo -e "  ${RED}FAIL${RST}  ${label}  (exit code ${rc})" >&2
    FAILED=$(( FAILED + 1 ))
    FAILURES+=("${priority}: ${label}")

    if [[ "$priority" == "P0" ]] && ! $CONTINUE_ON_FAILURE; then
      echo -e "\n${RED}${BLD}P0 suite failed — stopping.${RST} Use --continue-on-failure to run all." >&2
      return 1
    fi
  fi

  return 0
}

run_tier() {
  local priority="$1"
  shift
  local -a suites=("$@")

  echo -e "\n${BLD}══════════════════════════════════════════════════════════════${RST}" >&2
  echo -e "${BLD}  ${priority} SUITES${RST}" >&2
  echo -e "${BLD}══════════════════════════════════════════════════════════════${RST}" >&2

  for entry in "${suites[@]}"; do
    local script="${entry%%:*}"
    local label="${entry#*:}"
    if ! run_suite "$script" "$label" "$priority"; then
      return 1
    fi
  done
}

# ── Health check ─────────────────────────────────────────────────────────────

echo -e "${BLD}cairn-rs E2E Test Runner${RST}" >&2
echo -e "  Server: ${CYN}${BASE}${RST}" >&2
echo -e "  Mode:   ${CYN}$( $CONTINUE_ON_FAILURE && echo 'continue-on-failure' || echo 'stop-on-P0-failure' )${RST}" >&2
echo "" >&2

HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" --max-time 5 "${BASE}/health" 2>/dev/null || echo "000")
if [ "$HTTP_STATUS" != "200" ]; then
  echo -e "${RED}${BLD}Server not reachable at ${BASE} (HTTP ${HTTP_STATUS})${RST}" >&2
  echo -e "Start the server first: CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app" >&2
  exit 1
fi
echo -e "  ${GRN}Server healthy${RST}" >&2

# ── Run all tiers ────────────────────────────────────────────────────────────

set +e
run_tier "P0" "${P0_SUITES[@]}"
P0_RC=$?
if [ $P0_RC -eq 0 ]; then
  run_tier "P1" "${P1_SUITES[@]}"
  run_tier "P2" "${P2_SUITES[@]}"
  run_tier "P3" "${P3_SUITES[@]}"
fi
set -e

# ── Report ───────────────────────────────────────────────────────────────────

END_TIME=$(date +%s)
DURATION=$(( END_TIME - START_TIME ))

echo "" >&2
echo -e "${BLD}══════════════════════════════════════════════════════════════${RST}" >&2
echo -e "${BLD}  E2E RESULTS${RST}" >&2
echo -e "${BLD}══════════════════════════════════════════════════════════════${RST}" >&2
echo -e "  Total:   ${TOTAL}" >&2
echo -e "  ${GRN}Passed:  ${PASSED}${RST}" >&2
echo -e "  ${RED}Failed:  ${FAILED}${RST}" >&2
echo -e "  ${YLW}Skipped: ${SKIPPED}${RST}" >&2
echo -e "  Time:    ${DURATION}s" >&2

if [ ${#FAILURES[@]} -gt 0 ]; then
  echo "" >&2
  echo -e "  ${RED}${BLD}Failures:${RST}" >&2
  for f in "${FAILURES[@]}"; do
    echo -e "    ${RED}• ${f}${RST}" >&2
  done
fi

echo "" >&2

if [ "$FAILED" -gt 0 ]; then
  exit 1
else
  exit 0
fi
