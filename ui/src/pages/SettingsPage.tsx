import { useQuery } from "@tanstack/react-query";
import { RefreshCw, Loader2, Check, X, Radio, Wifi } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { usePreferences } from "../hooks/usePreferences";
import { useWebSocket } from "../hooks/useWebSocket";
import type { DeploymentSettings, SystemInfo } from "../lib/types";

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

// ── System info sections ──────────────────────────────────────────────────────

function FeatureRow({ label, value, enabled }: { label: string; value?: string | number; enabled?: boolean }) {
  return (
    <div className="flex items-center justify-between py-2.5 border-b border-zinc-800 last:border-0">
      <span className="text-[12px] text-zinc-500">{label}</span>
      <span className="flex items-center gap-2">
        {enabled !== undefined && (
          <span className={clsx(
            "inline-flex items-center justify-center w-3.5 h-3.5 rounded-full",
            enabled ? "bg-emerald-500/20 text-emerald-400" : "bg-zinc-800 text-zinc-600",
          )}>
            {enabled ? <Check size={9} strokeWidth={3} /> : <X size={9} strokeWidth={2} />}
          </span>
        )}
        {value !== undefined && (
          <span className="text-[12px] text-zinc-300 font-mono">{value}</span>
        )}
      </span>
    </div>
  );
}

function SystemInfoSections({ info }: { info: SystemInfo }) {
  return (
    <>
      {/* Version */}
      <Section title="Build Information">
        <KV label="Version"     value={<span className="font-mono text-indigo-300">v{info.version}</span>} />
        <KV label="OS / Arch"   value={`${info.os} / ${info.arch}`} mono />
        <KV label="Git Commit"  value={
          <span className="font-mono text-[12px] text-zinc-400">
            {info.git_commit === 'dev' ? (
              <span className="text-amber-400">dev build</span>
            ) : info.git_commit.slice(0, 12)}
          </span>
        } />
        <KV label="Build"       value={<span className="text-[12px] text-zinc-500">{info.build_date}</span>} />
      </Section>

      {/* Features */}
      <Section title="Features">
        <FeatureRow label="WebSocket transport"      enabled={info.features.websocket_enabled} />
        <FeatureRow label="Ollama connected"         enabled={info.features.ollama_connected} />
        <FeatureRow label="PostgreSQL backend"       enabled={info.features.postgres_enabled} />
        <FeatureRow label="SQLite backend"           enabled={info.features.sqlite_enabled} />
        <FeatureRow label="Store type"               value={info.features.store_type} />
        <FeatureRow label="SSE ring buffer"          value={`${info.features.sse_buffer_size.toLocaleString()} events`} />
        <FeatureRow label="Notification buffer"      value={`${info.features.notification_buffer} entries`} />
        <FeatureRow label="Rate limit (token)"       value={`${info.features.rate_limit_per_minute} req/min`} />
        <FeatureRow label="Rate limit (IP)"          value={`${info.features.ip_rate_limit_per_minute} req/min`} />
        <FeatureRow label="Max body size"            value={`${info.features.max_body_size_mb} MB`} />
      </Section>

      {/* Environment */}
      <Section title="Environment">
        <KV label="Deployment mode" value={<span className="font-mono text-[12px]">{info.environment.deployment_mode}</span>} />
        <KV
          label="Admin token"
          value={
            info.environment.admin_token_set ? (
              <span className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-400 bg-emerald-950/50 border border-emerald-800/40 rounded px-2 py-0.5">
                <Check size={10} strokeWidth={2.5} /> Set
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 text-[11px] font-medium text-amber-400 bg-amber-950/50 border border-amber-800/40 rounded px-2 py-0.5">
                <X size={10} strokeWidth={2} /> Not set
              </span>
            )
          }
        />
        <KV
          label="Ollama host"
          value={
            <span className="font-mono text-[12px] text-zinc-400 truncate max-w-[200px]" title={info.environment.ollama_host}>
              {info.environment.ollama_host}
            </span>
          }
        />
        <KV label="Uptime"      value={(() => {
          const s = info.environment.uptime_seconds;
          if (s < 60)   return `${s}s`;
          if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
          const h = Math.floor(s / 3600);
          return `${h}h ${Math.floor((s % 3600) / 60)}m`;
        })()} mono />
      </Section>
    </>
  );
}

