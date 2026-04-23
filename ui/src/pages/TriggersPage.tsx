import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Play, Pause, Trash2, Zap } from "lucide-react";
import { DataTable } from "../components/DataTable";
import { StatCard } from "../components/StatCard";
import { CopyButton } from "../components/CopyButton";
import { HelpTooltip } from "../components/HelpTooltip";
import { ErrorFallback } from "../components/ErrorFallback";
import { useToast } from "../components/Toast";
import { clsx } from "clsx";
import { useScope } from "../hooks/useScope";
import { sectionLabel } from "../lib/design-system";

// ── Helpers ──────────────────────────────────────────────────────────────────

const authHeaders = () => ({ Authorization: `Bearer ${localStorage.getItem("cairn_token") || ""}` });

function fmtRelative(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000) return "just now";
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function mono(s: string, max = 18): string {
  return s.length > max ? `${s.slice(0, max - 3)}…` : s;
}

// ── Types ────────────────────────────────────────────────────────────────────

interface Trigger {
  id: string;
  name: string;
  description?: string;
  signal_pattern: { signal_type: string; plugin_id?: string };
  conditions: unknown[];
  run_template_id: string;
  state: { state: string } | string;
  rate_limit: { max_per_minute: number; max_burst: number };
  max_chain_depth: number;
  created_at: number;
  updated_at: number;
}

interface RunTemplate {
  id: string;
  name: string;
  description?: string;
  default_mode: unknown;
  system_prompt: string;
  created_at: number;
}

function stateStr(s: Trigger["state"]): string {
  return typeof s === "string" ? s : s.state;
}

// ── State pill (matches SessionPill / RunState pattern) ──────────────────────

const STATE_PILL: Record<string, string> = {
  enabled:   "bg-emerald-500/10 text-emerald-400 border-emerald-500/20",
  disabled:  "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700",
  suspended: "bg-amber-500/10 text-amber-400 border-amber-500/20",
};
const STATE_DOT: Record<string, string> = {
  enabled:   "bg-emerald-400",
  disabled:  "bg-zinc-600",
  suspended: "bg-amber-400 animate-pulse",
};

function StatePill({ state }: { state: string }) {
  return (
    <span className={clsx(
      "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap",
      STATE_PILL[state] ?? STATE_PILL.disabled,
    )}>
      <span className={clsx("w-1 h-1 rounded-full shrink-0", STATE_DOT[state] ?? STATE_DOT.disabled)} />
      {state}
    </span>
  );
}

// ── Main page ────────────────────────────────────────────────────────────────

