import { useQuery } from "@tanstack/react-query";
import { RefreshCw, Loader2, ServerCrash, Check, X } from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import type { DeploymentSettings } from "../lib/types";

// ── Helpers ────────────────────────────────────────────────────────────────────

const fmtTime = (ms: number | null) =>
  ms ? new Date(ms).toLocaleString(undefined, {
    year: "numeric", month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  }) : "—";

// ── Atoms ─────────────────────────────────────────────────────────────────────

function KV({ label, value, mono }: { label: string; value: React.ReactNode; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between py-2.5 border-b border-zinc-800 last:border-0">
      <span className="text-[12px] text-zinc-500">{label}</span>
      <span className={clsx("text-[13px] text-zinc-200", mono && "font-mono")}>{value}</span>
    </div>
  );
}

function BoolChip({ value, trueLabel = "Yes", falseLabel = "No" }: {
  value: boolean; trueLabel?: string; falseLabel?: string;
}) {
  return value ? (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-400 bg-emerald-950/50 border border-emerald-800/40 rounded px-2 py-0.5">
      <Check size={10} strokeWidth={2.5} /> {trueLabel}
    </span>
  ) : (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-zinc-500 bg-zinc-800/60 border border-zinc-700 rounded px-2 py-0.5">
      <X size={10} strokeWidth={2} /> {falseLabel}
    </span>
  );
}

function ModeBadge({ mode }: { mode: string }) {
  const isTeam = mode === "self_hosted_team";
  return (
    <span className={clsx(
      "text-[11px] font-medium rounded px-2 py-0.5 border",
      isTeam
        ? "text-indigo-300 bg-indigo-950/50 border-indigo-800/40"
        : "text-zinc-300 bg-zinc-800/60 border-zinc-700",
    )}>
      {isTeam ? "Self-hosted Team" : "Local"}
    </span>
  );
}

function BackendBadge({ backend }: { backend: string }) {
  const colors: Record<string, string> = {
    postgres: "text-sky-300 bg-sky-950/50 border-sky-800/40",
    sqlite:   "text-amber-300 bg-amber-950/40 border-amber-800/40",
    memory:   "text-zinc-400 bg-zinc-800/60 border-zinc-700",
  };
  return (
    <span className={clsx("text-[11px] font-medium rounded px-2 py-0.5 border font-mono",
      colors[backend] ?? colors.memory)}>
      {backend}
    </span>
  );
}

// ── Settings section ───────────────────────────────────────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-zinc-800 overflow-hidden">
      <div className="border-l-2 border-indigo-500 px-4 py-2.5 bg-zinc-800/40">
        <p className="text-[12px] font-semibold text-zinc-300 uppercase tracking-wider">{title}</p>
      </div>
      <div className="px-4 bg-zinc-900/60">
        {children}
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function SettingsPage() {
  const { data, isLoading, isError, error, refetch, isFetching, dataUpdatedAt } = useQuery({
    queryKey: ["settings"],
    queryFn: () => defaultApi.getSettings(),
    staleTime: 60_000,
  });

  const s: DeploymentSettings | undefined = data;

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-zinc-300 font-medium">Failed to load settings</p>
      <p className="text-[12px] text-zinc-500">{error instanceof Error ? error.message : "Unknown"}</p>
      <button onClick={() => refetch()}
        className="mt-1 px-3 py-1.5 rounded bg-zinc-800 text-zinc-300 text-[12px] hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-zinc-800 shrink-0 bg-zinc-900">
        <span className="text-[13px] font-medium text-zinc-200">Settings</span>
        {dataUpdatedAt > 0 && (
          <span className="text-[11px] text-zinc-600 font-mono">
            {new Date(dataUpdatedAt).toLocaleTimeString()}
          </span>
        )}
        <button onClick={() => refetch()} disabled={isFetching}
          className="ml-auto flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors">
          <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-5">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading settings…</span>
          </div>
        ) : s ? (
          <div className="max-w-2xl space-y-4">

            {/* Deployment */}
            <Section title="Deployment">
              <KV label="Mode"       value={<ModeBadge mode={s.deployment_mode} />} />
              <KV label="Store"      value={<BackendBadge backend={s.store_backend} />} />
              <KV label="Plugins"    value={s.plugin_count} mono />
            </Section>

            {/* System health */}
            <Section title="System Health">
              <KV label="Providers"    value={s.system_health.provider_health_count} mono />
              <KV label="Plugins"      value={s.system_health.plugin_health_count}  mono />
              <KV label="Credentials"  value={s.system_health.credential_count}     mono />
              <KV
                label="Degraded components"
                value={
                  s.system_health.degraded_count > 0 ? (
                    <span className="text-[12px] font-semibold text-red-400">
                      {s.system_health.degraded_count}
                    </span>
                  ) : (
                    <span className="text-[12px] text-emerald-400">None</span>
                  )
                }
              />
            </Section>

            {/* Encryption */}
            <Section title="Encryption">
              <KV
                label="Key configured"
                value={<BoolChip value={s.key_management.encryption_key_configured} />}
              />
              <KV
                label="Key version"
                value={
                  s.key_management.key_version != null
                    ? <span className="font-mono">v{s.key_management.key_version}</span>
                    : <span className="text-zinc-600">—</span>
                }
              />
              <KV
                label="Last rotation"
                value={
                  <span className="font-mono text-[12px]">
                    {fmtTime(s.key_management.last_rotation_at)}
                  </span>
                }
              />
            </Section>

            {/* TLS (via settings endpoint — no dedicated fields yet, show a read-only hint) */}
            <Section title="TLS">
              <KV
                label="Status"
                value={
                  <span className="text-[12px] text-zinc-500 italic">
                    Managed by the server — see{" "}
                    <code className="text-zinc-400 bg-zinc-800 px-1 rounded text-[11px]">
                      GET /v1/settings/tls
                    </code>{" "}
                    for certificate details.
                  </span>
                }
              />
            </Section>

          </div>
        ) : null}
      </div>
    </div>
  );
}

export default SettingsPage;
