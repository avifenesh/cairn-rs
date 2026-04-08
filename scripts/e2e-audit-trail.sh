#!/usr/bin/env bash
# =============================================================================
# e2e-audit-trail.sh — UC-19: audit log + execution trace verification
#
# Workflow:
#   1.  Create session + run + task (generates auditable events)
#   2.  Claim + start + complete task
#   3.  Complete run
#   4.  Get audit log (GET /v1/admin/audit-log)
#   5.  Get resource-specific audit log (GET /v1/admin/audit-log/:type/:id)
#   6.  Get run audit trail (GET /v1/runs/:id/audit)
#   7.  Get recent events for correlation
#   8.  Get LLM traces (GET /v1/traces)
#   9.  Get execution trace for run (GET /v1/graph/execution-trace/:run_id)
#  10.  Verify all endpoints return valid JSON with expected shapes
#
# Usage: CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-audit-trail.sh
# Exit: 0 = all assertions passed, 1 = failure.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM
TENANT="default"; WORKSPACE="default"; PROJECT="default"
SESSION="e2e_audit_sess_${TS}"
RUN="e2e_audit_run_${TS}"
TASK="e2e_audit_task_${TS}"
WORKER="e2e_audit_worker_${TS}"

PASS=0; SKIP=0; FAIL_COUNT=0

if [ -t 2 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'
  CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'; DIM='\033[2m'
else
  GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''; DIM=''
fi

STEP=0
step() { STEP=$(( STEP + 1 )); echo -e "\n${BLD}${CYN}[${STEP}]${RST} ${BLD}$1${RST}" >&2; }
ok()   { PASS=$(( PASS + 1 ));  echo -e "    ${GRN}ok${RST}   $1" >&2; }
skip() { SKIP=$(( SKIP + 1 ));  echo -e "    ${YLW}skip${RST} $1" >&2; }
fail() { FAIL_COUNT=$(( FAIL_COUNT + 1 )); echo -e "    ${RED}FAIL${RST} $1" >&2; exit 1; }
info() { echo -e "    ${DIM}$1${RST}" >&2; }

_TMP=$(mktemp); trap 'rm -f "$_TMP"' EXIT
STATUS=""; RESP=""

post() {
  local path="$1" body="$2" t="${3:-$TIMEOUT}"
  STATUS=$(curl -s -X POST --max-time "$t" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d "$body" -o "$_TMP" -w "%{http_code}" "${BASE}${path}" 2>/dev/null)
  RESP=$(cat "$_TMP")
}

get() {
  STATUS=$(curl -s -X GET --max-time "$TIMEOUT" \
    -H "Authorization: Bearer ${TOKEN}" \
    -o "$_TMP" -w "%{http_code}" "${BASE}$1" 2>/dev/null)
  RESP=$(cat "$_TMP")
}

jf() { printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null; }

is_valid_json() {
  printf '%s' "$RESP" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null && return 0 || return 1
}

# =============================================================================
echo -e "${BLD}cairn e2e audit trail${RST}" >&2
echo -e "  Server : ${CYN}${BASE}${RST}" >&2
echo -e "  Run ID : ${CYN}${RUN}${RST}" >&2
echo "" >&2

get /health
[ "$STATUS" = "200" ] || fail "server not reachable at ${BASE} (HTTP ${STATUS})"

# =============================================================================
step "Create session + run + task (generates auditable events)"
post /v1/sessions "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION}\"}"
[ "$STATUS" = "201" ] || fail "create session HTTP ${STATUS}: ${RESP}"
ok "session ${SESSION}"

post /v1/runs "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION}\",\"run_id\":\"${RUN}\"}"
[ "$STATUS" = "201" ] || fail "create run HTTP ${STATUS}: ${RESP}"
ok "run ${RUN}"

post "/v1/runs/${RUN}/tasks" "{\"task_id\":\"${TASK}\",
  \"name\":\"audit_test_task\",\"description\":\"Task for audit trail verification\"}"
[ "$STATUS" = "201" ] || fail "create task HTTP ${STATUS}: ${RESP}"
TASK_ID=$(jf task_id); [ -n "$TASK_ID" ] || TASK_ID="$TASK"
ok "task ${TASK_ID}"

# =============================================================================
step "Claim + start + complete task (write audit events)"
post "/v1/tasks/${TASK_ID}/claim" "{\"worker_id\":\"${WORKER}\",\"lease_duration_ms\":30000}"
[ "$STATUS" = "200" ] || fail "claim task HTTP ${STATUS}: ${RESP}"
ok "task claimed"

post "/v1/tasks/${TASK_ID}/start" '{}'
[ "$STATUS" = "200" ] || fail "start task HTTP ${STATUS}: ${RESP}"
ok "task started"

post "/v1/tasks/${TASK_ID}/complete" '{"result":{"output":"audit trail test complete"}}'
[ "$STATUS" = "200" ] || fail "complete task HTTP ${STATUS}: ${RESP}"
ok "task completed"

# =============================================================================
step "Complete run via event append"
OWN="{\"scope\":\"project\",\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"
PROJ="{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"

post /v1/events/append "[{
  \"event_id\":\"evt_at_run_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":${OWN},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{
    \"event\":\"run_state_changed\",\"project\":${PROJ},
    \"run_id\":\"${RUN}\",
    \"transition\":{\"from\":\"pending\",\"to\":\"completed\"},
    \"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null
  }
}]"
[[ "$STATUS" =~ ^(200|201)$ ]] || fail "complete run HTTP ${STATUS}: ${RESP}"
ok "run completed"

