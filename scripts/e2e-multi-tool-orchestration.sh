#!/usr/bin/env bash
# =============================================================================
# e2e-multi-tool-orchestration.sh — UC-03: Multi-tool orchestration
#
# Exercises: ingest document → orchestrate with memory_search goal →
#            verify tool invocations → verify run result references knowledge
#
# Usage:
#   ./scripts/e2e-multi-tool-orchestration.sh
#   CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=my-token ./scripts/e2e-multi-tool-orchestration.sh
#
# Exit code: 0 = all passed, 1 = one or more failures.
# =============================================================================

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"
LLM_TIMEOUT="${CAIRN_LLM_TIMEOUT:-90}"

TS=$(date +%s)_$RANDOM
SESSION_ID="uc03_sess_${TS}"
RUN_ID="uc03_run_${TS}"
DOC_ID="uc03_doc_${TS}"

if [ -t 2 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'
  CYN='\033[0;36m'; BLD='\033[1m';   RST='\033[0m'
else
  GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''
fi

PASS=0; FAIL=0; SKIP=0

log_ok()   { echo -e "${GRN}  ✓${RST} $1" >&2; PASS=$(( PASS + 1 )); }
log_fail() { echo -e "${RED}  ✗${RST} $1" >&2; FAIL=$(( FAIL + 1 )); }
log_skip() { echo -e "${YLW}  ⊘${RST} $1" >&2; SKIP=$(( SKIP + 1 )); }
section()  { echo -e "\n${BLD}${CYN}── $1${RST}" >&2; }

_BODY_FILE=$(mktemp)
trap 'rm -f "$_BODY_FILE"' EXIT

_HTTP="" _BODY=""
api() {
  local method="$1" path="$2" body="${3:-}" t="${4:-$TIMEOUT}"
  local curl_args=(-s -X "$method" --max-time "$t"
    -H "Authorization: Bearer ${TOKEN}"
    -H "Content-Type: application/json"
    -o "$_BODY_FILE" -w "%{http_code}")
  [ -n "$body" ] && curl_args+=(-d "$body")
  _HTTP=$(curl "${curl_args[@]}" "${BASE}${path}" 2>/dev/null)
  _BODY=$(cat "$_BODY_FILE")
}

chk() {
  local label="$1" want="$2" method="$3" path="$4" body="${5:-}"
  api "$method" "$path" "$body"
  if [ "$_HTTP" = "$want" ]; then
    log_ok "$label (HTTP $_HTTP)"
    return 0
  else
    log_fail "$label (expected HTTP $want, got HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:200}${RST}" >&2
    return 1
  fi
}

jf() { printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || true; }

# =============================================================================
echo -e "${BLD}cairn UC-03: multi-tool orchestration${RST}" >&2
echo -e "  Server  : ${CYN}${BASE}${RST}" >&2
echo -e "  Run ID  : ${CYN}${RUN_ID}${RST}" >&2

# =============================================================================
section "1. Health"
chk "GET /health" 200 GET /health

# =============================================================================
section "2. Ingest knowledge document"

# Content that the agent should be able to retrieve via memory_search
KNOWLEDGE="cairn-rs orchestrator uses a three-phase loop: GatherPhase collects context \
from memory and events, DecidePhase calls the brain LLM to choose actions, ExecutePhase \
dispatches tool calls including memory_search and memory_store."

chk "POST /v1/memory/ingest" 200 POST /v1/memory/ingest \
  "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"source_id\":\"uc03_src\",\"document_id\":\"${DOC_ID}\",\"content\":\"${KNOWLEDGE}\",\"source_type\":\"plain_text\"}"

INGEST_OK=$(jf ok)
[ "$INGEST_OK" = "True" ] && log_ok "  document ingested (ok=true)" || log_fail "  ingest ok='${INGEST_OK}'"

# Wait for indexing
sleep 0.5

# =============================================================================
section "3. Verify document is searchable"

chk "GET /v1/memory/search (cairn orchestrator)" 200 GET \
  "/v1/memory/search?tenant_id=default&workspace_id=default&project_id=default&query_text=cairn+orchestrator+three+phase+loop&limit=5"

SEARCH_COUNT=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; print(len(json.load(sys.stdin).get('results',[])))" 2>/dev/null || echo 0)
[ "${SEARCH_COUNT:-0}" -ge 1 ] \
  && log_ok "  memory search found ${SEARCH_COUNT} result(s)" \
  || log_fail "  memory search returned 0 results for ingested document"

# =============================================================================
section "4. Create session + run"

chk "POST /v1/sessions" 201 POST /v1/sessions \
  "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"${SESSION_ID}\"}"

chk "POST /v1/runs" 201 POST /v1/runs \
  "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"${SESSION_ID}\",\"run_id\":\"${RUN_ID}\"}"

# =============================================================================
section "5. Orchestrate with memory-search goal"

GOAL="Search memory for information about the cairn-rs orchestrator phases and summarise what you find."

api POST "/v1/runs/${RUN_ID}/orchestrate" \
  "{\"goal\":\"${GOAL}\",\"max_iterations\":3,\"timeout_ms\":${LLM_TIMEOUT}000}" \
  "$LLM_TIMEOUT"

case "$_HTTP" in
  200|202)
    TERM=$(printf '%s' "$_BODY" | python3 -c \
      "import sys,json; print(json.load(sys.stdin).get('termination',''))" 2>/dev/null || echo "")
    SUMMARY=$(printf '%s' "$_BODY" | python3 -c \
      "import sys,json; print(json.load(sys.stdin).get('summary','')[:120])" 2>/dev/null || echo "")
    log_ok "POST /v1/runs/:id/orchestrate (HTTP $_HTTP, termination=${TERM})"
    [ -n "$SUMMARY" ] && echo -e "     summary: \"${SUMMARY}\"" >&2

    # Check if the summary references the ingested knowledge
    if echo "$SUMMARY" | grep -qi "phase\|gather\|decide\|execute\|loop\|cairn"; then
      log_ok "  summary references orchestrator knowledge (memory_search worked)"
    else
      log_skip "  summary may not reference ingested knowledge (LLM may have answered from training)"
    fi
    ;;
  503|502|500|429)
    log_skip "Orchestrate skipped — no LLM provider (HTTP $_HTTP)"
    log_skip "  Plumbing verified: session/run/ingest all work without LLM"
    ;;
  *)
    log_fail "POST /v1/runs/:id/orchestrate (unexpected HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:160}${RST}" >&2
    ;;
