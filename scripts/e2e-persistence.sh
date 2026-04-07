#!/usr/bin/env bash
# =============================================================================
# e2e-persistence.sh — SQLite persistence smoke test.
#
# Proves that a session, run, and task written before a server restart are
# still readable after the server restarts from the SQLite event log.
#
# Workflow:
#   1. Build the server binary (if not already built)
#   2. Start cairn-app with --db sqlite:<tmpfile>
#   3. Create a session, run, and task via the HTTP API
#   4. Kill the server
#   5. Restart the server against the same SQLite file
#   6. Verify the session, run, and task are still present via GET endpoints
#   7. Clean up (temp db, server process)
#
# Usage:
#   ./scripts/e2e-persistence.sh           # uses release build from PATH or builds
#   CAIRN_BIN=./target/debug/cairn-app ./scripts/e2e-persistence.sh
#   CAIRN_PORT=19090 ./scripts/e2e-persistence.sh
#
# Exit code: 0 = all checks passed, 1 = one or more failures.
# =============================================================================

set -euo pipefail

# ── Configuration ─────────────────────────────────────────────────────────────
PORT="${CAIRN_PORT:-19099}"
TOKEN="${CAIRN_TOKEN:-dev-admin-token}"
BASE="http://127.0.0.1:${PORT}"
TIMEOUT="${CAIRN_TIMEOUT:-10}"

# Locate binary: env override → release → debug build
if [ -n "${CAIRN_BIN:-}" ]; then
    SERVER_BIN="$CAIRN_BIN"
elif [ -x "./target/release/cairn-app" ]; then
    SERVER_BIN="./target/release/cairn-app"
elif [ -x "./target/debug/cairn-app" ]; then
    SERVER_BIN="./target/debug/cairn-app"
else
    echo "No cairn-app binary found. Building debug binary..." >&2
    cargo build -p cairn-app --bin cairn-app 2>&1 | tail -3 >&2
    SERVER_BIN="./target/debug/cairn-app"
fi

# ── Colours ───────────────────────────────────────────────────────────────────
if [ -t 2 ]; then
    GRN='\033[0;32m'; RED='\033[0;31m'; YLW='\033[0;33m'
    CYN='\033[0;36m'; BLD='\033[1m';   RST='\033[0m'
else
    GRN=''; RED=''; YLW=''; CYN=''; BLD=''; RST=''
fi

PASS=0; FAIL=0
log_ok()   { echo -e "${GRN}  ✓${RST} $1" >&2; PASS=$(( PASS + 1 )); }
log_fail() { echo -e "${RED}  ✗${RST} $1" >&2; FAIL=$(( FAIL + 1 )); }
section()  { echo -e "\n${BLD}${CYN}── $1${RST}" >&2; }

# ── HTTP helpers ──────────────────────────────────────────────────────────────
_BODY_FILE=$(mktemp)
trap 'rm -f "$_BODY_FILE"' EXIT

_HTTP="" _BODY=""
api() {
    local method="$1" path="$2" body="${3:-}"
    local curl_args=(-s -X "$method" --max-time "$TIMEOUT"
        -H "Authorization: Bearer ${TOKEN}"
        -H "Content-Type: application/json"
        -o "$_BODY_FILE"
        -w "%{http_code}")
    [ -n "$body" ] && curl_args+=(-d "$body")
    _HTTP=$(curl "${curl_args[@]}" "${BASE}${path}" 2>/dev/null)
    _BODY=$(cat "$_BODY_FILE")
}

chk() {
    local label="$1" want="$2" method="$3" path="$4" body="${5:-}"
    api "$method" "$path" "$body"
    if [ "$_HTTP" = "$want" ]; then
        log_ok "$label (HTTP $_HTTP)"
    else
        log_fail "$label (expected HTTP $want, got HTTP $_HTTP: ${_BODY:0:120})"
    fi
}

# jf KEY — extract field from top-level or from nested "session"/"run" object
jf() {
    printf '%s' "$_BODY" | python3 -c \
        "import sys,json; d=json.load(sys.stdin)
v=d.get('$1') or d.get('session',{}).get('$1') or d.get('run',{}).get('$1','')
print(v or '')" 2>/dev/null || true
}

# ── Server lifecycle helpers ───────────────────────────────────────────────────
SERVER_PID=""

