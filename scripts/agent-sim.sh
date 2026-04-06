#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# agent-sim.sh — Cairn-rs full lifecycle simulation
#
# Simulates what a real AI agent does against the running cairn-rs server:
#   1. Create a session          (agent registers)
#   2. Start a run               (agent begins work)
#   3. Decompose into 3 tasks    (via POST /v1/events/append)
#   4. Execute each task         (claim → work → release-lease)
#   5. Make LLM calls via Ollama (agent thinks)
#   6. Request + approve an approval
#   7. Complete the run via event
#   8. Print final status report
#
# Usage:
#   ./scripts/agent-sim.sh [BASE_URL] [TOKEN]
#
#   BASE_URL  default: http://localhost:3000
#   TOKEN     default: cairn-demo-token
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

BASE="${1:-http://localhost:3000}"
TOKEN="${2:-cairn-demo-token}"

# ── Colour palette ────────────────────────────────────────────────────────────
R=$'\e[0;31m'; G=$'\e[0;32m'; Y=$'\e[0;33m'; B=$'\e[0;34m'
M=$'\e[0;35m'; C=$'\e[0;36m'; W=$'\e[1;37m'; DIM=$'\e[2m'; X=$'\e[0m'

STEP=0
T0=$(date +%s%3N)

# ── Helpers ───────────────────────────────────────────────────────────────────

step() {
    STEP=$((STEP + 1))
    local elapsed=$(( $(date +%s%3N) - T0 ))
    printf "\n${W}[%02d]${X} ${C}%s${X} ${DIM}(+%dms)${X}\n" "$STEP" "$1" "$elapsed"
}

ok()   { printf "    ${G}✓${X}  %s\n" "$1"; }
info() { printf "    ${DIM}→  %s${X}\n" "$1" >&2; }   # stderr so it doesn't pollute captured output
warn() { printf "    ${Y}⚠${X}  %s\n" "$1"; }
fail() { printf "    ${R}✗${X}  %s\n" "$1" >&2; }

# Execute an API call; print method+path; echo the body on stdout.
# Exits the script if HTTP 4xx/5xx.
api() {
    local method="$1" path="$2" body="${3:-}"
    local url="${BASE}${path}"

    local resp http_code body_part
    resp=$(curl -sS -w '\n__HTTP__%{http_code}' -X "$method" "$url" \
        -H "Authorization: Bearer ${TOKEN}" \
        -H "Content-Type: application/json" \
        ${body:+-d "$body"} 2>&1)

    http_code="${resp##*$'\n'__HTTP__}"
    body_part="${resp%$'\n'__HTTP__*}"

    if [[ "$http_code" == "000" || "$http_code" -ge 400 ]]; then
        fail "HTTP ${http_code}  ${method} ${path}"
        printf "    ${R}%s${X}\n" "$body_part" >&2
        [[ "$http_code" == "000" ]] && printf "    ${R}Connection refused — is the server running at %s?${X}\n" "$BASE" >&2
        exit 1
    fi
    info "HTTP ${http_code}  ${method} ${path}"
    printf '%s' "$body_part"
}

# Extract a jq field from a JSON string.
jfield() { printf '%s' "$1" | jq -r "$2" 2>/dev/null || printf '—'; }

# ── Preflight ─────────────────────────────────────────────────────────────────
printf "\n${W}╔══════════════════════════════════════════════╗${X}\n"
printf "${W}║${X}   ${M}cairn-rs agent simulation${X}                 ${W}║${X}\n"
printf "${W}╚══════════════════════════════════════════════╝${X}\n"
printf "${DIM}target : %s${X}\n" "$BASE"
printf "${DIM}token  : %s…%s${X}\n" "${TOKEN:0:8}" "${TOKEN: -4}"

command -v jq >/dev/null 2>&1 || { warn "jq not installed — field extraction will show '—'"; }

# ── Generate unique IDs so repeated runs don't collide ───────────────────────
TS=$(date +%s)
SIM_ID="sim_${TS}"
SESS_ID="${SIM_ID}_sess"
RUN_ID="${SIM_ID}_run"
TASK_IDS=( "${SIM_ID}_t1" "${SIM_ID}_t2" "${SIM_ID}_t3" )
APPR_ID="${SIM_ID}_appr"

# Ownership block reused across event envelopes.
OWNERSHIP='"ownership":{"scope":"project","tenant_id":"default_tenant","workspace_id":"default_workspace","project_id":"demo_project"}'
PROJECT='{"tenant_id":"default_tenant","workspace_id":"default_workspace","project_id":"demo_project"}'

printf "${DIM}sim-id : %s${X}\n" "$SIM_ID"

# ─────────────────────────────────────────────────────────────────────────────
# 1 — Health check
# ─────────────────────────────────────────────────────────────────────────────
step "Health check"
health=$(api GET /health)
ok "Server healthy (ok=$(jfield "$health" '.ok'))"

# ─────────────────────────────────────────────────────────────────────────────
# 2 — Create session (agent registers)
# ─────────────────────────────────────────────────────────────────────────────
step "Create session  →  agent registers with cairn"

