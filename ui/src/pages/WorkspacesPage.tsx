/**
 * WorkspacesPage — discover and switch between workspaces for the current tenant.
 *
 * Workspaces are derived client-side from sessions and runs data
 * (no dedicated backend endpoint exists — workspace membership is implicit
 * in every entity's project.workspace_id field).
 *
 * Switching a workspace updates the global scope via useScope() so that
 * all subsequent API calls are scoped to the new workspace.
 */

import { useState, useMemo } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Layers, Plus, Check, RefreshCw, ArrowRight,
  FolderOpen, Play, MonitorPlay, Clock,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useScope, DEFAULT_SCOPE } from '../hooks/useScope';

// ── Types ─────────────────────────────────────────────────────────────────────

interface WorkspaceInfo {
  workspace_id: string;
  tenant_id:    string;
  project_ids:  Set<string>;
  sessions:     number;
  runs:         number;
  latest_at:    number;   // unix ms of most-recent activity
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtRelative(ms: number): string {
  if (ms === 0) return 'no activity';
  const d = Date.now() - ms;
  if (d < 60_000)      return 'just now';
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h ago`;
  if (d < 604_800_000) return `${Math.floor(d / 86_400_000)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

const WS_ID_RE = /^[a-z0-9][a-z0-9_-]{0,62}$/;

function validateWsId(id: string): string | null {
  if (!id.trim()) return 'Workspace ID is required.';
  if (!WS_ID_RE.test(id)) return 'Use lowercase letters, digits, hyphens, underscores (max 63 chars).';
  return null;
}

// ── Workspace card ────────────────────────────────────────────────────────────

function WorkspaceCard({
  ws,
  isActive,
  onActivate,
}: {
  ws: WorkspaceInfo;
  isActive: boolean;
  onActivate: () => void;
}) {
  const projectCount = ws.project_ids.size;

  return (
    <button
      onClick={onActivate}
      className={clsx(
        'group w-full text-left rounded-xl border p-4 transition-all',
        isActive
          ? 'border-indigo-500/60 bg-indigo-950/20 ring-1 ring-indigo-500/30'
          : 'border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 hover:border-gray-200 dark:border-zinc-700 hover:bg-gray-100/60 dark:hover:bg-zinc-800/60',
      )}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-3 mb-3">
        <div className="flex items-center gap-2.5 min-w-0">
          <div className={clsx(
            'shrink-0 w-8 h-8 rounded-lg flex items-center justify-center',
            isActive ? 'bg-indigo-500/20' : 'bg-gray-100 dark:bg-zinc-800 group-hover:bg-gray-200 dark:hover:bg-zinc-700',
          )}>
            <Layers size={14} className={isActive ? 'text-indigo-400' : 'text-gray-400 dark:text-zinc-500'} />
          </div>
          <div className="min-w-0">
            <p className={clsx(
              'text-[13px] font-medium font-mono truncate',
              isActive ? 'text-indigo-300' : 'text-gray-800 dark:text-zinc-200',
            )}>
              {ws.workspace_id}
            </p>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono mt-0.5">{ws.tenant_id}</p>
          </div>
        </div>

        {isActive ? (
          <span className="shrink-0 inline-flex items-center gap-1 text-[10px] font-medium rounded-full px-2 py-0.5 bg-indigo-500/20 text-indigo-300 border border-indigo-700/40">
            <Check size={9} strokeWidth={3} /> Active
          </span>
        ) : (
          <span className="shrink-0 opacity-0 group-hover:opacity-100 transition-opacity text-gray-400 dark:text-zinc-600 group-hover:text-gray-500 dark:hover:text-zinc-400">
            <ArrowRight size={13} />
          </span>
        )}
      </div>

      {/* Stats row */}
      <div className="grid grid-cols-3 gap-2">
        {[
          { icon: FolderOpen, label: 'Projects', value: projectCount },
          { icon: MonitorPlay, label: 'Sessions', value: ws.sessions },
          { icon: Play,       label: 'Runs',     value: ws.runs      },
        ].map(({ icon: Icon, label, value }) => (
          <div key={label} className="rounded-lg bg-white dark:bg-zinc-950/60 border border-gray-200/60 dark:border-zinc-800/60 px-2.5 py-2 text-center">
            <Icon size={11} className="mx-auto mb-1 text-gray-400 dark:text-zinc-600" />
            <p className="text-[16px] font-semibold text-gray-800 dark:text-zinc-200 tabular-nums leading-none">{value}</p>
            <p className="text-[9px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mt-0.5">{label}</p>
          </div>
        ))}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-1.5 mt-3 pt-2.5 border-t border-gray-200/50 dark:border-zinc-800/50">
        <Clock size={10} className="text-gray-300 dark:text-zinc-700 shrink-0" />
        <span className="text-[10px] text-gray-400 dark:text-zinc-600">
          {fmtRelative(ws.latest_at)}
        </span>
      </div>
    </button>
  );
}

// ── New workspace form ────────────────────────────────────────────────────────

function NewWorkspaceForm({
  tenantId,
  onCreated,
  onCancel,
}: {
  tenantId: string;
  onCreated: (wsId: string) => void;
  onCancel: () => void;
}) {
  const [wsId,  setWsId]  = useState('');
  const [error, setError] = useState<string | null>(null);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmed = wsId.trim();
    const err = validateWsId(trimmed);
    if (err) { setError(err); return; }
    onCreated(trimmed);
  }

  return (
    <form onSubmit={handleSubmit} className="rounded-xl border border-indigo-700/40 bg-gray-50 dark:bg-zinc-900 p-4 space-y-3">
      <p className="text-[12px] font-medium text-gray-700 dark:text-zinc-300">New Workspace</p>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500">
        Workspace IDs are implicit in cairn — creating a new one just activates that scope.
        Data will be automatically scoped to tenant <code className="text-gray-500 dark:text-zinc-400 font-mono">{tenantId}</code>.
      </p>

      <div>
        <label className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider block mb-1.5">
          Workspace ID <span className="text-red-500">*</span>
        </label>
        <input
          value={wsId}
          onChange={e => { setWsId(e.target.value); setError(null); }}
          placeholder="my-workspace"
          autoFocus
          spellCheck={false}
          className={clsx(
            'w-full rounded border bg-white dark:bg-zinc-950 text-[13px] text-gray-800 dark:text-zinc-200 font-mono px-3 py-2',
            'focus:outline-none transition-colors',
            error
              ? 'border-red-700 focus:border-red-500'
              : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
          )}
        />
        {error && <p className="text-[11px] text-red-400 mt-1">{error}</p>}
        <p className="text-[10px] text-gray-300 dark:text-zinc-700 mt-1">
          Lowercase letters, digits, hyphens, underscores · max 63 chars
        </p>
      </div>

      <div className="flex items-center gap-2 justify-end">
        <button
          type="button"
          onClick={onCancel}
          className="px-3 py-1.5 rounded text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={!wsId.trim()}
          className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                     bg-indigo-600 hover:bg-indigo-500 text-white
                     disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 disabled:cursor-not-allowed
                     transition-colors"
        >
          <Check size={11} /> Activate Workspace
        </button>
      </div>
    </form>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function WorkspacesPage() {
  const [scope, setScope]     = useScope();
  const qc                    = useQueryClient();
  const [showNew, setShowNew] = useState(false);
  const [filter, setFilter]   = useState('');

  // Fetch sessions and runs with a wide limit so we can discover all workspaces.
  const { data: sessions, isLoading: sessLoading, refetch: refetchSess, isFetching: sessFetching } = useQuery({
    queryKey: ['sessions', 'all-tenants'],
    queryFn:  () => defaultApi.getSessions({ limit: 500 }),
    staleTime: 60_000,
    // Override scope injection — we want ALL workspaces, not just the active one.
  });

  const { data: runs, isLoading: runsLoading, refetch: refetchRuns, isFetching: runsFetching } = useQuery({
    queryKey: ['runs', 'all-tenants'],
    queryFn:  () => defaultApi.getRuns({ limit: 500 }),
    staleTime: 60_000,
  });

  const isLoading  = sessLoading || runsLoading;
  const isFetching = sessFetching || runsFetching;

  // Build workspace map from entity data.
  const workspaces = useMemo((): WorkspaceInfo[] => {
    const map = new Map<string, WorkspaceInfo>();

    function ensureWs(tenantId: string, wsId: string) {
      const key = `${tenantId}::${wsId}`;
      if (!map.has(key)) {
        map.set(key, {
          workspace_id: wsId,
          tenant_id:    tenantId,
          project_ids:  new Set(),
          sessions:     0,
          runs:         0,
          latest_at:    0,
        });
      }
      return map.get(key)!;
    }

    for (const s of sessions ?? []) {
      const ws = ensureWs(s.project.tenant_id, s.project.workspace_id);
      ws.sessions++;
      ws.project_ids.add(s.project.project_id);
      ws.latest_at = Math.max(ws.latest_at, s.created_at);
    }

    for (const r of runs ?? []) {
      const ws = ensureWs(r.project.tenant_id, r.project.workspace_id);
      ws.runs++;
      ws.project_ids.add(r.project.project_id);
      ws.latest_at = Math.max(ws.latest_at, r.created_at);
    }

    // Always include the currently-active workspace even if it has no data.
    ensureWs(scope.tenant_id, scope.workspace_id);

    // Also include the default workspace so there's always at least one card.
    ensureWs(DEFAULT_SCOPE.tenant_id, DEFAULT_SCOPE.workspace_id);

    return Array.from(map.values()).sort((a, b) => b.latest_at - a.latest_at);
  }, [sessions, runs, scope.tenant_id, scope.workspace_id]);

  const filtered = filter.trim()
    ? workspaces.filter(ws =>
        ws.workspace_id.toLowerCase().includes(filter.toLowerCase()) ||
        ws.tenant_id.toLowerCase().includes(filter.toLowerCase()),
      )
    : workspaces;

  function activate(ws: WorkspaceInfo) {
    setScope({
      tenant_id:    ws.tenant_id,
      workspace_id: ws.workspace_id,
      project_id:   DEFAULT_SCOPE.project_id,
    });
    void qc.invalidateQueries();
  }

  function handleNewWorkspace(wsId: string) {
    setScope({
      tenant_id:    scope.tenant_id,
      workspace_id: wsId,
      project_id:   DEFAULT_SCOPE.project_id,
    });
    setShowNew(false);
    void qc.invalidateQueries();
  }

  // Aggregate stats across all discovered workspaces.
  const totalProjects  = new Set(workspaces.flatMap(ws => [...ws.project_ids])).size;
  const totalSessions  = workspaces.reduce((s, ws) => s + ws.sessions, 0);
  const totalRuns      = workspaces.reduce((s, ws) => s + ws.runs, 0);

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950 overflow-y-auto">
      <div className="max-w-5xl mx-auto px-5 py-5 space-y-6 w-full">

        {/* Header */}
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-lg bg-indigo-500/10 flex items-center justify-center shrink-0">
              <Layers size={16} className="text-indigo-400" />
            </div>
            <div>
              <h1 className="text-[15px] font-semibold text-gray-900 dark:text-zinc-100">Workspaces</h1>
              <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
                Tenant: <code className="text-gray-500 dark:text-zinc-400 font-mono">{scope.tenant_id}</code>
                {' · '}Active: <code className="text-indigo-400 font-mono">{scope.workspace_id}</code>
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <button
              onClick={() => { void refetchSess(); void refetchRuns(); }}
              disabled={isFetching}
              className="p-1.5 rounded text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors disabled:opacity-40"
              title="Refresh"
            >
              <RefreshCw size={14} className={isFetching ? 'animate-spin' : ''} />
            </button>
            <button
              onClick={() => setShowNew(v => !v)}
              className={clsx(
                'flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium transition-colors',
                showNew
                  ? 'bg-indigo-600/20 text-indigo-400 border border-indigo-700/40'
                  : 'bg-indigo-600 hover:bg-indigo-500 text-white',
              )}
            >
              <Plus size={12} /> New Workspace
            </button>
          </div>
        </div>

        {/* Summary stats */}
        {!isLoading && workspaces.length > 0 && (
          <div className="grid grid-cols-4 gap-3">
            {[
              { label: 'Workspaces', value: workspaces.length, color: 'border-l-indigo-500' },
              { label: 'Projects',   value: totalProjects,      color: 'border-l-violet-500' },
              { label: 'Sessions',   value: totalSessions,      color: 'border-l-blue-500'   },
              { label: 'Runs',       value: totalRuns,          color: 'border-l-emerald-500' },
            ].map(({ label, value, color }) => (
              <div key={label} className={clsx('rounded-lg border border-gray-200 dark:border-zinc-800 border-l-2 bg-gray-50 dark:bg-zinc-900 px-4 py-3', color)}>
                <p className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
                <p className="text-[22px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight mt-1">{value}</p>
              </div>
            ))}
          </div>
        )}

        {/* New workspace form */}
        {showNew && (
          <NewWorkspaceForm
            tenantId={scope.tenant_id}
            onCreated={handleNewWorkspace}
            onCancel={() => setShowNew(false)}
          />
        )}

        {/* Search */}
        <div className="relative">
          <input
            value={filter}
            onChange={e => setFilter(e.target.value)}
            placeholder="Filter workspaces…"
            className="w-full rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[13px] text-gray-800 dark:text-zinc-200
                       placeholder-zinc-600 px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors"
          />
          {filter && (
            <button
              onClick={() => setFilter('')}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
            >
              ×
            </button>
          )}
        </div>

        {/* Workspace grid */}
        {isLoading ? (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {[1, 2, 3].map(i => (
              <div key={i} className="rounded-xl border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 p-4 animate-pulse">
                <div className="flex items-center gap-2.5 mb-3">
                  <div className="w-8 h-8 rounded-lg bg-gray-100 dark:bg-zinc-800" />
                  <div className="space-y-1.5 flex-1">
                    <div className="h-3 w-32 rounded bg-gray-100 dark:bg-zinc-800" />
                    <div className="h-2 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  {[1, 2, 3].map(j => <div key={j} className="h-14 rounded-lg bg-gray-100 dark:bg-zinc-800" />)}
                </div>
              </div>
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
            <Layers size={28} className="text-gray-300 dark:text-zinc-700" />
            <p className="text-[13px] text-gray-400 dark:text-zinc-600">
              {filter ? `No workspaces match "${filter}"` : 'No workspaces discovered yet.'}
            </p>
            {!filter && (
              <p className="text-[11px] text-gray-300 dark:text-zinc-700">
                Create sessions or runs to populate workspaces, or create one above.
              </p>
            )}
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {filtered.map(ws => (
              <WorkspaceCard
                key={`${ws.tenant_id}::${ws.workspace_id}`}
                ws={ws}
                isActive={ws.workspace_id === scope.workspace_id && ws.tenant_id === scope.tenant_id}
                onActivate={() => activate(ws)}
              />
            ))}
          </div>
        )}

        {/* Scope hint */}
        {!isLoading && workspaces.length > 0 && (
          <p className="text-[11px] text-gray-300 dark:text-zinc-700 text-center pb-2">
            Clicking a workspace updates the global scope — all data views filter to that workspace immediately.
          </p>
        )}

      </div>
    </div>
  );
}

export default WorkspacesPage;
