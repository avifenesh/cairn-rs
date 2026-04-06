#!/usr/bin/env bash
# =============================================================================
# cairn smoke test — verifies the full API surface against a running server.
#
# Usage:
#   ./scripts/smoke-test.sh
#   CAIRN_URL=http://my-server:3000 CAIRN_TOKEN=my-token ./scripts/smoke-test.sh
#
# Exit code: 0 = all passed, 1 = one or more failures.
# =============================================================================

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

RUN_ID="smoke_$(date +%s)_$RANDOM"
SESSION_ID="sess_${RUN_ID}"
WORKER_ID="worker_${RUN_ID}"
TASK_ID="task_${RUN_ID}"
APPR_ID="appr_${RUN_ID}"

# ── Colour ────────────────────────────────────────────────────────────────────
if [ -t 2 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'
  CYN='\033[0;36m'; BLD='\033[1m';   RST='\033[0m'
else
  GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''
fi

PASS=0; FAIL=0; SKIP=0

# All output to stderr — stdout is pure JSON for pipeline use.
log_ok()   { echo -e "${GRN}  ✓${RST} $1" >&2; PASS=$(( PASS + 1 )); }
log_fail() { echo -e "${RED}  ✗${RST} $1" >&2; FAIL=$(( FAIL + 1 )); }
log_skip() { echo -e "${YLW}  ⊘${RST} $1" >&2; SKIP=$(( SKIP + 1 )); }
section()  { echo -e "\n${BLD}${CYN}── $1${RST}" >&2; }

# ── HTTP primitives ───────────────────────────────────────────────────────────
# Use a tmpfile so status is NOT captured in a subshell.
_BODY_FILE=$(mktemp)
trap 'rm -f "$_BODY_FILE"' EXIT

# api METHOD PATH [BODY]
# Sets globals: _HTTP (status code), _BODY (response body)
_HTTP="" _BODY=""
api() {
  local method="$1" path="$2" body="${3:-}"
  local curl_args=(-s -X "$method" --max-time "$TIMEOUT"
    -H "Authorization: Bearer ${TOKEN}"
    -H "Content-Type: application/json"
    -o "$_BODY_FILE"
    -w "%{http_code}")
  [ -n "$body" ] && curl_args+=(-d "$body")
  _HTTP=$(curl "${curl_args[@]}" "${BASE}${path}" 2>/dev/null)
  _BODY=$(cat "$_BODY_FILE")
}

# chk LABEL WANT_STATUS METHOD PATH [BODY]
chk() {
  local label="$1" want="$2" method="$3" path="$4" body="${5:-}"
  api "$method" "$path" "$body"
  if [ "$_HTTP" = "$want" ]; then
    log_ok "$label (HTTP $_HTTP)"
    return 0
  else
    log_fail "$label (expected HTTP $want, got HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:160}${RST}" >&2
    return 1
  fi
}

# chk2xx LABEL METHOD PATH [BODY]  — any 2xx/3xx is a pass
chk2xx() {
  local label="$1" method="$2" path="$3" body="${4:-}"
  api "$method" "$path" "$body"
  if [[ "$_HTTP" =~ ^[23] ]]; then
    log_ok "$label (HTTP $_HTTP)"
    return 0
  else
    log_fail "$label (HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:160}${RST}" >&2
    return 1
  fi
}

# jf KEY — extract string field from $_BODY
jf() { printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || true; }

# jlen — array length of $_BODY
jlen() { printf '%s' "$_BODY" | python3 -c \
  "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo 0; }

# =============================================================================
echo -e "${BLD}cairn smoke test${RST}" >&2
echo -e "  Server  : ${CYN}${BASE}${RST}" >&2
echo -e "  Token   : ${CYN}${TOKEN:0:8}…${RST}" >&2
echo -e "  Run ID  : ${CYN}${RUN_ID}${RST}" >&2

# =============================================================================
section "1. Health & status"

chk2xx "GET /health"             GET  /health
chk    "GET /v1/status"     200  GET  /v1/status
chk    "GET /v1/dashboard"  200  GET  /v1/dashboard
chk    "GET /v1/stats"      200  GET  /v1/stats
chk2xx "GET /v1/overview"        GET  /v1/overview
chk    "GET /v1/health/detailed" 200  GET /v1/health/detailed
chk    "GET /v1/metrics"    200  GET  /v1/metrics
chk    "GET /v1/settings"   200  GET  /v1/settings
chk2xx "GET /v1/db/status"       GET  /v1/db/status

# =============================================================================
section "2. Session lifecycle"

chk "POST /v1/sessions" 201 POST /v1/sessions \
  "{\"tenant_id\":\"smoke\",\"workspace_id\":\"default\",\"project_id\":\"test\",\"session_id\":\"${SESSION_ID}\"}"
[ "$(jf state)" = "open" ] && log_ok "  state=open" || log_fail "  state='$(jf state)' (expected open)"

chk "GET /v1/sessions" 200 GET /v1/sessions
echo "$_BODY" | grep -q "$SESSION_ID" \
  && log_ok "  session appears in list" || log_fail "  session missing from list"

# =============================================================================
section "3. Run lifecycle"

chk "POST /v1/runs" 201 POST /v1/runs \
  "{\"tenant_id\":\"smoke\",\"workspace_id\":\"default\",\"project_id\":\"test\",\"session_id\":\"${SESSION_ID}\",\"run_id\":\"${RUN_ID}\"}"
[ "$(jf state)" = "pending" ] && log_ok "  state=pending" || log_fail "  state='$(jf state)' (expected pending)"

chk "GET /v1/runs" 200 GET /v1/runs
echo "$_BODY" | grep -q "$RUN_ID" && log_ok "  run in list" || log_fail "  run missing from list"

chk "GET /v1/runs/:id"           200 GET "/v1/runs/${RUN_ID}"
chk "GET /v1/runs/:id/cost"      200 GET "/v1/runs/${RUN_ID}/cost"
chk "GET /v1/runs/:id/events"    200 GET "/v1/runs/${RUN_ID}/events"
chk "GET /v1/runs/:id/tasks"     200 GET "/v1/runs/${RUN_ID}/tasks"
chk "GET /v1/runs/:id/approvals" 200 GET "/v1/runs/${RUN_ID}/approvals"

chk "POST pause"  200 POST "/v1/runs/${RUN_ID}/pause" \
  '{"reason_kind":"operator_pause","detail":"smoke"}'
[ "$(jf state)" = "paused"  ] && log_ok "  paused"  || log_fail "  pause state='$(jf state)'"

chk "POST resume" 200 POST "/v1/runs/${RUN_ID}/resume" '{}'
[ "$(jf state)" = "running" ] && log_ok "  running" || log_fail "  resume state='$(jf state)'"

# =============================================================================
section "4. Task queue"

# Correct EventEnvelope + RuntimeEvent (tagged with "event" discriminator)
# OwnershipKey: tag="scope", rename_all="snake_case" → Project variant flattens its fields
OWNERSHIP="{\"scope\":\"project\",\"tenant_id\":\"smoke\",\"workspace_id\":\"default\",\"project_id\":\"test\"}"
PROJECT="{\"tenant_id\":\"smoke\",\"workspace_id\":\"default\",\"project_id\":\"test\"}"
# EventSource: tag="source_type", rename_all="snake_case" → Runtime has no fields
SOURCE="{\"source_type\":\"runtime\"}"

chk "POST /v1/events/append (TaskCreated)" 201 POST /v1/events/append \
  "[{\"event_id\":\"evt_t_${RUN_ID}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"task_created\",\"project\":${PROJECT},\"task_id\":\"${TASK_ID}\",\"parent_run_id\":\"${RUN_ID}\",\"parent_task_id\":null,\"prompt_release_id\":null}}]"

sleep 0.4

chk "GET /v1/tasks" 200 GET /v1/tasks

chk "POST /v1/tasks/:id/claim" 200 POST "/v1/tasks/${TASK_ID}/claim" \
  "{\"worker_id\":\"${WORKER_ID}\",\"lease_duration_ms\":30000}"
[ "$(jf state)" = "leased" ] && log_ok "  claimed (leased)" || log_fail "  claim state='$(jf state)'"

chk "POST /v1/tasks/:id/release-lease" 200 POST "/v1/tasks/${TASK_ID}/release-lease" ""
[ "$(jf state)" = "queued" ] && log_ok "  released (queued)" || log_fail "  release state='$(jf state)'"

# =============================================================================
section "5. Approval workflow"

chk "POST /v1/events/append (ApprovalRequested)" 201 POST /v1/events/append \
  "[{\"event_id\":\"evt_a_${RUN_ID}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"approval_requested\",\"project\":${PROJECT},\"approval_id\":\"${APPR_ID}\",\"run_id\":\"${RUN_ID}\",\"task_id\":null,\"requirement\":\"required\"}}]"

sleep 0.4

chk "GET /v1/approvals/pending" 200 GET /v1/approvals/pending

chk "POST /v1/approvals/:id/resolve" 200 POST \
  "/v1/approvals/${APPR_ID}/resolve" '{"decision":"approved","reason":"smoke"}'
[ "$(jf decision)" = "approved" ] && log_ok "  decision=approved" \
  || log_fail "  decision='$(jf decision)'"

# =============================================================================
section "6. Event log"

chk "GET /v1/events" 200 GET "/v1/events?limit=20"
ECNT=$(jlen)
[ "$ECNT" -gt 0 ] && log_ok "  ${ECNT} events in log" || log_fail "  event log empty after writes"

chk "GET /v1/events?after=0"  200 GET "/v1/events?after=0&limit=5"
chk "GET /v1/admin/audit-log" 200 GET "/v1/admin/audit-log?limit=5"
chk "GET /v1/admin/logs"      200 GET "/v1/admin/logs?limit=10"

# =============================================================================
section "7. Stats"

chk "GET /v1/stats" 200 GET /v1/stats
TR=$(jf total_runs)
[ "${TR:-0}" -ge 1 ] && log_ok "  total_runs=${TR}" || log_fail "  total_runs=${TR:-0} (expected ≥ 1)"

# =============================================================================
section "8. Prompts"

chk "GET /v1/prompts/assets"   200 GET /v1/prompts/assets
chk "GET /v1/prompts/releases" 200 GET /v1/prompts/releases

# =============================================================================
section "9. Costs & traces"

chk "GET /v1/costs" 200 GET /v1/costs
echo "$_BODY" | grep -q "total_cost_micros" \
  && log_ok "  has total_cost_micros" || log_fail "  missing total_cost_micros"

chk "GET /v1/traces" 200 GET "/v1/traces?limit=10"

# =============================================================================
section "10. Providers"

chk "GET /v1/providers"        200 GET /v1/providers
chk "GET /v1/providers/health" 200 GET /v1/providers/health

# =============================================================================
section "11. Ollama"

chk "GET /v1/providers/ollama/models" 200 GET /v1/providers/ollama/models
MNAME=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; m=json.load(sys.stdin).get('models',[]); print(next((x for x in m if 'embed' not in x),m[0] if m else ''))" 2>/dev/null || true)
MCNT=$(jf count)

if [ -n "$MNAME" ]; then
  log_ok "  Ollama: ${MCNT} model(s); selected=${MNAME}"
  # Ollama can be slow — use a longer one-shot timeout for this step
  saved_timeout="$TIMEOUT"
  TIMEOUT=90
  chk "POST /v1/providers/ollama/generate" 200 POST /v1/providers/ollama/generate \
    "{\"model\":\"${MNAME}\",\"prompt\":\"Reply with only the word: ok\"}"
  TIMEOUT="$saved_timeout"
  GT=$(jf text)
  [ -n "$GT" ] && log_ok "  generate → '${GT:0:40}'" || log_fail "  generate returned empty text"
else
  log_skip "Ollama not available — skipping generation"
fi

# =============================================================================
section "12. Memory"

chk "POST /v1/memory/ingest" 200 POST /v1/memory/ingest \
  "{\"source_id\":\"smoke_src\",\"document_id\":\"sdoc_${RUN_ID}\",\"content\":\"Smoke test. The quick brown fox.\",\"tenant_id\":\"smoke\",\"workspace_id\":\"default\",\"project_id\":\"test\"}"
[ "$(jf status)" = "ingested" ] && log_ok "  ingested" || log_fail "  ingest status='$(jf status)'"

chk "GET /v1/memory/search" 200 GET \
  "/v1/memory/search?query_text=fox&tenant_id=smoke&workspace_id=default&project_id=test&limit=5"
echo "$_BODY" | grep -q "results" && log_ok "  search returned results" || log_fail "  search missing results"

chk "GET /v1/sources" 200 GET /v1/sources

# =============================================================================
section "13. Metrics ring buffer"

chk "GET /v1/metrics" 200 GET /v1/metrics
MREQ=$(jf total_requests)
[ "${MREQ:-0}" -gt 0 ] && log_ok "  ${MREQ} requests in buffer" || log_fail "  total_requests=${MREQ:-0}"

# =============================================================================
section "14. SSE stream (brief connect)"

SSE=$(curl -s --max-time 2 \
  -H "Authorization: Bearer ${TOKEN}" \
  "${BASE}/v1/stream" 2>/dev/null || true)
echo "$SSE" | grep -qE "event: connected|head_position|data:" \
  && log_ok "SSE initial frame received" \
  || log_fail "SSE no initial frame (got: ${SSE:0:80})"

# =============================================================================
section "15. Admin"

chk   "GET /v1/admin/audit-log" 200 GET "/v1/admin/audit-log?limit=5"
# Accept 201 (created) or 400 (already exists from prior run) — both are correct
api POST /v1/admin/tenants '{"tenant_id":"smoke_admin_t","name":"Smoke Tenant"}'
[[ "$_HTTP" =~ ^(201|400)$ ]] \
  && log_ok "POST /v1/admin/tenants (HTTP $_HTTP — created or already exists)" \
  || log_fail "POST /v1/admin/tenants (unexpected HTTP $_HTTP)"

# =============================================================================
section "16. Evals"

chk "GET /v1/evals/runs" 200 GET /v1/evals/runs

# =============================================================================
TOTAL=$(( PASS + FAIL + SKIP ))
echo "" >&2
echo -e "${BLD}── Results $(printf '─%.0s' {1..36})${RST}" >&2
printf "  ${GRN}Passed${RST}   %3d\n"  "$PASS" >&2
printf "  ${RED}Failed${RST}   %3d\n"  "$FAIL" >&2
printf "  ${YLW}Skipped${RST}  %3d\n" "$SKIP"  >&2
printf "  Total    %3d\n"              "$TOTAL" >&2
echo "" >&2

if [ "$FAIL" -eq 0 ]; then
  echo -e "${GRN}${BLD}All tests passed.${RST}" >&2; exit 0
else
  echo -e "${RED}${BLD}${FAIL} test(s) failed.${RST}" >&2; exit 1
fi
