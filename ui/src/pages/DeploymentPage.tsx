/**
 * DeploymentPage — infrastructure and deployment topology overview.
 *
 * Aggregates:
 *   GET /v1/settings        → deployment mode, store backend, encryption, plugin count
 *   GET /v1/health/detailed → per-subsystem liveness, memory RSS, Ollama models
 *   GET /v1/system/info     → version, OS, build, features, environment
 *   GET /v1/status          → runtime_ok, store_ok, uptime_secs
 */

import { useQuery } from "@tanstack/react-query";
import {
  RefreshCw, CheckCircle2, AlertTriangle, XCircle,
  Database, Shield, Radio, Cpu, Globe,
  Clock, GitCommit, Lock, Unlock, Zap,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtUptime(secs: number): string {
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3_600);
  const m = Math.floor((secs % 3_600) / 60);
  const s = secs % 60;
  if (d > 0)  return `${d}d ${h}h ${m}m`;
  if (h > 0)  return `${h}h ${m}m ${s}s`;
  if (m > 0)  return `${m}m ${s}s`;
  return `${s}s`;
}

function fmtDate(iso: string): string {
  try { return new Date(iso).toLocaleString(); } catch { return iso; }
}

// ── Status indicator ──────────────────────────────────────────────────────────

type HealthStatus = "ok" | "degraded" | "error" | "unknown" | "off";

const STATUS_DOT: Record<HealthStatus, string> = {
  ok:       "bg-emerald-500",
  degraded: "bg-amber-500 animate-pulse",
  error:    "bg-red-500 animate-pulse",
  unknown:  "bg-zinc-600",
  off:      "bg-zinc-700",
};
const STATUS_RING: Record<HealthStatus, string> = {
  ok:       "ring-emerald-500/20",
  degraded: "ring-amber-500/20",
  error:    "ring-red-500/20",
  unknown:  "ring-zinc-700/20",
  off:      "ring-zinc-800/20",
};
const STATUS_LABEL: Record<HealthStatus, { text: string; color: string }> = {
  ok:       { text: "Healthy",      color: "text-emerald-400" },
  degraded: { text: "Degraded",     color: "text-amber-400"  },
  error:    { text: "Unhealthy",    color: "text-red-400"    },
  unknown:  { text: "Unknown",      color: "text-zinc-500"   },
  off:      { text: "Not configured", color: "text-zinc-600" },
};

function StatusDot({ status, size = "md" }: { status: HealthStatus; size?: "sm" | "md" }) {
  const sz = size === "sm" ? "w-1.5 h-1.5" : "w-2 h-2";
  return (
    <span className={clsx("rounded-full inline-block shrink-0", sz, STATUS_DOT[status])} />
  );
}

function StatusBadge({ status }: { status: HealthStatus }) {
  const { text, color } = STATUS_LABEL[status];
  return (
    <span className={clsx(
      "inline-flex items-center gap-1.5 text-[11px] font-medium rounded-full px-2 py-0.5 ring-1",
      `bg-${status === "ok" ? "emerald" : status === "degraded" ? "amber" : status === "error" ? "red" : "zinc"}-500/10`,
      STATUS_RING[status], color,
    )}>
      <StatusDot status={status} size="sm" />
      {text}
    </span>
  );
}

// ── Card shell ────────────────────────────────────────────────────────────────

function Card({
  icon: Icon, title, status, children, loading,
}: {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  title: string;
  status?: HealthStatus;
  children: React.ReactNode;
  loading?: boolean;
}) {
  return (
    <div className="bg-zinc-900 border border-zinc-800 rounded-xl overflow-hidden">
      {/* Card header */}
      <div className="flex items-center gap-2.5 px-4 py-3 border-b border-zinc-800 bg-zinc-950/40">
        <div className={clsx(
          "flex h-7 w-7 items-center justify-center rounded-lg border",
          status === "ok"       ? "bg-emerald-950/50 border-emerald-800/40" :
          status === "degraded" ? "bg-amber-950/50 border-amber-800/40" :
          status === "error"    ? "bg-red-950/50 border-red-800/40" :
                                  "bg-zinc-800 border-zinc-700",
        )}>
          <Icon size={13} className={
            status === "ok"       ? "text-emerald-400" :
            status === "degraded" ? "text-amber-400" :
            status === "error"    ? "text-red-400" : "text-zinc-500"
          } />
        </div>
        <span className="text-[12px] font-semibold text-zinc-200 flex-1">{title}</span>
        {status && <StatusBadge status={status} />}
      </div>

      {/* Card body */}
      <div className="p-4">
        {loading ? (
          <div className="space-y-2 animate-pulse">
            {[1, 2, 3].map(i => (
              <div key={i} className="h-4 rounded bg-zinc-800" style={{ width: `${60 + i * 12}%` }} />
            ))}
          </div>
        ) : children}
      </div>
    </div>
  );
}