export function TriggersPage() {
  const [tab, setTab] = useState<"triggers" | "templates">("triggers");
  const [scope] = useScope();
  // Backend `trigger_routes.rs` parses `:project` as
  // "tenant_id/workspace_id/project_id" and silently falls back to the
  // DEFAULT_* constants when it cannot split on `/`. Sending just
  // `scope.project_id` therefore leaks triggers across tenants — always
  // send the full slash path. See FE audit 2026-04-22 (CRITICAL).
  //
  // Axum 0.7's `:project` param captures a single path segment, so the
  // literal `/` characters MUST be percent-encoded (to `%2F`) on the
  // wire; the server decodes them back to `/` before handing the
  // segment to `parse_project_scope`. Using plain slashes here would
  // route to a completely different (non-existent) path and return 404.
  const projectPath = encodeURIComponent(
    `${scope.tenant_id}/${scope.workspace_id}/${scope.project_id}`,
  );
  const qc = useQueryClient();
  const toast = useToast();

  const triggersQ = useQuery<Trigger[]>({
    queryKey: ["triggers", projectPath],
    queryFn: async () => {
      const res = await fetch(`/v1/projects/${projectPath}/triggers`, { headers: authHeaders() });
      const data = await res.json();
      return Array.isArray(data) ? data : (data.items ?? []);
    },
    refetchInterval: 30_000,
  });

  const templatesQ = useQuery<RunTemplate[]>({
    queryKey: ["run-templates", projectPath],
    queryFn: async () => {
      const res = await fetch(`/v1/projects/${projectPath}/run-templates`, { headers: authHeaders() });
      const data = await res.json();
      return Array.isArray(data) ? data : (data.items ?? []);
    },
    refetchInterval: 30_000,
  });

  const enableMut = useMutation({
    mutationFn: (id: string) => fetch(`/v1/projects/${projectPath}/triggers/${id}/enable`, { method: "POST", headers: authHeaders() }),
    onSuccess: () => { toast.success("Trigger enabled."); void qc.invalidateQueries({ queryKey: ["triggers"] }); },
  });
  const disableMut = useMutation({
    mutationFn: (id: string) => fetch(`/v1/projects/${projectPath}/triggers/${id}/disable`, { method: "POST", headers: authHeaders() }),
    onSuccess: () => { toast.success("Trigger disabled."); void qc.invalidateQueries({ queryKey: ["triggers"] }); },
  });
  const deleteMut = useMutation({
    mutationFn: (id: string) => fetch(`/v1/projects/${projectPath}/triggers/${id}`, { method: "DELETE", headers: authHeaders() }),
    onSuccess: () => { toast.success("Trigger deleted."); void qc.invalidateQueries({ queryKey: ["triggers"] }); },
  });

  const triggers = triggersQ.data ?? [];
  const templates = templatesQ.data ?? [];
  const enabled = triggers.filter(t => stateStr(t.state) === "enabled").length;

  if (triggersQ.isError) return <ErrorFallback error={triggersQ.error} resource="triggers" onRetry={() => void triggersQ.refetch()} />;

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <p className={clsx(sectionLabel, "mb-0")}>Triggers</p>
            <HelpTooltip text="Signal-to-run bindings (RFC 022). When a signal arrives matching a trigger's pattern, a run is created from the linked template." placement="right" />
          </div>
          <p className="text-[11px] text-gray-500 dark:text-zinc-400">Implements RFC 022 — Trigger-Based Automation.</p>
        </div>
        <button onClick={() => triggersQ.refetch()} className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors">
          <RefreshCw size={11} className={clsx(triggersQ.isFetching && "animate-spin")} /> Refresh
        </button>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <StatCard label="Total Triggers" value={triggers.length} />
        <StatCard label="Enabled" value={enabled} variant="success" />
        <StatCard label="Disabled / Suspended" value={triggers.length - enabled} />
        <StatCard label="Templates" value={templates.length} variant="info" />
      </div>

      {/* Tab bar */}
      <div className="flex items-center gap-1 border-b border-gray-200 dark:border-zinc-800">
        {([["triggers", `Triggers (${triggers.length})`], ["templates", `Run Templates (${templates.length})`]] as const).map(([t, label]) => (
          <button key={t} onClick={() => setTab(t as "triggers" | "templates")}
            className={clsx(
              "px-3 py-1.5 text-[12px] font-medium border-b-2 -mb-px transition-colors",
              tab === t ? "text-gray-900 dark:text-zinc-100 border-indigo-500" : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
            )}>
            {label}
          </button>
        ))}
      </div>

      {/* Content */}
      {tab === "triggers" ? (
        <DataTable<Trigger>
          data={triggers}
          getRowId={t => t.id}
          columns={[
            { key: "name", header: "Name", render: r => <span className="flex items-center gap-1 font-medium text-gray-800 dark:text-zinc-200 text-[12px] whitespace-nowrap group/id"><Zap size={11} className="text-violet-400 shrink-0" />{r.name}<CopyButton text={r.id} label="Copy trigger ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span>, sortValue: r => r.name },
            { key: "signal", header: "Signal Type", render: r => <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded font-mono">{r.signal_pattern.signal_type}</code>, sortValue: r => r.signal_pattern.signal_type },
            { key: "template", header: "Template", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 font-mono">{mono(r.run_template_id)}</span> },
            { key: "state", header: "State", render: r => <StatePill state={stateStr(r.state)} />, sortValue: r => stateStr(r.state) },
            { key: "rate", header: "Rate", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{r.rate_limit.max_per_minute}/min</span> },
            { key: "created", header: "Created", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at },
            { key: "actions", header: "", render: r => (
              <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                {stateStr(r.state) === "enabled"
                  ? <button onClick={() => disableMut.mutate(r.id)} title="Disable" className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-gray-400 dark:text-zinc-500 transition-colors"><Pause size={12} /></button>
                  : <button onClick={() => enableMut.mutate(r.id)} title="Enable" className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-emerald-400 transition-colors"><Play size={12} /></button>}
                <button onClick={() => { if (window.confirm("Delete this trigger?")) deleteMut.mutate(r.id); }} title="Delete" className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-red-400 transition-colors"><Trash2 size={12} /></button>
              </div>
            )},
          ]}
          filterFn={(r, q) => r.name.includes(q) || r.signal_pattern.signal_type.includes(q) || r.id.includes(q)}
          csvRow={r => [r.id, r.name, r.signal_pattern.signal_type, stateStr(r.state), r.run_template_id, r.created_at]}
          csvHeaders={["ID", "Name", "Signal Type", "State", "Template", "Created"]}
          filename="triggers"
          emptyText="No triggers yet — create a run template first, then add a trigger to bind signals to runs."
        />
      ) : (
        <DataTable<RunTemplate>
          data={templates}
          getRowId={t => t.id}
          columns={[
            { key: "name", header: "Name", render: r => <span className="flex items-center gap-1 font-medium text-gray-800 dark:text-zinc-200 text-[12px] whitespace-nowrap group/id">{r.name}<CopyButton text={r.id} label="Copy template ID" size={10} className="opacity-0 group-hover/id:opacity-100" /></span>, sortValue: r => r.name },
            { key: "mode", header: "Mode", render: r => { const m = typeof r.default_mode === "object" && r.default_mode !== null ? (r.default_mode as Record<string, unknown>).type as string ?? "direct" : String(r.default_mode ?? "direct"); return <span className="text-[11px] text-gray-400 dark:text-zinc-500 capitalize">{m}</span>; } },
            { key: "prompt", header: "System Prompt", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 truncate max-w-xs block">{r.system_prompt.slice(0, 60)}{r.system_prompt.length > 60 ? "…" : ""}</span> },
            { key: "created", header: "Created", render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at },
          ]}
          filterFn={(r, q) => r.name.includes(q) || r.id.includes(q)}
          csvRow={r => [r.id, r.name, r.created_at]}
          csvHeaders={["ID", "Name", "Created"]}
          filename="run-templates"
          emptyText="No run templates — create one to define reusable run configurations for triggers."
        />
      )}
    </div>
  );
}
