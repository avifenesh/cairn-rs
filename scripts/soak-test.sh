#!/usr/bin/env bash
# =============================================================================
# soak-test.sh — Continuous dogfood loop for pre-release validation.
#
# Runs every INTERVAL seconds (default 300 = 5 min). Each iteration:
#   1. Health check
#   2. Create session + run + task (full lifecycle)
#   3. Claim → complete task → complete run
#   4. Approval gate (request + resolve)
#   5. Eval run (create → start → score → complete)
#   6. Memory ingest + search
#   7. Provider connection test
#   8. Read-back: dashboard, costs, events, traces, stats
#   9. Log result to soak-log.jsonl
#
# Exits on first hard failure (health unreachable). Soft failures (4xx on
# optional endpoints) are logged but don't stop the loop.
#
# Usage:
#   # Start soak test against local dev server
#   CAIRN_TOKEN=dev-admin-token ./scripts/soak-test.sh
#
#   # Custom interval (60s) and endpoint
#   CAIRN_URL=http://staging:3000 INTERVAL=60 ./scripts/soak-test.sh
#
#   # Run in background
#   nohup ./scripts/soak-test.sh &> /tmp/soak.log &
# =============================================================================

set -uo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-dev-admin-token}"
INTERVAL="${INTERVAL:-300}"
LOG_FILE="${SOAK_LOG:-/tmp/cairn-soak-log.jsonl}"
ITERATION=0
TOTAL_PASS=0
TOTAL_FAIL=0

# ── Helpers ──────────────────────────────────────────────────────────────────

