/**
 * WorkspacesPage — discover and switch between workspaces for the current tenant.
 *
 * Workspaces are persisted server-side and fetched from the admin API.
 * This page intentionally uses that persisted list as the source of truth so
 * newly created workspaces survive navigation and reloads.
 */

import { useState, useMemo } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Layers, Plus, Check, RefreshCw, ArrowRight, Clock,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useScope, DEFAULT_SCOPE } from '../hooks/useScope';
import { ErrorFallback } from '../components/ErrorFallback';
import { useToast } from '../components/Toast';

// ── Types ─────────────────────────────────────────────────────────────────────

interface WorkspaceInfo {
  workspace_id: string;
  tenant_id:    string;
  name:         string;
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
      <div className="flex items-start justify-between gap-3">
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

      {/* Footer */}
      <div className="flex items-center gap-1.5 mt-4 pt-2.5 border-t border-gray-200/50 dark:border-zinc-800/50">
        <Clock size={10} className="text-gray-300 dark:text-zinc-600 shrink-0" />
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
  creating,
}: {
  tenantId: string;
  onCreated: (wsId: string) => Promise<void>;
  onCancel: () => void;
  creating: boolean;
}) {
  const [wsId,  setWsId]  = useState('');
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmed = wsId.trim();
    const err = validateWsId(trimmed);
    if (err) { setError(err); return; }
    try {
      await onCreated(trimmed);
    } catch (submitError) {
      setError(submitError instanceof Error ? submitError.message : 'Failed to create workspace.');
    }
  }

  return (
    <form onSubmit={handleSubmit} className="rounded-xl border border-indigo-700/40 bg-gray-50 dark:bg-zinc-900 p-4 space-y-3">
      <p className="text-[12px] font-medium text-gray-700 dark:text-zinc-300">New Workspace</p>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500">
        Creates a persisted workspace in tenant <code className="text-gray-500 dark:text-zinc-400 font-mono">{tenantId}</code>
        {' '}and then switches the operator scope into it.
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
        <p className="text-[10px] text-gray-300 dark:text-zinc-600 mt-1">
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
          disabled={!wsId.trim() || creating}
          className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                     bg-indigo-600 hover:bg-indigo-500 text-white
                     disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 disabled:cursor-not-allowed
                     transition-colors"
        >
          <Check size={11} /> {creating ? 'Creating…' : 'Create Workspace'}
        </button>
      </div>
    </form>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function WorkspacesPage() {
  const [scope, setScope]     = useScope();
  const qc                    = useQueryClient();
  const toast                 = useToast();
  const [showNew, setShowNew] = useState(false);
  const [filter, setFilter]   = useState('');

  const {
    data: workspaceRecords,
    isLoading: wsLoading,
    isError: wsError,
    error: workspaceError,
    refetch: refetchWorkspaces,
    isFetching: wsFetching,
  } = useQuery({
    queryKey: ['workspaces', scope.tenant_id],
    queryFn:  () => defaultApi.getWorkspaces(scope.tenant_id, { limit: 200 }),
    staleTime: 60_000,
  });

  const createWorkspace = useMutation({
    mutationFn: (workspaceId: string) =>
      defaultApi.createWorkspace(scope.tenant_id, {
        workspace_id: workspaceId,
        name: workspaceId,
      }),
    onError: (e: unknown) =>
      toast.error(e instanceof Error ? e.message : 'Failed to create workspace.'),
  });

  const isLoading  = wsLoading;
  const isFetching = wsFetching;

  // Build workspace map from persisted records only.
  const workspaces = useMemo((): WorkspaceInfo[] => {
    const map = new Map<string, WorkspaceInfo>();

    function ensureWs(tenantId: string, wsId: string, name = wsId, latestAt = 0) {
      const key = `${tenantId}::${wsId}`;
      if (!map.has(key)) {
        map.set(key, {
          workspace_id: wsId,
          tenant_id:    tenantId,
          name,
          latest_at:    latestAt,
        });
      }
      return map.get(key)!;
    }

    for (const workspace of workspaceRecords ?? []) {
      ensureWs(
        workspace.tenant_id,
        workspace.workspace_id,
        workspace.name,
        Math.max(workspace.updated_at, workspace.created_at),
      );
    }

    // Always include the currently-active workspace even if it has no data.
    ensureWs(scope.tenant_id, scope.workspace_id);

    // Also include the default workspace so there's always at least one card.
    ensureWs(DEFAULT_SCOPE.tenant_id, DEFAULT_SCOPE.workspace_id);

    return Array.from(map.values()).sort((a, b) => b.latest_at - a.latest_at);
  }, [workspaceRecords, scope.tenant_id, scope.workspace_id]);

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
    // Invalidate data queries so they refetch with the new scope,
    // but exclude the workspace list itself to prevent a flash.
    void qc.invalidateQueries({
      predicate: (query) => query.queryKey[0] !== 'workspaces',
    });
  }

  async function handleNewWorkspace(wsId: string) {
    const created = await createWorkspace.mutateAsync(wsId);
    await qc.invalidateQueries({ queryKey: ['workspaces', scope.tenant_id] });
    setScope({
      tenant_id:    created.tenant_id,
      workspace_id: created.workspace_id,
      project_id:   DEFAULT_SCOPE.project_id,
    });
    setShowNew(false);
    void qc.invalidateQueries();
  }

  if (wsError) {
    return <ErrorFallback error={workspaceError} resource="workspaces" onRetry={() => void refetchWorkspaces()} />;
  }

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
              onClick={() => {
                void refetchWorkspaces();
              }}
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

        {/* Summary stat */}
        {!isLoading && workspaces.length > 0 && (
          <div className="rounded-lg border border-gray-200 dark:border-zinc-800 border-l-2 border-l-indigo-500 bg-gray-50 dark:bg-zinc-900 px-4 py-3 inline-block">
            <p className="text-[10px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Workspaces</p>
            <p className="text-[22px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight mt-1">{workspaces.length}</p>
          </div>
        )}

        {/* New workspace form */}
        {showNew && (
          <NewWorkspaceForm
            tenantId={scope.tenant_id}
            onCreated={handleNewWorkspace}
            onCancel={() => setShowNew(false)}
            creating={createWorkspace.isPending}
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
                <div className="flex items-center gap-2.5">
                  <div className="w-8 h-8 rounded-lg bg-gray-100 dark:bg-zinc-800" />
                  <div className="space-y-1.5 flex-1">
                    <div className="h-3 w-32 rounded bg-gray-100 dark:bg-zinc-800" />
                    <div className="h-2 w-20 rounded bg-gray-100 dark:bg-zinc-800" />
                  </div>
                </div>
                <div className="h-3 w-24 rounded bg-gray-100 dark:bg-zinc-800 mt-4" />
              </div>
            ))}
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
            <Layers size={28} className="text-gray-300 dark:text-zinc-600" />
            <p className="text-[13px] text-gray-400 dark:text-zinc-600">
              {filter ? `No workspaces match "${filter}"` : 'No workspaces discovered yet.'}
            </p>
            {!filter && (
              <p className="text-[11px] text-gray-300 dark:text-zinc-600">
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
          <p className="text-[11px] text-gray-300 dark:text-zinc-600 text-center pb-2">
            Clicking a workspace updates the global scope — all data views filter to that workspace immediately.
          </p>
        )}

      </div>
    </div>
  );
}

export default WorkspacesPage;