start_server() {
    local db_path="$1"
    CAIRN_ADMIN_TOKEN="$TOKEN" "$SERVER_BIN" \
        --db "sqlite:${db_path}" \
        --port "$PORT" \
        > /tmp/cairn-persist-server.log 2>&1 &
    SERVER_PID=$!
    echo -e "  ${CYN}PID ${SERVER_PID}${RST} — waiting for server to accept connections…" >&2

    local attempts=0
    while [ $attempts -lt 30 ]; do
        sleep 0.5
        attempts=$(( attempts + 1 ))
        if curl -s --max-time 1 \
                -H "Authorization: Bearer ${TOKEN}" \
                "${BASE}/v1/stats" > /dev/null 2>&1; then
            echo -e "  ${GRN}Server ready after ${attempts} attempts${RST}" >&2
            return 0
        fi
    done

    echo -e "  ${RED}Server did not become ready in time${RST}" >&2
    echo "  Last server log:" >&2
    tail -10 /tmp/cairn-persist-server.log >&2
    return 1
}

stop_server() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null
        wait "$SERVER_PID" 2>/dev/null || true
        SERVER_PID=""
        sleep 0.3
    fi
}

# Ensure server is stopped on script exit (success or error)
cleanup() {
    stop_server
    rm -f "$DB_PATH" "${DB_PATH}-shm" "${DB_PATH}-wal" 2>/dev/null || true
}
trap cleanup EXIT

# ── Unique IDs for this test run ──────────────────────────────────────────────
TS=$(date +%s)
SESSION_ID="persist_sess_${TS}"
RUN_ID="persist_run_${TS}"
TASK_ID="persist_task_${TS}"

# Tenant/workspace/project that exist after the default tenant is seeded
TENANT="default"
WORKSPACE="default"
PROJECT="default"

# ── Main ──────────────────────────────────────────────────────────────────────
echo -e "\n${BLD}cairn-app persistence smoke test${RST}" >&2
echo -e "  Binary  : ${CYN}${SERVER_BIN}${RST}" >&2
echo -e "  Port    : ${CYN}${PORT}${RST}" >&2

# Create a temp path for the SQLite DB.
# Touch an empty file so the server can open it with its default connection string
# (some sqlx builds require the file to pre-exist for the plain sqlite:path URL).
DB_PATH=$(mktemp --suffix=.db)
echo -e "  DB      : ${CYN}${DB_PATH}${RST}" >&2

# ── Phase 1: First boot — write data ─────────────────────────────────────────
section "Phase 1 — first boot, write data"

if ! start_server "$DB_PATH"; then
    echo -e "${RED}FATAL: server failed to start — aborting.${RST}" >&2
    exit 1
fi

# Create session
chk "POST /v1/sessions" 201 POST /v1/sessions \
    "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION_ID}\"}"
[ "$(jf state)" = "open" ] \
    && log_ok "  session.state = open" \
    || log_fail "  session.state = '$(jf state)' (expected open)"

# Create run under the session
chk "POST /v1/runs" 201 POST /v1/runs \
    "{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\",\"session_id\":\"${SESSION_ID}\",\"run_id\":\"${RUN_ID}\"}"
[ "$(jf state)" = "pending" ] \
    && log_ok "  run.state = pending" \
    || log_fail "  run.state = '$(jf state)' (expected pending)"

# Create task via event append (same pattern as smoke-test.sh)
OWNERSHIP="{\"scope\":\"project\",\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"
PROJECT_OBJ="{\"tenant_id\":\"${TENANT}\",\"workspace_id\":\"${WORKSPACE}\",\"project_id\":\"${PROJECT}\"}"
SOURCE="{\"source_type\":\"runtime\"}"

chk "POST /v1/events/append (TaskCreated)" 201 POST /v1/events/append \
    "[{\"event_id\":\"evt_persist_${TS}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"task_created\",\"project\":${PROJECT_OBJ},\"task_id\":\"${TASK_ID}\",\"parent_run_id\":\"${RUN_ID}\",\"parent_task_id\":null,\"prompt_release_id\":null}}]"
sleep 0.3   # let the async projection settle

# Quick pre-restart sanity: data is readable from first boot
chk "GET /v1/sessions (pre-restart)" 200 GET \
    "/v1/sessions?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PROJECT}"
echo "$_BODY" | grep -q "$SESSION_ID" \
    && log_ok "  session in list (pre-restart)" \
    || log_fail "  session missing from list (pre-restart)"

chk "GET /v1/runs (pre-restart)" 200 GET \
    "/v1/runs?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PROJECT}"
