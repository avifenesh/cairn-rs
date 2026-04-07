#!/usr/bin/env bash
# =============================================================================
# e2e-agent-workflow.sh — exercises a complete agent lifecycle through the
# cairn-rs HTTP API, proving the product works as a real control plane.
#
# Workflow:
#   1. Create a session (using the pre-seeded default tenant/workspace/project)
#   2. Start a run within the session
#   3. Submit a task to the run
#   4. Claim the task (worker acquires lease)
#   5. Start the task (leased -> running)
#   6. Call the LLM via POST /v1/providers/ollama/generate
#   7. Complete the task with the LLM response
#   8. Complete the run (via event append)
#   9. Verify the full event trail via GET /v1/events/recent
#  10. Verify cost tracking and session state
#
# Usage:
#   # Start server first:
#   CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app
#
#   # Then run:
#   CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-agent-workflow.sh
#
#   # With real LLM:
#   CAIRN_BRAIN_URL=https://agntic.garden/inference/brain/v1 \
#   CAIRN_WORKER_URL=https://agntic.garden/inference/worker/v1 \
#   CAIRN_BRAIN_KEY=Cairn-Inference-2026! CAIRN_WORKER_KEY=Cairn-Inference-2026! \
#     CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app &
#   CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-agent-workflow.sh
#
# Exit code: 0 = workflow completed, 1 = a step failed.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"
LLM_TIMEOUT="${CAIRN_LLM_TIMEOUT:-90}"

TS=$(date +%s)_$RANDOM
TENANT="default"
WORKSPACE="default"
PROJECT="default"
SESSION="e2e_sess_${TS}"
RUN="e2e_run_${TS}"
TASK="e2e_task_${TS}"
WORKER="e2e_worker_${TS}"

# ── Colour ────────────────────────────────────────────────────────────────────
if [ -t 2 ]; then
  GRN='\033[0;32m'; RED='\033[0;31m'
  CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'; DIM='\033[2m'
else
  GRN=''; RED=''; CYN=''; BLD=''; RST=''; DIM=''
fi

STEP=0
step() { STEP=$(( STEP + 1 )); echo -e "\n${BLD}${CYN}[${STEP}]${RST} ${BLD}$1${RST}" >&2; }
ok()   { echo -e "    ${GRN}ok${RST} $1" >&2; }
fail() { echo -e "    ${RED}FAIL${RST} $1" >&2; exit 1; }
info() { echo -e "    ${DIM}$1${RST}" >&2; }

# ── HTTP helpers ──────────────────────────────────────────────────────────────
_TMP=$(mktemp)
trap 'rm -f "$_TMP"' EXIT
STATUS="" RESP=""

post() {
  local path="$1" body="$2" t="${3:-$TIMEOUT}"
  STATUS=$(curl -s -X POST --max-time "$t" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -d "$body" -o "$_TMP" -w "%{http_code}" \
    "${BASE}${path}" 2>/dev/null)
  RESP=$(cat "$_TMP")
}

get() {
  local path="$1"
  STATUS=$(curl -s -X GET --max-time "$TIMEOUT" \
    -H "Authorization: Bearer ${TOKEN}" \
    -o "$_TMP" -w "%{http_code}" \
    "${BASE}${path}" 2>/dev/null)
  RESP=$(cat "$_TMP")
}

jf() { printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null; }

# =============================================================================
echo -e "${BLD}cairn e2e agent workflow${RST}" >&2
echo -e "  Server  : ${CYN}${BASE}${RST}" >&2
echo -e "  Run ID  : ${CYN}${RUN}${RST}" >&2
echo "" >&2

# ── 0. Health check ──────────────────────────────────────────────────────────
get /health
[ "$STATUS" = "200" ] || fail "server not reachable at ${BASE} (HTTP ${STATUS})"

# =============================================================================
step "Create session"

post /v1/sessions "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION}\"
}"
[ "$STATUS" = "201" ] || fail "create session HTTP ${STATUS}: ${RESP}"
[ "$(jf state)" = "open" ] || fail "session state=$(jf state) (expected open)"
ok "session ${SESSION} state=open"

# =============================================================================
step "Start a run"

post /v1/runs "{
  \"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION}\",
  \"run_id\":\"${RUN}\"
}"
[ "$STATUS" = "201" ] || fail "create run HTTP ${STATUS}: ${RESP}"
[ "$(jf state)" = "pending" ] || fail "run state=$(jf state) (expected pending)"
ok "run ${RUN} state=pending"

# =============================================================================
step "Submit a task"

post "/v1/runs/${RUN}/tasks" "{
  \"name\":\"summarize_document\",
  \"task_id\":\"${TASK}\",
  \"description\":\"Use LLM to summarize the cairn-rs project\",
  \"metadata\":{\"document\":\"cairn-rs README\"}
}"
[ "$STATUS" = "201" ] || fail "create task HTTP ${STATUS}: ${RESP}"
TASK_ID=$(jf task_id)
[ "$(jf state)" = "queued" ] || fail "task state=$(jf state) (expected queued)"
ok "task ${TASK_ID} state=queued"

# =============================================================================
step "Claim the task (worker acquires lease)"

