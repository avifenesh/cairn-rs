/**
 * ApiDocsPage — interactive API reference with:
 *   • Domain sidebar + endpoint accordion
 *   • "Try it" panel with pre-filled auth, curl command, copy-curl button
 *   • "Recent Requests" tab showing a DevTools-style network log
 */

import { useState } from "react";
import {
  FileCode2, ChevronRight, ChevronDown, Send, Loader2,
  Copy, Check, Trash2, Terminal, Clock,
} from "lucide-react";
import { clsx } from "clsx";
import { useRequestLog } from "../components/RequestLogContext";
import { CopyButton } from "../components/CopyButton";

// ── Types ─────────────────────────────────────────────────────────────────────

type HttpMethod = "GET" | "POST" | "DELETE" | "PUT";

interface Param {
  name: string;
  type: string;
  required?: boolean;
  description: string;
  example?: string;
}

interface Endpoint {
  id: string;
  method: HttpMethod;
  path: string;
  description: string;
  pathParams?: Param[];
  queryParams?: Param[];
  bodyFields?: Param[];
  bodyExample?: string;
  responseDesc: string;
  sse?: boolean;
  auth?: boolean;
}

interface DomainGroup {
  id: string;
  label: string;
  dot: string;
  endpoints: Endpoint[];
}

// ── Endpoint catalog ──────────────────────────────────────────────────────────

