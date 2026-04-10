import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Loader2, Inbox, Play, Pause, Trash2, Plus, Zap } from "lucide-react";
import { clsx } from "clsx";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });

// ── Types ────────────────────────────────────────────────────────────────────

interface Trigger {
  id: string;
  name: string;
  description?: string;
  signal_pattern: { signal_type: string; plugin_id?: string };
  conditions: unknown[];
  run_template_id: string;
  state: { state: string; reason?: string; since?: number };
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

// ── Stat card ────────────────────────────────────────────────────────────────

function StatCard({ label, value, accent }: { label: string; value: string | number; accent?: string }) {
  return (
    <div className={clsx("border-l-2 pl-3 py-0.5", accent ?? "border-violet-500")}>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[22px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
    </div>
  );
}

// ── State badge ──────────────────────────────────────────────────────────────

function StateBadge({ state }: { state: Trigger["state"] }) {
  const s = typeof state === "string" ? state : state.state;
  if (s === "enabled") return (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-400 bg-emerald-950/50 border border-emerald-800/40 rounded px-2 py-0.5">
      <Play size={9} /> Enabled
    </span>
  );
  if (s === "disabled") return (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-gray-400 bg-gray-900/50 border border-gray-700/40 rounded px-2 py-0.5">
      <Pause size={9} /> Disabled
    </span>
  );
  return (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-amber-400 bg-amber-950/50 border border-amber-800/40 rounded px-2 py-0.5">
      Suspended
    </span>
  );
}

// ── Table header ─────────────────────────────────────────────────────────────

const TH = ({ ch, right, hide }: { ch: React.ReactNode; right?: boolean; hide?: string }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-gray-200 dark:border-zinc-800",
    right ? "text-right" : "text-left", hide,
  )}>{ch}</th>
);

// ── Triggers table ───────────────────────────────────────────────────────────

