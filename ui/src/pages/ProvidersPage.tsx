import { useState, type FormEvent, useId } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  RefreshCw, ServerCrash, Loader2, Plus, Trash2, ChevronDown, ChevronRight,
  Download, HardDrive, Cpu as CpuIcon, Hash, FileType, Layers, XCircle,
  Zap, Globe, Server, Check, X, Settings, Tag, Sparkles,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";
import type { ProviderConnectionRecord, ProviderHealthEntry } from "../lib/types";

// ── Types ─────────────────────────────────────────────────────────────────────

type ProviderKind = "cairn_cloud" | "openrouter" | "openai_compat" | "ollama" | "local";

interface ProviderKindMeta {
  label: string;
  description: string;
  icon: React.ReactNode;
  defaultFamily: string;
  defaultAdapter: string;
  defaultUrl: string;
}

// Pre-configured Cairn Cloud connections (agntic.garden)
const CAIRN_CLOUD_CONNECTIONS = [
  {
    id: "conn_cairn_brain",
    label: "Brain",
    url: "https://agntic.garden/inference/brain/v1",
    models: ["cyankiwi/gemma-4-31B-it-AWQ-4bit"],
    description: "High-capability reasoning model",
  },
  {
    id: "conn_cairn_worker",
    label: "Worker",
    url: "https://agntic.garden/inference/worker/v1",
    models: ["qwen3.5:9b", "qwen3-embedding:8b"],
    description: "Fast worker + embedding models",
  },
] as const;

const OPENROUTER_CONFIG = {
  id: "conn_openrouter",
  baseUrl: "https://openrouter.ai/api/v1",
  models: ["openrouter/free", "google/gemma-3-4b-it:free"],
  brainModel: "openrouter/free",
  workerModel: "google/gemma-3-4b-it:free",
  brainContext: "200K ctx",
  workerContext: "32K ctx",
  brainTooltip: "openrouter/free automatically routes to the best available free model on OpenRouter. The selected model may change as availability shifts.",
} as const;

const PROVIDER_KINDS: Record<Exclude<ProviderKind, "cairn_cloud" | "openrouter">, ProviderKindMeta> = {
  openai_compat: {
    label: "OpenAI-compatible",
    description: "Any API serving the OpenAI chat/completions format — Groq, Together, custom endpoints.",
    icon: <Globe size={16} />,
    defaultFamily: "openai_compat",
    defaultAdapter: "openai_compat",
    defaultUrl: "https://api.openai.com",
  },
  ollama: {
    label: "Ollama",
    description: "Local Ollama instance running on this machine or your network.",
    icon: <Server size={16} />,
    defaultFamily: "ollama",
    defaultAdapter: "ollama",
    defaultUrl: "http://localhost:11434",
  },
  local: {
    label: "Local / Custom",
    description: "Custom provider with manual configuration.",
    icon: <HardDrive size={16} />,
    defaultFamily: "custom",
    defaultAdapter: "custom",
    defaultUrl: "",
  },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  if (ms === 0) return "Never";
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

function genConnectionId(family: string): string {
  const ts = Date.now().toString(36).slice(-4);
  return `conn_${family.replace(/[^a-z0-9]/gi, "_").toLowerCase()}_${ts}`;
}

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub, accent = "default" }: {
  label: string; value: string | number; sub?: string;
  accent?: "default" | "emerald" | "blue" | "red";
}) {
  const borders = { default: "border-l-zinc-700", emerald: "border-l-emerald-500", blue: "border-l-indigo-500", red: "border-l-red-500" };
  const values  = { default: "text-gray-900 dark:text-zinc-100", emerald: "text-emerald-400", blue: "text-indigo-400", red: "text-red-400" };
  return (
    <div className={clsx("bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 rounded-lg p-4", borders[accent])}>
      <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">{label}</p>
      <p className={clsx("text-2xl font-semibold tabular-nums", values[accent])}>{value}</p>
      {sub && <p className="mt-1 text-[11px] text-gray-400 dark:text-zinc-600">{sub}</p>}
    </div>
  );
}

// ── Model settings popover ────────────────────────────────────────────────────

