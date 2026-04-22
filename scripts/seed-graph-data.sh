#!/usr/bin/env bash
# =============================================================================
# seed-graph-data.sh — populate a multi-step workflow so the graph view
# has real nodes and edges to render.
#
# Creates: 1 session → 2 runs → 4 tasks → 2 approvals → memory docs → tool invocations
# This gives the graph view ~15 nodes and ~20 edges to visualize.
#
# Usage:
#   CAIRN_TOKEN=dev-admin-token ./scripts/seed-graph-data.sh
# =============================================================================

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-dev-admin-token}"
SUFFIX="graph_$(date +%s)"

GRN='\033[0;32m'; RED='\033[0;31m'; CYN='\033[0;36m'; BLD='\033[1m'; RST='\033[0m'

api() {
  local method="$1" path="$2" body="${3:-}"
  local args=(-s -X "$method" --max-time 10
    -H "Authorization: Bearer ${TOKEN}"
    -H "Content-Type: application/json")
  [ -n "$body" ] && args+=(-d "$body")
  local http_code
  http_code=$(curl "${args[@]}" -o /dev/null -w "%{http_code}" "${BASE}${path}" 2>/dev/null)
  echo -e "  ${GRN}✓${RST} $method $path → $http_code"
}

PROJECT='{"tenant_id":"default","workspace_id":"default","project_id":"default"}'
SCOPE="tenant_id=default&workspace_id=default&project_id=default"
OWN='{"scope":"project","tenant_id":"default","workspace_id":"default","project_id":"default"}'
SRC='{"source_type":"runtime"}'

echo -e "${BLD}Seeding graph data (${CYN}${SUFFIX}${RST}${BLD})${RST}\n"

# ── Session ──────────────────────────────────────────────────────────────────
echo -e "${BLD}Session + Runs:${RST}"
SESS="sess_${SUFFIX}"
RUN1="run_main_${SUFFIX}"
RUN2="run_sub_${SUFFIX}"

api POST /v1/sessions "{$( echo "$PROJECT" | tr -d '{}'),\"session_id\":\"${SESS}\"}"
api POST /v1/runs     "{$( echo "$PROJECT" | tr -d '{}'),\"session_id\":\"${SESS}\",\"run_id\":\"${RUN1}\"}"
api POST /v1/runs     "{$( echo "$PROJECT" | tr -d '{}'),\"session_id\":\"${SESS}\",\"run_id\":\"${RUN2}\",\"parent_run_id\":\"${RUN1}\"}"

# ── Tasks ────────────────────────────────────────────────────────────────────
echo -e "\n${BLD}Tasks:${RST}"
for i in 1 2 3 4; do
  TASK="task_${i}_${SUFFIX}"
  PARENT=$( [ "$i" -le 2 ] && echo "$RUN1" || echo "$RUN2" )
  api POST /v1/events/append \
    "[{\"event_id\":\"evt_task_${i}_${SUFFIX}\",\"source\":${SRC},\"ownership\":${OWN},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"task_created\",\"project\":${PROJECT},\"task_id\":\"${TASK}\",\"parent_run_id\":\"${PARENT}\",\"parent_task_id\":null,\"prompt_release_id\":null}}]"
done

sleep 0.3

# ── Approvals ────────────────────────────────────────────────────────────────
echo -e "\n${BLD}Approvals:${RST}"
for i in 1 2; do
  APPR="appr_${i}_${SUFFIX}"
  RUN=$( [ "$i" -eq 1 ] && echo "$RUN1" || echo "$RUN2" )
  api POST /v1/events/append \
    "[{\"event_id\":\"evt_appr_${i}_${SUFFIX}\",\"source\":${SRC},\"ownership\":${OWN},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"approval_requested\",\"project\":${PROJECT},\"approval_id\":\"${APPR}\",\"run_id\":\"${RUN}\",\"task_id\":null,\"requirement\":\"required\"}}]"
done

sleep 0.3

# Resolve first approval
api POST "/v1/approvals/appr_1_${SUFFIX}/resolve" '{"decision":"approved","reason":"graph seed"}'

# ── Memory documents (creates graph nodes via ingest) ────────────────────────
echo -e "\n${BLD}Memory documents:${RST}"
api POST /v1/memory/ingest \
  "{\"source_id\":\"graph_docs\",\"document_id\":\"gdoc1_${SUFFIX}\",\"content\":\"Design document for the agent orchestration loop. Covers gather, decide, execute phases.\",${SCOPE/&/\",\"}\"}"

api POST /v1/memory/ingest \
  "{\"source_id\":\"graph_docs\",\"document_id\":\"gdoc2_${SUFFIX}\",\"content\":\"Architecture decision record: chose event sourcing over CRUD for audit trail and replay guarantees.\",${SCOPE/&/\",\"}\"}"

# ── Tool invocations ─────────────────────────────────────────────────────────
echo -e "\n${BLD}Tool invocations:${RST}"
for tool in memory_search web_fetch bash; do
  api POST /v1/events/append \
    "[{\"event_id\":\"evt_tool_${tool}_${SUFFIX}\",\"source\":${SRC},\"ownership\":${OWN},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"tool_invocation_started\",\"project\":${PROJECT},\"invocation_id\":\"inv_${tool}_${SUFFIX}\",\"tool_name\":\"${tool}\",\"run_id\":\"${RUN1}\",\"task_id\":\"task_1_${SUFFIX}\",\"started_at_ms\":$(date +%s000)}}]"
done

sleep 0.3

# ── Verify graph endpoints ───────────────────────────────────────────────────
echo -e "\n${BLD}Graph endpoint verification:${RST}"
api GET "/v1/graph/nodes?${SCOPE}&limit=50"
api GET "/v1/graph/edges?${SCOPE}&limit=50"
api GET "/v1/graph/execution-trace/${RUN1}"
api GET "/v1/graph/dependency-path/${RUN1}"
api GET "/v1/graph/retrieval-provenance/${RUN1}"

echo -e "\n${BLD}${GRN}Graph data seeded.${RST}"
echo -e "  Session: ${CYN}${SESS}${RST}"
echo -e "  Runs:    ${CYN}${RUN1}${RST}, ${CYN}${RUN2}${RST}"
echo -e "  Open ${CYN}${BASE}${RST} → Graph page to see the visualization."
