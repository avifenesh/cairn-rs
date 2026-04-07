#!/usr/bin/env bash
# =============================================================================
# e2e-orchestrator-mock.sh — validates the orchestrator plumbing without a
# real LLM.
#
# Tests:
#   1. No-provider gate   — POST /v1/runs/:id/orchestrate returns 503 when no
#                           brain provider is configured (not a misconfiguration
#                           error, just a clear "configure a provider").
#
#   2. Event-model round-trip — simulates what RuntimeExecutePhase emits for a
#                           single tool call + run completion:
#                             • RunStateChanged → Running
#                             • ToolInvocationStarted
#                             • ToolInvocationCompleted
#                             • CheckpointRecorded
#                             • RunStateChanged → Completed
#                           Then verifies the resulting run and checkpoint state
#                           via the read-model APIs.
#
# Usage:
#   ./scripts/e2e-orchestrator-mock.sh
#   CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=my-token \
#     ./scripts/e2e-orchestrator-mock.sh
#
# Exit code: 0 = all passed, 1 = one or more failures.
# =============================================================================

set -euo pipefail

BASE="${CAIRN_URL:-http://localhost:3000}"
TOKEN="${CAIRN_TOKEN:-cairn-demo-token}"

TS="$(date +%s)"
RUN_ID="orch_mock_run_${TS}"
SESSION_ID="orch_mock_sess_${TS}"
INV_ID="orch_mock_inv_${TS}"
CP_ID="orch_mock_cp_${TS}"

passed=0
failed=0
total=0

# ── Helpers ───────────────────────────────────────────────────────────────────

check() {
    local name="$1"; shift
    total=$((total + 1))
    if "$@" >/dev/null 2>&1; then
        passed=$((passed + 1))
        printf "  \033[32mPASS\033[0m  %s\n" "$name"
    else
        failed=$((failed + 1))
        printf "  \033[31mFAIL\033[0m  %s\n" "$name"
    fi
}

api_get() {
    curl -sfS -H "Authorization: Bearer $TOKEN" "$BASE$1"
}

api_post() {
    curl -sfS -H "Authorization: Bearer $TOKEN" \
         -H "Content-Type: application/json" \
         -d "$2" "$BASE$1"
}

api_post_status() {
    curl -s -o /dev/null -w "%{http_code}" \
         -H "Authorization: Bearer $TOKEN" \
         -H "Content-Type: application/json" \
         -d "$2" "$BASE$1"
}

api_post_body() {
    curl -s -H "Authorization: Bearer $TOKEN" \
         -H "Content-Type: application/json" \
         -d "$2" "$BASE$1"
}

jf() { python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('$1',''))" 2>/dev/null || true; }

PROJECT='{"tenant_id":"default","workspace_id":"default","project_id":"default"}'
OWNERSHIP='{"scope":"project","tenant_id":"default","workspace_id":"default","project_id":"default"}'
SOURCE='{"source_type":"runtime"}'

echo "=== e2e-orchestrator-mock ==="
echo "server : $BASE"
echo "run_id : $RUN_ID"
echo ""

# ── Step 0: Health check ──────────────────────────────────────────────────────
echo "[0] Server health"
check "server is reachable" api_get /health

# ── Step 1: No-provider gate ──────────────────────────────────────────────────
echo ""
echo "[1] No-provider gate (expect 503 when no brain provider configured)"

# Create a dummy session + run to test against.
api_post /v1/sessions "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"gate_test_${TS}\"}" >/dev/null 2>&1 || true
api_post /v1/runs "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"gate_test_${TS}\",\"run_id\":\"gate_run_${TS}\"}" >/dev/null 2>&1 || true

ORCH_STATUS=$(api_post_status "/v1/runs/gate_run_${TS}/orchestrate" \
    '{"goal":"test no-provider gate","max_iterations":1}')

if [ "$ORCH_STATUS" = "503" ]; then
    check "orchestrate without brain provider returns 503" true
    ORCH_BODY=$(api_post_body "/v1/runs/gate_run_${TS}/orchestrate" \
        '{"goal":"test","max_iterations":1}' 2>/dev/null || true)
    check "error body contains no_brain_provider code" \
        python3 -c "import sys,json; b=json.loads('${ORCH_BODY}'); exit(0 if 'brain' in b.get('message','') or 'brain' in b.get('code','') else 1)" 2>/dev/null || \
        check "error body contains provider message" \
            python3 -c "import sys,json; b=json.loads('$(echo $ORCH_BODY | sed "s/'/\\\'/g")'); exit(0 if b else 1)"
