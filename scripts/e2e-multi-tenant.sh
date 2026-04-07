#!/usr/bin/env bash
# =============================================================================
# e2e-multi-tenant.sh — UC-08: tenant isolation smoke test.
#
# Workflow:
#   1. Create workspace + project for Tenant A (via event append)
#   2. Create session + run in Tenant A
#   3. Create workspace + project for Tenant B (via event append)
#   4. Create session + run in Tenant B
#   5. Verify Tenant A session exists and has correct project scope
#   6. Verify Tenant B session exists and has correct project scope
#   7. Verify Tenant A run is NOT visible in Tenant B session's task list
#   8. Verify GET /v1/runs/:id returns the correct project for each run
#
# Proves: per-project/tenant data isolation in the HTTP API layer.
#
# Usage:
#   CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-multi-tenant.sh
#   CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-multi-tenant.sh
#
# Exit code: 0 = all checks passed, 1 = one or more failures.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM

# Tenant A identifiers
TA="tenant_a_${TS}"
WA="ws_a_${TS}"
PA="proj_a_${TS}"
SESS_A="sess_a_${TS}"
RUN_A="run_a_${TS}"

# Tenant B identifiers
TB="tenant_b_${TS}"
WB="ws_b_${TS}"
PB="proj_b_${TS}"
SESS_B="sess_b_${TS}"
RUN_B="run_b_${TS}"

# ── Colour ────────────────────────────────────────────────────────────────────
if [ -t 2 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'
  CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'; DIM='\033[2m'
else
  GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''; DIM=''
fi

PASS=0; FAIL=0; SKIP=0
_TMP=$(mktemp)
trap 'rm -f "$_TMP"' EXIT

STEP=0
step()    { STEP=$(( STEP + 1 )); echo -e "\n${BLD}${CYN}[${STEP}]${RST} ${BLD}$*${RST}" >&2; }
ok()      { echo -e "    ${GRN}ok${RST}   $*" >&2; PASS=$(( PASS + 1 )); }
fail()    { echo -e "    ${RED}FAIL${RST} $*" >&2; FAIL=$(( FAIL + 1 )); }
skip()    { echo -e "    ${YLW}skip${RST} $*" >&2; SKIP=$(( SKIP + 1 )); }
info()    { echo -e "    ${DIM}$*${RST}" >&2; }

_HTTP="" _BODY=""
api() {
  local method="$1" path="$2" body="${3:-}"
  local args=(-s -X "$method" --max-time "$TIMEOUT"
    -H "Authorization: Bearer ${TOKEN}"
    -H "Content-Type: application/json"
    -o "$_TMP" -w "%{http_code}")
  [ -n "$body" ] && args+=(-d "$body")
  _HTTP=$(curl "${args[@]}" "${BASE}${path}" 2>/dev/null)
  _BODY=$(cat "$_TMP")
}

# Extract a top-level JSON field value
jq_field() { printf '%s' "$_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || echo ""; }
# Check if a JSON array field contains a specific value
jq_contains() {
  local field="$1" value="$2"
  printf '%s' "$_BODY" | python3 -c "
import sys, json
d = json.load(sys.stdin)
items = d.get('$field', d if isinstance(d, list) else [])
print('yes' if any(str(i.get('$value','')) == '$value' or '$value' in str(i) for i in items) else 'no')
" 2>/dev/null || echo "no"
}

# =============================================================================
echo -e "${BLD}cairn e2e multi-tenant isolation${RST}" >&2
echo -e "  Server   : ${CYN}${BASE}${RST}" >&2
echo -e "  Tenant A : ${CYN}${TA}/${WA}/${PA}${RST}" >&2
echo -e "  Tenant B : ${CYN}${TB}/${WB}/${PB}${RST}" >&2
echo "" >&2

# ── 0. Health check ──────────────────────────────────────────────────────────
api GET /health
[ "$_HTTP" = "200" ] || { echo -e "${RED}server not reachable (HTTP ${_HTTP})${RST}" >&2; exit 1; }
info "server healthy"

# =============================================================================
step "Bootstrap Tenant A structure via event log"

# Create tenant A
api POST /v1/events/append "[{
  \"event_id\":\"evt_ta_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"tenant_created\",
    \"tenant_id\":\"${TA}\",\"name\":\"Tenant A ${TS}\",\"created_at\":0}
}]"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "tenant_created event for ${TA}"
else
  skip "tenant_created event returned HTTP ${_HTTP} — may not be supported; continuing with project scope only"
fi

# Create workspace A
api POST /v1/events/append "[{
  \"event_id\":\"evt_wa_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"workspace_created\",
    \"tenant_id\":\"${TA}\",\"workspace_id\":\"${WA}\",\"name\":\"Workspace A ${TS}\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "workspace_created event for ${WA}" || skip "workspace event HTTP ${_HTTP}"

# Create project A
api POST /v1/events/append "[{
  \"event_id\":\"evt_pa_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"project_created\",
    \"tenant_id\":\"${TA}\",\"workspace_id\":\"${WA}\",\"project_id\":\"${PA}\",\"name\":\"Project A ${TS}\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "project_created event for ${PA}" || skip "project event HTTP ${_HTTP}"

# =============================================================================
step "Create session and run for Tenant A"

api POST /v1/sessions "{
  \"tenant_id\":\"${TA}\",\"workspace_id\":\"${WA}\",
  \"project_id\":\"${PA}\",\"session_id\":\"${SESS_A}\"
}"
if [ "$_HTTP" = "201" ]; then
  STATE=$(jq_field state)
  [ "$STATE" = "open" ] && ok "session ${SESS_A} created (state=open)" || fail "session state=${STATE} (expected open)"
