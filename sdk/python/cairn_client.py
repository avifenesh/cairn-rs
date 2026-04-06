"""
cairn_client.py — Python SDK for the cairn-rs Operator Control Plane.

Single-file client with no dependencies beyond the standard library and
``requests`` (pip install requests).

Quick start::

    from cairn_client import CairnClient

    client = CairnClient("http://localhost:3000", token="cairn-demo-token")
    print(client.health())

    sess = client.create_session("my-tenant", "default", "my-project")
    run  = client.create_run(sess["session_id"])
    print(run["state"])  # "pending"

"""

from __future__ import annotations

import uuid
from typing import Any

try:
    import requests
    from requests import Response
except ImportError as _e:  # pragma: no cover
    raise ImportError(
        "cairn_client requires the 'requests' library: pip install requests"
    ) from _e


# ── Exceptions ────────────────────────────────────────────────────────────────

class CairnError(Exception):
    """Base exception for all cairn SDK errors."""


class CairnHTTPError(CairnError):
    """Raised when the server returns a non-2xx status code."""

    def __init__(self, status_code: int, code: str, message: str) -> None:
        self.status_code = status_code
        self.code = code
        self.message = message
        super().__init__(f"HTTP {status_code} [{code}]: {message}")


class CairnConnectionError(CairnError):
    """Raised when the server cannot be reached."""


# ── Client ────────────────────────────────────────────────────────────────────

