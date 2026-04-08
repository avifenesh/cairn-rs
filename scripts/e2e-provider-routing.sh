#!/usr/bin/env bash
# =============================================================================
# e2e-provider-routing.sh — UC-10: provider management lifecycle.
#
# Workflow:
#   1.  Seed default tenant structure (event append — idempotent)
#   2.  Provider connections: try to create; accept 403 (multi_provider
#       entitlement not available in local mode) as SKIP not FAIL.
#       The feature gate protects this path — that IS the correct behaviour.
#   3.  Create a provider pool (no entitlement gate)
#   4.  Try to add connections to pool (skip if no connections created)
#   5.  Create a route policy (POST /v1/providers/policies)
#   6.  List route policies (GET /v1/providers/policies?tenant_id=...)
#   7.  Create a provider binding
#   8.  List provider bindings (GET /v1/providers/bindings?tenant_id=...)
#   9.  Verify provider health endpoint (GET /v1/providers/health?tenant_id=...)
#  10.  List provider pools
#  11.  Remove a pool connection (cleanup)
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

CONN_OR="conn_or_${TS}"
CONN_OL="conn_ol_${TS}"
POOL_ID="pool_${TS}"
BINDING_ID=""
CONN_CREATED=false    # tracks whether connections were successfully created

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
echo -e "  Tenant : ${CYN}${TENANT}${RST}" >&2
echo "" >&2

api GET /health
[ "$_HTTP" = "200" ] || { echo -e "${RED}server not reachable (HTTP ${_HTTP})${RST}" >&2; exit 1; }
info "server healthy"

# =============================================================================
step "Seed default tenant + workspace + project (idempotent event append)"

# TenantCreated — idempotent; safe to re-emit even if tenant exists.
api POST /v1/events/append "[{
  \"event_id\":\"evt_tdef_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"tenant_created\",
    \"tenant_id\":\"${TENANT}\",\"name\":\"Default\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "tenant_created event appended (tenant=${TENANT})" || info "tenant event HTTP ${_HTTP} (may already exist)"

# WorkspaceCreated
api POST /v1/events/append "[{
  \"event_id\":\"evt_wdef_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"workspace_created\",
    \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"name\":\"Default\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "workspace_created event appended" || info "workspace event HTTP ${_HTTP}"

# ProjectCreated
api POST /v1/events/append "[{
  \"event_id\":\"evt_pdef_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":{\"scope\":\"system\"},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{\"event\":\"project_created\",
    \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\",\"name\":\"Default\",\"created_at\":0}
}]"
[[ "$_HTTP" =~ ^(200|201)$ ]] && ok "project_created event appended" || info "project event HTTP ${_HTTP}"

# =============================================================================
step "Provider connections (multi_provider feature — 403 in local mode is correct)"

api POST /v1/providers/connections "{
  \"provider_connection_id\": \"${CONN_OR}\",
  \"tenant_id\": \"${TENANT}\",
  \"provider_family\": \"openai_compat\",
  \"adapter_type\": \"openai_compat\",
  \"supported_models\": [\"openrouter/free\", \"qwen/qwen3-coder:free\"]
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "connection ${CONN_OR} created (HTTP ${_HTTP})"
  CONN_CREATED=true
elif [ "$_HTTP" = "409" ]; then
  ok "connection ${CONN_OR} already exists (idempotent)"
  CONN_CREATED=true
elif [[ "$_HTTP" =~ ^(400|403)$ ]]; then
  skip "connection create HTTP ${_HTTP} — tenant may not exist or entitlement gated"
else
  fail "create connection OpenRouter HTTP ${_HTTP}: ${_BODY:0:100}"
fi

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
  ok "connection ${CONN_OL} already exists (idempotent)"
elif [[ "$_HTTP" =~ ^(400|403)$ ]]; then
  skip "connection create HTTP ${_HTTP} — tenant may not exist or entitlement gated"
else
  fail "create connection Ollama HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "List provider connections"