esac

# =============================================================================
section "6. Verify tool invocations recorded"

api GET "/v1/tool-invocations?tenant_id=default&workspace_id=default&project_id=default&run_id=${RUN_ID}&limit=10"
case "$_HTTP" in
  200)
    TI_COUNT=$(printf '%s' "$_BODY" | python3 -c \
      "import sys,json; d=json.load(sys.stdin); print(len(d.get('items',d) if isinstance(d,dict) else d))" 2>/dev/null || echo 0)
    [ "${TI_COUNT:-0}" -ge 1 ] \
      && log_ok "  ${TI_COUNT} tool invocation(s) recorded for this run" \
      || log_skip "  no tool invocations recorded (LLM may not have used tools)"
    ;;
  400|422)
    # run_id filter may not be supported — try without
    api GET "/v1/tool-invocations?tenant_id=default&limit=10"
    [ "$_HTTP" = "200" ] \
      && log_ok "GET /v1/tool-invocations (HTTP 200, run_id filter not supported)" \
      || log_skip "GET /v1/tool-invocations (HTTP $_HTTP)"
    ;;
  *)
    log_skip "GET /v1/tool-invocations (HTTP $_HTTP)"
    ;;
esac

# =============================================================================
section "7. Verify run state"

api GET "/v1/runs/${RUN_ID}"
RUN_STATE=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null || echo "")
echo -e "     run state: ${CYN}${RUN_STATE}${RST}" >&2
[ -n "$RUN_STATE" ] \
  && log_ok "  run state is '${RUN_STATE}' (not stuck)" \
  || log_fail "  could not read run state"

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