// ── Row inside a card ─────────────────────────────────────────────────────────

function Row({
  label, value, mono = false, badge, status,
}: {
  label: string;
  value: React.ReactNode;
  mono?:  boolean;
  badge?: React.ReactNode;
  status?: HealthStatus;
}) {
  return (
    <div className="flex items-center justify-between py-1.5 border-b border-zinc-800/50 last:border-0">
      <span className="text-[11px] text-zinc-500 shrink-0 mr-3">{label}</span>
      <span className={clsx(
        "text-[11px] text-right break-all",
        mono ? "font-mono text-zinc-300" : "text-zinc-300",
      )}>
        {status && (
          <StatusDot status={status} size="sm" />
        )}{badge}{value}
      </span>
    </div>
  );
}

// ── Role chip ─────────────────────────────────────────────────────────────────

function RoleChip({ label, active, icon: Icon }: {
  label: string;
  active: boolean;
  icon: React.ComponentType<{ size?: number; className?: string }>;
}) {
  return (
    <div className={clsx(
      "flex items-center gap-2 rounded-lg border px-3 py-2.5 transition-colors",
      active
        ? "border-emerald-800/50 bg-emerald-950/30"
        : "border-zinc-800 bg-zinc-900/50 opacity-50",
    )}>
      <div className={clsx(
        "flex h-6 w-6 items-center justify-center rounded-md border",
        active
          ? "bg-emerald-950/60 border-emerald-800/60"
          : "bg-zinc-800 border-zinc-700",
      )}>
        <Icon size={12} className={active ? "text-emerald-400" : "text-zinc-600"} />
      </div>
      <div>
        <p className={clsx("text-[11px] font-medium", active ? "text-zinc-200" : "text-zinc-600")}>
          {label}
        </p>
        <p className={clsx("text-[10px]", active ? "text-emerald-600" : "text-zinc-700")}>
          {active ? "active" : "inactive"}
        </p>
      </div>
      {active && (
        <span className="ml-auto w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
      )}
    </div>
  );
}

// ── Overall health banner ─────────────────────────────────────────────────────