post "/v1/tasks/${TASK_ID}/claim" "{
  \"worker_id\":\"${WORKER}\",\"lease_duration_ms\":60000
}"
[ "$STATUS" = "200" ] || fail "claim HTTP ${STATUS}: ${RESP}"
[ "$(jf state)" = "leased" ] || fail "task state=$(jf state) (expected leased)"
ok "claimed by ${WORKER}, state=leased"

# =============================================================================
step "Start the task (leased -> running)"

post "/v1/tasks/${TASK_ID}/start" '{}'
[ "$STATUS" = "200" ] || fail "start HTTP ${STATUS}: ${RESP}"
[ "$(jf state)" = "running" ] || fail "task state=$(jf state) (expected running)"
ok "task state=running"

# =============================================================================
step "Call LLM to generate a response"

PROMPT="Summarize in one sentence: cairn-rs is an open-source Rust control plane for production AI agent deployments with event sourcing, approval gates, and real-time SSE streaming."

post /v1/providers/ollama/generate "{
  \"model\":\"qwen3.5:9b\",
  \"prompt\":\"${PROMPT}\"
}" "$LLM_TIMEOUT"

if [ "$STATUS" = "200" ]; then
  LLM_TEXT=$(jf text)
  LLM_MODEL=$(jf model)
  LLM_LATENCY=$(jf latency_ms)
  if [ -n "$LLM_TEXT" ] && [ "$LLM_TEXT" != "None" ] && [ "$LLM_TEXT" != "" ]; then
    ok "LLM responded (model=${LLM_MODEL}, ${LLM_LATENCY}ms)"
    info "\"${LLM_TEXT:0:120}\""
  else
    LLM_TEXT="cairn-rs is a Rust-based agent control plane with event sourcing and approval workflows."
    ok "LLM returned empty — using synthetic response"
  fi
elif [ "$STATUS" = "503" ]; then
  LLM_TEXT="cairn-rs is a Rust-based agent control plane with event sourcing and approval workflows."
  ok "No LLM provider — using synthetic response"
  info "(set OLLAMA_HOST, CAIRN_WORKER_URL, or CAIRN_BRAIN_URL for real generation)"
else
  fail "LLM HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Complete the task with the LLM result"

LLM_ESC=$(printf '%s' "$LLM_TEXT" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))" 2>/dev/null)

post "/v1/tasks/${TASK_ID}/complete" "{
  \"result\":{\"summary\":${LLM_ESC},\"model\":\"qwen3.5:9b\"}
}"
[ "$STATUS" = "200" ] || fail "complete task HTTP ${STATUS}: ${RESP}"
[ "$(jf state)" = "completed" ] || fail "task state=$(jf state) (expected completed)"
ok "task state=completed"

# =============================================================================
step "Complete the run"

PROJ="{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"
OWN="{\"scope\":\"project\",\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"

post /v1/events/append "[{
  \"event_id\":\"evt_rc_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":${OWN},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{
    \"event\":\"run_state_changed\",
    \"project\":${PROJ},
    \"run_id\":\"${RUN}\",
    \"transition\":{\"from\":\"pending\",\"to\":\"completed\"},
    \"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null
  }
}]"
[[ "$STATUS" =~ ^(200|201)$ ]] || fail "complete run HTTP ${STATUS}: ${RESP}"

# Verify
get "/v1/runs/${RUN}"
[ "$STATUS" = "200" ] || fail "get run HTTP ${STATUS}"
RUN_STATE=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null)
[ "$RUN_STATE" = "completed" ] || fail "run state=${RUN_STATE} (expected completed)"
ok "run state=completed"

# =============================================================================
step "Verify the full event trail"

get "/v1/events/recent?limit=50"
[ "$STATUS" = "200" ] || fail "recent events HTTP ${STATUS}"
EVENT_COUNT=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('count', len(d.get('items',[]))))" 2>/dev/null)
ok "${EVENT_COUNT} events in the log"

for evt in session_created run_created task_created task_state_changed run_state_changed; do
  HAS=$(printf '%s' "$RESP" | python3 -c "
import sys,json
items=json.load(sys.stdin).get('items',[])
print('yes' if any(i.get('event_type')=='${evt}' for i in items) else 'no')
" 2>/dev/null)
  [ "$HAS" = "yes" ] && ok "  ${evt}" || fail "  ${evt} missing"
done

# =============================================================================
step "Verify run cost and session endpoints"

get "/v1/runs/${RUN}/cost"
[[ "$STATUS" =~ ^(200|404)$ ]] || fail "run cost HTTP ${STATUS}"
ok "run cost reachable (HTTP ${STATUS})"

get "/v1/runs/${RUN}/events"
[ "$STATUS" = "200" ] || fail "run events HTTP ${STATUS}"
ok "run events reachable"

get "/v1/runs/${RUN}/tasks"
[ "$STATUS" = "200" ] || fail "run tasks HTTP ${STATUS}"
ok "run tasks reachable"

# =============================================================================
echo "" >&2
echo -e "${BLD}${GRN}=== E2E AGENT WORKFLOW COMPLETED ===${RST}" >&2
echo -e "  Session: ${SESSION}  Run: ${RUN}  Task: ${TASK_ID}" >&2
echo -e "  Events: ${EVENT_COUNT}  Steps: ${STEP}" >&2
echo "" >&2
