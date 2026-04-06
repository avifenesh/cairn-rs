"""
example.py — demonstrates basic cairn Python SDK usage.

Run against a local server:
    python example.py

Or against a remote server:
    CAIRN_URL=http://my-server:3000 CAIRN_TOKEN=my-token python example.py
"""

import os
import sys

from cairn_client import CairnClient, CairnError, CairnHTTPError

# ── Config ────────────────────────────────────────────────────────────────────

BASE_URL = os.environ.get("CAIRN_URL", "http://localhost:3000")
TOKEN    = os.environ.get("CAIRN_TOKEN", "cairn-demo-token")


def section(title: str) -> None:
    print(f"\n{'─' * 60}")
    print(f"  {title}")
    print(f"{'─' * 60}")


def ok(label: str, value: object = None) -> None:
    suffix = f" → {value}" if value is not None else ""
    print(f"  ✓  {label}{suffix}")


def err(label: str, exc: Exception) -> None:
    print(f"  ✗  {label}: {exc}")


# ── Main ──────────────────────────────────────────────────────────────────────

def main() -> None:
    print(f"\nCairn Python SDK — example")
    print(f"  Server : {BASE_URL}")
    print(f"  Token  : {TOKEN[:8]}…")

    # ── Create client (context manager closes the session automatically) ──────
    with CairnClient(BASE_URL, token=TOKEN, timeout=30) as client:

        # ── 1. Health check ───────────────────────────────────────────────────
        section("1. Health check")
        try:
            h = client.health()
            ok("GET /health", h)
        except CairnError as e:
            err("health", e)
            sys.exit(1)

        # ── 2. System stats ───────────────────────────────────────────────────
        section("2. System stats")
        try:
            st = client.stats()
            ok("total_runs",       st["total_runs"])
            ok("total_tasks",      st["total_tasks"])
            ok("active_runs",      st["active_runs"])
            ok("uptime_seconds",   st["uptime_seconds"])
        except CairnError as e:
            err("stats", e)

        # ── 3. Create session ─────────────────────────────────────────────────
        section("3. Create session")
        try:
            sess = client.create_session(
                tenant_id    = "sdk-example",
                workspace_id = "default",
                project_id   = "demo",
            )
            session_id = sess["session_id"]
            ok("session created", session_id)
            ok("state", sess["state"])
        except CairnError as e:
            err("create_session", e)
            sys.exit(1)

        # ── 4. Create run ─────────────────────────────────────────────────────
        section("4. Create run")
        try:
            run = client.create_run(
                session_id   = session_id,
                tenant_id    = "sdk-example",
                workspace_id = "default",
                project_id   = "demo",
            )
            run_id = run["run_id"]
            ok("run created", run_id)
            ok("state", run["state"])
        except CairnError as e:
            err("create_run", e)
            sys.exit(1)

        # ── 5. Pause + resume ─────────────────────────────────────────────────
        section("5. Pause + resume run")
        try:
            paused = client.pause_run(run_id, detail="example pause")
            ok("paused", paused["state"])

            resumed = client.resume_run(run_id)
            ok("resumed", resumed["state"])
        except CairnError as e:
            err("pause/resume", e)

        # ── 6. Run details ────────────────────────────────────────────────────
        section("6. Run details")
        try:
            run_detail = client.get_run(run_id)
            ok("state",     run_detail["state"])
            ok("version",   run_detail["version"])

            cost = client.get_run_cost(run_id)
            ok("cost_micros",    cost["total_cost_micros"])
            ok("provider_calls", cost["provider_calls"])

            events = client.get_run_events(run_id, limit=10)
            ok(f"events ({len(events)} found)")
        except CairnError as e:
            err("run details", e)

        # ── 7. List sessions + runs ───────────────────────────────────────────
        section("7. List sessions & runs")
        try:
            sessions = client.list_sessions(limit=5)
            ok(f"sessions returned: {len(sessions)}")

            runs = client.list_runs(limit=5)
            ok(f"runs returned: {len(runs)}")
        except CairnError as e:
            err("list", e)

        # ── 8. Tasks ──────────────────────────────────────────────────────────
        section("8. Tasks")
        try:
            tasks = client.list_tasks(limit=10)
            ok(f"total tasks in store: {len(tasks)}")
            if tasks:
                t = tasks[0]
                ok("first task id", t["task_id"])
                ok("first task state", t["state"])
        except CairnError as e:
            err("list_tasks", e)

        # ── 9. Pending approvals ──────────────────────────────────────────────
        section("9. Pending approvals")
        try:
            approvals = client.get_pending_approvals()
            ok(f"pending approvals: {len(approvals)}")
            if approvals:
                appr = approvals[0]
                ok("  resolving approval", appr["approval_id"])
                resolved = client.approve(appr["approval_id"], reason="example auto-approve")
                ok("  decision", resolved.get("decision"))
        except CairnError as e:
            err("approvals", e)

        # ── 10. Ollama generation ─────────────────────────────────────────────
        section("10. Ollama LLM generation")
        try:
            models_info = client.list_ollama_models()
            models = models_info.get("models", [])
            ok(f"available models: {models}")

            # Skip embedding-only models
            gen_model = next(
                (m for m in models if "embed" not in m.lower()),
                models[0] if models else None,
            )

            if gen_model:
                print(f"  → generating with {gen_model} …", flush=True)
                result = client.generate(
                    "Say 'hello from cairn' and nothing else.",
                    model=gen_model,
                )
                ok("text",       result["text"].strip()[:60])
                ok("latency_ms", result["latency_ms"])
                ok("tokens_in",  result.get("tokens_in"))
                ok("tokens_out", result.get("tokens_out"))
            else:
                print("  ⊘ no models available")
        except CairnHTTPError as e:
            if e.status_code in (503, 502):
                print(f"  ⊘ Ollama not available ({e})")
            else:
                err("generate", e)
        except CairnError as e:
            err("generate", e)
        except Exception as e:  # ReadTimeout from requests
            if "timeout" in str(e).lower() or "Timeout" in type(e).__name__:
                print(f"  ⊘ Ollama generation timed out (model is slow on first load — retry with a longer timeout)")
            else:
                raise

        # ── 11. Memory ────────────────────────────────────────────────────────
        section("11. Memory / knowledge store")
        try:
            ingest_result = client.ingest(
                source_id   = "sdk-demo-docs",
                document_id = "sdk-example-001",
                content     = (
                    "cairn is a self-hostable control plane for AI agents. "
                    "It provides session management, run orchestration, "
                    "task queuing, approval workflows, and an operator dashboard."
                ),
                tenant_id    = "sdk-example",
                workspace_id = "default",
                project_id   = "demo",
            )
            ok("ingest status", ingest_result.get("status"))

            search = client.search_memory(
                "orchestration task queue",
                limit        = 3,
                tenant_id    = "sdk-example",
                workspace_id = "default",
                project_id   = "demo",
            )
            results = search.get("results", [])
            ok(f"search results: {len(results)}")
            if results:
                ok("  top score", round(results[0]["score"], 3))
        except CairnError as e:
            err("memory", e)

        # ── 12. Costs ─────────────────────────────────────────────────────────
        section("12. Costs")
        try:
            costs = client.get_costs()
            ok("total_cost_micros",   costs["total_cost_micros"])
            ok("total_provider_calls", costs["total_provider_calls"])
        except CairnError as e:
            err("costs", e)

        # ── 13. Events ────────────────────────────────────────────────────────
        section("13. Event log")
        try:
            events = client.list_events(limit=5)
            ok(f"last 5 events ({len(events)} returned)")
            for ev in events[-3:]:
                ok(f"  pos={ev['position']}  type={ev['event_type']}")
        except CairnError as e:
            err("events", e)

        # ── 14. Settings ──────────────────────────────────────────────────────
        section("14. Settings")
        try:
            settings = client.get_settings()
            ok("deployment_mode", settings["deployment_mode"])
            ok("store_backend",   settings["store_backend"])
            ok("plugin_count",    settings["plugin_count"])
            ok("key_configured",  settings["key_management"]["encryption_key_configured"])
        except CairnError as e:
            err("settings", e)

    # ── Done ──────────────────────────────────────────────────────────────────
    print("\n" + "═" * 60)
    print("  Example complete.")
    print("═" * 60 + "\n")


if __name__ == "__main__":
    main()