function HealthBanner({ status }: { status: HealthStatus }) {
  if (status === "ok") return (
    <div className="flex items-center gap-3 rounded-xl border border-emerald-800/40 bg-emerald-950/20 px-4 py-3">
      <CheckCircle2 size={16} className="text-emerald-400 shrink-0" />
      <div>
        <p className="text-[13px] font-semibold text-emerald-300">All systems operational</p>
        <p className="text-[11px] text-emerald-700 mt-0.5">Every subsystem is reporting healthy.</p>
      </div>
    </div>
  );
  if (status === "degraded") return (
    <div className="flex items-center gap-3 rounded-xl border border-amber-800/40 bg-amber-950/20 px-4 py-3">
      <AlertTriangle size={16} className="text-amber-400 shrink-0" />
      <div>
        <p className="text-[13px] font-semibold text-amber-300">Degraded</p>
        <p className="text-[11px] text-amber-700 mt-0.5">One or more subsystems are not fully healthy.</p>
      </div>
    </div>
  );
  return (
    <div className="flex items-center gap-3 rounded-xl border border-red-800/40 bg-red-950/20 px-4 py-3">
      <XCircle size={16} className="text-red-400 shrink-0" />
      <div>
        <p className="text-[13px] font-semibold text-red-300">Unhealthy</p>
        <p className="text-[11px] text-red-700 mt-0.5">One or more critical subsystems have failed.</p>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function DeploymentPage() {
  const { data: health, isLoading: hLoading, refetch: rHealth, isFetching: hFetching } = useQuery({
    queryKey: ["detailed-health"],
    queryFn:  () => defaultApi.getDetailedHealth(),
    refetchInterval: 30_000,
    retry: false,
  });

  const { data: settings, isLoading: sLoading } = useQuery({
    queryKey: ["settings"],
    queryFn:  () => defaultApi.getSettings(),
    refetchInterval: 60_000,
    retry: false,
  });

  const { data: info, isLoading: iLoading } = useQuery({
    queryKey: ["system-info"],
    queryFn:  () => defaultApi.getSystemInfo(),
    refetchInterval: 60_000,
    retry: false,
  });

  const { data: status } = useQuery({
    queryKey: ["status"],
    queryFn:  () => defaultApi.getStatus(),
    refetchInterval: 15_000,
    retry: false,
  });

  const isLoading = hLoading || sLoading || iLoading;

  // Derive overall health status
  const overallStatus: HealthStatus = !health ? "unknown"
    : health.status === "healthy"  ? "ok"
    : health.status === "degraded" ? "degraded"
    : "error";

  // Derive per-check statuses
  function checkStatus(entry: { status: string } | undefined): HealthStatus {
    if (!entry) return "unknown";
    if (entry.status === "healthy")      return "ok";
    if (entry.status === "degraded")     return "degraded";
    if (entry.status === "unhealthy")    return "error";
    if (entry.status === "unconfigured") return "off";
    return "unknown";
  }

  const storeStatus   = checkStatus(health?.checks.store);
  const ollamaStatus  = checkStatus(health?.checks.ollama);
  const memStatus     = checkStatus(health?.checks.memory);
  const bufStatus     = checkStatus(health?.checks.event_buffer);

  return (
    <div className="h-full overflow-y-auto bg-zinc-950">
      <div className="max-w-4xl mx-auto px-5 py-5 space-y-5">

        {/* Toolbar */}
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-[14px] font-semibold text-zinc-100">Deployment Health</h2>
            <p className="text-[11px] text-zinc-600 mt-0.5">
              Infrastructure topology, runtime status, and configuration snapshot
            </p>
          </div>
          <div className="flex items-center gap-3">
            {health && (
              <span className="text-[11px] text-zinc-600 font-mono">
                v{health.version}
              </span>
            )}
            <button
              onClick={() => rHealth()}
              disabled={hFetching}
              className="flex items-center gap-1.5 rounded border border-zinc-800 bg-zinc-900 text-zinc-500
                         text-[12px] px-2.5 py-1.5 hover:text-zinc-200 hover:bg-zinc-800 disabled:opacity-40 transition-colors"
            >
              <RefreshCw size={11} className={hFetching ? "animate-spin" : ""} />
              Refresh
            </button>
          </div>
        </div>

        {/* Overall health banner */}
        {!isLoading && <HealthBanner status={overallStatus} />}

        {/* Roles topology */}
        <div>
          <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-3">
            Active Roles
          </p>
          <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
            <RoleChip label="API Server"    active={status?.runtime_ok ?? false}     icon={Globe}    />
            <RoleChip label="Runtime"       active={status?.runtime_ok ?? false}     icon={Cpu}      />
            <RoleChip label="Ollama"        active={ollamaStatus === "ok"}           icon={Zap}      />
            <RoleChip label="Plugin Host"   active={(settings?.plugin_count ?? 0) > 0} icon={Radio}  />
          </div>
        </div>

        {/* Two-column grid */}
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">

          {/* Storage */}
          <Card icon={Database} title="Storage Backend" status={storeStatus} loading={isLoading}>
            <div className="space-y-0">
              <Row label="Backend"
                value={settings?.store_backend ?? "—"}
                mono
              />
              <Row label="Runtime healthy"
                value={status?.store_ok ? "Yes" : "No"}
                status={status?.store_ok ? "ok" : "error"}
              />
              {health?.checks.store.latency_ms !== undefined && (
                <Row label="Query latency" value={`${health.checks.store.latency_ms}ms`} mono />
              )}
              {info?.features.postgres_enabled !== undefined && (
                <Row label="Postgres"
                  value={info.features.postgres_enabled ? "enabled" : "disabled"}
                  status={info.features.postgres_enabled ? "ok" : "off"}
                />
              )}
              {info?.features.sqlite_enabled !== undefined && (
                <Row label="SQLite"
                  value={info.features.sqlite_enabled ? "enabled" : "disabled"}
                  status={info.features.sqlite_enabled ? "ok" : "off"}
                />
              )}
              {info?.features.store_type && (
                <Row label="Store type" value={info.features.store_type} mono />
              )}
            </div>
          </Card>

          {/* Process / runtime */}
          <Card icon={Cpu} title="Process" status={memStatus === "off" ? "ok" : memStatus} loading={isLoading}>
            <div className="space-y-0">
              {health?.uptime_seconds !== undefined && (
                <Row label="Uptime" value={fmtUptime(health.uptime_seconds)} />
              )}
              {health?.checks.memory.rss_mb !== undefined && (
                <Row label="Memory (RSS)" value={`${health.checks.memory.rss_mb} MB`} mono />
              )}
              {health?.checks.memory.heap_mb !== undefined && (
                <Row label="Memory (heap)" value={`${health.checks.memory.heap_mb} MB`} mono />
              )}
              {health?.started_at && (
                <Row label="Started at" value={fmtDate(health.started_at)} />
              )}
              {info?.os && (
                <Row label="OS / Arch" value={`${info.os} / ${info.arch}`} mono />
              )}
              {info?.rust_version && (
                <Row label="Rust version" value={info.rust_version} mono />
              )}
            </div>
          </Card>

          {/* Build info */}
          <Card icon={GitCommit} title="Build Info" loading={isLoading}>
            <div className="space-y-0">
              {info?.version && (
                <Row label="Version" value={`v${info.version}`} mono />
              )}
              {info?.git_commit && (
                <Row
                  label="Git commit"
                  value={info.git_commit.length > 12 ? info.git_commit.slice(0, 12) + "…" : info.git_commit}
                  mono
                />
              )}
              {info?.build_date && (
                <Row label="Build date" value={fmtDate(info.build_date)} />
              )}
              {info?.rust_version && (
                <Row label="Rust" value={info.rust_version} mono />
              )}
              {info?.os && (
                <Row label="Platform" value={`${info.os} ${info.arch}`} mono />
              )}
            </div>
          </Card>

          {/* Encryption */}
          <Card
            icon={settings?.key_management.encryption_key_configured ? Lock : Unlock}
            title="Encryption"
            status={settings?.key_management.encryption_key_configured ? "ok" : "off"}
            loading={isLoading}
          >
            <div className="space-y-0">
              <Row
                label="Key configured"
                value={settings?.key_management.encryption_key_configured ? "Yes" : "No"}
                status={settings?.key_management.encryption_key_configured ? "ok" : "off"}
              />
              {settings?.key_management.key_version !== null && (
                <Row label="Key version" value={String(settings?.key_management.key_version ?? "—")} mono />
              )}
              {settings?.key_management.last_rotation_at && (
                <Row
                  label="Last rotated"
                  value={new Date(settings.key_management.last_rotation_at).toLocaleString()}
                />
              )}
              {!settings?.key_management.encryption_key_configured && (
                <p className="text-[11px] text-zinc-600 italic mt-2">
                  Set <code className="bg-zinc-800 rounded px-1">CAIRN_ENCRYPTION_KEY</code> to enable at-rest encryption.
                </p>
              )}
            </div>
          </Card>

          {/* Network / connectivity */}
          <Card icon={Globe} title="Network" loading={isLoading}>
            <div className="space-y-0">
              {info?.environment.listen_addr && (
                <Row label="Listen address" value={info.environment.listen_addr} mono />
              )}
              {info?.environment.deployment_mode && (
                <Row label="Deployment mode" value={info.environment.deployment_mode} mono />
              )}
              {info?.features.max_body_size_mb !== undefined && (
                <Row label="Max body size" value={`${info.features.max_body_size_mb} MB`} mono />
              )}
              {info?.features.websocket_enabled !== undefined && (
                <Row
                  label="WebSocket"
                  value={info.features.websocket_enabled ? "enabled" : "disabled"}
                  status={info.features.websocket_enabled ? "ok" : "off"}
                />
              )}
              {settings?.deployment_mode && (
                <Row
                  label="CORS mode"
                  value={settings.deployment_mode === "self_hosted_team" ? "same-origin" : "allow any"}
                  status={settings.deployment_mode === "self_hosted_team" ? "ok" : "degraded"}
                />
              )}
            </div>
          </Card>

          {/* Rate limiting + features */}
          <Card icon={Shield} title="Rate Limiting & Features" loading={isLoading}>
            <div className="space-y-0">
              {info?.features.rate_limit_per_minute !== undefined && (
                <Row label="Token limit" value={`${info.features.rate_limit_per_minute} req/min`} mono />
              )}
              {info?.features.ip_rate_limit_per_minute !== undefined && (
                <Row label="IP limit" value={`${info.features.ip_rate_limit_per_minute} req/min`} mono />
              )}
              {info?.features.sse_buffer_size !== undefined && (
                <Row label="SSE buffer" value={`${info.features.sse_buffer_size} events`} mono />
              )}
              {info?.features.notification_buffer !== undefined && (
                <Row label="Notif buffer" value={`${info.features.notification_buffer}`} mono />
              )}
              {info?.environment.admin_token_set !== undefined && (
                <Row
                  label="Admin token"
                  value={info.environment.admin_token_set ? "configured" : "dev default"}
                  status={info.environment.admin_token_set ? "ok" : "degraded"}
                />
              )}
            </div>
          </Card>

          {/* Ollama */}
          <Card icon={Zap} title="Ollama (Local LLM)" status={ollamaStatus} loading={isLoading}>
            <div className="space-y-0">
              <Row
                label="Status"
                value={ollamaStatus === "ok" ? "Connected" : ollamaStatus === "off" ? "Not configured" : "Unreachable"}
                status={ollamaStatus}
              />
              {info?.environment.ollama_host && info.environment.ollama_host !== "" && (
                <Row label="Host" value={info.environment.ollama_host} mono />
              )}
              {health?.checks.ollama.latency_ms !== undefined && (
                <Row label="Last probe" value={`${health.checks.ollama.latency_ms}ms`} mono />
              )}
              {health?.checks.ollama.models !== undefined && (
                <Row label="Models loaded" value={String(health.checks.ollama.models)} mono />
              )}
              {info?.features.ollama_connected === false && (
                <p className="text-[11px] text-zinc-600 italic mt-2">
                  Set <code className="bg-zinc-800 rounded px-1">OLLAMA_HOST</code> to connect.
                </p>
              )}
            </div>
          </Card>

          {/* Event system */}
          <Card icon={Radio} title="Event System" status={bufStatus === "off" ? "ok" : bufStatus} loading={isLoading}>
            <div className="space-y-0">
              <Row
                label="Event buffer"
                value={bufStatus === "ok" ? "Healthy" : bufStatus === "off" ? "Inactive" : "Degraded"}
                status={bufStatus === "off" ? "ok" : bufStatus}
              />
              {info?.features.sse_buffer_size !== undefined && (
                <Row label="Buffer capacity" value={`${info.features.sse_buffer_size} events`} mono />
              )}
              {settings?.plugin_count !== undefined && (
                <Row label="Registered plugins" value={String(settings.plugin_count)} mono />
              )}
              {info?.features.notification_buffer !== undefined && (
                <Row label="Notification buffer" value={String(info.features.notification_buffer)} mono />
              )}
            </div>
          </Card>

        </div>

        {/* Last updated footer */}
        <div className="flex items-center gap-2 text-[10px] text-zinc-700 border-t border-zinc-800 pt-3">
          <Clock size={10} />
          <span>
            Health refreshes every 30s · Settings and build info every 60s
          </span>
        </div>

      </div>
    </div>
  );
}

export default DeploymentPage;