// ── Transport section ─────────────────────────────────────────────────────────

const WS_STATUS_COLOR: Record<string, string> = {
  connected:    "text-emerald-400",
  connecting:   "text-amber-400",
  reconnecting: "text-amber-400",
  failed:       "text-red-400",
  idle:         "text-zinc-600",
};

function TransportSection() {
  const [prefs, setPrefs] = usePreferences();
  const isWs = prefs.transport === "websocket";

  const { status: wsStatus, reconnect } = useWebSocket({
    enabled: isWs,
  });

  return (
    <Section title="Real-time Transport">
      <KV
        label="Protocol"
        value={
          <div className="flex items-center gap-2">
            <button
              onClick={() => setPrefs({ transport: "sse" })}
              title="Server-Sent Events (default)"
              className={clsx(
                "flex items-center gap-1.5 rounded px-2 py-1 text-[11px] font-medium transition-colors border",
                !isWs
                  ? "bg-indigo-600/20 text-indigo-300 border-indigo-700/50"
                  : "text-zinc-500 border-zinc-700 hover:text-zinc-300",
              )}
            >
              <Radio size={11} /> SSE
            </button>
            <button
              onClick={() => setPrefs({ transport: "websocket" })}
              title="WebSocket (bidirectional, with event filtering)"
              className={clsx(
                "flex items-center gap-1.5 rounded px-2 py-1 text-[11px] font-medium transition-colors border",
                isWs
                  ? "bg-indigo-600/20 text-indigo-300 border-indigo-700/50"
                  : "text-zinc-500 border-zinc-700 hover:text-zinc-300",
              )}
            >
              <Wifi size={11} /> WebSocket
            </button>
          </div>
        }
      />

      {isWs && (
        <KV
          label="WS Status"
          value={
            <div className="flex items-center gap-2">
              <span className={clsx("text-[12px] font-medium", WS_STATUS_COLOR[wsStatus] ?? "text-zinc-500")}>
                {wsStatus}
              </span>
              {(wsStatus === "failed" || wsStatus === "idle") && (
                <button
                  onClick={reconnect}
                  className="text-[11px] text-zinc-500 hover:text-zinc-300 transition-colors"
                >
                  Reconnect
                </button>
              )}
            </div>
          }
        />
      )}

      <KV
        label="Endpoint"
        value={
          <code className="text-[11px] text-zinc-500 bg-zinc-800 px-1.5 py-0.5 rounded">
            {isWs ? "GET /v1/ws?token=…" : "GET /v1/stream"}
          </code>
        }
      />

      <KV
        label="Description"
        value={
          <span className="text-[11px] text-zinc-600">
            {isWs
              ? "Bidirectional; supports event-type filtering. Uses ?token= for auth."
              : "Unidirectional push with Last-Event-ID replay. Default."}
          </span>
        }
      />
    </Section>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function SettingsPage() {
  const { data, isLoading, isError, error, refetch, isFetching, dataUpdatedAt } = useQuery({
    queryKey: ["settings"],
    queryFn: () => defaultApi.getSettings(),
    staleTime: 60_000,
  });

  const { data: sysInfo, isLoading: sysLoading } = useQuery({
    queryKey: ["system-info"],
    queryFn: () => defaultApi.getSystemInfo(),
    staleTime: 60_000,
    retry: false,
  });

  const s: DeploymentSettings | undefined = data;

  if (isError) return <ErrorFallback error={error} resource="settings" onRetry={() => void refetch()} />;

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

            {/* Transport */}
            <TransportSection />

            {/* System info — version, features, environment */}
            {sysLoading ? (
              <div className="flex items-center gap-2 py-4 text-zinc-600">
                <Loader2 size={13} className="animate-spin" />
                <span className="text-[12px]">Loading system info…</span>
              </div>
            ) : sysInfo ? (
              <SystemInfoSections info={sysInfo} />
            ) : (
              <Section title="Build Information">
                <div className="py-3 text-[12px] text-zinc-600 italic">
                  System info unavailable — upgrade cairn-app for this endpoint.
                </div>
              </Section>
            )}

          </div>
        ) : null}
      </div>
    </div>
  );
}

export default SettingsPage;