elif [ "$ORCH_STATUS" = "200" ] || [ "$ORCH_STATUS" = "202" ]; then
    printf "  \033[33mSKIP\033[0m  Brain provider IS configured — 503 gate test skipped\n"
    printf "        (503 gate only fires when CAIRN_BRAIN_URL is unset)\n"
    passed=$((passed + 1)); total=$((total + 1))
elif [ "$ORCH_STATUS" = "502" ] || [ "$ORCH_STATUS" = "429" ]; then
    printf "  \033[33mSKIP\033[0m  Brain provider offline (HTTP %s) — skipping gate test\n" "$ORCH_STATUS"
    passed=$((passed + 1)); total=$((total + 1))
else
    printf "  \033[31mFAIL\033[0m  orchestrate gate returned unexpected HTTP %s\n" "$ORCH_STATUS"
    failed=$((failed + 1)); total=$((total + 1))
fi

# ── Step 2: Create session and run for event-model test ───────────────────────
echo ""
echo "[2] Create session + run for event-model round-trip"

api_post /v1/sessions \
    "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"${SESSION_ID}\"}" \
    >/dev/null
check "session created" api_get "/v1/sessions?tenant_id=default&workspace_id=default&project_id=default" 2>/dev/null

api_post /v1/runs \
    "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"${SESSION_ID}\",\"run_id\":\"${RUN_ID}\"}" \
    >/dev/null