function ModelSettingsRow({ connectionId, modelId }: { connectionId: string; modelId: string }) {
  const toast       = useToast();
  const [open, setOpen] = useState(false);

  // ── Field state ───────────────────────────────────────────────────────────
  const [contextWindow,   setContextWindow]   = useState("");
  const [maxOutputTokens, setMaxOutputTokens] = useState("");
  const [temperature,     setTemperature]     = useState("");
  const [thinking,        setThinking]        = useState(false);
  const [saving,          setSaving]          = useState(false);
  const [loaded,          setLoaded]          = useState(false);

  // ── Load stored values once when the popover opens ────────────────────────
  const base = `model:${modelId}`;
  const loadDefaults = async () => {
    if (loaded) return;
    setLoaded(true);
    const keys = ["context_window", "max_output_tokens", "temperature", "thinking_mode"] as const;
    const results = await Promise.allSettled(
      keys.map(k => defaultApi.resolveDefaultSetting(`${base}:${k}`))
    );
    const [ctx, out, temp, think] = results;
    if (ctx.status === "fulfilled"  && ctx.value)   setContextWindow(String(ctx.value.value));
    if (out.status === "fulfilled"  && out.value)   setMaxOutputTokens(String(out.value.value));
    if (temp.status === "fulfilled" && temp.value)  setTemperature(String(temp.value.value));
    if (think.status === "fulfilled" && think.value) setThinking(think.value.value === true || think.value.value === "true");
  };

  const handleOpen = () => {
    setOpen(v => {
      if (!v) void loadDefaults();
      return !v;
    });
  };

  // ── Auto-fill max_output_tokens from context_window ───────────────────────
  const ctxNum = parseInt(contextWindow, 10) || 0;
  const outNum = parseInt(maxOutputTokens, 10) || (ctxNum > 0 ? Math.round(ctxNum / 4) : 0);

  const handleCtxChange = (v: string) => {
    setContextWindow(v);
    const n = parseInt(v, 10);
    if (!isNaN(n) && n > 0 && !maxOutputTokens) {
      setMaxOutputTokens(String(Math.round(n / 4)));
    }
  };

  // ── Budget bar computation ────────────────────────────────────────────────
  const SYS_ESTIMATE = 500; // rough system-prompt overhead in tokens
  const inputBudget  = ctxNum > 0 ? Math.max(0, ctxNum - outNum - SYS_ESTIMATE) : 0;

  const sysPct   = ctxNum > 0 ? (SYS_ESTIMATE / ctxNum) * 100 : 0;
  const outPct   = ctxNum > 0 ? (outNum        / ctxNum) * 100 : 0;
  const inPct    = ctxNum > 0 ? (inputBudget   / ctxNum) * 100 : 0;

  const fmtK = (n: number) => n >= 1000 ? `${(n / 1000).toFixed(0)}k` : String(n);

  // ── Save ─────────────────────────────────────────────────────────────────
  const save = async () => {
    setSaving(true);
    const writes: Promise<unknown>[] = [];
    if (contextWindow.trim())   writes.push(defaultApi.setDefaultSetting("tenant", "default", `${base}:context_window`,   parseInt(contextWindow, 10)));
    if (maxOutputTokens.trim()) writes.push(defaultApi.setDefaultSetting("tenant", "default", `${base}:max_output_tokens`, parseInt(maxOutputTokens, 10)));
    if (temperature.trim())     writes.push(defaultApi.setDefaultSetting("tenant", "default", `${base}:temperature`,       parseFloat(temperature)));
    writes.push(defaultApi.setDefaultSetting("tenant", "default", `${base}:thinking_mode`, thinking));
    try {
      await Promise.all(writes);
      toast.success(`Settings saved for ${modelId}`);
      setOpen(false);
    } catch (e) {
      toast.error(`Save failed: ${e instanceof Error ? e.message : "error"}`);
    } finally {
      setSaving(false);
    }
  };

  void connectionId; // used for future connection-scoped keying

  return (
    <div className="relative">
      <button
        onClick={handleOpen}
        className="flex items-center gap-1 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors px-1.5 py-0.5 rounded"
        title="Model settings"
      >
        <Settings size={11} />
        <span className="text-[10px]">Settings</span>
      </button>

      {open && (
        <div className="absolute right-0 top-6 z-50 w-80 bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 rounded-lg shadow-xl p-4 space-y-4">
          {/* Header */}
          <div className="flex items-center justify-between">
            <span className="text-[12px] font-medium text-gray-800 dark:text-zinc-200 truncate max-w-[220px]" title={modelId}>
              {modelId}
            </span>
            <button onClick={() => setOpen(false)} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400">
              <X size={13} />
            </button>
          </div>

          {/* Context window */}
          <label className="block">
            <div className="flex items-center justify-between mb-1">
              <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Context window (tokens)</span>
              {ctxNum > 0 && (
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono">{fmtK(ctxNum)}</span>
              )}
            </div>
            <input
              type="number" min={1024} max={2_000_000} step={1024}
              value={contextWindow}
              onChange={e => handleCtxChange(e.target.value)}
              placeholder="e.g. 32768 · 131072 · 200000"
              className="w-full rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500/30 transition-colors"
            />
          </label>

          {/* Max output tokens */}
          <label className="block">
            <div className="flex items-center justify-between mb-1">
              <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Max output tokens</span>
              {ctxNum > 0 && !maxOutputTokens && (
                <span className="text-[10px] text-gray-300 dark:text-zinc-700 italic">default ≈ {fmtK(Math.round(ctxNum / 4))}</span>
              )}
            </div>
            <input
              type="number" min={1} max={ctxNum > 0 ? ctxNum : 128000}
              value={maxOutputTokens}
              onChange={e => setMaxOutputTokens(e.target.value)}
              placeholder={ctxNum > 0 ? `e.g. ${fmtK(Math.round(ctxNum / 4))}` : "e.g. 4096"}
              className="w-full rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500/30 transition-colors"
            />
          </label>

          {/* Context budget bar */}
          {ctxNum > 0 && (
            <div className="space-y-1.5">
              <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Context budget</span>
              {/* Segmented bar */}
              <div className="h-4 rounded overflow-hidden flex bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800">
                {sysPct > 0 && (
                  <div
                    className="h-full bg-amber-500/70 transition-all"
                    style={{ width: `${Math.min(sysPct, 100)}%` }}
                    title={`System prompt ~${fmtK(SYS_ESTIMATE)} tokens`}
                  />
                )}
                {inPct > 0 && (
                  <div
                    className="h-full bg-indigo-500/70 transition-all"
                    style={{ width: `${Math.min(inPct, 100)}%` }}
                    title={`Input budget ~${fmtK(inputBudget)} tokens`}
                  />
                )}
                {outPct > 0 && (
                  <div
                    className="h-full bg-emerald-500/70 transition-all"
                    style={{ width: `${Math.min(outPct, 100)}%` }}
                    title={`Output budget ~${fmtK(outNum)} tokens`}
                  />
                )}
              </div>
              {/* Legend */}
              <div className="flex items-center gap-3 text-[10px]">
                <span className="flex items-center gap-1 text-amber-400">
                  <span className="w-2 h-2 rounded-sm bg-amber-500/70 shrink-0" />
                  System ~{fmtK(SYS_ESTIMATE)}
                </span>
                <span className="flex items-center gap-1 text-indigo-400">
                  <span className="w-2 h-2 rounded-sm bg-indigo-500/70 shrink-0" />
                  Input {fmtK(inputBudget)}
                </span>
                <span className="flex items-center gap-1 text-emerald-400">
                  <span className="w-2 h-2 rounded-sm bg-emerald-500/70 shrink-0" />
                  Output {fmtK(outNum)}
                </span>
              </div>
            </div>
          )}

          {/* Temperature */}
          <label className="block">
            <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Temperature (0\u20132)</span>
            <div className="flex items-center gap-2 mt-1">
              <input
                type="range" min={0} max={2} step={0.05}
                value={parseFloat(temperature) || 0.7}
                onChange={e => setTemperature(e.target.value)}
                className="flex-1 accent-indigo-500"
              />
              <span className="text-[11px] text-gray-500 dark:text-zinc-400 w-8 text-right font-mono">
                {temperature ? parseFloat(temperature).toFixed(2) : "0.70"}
              </span>
            </div>
          </label>

          {/* Thinking mode toggle */}
          <label className="flex items-center justify-between cursor-pointer">
            <span className="text-[11px] text-gray-500 dark:text-zinc-400">Thinking mode</span>
            <div
              onClick={() => setThinking(v => !v)}
              className={clsx(
                "w-9 h-5 rounded-full border transition-colors relative shrink-0",
                thinking ? "bg-indigo-600 border-indigo-500" : "bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700",
              )}
            >
              <div className={clsx(
                "absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform shadow-sm",
                thinking ? "translate-x-4" : "translate-x-0.5",
              )} />
            </div>
          </label>

          {/* Save */}
          <button
            onClick={save}
            disabled={saving}
            className="w-full h-8 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 text-white text-xs font-medium transition-colors flex items-center justify-center gap-1.5"
          >
            {saving ? <Loader2 size={11} className="animate-spin" /> : <Check size={11} />}
            {saving ? "Saving\u2026" : "Save defaults"}
          </button>
        </div>
      )}
    </div>
  );
}

// ── Connection row ────────────────────────────────────────────────────────────

