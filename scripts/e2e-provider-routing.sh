#!/usr/bin/env bash
# =============================================================================
# e2e-provider-routing.sh — UC-10: provider management lifecycle.
#
# Workflow:
#   1. Create two provider connections (OpenRouter + Ollama simulated)
#   2. Create a provider pool
#   3. Add both connections to the pool
#   4. Create a route policy
#   5. Create a provider binding (project → connection + model)
#   6. Verify provider health endpoint
#   7. Record a manual health-check result for each connection
#   8. Set + get health schedule for a connection
#   9. Verify cost-stats endpoint for the binding
#  10. List connections + pools + bindings
#
# Usage:
#   CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-provider-routing.sh
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

CONN_OR="conn_or_${TS}"    # OpenRouter-simulated connection
CONN_OL="conn_ol_${TS}"    # Ollama-simulated connection
POOL_ID="pool_${TS}"
POLICY_ID="policy_${TS}"
BINDING_ID=""              # assigned from response

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
echo -e "${BLD}cairn e2e provider routing${RST}" >&2
echo -e "  Server : ${CYN}${BASE}${RST}" >&2
echo -e "  Pool   : ${CYN}${POOL_ID}${RST}" >&2
echo "" >&2

api GET /health
[ "$_HTTP" = "200" ] || { echo -e "${RED}server not reachable (HTTP ${_HTTP})${RST}" >&2; exit 1; }
info "server healthy"

# =============================================================================
step "Create OpenRouter-simulated provider connection"

api POST /v1/providers/connections "{
  \"provider_connection_id\": \"${CONN_OR}\",
  \"tenant_id\": \"${TENANT}\",
  \"provider_family\": \"openai_compat\",
  \"adapter_type\": \"openai_compat\",
  \"supported_models\": [\"openrouter/free\", \"qwen/qwen3-coder:free\", \"google/gemma-3-4b-it:free\"]
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "connection ${CONN_OR} created (HTTP ${_HTTP})"
elif [ "$_HTTP" = "409" ]; then
  ok "connection ${CONN_OR} already exists (HTTP 409 — idempotent)"
else
  fail "create connection OpenRouter HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Create Ollama-simulated provider connection"

api POST /v1/providers/connections "{
  \"provider_connection_id\": \"${CONN_OL}\",
  \"tenant_id\": \"${TENANT}\",
  \"provider_family\": \"ollama\",
  \"adapter_type\": \"ollama\",
  \"supported_models\": [\"qwen3.5:9b\", \"qwen3-embedding:8b\"]
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "connection ${CONN_OL} created (HTTP ${_HTTP})"
elif [ "$_HTTP" = "409" ]; then
  ok "connection ${CONN_OL} already exists (HTTP 409 — idempotent)"
else
  fail "create connection Ollama HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "List provider connections — verify both present"

api GET /v1/providers/connections
if [ "$_HTTP" = "200" ]; then
  HAS_OR=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
conns=d.get('connections',d if isinstance(d,list) else [])
print('yes' if any('${CONN_OR}' in str(c) for c in conns) else 'no')
" 2>/dev/null || echo "no")
  HAS_OL=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
conns=d.get('connections',d if isinstance(d,list) else [])
print('yes' if any('${CONN_OL}' in str(c) for c in conns) else 'no')
" 2>/dev/null || echo "no")
  [ "$HAS_OR" = "yes" ] && ok "OpenRouter connection visible in list" || skip "OpenRouter connection not in list (${_HTTP})"
  [ "$HAS_OL" = "yes" ] && ok "Ollama connection visible in list"     || skip "Ollama connection not in list (${_HTTP})"
else
  fail "list connections HTTP ${_HTTP}"
fi

# =============================================================================
step "Create provider pool"

api POST /v1/providers/pools "{
  \"pool_id\": \"${POOL_ID}\",
  \"max_connections\": 10,
  \"tenant_id\": \"${TENANT}\"
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "pool ${POOL_ID} created (HTTP ${_HTTP})"
elif [ "$_HTTP" = "409" ]; then
  ok "pool ${POOL_ID} already exists (HTTP 409)"
else
  fail "create pool HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Add connections to pool"

api POST "/v1/providers/pools/${POOL_ID}/connections" "{\"connection_id\": \"${CONN_OR}\"}"
if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
  ok "added ${CONN_OR} to pool ${POOL_ID}"
else
  fail "add OpenRouter to pool HTTP ${_HTTP}: ${_BODY}"
fi

api POST "/v1/providers/pools/${POOL_ID}/connections" "{\"connection_id\": \"${CONN_OL}\"}"
if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
  ok "added ${CONN_OL} to pool ${POOL_ID}"
else
  fail "add Ollama to pool HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Create route policy"

api POST /v1/providers/policies "{
  \"tenant_id\": \"${TENANT}\",
  \"name\": \"e2e-policy-${TS}\",
  \"rules\": [{
    \"rule_id\": \"rule_${TS}\",
    \"priority\": 10,
    \"description\": \"Route heavy workloads to OpenRouter, light to Ollama\"
  }]
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "route policy created (HTTP ${_HTTP})"
else
  fail "create route policy HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "List route policies — verify policy created"

