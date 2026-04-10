import { useEffect, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, Inbox, Play, Pause, Trash2, Zap } from "lucide-react";
import { clsx } from "clsx";
import { useToast } from "../components/Toast";
import { useAutoRefresh, REFRESH_OPTIONS } from "../hooks/useAutoRefresh";
import type { RefreshOption } from "../hooks/useAutoRefresh";
import { useScope } from "../hooks/useScope";

const authHeaders = () => ({ Authorization: `Bearer ${localStorage.getItem("cairn_token") || ""}` });

async function parseErrorMessage(res: Response, fallback: string): Promise<string> {
  const data = await res.json().catch(() => ({}));
  return typeof data?.error === "string" ? data.error : fallback;
}

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });

interface Trigger {
  id: string;
  name: string;
  description?: string;
  signal_pattern: { signal_type: string; plugin_id?: string };
  conditions: unknown[];
  run_template_id: string;
  state: { state: string; reason?: string; since?: number } | string;
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

function stateStr(state: Trigger["state"]): string {
  return typeof state === "string" ? state : state.state;
}

function StatCard({ label, value, accent }: { label: string; value: string | number; accent?: string }) {
  return (
    <div className={clsx("border-l-2 pl-3 py-0.5", accent ?? "border-violet-500")}>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[22px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
    </div>
  );
}

function StateBadge({ state }: { state: Trigger["state"] }) {
  const s = stateStr(state);
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
    <span className="text-[11px] font-medium text-amber-400 bg-amber-950/50 border border-amber-800/40 rounded px-2 py-0.5">
      Suspended
    </span>
  );
}

const TH = ({ ch, right, hide }: { ch: React.ReactNode; right?: boolean; hide?: string }) => (
  <th className={clsx(
    "px-3 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-gray-200 dark:border-zinc-800",
    right ? "text-right" : "text-left", hide,
  )}>{ch}</th>
);

function TriggersTable({ triggers, projectPath }: { triggers: Trigger[]; projectPath: string }) {
  const qc = useQueryClient();
  const toast = useToast();

  const enableMut = useMutation({
    mutationFn: async (id: string) => {
      const res = await fetch(`/v1/projects/${projectPath}/triggers/${id}/enable`, {
        method: "POST",
        headers: authHeaders(),
      });
      if (!res.ok) {
        throw new Error(await parseErrorMessage(res, "Failed to enable trigger."));
      }
    },
    onSuccess: () => { toast.success("Trigger enabled."); void qc.invalidateQueries({ queryKey: ["triggers", projectPath] }); },
    onError: (error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Failed to enable trigger.");
    },
  });
  const disableMut = useMutation({
    mutationFn: async (id: string) => {
      const res = await fetch(`/v1/projects/${projectPath}/triggers/${id}/disable`, {
        method: "POST",
        headers: authHeaders(),
      });
      if (!res.ok) {
        throw new Error(await parseErrorMessage(res, "Failed to disable trigger."));
      }
    },
    onSuccess: () => { toast.success("Trigger disabled."); void qc.invalidateQueries({ queryKey: ["triggers", projectPath] }); },
    onError: (error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Failed to disable trigger.");
    },
  });
  const deleteMut = useMutation({
    mutationFn: async (id: string) => {
      const res = await fetch(`/v1/projects/${projectPath}/triggers/${id}`, {
        method: "DELETE",
        headers: authHeaders(),
      });
      if (!res.ok) {
        throw new Error(await parseErrorMessage(res, "Failed to delete trigger."));
      }
    },
    onSuccess: () => { toast.success("Trigger deleted."); void qc.invalidateQueries({ queryKey: ["triggers", projectPath] }); },
    onError: (error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Failed to delete trigger.");
    },
  });

  if (triggers.length === 0) return (
    <div className="flex flex-col items-center justify-center py-16 gap-2 text-center px-6">
      <Inbox size={26} className="text-gray-300 dark:text-zinc-600" />
      <p className="text-[13px] text-gray-400 dark:text-zinc-600 font-medium">No triggers</p>
      <p className="text-[11px] text-gray-300 dark:text-zinc-600 max-w-xs">
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
                {stateStr(t.state) === "enabled" ? (
                  <button onClick={() => disableMut.mutate(t.id)} className="px-2 py-0.5 rounded text-[11px] bg-gray-800 text-gray-300 hover:bg-gray-700 border border-gray-700">Disable</button>
                ) : (
                  <button onClick={() => enableMut.mutate(t.id)} className="px-2 py-0.5 rounded text-[11px] bg-emerald-900/50 text-emerald-300 hover:bg-emerald-900 border border-emerald-800/50">Enable</button>
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

function CreateRunTemplateForm({
  projectPath,
  onCreated,
}: {
  projectPath: string;
  onCreated: () => void;
}) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("You are a helpful operator automation.");
  const createTemplate = useMutation({
    mutationFn: async () => {
      const res = await fetch(`/v1/projects/${projectPath}/run-templates`, {
        method: "POST",
        headers: { ...authHeaders(), "Content-Type": "application/json" },
        body: JSON.stringify({
          name: name.trim(),
          system_prompt: systemPrompt.trim(),
        }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.error ?? "Failed to create run template.");
      }
    },
    onSuccess: () => {
      toast.success("Run template created.");
      setName("");
      onCreated();
    },
    onError: (error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Failed to create run template.");
    },
  });

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (!name.trim() || !systemPrompt.trim()) return;
        createTemplate.mutate();
      }}
      className="border-b border-gray-200 dark:border-zinc-800 p-4 space-y-3 bg-gray-50/60 dark:bg-zinc-900/40"
    >
      <div>
        <p className="text-[12px] font-medium text-gray-800 dark:text-zinc-200">Create Run Template</p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">Minimal template creation for trigger setup.</p>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="nightly-sync"
          className="rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-3 py-2 text-[12px] text-gray-800 dark:text-zinc-200"
        />
        <input
          value={systemPrompt}
          onChange={(e) => setSystemPrompt(e.target.value)}
          placeholder="System prompt"
          className="rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-3 py-2 text-[12px] text-gray-800 dark:text-zinc-200"
        />
      </div>
      <div className="flex justify-end">
        <button
          type="submit"
          disabled={createTemplate.isPending || !name.trim() || !systemPrompt.trim()}
          className="rounded bg-violet-600 px-3 py-1.5 text-[12px] font-medium text-white disabled:opacity-50"
        >
          {createTemplate.isPending ? "Creating…" : "Create Template"}
        </button>
      </div>
    </form>
  );
}

function CreateTriggerForm({
  projectPath,
  templates,
  onCreated,
}: {
  projectPath: string;
  templates: RunTemplate[];
  onCreated: () => void;
}) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [signalType, setSignalType] = useState("operator.signal");
  const [templateId, setTemplateId] = useState("");

  useEffect(() => {
    if (!templateId && templates.length > 0) {
      setTemplateId(templates[0].id);
    }
  }, [templateId, templates]);
  const createTrigger = useMutation({
    mutationFn: async () => {
      const res = await fetch(`/v1/projects/${projectPath}/triggers`, {
        method: "POST",
        headers: { ...authHeaders(), "Content-Type": "application/json" },
        body: JSON.stringify({
          name: name.trim(),
          signal_type: signalType.trim(),
          run_template_id: templateId,
          conditions: [],
        }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.error ?? "Failed to create trigger.");
      }
    },
    onSuccess: () => {
      toast.success("Trigger created.");
      setName("");
      setSignalType("operator.signal");
      onCreated();
    },
    onError: (error: unknown) => {
      toast.error(error instanceof Error ? error.message : "Failed to create trigger.");
    },
  });

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (!name.trim() || !signalType.trim() || !templateId) return;
        createTrigger.mutate();
      }}
      className="border-b border-gray-200 dark:border-zinc-800 p-4 space-y-3 bg-gray-50/60 dark:bg-zinc-900/40"
    >
      <div>
        <p className="text-[12px] font-medium text-gray-800 dark:text-zinc-200">Create Trigger</p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">Binds a signal type to a selected run template in the current project.</p>
      </div>
      {templates.length === 0 ? (
        <p className="text-[11px] text-amber-500">Create a run template first. A trigger must reference one.</p>
      ) : (
        <div className="grid gap-3 md:grid-cols-3">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="nightly-ingest"
            className="rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-3 py-2 text-[12px] text-gray-800 dark:text-zinc-200"
          />
          <input
            value={signalType}
            onChange={(e) => setSignalType(e.target.value)}
            placeholder="github.issue_opened"
            className="rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-3 py-2 text-[12px] text-gray-800 dark:text-zinc-200"
          />
          <select
            value={templateId}
            onChange={(e) => setTemplateId(e.target.value)}
            className="rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-3 py-2 text-[12px] text-gray-800 dark:text-zinc-200"
          >
            <option value="">Select template…</option>
            {templates.map((template) => (
              <option key={template.id} value={template.id}>{template.name}</option>
            ))}
          </select>
        </div>
      )}
      <div className="flex justify-end">
        <button
          type="submit"
          disabled={createTrigger.isPending || !name.trim() || !signalType.trim() || !templateId || templates.length === 0}
          className="rounded bg-violet-600 px-3 py-1.5 text-[12px] font-medium text-white disabled:opacity-50"
        >
          {createTrigger.isPending ? "Creating…" : "Create Trigger"}
        </button>
      </div>
    </form>
  );
}

