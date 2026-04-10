import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useCallback, useEffect, useRef } from "react";
import { RefreshCw, Loader2, Check, X, Radio, Wifi, ShieldCheck, SlidersHorizontal, Server } from "lucide-react";
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
    <div className="flex items-center justify-between py-2.5 border-b border-gray-200 dark:border-zinc-800 last:border-0">
      <span className="text-[12px] text-gray-400 dark:text-zinc-500">{label}</span>
      <span className={clsx("text-[13px] text-gray-800 dark:text-zinc-200", mono && "font-mono")}>{value}</span>
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
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-gray-400 dark:text-zinc-500 bg-gray-100/60 dark:bg-zinc-800/60 border border-gray-200 dark:border-zinc-700 rounded px-2 py-0.5">
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
        : "text-gray-700 dark:text-zinc-300 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700",
    )}>
      {isTeam ? "Self-hosted Team" : "Local"}
    </span>
  );
}

function BackendBadge({ backend }: { backend: string }) {
  const colors: Record<string, string> = {
    postgres: "text-sky-300 bg-sky-950/50 border-sky-800/40",
    sqlite:   "text-amber-300 bg-amber-950/40 border-amber-800/40",
    memory:   "text-gray-500 dark:text-zinc-400 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700",
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
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden">
      <div className="border-l-2 border-indigo-500 px-4 py-2.5 bg-gray-100/40 dark:bg-zinc-800/40">
        <p className="text-[12px] font-semibold text-gray-700 dark:text-zinc-300 uppercase tracking-wider">{title}</p>
      </div>
      <div className="px-4 bg-gray-50/60 dark:bg-zinc-900/60">
        {children}
      </div>
    </div>
  );
}

// ── System info sections ──────────────────────────────────────────────────────