const DOMAINS: DomainGroup[] = [
  {
    id: "health", label: "Health & Status", dot: "bg-emerald-500",
    endpoints: [
      { id: "get-health",  method: "GET", path: "/health",    description: "Liveness probe. Returns immediately with ok=true when the server is up.", responseDesc: '{ "ok": true }', auth: false },
      { id: "get-status",  method: "GET", path: "/v1/status", description: "Runtime and store health with uptime in seconds.", responseDesc: '{ "runtime_ok": true, "store_ok": true, "uptime_secs": 3600 }' },
      { id: "get-sysinfo", method: "GET", path: "/v1/system/info", description: "Comprehensive build metadata, feature flags, and environment configuration.", responseDesc: '{ version, os, arch, git_commit, features: {...}, environment: {...} }' },
      { id: "get-detailed-health", method: "GET", path: "/v1/health/detailed", description: "Per-subsystem health: store latency, process memory, event buffer.", responseDesc: '{ status, checks: { store, event_buffer, memory }, uptime_seconds }' },
    ],
  },
  {
    id: "sessions", label: "Sessions", dot: "bg-blue-500",
    endpoints: [
      {
        id: "list-sessions", method: "GET", path: "/v1/sessions",
        description: "List active sessions, most recent first.",
        queryParams: [
          { name: "limit",  type: "number", description: "Maximum results", example: "100" },
          { name: "offset", type: "number", description: "Pagination offset", example: "0"  },
        ],
        responseDesc: "Array of SessionRecord",
      },
      {
        id: "create-session", method: "POST", path: "/v1/sessions",
        description: "Create a new conversation session.",
        bodyFields: [
          { name: "tenant_id",    type: "string", description: "Tenant identifier",    example: "default"    },
          { name: "workspace_id", type: "string", description: "Workspace identifier", example: "default" },
          { name: "project_id",   type: "string", description: "Project identifier",   example: "default"   },
          { name: "session_id",   type: "string", description: "Override the auto-generated session ID" },
        ],
        bodyExample: '{\n  "tenant_id": "default",\n  "workspace_id": "default",\n  "project_id": "default"\n}',
        responseDesc: "SessionRecord — the newly created session",
      },
      {
        id: "export-session", method: "GET", path: "/v1/sessions/:id/export",
        description: "Export a session with all its runs, tasks, and events as a JSON bundle.",
        pathParams: [{ name: "id", type: "string", description: "Session ID", example: "sess_..." }],
        responseDesc: '{ version, type: "session_export", exported_at, data: { session, runs, tasks, events } }',
      },
    ],
  },
  {
    id: "runs", label: "Runs", dot: "bg-indigo-500",
    endpoints: [
      {
        id: "list-runs", method: "GET", path: "/v1/runs",
        description: "List runs, most recent first.",
        queryParams: [
          { name: "limit",  type: "number", description: "Max results (default 200)", example: "50" },
          { name: "offset", type: "number", description: "Pagination offset",          example: "0"  },
        ],
        responseDesc: "Array of RunRecord",
      },
      { id: "get-run",       method: "GET",  path: "/v1/runs/:id",              description: "Fetch a single run by ID.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], responseDesc: "RunRecord" },
      { id: "get-run-cost",  method: "GET",  path: "/v1/runs/:id/cost",         description: "Accumulated cost and token usage for a run.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], responseDesc: "RunCostRecord" },
      { id: "get-run-tasks", method: "GET",  path: "/v1/runs/:id/tasks",        description: "Tasks belonging to a run.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], responseDesc: "Array of TaskRecord" },
      { id: "get-run-events",method: "GET",  path: "/v1/runs/:id/events",       description: "Event timeline for a run.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], queryParams: [{ name: "limit", type: "number", description: "Max events (default 100)", example: "100" }], responseDesc: "Array of RunEventSummary" },
      { id: "export-run",    method: "GET",  path: "/v1/runs/:id/export",       description: "Export run + tasks + events as a JSON bundle.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], responseDesc: '{ version, type: "run_export", data: { run, tasks, events } }' },
      { id: "pause-run",     method: "POST", path: "/v1/runs/:id/pause",        description: "Pause a running run.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], bodyExample: '{ "detail": "Manual pause" }', responseDesc: "Updated RunRecord" },
      { id: "resume-run",    method: "POST", path: "/v1/runs/:id/resume",       description: "Resume a paused run.", pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }], bodyExample: '{}', responseDesc: "Updated RunRecord" },
    ],
  },
  {
    id: "tasks", label: "Tasks", dot: "bg-violet-500",
    endpoints: [
      {
        id: "list-tasks", method: "GET", path: "/v1/tasks",
        description: "All tasks across every project (operator view).",
        queryParams: [{ name: "limit", type: "number", description: "Max results (default 500)", example: "100" }],
        responseDesc: "Array of TaskRecord",
      },
      { id: "claim-task",   method: "POST", path: "/v1/tasks/:id/claim",         description: "Claim a queued task for a worker.", pathParams: [{ name: "id", type: "string", description: "Task ID", example: "task_..." }], bodyExample: '{ "worker_id": "operator", "lease_duration_ms": 30000 }', responseDesc: "Updated TaskRecord" },
      { id: "release-task", method: "POST", path: "/v1/tasks/:id/release-lease", description: "Release a leased task back to queued.", pathParams: [{ name: "id", type: "string", description: "Task ID", example: "task_..." }], bodyExample: '{}', responseDesc: "Updated TaskRecord" },
    ],
  },
  {
    id: "approvals", label: "Approvals", dot: "bg-amber-500",
    endpoints: [
      {
        id: "list-approvals",   method: "GET",  path: "/v1/approvals/pending",
        description: "List pending (unresolved) approvals.",
        responseDesc: "Array of ApprovalRecord",
      },
      {
        id: "resolve-approval", method: "POST", path: "/v1/approvals/:id/resolve",
        description: "Approve or reject a pending approval gate.",
        pathParams: [{ name: "id", type: "string", description: "Approval ID", example: "appr_..." }],
        bodyExample: '{ "decision": "approved" }',
        responseDesc: "Updated ApprovalRecord",
      },
    ],
  },
  {
    id: "costs", label: "Costs & Stats", dot: "bg-teal-500",
    endpoints: [
      { id: "get-costs",     method: "GET", path: "/v1/costs",     description: "Aggregate token and cost totals.", responseDesc: "CostSummary" },
      { id: "get-dashboard", method: "GET", path: "/v1/dashboard", description: "Operator overview: runs, tasks, approvals, cost, health.", responseDesc: "DashboardOverview" },
      { id: "get-stats",     method: "GET", path: "/v1/stats",     description: "Real-time system-wide counters.", responseDesc: "SystemStats — { total_events, active_runs, pending_approvals, uptime_seconds }" },
    ],
  },
  {
    id: "providers", label: "Providers", dot: "bg-orange-500",
    endpoints: [
      { id: "get-provider-health",  method: "GET",  path: "/v1/providers/health",                    description: "Health records for all registered LLM providers.", responseDesc: "Array of provider health records" },
      { id: "get-provider-registry",method: "GET",  path: "/v1/providers/registry",                  description: "Static provider registry with availability and known models.", responseDesc: "Array of provider registry entries" },
      { id: "list-connections",     method: "GET",  path: "/v1/providers/connections",                description: "List registered provider connections.", responseDesc: '{ items: ProviderConnection[], has_more }' },
      { id: "create-connection",    method: "POST", path: "/v1/providers/connections",                description: "Register a new provider connection (any OpenAI-compatible endpoint).", bodyExample: '{ "name": "my-provider", "base_url": "https://api.example.com/v1", "api_key": "sk-..." }', responseDesc: '{ provider_connection_id, ... }' },
    ],
  },
  {
    id: "memory", label: "Memory", dot: "bg-purple-500",
    endpoints: [
      {
        id: "memory-search", method: "GET", path: "/v1/memory/search",
        description: "Lexical retrieval over the knowledge store with scoring breakdown.",
        queryParams: [
          { name: "query_text",   type: "string", required: true, description: "Search query",    example: "agent" },
          { name: "tenant_id",    type: "string",                 description: "Tenant scope",    example: "default"    },
          { name: "workspace_id", type: "string",                 description: "Workspace scope", example: "default" },
          { name: "project_id",   type: "string",                 description: "Project scope",   example: "default"   },
          { name: "limit",        type: "number",                 description: "Max results",     example: "5" },
        ],
        responseDesc: '{ results: [{ score, chunk: { text, ... }, breakdown }], diagnostics }',
      },
    ],
  },
  {
    id: "events", label: "Events", dot: "bg-sky-500",
    endpoints: [
      { id: "events-stream", method: "GET", path: "/v1/stream",         description: "SSE stream of all runtime events. Supports Last-Event-ID replay.", sse: true, responseDesc: "SSE: event: <type>\\ndata: <json>\\nid: <seq>" },
      { id: "events-recent", method: "GET", path: "/v1/events/recent",  description: "Fetch the most recent N runtime events.", queryParams: [{ name: "limit", type: "number", description: "Max events (default 50)", example: "50" }], responseDesc: "Array of RecentEvent" },
      { id: "notifications",  method: "GET", path: "/v1/notifications", description: "Recent operator notifications (approvals, run failures, etc.).", queryParams: [{ name: "limit", type: "number", description: "Max notifications (default 50)", example: "50" }], responseDesc: '{ notifications: [...], unread_count: number }' },
    ],
  },
  {
    id: "traces", label: "Traces", dot: "bg-pink-500",
    endpoints: [
      { id: "list-traces", method: "GET", path: "/v1/traces", description: "All recent LLM call traces.", queryParams: [{ name: "limit", type: "number", description: "Max traces (default 500)", example: "100" }], responseDesc: '{ traces: [{ trace_id, model_id, prompt_tokens, completion_tokens, latency_ms, cost_micros, is_error }] }' },
    ],
  },
  {
    id: "admin", label: "Admin", dot: "bg-red-500",
    endpoints: [
      { id: "audit-log",   method: "GET", path: "/v1/admin/audit-log", description: "Paginated audit log of administrative actions.", queryParams: [{ name: "limit", type: "number", description: "Max entries (default 100)", example: "100" }], responseDesc: "Array of audit log entries" },
      { id: "request-log", method: "GET", path: "/v1/admin/logs",      description: "Structured request log from the in-memory ring buffer.", queryParams: [{ name: "limit", type: "number", description: "Max entries (default 100)", example: "50" }, { name: "level", type: "string", description: "Filter by level (info/warn/error)" }], responseDesc: '{ entries: [{ timestamp, method, path, status, latency_ms }], total, limit }' },
    ],
  },
];

// ── Constants ─────────────────────────────────────────────────────────────────

const API_BASE = import.meta.env.VITE_API_URL ?? "";
const getToken = () =>
  localStorage.getItem("cairn_token") ?? import.meta.env.VITE_API_TOKEN ?? "";

const METHOD_STYLE: Record<HttpMethod, string> = {
  GET:    "bg-emerald-950/60 text-emerald-300 border-emerald-800/40",
  POST:   "bg-blue-950/60    text-blue-300    border-blue-800/40",
  DELETE: "bg-red-950/60     text-red-300     border-red-800/40",
  PUT:    "bg-amber-950/60   text-amber-300   border-amber-800/40",
};

const STATUS_COLOR = (s: number | null): string => {
  if (s === null) return "text-gray-400 dark:text-zinc-500";
  if (s >= 200 && s < 300) return "text-emerald-400";
  if (s >= 400) return "text-red-400";
  return "text-amber-400";
};

// ── Helpers ───────────────────────────────────────────────────────────────────

interface TryResult {
  status: number;
  data: unknown;
  latency: number;
}

async function sendRequest(
  ep: Endpoint,
  pathVals: Record<string, string>,
  queryVals: Record<string, string>,
  bodyText: string,
  logEntry: { id: string; add: (e: import("../components/RequestLogContext").RequestLogEntry) => void; update: (id: string, patch: Partial<import("../components/RequestLogContext").RequestLogEntry>) => void },
): Promise<TryResult> {
  let path = ep.path;
  for (const [k, v] of Object.entries(pathVals)) {
    path = path.replace(`:${k}`, encodeURIComponent(v));
  }
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(queryVals)) {
    if (v.trim()) qs.set(k, v.trim());
  }
  const url = `${API_BASE}${path}${qs.toString() ? `?${qs}` : ""}`;
  const token = getToken();
  const reqHeaders: Record<string, string> = {
    Authorization: `Bearer ${token}`,
    ...(ep.method !== "GET" ? { "Content-Type": "application/json" } : {}),
  };
  const reqBody = ep.method !== "GET" && bodyText.trim() ? bodyText : null;

  // Log the pending entry immediately.
  logEntry.add({
    id:         logEntry.id,
    timestamp:  Date.now(),
    method:     ep.method,
    url,
    path:       path + (qs.toString() ? `?${qs}` : ""),
    reqHeaders,
    reqBody,
    status:     null,
    resBody:    null,
    latency:    null,
    error:      null,
  });

  const t0 = Date.now();
  try {
    const resp = await fetch(url, {
      method: ep.method,
      headers: reqHeaders,
      body: reqBody ?? undefined,
    });
    const latency = Date.now() - t0;
    const text = await resp.text();
    let data: unknown;
    try { data = JSON.parse(text); } catch { data = text; }
    logEntry.update(logEntry.id, { status: resp.status, resBody: data, latency });
    return { status: resp.status, data, latency };
  } catch (e) {
    const msg = e instanceof Error ? e.message : "Request failed";
    logEntry.update(logEntry.id, { error: msg, latency: Date.now() - t0 });
    throw e;
  }
}

/** Build a curl command string for an endpoint + current form values. */
function buildCurl(
  ep: Endpoint,
  pathVals: Record<string, string>,
  queryVals: Record<string, string>,
  bodyText: string,
): string {
  let path = ep.path;
  for (const [k, v] of Object.entries(pathVals)) {
    path = path.replace(`:${k}`, encodeURIComponent(v || `:${k}`));
  }
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(queryVals)) {
    if (v.trim()) qs.set(k, v.trim());
  }
  const url = `${API_BASE || "http://localhost:3000"}${path}${qs.toString() ? `?${qs}` : ""}`;
  const token = getToken() || "<TOKEN>";

  const parts = [
    `curl -s -X ${ep.method}`,
    `  -H 'Authorization: Bearer ${token}'`,
    ...(ep.method !== "GET" ? ["  -H 'Content-Type: application/json'"] : []),
    ...(ep.method !== "GET" && bodyText.trim()
      ? [`  -d '${bodyText.replace(/'/g, "\\'")}'`]
      : []),
    `  '${url}'`,
  ];
  return parts.join(" \\\n");
}

// ── Shared atoms ──────────────────────────────────────────────────────────────

function MethodBadge({ method }: { method: HttpMethod | string }) {
  const style = METHOD_STYLE[method as HttpMethod] ?? "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-500 dark:text-zinc-400 border-gray-200 dark:border-zinc-700";
  return (
    <span className={clsx(
      "shrink-0 inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-mono font-semibold border",
      style,
    )}>
      {method}
    </span>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex items-center justify-center min-w-[1.25rem] h-4 rounded bg-gray-100 dark:bg-zinc-800 px-1 text-[9px] font-mono text-gray-500 dark:text-zinc-400 ring-1 ring-inset ring-gray-300 dark:ring-zinc-700">
      {children}
    </kbd>
  );
}

function ParamTable({ params }: { params: Param[] }) {
  return (
    <table className="min-w-full text-[12px]">
      <thead>
        <tr className="border-b border-gray-200 dark:border-zinc-800">
          <th className="py-1.5 pr-3 text-left text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider w-32">Name</th>
          <th className="py-1.5 pr-3 text-left text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider w-24">Type</th>
          <th className="py-1.5 text-left text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Description</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/40">
        {params.map((p) => (
          <tr key={p.name}>
            <td className="py-1.5 pr-3 font-mono text-indigo-300">
              {p.name}{p.required && <span className="ml-1 text-red-500">*</span>}
            </td>
            <td className="py-1.5 pr-3 font-mono text-gray-400 dark:text-zinc-500">{p.type}</td>
            <td className="py-1.5 text-gray-400 dark:text-zinc-500">
              {p.description}
              {p.example && <span className="ml-2 text-gray-300 dark:text-zinc-600">e.g. <code className="text-gray-400 dark:text-zinc-600">{p.example}</code></span>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Try-it panel ──────────────────────────────────────────────────────────────

function TryItPanel({ ep }: { ep: Endpoint }) {
  const defaultPath:  Record<string, string> = {};
  const defaultQuery: Record<string, string> = {};
  ep.pathParams?.forEach(p => { defaultPath[p.name]  = p.example ?? ""; });
  ep.queryParams?.forEach(p => { defaultQuery[p.name] = p.example ?? ""; });

  const [pathVals,  setPathVals]  = useState<Record<string, string>>(defaultPath);
  const [queryVals, setQueryVals] = useState<Record<string, string>>(defaultQuery);
  const [bodyText,  setBodyText]  = useState(ep.bodyExample ?? "");
  const [result,    setResult]    = useState<TryResult | null>(null);
  const [loading,   setLoading]   = useState(false);
  const [error,     setError]     = useState<string | null>(null);
  const [copied,    setCopied]    = useState(false);
  const [curlCopied, setCurlCopied] = useState(false);
  const [showCurl,  setShowCurl]  = useState(false);

  const { add, update } = useRequestLog();
  const curl = buildCurl(ep, pathVals, queryVals, bodyText);

  async function handleSend() {
    setLoading(true); setError(null); setResult(null);
    const id = `req-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    try {
      const res = await sendRequest(ep, pathVals, queryVals, bodyText, { id, add, update });
      setResult(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Request failed");
    } finally {
      setLoading(false);
    }
  }

  function copyCurl() {
    void navigator.clipboard.writeText(curl).then(() => {
      setCurlCopied(true);
      setTimeout(() => setCurlCopied(false), 1500);
    });
  }

  function copyResult() {
    if (!result) return;
    void navigator.clipboard.writeText(JSON.stringify(result.data, null, 2)).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }

  const statusColor = result ? STATUS_COLOR(result.status) : "";

  return (
    <div className="mt-3 rounded-lg border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 overflow-hidden">
      {/* Header: endpoint URL + curl toggle */}
      <div className="px-3 py-2 border-b border-gray-200 dark:border-zinc-800 flex items-center justify-between gap-3">
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider shrink-0">Try it</span>
        <code className="text-[10px] font-mono text-gray-300 dark:text-zinc-600 truncate flex-1">
          {API_BASE || "http://localhost:3000"}{ep.path}
        </code>
        <button
          onClick={() => setShowCurl(v => !v)}
          className={clsx(
            "flex items-center gap-1 text-[10px] rounded px-1.5 py-0.5 transition-colors shrink-0",
            showCurl ? "bg-gray-200 dark:bg-zinc-700 text-gray-700 dark:text-zinc-300" : "text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400",
          )}
        >
          <Terminal size={10} /> curl
        </button>
      </div>

      {/* Auth header display */}
      <div className="px-3 pt-2.5 pb-0">
        <p className="text-[10px] text-gray-400 dark:text-zinc-600 mb-1 uppercase tracking-wider">Authorization</p>
        <div className="flex items-center gap-2 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 px-2 py-1">
          <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono">Bearer</span>
          <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 truncate flex-1">
            {getToken() ? `${getToken().slice(0, 12)}…` : "no token stored"}
          </span>
          <span className="text-[9px] text-emerald-600 shrink-0">pre-filled</span>
        </div>
      </div>

      <div className="p-3 space-y-3">
        {/* Path params */}
        {ep.pathParams && ep.pathParams.length > 0 && (
          <div>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 mb-1.5 uppercase tracking-wider">Path Parameters</p>
            <div className="grid grid-cols-2 gap-2">
              {ep.pathParams.map(p => (
                <div key={p.name}>
                  <label className="text-[10px] text-gray-400 dark:text-zinc-500 block mb-1 font-mono">{p.name}</label>
                  <input value={pathVals[p.name] ?? ""} onChange={e => setPathVals(v => ({ ...v, [p.name]: e.target.value }))}
                    placeholder={p.example ?? p.name}
                    className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] text-gray-700 dark:text-zinc-300
                               font-mono px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors" />
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Query params */}
        {ep.queryParams && ep.queryParams.length > 0 && (
          <div>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 mb-1.5 uppercase tracking-wider">Query Parameters</p>
            <div className="grid grid-cols-2 gap-2">
              {ep.queryParams.map(p => (
                <div key={p.name}>
                  <label className="text-[10px] text-gray-400 dark:text-zinc-500 block mb-1 font-mono">
                    {p.name}{p.required && <span className="text-red-500 ml-0.5">*</span>}
                  </label>
                  <input value={queryVals[p.name] ?? ""} onChange={e => setQueryVals(v => ({ ...v, [p.name]: e.target.value }))}
                    placeholder={p.example ?? ""}
                    className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] text-gray-700 dark:text-zinc-300
                               font-mono px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors" />
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Body */}
        {ep.method !== "GET" && (
          <div>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 mb-1.5 uppercase tracking-wider">Request Body (JSON)</p>
            <textarea value={bodyText} onChange={e => setBodyText(e.target.value)}
              rows={Math.min(8, (bodyText.match(/\n/g)?.length ?? 0) + 2)} spellCheck={false}
              className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] text-gray-700 dark:text-zinc-300
                         font-mono px-3 py-2 resize-none focus:outline-none focus:border-indigo-500
                         transition-colors leading-relaxed" />
          </div>
        )}

        {/* Curl command */}
        {showCurl && (
          <div>
            <div className="flex items-center justify-between mb-1">
              <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">curl command</p>
              <button onClick={copyCurl}
                className="flex items-center gap-1 text-[10px] text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
                {curlCopied ? <Check size={10} className="text-emerald-400" /> : <Copy size={10} />}
                {curlCopied ? "Copied!" : "Copy"}
              </button>
            </div>
            <pre className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2 text-[10px]
                           font-mono text-gray-500 dark:text-zinc-400 overflow-x-auto leading-relaxed whitespace-pre-wrap">
              {curl}
            </pre>
          </div>
        )}

        {/* Send / SSE note */}
        <div className="flex items-center gap-3">
          <button onClick={handleSend} disabled={loading || ep.sse}
            className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                       bg-indigo-600 hover:bg-indigo-500 text-white
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors">
            {loading ? <><Loader2 size={11} className="animate-spin" /> Sending…</> : <><Send size={11} /> Send</>}
          </button>
          {ep.sse && <span className="text-[11px] text-gray-400 dark:text-zinc-600 italic">SSE: use curl or the Playground page.</span>}
          {!ep.sse && <span className="text-[10px] text-gray-300 dark:text-zinc-600"><Kbd>⌘</Kbd><Kbd>↵</Kbd> to send</span>}
        </div>

        {/* Error */}
        {error && (
          <div className="rounded bg-red-950/40 border border-red-800/40 px-3 py-2">
            <p className="text-[12px] text-red-400">{error}</p>
          </div>
        )}

        {/* Response */}
        {result && (
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <div className="flex items-center gap-3">
                <span className={clsx("font-mono text-[12px] font-semibold", statusColor)}>{result.status}</span>
                <span className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono">{result.latency}ms</span>
              </div>
              <button onClick={copyResult}
                className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
                {copied ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
            <pre className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2.5 text-[11px]
                           font-mono text-gray-700 dark:text-zinc-300 overflow-x-auto leading-relaxed max-h-64">
              {JSON.stringify(result.data, null, 2)}
            </pre>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Endpoint row ──────────────────────────────────────────────────────────────

function EndpointRow({ ep }: { ep: Endpoint }) {
  const [expanded, setExpanded] = useState(false);
  const [tryOpen,  setTryOpen]  = useState(false);

  return (
    <div className={clsx(
      "rounded-lg border transition-colors",
      expanded ? "border-gray-200 dark:border-zinc-700 bg-gray-50/60 dark:bg-zinc-900/60" : "border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 hover:border-gray-200 dark:border-zinc-700",
    )}>
      <button onClick={() => setExpanded(v => !v)}
        className="w-full flex items-center gap-3 px-4 py-3 text-left group">
        <MethodBadge method={ep.method} />
        <code className="flex-1 text-[13px] font-mono text-gray-800 dark:text-zinc-200 truncate">{ep.path}</code>
        {ep.sse && <span className="text-[10px] font-medium text-sky-400 bg-sky-950/60 border border-sky-800/40 rounded px-1.5 py-0.5 shrink-0">SSE</span>}
        <CopyButton
          text={`${API_BASE || "http://localhost:3000"}${ep.path}`}
          label="Copy endpoint URL"
          size={11}
          className="shrink-0 hidden group-hover:inline-flex"
        />
        <span className="text-[12px] text-gray-400 dark:text-zinc-500 truncate max-w-xs hidden md:block">{ep.description}</span>
        {expanded ? <ChevronDown size={13} className="text-gray-400 dark:text-zinc-500 shrink-0" /> : <ChevronRight size={13} className="text-gray-400 dark:text-zinc-600 shrink-0" />}
      </button>

      {expanded && (
        <div className="px-4 pb-4 space-y-4 border-t border-gray-200 dark:border-zinc-800">
          <p className="text-[13px] text-gray-500 dark:text-zinc-400 pt-3">{ep.description}</p>
          {ep.pathParams?.length  && <div><p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Path Parameters</p><ParamTable params={ep.pathParams} /></div>}
          {ep.queryParams?.length && <div><p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Query Parameters</p><ParamTable params={ep.queryParams} /></div>}
          {ep.bodyFields?.length  && <div><p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Request Body</p><ParamTable params={ep.bodyFields} /></div>}
          {ep.bodyExample && (
            <div>
              <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Example Body</p>
              <pre className="rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 py-2 text-[11px] font-mono text-gray-500 dark:text-zinc-400 overflow-x-auto">{ep.bodyExample}</pre>
            </div>
          )}
          <div>
            <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Response</p>
            <p className="text-[12px] font-mono text-gray-400 dark:text-zinc-500">{ep.responseDesc}</p>
          </div>
          <div>
            <button onClick={() => setTryOpen(v => !v)}
              className={clsx(
                "flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium transition-colors",
                tryOpen ? "bg-indigo-600/20 text-indigo-400 border border-indigo-700/40" : "bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 hover:bg-gray-200 dark:hover:bg-zinc-700 hover:text-gray-800 dark:hover:text-zinc-200",
              )}>
              {tryOpen ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
              Try it
            </button>
            {tryOpen && <TryItPanel ep={ep} />}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Recent Requests panel ─────────────────────────────────────────────────────

function fmtRelative(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000) return `${Math.floor(d / 1000)}s ago`;
  return `${Math.floor(d / 60_000)}m ago`;
}

function RequestLogPanel() {
  const { entries, clear } = useRequestLog();
  const [expanded, setExpanded] = useState<string | null>(null);
  const [copied, setCopied]     = useState<string | null>(null);

  function copyEntry(id: string, text: string) {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(id);
      setTimeout(() => setCopied(null), 1500);
    });
  }

  if (entries.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 gap-3 text-gray-300 dark:text-zinc-600">
        <Clock size={24} />
        <p className="text-[13px]">No requests yet</p>
        <p className="text-[11px] text-center max-w-xs">
          Use the "Try it" panels in any endpoint to send requests.<br />
          They appear here automatically.
        </p>
      </div>
    );
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <span className="text-[11px] text-gray-400 dark:text-zinc-600">{entries.length} request{entries.length !== 1 ? "s" : ""}</span>
        <button onClick={clear}
          className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-red-400 transition-colors">
          <Trash2 size={11} /> Clear
        </button>
      </div>

      <div className="space-y-1.5">
        {entries.map(e => {
          const isOpen = expanded === e.id;
          const isCopied = copied === e.id;
          return (
            <div key={e.id} className={clsx("rounded-lg border overflow-hidden transition-colors",
              isOpen ? "border-gray-200 dark:border-zinc-700" : "border-gray-200 dark:border-zinc-800 hover:border-gray-200 dark:border-zinc-700")}>
              {/* Row */}
              <button onClick={() => setExpanded(isOpen ? null : e.id)}
                className="w-full flex items-center gap-3 px-3 py-2 text-left">
                <MethodBadge method={e.method as HttpMethod} />
                <code className="flex-1 text-[12px] font-mono text-gray-700 dark:text-zinc-300 truncate">{e.path}</code>
                {e.status !== null ? (
                  <span className={clsx("text-[12px] font-mono font-semibold shrink-0", STATUS_COLOR(e.status))}>{e.status}</span>
                ) : e.error ? (
                  <span className="text-[11px] text-red-400 shrink-0">error</span>
                ) : (
                  <Loader2 size={11} className="animate-spin text-gray-400 dark:text-zinc-600 shrink-0" />
                )}
                {e.latency !== null && (
                  <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono tabular-nums shrink-0">{e.latency}ms</span>
                )}
                <span className="text-[10px] text-gray-300 dark:text-zinc-600 shrink-0">{fmtRelative(e.timestamp)}</span>
                {isOpen ? <ChevronDown size={11} className="text-gray-400 dark:text-zinc-600 shrink-0" /> : <ChevronRight size={11} className="text-gray-300 dark:text-zinc-600 shrink-0" />}
              </button>

              {/* Expanded detail */}
              {isOpen && (
                <div className="border-t border-gray-200 dark:border-zinc-800 p-3 space-y-3 bg-white dark:bg-zinc-950">
                  {/* Request headers */}
                  <div>
                    <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1.5">Request Headers</p>
                    <pre className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2 text-[10px] font-mono text-gray-500 dark:text-zinc-400 overflow-x-auto">
                      {Object.entries(e.reqHeaders).map(([k, v]) =>
                        `${k}: ${k.toLowerCase() === 'authorization' ? `Bearer ${v.replace(/^Bearer /, '').slice(0, 8)}…` : v}`
                      ).join('\n')}
                    </pre>
                  </div>

                  {/* Request body */}
                  {e.reqBody && (
                    <div>
                      <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1.5">Request Body</p>
                      <pre className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2 text-[10px] font-mono text-gray-500 dark:text-zinc-400 overflow-x-auto max-h-32">
                        {(() => { try { return JSON.stringify(JSON.parse(e.reqBody), null, 2); } catch { return e.reqBody; } })()}
                      </pre>
                    </div>
                  )}

                  {/* Response */}
                  {(e.resBody !== null || e.error) && (
                    <div>
                      <div className="flex items-center justify-between mb-1.5">
                        <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">
                          Response {e.status !== null && <span className={clsx("ml-1", STATUS_COLOR(e.status))}>{e.status}</span>}
                          {e.latency !== null && <span className="ml-2 text-gray-300 dark:text-zinc-600">{e.latency}ms</span>}
                        </p>
                        {e.resBody != null && (
                          <button onClick={() => copyEntry(e.id, JSON.stringify(e.resBody as object, null, 2))}
                            className="flex items-center gap-1 text-[10px] text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
                            {isCopied ? <Check size={10} className="text-emerald-400" /> : <Copy size={10} />}
                            {isCopied ? "Copied" : "Copy"}
                          </button>
                        )}
                      </div>
                      {e.error ? (
                        <div className="rounded bg-red-950/40 border border-red-800/40 px-3 py-2">
                          <p className="text-[12px] text-red-400">{e.error}</p>
                        </div>
                      ) : (
                        <pre className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2 text-[10px] font-mono text-gray-700 dark:text-zinc-300 overflow-x-auto leading-relaxed max-h-48">
                          {JSON.stringify(e.resBody, null, 2)}
                        </pre>
                      )}
                    </div>
                  )}

                  {/* Full URL */}
                  <div>
                    <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Full URL</p>
                    <code className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 break-all">{e.url}</code>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

type PageTab = 'reference' | 'requests';

export function ApiDocsPage() {
  const [pageTab,      setPageTab]      = useState<PageTab>('reference');
  const [activeDomain, setActiveDomain] = useState(DOMAINS[0].id);
  const [search,       setSearch]       = useState("");
  const { entries }                     = useRequestLog();

  const domain = DOMAINS.find(d => d.id === activeDomain) ?? DOMAINS[0];

  const filtered = search.trim()
    ? DOMAINS.flatMap(d =>
        d.endpoints
          .filter(e =>
            e.path.toLowerCase().includes(search.toLowerCase()) ||
            e.description.toLowerCase().includes(search.toLowerCase()) ||
            e.method.toLowerCase().includes(search.toLowerCase()),
          )
          .map(e => ({ ...e, _domain: d.label }))
      )
    : null;

  const totalEndpoints = DOMAINS.reduce((s, d) => s + d.endpoints.length, 0);
  const unreadReqs = entries.filter(e => e.status !== null || e.error).length;

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <FileCode2 size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">API Reference</span>
        <span className="text-[10px] text-gray-300 dark:text-zinc-600">{totalEndpoints} endpoints</span>

        {/* Page tabs */}
        <div className="flex items-center gap-0 ml-4 border-b border-transparent -mb-px">
          {(['reference', 'requests'] as PageTab[]).map(tab => (
            <button key={tab} onClick={() => setPageTab(tab)}
              className={clsx(
                "px-3 h-11 text-[11px] font-medium transition-colors border-b-2 capitalize flex items-center gap-1.5",
                pageTab === tab ? "text-gray-900 dark:text-zinc-100 border-indigo-500" : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
              )}>
              {tab}
              {tab === 'requests' && unreadReqs > 0 && (
                <span className="text-[9px] bg-indigo-600/30 text-indigo-400 rounded-full px-1.5 py-0.5 font-bold">
                  {unreadReqs}
                </span>
              )}
            </button>
          ))}
        </div>

        {pageTab === 'reference' && (
          <div className="ml-auto relative w-56">
            <input value={search} onChange={e => setSearch(e.target.value)}
              placeholder="Search endpoints…"
              className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] text-gray-700 dark:text-zinc-300
                         placeholder-zinc-600 px-3 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors" />
          </div>
        )}
      </div>

      {/* Body */}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {pageTab === 'reference' ? (
          <>
            {/* Domain sidebar */}
            {!search && (
              <div className="w-[180px] shrink-0 border-r border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 overflow-y-auto py-2">
                {DOMAINS.map(d => (
                  <button key={d.id} onClick={() => setActiveDomain(d.id)}
                    className={clsx(
                      "w-full flex items-center gap-2.5 px-3 py-2 text-left transition-colors text-[12px]",
                      d.id === activeDomain ? "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-800 dark:text-zinc-200" : "text-gray-400 dark:text-zinc-500 hover:bg-gray-50/60 dark:bg-zinc-900/60 hover:text-gray-700 dark:hover:text-zinc-300",
                    )}>
                    <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", d.dot)} />
                    {d.label}
                    <span className="ml-auto text-[10px] text-gray-300 dark:text-zinc-600">{d.endpoints.length}</span>
                  </button>
                ))}
              </div>
            )}

            {/* Endpoint list */}
            <div className="flex-1 overflow-y-auto p-5">
              {filtered ? (
                <div className="space-y-3 max-w-3xl">
                  <p className="text-[11px] text-gray-400 dark:text-zinc-600 mb-4">{filtered.length} result{filtered.length !== 1 ? "s" : ""} for &ldquo;{search}&rdquo;</p>
                  {filtered.length === 0 ? (
                    <p className="text-[13px] text-gray-400 dark:text-zinc-600 italic py-8 text-center">No endpoints match.</p>
                  ) : (
                    filtered.map(ep => (
                      <div key={ep.id}>
                        <p className="text-[10px] text-gray-300 dark:text-zinc-600 mb-1.5 font-medium uppercase tracking-wider">{(ep as typeof ep & {_domain: string})._domain}</p>
                        <EndpointRow ep={ep} />
                      </div>
                    ))
                  )}
                </div>
              ) : (
                <div className="max-w-3xl space-y-3">
                  <div className="flex items-center gap-3 mb-4">
                    <span className={clsx("w-2 h-2 rounded-full", domain.dot)} />
                    <h2 className="text-[14px] font-semibold text-gray-800 dark:text-zinc-200">{domain.label}</h2>
                    <span className="text-[11px] text-gray-400 dark:text-zinc-600">{domain.endpoints.length} endpoint{domain.endpoints.length !== 1 ? "s" : ""}</span>
                  </div>
                  {domain.endpoints.map(ep => <EndpointRow key={ep.id} ep={ep} />)}
                </div>
              )}
            </div>
          </>
        ) : (
          /* Recent Requests panel */
          <div className="flex-1 overflow-y-auto p-5 max-w-3xl mx-auto w-full">
            <RequestLogPanel />
          </div>
        )}
      </div>
    </div>
  );
}

export default ApiDocsPage;