function TriggersTable({ triggers }: { triggers: Trigger[] }) {
  const qc = useQueryClient();
  const toast = useToast();

  const enableMut = useMutation({
    mutationFn: (id: string) => defaultApi.post(`/v1/projects/default/triggers/${id}/enable`, {}),
    onSuccess: () => { toast.success("Trigger enabled."); void qc.invalidateQueries({ queryKey: ["triggers"] }); },
  });
  const disableMut = useMutation({
    mutationFn: (id: string) => defaultApi.post(`/v1/projects/default/triggers/${id}/disable`, {}),
    onSuccess: () => { toast.success("Trigger disabled."); void qc.invalidateQueries({ queryKey: ["triggers"] }); },
  });
  const deleteMut = useMutation({
    mutationFn: (id: string) => defaultApi.del(`/v1/projects/default/triggers/${id}`),
    onSuccess: () => { toast.success("Trigger deleted."); void qc.invalidateQueries({ queryKey: ["triggers"] }); },
  });

  if (triggers.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-center px-6">
      <Inbox size={26} className="text-gray-300 dark:text-zinc-700" />
      <p className="text-[13px] text-gray-400 dark:text-zinc-600 font-medium">No triggers</p>
      <p className="text-[11px] text-gray-300 dark:text-zinc-700 max-w-xs">
        Triggers bind signals to runs. Create one to automate agent responses to external events.
      </p>
    </div>
  );

  return (
    <table className="min-w-full text-[13px]">
      <thead className="bg-gray-50 dark:bg-zinc-900 sticky top-0 z-10">
        <tr>
          <TH ch="Name" />
          <TH ch="Signal Type" />
          <TH ch="Template" hide="hidden sm:table-cell" />
          <TH ch="State" />
          <TH ch="Rate Limit" hide="hidden md:table-cell" />
          <TH ch="Created" hide="hidden md:table-cell" />
          <TH ch="" right />
        </tr>
      </thead>
      <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
        {triggers.map(t => (
          <tr key={t.id} className="group hover:bg-gray-50/50 dark:hover:bg-zinc-800/30 transition-colors">
            <td className="px-3 py-2.5">
              <span className="font-medium text-gray-900 dark:text-zinc-200">{t.name}</span>
              <span className="ml-2 text-[11px] text-gray-400 dark:text-zinc-600">{shortId(t.id)}</span>
            </td>
            <td className="px-3 py-2.5">
              <code className="text-[11px] bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded">{t.signal_pattern.signal_type}</code>
            </td>
            <td className="px-3 py-2.5 hidden sm:table-cell text-gray-500 dark:text-zinc-500">{shortId(t.run_template_id)}</td>
            <td className="px-3 py-2.5"><StateBadge state={t.state} /></td>
            <td className="px-3 py-2.5 hidden md:table-cell text-gray-500 dark:text-zinc-500 tabular-nums">{t.rate_limit.max_per_minute}/min</td>
            <td className="px-3 py-2.5 hidden md:table-cell text-gray-400 dark:text-zinc-600 text-[11px]">{fmtTime(t.created_at)}</td>
            <td className="px-3 py-2.5 text-right">
              <div className="flex items-center justify-end gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                {(typeof t.state === "object" ? t.state.state : t.state) === "enabled" ? (
                  <button onClick={() => disableMut.mutate(t.id)} className="px-2 py-0.5 rounded text-[11px] bg-gray-800 text-gray-300 hover:bg-gray-700 border border-gray-700">
                    Disable
                  </button>
                ) : (
                  <button onClick={() => enableMut.mutate(t.id)} className="px-2 py-0.5 rounded text-[11px] bg-emerald-900/50 text-emerald-300 hover:bg-emerald-900 border border-emerald-800/50">
                    Enable
                  </button>
                )}
                <button onClick={() => { if (window.confirm("Delete this trigger?")) deleteMut.mutate(t.id); }}
                  className="px-1.5 py-0.5 rounded text-[11px] bg-red-900/30 text-red-400 hover:bg-red-900/60 border border-red-800/40">
                  <Trash2 size={11} />
                </button>
              </div>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Templates section ────────────────────────────────────────────────────────

function TemplatesSection({ templates }: { templates: RunTemplate[] }) {
  if (templates.length === 0) return (
    <p className="text-[12px] text-gray-400 dark:text-zinc-600 py-4 text-center">No run templates yet.</p>
  );

  return (
    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {templates.map(t => (
        <div key={t.id} className="border border-gray-200 dark:border-zinc-800 rounded-lg p-3 hover:border-violet-500/50 transition-colors">
          <p className="text-[13px] font-medium text-gray-900 dark:text-zinc-200">{t.name}</p>
          {t.description && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{t.description}</p>}
          <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-2 font-mono truncate">{shortId(t.id)}</p>
        </div>
      ))}
    </div>
  );
}

// ── Main page ────────────────────────────────────────────────────────────────

export function TriggersPage() {
  const [tab, setTab] = useState<"triggers" | "templates">("triggers");
  const { interval, RefreshSelect } = useAutoRefresh("triggers-refresh");

  const triggersQ = useQuery<Trigger[]>({
    queryKey: ["triggers"],
    queryFn: () => defaultApi.get("/v1/projects/default/triggers"),
    refetchInterval: interval,
  });

  const templatesQ = useQuery<RunTemplate[]>({
    queryKey: ["run-templates"],
    queryFn: () => defaultApi.get("/v1/projects/default/run-templates"),
    refetchInterval: interval,
  });

  const triggers = triggersQ.data ?? [];
  const templates = templatesQ.data ?? [];

  const enabled  = triggers.filter(t => (typeof t.state === "object" ? t.state.state : t.state) === "enabled").length;
  const disabled = triggers.filter(t => (typeof t.state === "object" ? t.state.state : t.state) !== "enabled").length;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-gray-900 dark:text-zinc-100 flex items-center gap-2">
            <Zap size={18} className="text-violet-400" /> Triggers
          </h1>
          <p className="text-[12px] text-gray-400 dark:text-zinc-600 mt-0.5">Signal-to-run bindings (RFC 022)</p>
        </div>
        <RefreshSelect />
      </div>

      <div className="flex gap-6">
        <StatCard label="Total Triggers" value={triggers.length} />
        <StatCard label="Enabled" value={enabled} accent="border-emerald-500" />
        <StatCard label="Disabled" value={disabled} accent="border-gray-500" />
        <StatCard label="Templates" value={templates.length} accent="border-blue-500" />
      </div>

      <div className="flex gap-2 border-b border-gray-200 dark:border-zinc-800">
        <button onClick={() => setTab("triggers")}
          className={clsx("px-3 py-1.5 text-[12px] font-medium border-b-2 transition-colors", tab === "triggers" ? "border-violet-500 text-violet-400" : "border-transparent text-gray-400 hover:text-gray-300")}>
          Triggers ({triggers.length})
        </button>
        <button onClick={() => setTab("templates")}
          className={clsx("px-3 py-1.5 text-[12px] font-medium border-b-2 transition-colors", tab === "templates" ? "border-violet-500 text-violet-400" : "border-transparent text-gray-400 hover:text-gray-300")}>
          Run Templates ({templates.length})
        </button>
      </div>

      <div className="bg-white dark:bg-zinc-900/50 rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden">
        {tab === "triggers" ? (
          triggersQ.isLoading ? (
            <div className="flex items-center justify-center py-12"><Loader2 className="animate-spin text-gray-400" /></div>
          ) : (
            <TriggersTable triggers={triggers} />
          )
        ) : (
          templatesQ.isLoading ? (
            <div className="flex items-center justify-center py-12"><Loader2 className="animate-spin text-gray-400" /></div>
          ) : (
            <div className="p-4"><TemplatesSection templates={templates} /></div>
          )
        )}
      </div>
    </div>
  );
}
