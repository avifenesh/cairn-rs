import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, Plus, ChevronDown, ChevronRight,
  Trash2, X, Puzzle, Terminal, Download, ShieldCheck, Power, PowerOff,
  Key, Store,
} from 'lucide-react';
import { ErrorFallback } from '../components/ErrorFallback';
import { defaultApi } from '../lib/api';
import { useFocusTrap } from '../hooks/useFocusTrap';
import type { PluginManifest, PluginCapability, PluginDetailResponse, CatalogEntry } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function capabilityLabel(cap: PluginCapability): string {
  switch (cap.type) {
    case 'tool_provider':    return `Tools (${cap.tools.length})`;
    case 'signal_source':    return `Signals (${cap.signals.length})`;
    case 'channel_provider': return `Channels (${cap.channels.length})`;
    case 'post_turn_hook':   return 'Post-turn Hook';
    case 'policy_hook':      return 'Policy Hook';
    case 'eval_scorer':      return 'Eval Scorer';
    case 'mcp_server':       return 'MCP Server';
    default:                 return (cap as { type: string }).type;
  }
}

function stateColors(state: string): string {
  switch (state) {
    case 'ready':                        return 'text-emerald-400 bg-emerald-400/10 border-emerald-400/20';
    case 'spawning': case 'handshaking':
    case 'discovered':                   return 'text-amber-400 bg-amber-400/10 border-amber-400/20';
    case 'failed':                       return 'text-red-400 bg-red-400/10 border-red-400/20';
    case 'stopped': case 'draining':     return 'text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700';
    default:                             return 'text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700';
  }
}