export function TriggersPage() {
  const [scope] = useScope();
  const [tab, setTab] = useState<"triggers" | "templates">("triggers");
  const { ms: refreshMs, setOption: setRefreshOption } = useAutoRefresh("triggers", "15s");
  const qc = useQueryClient();
  const projectPath = encodeURIComponent(`${scope.tenant_id}/${scope.workspace_id}/${scope.project_id}`);

  const triggersQ = useQuery<Trigger[]>({
    queryKey: ["triggers", projectPath],
    queryFn: async () => {
      const res = await fetch(`/v1/projects/${projectPath}/triggers`, { headers: authHeaders() });
      if (!res.ok) {
        throw new Error(await parseErrorMessage(res, "Failed to load triggers."));
      }
      const data = await res.json();
      return Array.isArray(data) ? data : (data.items ?? []);
    },
    refetchInterval: refreshMs,
  });

  const templatesQ = useQuery<RunTemplate[]>({
    queryKey: ["run-templates", projectPath],
    queryFn: async () => {
      const res = await fetch(`/v1/projects/${projectPath}/run-templates`, { headers: authHeaders() });
      if (!res.ok) {
        throw new Error(await parseErrorMessage(res, "Failed to load run templates."));
      }
      const data = await res.json();
      return Array.isArray(data) ? data : (data.items ?? []);
    },
    refetchInterval: refreshMs,
  });

  const triggers = (triggersQ.data ?? []) as Trigger[];
  const templates = (templatesQ.data ?? []) as RunTemplate[];
  const enabled = triggers.filter(t => stateStr(t.state) === "enabled").length;
  const disabled = triggers.length - enabled;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-gray-900 dark:text-zinc-100 flex items-center gap-2">
            <Zap size={18} className="text-violet-400" /> Triggers
          </h1>
          <p className="text-[12px] text-gray-400 dark:text-zinc-600 mt-0.5">Signal-to-run bindings (RFC 022)</p>
        </div>
        <select
          className="text-[11px] bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 py-1 text-gray-600 dark:text-zinc-400"
          onChange={e => setRefreshOption(e.target.value as RefreshOption)}
        >
          {REFRESH_OPTIONS.map(o => <option key={o.option} value={o.option}>{o.label}</option>)}
        </select>
      </div>

      <div className="flex gap-6">
        <StatCard label="Total" value={triggers.length} />
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
          triggersQ.isLoading ? <div className="flex items-center justify-center py-12"><Loader2 className="animate-spin text-gray-400" /></div> : <>
            <CreateTriggerForm
              projectPath={projectPath}
              templates={templates}
              onCreated={() => {
                void qc.invalidateQueries({ queryKey: ["triggers", projectPath] });
                void qc.invalidateQueries({ queryKey: ["run-templates", projectPath] });
              }}
            />
            <TriggersTable triggers={triggers} projectPath={projectPath} />
          </>
        ) : (
          templatesQ.isLoading ? <div className="flex items-center justify-center py-12"><Loader2 className="animate-spin text-gray-400" /></div> : <div>
            <CreateRunTemplateForm
              projectPath={projectPath}
              onCreated={() => { void qc.invalidateQueries({ queryKey: ["run-templates", projectPath] }); }}
            />
            <div className="p-4"><TemplatesSection templates={templates} /></div>
          </div>
        )}
      </div>
    </div>
  );
}
