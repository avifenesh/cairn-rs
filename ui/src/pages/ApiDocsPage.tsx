import { useState } from "react";
import {
  FileCode2, ChevronRight, ChevronDown, Send, Loader2,
  Copy, Check,
} from "lucide-react";
import { clsx } from "clsx";

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
      {
        id: "get-health", method: "GET", path: "/health",
        description: "Liveness probe. Returns immediately with ok=true when the server is up.",
        responseDesc: '{ "ok": true }', auth: false,
      },
      {
        id: "get-status", method: "GET", path: "/v1/status",
        description: "Runtime and store health with uptime in seconds.",
        responseDesc: '{ "runtime_ok": true, "store_ok": true, "uptime_secs": 3600 }',
      },
      {
        id: "get-overview", method: "GET", path: "/v1/overview",
        description: "Combined deployment mode, store backend, and health summary.",
        responseDesc: '{ "deployment_mode": "local", "store_backend": "memory", "uptime_secs": 3600, "runtime_ok": true, "store_ok": true }',
      },
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
        responseDesc: "Array of SessionRecord — [{ session_id, project, state, version, created_at, updated_at }]",
      },
      {
        id: "create-session", method: "POST", path: "/v1/sessions",
        description: "Create a new conversation session. All fields are optional; defaults are auto-assigned.",
        bodyFields: [
          { name: "tenant_id",    type: "string", description: "Tenant identifier",    example: "default_tenant"    },
          { name: "workspace_id", type: "string", description: "Workspace identifier", example: "default_workspace" },
          { name: "project_id",   type: "string", description: "Project identifier",   example: "default_project"   },
          { name: "session_id",   type: "string", description: "Override the auto-generated session ID" },
        ],
        bodyExample: '{\n  "tenant_id": "default_tenant",\n  "workspace_id": "default_workspace",\n  "project_id": "default_project"\n}',
        responseDesc: "SessionRecord — the newly created session",
      },
      {
        id: "get-session-traces", method: "GET", path: "/v1/sessions/:id/llm-traces",
        description: "LLM call traces scoped to a single session.",
        pathParams: [{ name: "id", type: "string", description: "Session ID", example: "sess_..." }],
        queryParams: [{ name: "limit", type: "number", description: "Max traces (default 200)", example: "200" }],
        responseDesc: '{ "traces": [{ trace_id, model_id, prompt_tokens, completion_tokens, latency_ms, cost_micros, is_error }] }',
      },
    ],
  },
  {
    id: "runs", label: "Runs", dot: "bg-indigo-500",
    endpoints: [
      {
        id: "list-runs", method: "GET", path: "/v1/runs",
        description: "List runs, most recent first. Optionally filter by project.",
        queryParams: [
          { name: "limit",        type: "number", description: "Max results (default 200)", example: "50"     },
          { name: "offset",       type: "number", description: "Pagination offset",          example: "0"      },
          { name: "tenant_id",    type: "string", description: "Filter by tenant"                              },
          { name: "workspace_id", type: "string", description: "Filter by workspace"                           },
          { name: "project_id",   type: "string", description: "Filter by project"                             },
        ],
        responseDesc: "Array of RunRecord — [{ run_id, session_id, parent_run_id, project, state, failure_class, ... }]",
      },
      {
        id: "create-run", method: "POST", path: "/v1/runs",
        description: "Start a new agent run inside a session.",
        bodyFields: [
          { name: "session_id",   type: "string", description: "Session to attach this run to"            },
          { name: "parent_run_id",type: "string", description: "Parent run for sub-agent hierarchies"      },
          { name: "tenant_id",    type: "string", description: "Tenant override"                           },
          { name: "workspace_id", type: "string", description: "Workspace override"                        },
          { name: "project_id",   type: "string", description: "Project override"                          },
        ],
        bodyExample: '{}',
        responseDesc: "RunRecord — the newly created run",
      },
      {
        id: "get-run", method: "GET", path: "/v1/runs/:id",
        description: "Fetch a single run by its ID.",
        pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }],
        responseDesc: "RunRecord",
      },
      {
        id: "get-run-events", method: "GET", path: "/v1/runs/:id/events",
        description: "Event timeline entries for a run, ordered by position.",
        pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }],
        queryParams: [{ name: "limit", type: "number", description: "Max events (default 100)", example: "100" }],
        responseDesc: "Array of RunEventSummary — [{ position, stored_at, event_type }]",
      },
      {
        id: "get-run-tasks", method: "GET", path: "/v1/runs/:id/tasks",
        description: "Tasks that belong to a run.",
        pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }],
        responseDesc: "Array of TaskRecord",
      },
      {
        id: "get-run-cost", method: "GET", path: "/v1/runs/:id/cost",
        description: "Accumulated cost and token usage for a run.",
        pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }],
        responseDesc: "RunCostRecord — { run_id, total_cost_micros, total_tokens_in, total_tokens_out, provider_calls }",
      },
      {
        id: "pause-run", method: "POST", path: "/v1/runs/:id/pause",
        description: "Pause a running or pending run.",
        pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }],
        bodyFields: [{ name: "detail", type: "string", description: "Human-readable pause reason" }],
        bodyExample: '{ "detail": "Manual pause for inspection" }',
        responseDesc: "Updated RunRecord with state=paused",
      },
      {
        id: "resume-run", method: "POST", path: "/v1/runs/:id/resume",
        description: "Resume a paused run.",
        pathParams: [{ name: "id", type: "string", description: "Run ID", example: "run_..." }],
        bodyExample: '{}',
        responseDesc: "Updated RunRecord with state=running",
      },
    ],
  },
  {
    id: "tasks", label: "Tasks", dot: "bg-violet-500",
    endpoints: [
      {
        id: "list-tasks", method: "GET", path: "/v1/tasks",
        description: "All tasks across every project (operator view).",
        queryParams: [
          { name: "limit",  type: "number", description: "Max results (default 500)", example: "100" },
          { name: "offset", type: "number", description: "Pagination offset",          example: "0"   },
        ],
        responseDesc: "Array of TaskRecord — [{ task_id, project, parent_run_id, state, lease_owner, lease_expires_at, ... }]",
      },
      {
        id: "claim-task", method: "POST", path: "/v1/tasks/:id/claim",
        description: "Claim a queued task for a worker. Sets state to 'leased'.",
        pathParams: [{ name: "id", type: "string", description: "Task ID", example: "task_..." }],
        bodyFields: [
          { name: "worker_id",        type: "string", required: true,  description: "Worker identifier"                    },
          { name: "lease_duration_ms", type: "number", description: "Lease duration in ms (default 30 000)", example: "30000" },
        ],
        bodyExample: '{\n  "worker_id": "operator",\n  "lease_duration_ms": 30000\n}',
        responseDesc: "Updated TaskRecord with state=leased",
      },
      {
        id: "release-task", method: "POST", path: "/v1/tasks/:id/release-lease",
        description: "Release a leased task back to queued state.",
        pathParams: [{ name: "id", type: "string", description: "Task ID", example: "task_..." }],
        bodyExample: '{}',
        responseDesc: "Updated TaskRecord with state=queued",
      },
    ],
  },
  {
    id: "approvals", label: "Approvals", dot: "bg-amber-500",
    endpoints: [
      {
        id: "list-approvals", method: "GET", path: "/v1/approvals/pending",
        description: "List pending (unresolved) approvals for the operator inbox.",
        queryParams: [
          { name: "tenant_id",    type: "string", description: "Filter by tenant"    },
          { name: "workspace_id", type: "string", description: "Filter by workspace" },
          { name: "project_id",   type: "string", description: "Filter by project"   },
        ],
        responseDesc: "Array of ApprovalRecord — [{ approval_id, project, run_id, task_id, requirement, decision, created_at }]",
      },
      {
        id: "resolve-approval", method: "POST", path: "/v1/approvals/:id/resolve",
        description: "Approve or reject a pending approval gate.",
        pathParams: [{ name: "id", type: "string", description: "Approval ID", example: "appr_..." }],
        bodyFields: [
          { name: "decision", type: '"approved" | "rejected"', required: true, description: "Approval decision" },
        ],
        bodyExample: '{ "decision": "approved" }',
        responseDesc: "Updated ApprovalRecord with decision set",
      },
    ],
  },
  {
    id: "costs", label: "Costs", dot: "bg-teal-500",
    endpoints: [
      {
        id: "get-costs", method: "GET", path: "/v1/costs",
        description: "Aggregate token and cost totals across all provider calls.",
        responseDesc: "CostSummary — { total_provider_calls, total_tokens_in, total_tokens_out, total_cost_micros }",
      },
      {
        id: "get-dashboard", method: "GET", path: "/v1/dashboard",
        description: "Operator overview: active runs, tasks, approvals, cost, health, and recent critical events.",
        responseDesc: "DashboardOverview — { active_runs, active_tasks, pending_approvals, failed_runs_24h, system_healthy, degraded_components, ... }",
      },
      {
        id: "get-stats", method: "GET", path: "/v1/stats",
        description: "Real-time system-wide counters, updated continuously.",
        responseDesc: "SystemStats — { total_events, total_sessions, total_runs, total_tasks, active_runs, pending_approvals, uptime_seconds }",
      },
    ],
  },
  {
    id: "settings", label: "Settings", dot: "bg-zinc-500",
    endpoints: [
      {
        id: "get-settings", method: "GET", path: "/v1/settings",
        description: "Full deployment configuration: mode, store backend, plugin count, health, and key management status.",
        responseDesc: "DeploymentSettings — { deployment_mode, store_backend, plugin_count, system_health: { ... }, key_management: { ... } }",
      },
      {
        id: "get-settings-tls", method: "GET", path: "/v1/settings/tls",
        description: "TLS certificate details for the server.",
        responseDesc: "TLS certificate info object",
      },
    ],
  },
  {
    id: "providers", label: "Providers", dot: "bg-orange-500",
    endpoints: [
      {
        id: "get-provider-health", method: "GET", path: "/v1/providers/health",
        description: "Health records for all registered LLM providers.",
        responseDesc: "Array of provider health records",
      },
      {
        id: "get-ollama-models", method: "GET", path: "/v1/providers/ollama/models",
        description: "List locally available Ollama models.",
        responseDesc: '{ "host": "http://localhost:11434", "models": ["llama3.2:latest", ...], "count": 3 }',
      },
      {
        id: "get-ollama-model-info", method: "GET", path: "/v1/providers/ollama/models/:name/info",
        description: "Detailed model metadata (family, quantization, context length, size).",
        pathParams: [{ name: "name", type: "string", description: "Model name", example: "llama3.2:latest" }],
        responseDesc: '{ "name", "family", "format", "parameter_size", "quantization_level", "context_length", "size_human" }',
      },
      {
        id: "pull-ollama-model", method: "POST", path: "/v1/providers/ollama/pull",
        description: "Pull (download) a model into the local Ollama registry.",
        bodyFields: [
          { name: "model", type: "string", required: true, description: "Model tag to pull", example: "llama3.2:latest" },
        ],
        bodyExample: '{ "model": "llama3.2:latest" }',
        responseDesc: '{ "status": "ok", "model": "llama3.2:latest" }',
      },
      {
        id: "delete-ollama-model", method: "POST", path: "/v1/providers/ollama/delete",
        description: "Remove a model from the local Ollama registry.",
        bodyFields: [
          { name: "model", type: "string", required: true, description: "Model tag to remove", example: "llama3.2:latest" },
        ],
        bodyExample: '{ "model": "llama3.2:latest" }',
        responseDesc: '{ "status": "deleted", "model": "llama3.2:latest" }',
      },
      {
        id: "ollama-generate", method: "POST", path: "/v1/providers/ollama/generate",
        description: "Non-streaming single-turn generation via Ollama.",
        bodyFields: [
          { name: "prompt", type: "string", required: true, description: "User prompt",                         example: "Hello!" },
          { name: "model",  type: "string",                 description: "Model override (default: first available)" },
        ],
        bodyExample: '{\n  "prompt": "What is 2 + 2?",\n  "model": "llama3.2:latest"\n}',
        responseDesc: '{ "text": "4.", "model": "llama3.2:latest", "tokens_in": 12, "tokens_out": 3, "latency_ms": 842 }',
      },
      {
        id: "ollama-stream", method: "POST", path: "/v1/providers/ollama/stream",
        description: "Server-sent events streaming generation via Ollama. Streams `data: {text}` events followed by a `data: {latency_ms, model}` done event.",
        sse: true,
        bodyFields: [
          { name: "model",       type: "string", required: true,  description: "Model to use"                            },
          { name: "messages",    type: "array",  required: true,  description: "Chat history with system/user/assistant" },
          { name: "temperature", type: "number",                  description: "Sampling temperature (0–2)",  example: "0.7" },
          { name: "max_tokens",  type: "number",                  description: "Maximum tokens to generate",  example: "2048" },
        ],
        bodyExample: '{\n  "model": "llama3.2:latest",\n  "messages": [\n    { "role": "user", "content": "Hello!" }\n  ],\n  "temperature": 0.7,\n  "max_tokens": 512\n}',
        responseDesc: 'SSE stream — data: {"text": "..."} ... data: {"latency_ms": 841, "model": "llama3.2:latest"}',
      },
    ],
  },
  {
    id: "memory", label: "Memory", dot: "bg-purple-500",
    endpoints: [
      {
        id: "memory-search", method: "GET", path: "/v1/memory/search",
        description: "Lexical retrieval over the knowledge store with scoring breakdown.",
        queryParams: [
          { name: "query_text",   type: "string", required: true,  description: "Search query",                   example: "agent" },
          { name: "tenant_id",    type: "string",                  description: "Tenant scope",                   example: "default_tenant"    },
          { name: "workspace_id", type: "string",                  description: "Workspace scope",                example: "default_workspace" },
          { name: "project_id",   type: "string",                  description: "Project scope",                  example: "default_project"   },
          { name: "limit",        type: "number",                  description: "Max results (default 10)",        example: "5" },
        ],
        responseDesc: '{ "results": [{ score, chunk: { chunk_id, text, ... }, breakdown: { lexical_relevance, freshness, source_credibility } }], "diagnostics": {...} }',
      },
      {
        id: "list-sources", method: "GET", path: "/v1/sources",
        description: "List registered signal/document sources.",
        queryParams: [
          { name: "tenant_id",    type: "string", description: "Filter by tenant"    },
          { name: "workspace_id", type: "string", description: "Filter by workspace" },
          { name: "project_id",   type: "string", description: "Filter by project"   },
        ],
        responseDesc: "Array of SourceRecord — [{ source_id, document_count, avg_quality_score, last_ingested_at_ms }]",
      },
      {
        id: "source-quality", method: "GET", path: "/v1/sources/:id/quality",
        description: "Quality metrics for a specific source: credibility score, retrieval count, average rating.",
        pathParams: [{ name: "id", type: "string", description: "Source ID", example: "src_..." }],
        responseDesc: "SourceQualityRecord — { source_id, credibility_score, total_retrievals, avg_rating, chunk_count }",
      },
    ],
  },
  {
    id: "events", label: "Events", dot: "bg-sky-500",
    endpoints: [
      {
        id: "events-stream", method: "GET", path: "/v1/events/stream",
        description: "Server-sent events stream of all runtime events. Supports Last-Event-ID replay from a ring buffer (last 10 000 events).",
        sse: true,
        responseDesc: "SSE stream — event: <type>\\ndata: <json>\\nid: <seq>\\n\\n",
      },
      {
        id: "events-recent", method: "GET", path: "/v1/events/recent",
        description: "Fetch the most recent N runtime events for seeding the event log on page load.",
        queryParams: [
          { name: "limit", type: "number", description: "Max events (default 50)", example: "50" },
        ],
        responseDesc: "Array of RecentEvent — [{ seq, event_type, data, timestamp }]",
      },
    ],
  },
  {
    id: "traces", label: "Traces", dot: "bg-pink-500",
    endpoints: [
      {
        id: "list-traces", method: "GET", path: "/v1/traces",
        description: "All recent LLM call traces — model, tokens, latency, cost, error flag.",
        queryParams: [
          { name: "limit", type: "number", description: "Max traces (default 500)", example: "100" },
        ],
        responseDesc: '{ "traces": [{ trace_id, model_id, prompt_tokens, completion_tokens, latency_ms, cost_micros, session_id, run_id, created_at_ms, is_error }] }',
      },
    ],
  },
  {
    id: "admin", label: "Admin", dot: "bg-red-500",
    endpoints: [
      {
        id: "audit-log", method: "GET", path: "/v1/admin/audit-log",
        description: "Paginated audit log of administrative actions.",
        queryParams: [
          { name: "limit",  type: "number", description: "Max entries (default 100)", example: "100" },
          { name: "offset", type: "number", description: "Pagination offset",          example: "0"   },
        ],
        responseDesc: "Array of audit log entries",
      },
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
  const t0 = Date.now();
  const resp = await fetch(url, {
    method: ep.method,
    headers: {
      Authorization: `Bearer ${getToken()}`,
      ...(ep.method !== "GET" ? { "Content-Type": "application/json" } : {}),
    },
    body: ep.method !== "GET" && bodyText.trim() ? bodyText : undefined,
  });
  const latency = Date.now() - t0;
  const text = await resp.text();
  let data: unknown;
  try { data = JSON.parse(text); } catch { data = text; }
  return { status: resp.status, data, latency };
}

// ── Method badge ──────────────────────────────────────────────────────────────

function MethodBadge({ method }: { method: HttpMethod }) {
  return (
    <span className={clsx(
      "shrink-0 inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-mono font-semibold border",
      METHOD_STYLE[method],
    )}>
      {method}
    </span>
  );
}

// ── Param table ───────────────────────────────────────────────────────────────

function ParamTable({ params }: { params: Param[] }) {
  return (
    <table className="min-w-full text-[12px]">
      <thead>
        <tr className="border-b border-zinc-800">
          <th className="py-1.5 pr-3 text-left text-[10px] font-semibold text-zinc-600 uppercase tracking-wider w-32">Name</th>
          <th className="py-1.5 pr-3 text-left text-[10px] font-semibold text-zinc-600 uppercase tracking-wider w-24">Type</th>
          <th className="py-1.5 text-left text-[10px] font-semibold text-zinc-600 uppercase tracking-wider">Description</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-zinc-800/40">
        {params.map((p) => (
          <tr key={p.name}>
            <td className="py-1.5 pr-3 font-mono text-indigo-300">
              {p.name}
              {p.required && <span className="ml-1 text-red-500">*</span>}
            </td>
            <td className="py-1.5 pr-3 font-mono text-zinc-500">{p.type}</td>
            <td className="py-1.5 text-zinc-500">
              {p.description}
              {p.example && (
                <span className="ml-2 text-zinc-700">
                  e.g. <code className="text-zinc-600">{p.example}</code>
                </span>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Try-it form ───────────────────────────────────────────────────────────────

function TryItPanel({ ep }: { ep: Endpoint }) {
  const defaultPath: Record<string, string> = {};
  ep.pathParams?.forEach((p) => { defaultPath[p.name] = p.example ?? ""; });
  const defaultQuery: Record<string, string> = {};
  ep.queryParams?.forEach((p) => { defaultQuery[p.name] = p.example ?? ""; });

  const [pathVals,  setPathVals]  = useState<Record<string, string>>(defaultPath);
  const [queryVals, setQueryVals] = useState<Record<string, string>>(defaultQuery);
  const [bodyText,  setBodyText]  = useState(ep.bodyExample ?? "");
  const [result,    setResult]    = useState<TryResult | null>(null);
  const [loading,   setLoading]   = useState(false);
  const [error,     setError]     = useState<string | null>(null);
  const [copied,    setCopied]    = useState(false);

  async function handleSend() {
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const res = await sendRequest(ep, pathVals, queryVals, bodyText);
      setResult(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Request failed");
    } finally {
      setLoading(false);
    }
  }

  function handleCopy() {
    if (!result) return;
    void navigator.clipboard.writeText(JSON.stringify(result.data, null, 2)).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }

  const statusColor =
    result && result.status >= 200 && result.status < 300 ? "text-emerald-400" :
    result && result.status >= 400 ? "text-red-400" : "text-amber-400";

  return (
    <div className="mt-3 rounded-lg border border-zinc-800 bg-zinc-950 overflow-hidden">
      <div className="px-3 py-2 border-b border-zinc-800 flex items-center justify-between">
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Try it</span>
        <div className="font-mono text-[11px] text-zinc-700">
          {API_BASE || "http://localhost:3000"}{ep.path}
        </div>
      </div>

      <div className="p-3 space-y-3">
        {/* Path params */}
        {ep.pathParams && ep.pathParams.length > 0 && (
          <div>
            <p className="text-[10px] text-zinc-600 mb-1.5 uppercase tracking-wider">Path Parameters</p>
            <div className="grid grid-cols-2 gap-2">
              {ep.pathParams.map((p) => (
                <div key={p.name}>
                  <label className="text-[10px] text-zinc-500 block mb-1 font-mono">{p.name}</label>
                  <input
                    value={pathVals[p.name] ?? ""}
                    onChange={(e) => setPathVals((v) => ({ ...v, [p.name]: e.target.value }))}
                    placeholder={p.example ?? p.name}
                    className="w-full rounded border border-zinc-800 bg-zinc-900 text-[12px] text-zinc-300
                               font-mono px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors"
                  />
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Query params */}
        {ep.queryParams && ep.queryParams.length > 0 && (
          <div>
            <p className="text-[10px] text-zinc-600 mb-1.5 uppercase tracking-wider">Query Parameters</p>
            <div className="grid grid-cols-2 gap-2">
              {ep.queryParams.map((p) => (
                <div key={p.name}>
                  <label className="text-[10px] text-zinc-500 block mb-1 font-mono">
                    {p.name}
                    {p.required && <span className="text-red-500 ml-0.5">*</span>}
                  </label>
                  <input
                    value={queryVals[p.name] ?? ""}
                    onChange={(e) => setQueryVals((v) => ({ ...v, [p.name]: e.target.value }))}
                    placeholder={p.example ?? ""}
                    className="w-full rounded border border-zinc-800 bg-zinc-900 text-[12px] text-zinc-300
                               font-mono px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors"
                  />
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Body */}
        {ep.method !== "GET" && (
          <div>
            <p className="text-[10px] text-zinc-600 mb-1.5 uppercase tracking-wider">Request Body (JSON)</p>
            <textarea
              value={bodyText}
              onChange={(e) => setBodyText(e.target.value)}
              rows={Math.min(8, (bodyText.match(/\n/g)?.length ?? 0) + 2)}
              spellCheck={false}
              className="w-full rounded border border-zinc-800 bg-zinc-900 text-[12px] text-zinc-300
                         font-mono px-3 py-2 resize-none focus:outline-none focus:border-indigo-500
                         transition-colors leading-relaxed"
            />
          </div>
        )}

        {/* Send button */}
        <div className="flex items-center gap-3">
          <button
            onClick={handleSend}
            disabled={loading || ep.sse}
            className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                       bg-indigo-600 hover:bg-indigo-500 text-white
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {loading
              ? <><Loader2 size={11} className="animate-spin" /> Sending…</>
              : <><Send size={11} /> Send</>}
          </button>
          {ep.sse && (
            <span className="text-[11px] text-zinc-600 italic">
              SSE streams can't be tested here — use the Playground page or curl.
            </span>
          )}
        </div>

        {/* Response */}
        {error && (
          <div className="rounded bg-red-950/40 border border-red-800/40 px-3 py-2">
            <p className="text-[12px] text-red-400">{error}</p>
          </div>
        )}
        {result && (
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <div className="flex items-center gap-3">
                <span className={clsx("font-mono text-[12px] font-semibold", statusColor)}>
                  {result.status}
                </span>
                <span className="text-[11px] text-zinc-600 font-mono">{result.latency}ms</span>
              </div>
              <button onClick={handleCopy}
                className="flex items-center gap-1 text-[11px] text-zinc-600 hover:text-zinc-300 transition-colors">
                {copied ? <Check size={11} className="text-emerald-400" /> : <Copy size={11} />}
                {copied ? "Copied" : "Copy"}
              </button>
            </div>
            <pre className="rounded bg-zinc-900 border border-zinc-800 px-3 py-2.5 text-[11px]
                           font-mono text-zinc-300 overflow-x-auto leading-relaxed max-h-64">
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
      expanded ? "border-zinc-700 bg-zinc-900/60" : "border-zinc-800 bg-zinc-900 hover:border-zinc-700",
    )}>
      {/* Header row */}
      <button
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-3 px-4 py-3 text-left"
      >
        <MethodBadge method={ep.method} />
        <code className="flex-1 text-[13px] font-mono text-zinc-200 truncate">{ep.path}</code>
        {ep.sse && (
          <span className="text-[10px] font-medium text-sky-400 bg-sky-950/60 border border-sky-800/40 rounded px-1.5 py-0.5 shrink-0">
            SSE
          </span>
        )}
        <span className="text-[12px] text-zinc-500 truncate max-w-xs hidden md:block">{ep.description}</span>
        {expanded
          ? <ChevronDown size={13} className="text-zinc-500 shrink-0" />
          : <ChevronRight size={13} className="text-zinc-600 shrink-0" />
        }
      </button>

      {/* Expanded content */}
      {expanded && (
        <div className="px-4 pb-4 space-y-4 border-t border-zinc-800">
          <p className="text-[13px] text-zinc-400 pt-3">{ep.description}</p>

          {ep.pathParams && ep.pathParams.length > 0 && (
            <div>
              <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">Path Parameters</p>
              <ParamTable params={ep.pathParams} />
            </div>
          )}

          {ep.queryParams && ep.queryParams.length > 0 && (
            <div>
              <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">Query Parameters</p>
              <ParamTable params={ep.queryParams} />
            </div>
          )}

          {ep.bodyFields && ep.bodyFields.length > 0 && (
            <div>
              <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">Request Body</p>
              <ParamTable params={ep.bodyFields} />
            </div>
          )}

          {ep.bodyExample && (
            <div>
              <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">Example Body</p>
              <pre className="rounded bg-zinc-950 border border-zinc-800 px-3 py-2 text-[11px] font-mono text-zinc-400 overflow-x-auto">
                {ep.bodyExample}
              </pre>
            </div>
          )}

          <div>
            <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-1">Response</p>
            <p className="text-[12px] font-mono text-zinc-500">{ep.responseDesc}</p>
          </div>

          {/* Try it toggle */}
          <div>
            <button
              onClick={() => setTryOpen((v) => !v)}
              className={clsx(
                "flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium transition-colors",
                tryOpen
                  ? "bg-indigo-600/20 text-indigo-400 border border-indigo-700/40"
                  : "bg-zinc-800 text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200",
              )}
            >
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

// ── Page ──────────────────────────────────────────────────────────────────────

export function ApiDocsPage() {
  const [activeDomain, setActiveDomain] = useState(DOMAINS[0].id);
  const [search, setSearch] = useState("");

  const domain = DOMAINS.find((d) => d.id === activeDomain) ?? DOMAINS[0];

  const filtered = search.trim()
    ? DOMAINS.flatMap((d) =>
        d.endpoints
          .filter((e) =>
            e.path.toLowerCase().includes(search.toLowerCase()) ||
            e.description.toLowerCase().includes(search.toLowerCase()) ||
            e.method.toLowerCase().includes(search.toLowerCase()),
          )
          .map((e) => ({ ...e, _domain: d.label }))
      )
    : null;

  const totalEndpoints = DOMAINS.reduce((s, d) => s + d.endpoints.length, 0);

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-zinc-800 shrink-0">
        <FileCode2 size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">API Reference</span>
        <span className="text-[10px] text-zinc-700">{totalEndpoints} endpoints</span>
        <div className="ml-auto relative w-56">
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search endpoints…"
            className="w-full rounded border border-zinc-800 bg-zinc-900 text-[12px] text-zinc-300
                       placeholder-zinc-600 px-3 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>
      </div>

      {/* Body */}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {/* Domain sidebar */}
        {!search && (
          <div className="w-[180px] shrink-0 border-r border-zinc-800 bg-zinc-950 overflow-y-auto py-2">
            {DOMAINS.map((d) => (
              <button
                key={d.id}
                onClick={() => setActiveDomain(d.id)}
                className={clsx(
                  "w-full flex items-center gap-2.5 px-3 py-2 text-left transition-colors text-[12px]",
                  d.id === activeDomain
                    ? "bg-zinc-800/60 text-zinc-200"
                    : "text-zinc-500 hover:bg-zinc-900/60 hover:text-zinc-300",
                )}
              >
                <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", d.dot)} />
                {d.label}
                <span className="ml-auto text-[10px] text-zinc-700">{d.endpoints.length}</span>
              </button>
            ))}
          </div>
        )}

        {/* Endpoint list */}
        <div className="flex-1 overflow-y-auto p-5">
          {filtered ? (
            /* Search results */
            <div className="space-y-3 max-w-3xl">
              <p className="text-[11px] text-zinc-600 mb-4">
                {filtered.length} result{filtered.length !== 1 ? "s" : ""} for &ldquo;{search}&rdquo;
              </p>
              {filtered.length === 0 ? (
                <p className="text-[13px] text-zinc-600 italic py-8 text-center">No endpoints match.</p>
              ) : (
                filtered.map((ep) => (
                  <div key={ep.id}>
                    <p className="text-[10px] text-zinc-700 mb-1.5 font-medium uppercase tracking-wider">{ep._domain}</p>
                    <EndpointRow ep={ep} />
                  </div>
                ))
              )}
            </div>
          ) : (
            /* Domain view */
            <div className="max-w-3xl space-y-3">
              <div className="flex items-center gap-3 mb-4">
                <span className={clsx("w-2 h-2 rounded-full", domain.dot)} />
                <h2 className="text-[14px] font-semibold text-zinc-200">{domain.label}</h2>
                <span className="text-[11px] text-zinc-600">{domain.endpoints.length} endpoint{domain.endpoints.length !== 1 ? "s" : ""}</span>
              </div>
              {domain.endpoints.map((ep) => (
                <EndpointRow key={ep.id} ep={ep} />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default ApiDocsPage;
