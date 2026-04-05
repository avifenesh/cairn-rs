import { useState, type FormEvent } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import {
  RefreshCw, ServerCrash, Loader2,
  Download, Trash2, Plus,
  ChevronDown, ChevronRight,
  HardDrive, Cpu as CpuIcon, Hash, FileType, Layers,
  XCircle,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";

// ── Types ─────────────────────────────────────────────────────────────────────

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

// ── Stat card (left-border, no icon) ─────────────────────────────────────────

function StatCard({ label, value, sub, accent = "default" }: {
  label: string; value: string | number; sub?: string;
  accent?: "default" | "emerald" | "blue" | "red";
}) {
  const borders = { default: "border-l-zinc-700", emerald: "border-l-emerald-500", blue: "border-l-indigo-500", red: "border-l-red-500" };
  const values  = { default: "text-zinc-100", emerald: "text-emerald-400", blue: "text-indigo-400", red: "text-red-400" };
  return (
    <div className={clsx("bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4", borders[accent])}>
      <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2">{label}</p>
      <p className={clsx("text-2xl font-semibold tabular-nums", values[accent])}>{value}</p>
      {sub && <p className="mt-1 text-[11px] text-zinc-600">{sub}</p>}
    </div>
  );
}

// ── Provider health table row ─────────────────────────────────────────────────

function ProviderRow({ entry, even }: { entry: ProviderHealthEntry; even: boolean }) {
  return (
    <tr className={clsx("border-b border-zinc-800/50 hover:bg-white/5 transition-colors", even ? "bg-zinc-900" : "bg-zinc-900/50")}>
      <td className="px-4 h-9">
        <div className="flex items-center gap-1.5">
          <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0",
            entry.healthy ? "bg-emerald-400" : "bg-red-400 animate-pulse")} />
          <span className={clsx("text-[11px] font-medium", entry.healthy ? "text-emerald-400" : "text-red-400")}>
            {entry.healthy ? "Healthy" : entry.status || "Unhealthy"}
          </span>
        </div>
      </td>
      <td className="px-4 h-9 font-mono text-xs text-zinc-300 max-w-[200px] truncate" title={entry.connection_id}>
        {entry.connection_id.length > 24 ? `${entry.connection_id.slice(0, 10)}…${entry.connection_id.slice(-8)}` : entry.connection_id}
      </td>
      <td className="px-4 h-9 text-[11px] text-zinc-500 font-mono">{fmtTime(entry.last_checked_at)}</td>
      <td className="px-4 h-9 text-[11px] tabular-nums">
        <span className={entry.consecutive_failures > 0 ? "text-red-400" : "text-zinc-600"}>
          {entry.consecutive_failures}
        </span>
      </td>
      <td className="px-4 h-9 text-[11px] text-red-400 font-mono truncate max-w-[160px]">
        {entry.error_message ?? "—"}
      </td>
    </tr>
  );
}

// ── Model info panel ──────────────────────────────────────────────────────────