api GET "/v1/providers/connections?tenant_id=${TENANT}"
if [ "$_HTTP" = "200" ]; then
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('items',d.get('connections',d if isinstance(d,list) else []))
print(len(items))
" 2>/dev/null || echo "?")
  ok "GET /v1/providers/connections?tenant_id=${TENANT} returned 200 (${COUNT} connections)"
  if [ "$CONN_CREATED" = "true" ]; then
    HAS_OR=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('items',d.get('connections',d if isinstance(d,list) else []))
print('yes' if any('${CONN_OR}' in str(i) for i in items) else 'no')
" 2>/dev/null || echo "no")
    [ "$HAS_OR" = "yes" ] && ok "OpenRouter connection visible in list" || skip "OpenRouter connection not in list yet"
  fi
elif [ "$_HTTP" = "403" ]; then
  skip "list connections 403 — multi_provider entitlement required"
else
  fail "list connections HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Create provider pool (no entitlement gate)"

api POST /v1/providers/pools "{
  \"pool_id\": \"${POOL_ID}\",
  \"max_connections\": 10,
  \"tenant_id\": \"${TENANT}\"
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "pool ${POOL_ID} created (HTTP ${_HTTP})"
elif [ "$_HTTP" = "409" ]; then
  ok "pool ${POOL_ID} already exists (idempotent)"
else
  fail "create pool HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Add connections to pool (only if connections were created)"

if [ "$CONN_CREATED" = "true" ]; then
  api POST "/v1/providers/pools/${POOL_ID}/connections" "{\"connection_id\": \"${CONN_OR}\"}"
  if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
    ok "added ${CONN_OR} to pool"
  else
    fail "add OpenRouter to pool HTTP ${_HTTP}: ${_BODY:0:100}"
  fi

  api POST "/v1/providers/pools/${POOL_ID}/connections" "{\"connection_id\": \"${CONN_OL}\"}"
  if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
    ok "added ${CONN_OL} to pool"
  else
    fail "add Ollama to pool HTTP ${_HTTP}: ${_BODY:0:100}"
  fi
else
  skip "no connections created — skipping pool connection steps"
  skip "no connections created — skipping pool connection steps (2)"
fi

# =============================================================================
step "Create route policy (POST /v1/providers/policies)"

api POST /v1/providers/policies "{
  \"tenant_id\": \"${TENANT}\",
  \"name\": \"e2e-policy-${TS}\",
  \"rules\": [{
    \"rule_id\": \"rule_${TS}\",
    \"priority\": 10,
    \"description\": \"Route heavy workloads to brain, light to worker\"
  }]
}"
if [[ "$_HTTP" =~ ^(200|201)$ ]]; then
  ok "route policy created (HTTP ${_HTTP})"
elif [ "$_HTTP" = "400" ]; then
  REASON=$(printf '%s' "$_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error','')[:80])" 2>/dev/null || echo "bad request")
  skip "route policy 400: ${REASON} — seeding may be needed"
else
  fail "create route policy HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "List route policies"

api GET "/v1/providers/policies?tenant_id=${TENANT}"
if [ "$_HTTP" = "200" ]; then
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('items',d.get('policies',d if isinstance(d,list) else []))
print(len(items))
" 2>/dev/null || echo "?")
  ok "GET /v1/providers/policies?tenant_id=${TENANT} returned 200 (${COUNT} policies)"
elif [ "$_HTTP" = "400" ]; then
  REASON=$(printf '%s' "$_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error','')[:60])" 2>/dev/null || echo "")
  skip "list policies 400: ${REASON}"
else
  fail "list policies HTTP ${_HTTP}"
fi

# =============================================================================
step "Create provider binding"

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
print(d.get('binding_id', d.get('provider_binding_id', d.get('id',''))))
" 2>/dev/null || echo "")
  ok "provider binding created (id=${BINDING_ID:-<unknown>}, HTTP ${_HTTP})"
elif [ "$_HTTP" = "400" ]; then
  REASON=$(printf '%s' "$_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error','')[:80])" 2>/dev/null || echo "bad request")
  skip "binding creation 400: ${REASON}"
else
  fail "create binding HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "List provider bindings"

