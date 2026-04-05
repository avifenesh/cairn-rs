import { useState, type FormEvent } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import {
  Cpu, CheckCircle2, XCircle, AlertTriangle, Clock,
  RefreshCw, ServerCrash, Plug, Activity,
  Bot, Send, Loader2, Layers, Zap, Trash2, Download, Plus,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";

// ── Provider health type ──────────────────────────────────────────────────────

interface ProviderHealthEntry {
  connection_id: string;
  status: string;
  healthy: boolean;
  last_checked_at: number;
  consecutive_failures: number;
  error_message: string | null;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  if (ms === 0) return "Never";
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

function shortId(id: string): string {
  return id.length > 28 ? `${id.slice(0, 10)}\u2026${id.slice(-8)}` : id;
}

// ── Status badge ──────────────────────────────────────────────────────────────

function StatusBadge({ healthy, status }: { healthy: boolean; status: string }) {
  return (
    <span className={clsx(
      "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-semibold ring-1",
      healthy
        ? "bg-emerald-950 text-emerald-400 ring-emerald-800"
        : "bg-red-950 text-red-400 ring-red-800",
    )}>
      {healthy ? <CheckCircle2 size={11} strokeWidth={2.5} /> : <XCircle size={11} strokeWidth={2.5} />}
      {status || (healthy ? "Healthy" : "Unhealthy")}
    </span>
  );
}

// ── Provider card ─────────────────────────────────────────────────────────────

function ProviderCard({ entry }: { entry: ProviderHealthEntry }) {
  return (
    <div className={clsx(
      "rounded-xl bg-zinc-900 ring-1 p-5 space-y-4 transition-all",
      entry.healthy ? "ring-zinc-800 hover:ring-zinc-700" : "ring-red-900/60 hover:ring-red-800/80",
    )}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <div className={clsx(
            "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg",
            entry.healthy ? "bg-emerald-950 text-emerald-400" : "bg-red-950 text-red-400",
          )}>
            <Cpu size={16} strokeWidth={2} />
          </div>
          <p className="font-mono text-sm font-medium text-zinc-200 truncate">{shortId(entry.connection_id)}</p>
        </div>
        <StatusBadge healthy={entry.healthy} status={entry.status} />
      </div>
      <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
        <dt className="text-zinc-500 flex items-center gap-1"><Clock size={11} /> Last check</dt>
        <dd className="text-zinc-300 text-xs">{fmtTime(entry.last_checked_at)}</dd>
        <dt className="text-zinc-500 flex items-center gap-1"><Activity size={11} /> Failures</dt>
        <dd className={clsx("text-xs font-semibold", entry.consecutive_failures === 0 ? "text-emerald-400" : "text-red-400")}>
          {entry.consecutive_failures} consecutive
        </dd>
      </dl>
      {entry.error_message && (
        <div className="flex items-start gap-2 rounded-lg bg-red-950/40 px-3 py-2 text-xs ring-1 ring-red-900/40">
          <AlertTriangle size={12} className="mt-0.5 shrink-0 text-red-400" />
          <span className="text-red-300 break-words">{entry.error_message}</span>
        </div>
      )}
    </div>
  );
}

// ── Ollama section ────────────────────────────────────────────────────────────

function OllamaSection() {
  const toast = useToast();
  const [prompt, setPrompt] = useState("");
  const [selectedModel, setSelectedModel] = useState("");
  const [pullName, setPullName] = useState("");
  const [result, setResult] = useState<{
    text: string; model: string; tokens_in: number | null;
    tokens_out: number | null; latency_ms: number;
  } | null>(null);

  const { data: ollamaData, isLoading: ollamaLoading, error: ollamaError, refetch } = useQuery({
    queryKey: ["ollama-models"],
    queryFn: () => defaultApi.getOllamaModels(),
    retry: false,
    staleTime: 30_000,
  });

  const connected = !!ollamaData && !ollamaError;
  const models: string[] = ollamaData?.models ?? [];
  const activeModel = selectedModel || models[0] || "";

  const generate = useMutation({
    mutationFn: ({ prompt, model }: { prompt: string; model: string }) =>
      defaultApi.ollamaGenerate({ prompt, model }),
    onSuccess: (data) => setResult(data),
    onError: (e) => toast.error(`Generation failed: ${e instanceof Error ? e.message : "unknown error"}`),
  });

  const pullModel = useMutation({
    mutationFn: (model: string) => defaultApi.pullOllamaModel(model),
    onSuccess: (_, model) => {
      toast.success(`Model "${model}" downloaded successfully.`);
      setPullName("");
      void refetch();
    },
    onError: (e, model) =>
      toast.error(`Failed to pull "${model}": ${e instanceof Error ? e.message : "error"}`),
  });

  const deleteModel = useMutation({
    mutationFn: (model: string) => defaultApi.deleteOllamaModel(model),
    onSuccess: (_, model) => {
      toast.success(`Model "${model}" deleted.`);
      if (selectedModel === model) setSelectedModel("");
      void refetch();
    },
    onError: (e, model) =>
      toast.error(`Failed to delete "${model}": ${e instanceof Error ? e.message : "error"}`),
  });

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!prompt.trim() || !activeModel) return;
    setResult(null);
    generate.mutate({ prompt: prompt.trim(), model: activeModel });
  }

  return (
    <section className="space-y-4">
      {/* ── Section header ─────────────────────────────────────────────── */}
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-zinc-200 flex items-center gap-2">
          <Bot size={15} className="text-indigo-400" />
          Ollama — Local LLM
        </h2>
        <button
          onClick={() => refetch()}
          className="flex items-center gap-1 text-xs text-zinc-500 hover:text-zinc-300 transition-colors"
        >
          <RefreshCw size={11} /> Refresh
        </button>
      </div>

      {/* ── Connection status card ────────────────────────────────────── */}
      <div className={clsx(
        "rounded-xl ring-1 p-4 flex items-center justify-between gap-4",
        connected ? "bg-zinc-900 ring-zinc-800" : "bg-zinc-900/50 ring-zinc-800/50",
      )}>
        <div className="flex items-center gap-3">
          <div className={clsx(
            "w-8 h-8 rounded-lg flex items-center justify-center shrink-0",
            connected ? "bg-indigo-950 text-indigo-400" : "bg-zinc-800 text-zinc-600",
          )}>
            <Bot size={15} />
          </div>
          <div>
            <p className="text-sm font-medium text-zinc-200">
              {ollamaLoading ? "Checking Ollama…" : connected ? "Connected" : "Not available"}
            </p>
            <p className="text-xs text-zinc-500 mt-0.5">
              {ollamaLoading ? (
                "Probing OLLAMA_HOST…"
              ) : connected ? (
                <><span className="font-mono">{ollamaData.host}</span> · {models.length} model{models.length !== 1 ? "s" : ""}</>
              ) : (
                "Set OLLAMA_HOST env var and restart the server"
              )}
            </p>
          </div>
        </div>
        <span className={clsx(
          "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-semibold ring-1 shrink-0",
          connected
            ? "bg-indigo-950 text-indigo-300 ring-indigo-800"
            : "bg-zinc-800 text-zinc-500 ring-zinc-700",
        )}>
          {connected
            ? <><Zap size={10} className="text-indigo-400" /> Connected</>
            : <><XCircle size={10} /> Disconnected</>
          }
        </span>
      </div>

      {/* ── Model management ──────────────────────────────────────────── */}
      {connected && (
        <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-4 space-y-4">
          {/* Header */}
          <div className="flex items-center justify-between">
            <p className="text-xs font-medium text-zinc-400 flex items-center gap-1.5">
              <Layers size={11} className="text-zinc-500" />
              Models
              {models.length > 0 && <span className="text-zinc-600">({models.length})</span>}
            </p>
          </div>

          {/* Installed model list with delete buttons */}
          {models.length > 0 ? (
            <div className="space-y-1.5">
              {models.map((m) => {
                const isDeleting = deleteModel.isPending && deleteModel.variables === m;
                return (
                  <div key={m}
                    className={clsx(
                      "flex items-center justify-between rounded-lg px-3 py-2 ring-1 transition-colors",
                      activeModel === m
                        ? "bg-indigo-950/50 ring-indigo-800/60"
                        : "bg-zinc-800/50 ring-zinc-700/50",
                    )}
                  >
                    <button
                      onClick={() => setSelectedModel(m)}
                      className="text-xs font-mono text-zinc-300 hover:text-zinc-100 transition-colors"
                    >
                      {m}
                    </button>
                    <button
                      onClick={() => {
                        if (confirm(`Delete "${m}"? This removes it from Ollama.`)) {
                          deleteModel.mutate(m);
                        }
                      }}
                      disabled={isDeleting || deleteModel.isPending}
                      title={`Delete ${m}`}
                      className="flex items-center gap-1 text-zinc-600 hover:text-red-400
                                 disabled:opacity-30 transition-colors ml-3"
                    >
                      {isDeleting
                        ? <Loader2 size={12} className="animate-spin" />
                        : <Trash2 size={12} />
                      }
                    </button>
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-xs text-zinc-600 italic">No models installed yet.</p>
          )}

          {/* Pull model form */}
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const name = pullName.trim();
              if (name) pullModel.mutate(name);
            }}
            className="flex gap-2 pt-1 border-t border-zinc-800"
          >
            <div className="relative flex-1">
              <Plus size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
              <input
                value={pullName}
                onChange={(e) => setPullName(e.target.value)}
                placeholder="Pull model, e.g. llama3.2"
                disabled={pullModel.isPending}
                className="w-full rounded-lg bg-zinc-800 border border-zinc-700 pl-7 pr-3 py-2
                           text-xs text-zinc-200 placeholder-zinc-600
                           focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                           disabled:opacity-50 transition-colors"
              />
            </div>
            <button
              type="submit"
              disabled={!pullName.trim() || pullModel.isPending}
              className="flex items-center gap-1.5 px-3 py-2 rounded-lg bg-indigo-600 hover:bg-indigo-500
                         disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-xs font-medium
                         transition-colors whitespace-nowrap"
            >
              {pullModel.isPending
                ? <><Loader2 size={12} className="animate-spin" /> Pulling…</>
                : <><Download size={12} /> Pull</>
              }
            </button>
          </form>
          {pullModel.isPending && (
            <p className="text-[11px] text-indigo-400 flex items-center gap-1.5 animate-pulse">
              <Loader2 size={10} className="animate-spin" />
              Downloading "{pullModel.variables}" — this may take several minutes…
            </p>
          )}
        </div>
      )}

      {/* ── Chat test input ───────────────────────────────────────────── */}
      {connected && (
        <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-4 space-y-3">
          <p className="text-xs font-medium text-zinc-400 flex items-center gap-1.5">
            <Send size={11} className="text-zinc-500" />
            Test prompt
            {activeModel && (
              <span className="ml-auto font-mono text-zinc-600">{activeModel}</span>
            )}
          </p>

          <form onSubmit={handleSubmit} className="flex gap-2">
            <input
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Type a prompt and press Send…"
              disabled={generate.isPending}
              className="flex-1 rounded-lg bg-zinc-800 border border-zinc-700 px-3 py-2 text-sm
                         text-zinc-100 placeholder-zinc-600 focus:outline-none
                         focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition"
            />
            <button
              type="submit"
              disabled={!prompt.trim() || !activeModel || generate.isPending}
              className="px-3 py-2 rounded-lg bg-indigo-600 hover:bg-indigo-500
                         disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-sm
                         flex items-center gap-1.5 transition"
            >
              {generate.isPending
                ? <Loader2 size={14} className="animate-spin" />
                : <Send size={14} />
              }
              {generate.isPending ? "Generating…" : "Send"}
            </button>
          </form>

          {/* Response */}
          {result && (
            <div className="space-y-2">
              <div className="rounded-lg bg-zinc-800/60 px-4 py-3 text-sm text-zinc-200 leading-relaxed whitespace-pre-wrap">
                {result.text}
              </div>
              <div className="flex gap-4 text-[11px] text-zinc-600 font-mono">
                <span>model: <span className="text-zinc-400">{result.model}</span></span>
                <span>latency: <span className="text-zinc-400">{result.latency_ms}ms</span></span>
                {result.tokens_in != null && (
                  <span>tokens: <span className="text-zinc-400">{result.tokens_in}→{result.tokens_out}</span></span>
                )}
              </div>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

// ── Summary strip ─────────────────────────────────────────────────────────────

function SummaryStrip({ entries }: { entries: ProviderHealthEntry[] }) {
  const healthy = entries.filter((e) => e.healthy).length;
  const unhealthy = entries.length - healthy;
  return (
    <div className="flex items-center gap-6 rounded-xl bg-zinc-900 ring-1 ring-zinc-800 px-5 py-3">
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 rounded-full bg-emerald-400" />
        <span className="text-sm text-zinc-300">
          <span className="font-semibold text-emerald-400">{healthy}</span> healthy
        </span>
      </div>
      {unhealthy > 0 && (
        <div className="flex items-center gap-2">
          <span className="h-2 w-2 rounded-full bg-red-400" />
          <span className="text-sm text-zinc-300">
            <span className="font-semibold text-red-400">{unhealthy}</span> degraded
          </span>
        </div>
      )}
      <div className="ml-auto text-xs text-zinc-600">
        {entries.length} connection{entries.length !== 1 ? "s" : ""} total
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function ProvidersPage() {
  const { data, isLoading, isError, error, refetch, dataUpdatedAt } = useQuery({
    queryKey: ["providers-health"],
    queryFn: () => defaultApi.getProviderHealth() as Promise<ProviderHealthEntry[]>,
    refetchInterval: 20_000,
  });

  const entries: ProviderHealthEntry[] = Array.isArray(data) ? data : [];
  const lastUpdated = dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : null;

  return (
    <div className="space-y-8">
      {/* ── Cairn provider health ──────────────────────────────────────── */}
      <section className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold text-zinc-200 flex items-center gap-2">
            <Cpu size={15} className="text-blue-400" />
            Provider Health
            <span className="text-zinc-600 font-normal text-xs">({entries.length})</span>
          </h2>
          <div className="flex items-center gap-3">
            {lastUpdated && (
              <span className="text-xs text-zinc-600 flex items-center gap-1">
                <Clock size={11} /> {lastUpdated}
              </span>
            )}
            <button
              onClick={() => refetch()}
              className="flex items-center gap-1.5 rounded-lg bg-zinc-800 px-2.5 py-1.5 text-xs text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200 transition-colors"
            >
              <RefreshCw size={12} /> Refresh
            </button>
          </div>
        </div>

        {isError && (
          <div className="flex flex-col items-center justify-center min-h-32 gap-3 p-8 text-center rounded-xl ring-1 ring-zinc-800">
            <ServerCrash size={32} className="text-red-500" />
            <p className="text-sm text-zinc-400">{error instanceof Error ? error.message : "Failed to load"}</p>
          </div>
        )}

        {isLoading && (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {[1, 2].map((i) => (
              <div key={i} className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5 animate-pulse space-y-3 h-32" />
            ))}
          </div>
        )}

        {!isLoading && !isError && entries.length > 0 && (
          <>
            <SummaryStrip entries={entries} />
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {entries.map((entry) => (
                <ProviderCard key={entry.connection_id} entry={entry} />
              ))}
            </div>
          </>
        )}

        {!isLoading && !isError && entries.length === 0 && (
          <div className="flex flex-col items-center justify-center py-12 text-center rounded-xl ring-1 ring-zinc-800/50">
            <Plug size={32} className="text-zinc-700 mb-3" />
            <p className="text-sm text-zinc-500">No provider connections registered</p>
          </div>
        )}
      </section>

      {/* ── Ollama local LLM ──────────────────────────────────────────── */}
      <OllamaSection />
    </div>
  );
}

export default ProvidersPage;