function ModelInfoPanel({ name, onClose }: { name: string; onClose: () => void }) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ["ollama-model-info", name],
    queryFn:  () => defaultApi.getOllamaModelInfo(name),
    staleTime: 120_000,
    retry: false,
  });

  function fmt(n: number | null | undefined): string {
    if (!n) return "—";
    if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
    if (n >= 1_000_000)     return `${(n / 1_000_000).toFixed(0)}M`;
    return String(n);
  }

  return (
    <div className="bg-zinc-950 border border-zinc-800 rounded-md p-3 mt-1 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-mono text-zinc-400">{name}</span>
        <button onClick={onClose} className="text-zinc-600 hover:text-zinc-400 transition-colors">
          <XCircle size={12} />
        </button>
      </div>
      {isLoading && <p className="text-[11px] text-zinc-600 flex items-center gap-1"><Loader2 size={10} className="animate-spin" /> Loading…</p>}
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
              <Icon size={10} className="text-zinc-600 mt-0.5 shrink-0" />
              <div>
                <p className="text-[10px] text-zinc-600">{label}</p>
                <p className="text-[11px] text-zinc-300 font-mono">{value}</p>
              </div>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

// ── Ollama section ────────────────────────────────────────────────────────────

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

  // Derived stat card values
  const embedModel   = models.find(m => m.includes("embed")) ?? null;
  const ollamaStatus = ollamaLoading ? "checking" : connected ? "connected" : "offline";
  const statusAccent = ollamaStatus === "connected" ? "emerald" : ollamaStatus === "checking" ? "default" : "red";

  return (
    <section className="space-y-4">
      {/* Section header */}
      <div className="flex items-center justify-between">
        <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Ollama — Local LLM</p>
        <button onClick={() => refetch()}
          className="flex items-center gap-1 text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors">
          <RefreshCw size={11} /> Refresh
        </button>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-3 gap-3">
        <StatCard label="Ollama Status"    value={ollamaStatus}     accent={statusAccent as "default" | "emerald" | "blue" | "red"} sub={connected ? ollamaData.host : "Set OLLAMA_HOST"} />
        <StatCard label="Models Available" value={models.length}    accent={models.length > 0 ? "blue" : "default"} />
        <StatCard label="Embedding Model"  value={embedModel ? "yes" : "none"} accent={embedModel ? "emerald" : "default"} sub={embedModel ?? "no embed model"} />
      </div>

      {/* Connection info row */}
      <div className={clsx(
        "bg-zinc-900 border rounded-lg px-4 h-10 flex items-center justify-between",
        connected ? "border-zinc-800" : "border-zinc-800/50",
      )}>
        <div className="flex items-center gap-2">
          <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0", connected ? "bg-emerald-400" : "bg-red-400")} />
          <span className="text-xs text-zinc-400 font-medium">{connected ? "Connected" : "Not available"}</span>
          {connected && (
            <span className="text-[11px] font-mono text-zinc-600 ml-1">{ollamaData.host}</span>
          )}
        </div>
        {connected && (
          <span className="text-[11px] text-zinc-600">
            {models.length} model{models.length !== 1 ? "s" : ""} installed
          </span>
        )}
        {!connected && !ollamaLoading && (
          <span className="text-[11px] text-zinc-600">Set OLLAMA_HOST env var and restart</span>
        )}
      </div>

      {/* Model list */}
      {connected && (
        <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-4 h-9 border-b border-zinc-800 bg-zinc-950">
            <p className="text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Installed Models</p>
            <p className="text-[10px] text-zinc-700 uppercase tracking-wider">Info · Delete</p>
          </div>

          {models.length === 0 ? (
            <p className="px-4 py-3 text-[11px] text-zinc-600 italic">No models installed.</p>
          ) : (
            <div>
              {models.map((m, i) => {
                const isDeleting = deleteModel.isPending && deleteModel.variables === m;
                const isExpanded = expandedInfo === m;
                return (
                  <div key={m} className={clsx("border-b border-zinc-800/50 last:border-0", i % 2 === 0 ? "bg-zinc-900" : "bg-zinc-900/50")}>
                    <div className="flex items-center justify-between px-4 h-9 hover:bg-white/5 transition-colors">
                      <span className="text-xs font-mono text-zinc-300 truncate flex-1">{m}</span>
                      {/* Info toggle */}
                      <button
                        onClick={() => setExpandedInfo(isExpanded ? null : m)}
                        className="text-zinc-600 hover:text-zinc-400 transition-colors ml-3"
                        title="Show model info"
                      >
                        {isExpanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                      </button>
                      {/* Delete */}
                      <button
                        onClick={() => {
                          if (confirm(`Delete "${m}"?`)) deleteModel.mutate(m);
                        }}
                        disabled={isDeleting || deleteModel.isPending}
                        title={`Delete ${m}`}
                        className="text-zinc-600 hover:text-red-400 disabled:opacity-30 transition-colors ml-2"
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

          {/* Pull form */}
          <div className="border-t border-zinc-800 px-4 py-3">
            <form
              onSubmit={(e: FormEvent) => {
                e.preventDefault();
                const name = pullName.trim();
                if (name) pullModel.mutate(name);
              }}
              className="flex gap-2"
            >
              <div className="relative flex-1">
                <Plus size={11} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
                <input
                  value={pullName}
                  onChange={e => setPullName(e.target.value)}
                  placeholder="Pull model, e.g. llama3.2"
                  disabled={pullModel.isPending}
                  className="w-full rounded-md bg-zinc-950 border border-zinc-800 pl-7 pr-3 h-8
                             text-xs text-zinc-200 placeholder-zinc-600
                             focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                             disabled:opacity-50 transition-colors"
                />
              </div>
              <button
                type="submit"
                disabled={!pullName.trim() || pullModel.isPending}
                className="flex items-center gap-1.5 px-3 h-8 rounded-md bg-indigo-600 hover:bg-indigo-500
                           disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-xs font-medium
                           transition-colors whitespace-nowrap"
              >
                {pullModel.isPending
                  ? <><Loader2 size={11} className="animate-spin" /> Pulling…</>
                  : <><Download size={11} /> Pull</>}
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

// ── Main page ─────────────────────────────────────────────────────────────────

export function ProvidersPage() {
  const { data, isLoading, isError, error, refetch, dataUpdatedAt } = useQuery({
    queryKey: ["providers-health"],
    queryFn:  () => defaultApi.getProviderHealth() as Promise<ProviderHealthEntry[]>,
    refetchInterval: 20_000,
  });

  const entries: ProviderHealthEntry[] = Array.isArray(data) ? data : [];
  const healthy   = entries.filter(e => e.healthy).length;
  const unhealthy = entries.length - healthy;
  const lastUpdated = dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : null;

  return (
    <div className="p-6 space-y-6">
      {/* ── Cairn provider connections ────────────────────────────────── */}
      <section className="space-y-4">
        <div className="flex items-center justify-between">
          <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
            Provider Connections
            {entries.length > 0 && <span className="ml-1.5 normal-case tracking-normal font-normal text-zinc-700">({entries.length})</span>}
          </p>
          <div className="flex items-center gap-2">
            {lastUpdated && <span className="text-[11px] font-mono text-zinc-700">{lastUpdated}</span>}
            <button onClick={() => refetch()}
              className="flex items-center gap-1.5 rounded-md bg-zinc-900 border border-zinc-800 px-2.5 py-1.5 text-[11px] text-zinc-500 hover:bg-white/5 transition-colors">
              <RefreshCw size={11} /> Refresh
            </button>
          </div>
        </div>

        {/* Stat cards */}
        {!isLoading && entries.length > 0 && (
          <div className="grid grid-cols-3 gap-3">
            <StatCard label="Total"    value={entries.length} />
            <StatCard label="Healthy"  value={healthy}   accent="emerald" />
            <StatCard label="Degraded" value={unhealthy}  accent={unhealthy > 0 ? "red" : "default"} />
          </div>
        )}

        {/* Table */}
        <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
          <div className="border-b border-zinc-800 bg-zinc-950">
            <table className="w-full">
              <thead>
                <tr>
                  <th className="px-4 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Status</th>
                  <th className="px-4 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Connection ID</th>
                  <th className="px-4 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Last Check</th>
                  <th className="px-4 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Failures</th>
                  <th className="px-4 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Error</th>
                </tr>
              </thead>
            </table>
          </div>

          {isError ? (
            <div className="flex items-center gap-3 px-4 py-4 text-sm">
              <ServerCrash size={16} className="text-red-500 shrink-0" />
              <span className="text-zinc-400">{error instanceof Error ? error.message : "Failed to load"}</span>
            </div>
          ) : isLoading ? (
            <div className="flex items-center gap-2 px-4 h-10 text-[11px] text-zinc-600">
              <Loader2 size={11} className="animate-spin" /> Loading…
            </div>
          ) : entries.length === 0 ? (
            <div className="px-4 py-8 text-center text-[11px] text-zinc-600">
              No provider connections registered
            </div>
          ) : (
            <table className="w-full">
              <tbody>
                {entries.map((entry, i) => (
                  <ProviderRow key={entry.connection_id} entry={entry} even={i % 2 === 0} />
                ))}
              </tbody>
            </table>
          )}
        </div>
      </section>

      {/* ── Ollama section ────────────────────────────────────────────── */}
      <OllamaSection />
    </div>
  );
}

export default ProvidersPage;