class CairnClient:
    """
    HTTP client for the cairn-rs REST API.

    All methods return plain ``dict`` / ``list`` values (parsed JSON).
    On non-2xx responses a :class:`CairnHTTPError` is raised with the
    server's ``code`` and ``message`` fields populated.

    :param base_url: Base URL of the cairn-app server,
                     e.g. ``"http://localhost:3000"``.
    :param token:    Admin bearer token (``CAIRN_ADMIN_TOKEN`` on the server).
    :param timeout:  Per-request timeout in seconds (default: 30).
    """

    def __init__(
        self,
        base_url: str,
        token: str,
        timeout: int = 30,
        generate_timeout: int = 120,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout
        # LLM generation can be much slower than regular API calls — use a
        # separate (longer) timeout so ordinary endpoints stay snappy.
        self.generate_timeout = generate_timeout
        self._session = requests.Session()
        self._session.headers.update({
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        })

    # ── Internal ──────────────────────────────────────────────────────────────

    def _url(self, path: str) -> str:
        return f"{self.base_url}{path}"

    def _raise_for_status(self, resp: Response) -> None:
        if resp.ok:
            return
        try:
            body = resp.json()
            code = body.get("code", "unknown_error")
            message = body.get("message", resp.text)
        except Exception:
            code = "parse_error"
            message = resp.text or f"HTTP {resp.status_code}"
        raise CairnHTTPError(resp.status_code, code, message)

    def _get(self, path: str, params: dict[str, Any] | None = None) -> Any:
        try:
            resp = self._session.get(
                self._url(path), params=params, timeout=self.timeout
            )
        except requests.ConnectionError as e:
            raise CairnConnectionError(f"Cannot connect to {self.base_url}: {e}") from e
        self._raise_for_status(resp)
        return resp.json()

    def _post(self, path: str, body: Any = None) -> Any:
        try:
            resp = self._session.post(
                self._url(path), json=body, timeout=self.timeout
            )
        except requests.ConnectionError as e:
            raise CairnConnectionError(f"Cannot connect to {self.base_url}: {e}") from e
        self._raise_for_status(resp)
        return resp.json() if resp.text else {}

    def _delete(self, path: str) -> Any:
        try:
            resp = self._session.delete(self._url(path), timeout=self.timeout)
        except requests.ConnectionError as e:
            raise CairnConnectionError(f"Cannot connect to {self.base_url}: {e}") from e
        self._raise_for_status(resp)
        return resp.json() if resp.text else {}

    # ── Health & system status ────────────────────────────────────────────────

    def health(self) -> dict:
        """
        GET /health — public liveness probe.

        :returns: ``{"ok": true}``
        :raises CairnConnectionError: if the server is unreachable.
        """
        try:
            resp = self._session.get(
                self._url("/health"), timeout=self.timeout
            )
        except requests.ConnectionError as e:
            raise CairnConnectionError(f"Cannot connect to {self.base_url}: {e}") from e
        resp.raise_for_status()
        return resp.json()

    def status(self) -> dict:
        """
        GET /v1/status — runtime and store health with uptime.

        :returns: ``{"runtime_ok": bool, "store_ok": bool, "uptime_secs": int}``
        """
        return self._get("/v1/status")

    def stats(self) -> dict:
        """
        GET /v1/stats — real-time system-wide counters.

        :returns: Dict with ``total_runs``, ``total_tasks``, ``active_runs``,
                  ``pending_approvals``, ``uptime_seconds``, etc.
        """
        return self._get("/v1/stats")

    def dashboard(self) -> dict:
        """
        GET /v1/dashboard — high-level operator overview.

        :returns: Active runs, tasks, approvals, failed counts,
                  system_healthy flag, etc.
        """
        return self._get("/v1/dashboard")

    def detailed_health(self) -> dict:
        """
        GET /v1/health/detailed — per-subsystem health with latency and memory.

        :returns: Overall status plus individual checks for store, Ollama,
                  event buffer, and memory (RSS).
        """
        return self._get("/v1/health/detailed")

    def metrics(self) -> dict:
        """
        GET /v1/metrics — rolling request metrics from the tracing middleware.

        :returns: ``total_requests``, ``requests_by_path``, latency
                  percentiles (p50/p95/p99), error_rate, errors_by_status.
        """
        return self._get("/v1/metrics")

    # ── Session management ────────────────────────────────────────────────────

    def create_session(
        self,
        tenant_id: str,
        workspace_id: str = "default",
        project_id: str = "default",
        session_id: str | None = None,
    ) -> dict:
        """
        POST /v1/sessions — create a new agent session.

        :param tenant_id:    Tenant to scope the session to.
        :param workspace_id: Workspace within the tenant (default: "default").
        :param project_id:   Project within the workspace (default: "default").
        :param session_id:   Optional explicit session ID.  A UUID is
                             generated automatically when omitted.
        :returns: :class:`SessionRecord` dict with ``session_id`` and
                  ``state="open"``.
        """
        return self._post("/v1/sessions", {
            "tenant_id":    tenant_id,
            "workspace_id": workspace_id,
            "project_id":   project_id,
            "session_id":   session_id or f"sess_{uuid.uuid4().hex[:12]}",
        })

    def list_sessions(
        self,
        limit: int = 50,
        offset: int = 0,
        tenant_id: str | None = None,
        workspace_id: str | None = None,
        project_id: str | None = None,
    ) -> list:
        """
        GET /v1/sessions — list active sessions, most recent first.

        :param limit:        Maximum number of sessions to return (default 50).
        :param offset:       Pagination offset (default 0).
        :param tenant_id:    Filter by tenant.
        :param workspace_id: Filter by workspace.
        :param project_id:   Filter by project.
        :returns: List of :class:`SessionRecord` dicts.
        """
        params: dict[str, Any] = {"limit": limit, "offset": offset}
        if tenant_id:    params["tenant_id"]    = tenant_id
        if workspace_id: params["workspace_id"] = workspace_id
        if project_id:   params["project_id"]   = project_id
        return self._get("/v1/sessions", params)

    def get_session(self, session_id: str) -> dict:
        """
        GET /v1/sessions/:id — fetch a single session by ID.

        :param session_id: The session to retrieve.
        :returns: :class:`SessionRecord` dict.
        :raises CairnHTTPError: 404 if the session does not exist.
        """
        # sessions list is the authoritative read; filter client-side
        sessions = self.list_sessions(limit=500)
        for s in sessions:
            if s.get("session_id") == session_id:
                return s
        raise CairnHTTPError(404, "not_found", f"session {session_id} not found")

    # ── Run management ────────────────────────────────────────────────────────

    def create_run(
        self,
        session_id: str,
        tenant_id: str = "default",
        workspace_id: str = "default",
        project_id: str = "default",
        run_id: str | None = None,
        parent_run_id: str | None = None,
    ) -> dict:
        """
        POST /v1/runs — start a new run within a session.

        :param session_id:    Session to attach this run to.
        :param tenant_id:     Tenant scope (default: "default").
        :param workspace_id:  Workspace scope (default: "default").
        :param project_id:    Project scope (default: "default").
        :param run_id:        Optional explicit run ID.  A UUID is generated
                              automatically when omitted.
        :param parent_run_id: Parent run for sub-agent spawning.
        :returns: :class:`RunRecord` dict with ``run_id`` and
                  ``state="pending"``.
        """
        body: dict[str, Any] = {
            "tenant_id":    tenant_id,
            "workspace_id": workspace_id,
            "project_id":   project_id,
            "session_id":   session_id,
            "run_id":       run_id or f"run_{uuid.uuid4().hex[:12]}",
        }
        if parent_run_id:
            body["parent_run_id"] = parent_run_id
        return self._post("/v1/runs", body)

    def get_run(self, run_id: str) -> dict:
        """
        GET /v1/runs/:id — fetch a single run by ID.

        :param run_id: The run to retrieve.
        :returns: :class:`RunRecord` dict.
        :raises CairnHTTPError: 404 if the run does not exist.
        """
        return self._get(f"/v1/runs/{run_id}")

    def list_runs(
        self,
        limit: int = 50,
        offset: int = 0,
        tenant_id: str | None = None,
        workspace_id: str | None = None,
        project_id: str | None = None,
    ) -> list:
        """
        GET /v1/runs — list runs (most recent first).

        :param limit:        Maximum number of runs to return (default 50).
        :param offset:       Pagination offset (default 0).
        :param tenant_id:    Scope to tenant.
        :param workspace_id: Scope to workspace.
        :param project_id:   Scope to project.
        :returns: List of :class:`RunRecord` dicts.
        """
        params: dict[str, Any] = {"limit": limit, "offset": offset}
        if tenant_id:    params["tenant_id"]    = tenant_id
        if workspace_id: params["workspace_id"] = workspace_id
        if project_id:   params["project_id"]   = project_id
        return self._get("/v1/runs", params)

    def pause_run(self, run_id: str, detail: str = "") -> dict:
        """
        POST /v1/runs/:id/pause — pause a running run.

        :param run_id: The run to pause.
        :param detail: Optional human-readable reason.
        :returns: Updated :class:`RunRecord` with ``state="paused"``.
        :raises CairnHTTPError: 400 if the run is not in a pausable state.
        """
        body: dict[str, Any] = {"reason_kind": "operator_pause"}
        if detail:
            body["detail"] = detail
        return self._post(f"/v1/runs/{run_id}/pause", body)

    def resume_run(self, run_id: str) -> dict:
        """
        POST /v1/runs/:id/resume — resume a paused run.

        :param run_id: The run to resume.
        :returns: Updated :class:`RunRecord` with ``state="running"``.
        :raises CairnHTTPError: 400 if the run is not paused.
        """
        return self._post(f"/v1/runs/{run_id}/resume", {})

    def get_run_cost(self, run_id: str) -> dict:
        """
        GET /v1/runs/:id/cost — accumulated cost for a run.

        :param run_id: The run to query.
        :returns: Dict with ``total_cost_micros``, ``total_tokens_in``,
                  ``total_tokens_out``, ``provider_calls``.
        """
        return self._get(f"/v1/runs/{run_id}/cost")

    def get_run_events(self, run_id: str, limit: int = 100) -> list:
        """
        GET /v1/runs/:id/events — event timeline for a run.

        :param run_id: The run to query.
        :param limit:  Maximum events to return (default 100).
        :returns: List of ``{"position": int, "stored_at": int,
                  "event_type": str}`` dicts, oldest first.
        """
        return self._get(f"/v1/runs/{run_id}/events", {"limit": limit})

    def get_run_tasks(self, run_id: str) -> list:
        """
        GET /v1/runs/:id/tasks — tasks belonging to a run.

        :param run_id: The run to query.
        :returns: List of :class:`TaskRecord` dicts.
        """
        return self._get(f"/v1/runs/{run_id}/tasks")

    # ── Task management ───────────────────────────────────────────────────────

    def list_tasks(
        self,
        limit: int = 50,
        offset: int = 0,
    ) -> list:
        """
        GET /v1/tasks — list all tasks (operator view).

        :param limit:  Maximum number of tasks to return (default 50).
        :param offset: Pagination offset (default 0).
        :returns: List of :class:`TaskRecord` dicts.
        """
        return self._get("/v1/tasks", {"limit": limit, "offset": offset})

    def claim_task(
        self,
        task_id: str,
        worker_id: str,
        lease_duration_ms: int = 60_000,
    ) -> dict:
        """
        POST /v1/tasks/:id/claim — claim a queued task for a worker.

        Sets the task state to ``"leased"`` with an expiry.

        :param task_id:          The task to claim.
        :param worker_id:        Identifier for the claiming worker.
        :param lease_duration_ms: Lease expiry in milliseconds (default 60 s).
        :returns: Updated :class:`TaskRecord` with ``state="leased"`` and
                  ``lease_owner=worker_id``.
        :raises CairnHTTPError: 404 if task not found, 400 if not claimable.
        """
        return self._post(f"/v1/tasks/{task_id}/claim", {
            "worker_id":         worker_id,
            "lease_duration_ms": lease_duration_ms,
        })

    def release_task(self, task_id: str) -> dict:
        """
        POST /v1/tasks/:id/release-lease — release a leased task back to queue.

        :param task_id: The task whose lease to release.
        :returns: Updated :class:`TaskRecord` with ``state="queued"``.
        :raises CairnHTTPError: 404 if task not found.
        """
        return self._post(f"/v1/tasks/{task_id}/release-lease")

    # ── Approvals ─────────────────────────────────────────────────────────────

    def request_approval(
        self,
        approval_id: str,
        run_id: str,
        tenant_id: str = "default",
        workspace_id: str = "default",
        project_id: str = "default",
        requirement: str = "required",
    ) -> dict:
        """
        POST /v1/approvals — request an approval gate for a run.

        The associated run transitions to ``waiting_approval`` until the
        gate is resolved.

        :param approval_id:  Unique approval identifier.
        :param run_id:       Run to block on this approval.
        :param tenant_id:    Tenant scope (default: "default").
        :param workspace_id: Workspace scope (default: "default").
        :param project_id:   Project scope (default: "default").
        :param requirement:  ``"required"`` or ``"advisory"``
                             (default: ``"required"``).
        :returns: :class:`ApprovalRecord` dict.
        """
        return self._post("/v1/approvals", {
            "tenant_id":    tenant_id,
            "workspace_id": workspace_id,
            "project_id":   project_id,
            "approval_id":  approval_id,
            "run_id":       run_id,
            "requirement":  requirement,
        })

    def approve_approval(self, approval_id: str) -> dict:
        """
        POST /v1/approvals/:id/approve — approve a pending gate directly.

        The associated run transitions from ``waiting_approval`` to
        ``running``.

        :param approval_id: The approval to approve.
        :returns: Updated :class:`ApprovalRecord`.
        :raises CairnHTTPError: 404 if approval not found.
        """
        return self._post(f"/v1/approvals/{approval_id}/approve")

    def reject_approval(self, approval_id: str) -> dict:
        """
        POST /v1/approvals/:id/reject — reject a pending gate directly.

        The associated run transitions from ``waiting_approval`` to
        ``failed`` with failure_class ``ApprovalRejected``.

        :param approval_id: The approval to reject.
        :returns: Updated :class:`ApprovalRecord`.
        :raises CairnHTTPError: 404 if approval not found.
        """
        return self._post(f"/v1/approvals/{approval_id}/reject")

    def get_pending_approvals(
        self,
        tenant_id: str | None = None,
        workspace_id: str | None = None,
        project_id: str | None = None,
    ) -> list:
        """
        GET /v1/approvals/pending — list pending (undecided) approvals.

        :param tenant_id:    Scope to tenant.
        :param workspace_id: Scope to workspace.
        :param project_id:   Scope to project.
        :returns: List of :class:`ApprovalRecord` dicts.
        """
        params: dict[str, Any] = {}
        if tenant_id:    params["tenant_id"]    = tenant_id
        if workspace_id: params["workspace_id"] = workspace_id
        if project_id:   params["project_id"]   = project_id
        return self._get("/v1/approvals/pending", params or None)

    def approve(self, approval_id: str, reason: str = "") -> dict:
        """
        POST /v1/approvals/:id/resolve — approve a pending approval gate.

        :param approval_id: The approval to resolve.
        :param reason:      Optional free-text explanation.
        :returns: Updated :class:`ApprovalRecord` with
                  ``decision="approved"``.
        :raises CairnHTTPError: 404 if approval not found.
        """
        body: dict[str, Any] = {"decision": "approved"}
        if reason:
            body["reason"] = reason
        return self._post(f"/v1/approvals/{approval_id}/resolve", body)

    def reject(self, approval_id: str, reason: str = "") -> dict:
        """
        POST /v1/approvals/:id/resolve — reject a pending approval gate.

        :param approval_id: The approval to resolve.
        :param reason:      Optional free-text explanation.
        :returns: Updated :class:`ApprovalRecord` with
                  ``decision="rejected"``.
        :raises CairnHTTPError: 404 if approval not found.
        """
        body: dict[str, Any] = {"decision": "rejected"}
        if reason:
            body["reason"] = reason
        return self._post(f"/v1/approvals/{approval_id}/resolve", body)

    # ── LLM generation ────────────────────────────────────────────────────────

    def generate(
        self,
        prompt: str,
        model: str = "qwen3:8b",
        messages: list[dict] | None = None,
    ) -> dict:
        """
        POST /v1/providers/ollama/generate — run a prompt through the local
        Ollama LLM.

        :param prompt:   Single-turn prompt text.  Ignored when ``messages``
                         is provided.
        :param model:    Ollama model name (default: ``"qwen3:8b"``).
        :param messages: Multi-turn conversation history.  Each entry must be
                         ``{"role": "user"|"assistant"|"system",
                         "content": "..."}``.  When supplied, ``prompt`` is
                         ignored.
        :returns: Dict with ``text``, ``model``, ``tokens_in``,
                  ``tokens_out``, ``latency_ms``.
        :raises CairnHTTPError: 503 when Ollama is not configured,
                                502 when the Ollama daemon is unreachable.
        """
        body: dict[str, Any] = {"model": model, "prompt": prompt}
        if messages is not None:
            body["messages"] = messages
        # Temporarily override timeout for generation (LLMs are slow).
        saved = self.timeout
        self.timeout = self.generate_timeout
        try:
            return self._post("/v1/providers/ollama/generate", body)
        finally:
            self.timeout = saved

    def list_ollama_models(self) -> dict:
        """
        GET /v1/providers/ollama/models — list locally available Ollama models.

        :returns: Dict with ``host``, ``models`` (list of name strings),
                  ``count``.
        :raises CairnHTTPError: 503 when OLLAMA_HOST is not configured.
        """
        return self._get("/v1/providers/ollama/models")

    # ── Provider connections ──────────────────────────────────────────────────

    def create_provider_connection(
        self,
        tenant_id: str,
        provider_connection_id: str,
        provider_family: str,
        adapter_type: str,
    ) -> dict:
        """
        POST /v1/providers/connections — register a new provider connection.

        Entitlement-gated: requires a tier that supports external providers
        (returns 403 in ``local_eval`` tier).

        :param tenant_id:              Tenant that owns the connection.
        :param provider_connection_id: Unique connection identifier.
        :param provider_family:        Provider family (e.g. ``"openai"``).
        :param adapter_type:           Adapter type (e.g. ``"responses_api"``).
        :returns: Created provider connection record.
        :raises CairnHTTPError: 403 if the entitlement tier does not allow
                                external provider connections.
        """
        return self._post("/v1/providers/connections", {
            "tenant_id":              tenant_id,
            "provider_connection_id": provider_connection_id,
            "provider_family":        provider_family,
            "adapter_type":           adapter_type,
        })

    def list_provider_connections(
        self,
        tenant_id: str,
        limit: int = 50,
        offset: int = 0,
    ) -> list:
        """
        GET /v1/providers/connections — list provider connections for a tenant.

        :param tenant_id: Tenant to scope the listing to.
        :param limit:     Maximum connections to return (default 50).
        :param offset:    Pagination offset (default 0).
        :returns: List of provider connection record dicts.
        """
        return self._get("/v1/providers/connections", {
            "tenant_id": tenant_id,
            "limit": limit,
            "offset": offset,
        })

    # ── Memory / knowledge store ──────────────────────────────────────────────

    def ingest(
        self,
        source_id: str,
        document_id: str,
        content: str,
        tenant_id: str = "default",
        workspace_id: str = "default",
        project_id: str = "default",
        source_type: str = "plain_text",
    ) -> dict:
        """
        POST /v1/memory/ingest — add a document to the knowledge store.

        :param source_id:    Logical source identifier (e.g. ``"docs-v2"``).
        :param document_id:  Unique document ID within the source.
        :param content:      Full text content to chunk and index.
        :param tenant_id:    Tenant scope (default: "default").
        :param workspace_id: Workspace scope (default: "default").
        :param project_id:   Project scope (default: "default").
        :param source_type:  ``"plain_text"`` or ``"markdown"``
                             (default: ``"plain_text"``).
        :returns: ``{"document_id": str, "source_id": str, "status": "ingested"}``
        """
        return self._post("/v1/memory/ingest", {
            "source_id":    source_id,
            "document_id":  document_id,
            "content":      content,
            "tenant_id":    tenant_id,
            "workspace_id": workspace_id,
            "project_id":   project_id,
            "source_type":  source_type,
        })

    def search_memory(
        self,
        query: str,
        limit: int = 10,
        tenant_id: str = "default",
        workspace_id: str = "default",
        project_id: str = "default",
    ) -> dict:
        """
        GET /v1/memory/search — lexical retrieval over the knowledge store.

        :param query:        Search query string.
        :param limit:        Maximum number of results (default 10).
        :param tenant_id:    Tenant scope (default: "default").
        :param workspace_id: Workspace scope (default: "default").
        :param project_id:   Project scope (default: "default").
        :returns: Dict with ``results`` list.  Each result has ``chunk_id``,
                  ``source_id``, ``text``, ``score``.
        """
        return self._get("/v1/memory/search", {
            "query_text":   query,
            "limit":        limit,
            "tenant_id":    tenant_id,
            "workspace_id": workspace_id,
            "project_id":   project_id,
        })

    def list_sources(
        self,
        tenant_id: str = "default",
        workspace_id: str = "default",
        project_id: str = "default",
    ) -> list:
        """
        GET /v1/sources — list knowledge sources and their document counts.

        :param tenant_id:    Tenant scope (default: "default").
        :param workspace_id: Workspace scope (default: "default").
        :param project_id:   Project scope (default: "default").
        :returns: List of ``{"source_id": str, "document_count": int, ...}``.
        """
        return self._get("/v1/sources", {
            "tenant_id":    tenant_id,
            "workspace_id": workspace_id,
            "project_id":   project_id,
        })

    # ── Bundles ──────────────────────────────────────────────────────────────

    def apply_bundle(self, bundle: dict) -> dict:
        """
        POST /v1/bundles/apply — import a curated knowledge bundle.

        :param bundle: Full bundle envelope dict (with ``bundle_schema_version``,
                       ``artifacts``, etc.).
        :returns: Import report with ``create_count``, ``skip_count``, etc.
        """
        return self._post("/v1/bundles/apply", {"bundle": bundle})

    def export_bundle(
        self,
        project: str,
        source_ids: str | None = None,
    ) -> dict:
        """
        GET /v1/bundles/export — export documents as a bundle.

        :param project:    Project path as ``"tenant/workspace/project"``.
        :param source_ids: Comma-separated source IDs to filter by.
        :returns: Bundle envelope dict with ``artifact_count`` and ``artifacts``.
        """
        params: dict[str, Any] = {"project": project}
        if source_ids:
            params["source_ids"] = source_ids
        return self._get("/v1/bundles/export", params)

    # ── Costs ─────────────────────────────────────────────────────────────────

    def get_costs(self) -> dict:
        """
        GET /v1/costs — aggregate token and cost totals (server-wide).

        :returns: Dict with ``total_cost_micros``, ``total_tokens_in``,
                  ``total_tokens_out``, ``total_provider_calls``.
        """
        return self._get("/v1/costs")

    # ── Events ────────────────────────────────────────────────────────────────

    def list_events(
        self,
        limit: int = 100,
        after: int | None = None,
    ) -> list:
        """
        GET /v1/events — cursor-based replay of the global event log.

        :param limit: Maximum events to return (default 100, max 500).
        :param after: Return only events with position > this value
                      (for cursor-based pagination).
        :returns: List of ``{"position": int, "stored_at": int,
                  "event_type": str}`` dicts, oldest first.
        """
        params: dict[str, Any] = {"limit": limit}
        if after is not None:
            params["after"] = after
        return self._get("/v1/events", params)

    # ── Settings ──────────────────────────────────────────────────────────────

    def get_settings(self) -> dict:
        """
        GET /v1/settings — deployment configuration snapshot.

        :returns: Dict with ``deployment_mode``, ``store_backend``,
                  ``plugin_count``, ``system_health``, ``key_management``.
        """
        return self._get("/v1/settings")

    # ── Context manager ───────────────────────────────────────────────────────

    def __enter__(self) -> "CairnClient":
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()

    def close(self) -> None:
        """Close the underlying HTTP session."""
        self._session.close()

    def __repr__(self) -> str:
        return f"CairnClient(base_url={self.base_url!r})"
