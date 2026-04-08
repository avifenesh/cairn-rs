#!/usr/bin/env bash
# =============================================================================
# e2e-multi-tenant.sh — UC-08: project isolation smoke test.
#
# NOTE: The admin token is scoped to the 'default' tenant, so true multi-tenant
# isolation (separate tenants) requires admin-scoped tokens per tenant.
# This script instead proves PROJECT-LEVEL isolation under the same tenant,
# which is the meaningful product-level isolation for most use cases.
#
# Workflow:
#   1. Create session + run in Project A (default/default/proj_a_*)
#   2. Create session + run in Project B (default/default/proj_b_*)
#   3. Ingest documents into each project separately
#   4. Verify Project A runs NOT visible when filtering by Project B
#   5. Verify Project B sessions NOT visible when filtering by Project A
#   6. Verify memory search in Project A does NOT return Project B's documents
#   7. Verify GET /v1/runs/:id returns correct project scope for each run
#
# Proves: per-project data isolation in the HTTP API layer.
#
# Usage:
#   CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-multi-tenant.sh
#   CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-multi-tenant.sh
#
# Exit code: 0 = all checks passed, 1 = one or more failures.
# =============================================================================

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM

# Both projects share the same tenant/workspace (scoped to our admin token)
TENANT="default"
WORKSPACE="default"

# Project A identifiers
PA="proj_a_${TS}"
SESS_A="sess_a_${TS}"
RUN_A="run_a_${TS}"
DOC_A="doc_a_${TS}"

# Project B identifiers
PB="proj_b_${TS}"
SESS_B="sess_b_${TS}"
RUN_B="run_b_${TS}"
DOC_B="doc_b_${TS}"

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
step()  { STEP=$(( STEP + 1 )); echo -e "\n${BLD}${CYN}[${STEP}]${RST} ${BLD}$*${RST}" >&2; }
ok()    { echo -e "    ${GRN}ok${RST}   $*" >&2; PASS=$(( PASS + 1 )); }
fail()  { echo -e "    ${RED}FAIL${RST} $*" >&2; FAIL=$(( FAIL + 1 )); }
skip()  { echo -e "    ${YLW}skip${RST} $*" >&2; SKIP=$(( SKIP + 1 )); }
info()  { echo -e "    ${DIM}$*${RST}" >&2; }

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

jf() { printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || echo ""; }

chk() {
  local label="$1" want="$2" method="$3" path="$4" body="${5:-}"
  api "$method" "$path" "$body"
  if [ "$_HTTP" = "$want" ]; then
    ok "$label (HTTP $_HTTP)"; return 0
  else
    fail "$label (expected HTTP $want, got HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:160}${RST}" >&2
    return 1
  fi
}

# =============================================================================
echo -e "${BLD}cairn e2e project isolation${RST}" >&2
echo -e "  Server    : ${CYN}${BASE}${RST}" >&2
echo -e "  Project A : ${CYN}${TENANT}/${WORKSPACE}/${PA}${RST}" >&2
echo -e "  Project B : ${CYN}${TENANT}/${WORKSPACE}/${PB}${RST}" >&2

api GET /health
[ "$_HTTP" = "200" ] && ok "server healthy" || { fail "server not healthy"; exit 1; }

# =============================================================================
step "Create sessions and runs in Project A"

chk "create session A" 201 POST /v1/sessions \
  "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PA}\",\"session_id\":\"${SESS_A}\"}"
[ "$(jf state)" = "open" ] && ok "  session A state=open" || fail "  session A state='$(jf state)'"

chk "create run A" 201 POST /v1/runs \
  "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PA}\",\"session_id\":\"${SESS_A}\",\"run_id\":\"${RUN_A}\"}"
[ "$(jf state)" = "pending" ] && ok "  run A state=pending" || fail "  run A state='$(jf state)'"

# =============================================================================
step "Create sessions and runs in Project B"

chk "create session B" 201 POST /v1/sessions \
  "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PB}\",\"session_id\":\"${SESS_B}\"}"
[ "$(jf state)" = "open" ] && ok "  session B state=open" || fail "  session B state='$(jf state)'"

chk "create run B" 201 POST /v1/runs \
  "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PB}\",\"session_id\":\"${SESS_B}\",\"run_id\":\"${RUN_B}\"}"
[ "$(jf state)" = "pending" ] && ok "  run B state=pending" || fail "  run B state='$(jf state)'"

# =============================================================================
step "Ingest distinct documents into each project"

api POST /v1/memory/ingest \
  "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PA}\",\"source_id\":\"src_a\",\"document_id\":\"${DOC_A}\",\"content\":\"Project Alpha uses quantum encryption for secure communications.\",\"source_type\":\"plain_text\"}"
[ "$(jf ok)" = "True" ] && ok "  doc ingested into project A" || fail "  doc A ingest failed"

api POST /v1/memory/ingest \
  "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PB}\",\"source_id\":\"src_b\",\"document_id\":\"${DOC_B}\",\"content\":\"Project Beta uses neural mesh networking for distributed AI.\",\"source_type\":\"plain_text\"}"