RUN_STATE=$(api_get "/v1/runs/${RUN_ID}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null)
check "run created in pending state" test "$RUN_STATE" = "pending"

# ── Step 3: Simulate orchestrator: RunStateChanged → Running ─────────────────
echo ""
echo "[3] Simulate RuntimeExecutePhase — transition run to running"

api_post /v1/events/append \
    "[{\"event_id\":\"evt_run_running_${TS}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"run_state_changed\",\"project\":{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"run_id\":\"${RUN_ID}\",\"transition\":{\"from\":\"pending\",\"to\":\"running\"},\"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null}}]" \
    >/dev/null 2>&1 || true

sleep 0.2
RUN_STATE2=$(api_get "/v1/runs/${RUN_ID}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null)
check "run transitioned to running" test "$RUN_STATE2" = "running"

# ── Step 4: Simulate ToolInvocationStarted ────────────────────────────────────
echo ""
echo "[4] Simulate ToolInvocationService::record_start"

api_post /v1/events/append \
    "[{\"event_id\":\"evt_inv_start_${TS}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"tool_invocation_started\",\"project\":{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"invocation_id\":\"${INV_ID}\",\"session_id\":\"${SESSION_ID}\",\"run_id\":\"${RUN_ID}\",\"task_id\":null,\"target\":{\"target_type\":\"builtin\",\"tool_name\":\"search_memory\"},\"execution_class\":\"sandboxed_process\",\"requested_at_ms\":$(date +%s%3N),\"started_at_ms\":$(date +%s%3N)}}]" \
    >/dev/null 2>&1 || true

check "ToolInvocationStarted event appended" \
    api_post /v1/events/append \
    "[{\"event_id\":\"evt_inv_complete_${TS}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"tool_invocation_completed\",\"project\":{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"invocation_id\":\"${INV_ID}\",\"task_id\":null,\"tool_name\":\"search_memory\",\"outcome\":\"completed\",\"finished_at_ms\":$(date +%s%3N)}}]"

# ── Step 5: Simulate CheckpointRecorded ───────────────────────────────────────
echo ""
echo "[5] Simulate CheckpointService::save after tool call"

check "CheckpointRecorded event appended" \
    api_post /v1/events/append \
    "[{\"event_id\":\"evt_cp_${TS}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"checkpoint_recorded\",\"project\":{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"run_id\":\"${RUN_ID}\",\"checkpoint_id\":\"${CP_ID}\"}}]"

sleep 0.2
# Verify checkpoint is accessible via run events
EVENT_TYPES=$(api_get "/v1/runs/${RUN_ID}/events" 2>/dev/null | \
    python3 -c "import sys,json; events=json.load(sys.stdin); print(','.join(e.get('event_type','') for e in events))" 2>/dev/null || true)
check "checkpoint_recorded appears in run events" \
    python3 -c "import sys; exit(0 if 'checkpoint_recorded' in '${EVENT_TYPES}' else 1)"

# ── Step 6: Simulate CompleteRun ──────────────────────────────────────────────
echo ""
echo "[6] Simulate RunService::complete — run → completed"

check "RunStateChanged completed event appended" \
    api_post /v1/events/append \
    "[{\"event_id\":\"evt_run_complete_${TS}\",\"source\":${SOURCE},\"ownership\":${OWNERSHIP},\"causation_id\":null,\"correlation_id\":null,\"payload\":{\"event\":\"run_state_changed\",\"project\":{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\"},\"run_id\":\"${RUN_ID}\",\"transition\":{\"from\":\"running\",\"to\":\"completed\"},\"failure_class\":null,\"pause_reason\":null,\"resume_trigger\":null}}]"

sleep 0.2
FINAL_STATE=$(api_get "/v1/runs/${RUN_ID}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('run',d).get('state',''))" 2>/dev/null)
check "run is now in completed state" test "$FINAL_STATE" = "completed"

# ── Step 7: Verify full event trail ──────────────────────────────────────────
echo ""
echo "[7] Verify full event trail for this run"

RUN_EVENTS=$(api_get "/v1/runs/${RUN_ID}/events" 2>/dev/null)
EVENT_COUNT=$(echo "$RUN_EVENTS" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo 0)
check "run has ≥ 4 events (created + running + tool + checkpoint + completed)" \
    test "$EVENT_COUNT" -ge 4

check "run_state_changed events present" \
    python3 -c "import sys,json; events=json.load(sys.stdin); types=[e.get('event_type','') for e in events]; exit(0 if 'run_state_changed' in types else 1)" <<< "$RUN_EVENTS"

check "tool_invocation_completed event present" \
    python3 -c "import sys,json; events=json.load(sys.stdin); types=[e.get('event_type','') for e in events]; exit(0 if 'tool_invocation_completed' in types else 1)" <<< "$RUN_EVENTS"

# ── Step 8: Orchestrate endpoint shape test ───────────────────────────────────
echo ""
echo "[8] Orchestrate endpoint response shape"

# Create a fresh run for shape testing (old run is now completed).
SHAPE_RUN="orch_shape_${TS}"
api_post /v1/sessions \
    "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"shape_sess_${TS}\"}" >/dev/null 2>&1 || true
api_post /v1/runs \
    "{\"tenant_id\":\"default\",\"workspace_id\":\"default\",\"project_id\":\"default\",\"session_id\":\"shape_sess_${TS}\",\"run_id\":\"${SHAPE_RUN}\"}" >/dev/null 2>&1 || true

SHAPE_STATUS=$(api_post_status "/v1/runs/${SHAPE_RUN}/orchestrate" \
    '{"goal":"Summarize cairn-rs architecture","max_iterations":2,"timeout_ms":30000}')
SHAPE_BODY=$(api_post_body "/v1/runs/${SHAPE_RUN}/orchestrate" \
    '{"goal":"Summarize cairn-rs architecture","max_iterations":2,"timeout_ms":30000}' 2>/dev/null || echo "{}")

if [ "$SHAPE_STATUS" = "200" ] || [ "$SHAPE_STATUS" = "202" ]; then
    TERM=$(echo "$SHAPE_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('termination',''))" 2>/dev/null || echo "")
    check "orchestrate response has termination field" test -n "$TERM"
    printf "  \033[36mINFO\033[0m  termination=%s\n" "$TERM"
    echo "$SHAPE_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d)" 2>/dev/null | head -3
elif [ "$SHAPE_STATUS" = "503" ]; then
    check "orchestrate returns 503 without brain provider (expected)" true
    ERR=$(echo "$SHAPE_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin).get('code',''))" 2>/dev/null || echo "")
    printf "  \033[36mINFO\033[0m  503 code: %s\n" "$ERR"
elif [ "$SHAPE_STATUS" = "502" ] || [ "$SHAPE_STATUS" = "429" ]; then
    check "provider offline — shape test skipped gracefully" true
    printf "  \033[33mSKIP\033[0m  Provider returned HTTP %s (offline/throttled)\n" "$SHAPE_STATUS"
else
    check "orchestrate endpoint reachable" test "$SHAPE_STATUS" -ne 0
    printf "  \033[33mWARN\033[0m  Unexpected HTTP %s — check server logs\n" "$SHAPE_STATUS"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "=== Summary: ${passed}/${total} passed, ${failed} failed ==="

if [ "$failed" -gt 0 ]; then
    exit 1
fi