function FeatureRow({ label, value, enabled }: { label: string; value?: string | number; enabled?: boolean }) {
  return (
    <div className="flex items-center justify-between py-2.5 border-b border-gray-200 dark:border-zinc-800 last:border-0">
      <span className="text-[12px] text-gray-400 dark:text-zinc-500">{label}</span>
      <span className="flex items-center gap-2">
        {enabled !== undefined && (
          <span className={clsx(
            "inline-flex items-center justify-center w-3.5 h-3.5 rounded-full",
            enabled ? "bg-emerald-500/20 text-emerald-400" : "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-600",
          )}>
            {enabled ? <Check size={9} strokeWidth={3} /> : <X size={9} strokeWidth={2} />}
          </span>
        )}
        {value !== undefined && (
          <span className="text-[12px] text-gray-700 dark:text-zinc-300 font-mono">{value}</span>
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
          <span className="font-mono text-[12px] text-gray-500 dark:text-zinc-400">
            {info.git_commit === 'dev' ? (
              <span className="text-amber-400">dev build</span>
            ) : info.git_commit.slice(0, 12)}
          </span>
        } />
        <KV label="Build"       value={<span className="text-[12px] text-gray-400 dark:text-zinc-500">{info.build_date}</span>} />
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
            <span className="font-mono text-[12px] text-gray-500 dark:text-zinc-400 truncate max-w-[200px]" title={info.environment.ollama_host}>
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

// ── Environment variables ─────────────────────────────────────────────────────

type EnvSecret = 'set' | 'unset' | 'unknown';

interface EnvVar {
  name:        string;
  current:     React.ReactNode;
  default_:    string;
  description: string;
  secret?:     boolean;
}

function SecretChip({ status }: { status: EnvSecret }) {
  if (status === 'set') return (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium font-mono text-emerald-400 bg-emerald-950/40 border border-emerald-800/30 rounded px-1.5 py-0.5">
      <span className="tracking-widest text-[8px] leading-none">●●●●●●</span>
      <span className="text-[10px]">set</span>
    </span>
  );
  if (status === 'unset') return (
    <span className="inline-flex items-center gap-1 text-[11px] font-medium text-gray-400 dark:text-zinc-600 bg-gray-100/60 dark:bg-zinc-800/60 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
      not set
    </span>
  );
  return <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono">—</span>;
}

function EnvValue({ value }: { value: string }) {
  return (
    <span className="font-mono text-[11px] text-gray-700 dark:text-zinc-300 bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded max-w-[180px] truncate block text-right"
          title={value}>
      {value || '—'}
    </span>
  );
}

function EnvVarsSection({
  settings,
  info,
}: {
  settings?: import('../lib/types').DeploymentSettings;
  info?:     import('../lib/types').SystemInfo;
}) {
  const listenAddr = info?.environment.listen_addr ?? '';
  const colonIdx   = listenAddr.lastIndexOf(':');
  const host       = colonIdx > 0 ? listenAddr.slice(0, colonIdx) : listenAddr;
  const port       = colonIdx > 0 ? listenAddr.slice(colonIdx + 1) : '';

  const rows: EnvVar[] = [
    {
      name: 'CAIRN_ADMIN_TOKEN',
      current: <SecretChip status={
        info ? (info.environment.admin_token_set ? 'set' : 'unset') : 'unknown'
      } />,
      default_:    '(none)',
      description: 'Bearer token for admin API authentication. Required for all /v1/* requests.',
      secret: true,
    },
    {
      name: 'OLLAMA_HOST',
      current: info?.environment.ollama_host
        ? <EnvValue value={info.environment.ollama_host} />
        : <SecretChip status="unset" />,
      default_:    '(none)',
      description: 'Base URL for the local Ollama API. Enables LLM generation, embedding, and model management.',
    },
    {
      name: 'OPENROUTER_API_KEY',
      current: <SecretChip status="unknown" />,
      default_:    '(none)',
      description: 'API key for OpenRouter (openrouter.ai). Enables 200+ models via the OpenAI-compatible endpoint. Get a free key at openrouter.ai/settings/keys.',
      secret: true,
    },
    {
      name: 'CAIRN_BRAIN_URL',
      current: <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono italic">not exposed</span>,
      default_:    '(none)',
      description: 'Base URL for the brain LLM provider (OpenAI-compatible). Used for orchestration and high-capability tasks.',
    },
    {
      name: 'CAIRN_STORAGE',
      current: settings
        ? <EnvValue value={settings.store_backend} />
        : <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono">—</span>,
      default_:    'in_memory',
      description: 'Persistence backend: in_memory (default), sqlite, or postgres.',
    },
    {
      name: 'CAIRN_MODE',
      current: info?.environment.deployment_mode
        ? <EnvValue value={info.environment.deployment_mode} />
        : <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono">—</span>,
      default_:    'local',
      description: 'Deployment mode: local (single user) or self_hosted_team (multi-tenant).',
    },
    {
      name: 'CAIRN_LISTEN_ADDR',
      current: host ? <EnvValue value={host} /> : <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono">—</span>,
      default_:    '127.0.0.1',
      description: 'TCP address to bind. Set to 0.0.0.0 to listen on all interfaces.',
    },
    {
      name: 'CAIRN_LISTEN_PORT',
      current: port ? <EnvValue value={port} /> : <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono">—</span>,
      default_:    '3000',
      description: 'TCP port to listen on.',
    },
    {
      name: 'CAIRN_ENCRYPTION_KEY',
      current: <SecretChip status={
        settings
          ? (settings.key_management?.encryption_key_configured ? 'set' : 'unset')
          : 'unknown'
      } />,
      default_:    '(none)',
      description: 'AES-256 key for at-rest credential encryption. If unset, credentials are stored unencrypted.',
      secret: true,
    },
    {
      name: 'CAIRN_TLS_CERT',
      current: <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono italic">not exposed</span>,
      default_:    '(none)',
      description: 'Path to PEM certificate file for HTTPS. Requires CAIRN_TLS_KEY.',
    },
    {
      name: 'CAIRN_TLS_KEY',
      current: <span className="text-[11px] text-gray-300 dark:text-zinc-700 font-mono italic">not exposed</span>,
      default_:    '(none)',
      description: 'Path to PEM private key file for HTTPS. Requires CAIRN_TLS_CERT.',
      secret: true,
    },
  ];

  return (
    <Section title="Environment Variables">
      <div className="-mx-4">
        <table className="w-full text-left border-collapse">
          <thead>
            <tr className="border-b border-gray-200 dark:border-zinc-800">
              <th className="px-4 py-2 text-[10px] font-semibold uppercase tracking-wider text-gray-400 dark:text-zinc-600 w-[220px]">Variable</th>
              <th className="px-4 py-2 text-[10px] font-semibold uppercase tracking-wider text-gray-400 dark:text-zinc-600 w-[160px] text-right">Current</th>
              <th className="px-4 py-2 text-[10px] font-semibold uppercase tracking-wider text-gray-400 dark:text-zinc-600 w-[90px]">Default</th>
              <th className="px-4 py-2 text-[10px] font-semibold uppercase tracking-wider text-gray-400 dark:text-zinc-600">Description</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.name} className="border-b border-gray-200/60 dark:border-zinc-800/60 last:border-0 hover:bg-gray-100/20 dark:hover:bg-gray-100/20 dark:bg-zinc-800/20 transition-colors">
                <td className="px-4 py-2.5 align-top">
                  <code className="text-[11px] font-mono text-indigo-300 select-all">
                    {row.name}
                  </code>
                  {row.secret && (
                    <span className="ml-1.5 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wide">secret</span>
                  )}
                </td>
                <td className="px-4 py-2.5 align-top text-right">
                  {row.current}
                </td>
                <td className="px-4 py-2.5 align-top">
                  <span className="font-mono text-[11px] text-gray-400 dark:text-zinc-600">{row.default_}</span>
                </td>
                <td className="px-4 py-2.5 align-top">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 leading-relaxed">{row.description}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Section>
  );
}

// ── Transport section ─────────────────────────────────────────────────────────

const WS_STATUS_COLOR: Record<string, string> = {
  connected:    "text-emerald-400",
  connecting:   "text-amber-400",
  reconnecting: "text-amber-400",
  failed:       "text-red-400",
  idle:         "text-gray-400 dark:text-zinc-600",
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
                  : "text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700 hover:text-gray-700 dark:hover:text-zinc-300",
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
                  : "text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700 hover:text-gray-700 dark:hover:text-zinc-300",
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
              <span className={clsx("text-[12px] font-medium", WS_STATUS_COLOR[wsStatus] ?? "text-gray-400 dark:text-zinc-500")}>
                {wsStatus}
              </span>
              {(wsStatus === "failed" || wsStatus === "idle") && (
                <button
                  onClick={reconnect}
                  className="text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
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
          <code className="text-[11px] text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 px-1.5 py-0.5 rounded">
            {isWs ? "GET /v1/ws?token=…" : "GET /v1/stream"}
          </code>
        }
      />

      <KV
        label="Description"
        value={
          <span className="text-[11px] text-gray-400 dark:text-zinc-600">
            {isWs
              ? "Bidirectional; supports event-type filtering. Uses ?token= for auth."
              : "Unidirectional push with Last-Event-ID replay. Default."}
          </span>
        }
      />
    </Section>
  );
}

// ── CORS diagnostics ──────────────────────────────────────────────────────────

const CORS_HEADERS_OF_INTEREST = [
  "access-control-allow-origin",
  "access-control-allow-methods",
  "access-control-allow-headers",
  "access-control-max-age",
] as const;

type CorsHeader = typeof CORS_HEADERS_OF_INTEREST[number];

interface PreflightResult {
  status:  number;
  ok:      boolean;
  latency: number;
  headers: Partial<Record<CorsHeader, string>>;
  error?:  string;
}

function CorsDiagnosticsSection({ deploymentMode }: { deploymentMode?: string }) {
  const [result, setResult]   = useState<PreflightResult | null>(null);
  const [testing, setTesting] = useState(false);

  const corsMode = deploymentMode === "self_hosted_team"
    ? "same-origin (team)"
    : "wildcard (*)";

  const allowedOrigins = deploymentMode === "self_hosted_team"
    ? ["same-origin only — configure a reverse proxy for cross-origin access"]
    : ["* (any origin)"];

  const runPreflight = useCallback(async () => {
    setTesting(true);
    setResult(null);
    const t0 = performance.now();
    try {
      const resp = await fetch("/v1/rate-limit", {
        method: "OPTIONS",
        headers: {
          "Access-Control-Request-Method":  "GET",
          "Access-Control-Request-Headers": "Authorization, Content-Type",
        },
      });
      const latency = Math.round(performance.now() - t0);
      const headers: Partial<Record<CorsHeader, string>> = {};
      for (const h of CORS_HEADERS_OF_INTEREST) {
        const v = resp.headers.get(h);
        if (v) headers[h] = v;
      }
      setResult({ status: resp.status, ok: resp.ok || resp.status === 204, latency, headers });
    } catch (err) {
      const latency = Math.round(performance.now() - t0);
      setResult({ status: 0, ok: false, latency, headers: {}, error: String(err) });
    } finally {
      setTesting(false);
    }
  }, []);

  const fmtHeader = (h: CorsHeader): string => {
    return h.split("-").map(w => w.charAt(0).toUpperCase() + w.slice(1)).join("-");
  };

  const fmtMaxAge = (v?: string) => {
    if (!v) return null;
    const secs = parseInt(v, 10);
    if (isNaN(secs)) return v;
    if (secs >= 3600) return `${secs / 3600}h (${v}s)`;
    return `${secs}s`;
  };

  return (
    <Section title="CORS Diagnostics">
      {/* Static config */}
      <KV label="CORS mode" value={
        <span className={clsx(
          "font-mono text-[11px] px-1.5 py-0.5 rounded border",
          deploymentMode === "self_hosted_team"
            ? "text-amber-300 bg-amber-950/30 border-amber-800/30"
            : "text-emerald-300 bg-emerald-950/30 border-emerald-800/30",
        )}>
          {corsMode}
        </span>
      } />
      <KV label="Allowed origins" value={
        <div className="flex flex-col items-end gap-1">
          {allowedOrigins.map(o => (
            <span key={o} className="font-mono text-[11px] text-gray-500 dark:text-zinc-400 max-w-[280px] truncate text-right" title={o}>{o}</span>
          ))}
        </div>
      } />
      <KV label="Allowed methods" value={
        <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400">GET POST PUT DELETE PATCH OPTIONS</span>
      } />
      <KV label="Allowed headers" value={
        <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400">Authorization, Content-Type</span>
      } />
      <KV label="Max-Age (cache)" value={
        <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400">86400s (24 h)</span>
      } />

      {/* Preflight test */}
      <div className="flex items-center justify-between py-2.5 border-b border-gray-200 dark:border-zinc-800">
        <span className="text-[12px] text-gray-400 dark:text-zinc-500">Preflight test</span>
        <button
          onClick={() => void runPreflight()}
          disabled={testing}
          className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800
                     text-[11px] font-medium text-gray-700 dark:text-zinc-300 hover:text-gray-900 dark:hover:text-zinc-100 hover:border-zinc-600
                     disabled:opacity-40 px-2.5 py-1 transition-colors"
        >
          {testing
            ? <Loader2 size={11} className="animate-spin" />
            : <ShieldCheck size={11} />}
          {testing ? "Testing…" : "Run OPTIONS /v1/rate-limit"}
        </button>
      </div>

      {/* Results */}
      {result && (
        <div className="py-3 space-y-2">
          {/* Status line */}
          <div className="flex items-center gap-2 pb-2 border-b border-gray-200/60 dark:border-zinc-800/60">
            <span className={clsx(
              "inline-flex items-center gap-1 text-[11px] font-medium rounded px-2 py-0.5 border",
              result.ok
                ? "text-emerald-400 bg-emerald-950/40 border-emerald-800/30"
                : "text-red-400 bg-red-950/40 border-red-800/30",
            )}>
              {result.ok ? <Check size={10} strokeWidth={2.5} /> : <X size={10} />}
              {result.status > 0 ? `HTTP ${result.status}` : "Network error"}
            </span>
            <span className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono">{result.latency} ms</span>
            {result.error && (
              <span className="text-[11px] text-red-400 truncate max-w-xs" title={result.error}>
                {result.error}
              </span>
            )}
          </div>

          {/* Response headers */}
          {CORS_HEADERS_OF_INTEREST.map(h => {
            const raw = result.headers[h];
            const display = h === "access-control-max-age"
              ? (fmtMaxAge(raw) ?? <span className="text-gray-300 dark:text-zinc-700">absent</span>)
              : raw ?? <span className="text-gray-300 dark:text-zinc-700">absent</span>;
            const present = raw !== undefined;
            return (
              <div key={h} className="flex items-start justify-between gap-4 text-[11px]">
                <span className="font-mono text-gray-400 dark:text-zinc-600 shrink-0">{fmtHeader(h)}</span>
                <span className={clsx(
                  "font-mono truncate text-right max-w-[260px]",
                  present ? "text-gray-700 dark:text-zinc-300" : "text-gray-300 dark:text-zinc-700 italic",
                )} title={raw}>
                  {display}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </Section>
  );
}

// ── Preferences tab ───────────────────────────────────────────────────────────

type SaveState = "idle" | "saving" | "saved" | "error";

/** A single tenant-level default setting row. */
function PreferenceRow({
  label,
  description,
  settingKey,
  control,
}: {
  label:       string;
  description: string;
  settingKey:  string;
  /** Render prop receives (currentValue, localValue, setLocal) — returns the form control. */
  control: (
    stored:   unknown,
    local:    string,
    setLocal: (v: string) => void,
  ) => React.ReactNode;
}) {
  const qc = useQueryClient();
  const [local, setLocal]     = useState<string>("");
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["defaults", settingKey],
    queryFn:  () => defaultApi.resolveDefaultSetting(settingKey),
    staleTime: 60_000,
    retry: false,
  });

  // Seed local from stored value on first load — but only if user hasn't started typing.
  const storedStr = data?.value !== undefined ? String(data.value) : "";
  const initialised = useRef(false);
  useEffect(() => {
    if (!initialised.current && storedStr !== "") {
      setLocal(storedStr);
      initialised.current = true;
    }
  }, [storedStr]);

  const dirty = local !== storedStr && local !== "";

  const saveMutation = useMutation({
    mutationFn: () => {
      // Coerce to the right JSON type based on the key suffix.
      let value: unknown = local;
      if (settingKey === "approval_required") {
        value = local === "true";
      } else if (["max_tokens", "timeout_ms"].includes(settingKey)) {
        value = Number(local);
      } else if (settingKey === "temperature") {
        value = parseFloat(local);
      }
      return defaultApi.setDefaultSetting("tenant", "default", settingKey, value);
    },
    onSuccess: () => {
      setSaveState("saved");
      void qc.invalidateQueries({ queryKey: ["defaults", settingKey] });
      savedTimerRef.current = setTimeout(() => setSaveState("idle"), 2_500);
    },
    onError: () => setSaveState("error"),
    onMutate: () => setSaveState("saving"),
  });

  // Clear timer on unmount.
  useEffect(() => () => { if (savedTimerRef.current) clearTimeout(savedTimerRef.current); }, []);

  const stored = data?.value;

  return (
    <div className="flex items-start justify-between py-4 border-b border-gray-200 dark:border-zinc-800 last:border-0 gap-6">
      {/* Left: label + description + current stored value */}
      <div className="shrink-0 w-56">
        <p className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">{label}</p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5 leading-relaxed">{description}</p>
        {isLoading ? (
          <span className="text-[10px] text-gray-300 dark:text-zinc-700 font-mono mt-1 block">loading…</span>
        ) : stored !== undefined ? (
          <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono mt-1 block">
            stored: <span className="text-gray-400 dark:text-zinc-500">{String(stored)}</span>
          </span>
        ) : (
          <span className="text-[10px] text-gray-300 dark:text-zinc-700 italic mt-1 block">not set</span>
        )}
      </div>

      {/* Right: control + save button */}
      <div className="flex items-center gap-3 flex-1 justify-end">
        <div className="flex-1 max-w-xs">
          {control(stored, local, setLocal)}
        </div>

        <button
          onClick={() => saveMutation.mutate()}
          disabled={!dirty || saveState === "saving"}
          className={clsx(
            "flex items-center gap-1.5 px-3 h-8 rounded-md text-[12px] font-medium transition-all shrink-0 w-20 justify-center",
            saveState === "saved"
              ? "bg-emerald-700/30 border border-emerald-700/50 text-emerald-400"
              : saveState === "error"
                ? "bg-red-900/30 border border-red-700/40 text-red-400"
                : dirty
                  ? "bg-indigo-600 hover:bg-indigo-500 text-white border border-transparent"
                  : "bg-gray-100/60 dark:bg-zinc-800/60 border border-gray-200 dark:border-zinc-700 text-gray-400 dark:text-zinc-600 cursor-default",
          )}
        >
          {saveState === "saving" ? (
            <><Loader2 size={12} className="animate-spin" />Saving</>
          ) : saveState === "saved" ? (
            <><Check size={12} />Saved</>
          ) : saveState === "error" ? (
            <><X size={12} />Error</>
          ) : (
            "Save"
          )}
        </button>
      </div>
    </div>
  );
}

/** Text input control for preference rows. */
function PrefText({ local, setLocal, placeholder, mono = false }: {
  local: string; setLocal: (v: string) => void; placeholder?: string; mono?: boolean;
}) {
  return (
    <input
      value={local}
      onChange={e => setLocal(e.target.value)}
      placeholder={placeholder}
      className={clsx(
        "w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-1.5 text-[13px] text-gray-800 dark:text-zinc-200 placeholder-zinc-600",
        "focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors",
        mono && "font-mono",
      )}
    />
  );
}

/** Number input control for preference rows. */
function PrefNumber({ local, setLocal, min, max, placeholder }: {
  local: string; setLocal: (v: string) => void; min?: number; max?: number; placeholder?: string;
}) {
  return (
    <input
      type="number"
      value={local}
      onChange={e => setLocal(e.target.value)}
      min={min}
      max={max}
      placeholder={placeholder}
      className="w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-1.5 text-[13px] text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 transition-colors"
    />
  );
}

/** Slider + numeric display for temperature. */
function PrefSlider({ local, setLocal }: { local: string; setLocal: (v: string) => void }) {
  const v = parseFloat(local) || 0.7;
  return (
    <div className="flex items-center gap-3">
      <input
        type="range"
        min={0} max={2} step={0.05}
        value={v}
        onChange={e => setLocal(e.target.value)}
        className="flex-1 accent-indigo-500"
      />
      <input
        type="number"
        min={0} max={2} step={0.05}
        value={local}
        onChange={e => setLocal(e.target.value)}
        className="w-16 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-2 py-1.5 text-[12px] text-gray-800 dark:text-zinc-200 font-mono text-center focus:outline-none focus:border-indigo-500 transition-colors"
      />
    </div>
  );
}

/** Toggle control for boolean preferences. */
function PrefToggle({ local, setLocal }: { local: string; setLocal: (v: string) => void }) {
  const on = local === "true";
  return (
    <div className="flex items-center gap-3">
      <button
        type="button"
        onClick={() => setLocal(on ? "false" : "true")}
        className={clsx(
          "relative w-10 h-5 rounded-full border transition-colors",
          on ? "bg-indigo-600 border-indigo-500" : "bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700",
        )}
      >
        <div className={clsx(
          "absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform shadow-sm",
          on ? "translate-x-5" : "translate-x-0.5",
        )} />
      </button>
      <span className={clsx("text-[12px] font-medium", on ? "text-indigo-300" : "text-gray-400 dark:text-zinc-500")}>
        {on ? "Enabled" : "Disabled"}
      </span>
    </div>
  );
}

function PreferencesTab() {
  return (
    <div className="max-w-3xl space-y-6">
      {/* Section: operator defaults */}
      <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden">
        <div className="border-l-2 border-indigo-500 px-4 py-2.5 bg-gray-100/40 dark:bg-zinc-800/40">
          <p className="text-[12px] font-semibold text-gray-700 dark:text-zinc-300 uppercase tracking-wider">Operator Defaults</p>
          <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
            Tenant-level defaults applied when no project-level or run-level override exists.
            Changes are saved automatically when you click Save.
          </p>
        </div>
        <div className="px-5 bg-gray-50/60 dark:bg-zinc-900/60">
          <PreferenceRow
            label="Default model"
            description="Model ID used when no binding specifies one."
            settingKey="default_model"
            control={(_, local, setLocal) => (
              <PrefText local={local} setLocal={setLocal} placeholder="e.g. gemma4, gpt-4o" mono />
            )}
          />
          <PreferenceRow
            label="Max output tokens"
            description="Hard cap on output length. 0 = provider default."
            settingKey="max_tokens"
            control={(_, local, setLocal) => (
              <PrefNumber local={local} setLocal={setLocal} min={0} max={128_000} placeholder="e.g. 4096" />
            )}
          />
          <PreferenceRow
            label="Temperature"
            description="Sampling randomness (0 = deterministic, 2 = very creative)."
            settingKey="temperature"
            control={(_, local, setLocal) => (
              <PrefSlider local={local} setLocal={setLocal} />
            )}
          />
          <PreferenceRow
            label="Request timeout"
            description="Provider call timeout in milliseconds."
            settingKey="timeout_ms"
            control={(_, local, setLocal) => (
              <PrefNumber local={local} setLocal={setLocal} min={1_000} max={600_000} placeholder="e.g. 30000" />
            )}
          />
          <PreferenceRow
            label="Approval required"
            description="Require operator approval before any run executes."
            settingKey="approval_required"
            control={(_, local, setLocal) => (
              <PrefToggle local={local || "false"} setLocal={setLocal} />
            )}
          />
        </div>
      </div>

      {/* Hint about future per-project overrides */}
      <p className="text-[11px] text-gray-300 dark:text-zinc-700 leading-relaxed px-1">
        These defaults apply at the tenant level.
        Project-level overrides will be configurable here once multi-tenant project isolation is enabled.
      </p>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

type SettingsTab = "system" | "preferences";

export function SettingsPage() {
  const [activeTab, setActiveTab] = useState<SettingsTab>("system");

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
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">Settings</span>
        {activeTab === "system" && dataUpdatedAt > 0 && (
          <span className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono">
            {new Date(dataUpdatedAt).toLocaleTimeString()}
          </span>
        )}
        {activeTab === "system" && (
          <button onClick={() => refetch()} disabled={isFetching}
            className="ml-auto flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors">
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
            Refresh
          </button>
        )}
      </div>

      {/* Tab bar */}
      <div className="flex items-center gap-1 px-4 h-9 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50/80 dark:bg-zinc-900/80">
        {([ ["system", "System", <Server size={12} />], ["preferences", "Preferences", <SlidersHorizontal size={12} />] ] as [SettingsTab, string, React.ReactNode][]).map(([tab, label, icon]) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={clsx(
              "flex items-center gap-1.5 px-3 h-7 rounded text-[12px] font-medium transition-colors",
              activeTab === tab
                ? "bg-gray-100 dark:bg-zinc-800 text-gray-800 dark:text-zinc-200"
                : "text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400",
            )}
          >
            {icon}{label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-5">

        {/* ── Preferences tab ── */}
        {activeTab === "preferences" && <PreferencesTab />}

        {/* ── System tab ── */}
        {activeTab === "system" && (
          isLoading ? (
            <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
              <Loader2 size={16} className="animate-spin" />
              <span className="text-[13px]">Loading settings…</span>
            </div>
          ) : s ? (
            <div className="max-w-4xl space-y-4">

              <Section title="Deployment">
                <KV label="Mode"       value={<ModeBadge mode={s.deployment_mode} />} />
                <KV label="Store"      value={<BackendBadge backend={s.store_backend} />} />
                <KV label="Plugins"    value={s.plugin_count} mono />
              </Section>

              <Section title="System Health">
                <KV label="Providers"    value={s.system_health?.provider_health_count ?? 0} mono />
                <KV label="Plugins"      value={s.system_health?.plugin_health_count ?? 0}  mono />
                <KV label="Credentials"  value={s.system_health?.credential_count ?? 0}     mono />
                <KV
                  label="Degraded components"
                  value={
                    (s.system_health?.degraded_count ?? 0) > 0 ? (
                      <span className="text-[12px] font-semibold text-red-400">
                        {s.system_health?.degraded_count}
                      </span>
                    ) : (
                      <span className="text-[12px] text-emerald-400">None</span>
                    )
                  }
                />
              </Section>

              <Section title="Encryption">
                <KV
                  label="Key configured"
                  value={<BoolChip value={s.key_management?.encryption_key_configured ?? false} />}
                />
                <KV
                  label="Key version"
                  value={
                    s.key_management?.key_version != null
                      ? <span className="font-mono">v{s.key_management?.key_version}</span>
                      : <span className="text-gray-400 dark:text-zinc-600">—</span>
                  }
                />
                <KV
                  label="Last rotation"
                  value={
                    <span className="font-mono text-[12px]">
                      {fmtTime(s.key_management?.last_rotation_at ?? null)}
                    </span>
                  }
                />
              </Section>

              <Section title="TLS">
                <KV
                  label="Status"
                  value={
                    <span className="text-[12px] text-gray-400 dark:text-zinc-500 italic">
                      Managed by the server. Certificate details are available on the Deployment page.
                    </span>
                  }
                />
              </Section>

              <EnvVarsSection settings={s} info={sysInfo} />
              <TransportSection />
              <CorsDiagnosticsSection deploymentMode={s?.deployment_mode} />

              {sysLoading ? (
                <div className="flex items-center gap-2 py-4 text-gray-400 dark:text-zinc-600">
                  <Loader2 size={13} className="animate-spin" />
                  <span className="text-[12px]">Loading system info…</span>
                </div>
              ) : sysInfo ? (
                <SystemInfoSections info={sysInfo} />
              ) : (
                <Section title="Build Information">
                  <div className="py-3 text-[12px] text-gray-400 dark:text-zinc-600 italic">
                    System info unavailable — upgrade cairn-app for this endpoint.
                  </div>
                </Section>
              )}

            </div>
          ) : null
        )}

      </div>
    </div>
  );
}

export default SettingsPage;