sess_resp=$(api POST /v1/sessions \
    '{"tenant_id":"default_tenant","workspace_id":"default_workspace",
      "project_id":"demo_project","session_id":"'"$SESS_ID"'"}')
ok "Session ${SESS_ID}  state=$(jfield "$sess_resp" '.state')"

# ─────────────────────────────────────────────────────────────────────────────
# 3 — Start run (agent begins executing)
# ─────────────────────────────────────────────────────────────────────────────
step "Start run  →  agent begins execution"

run_resp=$(api POST /v1/runs \
    '{"tenant_id":"default_tenant","workspace_id":"default_workspace",
      "project_id":"demo_project","session_id":"'"$SESS_ID"'",
      "run_id":"'"$RUN_ID"'"}')
ok "Run ${RUN_ID}  state=$(jfield "$run_resp" '.state')"

# ─────────────────────────────────────────────────────────────────────────────
# 4 — Task decomposition: post 3 TaskCreated events
# ─────────────────────────────────────────────────────────────────────────────
step "Task decomposition  →  break work into 3 tasks"

task_names=("gather_context" "analyse_data" "synthesise_output")

for i in 0 1 2; do
    tid="${TASK_IDS[$i]}"
    tname="${task_names[$i]}"
    eid="${SIM_ID}_evt_task_${i}"

    result=$(api POST /v1/events/append \
        '[{"event_id":"'"$eid"'","source":{"source_type":"runtime"},'"$OWNERSHIP"',
           "causation_id":null,"correlation_id":null,
           "payload":{"event":"task_created","project":'"$PROJECT"',
                      "task_id":"'"$tid"'","parent_run_id":"'"$RUN_ID"'",
                      "parent_task_id":null,"prompt_release_id":null}}]')
    pos=$(jfield "$result" '.[0].position')
    ok "Task $((i+1))/3 created: ${tid} (${tname})  event_pos=${pos}"
done

sleep 0.3   # let projections settle

# ─────────────────────────────────────────────────────────────────────────────
# 5 — Execute each task: claim → work → release-lease
# ─────────────────────────────────────────────────────────────────────────────
step "Task execution  →  claim, work, release each task"

WORKER_ID="agent-worker-${SIM_ID}"

for i in 0 1 2; do
    tid="${TASK_IDS[$i]}"
    tname="${task_names[$i]}"
    printf "\n    ${B}task %d/3${X}  %s  (%s)\n" "$((i+1))" "$tid" "$tname"

    claim_resp=$(api POST "/v1/tasks/${tid}/claim" \
        '{"worker_id":"'"$WORKER_ID"'","lease_duration_ms":60000}')
    ok "Claimed   state=$(jfield "$claim_resp" '.state')"

    # Simulate the agent doing real work.
    printf "    ${DIM}   working"
    for _ in 1 2; do sleep 1; printf "."; done
    printf "  done${X}\n"

    rel_resp=$(api POST "/v1/tasks/${tid}/release-lease" '')
    ok "Released  state=$(jfield "$rel_resp" '.state')"
done

# ─────────────────────────────────────────────────────────────────────────────
# 6 — LLM calls via Ollama (agent thinks)
# ─────────────────────────────────────────────────────────────────────────────
step "LLM calls  →  agent reasons via Ollama"

mlist_raw=$(curl -sS -w '\n__HTTP__%{http_code}' \
    -X GET "${BASE}/v1/providers/ollama/models" \
    -H "Authorization: Bearer ${TOKEN}" 2>/dev/null) || mlist_raw="__HTTP__000"

models_http="${mlist_raw##*$'\n'__HTTP__}"
mlist_body="${mlist_raw%$'\n'__HTTP__*}"

if [[ "$models_http" == "200" ]]; then
    # Response shape: {"count":N,"host":"...","models":["name1","name2"]}
    mcount=$(printf '%s' "$mlist_body" | jq '.count // (.models | length) // 0' 2>/dev/null || echo 0)
    # Skip embedding-only models; prefer a text generation model.
    model=$(printf '%s' "$mlist_body" | jq -r \
        '(.models // []) | map(select(test("embed|nomic";"i") | not)) | first // .models[0] // "llama3"' \
        2>/dev/null || echo "llama3")

    if [[ "$mcount" -gt 0 && "$model" != "null" && "$model" != "llama3" ]] || \
       printf '%s' "$mlist_body" | jq -e '.models | length > 0' &>/dev/null; then

        ok "Ollama ready  ${mcount} model(s)  using: ${model}"

        prompts=(
            "In one sentence, what is a cairn? Be brief."
            "Name one benefit of event-sourced architecture. One sentence only."
            "What does an AI agent need for long-running task management? One sentence."
        )
        for pi in 0 1 2; do
            printf "\n    ${B}llm %d/3${X}  \"%s\"\n" "$((pi+1))" "${prompts[$pi]}"
            t0_llm=$(date +%s%3N)
            # Hard 20s timeout so a slow model doesn't block the rest of the sim.
            llm_raw=$(curl -sS --max-time 20 -w '\n__HTTP__%{http_code}' \
                -X POST "${BASE}/v1/providers/ollama/generate" \
                -H "Authorization: Bearer ${TOKEN}" \
                -H "Content-Type: application/json" \
                -d '{"model":"'"$model"'","prompt":"'"${prompts[$pi]}"'"}' 2>/dev/null) || llm_raw=$'\n__HTTP__000'
            latency=$(( $(date +%s%3N) - t0_llm ))
            llm_http="${llm_raw##*$'\n'__HTTP__}"
            llm_body="${llm_raw%$'\n'__HTTP__*}"
            if [[ "$llm_http" == "200" ]]; then
                text=$(jfield "$llm_body" '.response // .content // .text // "—"')
                ok "${latency}ms → ${text:0:110}"
            else
                warn "LLM call: HTTP ${llm_http} after ${latency}ms (model may be slow — skipping)"
            fi
        done
    else
        warn "Ollama reachable but no generation models loaded — skipping LLM calls"
        printf "    ${DIM}available: %s${X}\n" "$(jfield "$mlist_body" '.models | join(", ")')"
    fi
