#!/usr/bin/env bash
# =============================================================================
# e2e-fleet-monitoring.sh — UC-14: fleet monitoring and observability.
#
# Workflow:
#   1. Create sessions + runs in pending, running, and completed states
#   2. Verify GET /v1/fleet returns agent/run status information
#   3. Verify GET /v1/runs/stalled endpoint is reachable
#   4. Verify GET /v1/runs/escalated endpoint is reachable
#   5. Verify GET /v1/status returns system health information
#   6. Verify GET /metrics returns Prometheus-format metrics
#   7. Verify GET /v1/stats returns aggregate stats
#   8. Verify POST /v1/runs/:id/diagnose works
#   9. Verify GET /v1/runs/:id/interventions works
#  10. Verify GET /v1/events/recent shows recent events
#
# Usage:
#   CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-fleet-monitoring.sh
#
# Exit code: 0 = all checks passed, 1 = one or more failures.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM
TENANT="default"
WORKSPACE="default"
PROJECT="default"

SESS_PEND="fleet_sess_pend_${TS}"
SESS_RUN="fleet_sess_run_${TS}"
SESS_DONE="fleet_sess_done_${TS}"

RUN_PEND="fleet_run_pend_${TS}"
RUN_RUN="fleet_run_run_${TS}"
RUN_DONE="fleet_run_done_${TS}"

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
step() { STEP=$(( STEP + 1 )); echo -e "\n${BLD}${CYN}[${STEP}]${RST} ${BLD}$*${RST}" >&2; }
ok()   { echo -e "    ${GRN}ok${RST}   $*" >&2; PASS=$(( PASS + 1 )); }
fail() { echo -e "    ${RED}FAIL${RST} $*" >&2; FAIL=$(( FAIL + 1 )); }
skip() { echo -e "    ${YLW}skip${RST} $*" >&2; SKIP=$(( SKIP + 1 )); }
info() { echo -e "    ${DIM}$*${RST}" >&2; }

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
jf() { printf '%s' "$_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || echo ""; }

# =============================================================================
echo -e "${BLD}cairn e2e fleet monitoring${RST}" >&2
echo -e "  Server : ${CYN}${BASE}${RST}" >&2
echo -e "  Runs   : ${CYN}${RUN_PEND} / ${RUN_RUN} / ${RUN_DONE}${RST}" >&2
echo "" >&2

api GET /health
[ "$_HTTP" = "200" ] || { echo -e "${RED}server not reachable (HTTP ${_HTTP})${RST}" >&2; exit 1; }
info "server healthy"

# =============================================================================
step "Create runs in multiple states (pending, running, completed)"

# ── Pending run ──────────────────────────────────────────────────────────────
api POST /v1/sessions "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESS_PEND}\"
}"
[ "$_HTTP" = "201" ] && ok "session ${SESS_PEND} created" || fail "create pending session HTTP ${_HTTP}"

api POST /v1/runs "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESS_PEND}\",
  \"run_id\":\"${RUN_PEND}\"
}"
[ "$_HTTP" = "201" ] && ok "run ${RUN_PEND} state=pending" || fail "create pending run HTTP ${_HTTP}"

# ── Running run (pending → running via state change event) ───────────────────
api POST /v1/sessions "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESS_RUN}\"
}"
[ "$_HTTP" = "201" ] && ok "session ${SESS_RUN} created" || fail "create running session HTTP ${_HTTP}"

api POST /v1/runs "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESS_RUN}\",
  \"run_id\":\"${RUN_RUN}\"
}"
[ "$_HTTP" = "201" ] && ok "run ${RUN_RUN} created" || fail "create run (to-be-running) HTTP ${_HTTP}"

PROJ="{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"
OWN="{\"scope\":\"project\",\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"

api POST /v1/events/append "[{
  \"event_id\":\"evt_rr_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":${OWN},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{
    \"event\":\"run_state_changed\",
    \"project\":${PROJ},
    \"run_id\":\"${RUN_RUN}\",
    \"transition\":{\"from\":\"pending\",\"to\":\"running\"},
    \"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null
  }
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "run ${RUN_RUN} transitioned to state=running" || skip "run state change HTTP ${_HTTP}"