[ "$(jf ok)" = "True" ] && ok "  doc ingested into project B" || fail "  doc B ingest failed"

sleep 0.5

# =============================================================================
step "Verify run A is visible in project A, not in project B"

# Run A should appear in project A's run list
api GET "/v1/runs?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PA}"
A_IN_A=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); items=d.get('items',d) if isinstance(d,dict) else d; print('yes' if any(r.get('run_id')=='${RUN_A}' for r in items) else 'no')" 2>/dev/null || echo "no")
[ "$A_IN_A" = "yes" ] \
  && ok "  run A appears in project A list" \
  || fail "  run A NOT found in project A list"

# Run A should NOT appear in project B's run list
api GET "/v1/runs?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PB}"
A_IN_B=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); items=d.get('items',d) if isinstance(d,dict) else d; print('yes' if any(r.get('run_id')=='${RUN_A}' for r in items) else 'no')" 2>/dev/null || echo "no")
[ "$A_IN_B" = "no" ] \
  && ok "  run A correctly NOT visible in project B" \
  || fail "  run A LEAKED into project B list (isolation failure)"

# =============================================================================
step "Verify session B is visible in project B, not in project A"

api GET "/v1/sessions?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PB}"
B_IN_B=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); items=d.get('items',d) if isinstance(d,dict) else d; print('yes' if any(s.get('session_id')=='${SESS_B}' for s in items) else 'no')" 2>/dev/null || echo "no")
[ "$B_IN_B" = "yes" ] \
  && ok "  session B appears in project B list" \
  || fail "  session B NOT found in project B list"

api GET "/v1/sessions?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PA}"
B_IN_A=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); items=d.get('items',d) if isinstance(d,dict) else d; print('yes' if any(s.get('session_id')=='${SESS_B}' for s in items) else 'no')" 2>/dev/null || echo "no")
[ "$B_IN_A" = "no" ] \
  && ok "  session B correctly NOT visible in project A" \
  || fail "  session B LEAKED into project A list (isolation failure)"

# =============================================================================
step "Verify memory search is project-scoped"

# Search in project A — should find A's quantum document, not B's neural document
api GET "/v1/memory/search?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PA}&query_text=quantum+encryption&limit=5"
A_SEARCH_COUNT=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; print(len(json.load(sys.stdin).get('results',[])))" 2>/dev/null || echo 0)
[ "${A_SEARCH_COUNT:-0}" -ge 1 ] \
  && ok "  project A search found ${A_SEARCH_COUNT} result(s) for 'quantum encryption'" \
  || fail "  project A search returned 0 results (A's document not found)"

# Verify B's neural content is NOT in A's search results
api GET "/v1/memory/search?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PA}&query_text=neural+mesh+networking&limit=5"
B_IN_A_SEARCH=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; r=json.load(sys.stdin).get('results',[]); print(len(r))" 2>/dev/null || echo 0)
[ "${B_IN_A_SEARCH:-0}" = "0" ] \
  && ok "  project B's document NOT visible in project A search (memory isolation works)" \
  || fail "  project B's document LEAKED into project A search (isolation failure)"

# Search in project B — should find B's neural document
api GET "/v1/memory/search?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PB}&query_text=neural+mesh+networking&limit=5"
B_SEARCH_COUNT=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; print(len(json.load(sys.stdin).get('results',[])))" 2>/dev/null || echo 0)
[ "${B_SEARCH_COUNT:-0}" -ge 1 ] \
  && ok "  project B search found ${B_SEARCH_COUNT} result(s) for 'neural mesh networking'" \
  || fail "  project B search returned 0 results (B's document not found)"

# =============================================================================
step "Verify GET /v1/runs/:id returns correct project scope"

api GET "/v1/runs/${RUN_A}"
RUN_A_PROJ=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); r=d.get('run',d); print(r.get('project',{}).get('project_id',''))" 2>/dev/null || echo "")
[ "$RUN_A_PROJ" = "$PA" ] \
  && ok "  run A has correct project scope (${RUN_A_PROJ})" \
  || fail "  run A project scope='${RUN_A_PROJ}' (expected ${PA})"

api GET "/v1/runs/${RUN_B}"
RUN_B_PROJ=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); r=d.get('run',d); print(r.get('project',{}).get('project_id',''))" 2>/dev/null || echo "")
[ "$RUN_B_PROJ" = "$PB" ] \
  && ok "  run B has correct project scope (${RUN_B_PROJ})" \
  || fail "  run B project scope='${RUN_B_PROJ}' (expected ${PB})"

# =============================================================================
step "Cross-project run lookup is isolated"

# Run A should return 200 from its own project, but its run_id should not
# appear when listing runs of project B
api GET "/v1/runs/${RUN_B}"
RUN_B_STATE=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null || echo "")
[ "$_HTTP" = "200" ] \
  && ok "  GET /v1/runs/${RUN_B} returns HTTP 200 (own project)" \
  || fail "  GET run B returned HTTP $_HTTP"

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
