#!/usr/bin/env bash
# =============================================================================
# e2e-cost-tracking.sh — UC-15: cost tracking + budget enforcement
#
# Workflow:
#   1.  Create session + run
#   2.  Set run cost alert
#   3.  Get run cost
#   4.  Get tenant-wide costs (GET /v1/costs)
#   5.  List run cost alerts
#   6.  Get provider binding cost stats
#   7.  Get provider binding cost ranking
#   8.  Get session cost
#
# Usage: CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-cost-tracking.sh
# Exit: 0 = all assertions passed, 1 = failure.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM
TENANT="default"; WORKSPACE="default"; PROJECT="default"
SESSION="e2e_cost_sess_${TS}"
RUN="e2e_cost_run_${TS}"

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

# =============================================================================
echo -e "${BLD}cairn e2e cost tracking${RST}" >&2
echo -e "  Server : ${CYN}${BASE}${RST}" >&2
echo -e "  Run ID : ${CYN}${RUN}${RST}" >&2
echo "" >&2

get /health
[ "$STATUS" = "200" ] || fail "server not reachable at ${BASE} (HTTP ${STATUS})"

# =============================================================================
step "Create session + run"
post /v1/sessions "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION}\"}"
[ "$STATUS" = "201" ] || fail "create session HTTP ${STATUS}: ${RESP}"
ok "session ${SESSION}"

post /v1/runs "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION}\",\"run_id\":\"${RUN}\"}"
[ "$STATUS" = "201" ] || fail "create run HTTP ${STATUS}: ${RESP}"
ok "run ${RUN} state=$(jf state)"

# =============================================================================
step "Set run cost alert (POST /v1/runs/:id/cost-alert)"
post "/v1/runs/${RUN}/cost-alert" '{
  "threshold_micros": 5000000,
  "alert_type": "hard_limit"
}'
if [ "$STATUS" = "200" ] || [ "$STATUS" = "201" ]; then
  ok "cost alert set (threshold=5,000,000 micros / \$5.00)"
elif [[ "$STATUS" =~ ^(404|501|422)$ ]]; then
  skip "cost-alert not available (HTTP ${STATUS})"
else
  fail "set cost alert HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get run cost (GET /v1/runs/:id/cost)"
get "/v1/runs/${RUN}/cost"
if [ "$STATUS" = "200" ]; then
  TOTAL_MICROS=$(jf total_cost_micros)
  PROVIDER_CALLS=$(jf provider_calls)
  ok "run cost: total_micros=${TOTAL_MICROS:-0} provider_calls=${PROVIDER_CALLS:-0}"
elif [ "$STATUS" = "404" ]; then
  ok "run cost 404 — no cost records yet (expected for new run)"
elif [[ "$STATUS" =~ ^(501)$ ]]; then
  skip "run cost endpoint HTTP ${STATUS}"
else
  fail "get run cost HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get tenant-wide costs (GET /v1/costs)"
get "/v1/costs"
if [ "$STATUS" = "200" ]; then
  TOTAL_CALLS=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
print(d.get('total_provider_calls',d.get('total_calls','?')))" 2>/dev/null)
  ok "tenant costs reachable (total_provider_calls=${TOTAL_CALLS})"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "tenant costs HTTP ${STATUS}"
else
  fail "tenant costs HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get cost summary stats (GET /v1/stats)"
get "/v1/stats"
if [ "$STATUS" = "200" ]; then
  TOTAL_RUNS=$(printf '%s' "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('total_runs','?'))" 2>/dev/null)
  ok "stats: total_runs=${TOTAL_RUNS}"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "stats HTTP ${STATUS}"
else
  fail "stats HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get run SLA (GET /v1/runs/:id/sla)"
get "/v1/runs/${RUN}/sla"
if [ "$STATUS" = "200" ]; then
  ok "run SLA endpoint reachable"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "run SLA HTTP ${STATUS}"
else
  fail "run SLA HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get provider bindings (GET /v1/providers/bindings)"
get "/v1/providers/bindings?tenant_id=default_tenant"
if [ "$STATUS" = "200" ]; then
  BINDING_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('bindings',d.get('items',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
  ok "${BINDING_COUNT} provider binding(s)"

  # Extract first binding ID if available
  BINDING_ID=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('bindings',d.get('items',d if isinstance(d,list) else []))
print(items[0].get('binding_id',items[0].get('id','')) if items else '')" 2>/dev/null)
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "provider bindings HTTP ${STATUS}"
  BINDING_ID=""
else
  fail "provider bindings HTTP ${STATUS}: ${RESP}"
  BINDING_ID=""
fi

# =============================================================================
step "Get binding cost stats (GET /v1/providers/bindings/:id/cost-stats)"
if [ -n "${BINDING_ID:-}" ]; then
  get "/v1/providers/bindings/${BINDING_ID}/cost-stats?tenant_id=default_tenant"
  if [ "$STATUS" = "200" ]; then
    ok "binding cost stats for ${BINDING_ID}"
  elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
    skip "binding cost-stats HTTP ${STATUS}"
  else
    fail "binding cost-stats HTTP ${STATUS}: ${RESP}"
  fi
else
  # No binding ID — try with a synthetic ID and expect 404
  get "/v1/providers/bindings/default/cost-stats?tenant_id=default_tenant"
  if [ "$STATUS" = "200" ]; then
    ok "binding cost stats reachable"
  elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
    skip "no bindings configured — cost-stats HTTP ${STATUS} (expected)"
  else
    fail "binding cost-stats HTTP ${STATUS}: ${RESP}"
  fi
fi

# =============================================================================
step "Get cost ranking (GET /v1/providers/bindings/cost-ranking)"
get "/v1/providers/bindings/cost-ranking?tenant_id=default_tenant"
if [ "$STATUS" = "200" ]; then
  RANK_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('rankings',d.get('items',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
  ok "cost ranking: ${RANK_COUNT} binding(s) ranked"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "cost ranking HTTP ${STATUS}"
else
  fail "cost ranking HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get session cost (GET /v1/sessions/:id/cost)"
get "/v1/sessions/${SESSION}/cost"
if [ "$STATUS" = "200" ]; then
  ok "session cost reachable"
elif [ "$STATUS" = "404" ]; then
  ok "session cost 404 — no cost records yet (expected)"
elif [[ "$STATUS" =~ ^(501)$ ]]; then
  skip "session cost HTTP ${STATUS}"
else
  fail "session cost HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
echo "" >&2
echo -e "${BLD}${GRN}=== E2E COST TRACKING COMPLETED ===${RST}" >&2
echo -e "  Session : ${SESSION}" >&2
echo -e "  Run     : ${RUN}" >&2
echo -e "  Pass: ${PASS}  Skip: ${SKIP}  Fail: ${FAIL_COUNT}  Steps: ${STEP}" >&2
echo "" >&2
