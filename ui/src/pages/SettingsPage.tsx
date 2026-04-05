import { useQuery } from "@tanstack/react-query";
import {
  Server,
  Database,
  ShieldCheck,
  ShieldOff,
  Activity,
  AlertTriangle,
  RefreshCw,
  ServerCrash,
  CheckCircle2,
  XCircle,
  Lock,
  Users,
  Laptop,
  RotateCcw,
  Plug,
  Key,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import type { DeploymentSettings } from "../lib/types";

// ── Small helpers ─────────────────────────────────────────────────────────────

function fmtTime(ms: number | null): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// ── Card shell ────────────────────────────────────────────────────────────────

function Card({
  title,
  icon: Icon,
  iconColor = "text-zinc-400",
  children,
}: {
  title: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  iconColor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5 space-y-4">
      <div className="flex items-center gap-2">
        <Icon size={16} className={iconColor} />
        <h2 className="text-sm font-semibold text-zinc-200">{title}</h2>
      </div>
      {children}
    </div>
  );
}

// ── Metric row ────────────────────────────────────────────────────────────────

function MetricRow({
  label,
  value,
  ok,
  mono = false,
}: {
  label: string;
  value: React.ReactNode;
  ok?: boolean;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between py-2 border-b border-zinc-800 last:border-0">
      <span className="text-xs text-zinc-500">{label}</span>
      <span
        className={clsx(
          "text-xs font-medium",
          mono && "font-mono",
          ok === true && "text-emerald-400",
          ok === false && "text-red-400",
          ok === undefined && "text-zinc-300",
        )}
      >
        {value}
      </span>
    </div>
  );
}

// ── Deployment Mode card ──────────────────────────────────────────────────────

function DeploymentModeCard({ mode }: { mode: DeploymentSettings["deployment_mode"] }) {
  const isLocal = mode === "local";
  return (
    <Card
      title="Deployment Mode"
      icon={isLocal ? Laptop : Users}
      iconColor={isLocal ? "text-sky-400" : "text-indigo-400"}
    >
      <div className="flex items-center gap-3">
        <span
          className={clsx(
            "inline-flex items-center gap-2 rounded-lg px-4 py-2.5 text-sm font-semibold ring-1",
            isLocal
              ? "bg-sky-950 text-sky-300 ring-sky-800"
              : "bg-indigo-950 text-indigo-300 ring-indigo-800",
          )}
        >
          {isLocal ? <Laptop size={15} /> : <Users size={15} />}
          {isLocal ? "Local" : "Self-Hosted Team"}
        </span>
      </div>
      <MetricRow
        label="Mode identifier"
        value={mode}
        mono
      />
      <p className="text-xs text-zinc-600">
        {isLocal
          ? "Single-user local development. No multi-tenancy or credential encryption required."
          : "Team deployment. Credential encryption and admin token are required."}
      </p>
    </Card>
  );
}

// ── Storage Backend card ──────────────────────────────────────────────────────

const BACKEND_META: Record<
  DeploymentSettings["store_backend"],
  { label: string; color: string; ring: string; desc: string }
> = {
  memory:   { label: "In-Memory",  color: "bg-zinc-800  text-zinc-300  ring-zinc-700",  ring: "text-zinc-400",  desc: "Ephemeral store — data is lost on restart. Suitable for local development only." },
  sqlite:   { label: "SQLite",     color: "bg-amber-950 text-amber-300 ring-amber-800", ring: "text-amber-400", desc: "File-backed SQLite store. Durable across restarts; single-writer." },
  postgres: { label: "PostgreSQL", color: "bg-blue-950  text-blue-300  ring-blue-800",  ring: "text-blue-400",  desc: "Full PostgreSQL backend. Supports concurrent writers and high durability." },
};

function StorageBackendCard({
  backend,
  pluginCount,
}: {
  backend: DeploymentSettings["store_backend"];
  pluginCount: number;
}) {
  const meta = BACKEND_META[backend] ?? BACKEND_META.memory;
  return (
    <Card title="Storage Backend" icon={Database} iconColor={meta.ring}>
      <div>
        <span
          className={clsx(
            "inline-flex items-center gap-2 rounded-lg px-4 py-2.5 text-sm font-semibold ring-1",
            meta.color,
          )}
        >
          <Database size={14} />
          {meta.label}
        </span>
      </div>
      <div className="space-y-0">
        <MetricRow label="Backend"      value={backend} mono />
        <MetricRow label="Active Plugins" value={pluginCount} />
      </div>
      <p className="text-xs text-zinc-600">{meta.desc}</p>
    </Card>
  );
}

// ── System Health card ────────────────────────────────────────────────────────

function SystemHealthCard({ health }: { health: DeploymentSettings["system_health"] }) {
  const allHealthy = health.degraded_count === 0;
  return (
    <Card
      title="System Health"
      icon={Activity}
      iconColor={allHealthy ? "text-emerald-400" : "text-amber-400"}
    >
      {/* Summary badge */}
      <div>
        <span
          className={clsx(
            "inline-flex items-center gap-2 rounded-lg px-4 py-2.5 text-sm font-semibold ring-1",
            allHealthy
              ? "bg-emerald-950 text-emerald-300 ring-emerald-800"
              : "bg-amber-950 text-amber-300 ring-amber-800",
          )}
        >
          {allHealthy ? (
            <CheckCircle2 size={15} />
          ) : (
            <AlertTriangle size={15} />
          )}
          {allHealthy ? "All systems operational" : `${health.degraded_count} degraded`}
        </span>
      </div>

      <div className="space-y-0">
        <MetricRow
          label="Provider health checks"
          value={health.provider_health_count}
        />
        <MetricRow
          label="Plugin health checks"
          value={health.plugin_health_count}
        />
        <MetricRow
          label="Credentials configured"
          value={health.credential_count}
        />
        <MetricRow
          label="Degraded components"
          value={health.degraded_count}
          ok={health.degraded_count === 0}
        />
      </div>

      {health.degraded_count > 0 && (
        <div className="flex items-start gap-2 rounded-lg bg-amber-950/40 ring-1 ring-amber-800/50 px-3 py-2">
          <AlertTriangle size={13} className="text-amber-400 mt-0.5 shrink-0" />
          <p className="text-xs text-amber-300">
            {health.degraded_count} component(s) are degraded. Check provider
            and plugin health endpoints for details.
          </p>
        </div>
      )}
    </Card>
  );
}

// ── Key Management card ───────────────────────────────────────────────────────

function KeyManagementCard({ km }: { km: DeploymentSettings["key_management"] }) {
  const configured = km.encryption_key_configured;
  return (
    <Card
      title="Key Management"
      icon={configured ? ShieldCheck : ShieldOff}
      iconColor={configured ? "text-emerald-400" : "text-zinc-500"}
    >
      <div>
        <span
          className={clsx(
            "inline-flex items-center gap-2 rounded-lg px-4 py-2.5 text-sm font-semibold ring-1",
            configured
              ? "bg-emerald-950 text-emerald-300 ring-emerald-800"
              : "bg-zinc-800 text-zinc-400 ring-zinc-700",
          )}
        >
          {configured ? <Lock size={14} /> : <Key size={14} />}
          {configured ? "Encryption key configured" : "No encryption key"}
        </span>
      </div>

      <div className="space-y-0">
        <MetricRow
          label="Encryption configured"
          value={configured ? "Yes" : "No"}
          ok={configured}
        />
        <MetricRow
          label="Key version"
          value={km.key_version !== null ? String(km.key_version) : "—"}
          mono
        />
        <MetricRow
          label="Last rotation"
          value={fmtTime(km.last_rotation_at)}
        />
      </div>

      {!configured && (
        <div className="flex items-start gap-2 rounded-lg bg-zinc-800/60 ring-1 ring-zinc-700 px-3 py-2">
          <XCircle size={13} className="text-zinc-500 mt-0.5 shrink-0" />
          <p className="text-xs text-zinc-500">
            Credential encryption is disabled. Set{" "}
            <code className="text-zinc-400 bg-zinc-700 rounded px-1">CAIRN_ENCRYPTION_KEY</code>{" "}
            to enable at-rest encryption.
          </p>
        </div>
      )}

      {configured && km.last_rotation_at === null && (
        <div className="flex items-start gap-2 rounded-lg bg-sky-950/40 ring-1 ring-sky-800/50 px-3 py-2">
          <RotateCcw size={13} className="text-sky-400 mt-0.5 shrink-0" />
          <p className="text-xs text-sky-300">
            Key has never been rotated. Consider scheduling periodic rotation.
          </p>
        </div>
      )}
    </Card>
  );
}

// ── Plugins summary card ──────────────────────────────────────────────────────

function PluginsSummaryCard({ count }: { count: number }) {
  return (
    <Card title="Plugins" icon={Plug} iconColor="text-violet-400">
      <div className="flex items-end gap-2">
        <span className="text-4xl font-bold tabular-nums text-zinc-100">{count}</span>
        <span className="text-sm text-zinc-500 mb-1">registered</span>
      </div>
      {count === 0 ? (
        <p className="text-xs text-zinc-600">
          No plugins registered. Register a plugin via{" "}
          <code className="text-zinc-500 bg-zinc-800 rounded px-1">POST /v1/plugins</code>.
        </p>
      ) : (
        <p className="text-xs text-zinc-600">
          {count} plugin(s) available. Check{" "}
          <code className="text-zinc-500 bg-zinc-800 rounded px-1">GET /v1/plugins</code>{" "}
          for capability details.
        </p>
      )}
    </Card>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function SettingsPage() {
  const { data, isLoading, isError, error, refetch, isFetching, dataUpdatedAt } =
    useQuery({
      queryKey: ["settings"],
      queryFn: () => defaultApi.getSettings(),
      refetchInterval: 30_000,
    });

  return (
    <div className="h-full overflow-y-auto bg-zinc-950">
      <div className="max-w-4xl mx-auto p-6 space-y-6">
        {/* ── Page header ─────────────────────────────────────────────── */}
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-semibold text-zinc-100 flex items-center gap-2">
              <Server size={20} className="text-zinc-400" />
              Settings
            </h1>
            <p className="text-xs text-zinc-500 mt-0.5">
              Deployment configuration and system health
            </p>
          </div>

          <div className="flex items-center gap-3">
            {dataUpdatedAt > 0 && (
              <span className="text-xs text-zinc-600">
                Updated {new Date(dataUpdatedAt).toLocaleTimeString()}
              </span>
            )}
            <button
              onClick={() => void refetch()}
              disabled={isFetching}
              className="flex items-center gap-1.5 rounded-md bg-zinc-800 border border-zinc-700 text-zinc-400 text-xs px-2.5 py-1.5 hover:text-zinc-200 hover:bg-zinc-700 disabled:opacity-40 transition-colors"
            >
              <RefreshCw size={12} className={clsx(isFetching && "animate-spin")} />
              Refresh
            </button>
          </div>
        </div>

        {/* ── Error state ─────────────────────────────────────────────── */}
        {isError && (
          <div className="flex flex-col items-center justify-center min-h-48 gap-3 text-center rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-8">
            <ServerCrash size={36} className="text-red-500" />
            <p className="text-zinc-300 font-medium">Failed to load settings</p>
            <p className="text-sm text-zinc-500">
              {error instanceof Error ? error.message : "Unknown error"}
            </p>
            <button
              onClick={() => void refetch()}
              className="mt-1 px-4 py-2 rounded-lg bg-zinc-800 text-zinc-300 text-sm hover:bg-zinc-700 transition-colors"
            >
              Retry
            </button>
          </div>
        )}

        {/* ── Loading skeleton ─────────────────────────────────────────── */}
        {isLoading && !isError && (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            {Array.from({ length: 4 }).map((_, i) => (
              <div
                key={i}
                className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-5 space-y-3 animate-pulse"
              >
                <div className="h-4 w-32 rounded bg-zinc-800" />
                <div className="h-10 w-40 rounded-lg bg-zinc-800" />
                <div className="space-y-2">
                  <div className="h-3 w-full rounded bg-zinc-800" />
                  <div className="h-3 w-3/4 rounded bg-zinc-800" />
                  <div className="h-3 w-1/2 rounded bg-zinc-800" />
                </div>
              </div>
            ))}
          </div>
        )}

        {/* ── Cards grid ───────────────────────────────────────────────── */}
        {data && (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <DeploymentModeCard  mode={data.deployment_mode} />
            <StorageBackendCard  backend={data.store_backend} pluginCount={data.plugin_count} />
            <SystemHealthCard    health={data.system_health} />
            <KeyManagementCard   km={data.key_management} />
          </div>
        )}

        {/* ── Plugins card — full-width below the 2-col grid ───────────── */}
        {data && (
          <PluginsSummaryCard count={data.plugin_count} />
        )}
      </div>
    </div>
  );
}

export default SettingsPage;