function fmtUptime(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub }: { label: string; value: string | number; sub?: string }) {
  return (
    <div className="border-l-2 border-indigo-500 pl-3 py-0.5">
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

// ── Plugin detail panel ───────────────────────────────────────────────────────

function PluginDetail({ pluginId }: { pluginId: string }) {
  const [tab, setTab] = useState<'config' | 'logs' | 'health'>('config');

  const { data, isLoading } = useQuery({
    queryKey: ['plugin-detail', pluginId],
    queryFn: () => defaultApi.getPlugin(pluginId),
    staleTime: 10_000,
  });

  const { data: logsData } = useQuery({
    queryKey: ['plugin-logs', pluginId],
    queryFn: () => defaultApi.getPluginLogs(pluginId),
    enabled: tab === 'logs',
    staleTime: 5_000,
  });

  if (isLoading) return (
    <div className="flex items-center gap-2 px-4 py-3 text-gray-400 dark:text-zinc-600 text-[12px]">
      <Loader2 size={12} className="animate-spin" /> Loading detail…
    </div>
  );

  const d = data as PluginDetailResponse | undefined;

  return (
    <div className="border-t border-gray-200 dark:border-zinc-800">
      {/* Tab bar */}
      <div className="flex items-center gap-0.5 px-4 pt-2.5 pb-1.5">
        {(['config', 'logs', 'health'] as const).map(t => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`px-2.5 py-1 rounded text-[11px] font-medium capitalize transition-colors ${
              tab === t ? 'bg-gray-100 dark:bg-zinc-800 text-gray-900 dark:text-zinc-100' : 'text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300'
            }`}
          >
            {t === 'health' ? 'Health' : t === 'logs' ? 'Logs' : 'Config'}
          </button>
        ))}
        {d?.lifecycle && (
          <span className="ml-auto text-[10px] font-mono text-gray-400 dark:text-zinc-600">
            uptime {fmtUptime(d.lifecycle.uptime_ms)}
          </span>
        )}
      </div>

      <div className="px-4 pb-4">
        {tab === 'config' && (
          <pre className="text-[11px] font-mono text-gray-500 dark:text-zinc-400 bg-white dark:bg-zinc-950 rounded-md p-3 overflow-x-auto max-h-56 leading-relaxed">
            {d ? JSON.stringify({
              id:              d.manifest.id,
              command:         d.manifest.command,
              execution_class: d.manifest.execution_class,
              limits:          d.manifest.limits,
              homepage:        d.manifest.homepage,
            }, null, 2) : 'No config data available.'}
          </pre>
        )}

        {tab === 'logs' && (
          <div className="bg-white dark:bg-zinc-950 rounded-md p-3 max-h-56 overflow-y-auto">
            {logsData?.entries && logsData.entries.length > 0 ? (
              logsData.entries.map((entry, i) => (
                <div key={i} className="flex gap-3 py-0.5">
                  <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-600 shrink-0">
                    {new Date(entry.timestamp_ms).toLocaleTimeString()}
                  </span>
                  <span className={`text-[10px] font-mono shrink-0 ${
                    entry.level === 'error' ? 'text-red-400' :
                    entry.level === 'warn'  ? 'text-amber-400' : 'text-gray-400 dark:text-zinc-500'
                  }`}>{entry.level.toUpperCase()}</span>
                  <span className="text-[11px] font-mono text-gray-500 dark:text-zinc-400">{entry.message}</span>
                </div>
              ))
            ) : (
              <p className="text-[11px] font-mono text-gray-400 dark:text-zinc-600">No recent log entries.</p>
            )}
          </div>
        )}

        {tab === 'health' && (
          <div className="space-y-2">
            {d?.metrics ? (
              <div className="grid grid-cols-3 gap-3">
                {[
                  { label: 'Invocations', value: d.metrics.invocation_count.toLocaleString() },
                  { label: 'Errors',      value: d.metrics.error_count.toLocaleString() },
                  { label: 'Avg Latency', value: `${d.metrics.avg_latency_ms.toFixed(1)}ms` },
                ].map(({ label, value }) => (
                  <div key={label} className="bg-white dark:bg-zinc-950 rounded-md p-2.5">
                    <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">{label}</p>
                    <p className="text-[15px] font-semibold text-gray-800 dark:text-zinc-200 tabular-nums">{value}</p>
                  </div>
                ))}
              </div>
            ) : (
              <p className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono">No health data available.</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// ── Plugin card ───────────────────────────────────────────────────────────────

function PluginCard({
  manifest,
  expanded,
  onToggle,
  onUnregister,
}: {
  manifest: PluginManifest;
  expanded: boolean;
  onToggle: () => void;
  onUnregister: () => void;
}) {
  return (
    <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
      {/* Header row */}
      <div
        className="flex items-start gap-3 px-4 py-3 cursor-pointer hover:bg-white/[0.02] transition-colors select-none"
        onClick={onToggle}
      >
        {/* Icon */}
        <div className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
          <Puzzle size={13} className="text-gray-500 dark:text-zinc-400" />
        </div>

        {/* Main info */}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-[13px] font-medium text-gray-900 dark:text-zinc-100">{manifest.name}</span>
            <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500">v{manifest.version}</span>
            <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-600">{manifest.id}</span>
          </div>
          {manifest.description && (
            <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-0.5 truncate">{manifest.description}</p>
          )}

          {/* Capability tags */}
          {manifest.capabilities.length > 0 && (
            <div className="flex flex-wrap gap-1 mt-1.5">
              {manifest.capabilities.map((cap, i) => (
                <span key={i} className="inline-flex items-center px-1.5 py-0.5 rounded bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 text-[10px] text-gray-500 dark:text-zinc-400">
                  {capabilityLabel(cap)}
                </span>
              ))}
            </div>
          )}

          {/* Command preview */}
          {manifest.command.length > 0 && (
            <div className="flex items-center gap-1.5 mt-1.5">
              <Terminal size={10} className="text-gray-400 dark:text-zinc-600 shrink-0" />
              <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-600 truncate">
                {manifest.command.join(' ')}
              </span>
            </div>
          )}
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2 shrink-0">
          <button
            onClick={e => { e.stopPropagation(); onUnregister(); }}
            title="Unregister plugin"
            className="flex items-center gap-1 px-2 py-1 rounded bg-gray-100 dark:bg-zinc-800 text-red-500/80 text-[11px] hover:bg-red-500/10 hover:text-red-400 transition-colors"
          >
            <Trash2 size={10} /> Unregister
          </button>
          {expanded
            ? <ChevronDown size={14} className="text-gray-400 dark:text-zinc-500" />
            : <ChevronRight size={14} className="text-gray-400 dark:text-zinc-500" />
          }
        </div>
      </div>

      {/* Detail panel — lazy-loaded */}
      {expanded && <PluginDetail pluginId={manifest.id} />}
    </div>
  );
}

// ── Register modal ────────────────────────────────────────────────────────────

const DEFAULT_MANIFEST = `{
  "id": "my-plugin",
  "name": "My Plugin",
  "version": "0.1.0",
  "command": ["./my-plugin"],
  "capabilities": [{ "type": "tool_provider", "tools": [] }],
  "permissions": {},
  "execution_class": "sandboxed_process"
}`;

function RegisterModal({ onClose }: { onClose: () => void }) {
  const [json, setJson]     = useState(DEFAULT_MANIFEST);
  const [parseErr, setErr]  = useState<string | null>(null);
  const queryClient         = useQueryClient();

  const { mutate, isPending, error: mutErr } = useMutation({
    mutationFn: (m: Record<string, unknown>) => defaultApi.registerPlugin(m),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['plugins'] });
      onClose();
    },
  });

  function submit() {
    setErr(null);
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(json) as Record<string, unknown>;
    } catch (e) {
      setErr(`Invalid JSON: ${(e as Error).message}`);
      return;
    }
    mutate(parsed);
  }

  const displayErr = parseErr ?? (mutErr instanceof Error ? mutErr.message : null);

  const trapRef = useFocusTrap({ onClose: onClose });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-lg mx-4 shadow-2xl"
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
          <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Register Plugin</span>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>

        <div className="p-4 space-y-3">
          <p className="text-[11px] text-gray-400 dark:text-zinc-500">
            Paste a plugin manifest as JSON. See the{' '}
            <span className="text-gray-500 dark:text-zinc-400">RFC 007</span> spec for the full schema.
          </p>
          <textarea
            value={json}
            onChange={e => setJson(e.target.value)}
            className="w-full h-56 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-3 py-2.5
                       text-[11px] font-mono text-gray-700 dark:text-zinc-300 resize-none leading-relaxed
                       focus:outline-none focus:border-indigo-500 transition-colors"
            spellCheck={false}
          />
          {displayErr && (
            <p className="text-[11px] text-red-400 font-mono">{displayErr}</p>
          )}
          <div className="flex justify-end gap-2">
            <button
              onClick={onClose}
              className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={submit}
              disabled={isPending}
              className="px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 disabled:opacity-50 transition-colors flex items-center gap-1.5"
            >
              {isPending && <Loader2 size={11} className="animate-spin" />}
              {isPending ? 'Registering…' : 'Register'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Credential Wizard Modal ───────────────────────────────────────────────────

function CredentialWizardModal({
  pluginId,
  onClose,
}: {
  pluginId: string;
  onClose: () => void;
}) {
  const [creds, setCreds] = useState<Record<string, string>>({});
  const queryClient = useQueryClient();
  const trapRef = useFocusTrap({ onClose });

  const { mutate, isPending, error } = useMutation({
    mutationFn: () => defaultApi.providePluginCredentials(pluginId, creds),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['catalog'] });
      onClose();
    },
  });

  // Common credential keys for GitHub plugin (can be extended)
  const fields = [
    { key: 'github_app_id', label: 'GitHub App ID', required: true },
    { key: 'github_app_private_key', label: 'Private Key (PEM)', required: true },
  ];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-md mx-4 shadow-2xl"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
          <div className="flex items-center gap-2">
            <Key size={13} className="text-amber-400" />
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Credentials for {pluginId}</span>
          </div>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>
        <div className="p-4 space-y-3">
          {fields.map(f => (
            <div key={f.key}>
              <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1">{f.label}</label>
              <input
                type={f.key.includes('key') || f.key.includes('secret') ? 'password' : 'text'}
                value={creds[f.key] ?? ''}
                onChange={e => setCreds(prev => ({ ...prev, [f.key]: e.target.value }))}
                className="w-full bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-3 py-2 text-[12px] text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500"
                placeholder={f.required ? 'Required' : 'Optional'}
              />
            </div>
          ))}
          {error instanceof Error && (
            <p className="text-[11px] text-red-400">{error.message}</p>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <button onClick={onClose} className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors">
              Cancel
            </button>
            <button
              onClick={() => mutate()}
              disabled={isPending || !creds.github_app_id?.trim()}
              className="px-3 py-1.5 rounded bg-amber-600 text-white text-[12px] hover:bg-amber-500 disabled:opacity-50 transition-colors flex items-center gap-1.5"
            >
              {isPending && <Loader2 size={11} className="animate-spin" />}
              Save Credentials
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Marketplace Catalog Card ─────────────────────────────────────────────────

function CatalogCard({ entry }: { entry: CatalogEntry }) {
  const queryClient = useQueryClient();
  const [showCreds, setShowCreds] = useState(false);
  const [enableProject, setEnableProject] = useState('');

  const installMut = useMutation({
    mutationFn: () => defaultApi.installPlugin(entry.id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['catalog'] }),
  });

  const verifyMut = useMutation({
    mutationFn: () => defaultApi.verifyPlugin(entry.id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['catalog'] }),
  });

  const enableMut = useMutation({
    mutationFn: (project: string) => defaultApi.enablePluginForProject(project, entry.id),
    onSuccess: () => {
      setEnableProject('');
      queryClient.invalidateQueries({ queryKey: ['catalog'] });
    },
  });

  const disableMut = useMutation({
    mutationFn: (project: string) => defaultApi.disablePluginForProject(project, entry.id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['catalog'] }),
  });

  const isInstalled = entry.state === 'installed' || entry.state === 'Installed';
  const isListed = entry.state === 'listed' || entry.state === 'Listed';

  return (
    <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
      <div className="px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          {/* Info */}
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 flex-wrap">
              <Store size={13} className="text-indigo-400 shrink-0" />
              <span className="text-[13px] font-medium text-gray-900 dark:text-zinc-100">{entry.name}</span>
              <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500">v{entry.version}</span>
              <span className={`px-1.5 py-0.5 rounded text-[10px] font-medium border ${
                isInstalled
                  ? 'bg-emerald-950 text-emerald-300 border-emerald-800/50'
                  : 'bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700'
              }`}>
                {entry.state}
              </span>
            </div>
            <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-0.5">{entry.description}</p>
            <div className="flex items-center gap-3 mt-1.5">
              <span className="text-[10px] text-gray-400 dark:text-zinc-600">{entry.category}</span>
              <span className="text-[10px] text-gray-400 dark:text-zinc-600">{entry.tools_count} tools</span>
              <span className="text-[10px] text-gray-400 dark:text-zinc-600">{entry.signals_count} signals</span>
              <span className="text-[10px] text-gray-400 dark:text-zinc-600">by {entry.vendor}</span>
            </div>
          </div>

          {/* Actions */}
          <div className="flex items-center gap-1.5 shrink-0">
            {isListed && (
              <button
                onClick={() => installMut.mutate()}
                disabled={installMut.isPending}
                className="flex items-center gap-1 px-2.5 py-1.5 rounded bg-indigo-600 text-white text-[11px] font-medium hover:bg-indigo-500 disabled:opacity-50 transition-colors"
              >
                {installMut.isPending ? <Loader2 size={10} className="animate-spin" /> : <Download size={10} />}
                Install
              </button>
            )}
            {isInstalled && (
              <>
                <button
                  onClick={() => setShowCreds(true)}
                  className="flex items-center gap-1 px-2 py-1.5 rounded bg-amber-600/80 text-white text-[11px] hover:bg-amber-500 transition-colors"
                  title="Configure credentials"
                >
                  <Key size={10} /> Credentials
                </button>
                <button
                  onClick={() => verifyMut.mutate()}
                  disabled={verifyMut.isPending}
                  className="flex items-center gap-1 px-2 py-1.5 rounded bg-zinc-700 text-zinc-200 text-[11px] hover:bg-zinc-600 disabled:opacity-50 transition-colors"
                  title="Verify credentials"
                >
                  {verifyMut.isPending ? <Loader2 size={10} className="animate-spin" /> : <ShieldCheck size={10} />}
                  Verify
                </button>
              </>
            )}
          </div>
        </div>

        {/* Per-project enable/disable — shown when installed */}
        {isInstalled && (
          <div className="mt-3 pt-3 border-t border-gray-200 dark:border-zinc-800">
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Per-Project</p>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={enableProject}
                onChange={e => setEnableProject(e.target.value)}
                placeholder="project_id"
                className="flex-1 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded px-2.5 py-1.5 text-[11px] text-gray-700 dark:text-zinc-300 font-mono focus:outline-none focus:border-indigo-500"
              />
              <button
                onClick={() => enableProject.trim() && enableMut.mutate(enableProject.trim())}
                disabled={enableMut.isPending || !enableProject.trim()}
                className="flex items-center gap-1 px-2 py-1.5 rounded bg-emerald-600/80 text-white text-[11px] hover:bg-emerald-500 disabled:opacity-50 transition-colors"
              >
                <Power size={10} /> Enable
              </button>
              <button
                onClick={() => enableProject.trim() && disableMut.mutate(enableProject.trim())}
                disabled={disableMut.isPending || !enableProject.trim()}
                className="flex items-center gap-1 px-2 py-1.5 rounded bg-zinc-700 text-zinc-300 text-[11px] hover:bg-zinc-600 disabled:opacity-50 transition-colors"
              >
                <PowerOff size={10} /> Disable
              </button>
            </div>
          </div>
        )}
      </div>

      {showCreds && <CredentialWizardModal pluginId={entry.id} onClose={() => setShowCreds(false)} />}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function PluginsPage() {
  const [expanded,  setExpanded]  = useState<string | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [tab, setTab] = useState<'marketplace' | 'registered'>('marketplace');
  const queryClient = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ['plugins'],
    queryFn:  () => defaultApi.getPlugins(),
    refetchInterval: 15_000,
  });

  const { data: catalogData, isLoading: catalogLoading } = useQuery({
    queryKey: ['catalog'],
    queryFn: () => defaultApi.getPluginCatalog(),
    refetchInterval: 30_000,
  });

  const { mutate: unregister } = useMutation({
    mutationFn: (id: string) => defaultApi.deletePlugin(id),
    onSuccess:  () => queryClient.invalidateQueries({ queryKey: ['plugins'] }),
  });

  const plugins = data?.items ?? [];
  const catalogEntries = catalogData?.plugins ?? [];

  if (isError) return <ErrorFallback error={error} resource="plugins" onRetry={() => void refetch()} />;

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        {/* Tab buttons */}
        <div className="flex items-center gap-1">
          {([
            { key: 'marketplace' as const, label: 'Marketplace', icon: Store, count: catalogEntries.length },
            { key: 'registered' as const, label: 'Registered', icon: Puzzle, count: plugins.length },
          ]).map(t => (
            <button
              key={t.key}
              onClick={() => setTab(t.key)}
              className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-[12px] font-medium transition-colors ${
                tab === t.key
                  ? 'bg-gray-100 dark:bg-zinc-800 text-gray-900 dark:text-zinc-100'
                  : 'text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300'
              }`}
            >
              <t.icon size={11} />
              {t.label}
              {!isLoading && !catalogLoading && (
                <span className="text-[10px] text-gray-400 dark:text-zinc-600">{t.count}</span>
              )}
            </button>
          ))}
        </div>
        {tab === 'registered' && (
          <button
            onClick={() => setShowModal(true)}
            className="ml-auto flex items-center gap-1.5 px-2.5 py-1 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
          >
            <Plus size={11} /> Register Plugin
          </button>
        )}
        <button
          onClick={() => { refetch(); queryClient.invalidateQueries({ queryKey: ['catalog'] }); }}
          disabled={isFetching}
          className={`${tab === 'registered' ? '' : 'ml-auto'} flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors`}
        >
          <RefreshCw size={11} className={isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && (
        <div className="grid grid-cols-3 gap-x-6 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <StatCard label="Total"     value={plugins.length} />
          <StatCard label="Active"    value="—" sub="expand to check" />
          <StatCard label="Errored"   value="—" sub="expand to check" />
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-2">
        {tab === 'marketplace' ? (
          /* ── Marketplace Catalog ─────────────────────────────────────── */
          catalogLoading ? (
            <div className="flex items-center justify-center min-h-32 gap-2 text-gray-400 dark:text-zinc-600">
              <Loader2 size={16} className="animate-spin" />
              <span className="text-[13px]">Loading catalog…</span>
            </div>
          ) : catalogEntries.length === 0 ? (
            <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
              <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
                <Store size={24} className="text-gray-400 dark:text-zinc-500" />
              </div>
              <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No catalog entries</p>
              <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
                The plugin marketplace is empty. Plugins will appear here when published to the catalog.
              </p>
            </div>
          ) : (
            catalogEntries.map(entry => (
              <CatalogCard key={entry.id} entry={entry} />
            ))
          )
        ) : (
          /* ── Registered Plugins ──────────────────────────────────────── */
          isLoading ? (
            <div className="flex items-center justify-center min-h-32 gap-2 text-gray-400 dark:text-zinc-600">
              <Loader2 size={16} className="animate-spin" />
              <span className="text-[13px]">Loading…</span>
            </div>
          ) : plugins.length === 0 ? (
            <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
              <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
                <Puzzle size={24} className="text-gray-400 dark:text-zinc-500" />
              </div>
              <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No plugins registered</p>
              <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
                Register a plugin to extend cairn with custom tools, signal sources, eval scorers, and MCP servers.
              </p>
              <button
                onClick={() => setShowModal(true)}
                className="mt-1 flex items-center gap-1.5 px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
              >
                <Plus size={11} /> Register Plugin
              </button>
            </div>
          ) : (
            plugins.map(manifest => (
              <PluginCard
                key={manifest.id}
                manifest={manifest}
                expanded={expanded === manifest.id}
                onToggle={() => setExpanded(v => v === manifest.id ? null : manifest.id)}
                onUnregister={() => unregister(manifest.id)}
              />
            ))
          )
        )}
      </div>

      {/* Badge legend */}
      {plugins.length > 0 && (
        <div className="flex items-center gap-4 px-5 py-2 border-t border-gray-200 dark:border-zinc-800 shrink-0">
          {(['ready', 'spawning', 'failed', 'stopped'] as const).map(s => (
            <div key={s} className="flex items-center gap-1.5">
              <span className={`inline-flex px-1.5 py-0.5 rounded border text-[10px] font-medium ${stateColors(s)}`}>
                {s}
              </span>
            </div>
          ))}
        </div>
      )}

      {showModal && <RegisterModal onClose={() => setShowModal(false)} />}
    </div>
  );
}

export default PluginsPage;