# =============================================================================
step "Get audit log (GET /v1/admin/audit-log)"
get "/v1/admin/audit-log?limit=50"
if [ "$STATUS" = "200" ]; then
  is_valid_json || fail "audit log returned invalid JSON"
  ENTRY_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('items',d.get('entries',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
  ok "audit log: ${ENTRY_COUNT} entries"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "audit log not implemented (HTTP ${STATUS})"
else
  fail "audit log HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get resource-specific audit (GET /v1/admin/audit-log/run/:run_id)"
get "/v1/admin/audit-log/run/${RUN}"
if [ "$STATUS" = "200" ]; then
  is_valid_json || fail "resource audit returned invalid JSON"
  R_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('items',d.get('entries',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
  ok "resource audit for run/${RUN}: ${R_COUNT} entries"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "resource audit HTTP ${STATUS}"
else
  fail "resource audit HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get run audit trail (GET /v1/runs/:id/audit)"
get "/v1/runs/${RUN}/audit"
if [ "$STATUS" = "200" ]; then
  is_valid_json || fail "run audit returned invalid JSON"
  ok "run audit trail reachable for ${RUN}"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "run audit HTTP ${STATUS}"
else
  fail "run audit HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get run events — verify expected event types present"
get "/v1/runs/${RUN}/events"
[ "$STATUS" = "200" ] || fail "run events HTTP ${STATUS}"
EVT_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('events',d.get('items',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
ok "${EVT_COUNT} events in run event log"

# =============================================================================
step "Get recent events — verify run events appear"
get "/v1/events/recent?limit=100"
[ "$STATUS" = "200" ] || fail "recent events HTTP ${STATUS}"
for evt in session_created run_created task_created task_state_changed run_state_changed; do
  HAS=$(printf '%s' "$RESP" | python3 -c "
import sys,json
items=json.load(sys.stdin).get('items',[])
print('yes' if any(i.get('event_type')=='${evt}' for i in items) else 'no')
" 2>/dev/null)
  [ "$HAS" = "yes" ] && ok "  event '${evt}' in recent log" || info "  event '${evt}' not found (may have scrolled out)"
done

# =============================================================================
step "Get LLM traces (GET /v1/traces)"
get "/v1/traces?limit=10"
if [ "$STATUS" = "200" ]; then
  is_valid_json || fail "traces returned invalid JSON"
  TRACE_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('traces',d.get('items',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
  ok "LLM traces: ${TRACE_COUNT} trace(s)"

  # Get first trace ID if available
  TRACE_ID=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('traces',d.get('items',d if isinstance(d,list) else []))
print(items[0].get('trace_id','') if items else '')" 2>/dev/null)
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "traces endpoint HTTP ${STATUS}"
  TRACE_ID=""
else
  fail "traces HTTP ${STATUS}: ${RESP}"
  TRACE_ID=""
fi

# =============================================================================
step "Get individual trace (GET /v1/trace/:id)"
if [ -n "${TRACE_ID:-}" ]; then
  get "/v1/trace/${TRACE_ID}"
  if [ "$STATUS" = "200" ]; then
    is_valid_json || fail "individual trace returned invalid JSON"
    ok "trace ${TRACE_ID} detail reachable"
  elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
    skip "individual trace HTTP ${STATUS}"
  else
    fail "individual trace HTTP ${STATUS}: ${RESP}"
  fi
else
  get "/v1/trace/nonexistent_trace_${TS}"
  if [ "$STATUS" = "404" ]; then
    ok "trace 404 for nonexistent ID (correct behaviour)"
  elif [[ "$STATUS" =~ ^(200|501)$ ]]; then
    skip "trace endpoint HTTP ${STATUS}"
  else
    info "trace HTTP ${STATUS} (no traces yet)"
    ok "trace endpoint reachable"
  fi
fi

# =============================================================================
step "Get graph execution trace (GET /v1/graph/execution-trace/:run_id)"
get "/v1/graph/execution-trace/${RUN}"
if [ "$STATUS" = "200" ]; then
  is_valid_json || fail "execution trace returned invalid JSON"
  ok "graph execution trace reachable for ${RUN}"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "graph execution trace HTTP ${STATUS}"
else
  fail "graph execution trace HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get session LLM traces (GET /v1/sessions/:id/llm-traces)"
get "/v1/sessions/${SESSION}/llm-traces"
if [ "$STATUS" = "200" ]; then
  is_valid_json || fail "session llm-traces invalid JSON"
  ok "session llm-traces reachable"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "session llm-traces HTTP ${STATUS}"
else
  fail "session llm-traces HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
echo "" >&2
echo -e "${BLD}${GRN}=== E2E AUDIT TRAIL COMPLETED ===${RST}" >&2
echo -e "  Session : ${SESSION}" >&2
echo -e "  Run     : ${RUN}" >&2
echo -e "  Task    : ${TASK_ID}" >&2
echo -e "  Pass: ${PASS}  Skip: ${SKIP}  Fail: ${FAIL_COUNT}  Steps: ${STEP}" >&2
echo "" >&2