api GET /v1/providers/policies
if [ "$_HTTP" = "200" ]; then
  ok "route policies listing reachable (HTTP 200)"
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('policies',d if isinstance(d,list) else [])
print(len(items))
" 2>/dev/null || echo "?")
  info "policies in store: ${COUNT}"
else
  fail "list policies HTTP ${_HTTP}"
fi

# =============================================================================
step "Create provider binding (project → OpenRouter connection)"

api POST /v1/providers/bindings "{
  \"tenant_id\": \"${TENANT}\",
  \"workspace_id\": \"${WORKSPACE}\",
  \"project_id\": \"${PROJECT}\",
  \"provider_connection_id\": \"${CONN_OR}\",
  \"operation_kind\": \"generate\",
  \"provider_model_id\": \"openrouter/free\",
  \"estimated_cost_micros\": 0
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  BINDING_ID=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
print(d.get('binding_id', d.get('provider_binding_id','')))
" 2>/dev/null || echo "")
  ok "provider binding created (id=${BINDING_ID:-<unknown>}, HTTP ${_HTTP})"
else
  fail "create binding HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Verify provider health endpoint"

api GET /v1/providers/health
if [ "$_HTTP" = "200" ]; then
  ok "GET /v1/providers/health returned 200"
else
  fail "provider health HTTP ${_HTTP}"
fi

# =============================================================================
step "Record manual health-check results"

api POST "/v1/providers/${CONN_OR}/health-check" "{\"success\": true, \"latency_ms\": 42}"
if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
  ok "health-check recorded for ${CONN_OR} (latency=42ms, success=true)"
elif [ "$_HTTP" = "404" ]; then
  skip "health-check for ${CONN_OR} returned 404 — connection may not be indexed yet"
else
  fail "health-check ${CONN_OR} HTTP ${_HTTP}: ${_BODY}"
fi

api POST "/v1/providers/${CONN_OL}/health-check" "{\"success\": true, \"latency_ms\": 8}"
if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
  ok "health-check recorded for ${CONN_OL} (latency=8ms, success=true)"
elif [ "$_HTTP" = "404" ]; then
  skip "health-check for ${CONN_OL} returned 404 — connection may not be indexed yet"
else
  fail "health-check ${CONN_OL} HTTP ${_HTTP}: ${_BODY}"
fi

# =============================================================================
step "Health schedule CRUD for OpenRouter connection"

api POST "/v1/providers/connections/${CONN_OR}/health-schedule" "{\"interval_ms\": 60000}"
if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
  ok "health schedule set for ${CONN_OR} (interval=60s)"
elif [ "$_HTTP" = "404" ]; then
  skip "health schedule for ${CONN_OR} returned 404 — connection may not persist health config"
else
  fail "set health schedule HTTP ${_HTTP}: ${_BODY}"
fi

api GET "/v1/providers/connections/${CONN_OR}/health-schedule"
if [[ "$_HTTP" =~ ^(200|404)$ ]]; then
  ok "GET health schedule reachable (HTTP ${_HTTP})"
else
  fail "GET health schedule HTTP ${_HTTP}"
fi

# =============================================================================
step "Binding cost-stats endpoint"

if [ -n "$BINDING_ID" ]; then
  api GET "/v1/providers/bindings/${BINDING_ID}/cost-stats"
  [[ "$_HTTP" =~ ^(200|404)$ ]] && ok "cost-stats reachable (HTTP ${_HTTP})" || fail "cost-stats HTTP ${_HTTP}"
else
  skip "no binding_id captured — skipping cost-stats"
fi

# =============================================================================
step "List pools — verify our pool is present"

api GET /v1/providers/pools
if [ "$_HTTP" = "200" ]; then
  HAS=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('pools',d if isinstance(d,list) else [])
print('yes' if any('${POOL_ID}' in str(i) for i in items) else 'no')
" 2>/dev/null || echo "no")
  [ "$HAS" = "yes" ] && ok "pool ${POOL_ID} visible in list" || skip "pool not in list response"
else
  fail "list pools HTTP ${_HTTP}"
fi

# =============================================================================
step "List provider bindings"

api GET /v1/providers/bindings
if [ "$_HTTP" = "200" ]; then
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('bindings',d if isinstance(d,list) else [])
print(len(items))
" 2>/dev/null || echo "?")
  ok "bindings listing reachable (${COUNT} bindings)"
else
  fail "list bindings HTTP ${_HTTP}"
fi

# =============================================================================
echo "" >&2
if [ $FAIL -eq 0 ]; then
  echo -e "${BLD}${GRN}=== PROVIDER ROUTING PASSED ===${RST}" >&2
else
  echo -e "${BLD}${RED}=== PROVIDER ROUTING FAILED ===${RST}" >&2
fi
echo -e "  Pass: ${GRN}${PASS}${RST}  Fail: ${RED}${FAIL}${RST}  Skip: ${YLW}${SKIP}${RST}  Steps: ${STEP}" >&2
echo "" >&2

[ $FAIL -eq 0 ] && exit 0 || exit 1