api() {
  local method="$1" path="$2"; shift 2
  local body="${1:-}"
  local args=(-sf -w '\n%{http_code}' -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json")
  [ "$method" != "GET" ] && [ -n "$body" ] && args+=(-X "$method" -d "$body") || args+=(-X "$method")
  local output
  output=$(curl "${args[@]}" "${BASE}${path}" 2>/dev/null) || { echo "000"; return 1; }
  echo "$output"
}

check() {
  local label="$1" expect="$2" method="$3" path="$4"; shift 4
  local body="${1:-}"
  local raw
  raw=$(api "$method" "$path" "$body") || raw=$'000\n000'
  local http_code
  http_code=$(echo "$raw" | tail -1)
  local response_body
  response_body=$(echo "$raw" | sed '$d')
  if [ "$http_code" = "$expect" ]; then
    PASS=$((PASS + 1))
    return 0
  else
    FAIL=$((FAIL + 1))
    FAIL_DETAILS+=("$label: expected=$expect got=$http_code")
    return 1
  fi
}

log_iteration() {
  local status="$1" duration="$2"
  local ts
  ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  local fails_json="[]"
  if [ ${#FAIL_DETAILS[@]} -gt 0 ]; then
    fails_json=$(printf '%s\n' "${FAIL_DETAILS[@]}" | python3 -c "import sys,json; print(json.dumps([l.strip() for l in sys.stdin]))" 2>/dev/null || echo "[]")
  fi
  printf '{"ts":"%s","iter":%d,"pass":%d,"fail":%d,"duration_s":%d,"status":"%s","fails":%s}\n' \
    "$ts" "$ITERATION" "$PASS" "$FAIL" "$duration" "$status" "$fails_json" \
    >> "$LOG_FILE"
}

# ── Main loop ────────────────────────────────────────────────────────────────

echo "Cairn soak test starting"
echo "  endpoint:  $BASE"
echo "  interval:  ${INTERVAL}s"
echo "  log:       $LOG_FILE"
echo ""

while true; do
  ITERATION=$((ITERATION + 1))
  PASS=0
  FAIL=0
  FAIL_DETAILS=()
  START_TS=$(date +%s)

  SUFFIX="${ITERATION}_$(date +%s)_$(head -c4 /dev/urandom | xxd -p)"
  SID="soak_sess_${SUFFIX}"
  RID="soak_run_${SUFFIX}"
  TID="soak_task_${SUFFIX}"
  AID="soak_appr_${SUFFIX}"
  EID="soak_eval_${SUFFIX}"
  SCOPE='tenant_id=default&workspace_id=default&project_id=default'
  PROJECT='{"tenant_id":"default","workspace_id":"default","project_id":"default"}'

  # 1. Health
  if ! check "health" 200 GET /health; then
    echo "[$(date -u +%H:%M:%S)] iter=$ITERATION FATAL: server unreachable"
    log_iteration "fatal" 0
    exit 1
  fi

  # 2. Session + Run
  check "create-session" 201 POST /v1/sessions \
    "{\"session_id\":\"${SID}\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"}"
  check "create-run" 201 POST /v1/runs \
    "{\"session_id\":\"${SID}\",\"run_id\":\"${RID}\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"}"

  # 3. Task lifecycle
  check "append-task" 201 POST /v1/events/append \
    "[{\"event_id\":\"evt_${SUFFIX}\",\"source\":{\"source_type\":\"runtime\"},\"ownership\":{\"scope\":\"project\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"payload\":{\"event\":\"task_created\",\"project\":${PROJECT},\"task_id\":\"${TID}\",\"parent_run_id\":\"${RID}\",\"parent_task_id\":null,\"prompt_release_id\":null}}]"
  sleep 0.2
  check "claim-task" 200 POST "/v1/tasks/${TID}/claim" \
    "{\"worker_id\":\"soak_worker_${SUFFIX}\",\"lease_duration_ms\":30000}"
  check "complete-task" 200 POST "/v1/tasks/${TID}/complete" '{}'

  # 4. Approval gate
  check "append-approval" 201 POST /v1/events/append \
    "[{\"event_id\":\"evt_a_${SUFFIX}\",\"source\":{\"source_type\":\"runtime\"},\"ownership\":{\"scope\":\"project\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"payload\":{\"event\":\"approval_requested\",\"project\":${PROJECT},\"approval_id\":\"${AID}\",\"run_id\":\"${RID}\",\"task_id\":null,\"requirement\":\"required\"}}]"
  sleep 0.2
  check "resolve-approval" 200 POST "/v1/approvals/${AID}/resolve" \
    '{"decision":"approved","reason":"soak test"}'

  # 5. Verify run state (auto-completes when all tasks finish)
  check "get-run" 200 GET "/v1/runs/${RID}"

  # 5b. Real orchestration — LLM call via brain provider (skipped if no provider)
  ORCH_RID="soak_orch_${SUFFIX}"
  curl -s -X POST -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
    -d "{\"session_id\":\"${SID}\",\"run_id\":\"${ORCH_RID}\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"}" \
    "${BASE}/v1/runs" >/dev/null 2>&1
  ORCH_RESP=$(curl -s -w '\n%{http_code}' -X POST -H "Authorization: Bearer ${TOKEN}" -H "Content-Type: application/json" \
    -d "{\"goal\":\"What is ${ITERATION}+${ITERATION}? Answer with just the number.\",\"model_id\":\"openrouter/auto\",\"max_iterations\":1,\"timeout_ms\":15000}" \
    "${BASE}/v1/runs/${ORCH_RID}/orchestrate" 2>/dev/null)
  ORCH_HTTP=$(echo "$ORCH_RESP" | tail -1)
  if [ "$ORCH_HTTP" = "200" ]; then
    PASS=$((PASS + 1))
  elif [ "$ORCH_HTTP" = "503" ]; then
    : # No brain provider — skip silently
  else
    FAIL=$((FAIL + 1))
    FAIL_DETAILS+=("orchestrate: expected=200 got=$ORCH_HTTP")
  fi

  # 6. Eval lifecycle
  check "create-eval" 201 POST /v1/evals/runs \
    "{\"eval_run_id\":\"${EID}\",\"subject_kind\":\"prompt_release\",\"evaluator_type\":\"accuracy\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"}"
  check "start-eval" 200 POST "/v1/evals/runs/${EID}/start" '{}'
  check "score-eval" 200 POST "/v1/evals/runs/${EID}/score" \
    '{"metrics":{"accuracy":0.91,"latency_p50_ms":100}}'
  check "complete-eval" 200 POST "/v1/evals/runs/${EID}/complete" \
    '{"metrics":{"accuracy":0.91,"latency_p50_ms":100},"cost":0.02}'

  # 7. Memory
  check "ingest" 200 POST /v1/memory/ingest \
    "{\"source_id\":\"soak_src\",\"document_id\":\"soak_doc_${SUFFIX}\",\"content\":\"Soak test iteration ${ITERATION}. The quick brown fox.\",\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"}"
  check "search" 200 GET "/v1/memory/search?query_text=fox&${SCOPE}&limit=3"

  # 8. Read-back
  check "dashboard" 200 GET "/v1/dashboard?${SCOPE}"
  check "costs" 200 GET "/v1/costs?tenant_id=default"
  check "events" 200 GET "/v1/events/recent?limit=5"
  check "stats" 200 GET /v1/stats
  check "providers-health" 200 GET "/v1/providers/health?tenant_id=default"
  check "audit-log" 200 GET /v1/admin/audit-log

  # 9. Log result
  END_TS=$(date +%s)
  DURATION=$((END_TS - START_TS))
  TOTAL_PASS=$((TOTAL_PASS + PASS))
  TOTAL_FAIL=$((TOTAL_FAIL + FAIL))

  if [ "$FAIL" -eq 0 ]; then
    echo "[$(date -u +%H:%M:%S)] iter=$ITERATION pass=$PASS fail=0 (${DURATION}s) ✓"
    log_iteration "pass" "$DURATION"
  else
    echo "[$(date -u +%H:%M:%S)] iter=$ITERATION pass=$PASS fail=$FAIL (${DURATION}s) ✗"
    for detail in "${FAIL_DETAILS[@]}"; do
      echo "  ✗ $detail"
    done
    log_iteration "fail" "$DURATION"
  fi

  # Summary every 10 iterations
  if [ $((ITERATION % 10)) -eq 0 ]; then
    echo ""
    echo "── Soak summary after $ITERATION iterations ──"
    echo "  Total checks: $((TOTAL_PASS + TOTAL_FAIL))"
    echo "  Passed: $TOTAL_PASS"
    echo "  Failed: $TOTAL_FAIL"
    echo ""
  fi

  sleep "$INTERVAL"
done