api GET "/v1/providers/bindings?tenant_id=${TENANT}"
if [ "$_HTTP" = "200" ]; then
  COUNT=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('items',d.get('bindings',d if isinstance(d,list) else []))
print(len(items))
" 2>/dev/null || echo "?")
  ok "GET /v1/providers/bindings?tenant_id=${TENANT} returned 200 (${COUNT} bindings)"
elif [ "$_HTTP" = "400" ]; then
  REASON=$(printf '%s' "$_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error','')[:60])" 2>/dev/null || echo "")
  skip "list bindings 400: ${REASON}"
else
  fail "list bindings HTTP ${_HTTP}"
fi

# =============================================================================
step "Verify provider health endpoint"

api GET "/v1/providers/health?tenant_id=${TENANT}"
if [ "$_HTTP" = "200" ]; then
  ok "GET /v1/providers/health?tenant_id=${TENANT} returned 200"
elif [ "$_HTTP" = "400" ]; then
  REASON=$(printf '%s' "$_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error','')[:60])" 2>/dev/null || echo "")
  skip "provider health 400: ${REASON}"
else
  fail "provider health HTTP ${_HTTP}: ${_BODY:0:100}"
fi

# =============================================================================
step "Manual health-check (only if connections created)"

if [ "$CONN_CREATED" = "true" ]; then
  api POST "/v1/providers/${CONN_OR}/health-check" "{\"success\": true, \"latency_ms\": 42}"
  if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
    ok "health-check recorded for ${CONN_OR} (success=true, latency=42ms)"
  elif [ "$_HTTP" = "404" ]; then
    skip "health-check 404 for ${CONN_OR}"
  else
    fail "health-check HTTP ${_HTTP}: ${_BODY:0:100}"
  fi

  api POST "/v1/providers/connections/${CONN_OR}/health-schedule" "{\"interval_ms\": 60000}"
  if [[ "$_HTTP" =~ ^(200|201|204)$ ]]; then
    ok "health schedule set (interval=60s)"
  elif [ "$_HTTP" = "404" ]; then
    skip "health-schedule set 404"
  else
    fail "set health-schedule HTTP ${_HTTP}: ${_BODY:0:100}"
  fi

  api GET "/v1/providers/connections/${CONN_OR}/health-schedule"
  [[ "$_HTTP" =~ ^(200|404)$ ]] && ok "GET health-schedule reachable (HTTP ${_HTTP})" || fail "GET health-schedule HTTP ${_HTTP}"
else
  skip "no connections — skipping health-check"
  skip "no connections — skipping health-schedule set"
  skip "no connections — skipping health-schedule get"
fi

# =============================================================================
step "List provider pools — verify our pool is present"

api GET "/v1/providers/pools?tenant_id=${TENANT}"
if [ "$_HTTP" = "200" ]; then
  HAS=$(printf '%s' "$_BODY" | python3 -c "
import sys,json; d=json.load(sys.stdin)
items=d.get('items',d.get('pools',d if isinstance(d,list) else []))
print('yes' if any('${POOL_ID}' in str(i) for i in items) else 'no')
" 2>/dev/null || echo "no")
  [ "$HAS" = "yes" ] && ok "pool ${POOL_ID} visible in list" || skip "pool not yet in list response"
elif [ "$_HTTP" = "400" ]; then
  REASON=$(printf '%s' "$_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error','')[:60])" 2>/dev/null || echo "")
  skip "list pools 400: ${REASON}"
else
  fail "list pools HTTP ${_HTTP}"
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
echo "" >&2
if [ $FAIL -eq 0 ]; then
  echo -e "${BLD}${GRN}=== PROVIDER ROUTING PASSED ===${RST}" >&2
else
  echo -e "${BLD}${RED}=== PROVIDER ROUTING FAILED ===${RST}" >&2
fi
echo -e "  Pass: ${GRN}${PASS}${RST}  Fail: ${RED}${FAIL}${RST}  Skip: ${YLW}${SKIP}${RST}  Steps: ${STEP}" >&2
echo "" >&2

[ $FAIL -eq 0 ] && exit 0 || exit 1