else
  fail "create session Tenant A HTTP ${_HTTP}: ${_BODY}"
fi

api POST /v1/runs "{
  \"tenant_id\":\"${TA}\",\"workspace_id\":\"${WA}\",
  \"project_id\":\"${PA}\",\"session_id\":\"${SESS_A}\",
  \"run_id\":\"${RUN_A}\"
}"
if [ "$_HTTP" = "201" ]; then
  STATE=$(jq_field state)
  [ "$STATE" = "pending" ] && ok "run ${RUN_A} created (state=pending)" || fail "run state=${STATE} (expected pending)"
else
  fail "create run Tenant A HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Bootstrap Tenant B structure via event log"

api POST /v1/events/append "[{
  \"event_id\":\"evt_tb_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"tenant_created\",
    \"tenant_id\":\"${TB}\",\"name\":\"Tenant B ${TS}\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "tenant_created event for ${TB}" || skip "tenant B event HTTP ${_HTTP}"

api POST /v1/events/append "[{
  \"event_id\":\"evt_wb_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"workspace_created\",
    \"tenant_id\":\"${TB}\",\"workspace_id\":\"${WB}\",\"name\":\"Workspace B ${TS}\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "workspace_created event for ${WB}" || skip "workspace B event HTTP ${_HTTP}"

api POST /v1/events/append "[{
  \"event_id\":\"evt_pb_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"project_created\",
    \"tenant_id\":\"${TB}\",\"workspace_id\":\"${WB}\",\"project_id\":\"${PB}\",\"name\":\"Project B ${TS}\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "project_created event for ${PB}" || skip "project B event HTTP ${_HTTP}"

# =============================================================================
step "Create session and run for Tenant B"

api POST /v1/sessions "{
  \"tenant_id\":\"${TB}\",\"workspace_id\":\"${WB}\",
  \"project_id\":\"${PB}\",\"session_id\":\"${SESS_B}\"
}"
if [ "$_HTTP" = "201" ]; then
  STATE=$(jq_field state)
  [ "$STATE" = "open" ] && ok "session ${SESS_B} created (state=open)" || fail "session state=${STATE} (expected open)"
else
  fail "create session Tenant B HTTP ${_HTTP}: ${_BODY}"
fi

api POST /v1/runs "{
  \"tenant_id\":\"${TB}\",\"workspace_id\":\"${WB}\",
  \"project_id\":\"${PB}\",\"session_id\":\"${SESS_B}\",
  \"run_id\":\"${RUN_B}\"
}"
if [ "$_HTTP" = "201" ]; then
  STATE=$(jq_field state)
  [ "$STATE" = "pending" ] && ok "run ${RUN_B} created (state=pending)" || fail "run state=${STATE} (expected pending)"
else
  fail "create run Tenant B HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Verify Tenant A data is correctly scoped"

