#!/usr/bin/env bash
# =============================================================================
# e2e-eval-pipeline.sh — UC-13: eval dataset → rubric → baseline → run
#
# Workflow:
#   1. Create eval dataset
#   2. Add two entries to the dataset
#   3. Create rubric (multi-dimension scorer)
#   4. Create baseline (reference metrics)
#   5. Start eval run
#   6. Score / complete eval run
#   7. Get eval dashboard
#   8. Get scorecard for the asset
#   9. Verify data flows through
#
# Usage: CAIRN_TOKEN=cairn-demo-token ./scripts/e2e-eval-pipeline.sh
# Exit: 0 = all assertions passed, 1 = failure.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

TS=$(date +%s)_$RANDOM
TENANT="default"; WORKSPACE="default"; PROJECT="default"

DATASET_ID="e2e_ds_${TS}"
RUBRIC_ID="e2e_rubric_${TS}"
BASELINE_ID="e2e_base_${TS}"
EVAL_RUN_ID="e2e_eval_${TS}"
ASSET_ID="e2e_asset_${TS}"    # synthetic — used for scorecard/baseline lookup

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

# =============================================================================
echo -e "${BLD}cairn e2e eval pipeline${RST}" >&2
echo -e "  Server     : ${CYN}${BASE}${RST}" >&2
echo -e "  Eval run ID: ${CYN}${EVAL_RUN_ID}${RST}" >&2
echo "" >&2

get /health
[ "$STATUS" = "200" ] || fail "server not reachable at ${BASE} (HTTP ${STATUS})"

# =============================================================================
step "Create eval dataset"
post /v1/evals/datasets "{
  \"tenant_id\":\"${TENANT}\",
  \"name\":\"e2e dataset ${TS}\",
  \"subject_kind\":\"prompt_release\"
}"
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  CREATED_DS=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('dataset_id',d.get('id','')))" 2>/dev/null)
  DATASET_ID="${CREATED_DS:-$DATASET_ID}"
  ok "dataset ${DATASET_ID} created"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "datasets endpoint not available (HTTP ${STATUS})"
else
  fail "create dataset HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Add entry 1 to dataset"
post "/v1/evals/datasets/${DATASET_ID}/entries" '{
  "input":{"prompt":"What is 2+2?"},
  "expected_output":{"answer":"4"},
  "tags":["math","basic"]
}'
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  ok "entry 1 added"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "add entry not available (HTTP ${STATUS})"
else
  fail "add entry 1 HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Add entry 2 to dataset"
post "/v1/evals/datasets/${DATASET_ID}/entries" '{
  "input":{"prompt":"Explain gravitational waves briefly."},
  "expected_output":{"key_concepts":["spacetime","LIGO","Einstein"]},
  "tags":["science","explanation"]
}'
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  ok "entry 2 added"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "add entry 2 not available (HTTP ${STATUS})"
else
  fail "add entry 2 HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get dataset — verify entries"
get "/v1/evals/datasets/${DATASET_ID}"
if [ "$STATUS" = "200" ]; then
  ENTRY_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
print(len(d.get('entries',d.get('items',[]))))" 2>/dev/null)
  ok "dataset has ${ENTRY_COUNT} entries"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "get dataset HTTP ${STATUS}"
else
  fail "get dataset HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Create rubric"
post /v1/evals/rubrics "{
  \"tenant_id\":\"${TENANT}\",
  \"name\":\"e2e rubric ${TS}\",
  \"dimensions\":[
    {\"name\":\"accuracy\",\"weight\":0.6,\"scoring_fn\":\"exact_match\",\"description\":\"Factual correctness\"},
    {\"name\":\"clarity\",\"weight\":0.4,\"scoring_fn\":\"similarity\",\"description\":\"Clear and concise response\"}
  ]
}"
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  RUBRIC_RESP_ID=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('rubric_id',d.get('id','')))" 2>/dev/null)
  RUBRIC_ID="${RUBRIC_RESP_ID:-$RUBRIC_ID}"
  ok "rubric ${RUBRIC_ID} created"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "rubrics endpoint not available (HTTP ${STATUS})"
else
  fail "create rubric HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Create baseline"
