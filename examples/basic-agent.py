#!/usr/bin/env python3
"""
basic-agent.py — Minimal SDK client exercising the full Cairn API lifecycle.

Usage:
    # Against a local dev instance:
    python3 examples/basic-agent.py

    # Custom endpoint / token:
    CAIRN_URL=http://my-host:3000 CAIRN_TOKEN=my-token python3 examples/basic-agent.py

Requires: Python 3.8+, no external dependencies (stdlib only).
"""

import json
import os
import sys
import time
import urllib.request
import urllib.error
import uuid

BASE = os.environ.get("CAIRN_URL", "http://localhost:3000")
TOKEN = os.environ.get("CAIRN_TOKEN", "dev-admin-token")

# ── Unique IDs for this run ──────────────────────────────────────────────────

suffix = f"{int(time.time())}_{uuid.uuid4().hex[:6]}"
SESSION_ID = f"sdk_sess_{suffix}"
RUN_ID = f"sdk_run_{suffix}"
TASK_ID = f"sdk_task_{suffix}"
APPROVAL_ID = f"sdk_appr_{suffix}"
EVAL_RUN_ID = f"sdk_eval_{suffix}"

PROJECT = {"tenant_id": "default", "workspace_id": "default", "project_id": "default"}
SCOPE = f"tenant_id=default&workspace_id=default&project_id=default"


def api(method: str, path: str, body=None):
    """Make an API call. Returns (status_code, parsed_json | raw_text)."""
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url,
        method=method,
        data=data,
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            text = resp.read().decode()
            try:
                return resp.status, json.loads(text)
            except json.JSONDecodeError:
                return resp.status, text
    except urllib.error.HTTPError as e:
        text = e.read().decode() if e.fp else ""
        try:
            return e.code, json.loads(text)
        except json.JSONDecodeError:
            return e.code, text


def check(label: str, status: int, method: str, path: str, body=None):
    """Call API and assert expected status code."""
    code, data = api(method, path, body)
    ok = code == status
    mark = "✓" if ok else "✗"
    print(f"  {mark} {label} (HTTP {code})")
    if not ok:
        detail = json.dumps(data)[:200] if isinstance(data, dict) else str(data)[:200]
        print(f"    expected {status}, body: {detail}")
    return ok, data


def section(title: str):
    print(f"\n── {title}")


# ── 1. Health check ──────────────────────────────────────────────────────────

section("1. Health check")
ok, data = check("GET /health", 200, "GET", "/health")
if not ok:
    print("Server not reachable. Start with: cargo run -p cairn-app")
    sys.exit(1)

# ── 2. Create session ────────────────────────────────────────────────────────

section("2. Create session")
check("POST /v1/sessions", 201, "POST", "/v1/sessions", {
    **PROJECT, "session_id": SESSION_ID,
})

# ── 3. Start a run ───────────────────────────────────────────────────────────

section("3. Start run")
check("POST /v1/runs", 201, "POST", "/v1/runs", {
    **PROJECT, "session_id": SESSION_ID, "run_id": RUN_ID,
})

# ── 4. Submit a task via event append ────────────────────────────────────────

section("4. Submit task (event append)")
check("POST /v1/events/append (TaskCreated)", 201, "POST", "/v1/events/append", [{
    "event_id": f"evt_task_{suffix}",
    "source": {"source_type": "runtime"},
    "ownership": {"scope": "project", **PROJECT},
    "causation_id": None,
    "correlation_id": None,
    "payload": {
        "event": "task_created",
        "project": PROJECT,
        "task_id": TASK_ID,
        "parent_run_id": RUN_ID,
        "parent_task_id": None,
        "prompt_release_id": None,
    },
}])
time.sleep(0.3)

ok, tasks = check(f"GET /v1/tasks?{SCOPE}", 200, "GET", f"/v1/tasks?{SCOPE}")
task_list = tasks if isinstance(tasks, list) else tasks.get("items", [])
found = any(t.get("task_id") == TASK_ID for t in task_list)
print(f"  {'✓' if found else '✗'} task {TASK_ID} in list")

# ── 5. Claim and release task ────────────────────────────────────────────────

section("5. Worker claim/release")
check("POST claim", 200, "POST", f"/v1/tasks/{TASK_ID}/claim", {
    "worker_id": f"worker_{suffix}", "lease_duration_ms": 30000,
})
check("POST release-lease", 200, "POST", f"/v1/tasks/{TASK_ID}/release-lease", {})

# ── 6. Approval gate ─────────────────────────────────────────────────────────