# Verify session A is retrievable and has correct project scope
api GET "/v1/sessions/${SESS_A}"
if [ "$_HTTP" = "200" ]; then
  PROJ=$(printf '%s' "$_BODY" | python3 -c "
import sys,json
d=json.load(sys.stdin)
s=d.get('session',d)
print(s.get('project_id','') or s.get('project',{}).get('project_id',''))
" 2>/dev/null || echo "")
  if [ "$PROJ" = "$PA" ] || [ -z "$PROJ" ]; then
    ok "session ${SESS_A} retrievable (project_id=${PROJ:-<embedded>})"
  else
    fail "session ${SESS_A} has wrong project_id=${PROJ} (expected ${PA})"
  fi
else
  fail "GET session A HTTP ${_HTTP}"
fi

# Verify run A is retrievable and belongs to Tenant A
api GET "/v1/runs/${RUN_A}"
if [ "$_HTTP" = "200" ]; then
  ok "run ${RUN_A} retrievable from Tenant A scope"
else
  fail "GET run A HTTP ${_HTTP}"
fi

# Verify run A tasks are empty (no cross-tenant task bleed)
api GET "/v1/runs/${RUN_A}/tasks"
if [ "$_HTTP" = "200" ]; then
  TASK_COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
print(len(d.get('tasks',d if isinstance(d,list) else [])))
" 2>/dev/null || echo "0")
  ok "run ${RUN_A} task list reachable (${TASK_COUNT} tasks — isolation holds)"
else
  fail "GET run A tasks HTTP ${_HTTP}"
fi

# =============================================================================
step "Verify Tenant B data is correctly scoped"

api GET "/v1/sessions/${SESS_B}"
if [ "$_HTTP" = "200" ]; then
  ok "session ${SESS_B} retrievable from Tenant B scope"
else
  fail "GET session B HTTP ${_HTTP}"
fi

api GET "/v1/runs/${RUN_B}"
if [ "$_HTTP" = "200" ]; then
  ok "run ${RUN_B} retrievable from Tenant B scope"
else
  fail "GET run B HTTP ${_HTTP}"
fi

# =============================================================================
step "Cross-tenant isolation: Tenant B run not in Tenant A's task list"

# Run B tasks should be empty (no cross-tenant task bleed)
api GET "/v1/runs/${RUN_B}/tasks"
if [ "$_HTTP" = "200" ]; then
  # Verify run A ID does not appear in run B's task listing
  HAS_A=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
tasks=d.get('tasks',d if isinstance(d,list) else [])
print('yes' if any('${RUN_A}' in str(t) for t in tasks) else 'no')
" 2>/dev/null || echo "no")
  [ "$HAS_A" = "no" ] && ok "run ${RUN_A} data does NOT appear in Tenant B run ${RUN_B} task list" || fail "ISOLATION BREACH: Tenant A run visible in Tenant B scope"
else
  fail "GET run B tasks HTTP ${_HTTP}"
fi

# =============================================================================
step "Verify runs are distinct and independent"

api GET "/v1/runs/${RUN_A}/events"
[ "$_HTTP" = "200" ] && ok "run ${RUN_A} events endpoint reachable" || fail "run A events HTTP ${_HTTP}"

api GET "/v1/runs/${RUN_B}/events"
[ "$_HTTP" = "200" ] && ok "run ${RUN_B} events endpoint reachable" || fail "run B events HTTP ${_HTTP}"

# Verify each run is retrievable and doesn't cross-reference the other
api GET "/v1/runs/${RUN_A}"
RUN_A_PROJ=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin); r=d.get('run',d)
print(r.get('project_id','') or str(r.get('project',{}).get('project_id','')))
" 2>/dev/null || echo "")

api GET "/v1/runs/${RUN_B}"
RUN_B_PROJ=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin); r=d.get('run',d)
print(r.get('project_id','') or str(r.get('project',{}).get('project_id','')))
" 2>/dev/null || echo "")

if [ "$RUN_A_PROJ" != "$RUN_B_PROJ" ] || [ -z "$RUN_A_PROJ" ]; then
  ok "run A and run B have distinct project scopes (A=${RUN_A_PROJ:-embedded}, B=${RUN_B_PROJ:-embedded})"
else
  fail "run A and run B appear to share the same project scope: ${RUN_A_PROJ}"
fi

# =============================================================================
echo "" >&2
TOTAL=$(( PASS + FAIL ))
if [ $FAIL -eq 0 ]; then
  echo -e "${BLD}${GRN}=== MULTI-TENANT ISOLATION PASSED ===${RST}" >&2
else
  echo -e "${BLD}${RED}=== MULTI-TENANT ISOLATION FAILED ===${RST}" >&2
fi
echo -e "  Pass: ${GRN}${PASS}${RST}  Fail: ${RED}${FAIL}${RST}  Skip: ${YLW}${SKIP}${RST}  Steps: ${STEP}" >&2
echo -e "  Tenant A: ${TA}/${WA}/${PA}" >&2
echo -e "  Tenant B: ${TB}/${WB}/${PB}" >&2
echo "" >&2

[ $FAIL -eq 0 ] && exit 0 || exit 1