function ConnectionRow({
  record,
  health,
  even,
  onDelete,
}: {
  record: ProviderConnectionRecord;
  health?: ProviderHealthEntry;
  even: boolean;
  onDelete: (id: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const isHealthy = health?.healthy ?? null;

  const familyColor: Record<string, string> = {
    openai_compat: "text-sky-400 bg-sky-950/40 border-sky-800/40",
    ollama:        "text-emerald-400 bg-emerald-950/40 border-emerald-800/40",
    custom:        "text-gray-500 dark:text-zinc-400 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700",
  };

  const familyClass = familyColor[record.provider_family] ?? familyColor.custom;

  return (
    <>
      <tr className={clsx("border-b border-gray-200/50 dark:border-zinc-800/50 hover:bg-white/5 transition-colors", even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
        {/* Status */}
        <td className="px-4 h-10 w-6">
          <div className="flex items-center gap-1.5">
            {isHealthy === null ? (
              <span className="w-1.5 h-1.5 rounded-full bg-zinc-700" title="No health data" />
            ) : isHealthy ? (
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-400" title="Healthy" />
            ) : (
              <span className="w-1.5 h-1.5 rounded-full bg-red-400 animate-pulse" title="Unhealthy" />
            )}
          </div>
        </td>
        {/* Connection ID */}
        <td className="px-3 h-10">
          <span className="text-xs font-mono text-gray-700 dark:text-zinc-300 truncate block max-w-[180px]" title={record.provider_connection_id}>
            {record.provider_connection_id}
          </span>
        </td>
        {/* Family */}
        <td className="px-3 h-10">
          <span className={clsx("text-[10px] font-medium px-1.5 py-0.5 rounded border", familyClass)}>
            {record.provider_family}
          </span>
        </td>
        {/* Adapter */}
        <td className="px-3 h-10">
          <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500">{record.adapter_type}</span>
        </td>
        {/* Models */}
        <td className="px-3 h-10">
          <div className="flex items-center gap-1 flex-wrap">
            {record.supported_models.length === 0 ? (
              <span className="text-[10px] text-gray-300 dark:text-zinc-700 italic">none</span>
            ) : (
              record.supported_models.slice(0, 3).map(m => (
                <span key={m} className="flex items-center gap-0.5 text-[10px] font-mono text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
                  <Tag size={9} className="text-gray-400 dark:text-zinc-600" />{m}
                </span>
              ))
            )}
            {record.supported_models.length > 3 && (
              <span className="text-[10px] text-gray-400 dark:text-zinc-600">+{record.supported_models.length - 3}</span>
            )}
          </div>
        </td>
        {/* Actions */}
        <td className="px-3 h-10 text-right">
          <div className="flex items-center justify-end gap-2">
            <button
              onClick={() => setExpanded(v => !v)}
              className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors"
              title={expanded ? "Collapse" : "Expand models"}
            >
              {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
            </button>
            <button
              onClick={() => onDelete(record.provider_connection_id)}
              className="text-gray-400 dark:text-zinc-600 hover:text-red-400 transition-colors"
              title="Delete connection"
            >
              <Trash2 size={12} />
            </button>
          </div>
        </td>
      </tr>

      {/* Expanded: per-model settings */}
      {expanded && record.supported_models.length > 0 && (
        <tr className={clsx("border-b border-gray-200/50 dark:border-zinc-800/50", even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
          <td colSpan={6} className="px-4 pb-3 pt-1">
            <div className="bg-white dark:bg-zinc-950 rounded-md border border-gray-200 dark:border-zinc-800 overflow-hidden">
              <div className="px-3 py-1.5 border-b border-gray-200 dark:border-zinc-800 flex items-center gap-2">
                <span className="text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Models</span>
                <span className="text-[10px] text-gray-300 dark:text-zinc-700">· defaults are tenant-scoped</span>
              </div>
              {record.supported_models.map((m, i) => (
                <div key={m} className={clsx(
                  "flex items-center justify-between px-3 h-8 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0",
                  i % 2 === 0 ? "" : "bg-gray-50/30 dark:bg-zinc-900/30",
                )}>
                  <span className="text-[11px] font-mono text-gray-700 dark:text-zinc-300">{m}</span>
                  <ModelSettingsRow connectionId={record.provider_connection_id} modelId={m} />
                </div>
              ))}
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

// ── Add Provider Modal ────────────────────────────────────────────────────────

interface AddProviderModalProps {
  onClose: () => void;
  onCreated: () => void;
}

function AddProviderModal({ onClose, onCreated }: AddProviderModalProps) {
  const toast = useToast();
  const formId = useId();

  // Step: 0=type, 1=form, 2=models  (cairn_cloud skips to a dedicated confirm screen)
  const [step, setStep] = useState<0 | 1 | 2>(0);
  const [kind, setKind] = useState<ProviderKind>("openai_compat");

  // Safe accessor — cairn_cloud and openrouter don't have PROVIDER_KINDS entries
  const meta = (kind !== "cairn_cloud" && kind !== "openrouter") ? PROVIDER_KINDS[kind] : PROVIDER_KINDS.openai_compat;

  // Form fields (used for single-connection flow)
  const [connectionId, setConnectionId] = useState(() => genConnectionId("openai_compat"));
  const [baseUrl,      setBaseUrl]       = useState(meta.defaultUrl);
  const [apiKey,       setApiKey]        = useState("");
  const [openrouterKey, setOpenrouterKey] = useState("");
  const [family,       setFamily]        = useState(meta.defaultFamily);
  const [adapter,      setAdapter]       = useState(meta.defaultAdapter);

  // Model entry (single-connection flow)
  const [models, setModels]     = useState<string[]>([]);
  const [modelInput, setModelInput] = useState("");

  const selectKind = (k: ProviderKind) => {
    setKind(k);
    if (k === "cairn_cloud" || k === "openrouter") return; // handled as presets
    const m = PROVIDER_KINDS[k];
    setConnectionId(genConnectionId(m.defaultFamily));
    setBaseUrl(m.defaultUrl);
    setFamily(m.defaultFamily);
    setAdapter(m.defaultAdapter);
  };

  const addModel = () => {
    const v = modelInput.trim();
    if (v && !models.includes(v)) setModels(prev => [...prev, v]);
    setModelInput("");
  };

  const removeModel = (m: string) => setModels(prev => prev.filter(x => x !== m));

  // Single-connection mutation (standard flow)
  const createMutation = useMutation({
    mutationFn: () =>
      defaultApi.createProviderConnection({
        tenant_id: "default",
        provider_connection_id: connectionId.trim(),
        provider_family: family.trim(),
        adapter_type: adapter.trim(),
        supported_models: models,
      }),
    onSuccess: () => {
      toast.success(`Provider "${connectionId}" registered.`);
      onCreated();
      onClose();
    },
    onError: (e) => toast.error(`Failed: ${e instanceof Error ? e.message : "error"}`),
  });

  // Cairn Cloud dual-connection mutation
  const cairnCloudMutation = useMutation({
    mutationFn: async () => {
      for (const conn of CAIRN_CLOUD_CONNECTIONS) {
        await defaultApi.createProviderConnection({
          tenant_id:              "default",
          provider_connection_id: conn.id,
          provider_family:        "openai_compat",
          adapter_type:           "openai_compat",
          supported_models:       [...conn.models],
        });
      }
    },
    onSuccess: () => {
      toast.success("Cairn Cloud (agntic.garden) — 2 connections registered.");
      onCreated();
      onClose();
    },
    onError: (e) => toast.error(`Failed: ${e instanceof Error ? e.message : "error"}`),
  });

  // OpenRouter single-connection mutation
  const openrouterMutation = useMutation({
    mutationFn: () =>
      defaultApi.createProviderConnection({
        tenant_id:              "default",
        provider_connection_id: OPENROUTER_CONFIG.id,
        provider_family:        "openai_compat",
        adapter_type:           "openai_compat",
        supported_models:       [...OPENROUTER_CONFIG.models],
      }),
    onSuccess: () => {
      toast.success("OpenRouter — connection registered.");
      onCreated();
      onClose();
    },
    onError: (e) => toast.error(`Failed: ${e instanceof Error ? e.message : "error"}`),
  });

  const steps = ["Type", "Connection", "Models"];

  return (
    <>
      {/* Backdrop */}
      <div className="fixed inset-0 z-40 bg-black/60" onClick={onClose} />

      {/* Panel */}
      <div className="fixed right-0 top-0 bottom-0 z-50 w-[480px] bg-white dark:bg-zinc-950 border-l border-gray-200 dark:border-zinc-800 flex flex-col shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-5 h-12 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">Add Provider</span>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors">
            <XCircle size={16} />
          </button>
        </div>

        {/* Stepper */}
        <div className="flex items-center px-5 h-10 border-b border-gray-200/60 dark:border-zinc-800/60 shrink-0 gap-0">
          {steps.map((s, i) => (
            <div key={s} className="flex items-center">
              <div className={clsx(
                "flex items-center gap-1.5 text-[11px] font-medium px-2 py-1 rounded",
                i === step
                  ? "text-indigo-300"
                  : i < step
                    ? "text-gray-500 dark:text-zinc-400 cursor-pointer hover:text-gray-700 dark:text-zinc-300"
                    : "text-gray-300 dark:text-zinc-700",
              )}
                onClick={() => i < step && setStep(i as 0 | 1 | 2)}
              >
                <span className={clsx(
                  "w-4 h-4 rounded-full flex items-center justify-center text-[10px] font-semibold",
                  i === step ? "bg-indigo-600 text-white" : i < step ? "bg-zinc-700 text-gray-700 dark:text-zinc-300" : "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-600",
                )}>
                  {i < step ? <Check size={9} strokeWidth={3} /> : i + 1}
                </span>
                {s}
              </div>
              {i < steps.length - 1 && (
                <div className="w-6 h-px bg-gray-100 dark:bg-zinc-800 mx-1" />
              )}
            </div>
          ))}
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto p-5">

          {/* ── Step 0: Type selection ── */}
          {step === 0 && (
            <div className="space-y-3">
              <p className="text-[12px] text-gray-400 dark:text-zinc-500 mb-4">
                Choose the type of provider you want to connect.
              </p>

              {/* ── Cairn Cloud quick-start (featured) ── */}
              <button
                onClick={() => { selectKind("cairn_cloud"); setStep(1); }}
                className={clsx(
                  "w-full flex items-start gap-3 p-4 rounded-lg border text-left transition-colors",
                  kind === "cairn_cloud"
                    ? "border-emerald-500/60 bg-emerald-950/20"
                    : "border-emerald-800/40 bg-emerald-950/10 hover:border-emerald-700/60 hover:bg-emerald-950/20",
                )}
              >
                <span className={clsx("mt-0.5 shrink-0", kind === "cairn_cloud" ? "text-emerald-400" : "text-emerald-600")}>
                  <Sparkles size={16} />
                </span>
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <p className="text-[13px] font-medium text-gray-900 dark:text-zinc-100">Cairn Cloud</p>
                    <span className="text-[10px] font-medium text-emerald-400 bg-emerald-900/40 border border-emerald-700/40 rounded px-1.5 py-0.5">
                      Recommended
                    </span>
                  </div>
                  <p className="text-[11px] text-gray-400 dark:text-zinc-500 mt-0.5 leading-relaxed">
                    agntic.garden — registers Brain (Gemma 4 31B) + Worker (Qwen 3.5 9B + embeddings) in one click.
                  </p>
                  <div className="flex items-center gap-2 mt-2">
                    {CAIRN_CLOUD_CONNECTIONS.map(c => (
                      <span key={c.id} className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
                        {c.label}: {c.models.join(", ")}
                      </span>
                    ))}
                  </div>
                </div>
                {kind === "cairn_cloud" && (
                  <Check size={14} className="text-emerald-400 ml-auto mt-0.5 shrink-0" />
                )}
              </button>

              {/* ── OpenRouter quick-start ── */}
              <button
                onClick={() => { selectKind("openrouter"); setStep(1); }}
                className={clsx(
                  "w-full flex items-start gap-3 p-4 rounded-lg border text-left transition-colors",
                  kind === "openrouter"
                    ? "border-violet-500/60 bg-violet-950/20"
                    : "border-violet-800/40 bg-violet-950/10 hover:border-violet-700/60 hover:bg-violet-950/20",
                )}
              >
                <span className={clsx("mt-0.5 shrink-0 font-bold text-[13px]", kind === "openrouter" ? "text-violet-400" : "text-violet-600")}>
                  OR
                </span>
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <p className="text-[13px] font-medium text-gray-900 dark:text-zinc-100">OpenRouter</p>
                    <span className="text-[10px] font-medium text-violet-400 bg-violet-900/40 border border-violet-700/40 rounded px-1.5 py-0.5">
                      Free tier available
                    </span>
                  </div>
                  <p className="text-[11px] text-gray-400 dark:text-zinc-500 mt-0.5 leading-relaxed">
                    openrouter.ai — access 200+ models via one OpenAI-compatible endpoint. Free API key at openrouter.ai.
                  </p>
                  <div className="flex items-center gap-2 mt-2">
                    <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
                      Brain: {OPENROUTER_CONFIG.brainModel} ({OPENROUTER_CONFIG.brainContext})
                    </span>
                    <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
                      Worker: {OPENROUTER_CONFIG.workerModel} ({OPENROUTER_CONFIG.workerContext})
                    </span>
                  </div>
                </div>
                {kind === "openrouter" && (
                  <Check size={14} className="text-violet-400 ml-auto mt-0.5 shrink-0" />
                )}
              </button>

              {/* Divider */}
              <div className="flex items-center gap-2 py-1">
                <div className="flex-1 h-px bg-gray-100 dark:bg-zinc-800" />
                <span className="text-[10px] text-gray-300 dark:text-zinc-700 uppercase tracking-wide">or configure manually</span>
                <div className="flex-1 h-px bg-gray-100 dark:bg-zinc-800" />
              </div>

              {/* Standard provider types */}
              {(Object.entries(PROVIDER_KINDS) as [Exclude<ProviderKind, "cairn_cloud">, ProviderKindMeta][]).map(([k, m]) => (
                <button
                  key={k}
                  onClick={() => { selectKind(k); setStep(1); }}
                  className={clsx(
                    "w-full flex items-start gap-3 p-4 rounded-lg border text-left transition-colors",
                    kind === k
                      ? "border-indigo-500/60 bg-indigo-950/30"
                      : "border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 hover:border-gray-200 dark:border-zinc-700 hover:bg-gray-100/40 dark:hover:bg-gray-100/40 dark:bg-zinc-800/40",
                  )}
                >
                  <span className={clsx("mt-0.5 shrink-0", kind === k ? "text-indigo-400" : "text-gray-400 dark:text-zinc-500")}>
                    {m.icon}
                  </span>
                  <div>
                    <p className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">{m.label}</p>
                    <p className="text-[11px] text-gray-400 dark:text-zinc-500 mt-0.5 leading-relaxed">{m.description}</p>
                  </div>
                  {kind === k && (
                    <Check size={14} className="text-indigo-400 ml-auto mt-0.5 shrink-0" />
                  )}
                </button>
              ))}
            </div>
          )}

          {/* ── Step 1: Cairn Cloud confirm screen ── */}
          {step === 1 && kind === "cairn_cloud" && (
            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <Sparkles size={14} className="text-emerald-400" />
                <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">Cairn Cloud — agntic.garden</span>
              </div>
              <p className="text-[12px] text-gray-400 dark:text-zinc-500 leading-relaxed">
                Two connections will be registered immediately. No API key is required for the agntic.garden inference endpoints.
              </p>
              <div className="space-y-3">
                {CAIRN_CLOUD_CONNECTIONS.map(conn => (
                  <div key={conn.id} className="rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 p-4">
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-[12px] font-medium text-gray-800 dark:text-zinc-200">{conn.label}</span>
                      <code className="text-[10px] font-mono text-gray-400 dark:text-zinc-600">{conn.id}</code>
                    </div>
                    <p className="text-[11px] text-gray-400 dark:text-zinc-600 mb-2">{conn.description}</p>
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="text-[10px] text-gray-400 dark:text-zinc-600 w-12 shrink-0">URL</span>
                        <code className="text-[10px] font-mono text-gray-500 dark:text-zinc-400 truncate">{conn.url}</code>
                      </div>
                      <div className="flex items-center gap-2">
                        <span className="text-[10px] text-gray-400 dark:text-zinc-600 w-12 shrink-0">Models</span>
                        <div className="flex flex-wrap gap-1">
                          {conn.models.map(m => (
                            <span key={m} className="text-[10px] font-mono text-gray-700 dark:text-zinc-300 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">{m}</span>
                          ))}
                        </div>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
              <div className="flex items-start gap-2 px-3 py-2.5 rounded-md bg-emerald-950/20 border border-emerald-800/30">
                <Check size={12} className="text-emerald-400 mt-0.5 shrink-0" />
                <p className="text-[11px] text-emerald-300/70 leading-relaxed">
                  You can add more models or edit connection settings later from the Providers table.
                </p>
              </div>
            </div>
          )}

          {/* ── Step 1: OpenRouter confirm screen ── */}
          {step === 1 && kind === "openrouter" && (
            <div className="space-y-4">
              <div className="flex items-center gap-2">
                <span className="font-bold text-[13px] text-violet-400">OR</span>
                <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">OpenRouter</span>
                <span className="text-[10px] font-medium text-violet-400 bg-violet-900/40 border border-violet-700/40 rounded px-1.5 py-0.5">Free tier available</span>
              </div>
              <p className="text-[12px] text-gray-400 dark:text-zinc-500 leading-relaxed">
                One connection registers both models. Requires a free API key from{" "}
                <span className="text-violet-400 font-mono text-[11px]">openrouter.ai</span>.
              </p>

              {/* Base URL (read-only) */}
              <div>
                <p className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide mb-1">Base URL</p>
                <code className="text-[11px] font-mono text-gray-500 dark:text-zinc-400 bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded px-2 py-1.5 block">
                  {OPENROUTER_CONFIG.baseUrl}
                </code>
              </div>

              {/* API Key */}
              <label className="block">
                <p className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide mb-1.5">API Key <span className="text-red-400">*</span></p>
                <input
                  type="password"
                  value={openrouterKey}
                  onChange={e => setOpenrouterKey(e.target.value)}
                  placeholder="sk-or-v1-…"
                  autoComplete="off"
                  className="w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500/30 transition-colors"
                />
                <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">
                  Get a free key at <span className="text-gray-400 dark:text-zinc-500 font-mono">openrouter.ai/settings/keys</span>. Never stored in plaintext.
                </p>
              </label>

              {/* Pre-selected models */}
              <div>
                <p className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide mb-2">Pre-selected models</p>
                <div className="space-y-2">
                  <div className="flex items-center justify-between rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-1.5">
                        <p className="text-[11px] font-mono text-gray-800 dark:text-zinc-200">{OPENROUTER_CONFIG.brainModel}</p>
                        <span
                          title={OPENROUTER_CONFIG.brainTooltip}
                          className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full bg-zinc-700 text-gray-500 dark:text-zinc-400 text-[9px] font-bold cursor-help shrink-0"
                        >?</span>
                      </div>
                      <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-0.5">Brain — {OPENROUTER_CONFIG.brainContext} context · auto-routes to best free model</p>
                    </div>
                    <span className="text-[10px] text-violet-400 bg-violet-900/30 border border-violet-700/40 rounded px-1.5 py-0.5 ml-2 shrink-0">free</span>
                  </div>
                  <div className="flex items-center justify-between rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2">
                    <div>
                      <p className="text-[11px] font-mono text-gray-800 dark:text-zinc-200">{OPENROUTER_CONFIG.workerModel}</p>
                      <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-0.5">Worker — {OPENROUTER_CONFIG.workerContext} context</p>
                    </div>
                    <span className="text-[10px] text-violet-400 bg-violet-900/30 border border-violet-700/40 rounded px-1.5 py-0.5">free</span>
                  </div>
                </div>
              </div>

              <div className="flex items-start gap-2 px-3 py-2.5 rounded-md bg-violet-950/20 border border-violet-800/30">
                <Check size={12} className="text-violet-400 mt-0.5 shrink-0" />
                <p className="text-[11px] text-violet-300/70 leading-relaxed">
                  Both models are served through a single connection. You can add more models from the connections table after registering.
                </p>
              </div>
            </div>
          )}

          {/* ── Step 1: Standard connection form ── */}
          {step === 1 && kind !== "cairn_cloud" && kind !== "openrouter" && (
            <form
              id={`${formId}-form`}
              onSubmit={(e: FormEvent) => { e.preventDefault(); setStep(2); }}
              className="space-y-4"
            >
              <div className="flex items-center gap-2 mb-1">
                <span className="text-gray-400 dark:text-zinc-500">{meta.icon}</span>
                <span className="text-[12px] font-medium text-gray-700 dark:text-zinc-300">{meta.label}</span>
              </div>

              <label className="block">
                <span className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Connection ID</span>
                <input
                  required
                  value={connectionId}
                  onChange={e => setConnectionId(e.target.value)}
                  placeholder="conn_openai_abc1"
                  className="mt-1.5 w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 font-mono placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                />
                <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">Unique identifier for this connection. Cannot be changed later.</p>
              </label>

              {kind !== "local" && (
                <label className="block">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Base URL</span>
                  <input
                    value={baseUrl}
                    onChange={e => setBaseUrl(e.target.value)}
                    placeholder={meta.defaultUrl || "https://…"}
                    className="mt-1.5 w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                  />
                </label>
              )}

              {kind === "openai_compat" && (
                <label className="block">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">API Key</span>
                  <input
                    type="password"
                    value={apiKey}
                    onChange={e => setApiKey(e.target.value)}
                    placeholder="sk-… (optional)"
                    autoComplete="off"
                    className="mt-1.5 w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                  />
                  <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">Stored as a credential. Leave blank for unauthenticated endpoints.</p>
                </label>
              )}

              <div className="grid grid-cols-2 gap-3">
                <label className="block">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Provider family</span>
                  <input
                    required
                    value={family}
                    onChange={e => setFamily(e.target.value)}
                    className="mt-1.5 w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 font-mono placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                  />
                </label>
                <label className="block">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Adapter type</span>
                  <input
                    required
                    value={adapter}
                    onChange={e => setAdapter(e.target.value)}
                    className="mt-1.5 w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 font-mono placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                  />
                </label>
              </div>
            </form>
          )}

          {/* ── Step 2: Model discovery / manual entry ── */}
          {step === 2 && (
            <div className="space-y-4">
              <div>
                <p className="text-[12px] text-gray-700 dark:text-zinc-300 font-medium">Add models</p>
                <p className="text-[11px] text-gray-400 dark:text-zinc-500 mt-1 leading-relaxed">
                  Enter the model IDs served through this connection.
                  {kind === "ollama" && " Model discovery via Ollama is coming — enter IDs manually for now."}
                  {kind === "openai_compat" && " Discovery from the base URL is coming — enter IDs manually for now."}
                </p>
              </div>

              {/* Manual entry */}
              <div className="flex gap-2">
                <div className="relative flex-1">
                  <Plus size={11} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
                  <input
                    value={modelInput}
                    onChange={e => setModelInput(e.target.value)}
                    onKeyDown={e => { if (e.key === "Enter") { e.preventDefault(); addModel(); } }}
                    placeholder={
                      kind === "openai_compat" ? "e.g. gemma4, qwen3.5:9b" :
                      kind === "ollama"        ? "e.g. llama3.2, mistral" :
                                                "model_id"
                    }
                    className="w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 pl-7 pr-3 h-8 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                  />
                </div>
                <button
                  type="button"
                  onClick={addModel}
                  disabled={!modelInput.trim()}
                  className="flex items-center gap-1.5 px-3 h-8 rounded-md bg-gray-100 dark:bg-zinc-800 hover:bg-gray-200 dark:hover:bg-zinc-700 disabled:opacity-40 text-gray-700 dark:text-zinc-300 text-xs font-medium transition-colors"
                >
                  <Plus size={11} /> Add
                </button>
              </div>

              {/* Discovery stub — placeholder for when W1 ships the endpoint */}
              <div className="flex items-center gap-2 px-3 py-2 rounded-md bg-gray-50/60 dark:bg-zinc-900/60 border border-gray-200/60 dark:border-zinc-800/60 border-dashed">
                <Zap size={12} className="text-gray-400 dark:text-zinc-600 shrink-0" />
                <span className="text-[11px] text-gray-400 dark:text-zinc-600">
                  Auto-discover coming soon — will call{" "}
                  <code className="text-gray-400 dark:text-zinc-500 font-mono text-[10px]">GET /v1/providers/connections/:id/discover-models</code>
                </span>
              </div>

              {/* Model chips */}
              {models.length > 0 && (
                <div className="space-y-1">
                  <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wide">{models.length} model{models.length !== 1 ? "s" : ""} added</p>
                  <div className="flex flex-wrap gap-1.5">
                    {models.map(m => (
                      <span
                        key={m}
                        className="flex items-center gap-1 text-[11px] font-mono text-gray-700 dark:text-zinc-300 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 py-0.5"
                      >
                        {m}
                        <button
                          onClick={() => removeModel(m)}
                          className="text-gray-400 dark:text-zinc-600 hover:text-red-400 transition-colors ml-0.5"
                        >
                          <X size={10} />
                        </button>
                      </span>
                    ))}
                  </div>
                </div>
              )}

              {models.length === 0 && (
                <p className="text-[11px] text-gray-300 dark:text-zinc-700 italic">
                  You can register models later from the connection row.
                </p>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-5 py-3 border-t border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
          <button
            onClick={() => step === 0 ? onClose() : setStep((step - 1) as 0 | 1 | 2)}
            className="text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 transition-colors px-3 py-1.5"
          >
            {step === 0 ? "Cancel" : "← Back"}
          </button>

          {/* Preset quick-registers: step 1 shows the register button immediately */}
          {kind === "cairn_cloud" && step === 1 ? (
            <button
              onClick={() => cairnCloudMutation.mutate()}
              disabled={cairnCloudMutation.isPending}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md bg-emerald-600 hover:bg-emerald-500 disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 text-white text-[12px] font-medium transition-colors"
            >
              {cairnCloudMutation.isPending ? (
                <><Loader2 size={12} className="animate-spin" /> Registering…</>
              ) : (
                <><Sparkles size={12} /> Register Both</>
              )}
            </button>
          ) : kind === "openrouter" && step === 1 ? (
            <button
              onClick={() => openrouterMutation.mutate()}
              disabled={openrouterMutation.isPending || !openrouterKey.trim()}
              title={!openrouterKey.trim() ? "Enter your OpenRouter API key first" : undefined}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md bg-violet-600 hover:bg-violet-500 disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 text-white text-[12px] font-medium transition-colors"
            >
              {openrouterMutation.isPending ? (
                <><Loader2 size={12} className="animate-spin" /> Registering…</>
              ) : (
                <><Check size={12} /> Register OpenRouter</>
              )}
            </button>
          ) : step < 2 ? (
            <button
              type={step === 1 ? "submit" : "button"}
              form={step === 1 ? `${formId}-form` : undefined}
              onClick={step !== 1 ? () => setStep((step + 1) as 1 | 2) : undefined}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 text-white text-[12px] font-medium transition-colors"
            >
              Next →
            </button>
          ) : (
            <button
              onClick={() => createMutation.mutate()}
              disabled={createMutation.isPending}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 text-white text-[12px] font-medium transition-colors"
            >
              {createMutation.isPending ? (
                <><Loader2 size={12} className="animate-spin" /> Registering…</>
              ) : (
                <><Check size={12} /> Register Provider</>
              )}
            </button>
          )}
        </div>
      </div>
    </>
  );
}

// ── Connections section ───────────────────────────────────────────────────────

function ConnectionsSection({ onAdd }: { onAdd: () => void }) {
  const toast = useToast();
  const qc = useQueryClient();

  const { data, isLoading, isError, error, refetch, dataUpdatedAt } = useQuery({
    queryKey: ["provider-connections"],
    queryFn:  () => defaultApi.listProviderConnections("default"),
    staleTime: 30_000,
  });

  const { data: healthData } = useQuery({
    queryKey: ["providers-health"],
    queryFn:  () => defaultApi.getProviderHealth(),
    refetchInterval: 20_000,
  });

  const entries: ProviderConnectionRecord[] = data?.items ?? [];
  const healthMap = new Map<string, ProviderHealthEntry>(
    (Array.isArray(healthData) ? healthData : []).map(h => [h.connection_id, h])
  );

  const healthy   = entries.filter(e => healthMap.get(e.provider_connection_id)?.healthy === true).length;
  const unhealthy = entries.length - healthy;

  // TODO: DELETE /v1/providers/connections/:id when endpoint lands
  const handleDelete = (id: string) => {
    if (!confirm(`Delete connection "${id}"?`)) return;
    toast.info(`Delete not yet implemented — remove via API.`);
    void id;
  };

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
            Provider Connections
          </p>
          {entries.length > 0 && (
            <span className="text-[11px] text-gray-300 dark:text-zinc-700">({entries.length})</span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {dataUpdatedAt > 0 && (
            <span className="text-[11px] font-mono text-gray-300 dark:text-zinc-700">
              {new Date(dataUpdatedAt).toLocaleTimeString()}
            </span>
          )}
          <button
            onClick={() => void qc.invalidateQueries({ queryKey: ["provider-connections"] })}
            className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors"
          >
            <RefreshCw size={11} /> Refresh
          </button>
          <button
            onClick={onAdd}
            className="flex items-center gap-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 px-3 py-1.5 text-[11px] text-white font-medium transition-colors"
          >
            <Plus size={11} /> Add Provider
          </button>
        </div>
      </div>

      {/* Stat cards when there are connections */}
      {!isLoading && entries.length > 0 && (
        <div className="grid grid-cols-3 gap-3">
          <StatCard label="Total"    value={entries.length} />
          <StatCard label="Healthy"  value={healthy}   accent="emerald" />
          <StatCard label="Degraded" value={unhealthy}  accent={unhealthy > 0 ? "red" : "default"} />
        </div>
      )}

      {/* Table */}
      <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
        <table className="w-full">
          <thead>
            <tr className="border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950">
              <th className="px-4 h-8 w-6" />
              <th className="px-3 h-8 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Connection ID</th>
              <th className="px-3 h-8 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Family</th>
              <th className="px-3 h-8 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Adapter</th>
              <th className="px-3 h-8 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Models</th>
              <th className="px-3 h-8" />
            </tr>
          </thead>
        </table>

        {isError ? (
          <div className="flex items-center gap-3 px-4 py-4">
            <ServerCrash size={16} className="text-red-500 shrink-0" />
            <span className="text-[12px] text-gray-500 dark:text-zinc-400">{error instanceof Error ? error.message : "Failed to load"}</span>
          </div>
        ) : isLoading ? (
          <div className="flex items-center gap-2 px-4 h-10 text-[11px] text-gray-400 dark:text-zinc-600">
            <Loader2 size={11} className="animate-spin" /> Loading…
          </div>
        ) : entries.length === 0 ? (
          <div className="px-4 py-10 text-center space-y-3">
            <Server size={24} className="text-gray-300 dark:text-zinc-700 mx-auto" />
            <p className="text-[12px] text-gray-400 dark:text-zinc-600">No provider connections registered.</p>
            <button
              onClick={onAdd}
              className="inline-flex items-center gap-1.5 text-[11px] text-indigo-400 hover:text-indigo-300 transition-colors"
            >
              <Plus size={11} /> Add your first provider
            </button>
          </div>
        ) : (
          <table className="w-full">
            <tbody>
              {entries.map((entry, i) => (
                <ConnectionRow
                  key={entry.provider_connection_id}
                  record={entry}
                  health={healthMap.get(entry.provider_connection_id)}
                  even={i % 2 === 0}
                  onDelete={handleDelete}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* Legacy health table header note */}
      {entries.length > 0 && healthData && (
        <p className="text-[10px] text-gray-300 dark:text-zinc-700 text-right">
          Health data auto-refreshes every 20 s · last check{" "}
          {Array.isArray(healthData) && healthData.length > 0
            ? fmtTime(Math.max(...healthData.map(h => h.last_checked_at)))
            : "never"}
        </p>
      )}

      {/* Suppress unused import */}
      {void refetch}
    </section>
  );
}

// ── Ollama section (preserved from original, minimal changes) ─────────────────

function ModelInfoPanel({ name, onClose }: { name: string; onClose: () => void }) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ["ollama-model-info", name],
    queryFn:  () => defaultApi.getOllamaModelInfo(name),
    staleTime: 300_000,
    gcTime:    Infinity,
    enabled:   !!name,
    retry: false,
  });

  function fmt(n: number | null | undefined): string {
    if (!n) return "—";
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000)     return `${(n / 1_000_000).toFixed(0)}M`;
    return String(n);
  }

  return (
    <div className="bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md p-3 mt-1 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-mono text-gray-500 dark:text-zinc-400">{name}</span>
        <button onClick={onClose} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors"><XCircle size={12} /></button>
      </div>
      {isLoading && <p className="text-[11px] text-gray-400 dark:text-zinc-600 flex items-center gap-1"><Loader2 size={10} className="animate-spin" /> Loading…</p>}
      {isError   && <p className="text-[11px] text-red-400">Could not load model info.</p>}
      {data && (
        <dl className="grid grid-cols-2 gap-x-4 gap-y-1.5">
          {[
            { Icon: Hash,      label: "Parameters",   value: data.parameter_size },
            { Icon: CpuIcon,   label: "Quantization", value: data.quantization_level },
            { Icon: HardDrive, label: "Size on disk",  value: data.size_human },
            { Icon: FileType,  label: "Family/Format", value: `${data.family} · ${data.format.toUpperCase()}` },
            ...(data.context_length  ? [{ Icon: Layers, label: "Context",    value: `${(data.context_length / 1024).toFixed(0)}K` }] : []),
            ...(data.parameter_count ? [{ Icon: Hash,   label: "Param count", value: fmt(data.parameter_count) }] : []),
          ].map(({ Icon, label, value }) => (
            <div key={label} className="flex items-start gap-1.5">
              <Icon size={10} className="text-gray-400 dark:text-zinc-600 mt-0.5 shrink-0" />
              <div>
                <p className="text-[10px] text-gray-400 dark:text-zinc-600">{label}</p>
                <p className="text-[11px] text-gray-700 dark:text-zinc-300 font-mono">{value}</p>
              </div>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

function OllamaSection() {
  const toast = useToast();
  const [expandedInfo, setExpandedInfo] = useState<string | null>(null);
  const [pullName, setPullName]         = useState("");

  const { data: ollamaData, isLoading: ollamaLoading, error: ollamaError, refetch } = useQuery({
    queryKey: ["ollama-models"],
    queryFn:  () => defaultApi.getOllamaModels(),
    retry: false, staleTime: 30_000,
  });

  const connected = !!ollamaData && !ollamaError;
  const models: string[] = ollamaData?.models ?? [];

  const pullModel = useMutation({
    mutationFn: (model: string) => defaultApi.pullOllamaModel(model),
    onSuccess: (_, model) => { toast.success(`"${model}" downloaded.`); setPullName(""); void refetch(); },
    onError:   (e, model) => toast.error(`Failed to pull "${model}": ${e instanceof Error ? e.message : "error"}`),
  });

  const deleteModel = useMutation({
    mutationFn: (model: string) => defaultApi.deleteOllamaModel(model),
    onSuccess: (_, model) => { toast.success(`"${model}" deleted.`); void refetch(); },
    onError:   (e, model) => toast.error(`Failed to delete "${model}": ${e instanceof Error ? e.message : "error"}`),
  });

  const embedModel   = models.find(m => m.includes("embed")) ?? null;
  const ollamaStatus = ollamaLoading ? "checking" : connected ? "connected" : "offline";
  const statusAccent = ollamaStatus === "connected" ? "emerald" : ollamaStatus === "checking" ? "default" : "red";

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Ollama — Local LLM</p>
        <button onClick={() => refetch()}
          className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors">
          <RefreshCw size={11} /> Refresh
        </button>
      </div>

      <div className="grid grid-cols-3 gap-3">
        <StatCard label="Ollama Status"    value={ollamaStatus}     accent={statusAccent as "default" | "emerald" | "blue" | "red"} sub={connected ? ollamaData.host : "Set OLLAMA_HOST"} />
        <StatCard label="Models Available" value={models.length}    accent={models.length > 0 ? "blue" : "default"} />
        <StatCard label="Embedding Model"  value={embedModel ? "yes" : "none"} accent={embedModel ? "emerald" : "default"} sub={embedModel ?? "no embed model"} />
      </div>

      <div className={clsx(
        "bg-gray-50 dark:bg-zinc-900 border rounded-lg px-4 h-10 flex items-center justify-between",
        connected ? "border-gray-200 dark:border-zinc-800" : "border-gray-200/50 dark:border-zinc-800/50",
      )}>
        <div className="flex items-center gap-2">
          <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", connected ? "bg-emerald-400" : "bg-red-400")} />
          <span className="text-xs text-gray-500 dark:text-zinc-400 font-medium">{connected ? "Connected" : "Not available"}</span>
          {connected && <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-600 ml-1">{ollamaData.host}</span>}
        </div>
        {connected && <span className="text-[11px] text-gray-400 dark:text-zinc-600">{models.length} model{models.length !== 1 ? "s" : ""} installed</span>}
        {!connected && !ollamaLoading && <span className="text-[11px] text-gray-400 dark:text-zinc-600">Set OLLAMA_HOST env var and restart</span>}
      </div>

      {connected && (
        <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
          <div className="flex items-center justify-between px-4 h-9 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950">
            <p className="text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Installed Models</p>
            <p className="text-[10px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider">Info · Delete</p>
          </div>
          {models.length === 0 ? (
            <p className="px-4 py-3 text-[11px] text-gray-400 dark:text-zinc-600 italic">No models installed.</p>
          ) : (
            <div>
              {models.map((m, i) => {
                const isDeleting = deleteModel.isPending && deleteModel.variables === m;
                const isExpanded = expandedInfo === m;
                return (
                  <div key={m} className={clsx("border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0", i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
                    <div className="flex items-center justify-between px-4 h-9 hover:bg-white/5 transition-colors">
                      <span className="text-xs font-mono text-gray-700 dark:text-zinc-300 truncate flex-1">{m}</span>
                      <button onClick={() => setExpandedInfo(isExpanded ? null : m)} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors ml-3" title="Show model info">
                        {isExpanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                      </button>
                      <button
                        onClick={() => { if (confirm(`Delete "${m}"?`)) deleteModel.mutate(m); }}
                        disabled={isDeleting || deleteModel.isPending}
                        className="text-gray-400 dark:text-zinc-600 hover:text-red-400 disabled:opacity-30 transition-colors ml-2"
                      >
                        {isDeleting ? <Loader2 size={12} className="animate-spin" /> : <Trash2 size={12} />}
                      </button>
                    </div>
                    {isExpanded && (
                      <div className="px-4 pb-3">
                        <ModelInfoPanel name={m} onClose={() => setExpandedInfo(null)} />
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
          <div className="border-t border-gray-200 dark:border-zinc-800 px-4 py-3">
            <form onSubmit={(e: FormEvent) => { e.preventDefault(); const name = pullName.trim(); if (name) pullModel.mutate(name); }} className="flex gap-2">
              <div className="relative flex-1">
                <Plus size={11} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
                <input value={pullName} onChange={e => setPullName(e.target.value)} placeholder="Pull model, e.g. llama3.2"
                  disabled={pullModel.isPending}
                  className="w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 pl-7 pr-3 h-8 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 disabled:opacity-50 transition-colors"
                />
              </div>
              <button type="submit" disabled={!pullName.trim() || pullModel.isPending}
                className="flex items-center gap-1.5 px-3 h-8 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 text-white text-xs font-medium transition-colors whitespace-nowrap">
                {pullModel.isPending ? <><Loader2 size={11} className="animate-spin" /> Pulling…</> : <><Download size={11} /> Pull</>}
              </button>
            </form>
            {pullModel.isPending && (
              <p className="mt-1.5 text-[11px] text-indigo-400 flex items-center gap-1.5 animate-pulse">
                <Loader2 size={10} className="animate-spin" />
                Downloading "{pullModel.variables}" — may take several minutes…
              </p>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function ProvidersPage() {
  const qc = useQueryClient();
  const [showAddModal, setShowAddModal] = useState(false);

  const handleCreated = () => {
    void qc.invalidateQueries({ queryKey: ["provider-connections"] });
    void qc.invalidateQueries({ queryKey: ["providers-health"] });
  };

  return (
    <div className="p-6 space-y-6">
      {/* Connections with Add Provider button */}
      <ConnectionsSection onAdd={() => setShowAddModal(true)} />

      {/* Ollama local LLM */}
      <OllamaSection />

      {/* Add Provider slide-over */}
      {showAddModal && (
        <AddProviderModal
          onClose={() => setShowAddModal(false)}
          onCreated={handleCreated}
        />
      )}
    </div>
  );
}

export default ProvidersPage;