section("6. Approval gate")
check("POST /v1/events/append (ApprovalRequested)", 201, "POST", "/v1/events/append", [{
    "event_id": f"evt_appr_{suffix}",
    "source": {"source_type": "runtime"},
    "ownership": {"scope": "project", **PROJECT},
    "causation_id": None,
    "correlation_id": None,
    "payload": {
        "event": "approval_requested",
        "project": PROJECT,
        "approval_id": APPROVAL_ID,
        "run_id": RUN_ID,
        "task_id": None,
        "requirement": "required",
    },
}])
time.sleep(0.3)

ok, pending = check("GET /v1/approvals/pending", 200, "GET", "/v1/approvals/pending")
pending_list = pending if isinstance(pending, list) else pending.get("items", [])
has_appr = any(a.get("approval_id") == APPROVAL_ID for a in pending_list)
print(f"  {'✓' if has_appr else '✗'} approval in pending list")

check("POST resolve (approved)", 200, "POST", f"/v1/approvals/{APPROVAL_ID}/resolve", {
    "decision": "approved", "reason": "sdk-test",
})

# ── 7. Pause / resume run ───────────────────────────────────────────────────

section("7. Pause / resume")
check("POST pause", 200, "POST", f"/v1/runs/{RUN_ID}/pause", {
    "reason_kind": "operator_pause", "detail": "sdk test pause",
})
check("POST resume", 200, "POST", f"/v1/runs/{RUN_ID}/resume", {})

# ── 8. Complete task and run ─────────────────────────────────────────────────

section("8. Complete lifecycle")
# Re-claim task for completion
check("POST claim (for complete)", 200, "POST", f"/v1/tasks/{TASK_ID}/claim", {
    "worker_id": f"worker_{suffix}", "lease_duration_ms": 30000,
})
check("POST complete task", 200, "POST", f"/v1/tasks/{TASK_ID}/complete", {})

ok, run = check(f"GET /v1/runs/{RUN_ID}", 200, "GET", f"/v1/runs/{RUN_ID}")
print(f"  run state: {run.get('state', '?')}")

check("POST complete run", 200, "POST", f"/v1/runs/{RUN_ID}/complete", {})
ok, run = check(f"GET run (after complete)", 200, "GET", f"/v1/runs/{RUN_ID}")
print(f"  run state: {run.get('state', '?')}")

# ── 9. Eval run ──────────────────────────────────────────────────────────────

section("9. Eval lifecycle")
check("POST /v1/evals/runs", 201, "POST", "/v1/evals/runs", {
    **PROJECT, "eval_run_id": EVAL_RUN_ID,
    "subject_kind": "prompt_release", "evaluator_type": "accuracy",
})
check("POST start eval", 200, "POST", f"/v1/evals/runs/{EVAL_RUN_ID}/start", {})
check("POST score eval", 200, "POST", f"/v1/evals/runs/{EVAL_RUN_ID}/score", {
    "metrics": {"accuracy": 0.92, "latency_p50_ms": 85.0},
})
check("POST complete eval", 200, "POST", f"/v1/evals/runs/{EVAL_RUN_ID}/complete", {
    "metrics": {"accuracy": 0.92, "latency_p50_ms": 85.0}, "cost": 0.03,
})

# ── 10. Cancel a run ─────────────────────────────────────────────────────────

section("10. Cancel run")
cancel_run_id = f"sdk_crun_{suffix}"
check("POST /v1/runs (for cancel)", 201, "POST", "/v1/runs", {
    **PROJECT, "session_id": SESSION_ID, "run_id": cancel_run_id,
})
ok, data = check("POST /v1/runs/:id/cancel", 200, "POST", f"/v1/runs/{cancel_run_id}/cancel", {})
print(f"  state: {data.get('state', '?')}")

# ── 11. Read-back: costs, events, traces ─────────────────────────────────────

section("11. Observability read-back")
check("GET /v1/costs", 200, "GET", "/v1/costs?tenant_id=default")
check("GET /v1/events/recent", 200, "GET", "/v1/events/recent?limit=5")
check(f"GET /v1/sessions/{SESSION_ID}/llm-traces", 200, "GET",
      f"/v1/sessions/{SESSION_ID}/llm-traces")
check("GET /v1/dashboard", 200, "GET", f"/v1/dashboard?{SCOPE}")

# ── Summary ──────────────────────────────────────────────────────────────────

section("Summary")
print(f"  session:  {SESSION_ID}")
print(f"  run:      {RUN_ID}")
print(f"  task:     {TASK_ID}")
print(f"  eval:     {EVAL_RUN_ID}")
print(f"  canceled: {cancel_run_id}")
print(f"\n  All steps completed. Cairn API exercised end-to-end.")