echo "$_BODY" | grep -q "$RUN_ID" \
    && log_ok "  run in list (pre-restart)" \
    || log_fail "  run missing from list (pre-restart)"

# Record event count before restart for comparison
api GET /v1/stats
PRE_RESTART_EVENTS=$(printf '%s' "$_BODY" | python3 -c \
    "import sys,json; print(json.load(sys.stdin).get('total_events',0))" 2>/dev/null || echo 0)
echo -e "  ${CYN}Pre-restart event count: ${PRE_RESTART_EVENTS}${RST}" >&2

# ── Phase 2: Restart ──────────────────────────────────────────────────────────
section "Phase 2 — server restart"

stop_server
echo -e "  ${YLW}Server stopped — DB at ${DB_PATH}${RST}" >&2

if ! start_server "$DB_PATH"; then
    echo -e "${RED}FATAL: server failed to restart — aborting.${RST}" >&2
    exit 1
fi
log_ok "Server restarted successfully"

# ── Phase 3: Post-restart verification ────────────────────────────────────────
section "Phase 3 — verify persistence after restart"

# Event count must be at least as large as before (replay from SQLite fills the in-memory store)
api GET /v1/stats
POST_RESTART_EVENTS=$(printf '%s' "$_BODY" | python3 -c \
    "import sys,json; print(json.load(sys.stdin).get('total_events',0))" 2>/dev/null || echo 0)
echo -e "  Post-restart event count: ${POST_RESTART_EVENTS}" >&2

if [ "${POST_RESTART_EVENTS}" -ge "${PRE_RESTART_EVENTS}" ] 2>/dev/null; then
    log_ok "Event count restored (${POST_RESTART_EVENTS} ≥ pre-restart ${PRE_RESTART_EVENTS})"
else
    log_fail "Event count dropped: ${POST_RESTART_EVENTS} < pre-restart ${PRE_RESTART_EVENTS}"
fi

# Session must still be readable
chk "GET /v1/sessions/:id (post-restart)" 200 GET "/v1/sessions/${SESSION_ID}"
[ "$(jf state)" = "open" ] \
    && log_ok "  session.state still = open" \
    || log_fail "  session.state = '$(jf state)' after restart (expected open)"

# Session must still appear in project listing
chk "GET /v1/sessions (post-restart)" 200 GET \
    "/v1/sessions?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PROJECT}"
echo "$_BODY" | grep -q "$SESSION_ID" \
    && log_ok "  session still in list after restart" \
    || log_fail "  session missing from list after restart"

# Run must still be readable
chk "GET /v1/runs/:id (post-restart)" 200 GET "/v1/runs/${RUN_ID}"
[ "$(jf state)" = "pending" ] \
    && log_ok "  run.state still = pending" \
    || log_fail "  run.state = '$(jf state)' after restart (expected pending)"

# Run must still appear in project listing
chk "GET /v1/runs (post-restart)" 200 GET \
    "/v1/runs?tenant_id=${TENANT}&workspace_id=${WORKSPACE}&project_id=${PROJECT}"
echo "$_BODY" | grep -q "$RUN_ID" \
    && log_ok "  run still in list after restart" \
    || log_fail "  run missing from list after restart"

# Task must appear in the run's task list
chk "GET /v1/runs/:id/tasks (post-restart)" 200 GET "/v1/runs/${RUN_ID}/tasks"
echo "$_BODY" | grep -q "$TASK_ID" \
    && log_ok "  task still present after restart" \
    || log_fail "  task missing from run tasks after restart"

# ── Summary ───────────────────────────────────────────────────────────────────
TOTAL=$(( PASS + FAIL ))
echo "" >&2
echo -e "${BLD}── Results $(printf '─%.0s' {1..36})${RST}" >&2
printf "  ${GRN}Passed${RST}   %3d\n" "$PASS" >&2
printf "  ${RED}Failed${RST}   %3d\n" "$FAIL" >&2
printf "  Total    %3d\n"            "$TOTAL" >&2
echo "" >&2

if [ "$FAIL" -eq 0 ]; then
    echo -e "${GRN}${BLD}Persistence smoke test passed.${RST}" >&2
    exit 0
else
    echo -e "${RED}${BLD}${FAIL} persistence check(s) failed.${RST}" >&2
    echo -e "  Server log: /tmp/cairn-persist-server.log" >&2
    exit 1
fi
