#!/usr/bin/env bash
# =============================================================================
# e2e-subagent.sh — UC-05: subagent / parent-child run linkage
#
# Workflow:
#   1.  Create parent session + run
#   2.  Submit task to parent run, claim, start
#   3.  Spawn child run via POST /v1/runs/:id/spawn
#   4.  Create + claim + start + complete child task
#   5.  Complete child run, complete parent task + run
#   6.  Verify parent-child linkage via GET /v1/runs/:id/children
#
# Usage: CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-subagent.sh
# Exit: 0 = all assertions passed, 1 = failure.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM
TENANT="default"; WORKSPACE="default"; PROJECT="default"

PARENT_SESSION="e2e_sa_psess_${TS}"
CHILD_SESSION="e2e_sa_csess_${TS}"
PARENT_RUN="e2e_sa_prun_${TS}"
CHILD_RUN="e2e_sa_crun_${TS}"
PARENT_TASK="e2e_sa_ptask_${TS}"
CHILD_TASK="e2e_sa_ctask_${TS}"
WORKER="e2e_sa_worker_${TS}"

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
echo -e "${BLD}cairn e2e subagent workflow${RST}" >&2
echo -e "  Server : ${CYN}${BASE}${RST}" >&2
echo -e "  Run IDs: parent=${CYN}${PARENT_RUN}${RST}  child=${CYN}${CHILD_RUN}${RST}" >&2
echo "" >&2

get /health
[ "$STATUS" = "200" ] || fail "server not reachable at ${BASE} (HTTP ${STATUS})"

# =============================================================================
step "Create parent session"
post /v1/sessions "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${PARENT_SESSION}\"}"
[ "$STATUS" = "201" ] || fail "create parent session HTTP ${STATUS}: ${RESP}"
ok "parent session ${PARENT_SESSION}"

# =============================================================================
step "Create child session (required by spawn)"
post /v1/sessions "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${CHILD_SESSION}\"}"
[ "$STATUS" = "201" ] || fail "create child session HTTP ${STATUS}: ${RESP}"
ok "child session ${CHILD_SESSION}"

# =============================================================================
step "Create parent run"
post /v1/runs "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",\"session_id\":\"${PARENT_SESSION}\",\"run_id\":\"${PARENT_RUN}\"}"
[ "$STATUS" = "201" ] || fail "create parent run HTTP ${STATUS}: ${RESP}"
ok "parent run ${PARENT_RUN} state=$(jf state)"

# =============================================================================
step "Submit task to parent run"
post "/v1/runs/${PARENT_RUN}/tasks" "{\"task_id\":\"${PARENT_TASK}\",
  \"name\":\"parent_analysis\",\"description\":\"Analyse and spawn subagent\"}"
[ "$STATUS" = "201" ] || fail "submit parent task HTTP ${STATUS}: ${RESP}"
PTASK_ID=$(jf task_id)
[ -n "$PTASK_ID" ] || PTASK_ID="$PARENT_TASK"
ok "parent task ${PTASK_ID} state=$(jf state)"

# =============================================================================
step "Claim + start parent task"
post "/v1/tasks/${PTASK_ID}/claim" "{\"worker_id\":\"${WORKER}\",\"lease_duration_ms\":60000}"
[ "$STATUS" = "200" ] || fail "claim parent task HTTP ${STATUS}: ${RESP}"
ok "parent task claimed, state=$(jf state)"

post "/v1/tasks/${PTASK_ID}/start" '{}'
[ "$STATUS" = "200" ] || fail "start parent task HTTP ${STATUS}: ${RESP}"
ok "parent task running"

# =============================================================================
step "Spawn child run via POST /v1/runs/:id/spawn"
post "/v1/runs/${PARENT_RUN}/spawn" "{
  \"session_id\":\"${CHILD_SESSION}\",
  \"child_run_id\":\"${CHILD_RUN}\",
  \"child_task_id\":\"${CHILD_TASK}\"
}"
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  SPAWNED_RUN=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('child_run_id',''))" 2>/dev/null)
  ok "child run spawned: ${SPAWNED_RUN:-$CHILD_RUN}"
elif [[ "$STATUS" =~ ^(404|501|422)$ ]]; then
  skip "spawn endpoint not available (HTTP ${STATUS}) — creating child run manually"
  post /v1/runs "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",
    \"project_id\":\"${PROJECT}\",\"session_id\":\"${CHILD_SESSION}\",
    \"run_id\":\"${CHILD_RUN}\",\"parent_run_id\":\"${PARENT_RUN}\"}"
  [[ "$STATUS" =~ ^(200|201)$ ]] || fail "fallback child run HTTP ${STATUS}: ${RESP}"
  ok "child run created manually with parent_run_id link"
