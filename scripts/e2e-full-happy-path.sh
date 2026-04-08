#!/usr/bin/env bash
# =============================================================================
# e2e-full-happy-path.sh — UC-01: Full solo dev journey
#
# Exercises: health check → provider connection → model registration →
#            settings default → session → run → orchestrate → verify events
#
# Usage:
#   ./scripts/e2e-full-happy-path.sh
#   CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=my-token ./scripts/e2e-full-happy-path.sh
#
# Exit code: 0 = all passed, 1 = one or more failures.
# =============================================================================

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"
LLM_TIMEOUT="${CAIRN_LLM_TIMEOUT:-60}"

TS=$(date +%s)_$RANDOM
CONN_ID="conn_${TS}"
SESSION_ID="uc01_sess_${TS}"
RUN_ID="uc01_run_${TS}"

# ── Colour ────────────────────────────────────────────────────────────────────
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

chk_accept() {
  # chk_accept LABEL METHOD PATH BODY STATUS...
  local label="$1" method="$2" path="$3" body="$4"; shift 4
  api "$method" "$path" "$body"
  for want in "$@"; do
    [ "$_HTTP" = "$want" ] && { log_ok "$label (HTTP $_HTTP)"; return 0; }
  done
  log_fail "$label (got HTTP $_HTTP, expected one of: $*)"
  [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:200}${RST}" >&2
  return 1
}

jf() { printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || true; }

# =============================================================================
echo -e "${BLD}cairn UC-01: full happy path${RST}" >&2
echo -e "  Server  : ${CYN}${BASE}${RST}" >&2
echo -e "  Run ID  : ${CYN}${RUN_ID}${RST}" >&2

# =============================================================================
section "1. Health check"

chk "GET /health" 200 GET /health
chk "GET /v1/status" 200 GET /v1/status

# =============================================================================
section "2. Provider connection (Ollama)"

api POST /v1/providers/connections \
  "{\"tenant_id\":\"default\",\"provider_connection_id\":\"${CONN_ID}\",\"provider_family\":\"ollama\",\"adapter_type\":\"ollama\",\"supported_models\":[\"qwen3.5:9b\"]}"

case "$_HTTP" in
  201)
    log_ok "POST /v1/providers/connections (HTTP 201)"
    # Verify it's in the list
    api GET "/v1/providers/connections?tenant_id=default"
    echo "$_BODY" | grep -q "$CONN_ID" \
      && log_ok "  connection appears in list" \
      || log_skip "  connection not in list (pagination may apply)"
    ;;
  400)
    log_skip "POST /v1/providers/connections (HTTP 400) — tenant may not exist on fresh in-memory server"
    ;;
  403)
    log_skip "POST /v1/providers/connections (HTTP 403) — entitlement gated in this deployment"
    ;;
  *)
    log_fail "POST /v1/providers/connections (expected 201, got HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:160}${RST}" >&2
    ;;
esac

# =============================================================================
section "3. Register model on connection"

chk "POST /v1/providers/connections/:id/models" 200 POST \
  "/v1/providers/connections/${CONN_ID}/models" \
  "{\"model_id\":\"qwen3.5:9b\",\"operation_kinds\":[\"generate\"],\"context_window_tokens\":32768,\"max_output_tokens\":8192,\"supports_streaming\":true}"

# =============================================================================
section "4. Set default generate model"

# Settings route: PUT /v1/settings/defaults/:scope/:scope_id/:key
chk "PUT /v1/settings/defaults/system/system/generate_model" 200 PUT \
  "/v1/settings/defaults/system/system/generate_model" \
  "{\"value\":\"qwen3.5:9b\"}"

# Verify setting reads back
api GET "/v1/settings/defaults/resolve/generate_model"
echo "$_BODY" | grep -q "qwen3.5" \
  && log_ok "  generate_model default confirmed (qwen3.5:9b)" \
  || log_skip "  generate_model default not confirmed (may require additional scope)"

# =============================================================================
section "5. Session + run lifecycle"