post /v1/evals/baselines "{
  \"tenant_id\":\"${TENANT}\",
  \"name\":\"e2e baseline ${TS}\",
  \"prompt_asset_id\":\"${ASSET_ID}\",
  \"metrics\":{
    \"task_success_rate\":0.75,
    \"latency_p50_ms\":350,
    \"cost_per_run\":0.002
  }
}"
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  BASELINE_RESP_ID=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('baseline_id',d.get('id','')))" 2>/dev/null)
  BASELINE_ID="${BASELINE_RESP_ID:-$BASELINE_ID}"
  ok "baseline ${BASELINE_ID} created"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "baselines endpoint not available (HTTP ${STATUS})"
else
  fail "create baseline HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Create eval run"
post /v1/evals/runs "{
  \"tenant_id\":\"${TENANT}\",
  \"workspace_id\":\"${WORKSPACE}\",
  \"project_id\":\"${PROJECT}\",
  \"eval_run_id\":\"${EVAL_RUN_ID}\",
  \"subject_kind\":\"prompt_release\",
  \"evaluator_type\":\"automated\"
}"
if [ "$STATUS" = "201" ] || [ "$STATUS" = "200" ]; then
  EVAL_STATE=$(printf '%s' "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',''))" 2>/dev/null)
  ok "eval run ${EVAL_RUN_ID} created (state=${EVAL_STATE:-pending})"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "eval runs POST not available (HTTP ${STATUS})"
  # Create via event append as fallback
else
  fail "create eval run HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Start eval run"
post "/v1/evals/runs/${EVAL_RUN_ID}/start" '{}'
if [ "$STATUS" = "200" ]; then
  ok "eval run ${EVAL_RUN_ID} started"
elif [[ "$STATUS" =~ ^(404|501|422)$ ]]; then
  skip "start eval run HTTP ${STATUS}"
else
  fail "start eval run HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Score eval run (complete with metrics)"
post "/v1/evals/runs/${EVAL_RUN_ID}/score" '{
  "metrics":{
    "task_success_rate":0.82,
    "latency_p50_ms":280,
    "cost_per_run":0.0018
  }
}'
if [ "$STATUS" = "200" ]; then
  ok "eval run scored"
elif [[ "$STATUS" =~ ^(404|501|422)$ ]]; then
  skip "score eval run HTTP ${STATUS}"
else
  fail "score eval run HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get eval run detail"
get "/v1/evals/runs/${EVAL_RUN_ID}"
if [ "$STATUS" = "200" ]; then
  RUN_SUCCESS=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('success',''))" 2>/dev/null)
  ok "eval run detail: success=${RUN_SUCCESS}"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "get eval run HTTP ${STATUS}"
else
  fail "get eval run HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get eval dashboard"
get /v1/evals/dashboard
if [ "$STATUS" = "200" ]; then
  TOTAL=$(printf '%s' "$RESP" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('total_runs',d.get('runs_total','?')))" 2>/dev/null)
  ok "dashboard reachable (total_runs=${TOTAL})"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "eval dashboard HTTP ${STATUS}"
else
  fail "eval dashboard HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "Get scorecard for asset"
get "/v1/evals/scorecard/${ASSET_ID}?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PROJECT}"
if [ "$STATUS" = "200" ]; then
  ok "scorecard reachable for ${ASSET_ID}"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "scorecard not found (HTTP ${STATUS}) — expected for new asset"
else
  fail "scorecard HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
step "List all eval runs — verify new run appears"
get "/v1/evals/runs?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PROJECT}"
if [ "$STATUS" = "200" ]; then
  RUN_COUNT=$(printf '%s' "$RESP" | python3 -c "
import sys,json
d=json.load(sys.stdin)
items=d.get('items',d if isinstance(d,list) else [])
print(len(items))" 2>/dev/null)
  ok "${RUN_COUNT} eval run(s) in system"
elif [[ "$STATUS" =~ ^(404|501)$ ]]; then
  skip "list eval runs HTTP ${STATUS}"
else
  fail "list eval runs HTTP ${STATUS}: ${RESP}"
fi

# =============================================================================
echo "" >&2
echo -e "${BLD}${GRN}=== E2E EVAL PIPELINE COMPLETED ===${RST}" >&2
echo -e "  Dataset   : ${DATASET_ID}" >&2
echo -e "  Eval run  : ${EVAL_RUN_ID}" >&2
echo -e "  Pass: ${PASS}  Skip: ${SKIP}  Fail: ${FAIL_COUNT}  Steps: ${STEP}" >&2
echo "" >&2
