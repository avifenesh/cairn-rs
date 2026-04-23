import { useState, type FormEvent, useId } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  RefreshCw, ServerCrash, Loader2, Plus, Trash2, ChevronDown, ChevronRight,
  HardDrive, Zap, XCircle, Pencil,
  Globe, Server, Check, X, Settings, Tag,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";
import { sectionLabel } from "../lib/design-system";
import { useScope } from "../hooks/useScope";
import type { ProviderConnectionRecord, ProviderHealthEntry } from "../lib/types";

// ── Types ─────────────────────────────────────────────────────────────────────

type ProviderKind =
  | "openai" | "anthropic" | "ollama" | "deepseek" | "xai" | "google"
  | "groq" | "azure-openai" | "openrouter" | "minimax" | "bedrock" | "bedrock-compat"
  | "openai-compatible";

interface ProviderKindMeta {
  label: string;
  description: string;
  icon: React.ReactNode;
  defaultFamily: string;
  defaultAdapter: string;
  defaultUrl: string;
  defaultModel: string;
}

const PROVIDER_KINDS: Record<ProviderKind, ProviderKindMeta> = {
  openai: {
    label: "OpenAI",
    description: "GPT-4, GPT-4.1, o-series models via api.openai.com.",
    icon: <Zap size={16} />,
    defaultFamily: "openai",
    defaultAdapter: "openai",
    defaultUrl: "https://api.openai.com/v1",
    defaultModel: "gpt-4.1-nano",
  },
  anthropic: {
    label: "Anthropic",
    description: "Claude models via api.anthropic.com.",
    icon: <Zap size={16} />,
    defaultFamily: "anthropic",
    defaultAdapter: "anthropic",
    defaultUrl: "https://api.anthropic.com/v1",
    defaultModel: "claude-sonnet-4-6",
  },
  ollama: {
    label: "Ollama",
    description: "Local self-hosted models via Ollama.",
    icon: <HardDrive size={16} />,
    defaultFamily: "ollama",
    defaultAdapter: "ollama",
    defaultUrl: "http://localhost:11434/v1",
    defaultModel: "llama3.2:3b",
  },
  deepseek: {
    label: "DeepSeek",
    description: "DeepSeek reasoning and chat models.",
    icon: <Zap size={16} />,
    defaultFamily: "deepseek",
    defaultAdapter: "deepseek",
    defaultUrl: "https://api.deepseek.com/v1",
    defaultModel: "deepseek-chat",
  },
  xai: {
    label: "xAI",
    description: "Grok models via api.x.ai.",
    icon: <Zap size={16} />,
    defaultFamily: "xai",
    defaultAdapter: "xai",
    defaultUrl: "https://api.x.ai/v1",
    defaultModel: "grok-3-mini",
  },
  google: {
    label: "Google Gemini",
    description: "Gemini models via Google AI Studio or Vertex AI.",
    icon: <Globe size={16} />,
    defaultFamily: "google",
    defaultAdapter: "google",
    defaultUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    defaultModel: "gemini-2.5-flash",
  },
  groq: {
    label: "Groq",
    description: "Ultra-fast inference for open models.",
    icon: <Zap size={16} />,
    defaultFamily: "groq",
    defaultAdapter: "groq",
    defaultUrl: "https://api.groq.com/openai/v1",
    defaultModel: "llama-3.3-70b-versatile",
  },
  "azure-openai": {
    label: "Azure OpenAI",
    description: "OpenAI models hosted on your Azure subscription.",
    icon: <Server size={16} />,
    defaultFamily: "azure-openai",
    defaultAdapter: "azure-openai",
    defaultUrl: "",
    defaultModel: "gpt-4.1",
  },
  openrouter: {
    label: "OpenRouter",
    description: "Access 200+ models via one endpoint. Free tier available.",
    icon: <Globe size={16} />,
    defaultFamily: "openrouter",
    defaultAdapter: "openrouter",
    defaultUrl: "https://openrouter.ai/api/v1",
    defaultModel: "openrouter/auto",
  },
  minimax: {
    label: "MiniMax",
    description: "MiniMax M1 and M2.5 models.",
    icon: <Zap size={16} />,
    defaultFamily: "minimax",
    defaultAdapter: "minimax",
    defaultUrl: "https://api.minimaxi.chat/v1",
    defaultModel: "MiniMax-M1",
  },
  bedrock: {
    label: "AWS Bedrock (Converse)",
    description: "Full-featured Converse API — guardrails, documents, native tool_config.",
    icon: <Server size={16} />,
    defaultFamily: "bedrock",
    defaultAdapter: "bedrock",
    defaultUrl: "https://bedrock-runtime.us-east-1.amazonaws.com",
    defaultModel: "us.anthropic.claude-sonnet-4-6",
  },
  "bedrock-compat": {
    label: "AWS Bedrock (OpenAI/Mantle)",
    description: "Bedrock Mantle gateway — OpenAI Responses API + Chat Completions format.",
    icon: <Server size={16} />,
    defaultFamily: "bedrock-compat",
    defaultAdapter: "bedrock-compat",
    defaultUrl: "https://bedrock-mantle.us-east-1.api.aws/v1",
    defaultModel: "us.anthropic.claude-sonnet-4-6",
  },
  "openai-compatible": {
    label: "OpenAI-Compatible",
    description: "Any endpoint that speaks the /chat/completions format. You supply the URL and key.",
    icon: <Settings size={16} />,
    defaultFamily: "openai-compatible",
    defaultAdapter: "openai-compatible",
    defaultUrl: "",
    defaultModel: "",
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

// ── Model settings popover ────────────────────────────────────────────────────

function ModelSettingsRow({ connectionId, modelId }: { connectionId: string; modelId: string }) {
  const toast       = useToast();
  const [scope]     = useScope();
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
    if (contextWindow.trim())   writes.push(defaultApi.setDefaultSetting("tenant", scope.tenant_id, `${base}:context_window`,   parseInt(contextWindow, 10)));
    if (maxOutputTokens.trim()) writes.push(defaultApi.setDefaultSetting("tenant", scope.tenant_id, `${base}:max_output_tokens`, parseInt(maxOutputTokens, 10)));
    if (temperature.trim())     writes.push(defaultApi.setDefaultSetting("tenant", scope.tenant_id, `${base}:temperature`,       parseFloat(temperature)));
    writes.push(defaultApi.setDefaultSetting("tenant", scope.tenant_id, `${base}:thinking_mode`, thinking));
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
        className="flex items-center gap-1 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors px-1.5 py-0.5 rounded"
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
            <button onClick={() => setOpen(false)} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400">
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
                <span className="text-[10px] text-gray-300 dark:text-zinc-600 italic">default ≈ {fmtK(Math.round(ctxNum / 4))}</span>
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
  onUpdated,
}: {
  record: ProviderConnectionRecord;
  health?: ProviderHealthEntry;
  even: boolean;
  onDelete: (id: string) => void;
  onUpdated: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editFamily, setEditFamily] = useState(record.provider_family);
  const [editAdapter, setEditAdapter] = useState(record.adapter_type);
  const [editModels, setEditModels] = useState(record.supported_models.join(", "));
  const [editEndpoint, setEditEndpoint] = useState("");
  const [saving, setSaving] = useState(false);
  const isHealthy = health?.healthy ?? null;
  const toast = useToast();

  const startEdit = () => {
    setEditFamily(record.provider_family);
    setEditAdapter(record.adapter_type);
    setEditModels(record.supported_models.join(", "));
    setEditEndpoint("");
    setEditing(true);
  };

  const saveEdit = async () => {
    setSaving(true);
    try {
      await defaultApi.updateProviderConnection(record.provider_connection_id, {
        provider_family: editFamily.trim(),
        adapter_type: editAdapter.trim(),
        supported_models: editModels.split(",").map(m => m.trim()).filter(Boolean),
        ...(editEndpoint.trim() ? { endpoint_url: editEndpoint.trim() } : {}),
      });
      toast.success(`Updated ${record.provider_connection_id}`);
      setEditing(false);
      onUpdated();
    } catch (e) {
      toast.error(`Update failed: ${e instanceof Error ? e.message : "error"}`);
    } finally {
      setSaving(false);
    }
  };

  const testConn = useMutation({
    mutationFn: () => defaultApi.testConnection(record.provider_connection_id),
    onSuccess: (r) => {
      if (r.ok) {
        toast.success(`${record.provider_connection_id} — reachable (${r.latency_ms}ms)`);
      } else {
        toast.error(`${record.provider_connection_id} — ${r.detail} (HTTP ${r.status})`);
      }
    },
    onError: () => toast.error(`Failed to test ${record.provider_connection_id}`),
  });

  const discoverConn = useMutation({
    mutationFn: async () => {
      const ids = await defaultApi.discoverModelIds(record.provider_connection_id);
      if (ids.length === 0) {
        throw new Error("provider returned an empty model list");
      }
      await defaultApi.updateProviderConnection(record.provider_connection_id, {
        supported_models: ids,
      });
      return ids;
    },
    onSuccess: (ids) => {
      toast.success(`Discovered ${ids.length} model(s) on ${record.provider_connection_id}.`);
      onUpdated();
    },
    onError: (e) =>
      toast.error(`Discover failed on ${record.provider_connection_id}: ${e instanceof Error ? e.message : "error"}`),
  });

  const familyColor: Record<string, string> = {
    openai:                "text-sky-400 bg-sky-950/40 border-sky-800/40",
    anthropic:             "text-violet-400 bg-violet-950/40 border-violet-800/40",
    ollama:                "text-amber-400 bg-amber-950/40 border-amber-800/40",
    google:                "text-emerald-400 bg-emerald-950/40 border-emerald-800/40",
    groq:                  "text-cyan-400 bg-cyan-950/40 border-cyan-800/40",
    openrouter:            "text-indigo-400 bg-indigo-950/40 border-indigo-800/40",
    bedrock:               "text-orange-400 bg-orange-950/40 border-orange-800/40",
    "bedrock-compat":      "text-orange-400 bg-orange-950/40 border-orange-800/40",
    "azure-openai":        "text-blue-400 bg-blue-950/40 border-blue-800/40",
    "openai-compatible":   "text-gray-400 dark:text-zinc-500 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700",
    custom:                "text-gray-500 dark:text-zinc-400 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700",
  };

  const familyClass = familyColor[record.provider_family] ?? familyColor.custom;

  return (
    <>
      <tr data-testid={`provider-row-${record.provider_connection_id}`} className={clsx("border-b border-gray-200/50 dark:border-zinc-800/50 hover:bg-white/5 transition-colors", even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
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
              <span
                className="text-[10px] text-amber-500 italic"
                title="This connection won't serve any chat/generate calls until models are registered. Click Discover or edit the row."
              >
                no models registered
              </span>
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
              onClick={() => testConn.mutate()}
              disabled={testConn.isPending}
              className={clsx(
                "flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium transition-colors",
                testConn.isPending
                  ? "text-gray-400 dark:text-zinc-600"
                  : "text-indigo-400 hover:text-indigo-300 hover:bg-indigo-950/30",
              )}
              title="Test connection reachability"
            >
              {testConn.isPending ? <Loader2 size={10} className="animate-spin" /> : <Zap size={10} />}
              Test
            </button>
            <button
              onClick={() => discoverConn.mutate()}
              disabled={discoverConn.isPending}
              className={clsx(
                "flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium transition-colors",
                discoverConn.isPending
                  ? "text-gray-400 dark:text-zinc-600"
                  : record.supported_models.length === 0
                    ? "text-amber-500 hover:text-amber-400 hover:bg-amber-950/30"
                    : "text-emerald-400 hover:text-emerald-300 hover:bg-emerald-950/30",
              )}
              title={record.supported_models.length === 0
                ? "No models registered — click to auto-discover from the provider"
                : "Re-run discovery and replace supported_models"}
            >
              {discoverConn.isPending ? <Loader2 size={10} className="animate-spin" /> : <RefreshCw size={10} />}
              Discover
            </button>
            <button
              onClick={() => setExpanded(v => !v)}
              className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
              title={expanded ? "Collapse" : "Expand models"}
            >
              {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
            </button>
            <button
              onClick={startEdit}
              className="text-gray-400 dark:text-zinc-600 hover:text-indigo-400 transition-colors"
              title="Edit connection"
            >
              <Pencil size={12} />
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

      {/* Inline edit form */}
      {editing && (
        <tr className="border-b border-indigo-500/30 bg-indigo-950/10">
          <td colSpan={6} className="px-4 py-3">
            <div className="grid grid-cols-4 gap-3">
              <label className="block">
                <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Family</span>
                <input value={editFamily} onChange={e => setEditFamily(e.target.value)}
                  className="mt-1 w-full rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500" />
              </label>
              <label className="block">
                <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Adapter</span>
                <input value={editAdapter} onChange={e => setEditAdapter(e.target.value)}
                  className="mt-1 w-full rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500" />
              </label>
              <label className="block">
                <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Models (comma-sep)</span>
                <input value={editModels} onChange={e => setEditModels(e.target.value)}
                  className="mt-1 w-full rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500" />
              </label>
              <label className="block">
                <span className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Endpoint URL</span>
                <input value={editEndpoint} onChange={e => setEditEndpoint(e.target.value)}
                  placeholder="leave blank to keep current"
                  className="mt-1 w-full rounded bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-2 py-1.5 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500" />
              </label>
            </div>
            <div className="flex items-center gap-2 mt-3">
              <button onClick={saveEdit} disabled={saving}
                className="flex items-center gap-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 px-3 py-1.5 text-[11px] font-medium text-white transition-colors disabled:opacity-50">
                {saving ? <Loader2 size={11} className="animate-spin" /> : <Check size={11} />}
                Save
              </button>
              <button onClick={() => setEditing(false)}
                className="rounded-md border border-gray-200 dark:border-zinc-700 px-3 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-600 dark:hover:text-zinc-300 transition-colors">
                Cancel
              </button>
            </div>
          </td>
        </tr>
      )}

      {/* Expanded: per-model settings */}
      {expanded && record.supported_models.length > 0 && (
        <tr className={clsx("border-b border-gray-200/50 dark:border-zinc-800/50", even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
          <td colSpan={6} className="px-4 pb-3 pt-1">
            <div className="bg-white dark:bg-zinc-950 rounded-md border border-gray-200 dark:border-zinc-800">
              <div className="px-3 py-1.5 border-b border-gray-200 dark:border-zinc-800 flex items-center gap-2">
                <span className="text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Models</span>
                <span className="text-[10px] text-gray-300 dark:text-zinc-600">· defaults are tenant-scoped</span>
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
  const [scope] = useScope();
  const formId = useId();

  // Step: 0=type, 1=form, 2=models
  const [step, setStep] = useState<0 | 1 | 2>(0);
  const [kind, setKind] = useState<ProviderKind>("openai");

  const meta = PROVIDER_KINDS[kind];

  // Form fields
  const [connectionId, setConnectionId] = useState(() => genConnectionId("openai"));
  const [baseUrl,      setBaseUrl]       = useState(meta.defaultUrl);
  const [apiKey,       setApiKey]        = useState("");
  const [family,       setFamily]        = useState(meta.defaultFamily);
  const [adapter,      setAdapter]       = useState(meta.defaultAdapter);

  // Model entry
  const [models, setModels]     = useState<string[]>([meta.defaultModel].filter(Boolean));
  const [modelInput, setModelInput] = useState("");

  const selectKind = (k: ProviderKind) => {
    setKind(k);
    const m = PROVIDER_KINDS[k];
    setConnectionId(genConnectionId(m.defaultFamily));
    setBaseUrl(m.defaultUrl);
    setFamily(m.defaultFamily);
    setAdapter(m.defaultAdapter);
    setModels(m.defaultModel ? [m.defaultModel] : []);
  };

  const addModel = () => {
    const v = modelInput.trim();
    if (v && !models.includes(v)) setModels(prev => [...prev, v]);
    setModelInput("");
  };

  const removeModel = (m: string) => setModels(prev => prev.filter(x => x !== m));

  const createMutation = useMutation({
    mutationFn: async () => {
      let credentialId: string | undefined;
      if (apiKey.trim()) {
        const stored = await defaultApi.storeCredential(scope.tenant_id, {
          provider_id: connectionId.trim(),
          plaintext_value: apiKey,
        });
        credentialId = stored.id;
      }

      const created = await defaultApi.createProviderConnection({
        tenant_id: scope.tenant_id,
        provider_connection_id: connectionId.trim(),
        provider_family: family.trim(),
        adapter_type: adapter.trim(),
        supported_models: models,
        credential_id: credentialId,
        endpoint_url: baseUrl.trim() || undefined,
      });

      // Issue #156 / closed-loop UX: if the operator didn't enter any
      // models manually, kick off auto-discovery against the freshly
      // registered connection. A connection with empty supported_models
      // silently fails every chat/generate resolve — so we refuse to
      // leave the wizard in that state without at least trying to fill
      // it in. Failure is non-fatal (auth not yet active, endpoint
      // temporarily unreachable, etc.): surface a warning toast and let
      // the operator add IDs by hand on the row.
      if (models.length === 0) {
        try {
          const discovered = await defaultApi.discoverModelIds(connectionId.trim());
          if (discovered.length > 0) {
            await defaultApi.updateProviderConnection(connectionId.trim(), {
              supported_models: discovered,
            });
            toast.success(`Discovered ${discovered.length} model(s) on "${connectionId}".`);
          } else {
            toast.warning(
              `No models discovered on "${connectionId}" — add IDs manually on the row.`,
            );
          }
        } catch (e) {
          toast.warning(
            `Discover failed on "${connectionId}" (${e instanceof Error ? e.message : "error"}). Add model IDs manually on the row.`,
          );
        }
      }

      return created;
    },
    onSuccess: () => {
      toast.success(`Provider "${connectionId}" registered.`);
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
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors">
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
                    ? "text-gray-500 dark:text-zinc-400 cursor-pointer hover:text-gray-700 dark:hover:text-zinc-300"
                    : "text-gray-300 dark:text-zinc-600",
              )}
                onClick={() => i < step && setStep(i as 0 | 1 | 2)}
              >
                <span className={clsx(
                  "w-4 h-4 rounded-full flex items-center justify-center text-[10px] font-semibold",
                  i === step ? "bg-indigo-600 text-white" : i < step ? "bg-gray-200 dark:bg-zinc-700 text-gray-700 dark:text-zinc-300" : "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-600",
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
            <div>
              <p className="text-[12px] text-gray-400 dark:text-zinc-500 mb-4">
                Choose the provider you want to connect.
              </p>

              <div className="grid grid-cols-2 gap-2">
              {(Object.entries(PROVIDER_KINDS) as [ProviderKind, ProviderKindMeta][]).map(([k, m]) => (
                <button
                  key={k}
                  onClick={() => { selectKind(k); setStep(1); }}
                  className={clsx(
                    "flex items-start gap-2 p-3 rounded-lg border text-left transition-colors",
                    kind === k
                      ? "border-indigo-500/60 bg-indigo-950/30"
                      : "border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 hover:border-gray-300 dark:hover:border-zinc-700 hover:bg-gray-100/40 dark:hover:bg-zinc-800/40",
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
            </div>
          )}

          {/* ── Step 1: Connection form ── */}
          {step === 1 && (
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

              {kind !== "openai-compatible" && (
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

              {kind !== "ollama" && (
                <label className="block">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide">API Key</span>
                  <input
                    type="password"
                    value={apiKey}
                    onChange={e => setApiKey(e.target.value)}
                    placeholder="sk-… or $ENV_VAR_NAME"
                    autoComplete="off"
                    className="mt-1.5 w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
                  />
                  <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">
                    Paste a key directly, or prefix with <code className="text-[10px] font-mono text-indigo-400">$</code> to reference an env var (e.g. <code className="text-[10px] font-mono text-indigo-400">$BEDROCK_API_KEY</code>).
                  </p>
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
                  Enter the model IDs served through this connection, or leave this blank
                  — we will call the provider&apos;s <code className="text-gray-500 dark:text-zinc-400 font-mono text-[10px]">/models</code> endpoint right after registration and fill it in automatically.
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
                      kind === "openrouter"    ? "e.g. openai/gpt-4o, anthropic/claude-3.5-sonnet" :
                      kind === "openai" ? "e.g. gpt-4.1-nano, gpt-4o" :
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

              {/* Discovery notice */}
              <div className="flex items-center gap-2 px-3 py-2 rounded-md bg-gray-50/60 dark:bg-zinc-900/60 border border-gray-200/60 dark:border-zinc-800/60 border-dashed">
                <Zap size={12} className="text-emerald-400 shrink-0" />
                <span className="text-[11px] text-gray-500 dark:text-zinc-400">
                  Auto-discovery runs on registration when this list is empty — calls{" "}
                  <code className="text-gray-400 dark:text-zinc-500 font-mono text-[10px]">GET /v1/providers/connections/:id/discover-models</code>.
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
                <p className="text-[11px] text-gray-300 dark:text-zinc-600 italic">
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
            className="text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors px-3 py-1.5"
          >
            {step === 0 ? "Cancel" : "← Back"}
          </button>

          {step < 2 ? (
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
    queryFn:  () => defaultApi.listProviderConnections(),
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

  const handleDelete = (id: string) => {
    if (!confirm(`Delete connection "${id}"?`)) return;
    defaultApi
      .deleteProviderConnection(id)
      .then(() => {
        toast.success(`Connection ${id} deleted.`);
        refetch();
      })
      .catch((error: unknown) => {
        toast.error(error instanceof Error ? error.message : "Delete failed.");
      });
  };

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <p className={clsx(sectionLabel, "mb-0")}>
            Provider Connections
          </p>
          {entries.length > 0 && (
            <span className="text-[11px] text-gray-300 dark:text-zinc-600">({entries.length})</span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {dataUpdatedAt > 0 && (
            <span className="text-[11px] font-mono text-gray-300 dark:text-zinc-600">
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
            data-testid="add-provider-btn"
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
          <StatCard label="Healthy"  value={healthy}   variant="success" />
          <StatCard label="Degraded" value={unhealthy}  variant={unhealthy > 0 ? "danger" : "default"} />
        </div>
      )}

      {/* Table */}
      <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg">
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
            <Server size={24} className="text-gray-300 dark:text-zinc-600 mx-auto" />
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
                  onUpdated={refetch}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* Legacy health table header note */}
      {entries.length > 0 && healthData && (
        <p className="text-[10px] text-gray-300 dark:text-zinc-600 text-right">
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
      <div className="space-y-1">
        <p className={clsx(sectionLabel, "mb-0")}>
          Providers
        </p>
        <p className="text-[12px] text-gray-500 dark:text-zinc-400">
          Only real provider connections registered for the current scope appear here.
        </p>
      </div>

      {/* User-created connections with Add Provider button */}
      <ConnectionsSection onAdd={() => setShowAddModal(true)} />

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
