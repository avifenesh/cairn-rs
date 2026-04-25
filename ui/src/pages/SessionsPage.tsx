import { useMemo, useRef } from 'react';
import { useQuery, useQueryClient, useMutation } from '@tanstack/react-query';
import { ChevronRight, RefreshCw, Plus, Upload } from 'lucide-react';
import { DataTable } from '../components/DataTable';
import { StatCard } from '../components/StatCard';
import { ErrorFallback } from '../components/ErrorFallback';
import { EmptyScopeHint } from '../components/EmptyScopeHint';
import { useToast } from '../components/Toast';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { sectionLabel } from '../lib/design-system';
import type { SessionRecord, SessionState } from '../lib/types';
import { EntityExplainer } from '../components/EntityExplainer';
import { ENTITY_EXPLAINERS } from '../lib/entityExplainers';

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTs(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
  });
}

function fmtRelative(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000)      return 'just now';
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h ago`;
  if (d < 604_800_000) return `${Math.floor(d / 86_400_000)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function mono(s: string, max = 18): string {
  return s.length > max ? `${s.slice(0, max - 3)}…` : s;
}

// ── Session state pill — compact version ──────────────────────────────────────

const SESSION_PILL: Record<SessionState, string> = {
  open:      'bg-blue-500/10 text-blue-400 border-blue-500/20',
  completed: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  failed:    'bg-red-500/10 text-red-400 border-red-500/20',
  archived:  'bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700',
};
const SESSION_DOT: Record<SessionState, string> = {
  open:      'bg-blue-400 animate-pulse',
  completed: 'bg-emerald-400',
  failed:    'bg-red-400',
  archived:  'bg-zinc-600',
};

function SessionPill({ state }: { state: SessionState }) {
  return (
    <span className={clsx(
      'inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap',
      SESSION_PILL[state],
    )}>
      <span className={clsx('w-1 h-1 rounded-full shrink-0', SESSION_DOT[state])} />
      {state}
    </span>
  );
}

// ── Session row ───────────────────────────────────────────────────────────────


// ── Main page ─────────────────────────────────────────────────────────────────

export function SessionsPage() {
  const { data: sessions, isError, error, refetch, isFetching } = useQuery({
    queryKey: ['sessions'],
    queryFn:  () => defaultApi.getSessions({ limit: 100 }),
    refetchInterval: 30_000,
  });
  const { data: allRuns } = useQuery({
    queryKey: ['runs'],
    queryFn:  () => defaultApi.getRuns(),
    staleTime: 30_000,
  });

  const toast      = useToast();
  const qc         = useQueryClient();
  const importRef  = useRef<HTMLInputElement>(null);

  const createSession = useMutation({
    mutationFn: () => {
      const id = `sess_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`;
      return defaultApi.createSession({ session_id: id });
    },
    onSuccess: (s) => {
      toast.success(`Session ${s.session_id} created`);
      qc.invalidateQueries({ queryKey: ['sessions'] });
    },
    onError: () => toast.error('Failed to create session'),
  });

  const list = sessions ?? [];
  const runsBySession = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of allRuns ?? []) {
      m.set(r.session_id, (m.get(r.session_id) ?? 0) + 1);
    }
    return m;
  }, [allRuns]);
  const runCountFor = (id: string) => runsBySession.get(id) ?? 0;
  const activeNow   = list.filter(s => s.state === 'open').length;

  /** Parse a JSON file chosen by the user and POST it to /v1/sessions/import. */
  function handleImportFile(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = async () => {
      try {
        const data = JSON.parse(reader.result as string);
        await defaultApi.importSession(data);
        toast.success('Session imported successfully.');
        void qc.invalidateQueries({ queryKey: ['sessions'] });
      } catch (err) {
        toast.error(err instanceof Error ? err.message : 'Import failed — invalid JSON or incompatible format.');
      }
      // Reset input so the same file can be re-imported if needed.
      if (importRef.current) importRef.current.value = '';
    };
    reader.readAsText(file);
  }

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div className="min-w-0">
          <p className={`${sectionLabel} mb-0`}>Sessions</p>
          <EntityExplainer className="mt-1">{ENTITY_EXPLAINERS.sessionsList}</EntityExplainer>
        </div>
        <div className="flex items-center gap-2">
          <button onClick={() => refetch()} className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors">
            <RefreshCw size={11} className={clsx(isFetching && 'animate-spin')} /> Refresh
          </button>
          {/* Hidden file input for importing a session JSON */}
          <input
            ref={importRef}
            type="file"
            accept=".json,application/json"
            className="hidden"
            onChange={handleImportFile}
          />
          <button
            onClick={() => importRef.current?.click()}
            title="Import a previously exported session JSON file"
            className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors"
          >
            <Upload size={11} /> Import
          </button>
          <button
            data-testid="new-session-btn"
            onClick={() => createSession.mutate()}
            disabled={createSession.isPending}
            className="flex items-center gap-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 px-2.5 py-1.5 text-[11px] text-white font-medium transition-colors disabled:opacity-50"
          >
            <Plus size={11} /> {createSession.isPending ? 'Creating…' : 'New Session'}
          </button>
        </div>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-3 gap-3">
        <StatCard label="Total Sessions" value={list.length} />
        <StatCard label="Active Now"     value={activeNow}   variant={activeNow > 0 ? 'info' : 'default'} description={activeNow > 0 ? `${activeNow} open` : 'none open'} />
        <StatCard label="Completed"      value={list.filter(s => s.state === 'completed').length} variant="success" />
      </div>

      {/* Table */}
      {isError ? (
        <ErrorFallback error={error} resource="sessions" onRetry={() => void refetch()} compact />
      ) : (
        <DataTable<SessionRecord>
          data={list}
          onRowClick={r => { window.location.hash = `session/${r.session_id}`; }}
          columns={[
            { key: 'arrow',      header: '',           render: _r => <ChevronRight size={13} className="text-gray-300 dark:text-zinc-600" /> },
            { key: 'session_id', header: 'Session ID', render: r => <span className="font-mono text-xs text-gray-800 dark:text-zinc-200 whitespace-nowrap" title={r.session_id}>{mono(r.session_id, 22)}</span>, sortValue: r => r.session_id },
            { key: 'tenant',     header: 'Tenant',     render: r => <span className="font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap hidden md:block">{r.project.tenant_id}</span> },
            { key: 'workspace',  header: 'Workspace',  render: r => <span className="font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap hidden lg:block">{r.project.workspace_id}</span> },
            { key: 'project',    header: 'Project',    render: r => <span className="font-mono text-[11px] text-gray-500 dark:text-zinc-400 whitespace-nowrap">{mono(r.project.project_id, 16)}</span> },
            { key: 'state',      header: 'Status',     render: r => <SessionPill state={r.state} />,                      sortValue: r => r.state },
            { key: 'runs',       header: 'Runs',       render: r => { const n = runCountFor(r.session_id); return n > 0 ? <span className="inline-flex items-center justify-center w-5 h-5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[10px] font-medium tabular-nums">{n}</span> : <span className="text-gray-300 dark:text-zinc-600 text-[11px]">—</span>; } },
            { key: 'created',    header: 'Created',    render: r => <span className="font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap" title={fmtTs(r.created_at)}>{fmtRelative(r.created_at)}</span>, sortValue: r => r.created_at },
          ]}
          filterFn={(r, q) => r.session_id.includes(q) || r.project.project_id.includes(q) || r.project.tenant_id.includes(q) || r.state.includes(q)}
          csvRow={r => [r.session_id, r.project.tenant_id, r.project.workspace_id, r.project.project_id, r.state, r.created_at]}
          csvHeaders={['Session ID', 'Tenant', 'Workspace', 'Project', 'State', 'Created At']}
          filename="sessions"
          emptyText="No sessions yet — click New Session above to create one."
        />
      )}

      <EmptyScopeHint empty={list.length === 0} />
    </div>
  );
}

export default SessionsPage;