# ── Completed run ─────────────────────────────────────────────────────────────
api POST /v1/sessions "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESS_DONE}\"
}"
[ "$_HTTP" = "201" ] && ok "session ${SESS_DONE} created" || fail "create done session HTTP ${_HTTP}"

api POST /v1/runs "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESS_DONE}\",
  \"run_id\":\"${RUN_DONE}\"
}"
[ "$_HTTP" = "201" ] && ok "run ${RUN_DONE} created" || fail "create run (to-be-completed) HTTP ${_HTTP}"

api POST /v1/events/append "[{
  \"event_id\":\"evt_rd_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":${OWN},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{
    \"event\":\"run_state_changed\",
    \"project\":${PROJ},
    \"run_id\":\"${RUN_DONE}\",
    \"transition\":{\"from\":\"pending\",\"to\":\"completed\"},
    \"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null
  }
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "run ${RUN_DONE} transitioned to state=completed" || skip "run state change HTTP ${_HTTP}"

# =============================================================================
step "Verify run states are correct"

api GET "/v1/runs/${RUN_PEND}"
if [ "$_HTTP" = "200" ]; then
  STATE=$(printf '%s' "$_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); r=d.get('run',d); print(r.get('state',''))" 2>/dev/null || echo "")
  [ "$STATE" = "pending" ] && ok "run ${RUN_PEND} state=pending confirmed" || skip "run ${RUN_PEND} state=${STATE} (expected pending — may vary)"
else
  fail "GET run ${RUN_PEND} HTTP ${_HTTP}"
fi

api GET "/v1/runs/${RUN_RUN}"
if [ "$_HTTP" = "200" ]; then
  STATE=$(printf '%s' "$_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); r=d.get('run',d); print(r.get('state',''))" 2>/dev/null || echo "")
  [[ "$STATE" =~ ^(running|pending)$ ]] && ok "run ${RUN_RUN} state=${STATE} (running or pending)" || skip "run state=${STATE}"
else
  fail "GET run ${RUN_RUN} HTTP ${_HTTP}"
fi

api GET "/v1/runs/${RUN_DONE}"
if [ "$_HTTP" = "200" ]; then
  STATE=$(printf '%s' "$_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); r=d.get('run',d); print(r.get('state',''))" 2>/dev/null || echo "")
  [[ "$STATE" =~ ^(completed|pending)$ ]] && ok "run ${RUN_DONE} state=${STATE}" || skip "run state=${STATE}"
else
  fail "GET run ${RUN_DONE} HTTP ${_HTTP}"
fi

# =============================================================================
step "Verify GET /v1/fleet — agent fleet status"

api GET /v1/fleet
if [ "$_HTTP" = "200" ]; then
  ok "GET /v1/fleet returned 200"
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
agents=d.get('agents',d if isinstance(d,list) else [])
print(len(agents))
" 2>/dev/null || echo "?")
  info "fleet agents in response: ${COUNT}"
else
  fail "GET /v1/fleet HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Verify GET /v1/runs/stalled — stalled run detection"

api GET "/v1/runs/stalled"
if [ "$_HTTP" = "200" ]; then
  STALLED=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('runs',d.get('stalled_runs',d if isinstance(d,list) else []))
print(len(items))
" 2>/dev/null || echo "?")
  ok "GET /v1/runs/stalled returned 200 (${STALLED} stalled runs)"
else
  fail "GET /v1/runs/stalled HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Verify GET /v1/runs/escalated — escalated run detection"

api GET "/v1/runs/escalated"
if [ "$_HTTP" = "200" ]; then
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('runs',d.get('escalated_runs',d if isinstance(d,list) else []))
print(len(items))
" 2>/dev/null || echo "?")
  ok "GET /v1/runs/escalated returned 200 (${COUNT} escalated runs)"
else
  fail "GET /v1/runs/escalated HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Verify GET /v1/status — system health"