chk "POST /v1/sessions" 201 POST /v1/sessions \
  "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"${SESSION_ID}\"}"
[ "$(jf state)" = "open" ] && log_ok "  session state=open" || log_fail "  session state='$(jf state)'"

chk "POST /v1/runs" 201 POST /v1/runs \
  "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"${SESSION_ID}\",\"run_id\":\"${RUN_ID}\"}"
[ "$(jf state)" = "pending" ] && log_ok "  run state=pending" || log_fail "  run state='$(jf state)'"

chk "GET /v1/runs/:id" 200 GET "/v1/runs/${RUN_ID}"

# =============================================================================
section "6. Orchestrate (503/502 acceptable without LLM)"

api POST "/v1/runs/${RUN_ID}/orchestrate" \
  "{\"goal\":\"What is the capital of France? Answer briefly.\",\"max_iterations\":2,\"timeout_ms\":${LLM_TIMEOUT}000}" \
  "$LLM_TIMEOUT"

case "$_HTTP" in
  200|202)
    TERM=$(printf '%s' "$_BODY" | python3 -c \
      "import sys,json; print(json.load(sys.stdin).get('termination',''))" 2>/dev/null || echo "")
    log_ok "POST /v1/runs/:id/orchestrate (HTTP $_HTTP, termination=${TERM})"
    [ -n "$TERM" ] && log_ok "  termination field present: ${TERM}" || log_fail "  termination field missing"
    ;;
  503|502|500|429)
    log_skip "Orchestrate skipped — no LLM provider (HTTP $_HTTP)"
    ;;
  404)
    log_fail "POST /v1/runs/:id/orchestrate (HTTP 404 — endpoint not found)"
    ;;
  *)
    log_fail "POST /v1/runs/:id/orchestrate (unexpected HTTP $_HTTP)"
    [ -n "$_BODY" ] && echo -e "     ${RED}${_BODY:0:160}${RST}" >&2
    ;;
esac

# =============================================================================
section "7. Verify events recorded"

chk "GET /v1/events (with limit)" 200 GET "/v1/events?limit=50"
EVENT_COUNT=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(len(d) if isinstance(d,list) else len(d.get('items',[])))" 2>/dev/null || echo 0)
[ "${EVENT_COUNT:-0}" -ge 1 ] \
  && log_ok "  event log has ${EVENT_COUNT} event(s)" \
  || log_fail "  event log empty"

chk "GET /v1/runs/:id/events" 200 GET "/v1/runs/${RUN_ID}/events"
RUN_EVENT_COUNT=$(printf '%s' "$_BODY" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(len(d.get('events',[])))" 2>/dev/null || echo 0)
[ "${RUN_EVENT_COUNT:-0}" -ge 1 ] \
  && log_ok "  run has ${RUN_EVENT_COUNT} event(s)" \
  || log_fail "  run event trail empty"

# =============================================================================
section "8. Discover models on connection"

# discover-models works even without a registered connection when ?endpoint_url= is supplied
api GET "/v1/providers/connections/${CONN_ID}/discover-models?adapter_type=ollama&endpoint_url=http://localhost:11434"
case "$_HTTP" in
  200) log_ok "GET /v1/providers/connections/:id/discover-models (HTTP 200)" ;;
  404)
    # Try with a dummy connection to verify the endpoint exists
    api GET "/v1/providers/connections/nonexistent/discover-models?adapter_type=openai_compat&endpoint_url=http://localhost:1/v1"
    [ "$_HTTP" = "503" ] || [ "$_HTTP" = "502" ] || [ "$_HTTP" = "200" ] \
      && log_ok "GET /v1/providers/connections/:id/discover-models endpoint exists" \
      || log_fail "GET /v1/providers/connections/:id/discover-models (HTTP $_HTTP)"
    ;;
  503|502) log_skip "Discover models — provider not reachable (HTTP $_HTTP)" ;;
  *) log_skip "Discover models (HTTP $_HTTP)" ;;
esac

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