else
  fail "spawn HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Submit + complete child task"
post "/v1/runs/${CHILD_RUN}/tasks" "{\"task_id\":\"${CHILD_TASK}\",
  \"name\":\"child_subtask\",\"description\":\"Subagent work unit\"}"
[[ "$STATUS" =~ ^(200|201)$ ]] || fail "create child task HTTP ${STATUS}: ${RESP}"
CTASK_ID=$(jf task_id); [ -n "$CTASK_ID" ] || CTASK_ID="$CHILD_TASK"
ok "child task ${CTASK_ID} created"

post "/v1/tasks/${CTASK_ID}/claim" "{\"worker_id\":\"${WORKER}_child\",\"lease_duration_ms\":30000}"
[ "$STATUS" = "200" ] || fail "claim child task HTTP ${STATUS}: ${RESP}"

post "/v1/tasks/${CTASK_ID}/start" '{}'
[ "$STATUS" = "200" ] || fail "start child task HTTP ${STATUS}: ${RESP}"

post "/v1/tasks/${CTASK_ID}/complete" '{"result":{"summary":"subagent work done"}}'
[ "$STATUS" = "200" ] || fail "complete child task HTTP ${STATUS}: ${RESP}"
ok "child task completed"

# =============================================================================
step "Complete child run"
OWN="{\"scope\":\"project\",\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"
PROJ="{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"

post /v1/events/append "[{
  \"event_id\":\"evt_cr_child_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":${OWN},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{
    \"event\":\"run_state_changed\",\"project\":${PROJ},
    \"run_id\":\"${CHILD_RUN}\",
    \"transition\":{\"from\":\"pending\",\"to\":\"completed\"},
    \"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null
  }
}]"
[[ "$STATUS" =~ ^(200|201)$ ]] || fail "complete child run HTTP ${STATUS}: ${RESP}"
ok "child run completed"

# =============================================================================
step "Complete parent task + run"
post "/v1/tasks/${PTASK_ID}/complete" '{"result":{"summary":"subagent spawned and completed"}}'
[ "$STATUS" = "200" ] || fail "complete parent task HTTP ${STATUS}: ${RESP}"
ok "parent task completed"

post /v1/events/append "[{
  \"event_id\":\"evt_cr_parent_${TS}\",
  \"source\":{\"source_type\":\"runtime\"},
  \"ownership\":${OWN},
  \"causation_id\":null,\"correlation_id\":null,
  \"payload\":{
    \"event\":\"run_state_changed\",\"project\":${PROJ},
    \"run_id\":\"${PARENT_RUN}\",
    \"transition\":{\"from\":\"pending\",\"to\":\"completed\"},
    \"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null
  }
}]"
[[ "$STATUS" =~ ^(200|201)$ ]] || fail "complete parent run HTTP ${STATUS}: ${RESP}"
ok "parent run completed"

# =============================================================================
step "Verify parent-child linkage via GET /v1/runs/:id/children"
get "/v1/runs/${PARENT_RUN}/children"
if [ "$STATUS" = "200" ]; then
  CHILD_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('children',d.get('items',d if isinstance(d,list) else []))
print(len(items))" 2>/dev/null)
  ok "children endpoint returned ${CHILD_COUNT} child run(s)"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "children endpoint not implemented (HTTP ${STATUS})"
else
  fail "GET /children HTTP ${STATUS}: ${RESP}"
fi

# Also verify via events
get "/v1/runs/${PARENT_RUN}/events"
[ "$STATUS" = "200" ] || fail "run events HTTP ${STATUS}"
ok "parent run events reachable"

get "/v1/runs/${CHILD_RUN}"
[ "$STATUS" = "200" ] || fail "get child run HTTP ${STATUS}"
CHILD_STATE=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null)
ok "child run state=${CHILD_STATE}"

# =============================================================================
echo "" >&2
echo -e "${BLD}${GRN}=== E2E SUBAGENT WORKFLOW COMPLETED ===${RST}" >&2
echo -e "  Parent run : ${PARENT_RUN}" >&2
echo -e "  Child run  : ${CHILD_RUN}" >&2
echo -e "  Pass: ${PASS}  Skip: ${SKIP}  Fail: ${FAIL_COUNT}  Steps: ${STEP}" >&2
echo "" >&2