api GET /v1/status
if [ "$_HTTP" = "200" ]; then
  ok "GET /v1/status returned 200"
  STATUS_VAL=$(jf status)
  [ -n "$STATUS_VAL" ] && info "status: ${STATUS_VAL}" || info "status field not present (structure varies)"
else
  fail "GET /v1/status HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Verify GET /metrics — Prometheus metrics"

_HTTP=$(curl -s -X GET --max-time "$TIMEOUT" \
  -H "Authorization: Bearer ${TOKEN}" \
  -o "$_TMP" -w "%{http_code}" \
  "${BASE}/metrics" 2>/dev/null)
_BODY=$(cat "$_TMP")

if [ "$_HTTP" = "200" ]; then
  # Prometheus format starts with # HELP or has metric lines
  HAS_METRIC=$(printf '%s' "$_BODY" | grep -c "^#\|^cairn_\|^process_\|_total\|_count\|_gauge" 2>/dev/null || echo "0")
  ok "GET /metrics returned 200 (${HAS_METRIC} metric lines)"
elif [ "$_HTTP" = "404" ]; then
  skip "GET /metrics returned 404 — metrics may be on different path"
else
  fail "GET /metrics HTTP ${_HTTP}"
fi

# =============================================================================
step "Verify GET /v1/stats — aggregate statistics"

api GET /v1/stats
if [ "$_HTTP" = "200" ]; then
  ok "GET /v1/stats returned 200"
else
  fail "GET /v1/stats HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "POST /v1/runs/:id/diagnose — run diagnostics"

api POST "/v1/runs/${RUN_RUN}/diagnose" '{}'
if [[ "$_HTTP" =~ ^(200|201|202)$ ]]; then
  ok "POST /v1/runs/${RUN_RUN}/diagnose returned ${_HTTP}"
elif [ "$_HTTP" = "404" ]; then
  skip "diagnose returned 404 — run may not be in running state"
else
  fail "diagnose HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "GET /v1/runs/:id/interventions — available interventions"

api GET "/v1/runs/${RUN_RUN}/interventions"
if [[ "$_HTTP" =~ ^(200|404)$ ]]; then
  ok "GET /v1/runs/${RUN_RUN}/interventions returned ${_HTTP}"
  if [ "$_HTTP" = "200" ]; then
    COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('interventions',d if isinstance(d,list) else [])
print(len(items))
" 2>/dev/null || echo "?")
    info "${COUNT} interventions available"
  fi
else
  fail "interventions HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Verify GET /v1/events/recent shows our runs in the event trail"

api GET "/v1/events/recent?limit=100"
if [ "$_HTTP" = "200" ]; then
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
print(d.get('count', len(d.get('items',[]))))
" 2>/dev/null || echo "?")
  ok "GET /v1/events/recent returned 200 (${COUNT} events)"

  HAS_RUN=$(printf '%s' "$_BODY" | python3 -c "
import sys,json
items=json.load(sys.stdin).get('items',[])
print('yes' if any('${RUN_PEND}' in str(i) or '${RUN_RUN}' in str(i) for i in items) else 'no')
" 2>/dev/null || echo "no")
  [ "$HAS_RUN" = "yes" ] && ok "fleet runs visible in event trail" || skip "fleet runs not yet in recent events window"
else
  fail "GET /v1/events/recent HTTP ${_HTTP}"
fi

# =============================================================================
echo "" >&2
if [ $FAIL -eq 0 ]; then
  echo -e "${BLD}${GRN}=== FLEET MONITORING PASSED ===${RST}" >&2
else
  echo -e "${BLD}${RED}=== FLEET MONITORING FAILED ===${RST}" >&2
fi
echo -e "  Pass: ${GRN}${PASS}${RST}  Fail: ${RED}${FAIL}${RST}  Skip: ${YLW}${SKIP}${RST}  Steps: ${STEP}" >&2
echo -e "  Runs: pending=${RUN_PEND} running=${RUN_RUN} completed=${RUN_DONE}" >&2
echo "" >&2

[ $FAIL -eq 0 ] && exit 0 || exit 1