else
    warn "Ollama not configured (HTTP ${models_http}) — skipping LLM calls"
    printf "    ${DIM}enable: OLLAMA_HOST=http://localhost:11434 cargo run -p cairn-app ...${X}\n"
fi

# ─────────────────────────────────────────────────────────────────────────────
# 7 — Request approval, then approve it
# ─────────────────────────────────────────────────────────────────────────────
step "Approval gate  →  request then auto-approve"

appr_eid="${SIM_ID}_evt_appr"
appr_result=$(api POST /v1/events/append \
    '[{"event_id":"'"$appr_eid"'","source":{"source_type":"runtime"},'"$OWNERSHIP"',
       "causation_id":null,"correlation_id":null,
       "payload":{"event":"approval_requested","project":'"$PROJECT"',
                  "approval_id":"'"$APPR_ID"'","run_id":"'"$RUN_ID"'",
                  "task_id":null,"requirement":"required"}}]')
ok "Approval ${APPR_ID} requested  pos=$(jfield "$appr_result" '.[0].position')"

sleep 0.3

pending=$(api GET "/v1/approvals/pending")
pcount=$(printf '%s' "$pending" | jq '. | length' 2>/dev/null || echo '?')
ok "Pending queue: ${pcount} approval(s)"

resolve_resp=$(api POST "/v1/approvals/${APPR_ID}/resolve" \
    '{"decision":"approved","reason":"agent-sim auto-approve: all checks passed"}')
ok "Resolved  decision=$(jfield "$resolve_resp" '.decision')"

# ─────────────────────────────────────────────────────────────────────────────
# 8 — Complete the run via RunStateChanged event
# ─────────────────────────────────────────────────────────────────────────────
step "Complete run  →  mark execution finished"

rsc_eid="${SIM_ID}_evt_rsc"
rsc_result=$(api POST /v1/events/append \
    '[{"event_id":"'"$rsc_eid"'","source":{"source_type":"runtime"},'"$OWNERSHIP"',
       "causation_id":null,"correlation_id":null,
       "payload":{"event":"run_state_changed","project":'"$PROJECT"',
                  "run_id":"'"$RUN_ID"'","session_id":"'"$SESS_ID"'",
                  "transition":{"from":"pending","to":"completed",
                               "failure_class":null,"pause_reason":null,
                               "metadata":{}}}}]')
ok "RunStateChanged appended  pos=$(jfield "$rsc_result" '.[0].position')"

sleep 0.3
final_run=$(api GET "/v1/runs/${RUN_ID}")
final_state=$(jfield "$final_run" '.state')
ok "Run ${RUN_ID}  final state: ${final_state}"

# ─────────────────────────────────────────────────────────────────────────────
# 9 — Summary
# ─────────────────────────────────────────────────────────────────────────────
ELAPSED=$(( $(date +%s%3N) - T0 ))

printf "\n"
printf "${W}╔══════════════════════════════════════════════╗${X}\n"
printf "${W}║${X}   ${G}Simulation complete${X}                       ${W}║${X}\n"
printf "${W}╠══════════════════════════════════════════════╣${X}\n"
printf "${W}║${X}  session  : %-33s${W}║${X}\n" "$SESS_ID"
printf "${W}║${X}  run      : %-33s${W}║${X}\n" "$RUN_ID"
printf "${W}║${X}  tasks    : %-33s${W}║${X}\n" "3 × (claim → work → release)"
printf "${W}║${X}  approval : %-33s${W}║${X}\n" "${APPR_ID} → approved"
printf "${W}║${X}  run state: %-33s${W}║${X}\n" "$final_state"
printf "${W}║${X}  total    : %-33s${W}║${X}\n" "${ELAPSED}ms"
printf "${W}╚══════════════════════════════════════════════╝${X}\n"
printf "${DIM}view in UI: %s/#runs${X}\n\n" "$BASE"
