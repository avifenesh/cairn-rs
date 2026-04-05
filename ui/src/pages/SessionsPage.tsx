import { useQuery } from '@tanstack/react-query';
import { ChevronRight, Loader2, RefreshCw, Plus } from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import type { SessionRecord, SessionState } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTs(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
  });
}

function mono(s: string, max = 18): string {
  return s.length > max ? `${s.slice(0, max - 3)}…` : s;
}

// ── Session state pill — compact version ──────────────────────────────────────

const SESSION_PILL: Record<SessionState, string> = {
  open:      'bg-blue-500/10 text-blue-400 border-blue-500/20',
  completed: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  failed:    'bg-red-500/10 text-red-400 border-red-500/20',
  archived:  'bg-zinc-800 text-zinc-500 border-zinc-700',
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

// ── Stat card (left-border accent, no icon) ───────────────────────────────────

function StatCard({ label, value, sub, accent = 'default' }: {
  label: string; value: string | number; sub?: string;
  accent?: 'default' | 'blue' | 'emerald';
}) {
  const border = { default: 'border-l-zinc-700', blue: 'border-l-blue-500', emerald: 'border-l-emerald-500' }[accent];
  const val    = { default: 'text-zinc-100', blue: 'text-blue-400', emerald: 'text-emerald-400' }[accent];
  return (
    <div className={clsx('bg-zinc-900 border border-zinc-800 border-l-2 rounded-lg p-4', border)}>
      <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2">{label}</p>
      <p className={clsx('text-2xl font-semibold tabular-nums', val)}>{value}</p>
      {sub && <p className="mt-1 text-[11px] text-zinc-600">{sub}</p>}
    </div>
  );
}

// ── Session row ───────────────────────────────────────────────────────────────

function SessionRow({ session, runCount, even }: {
  session: SessionRecord; runCount: number; even: boolean;
}) {
  return (
    <tr
      onClick={() => { window.location.hash = `session/${session.session_id}`; }}
      className={clsx(
        'cursor-pointer border-b border-zinc-800/50 select-none transition-colors',
        even ? 'bg-zinc-900/50 hover:bg-white/5' : 'bg-zinc-900 hover:bg-white/5',
      )}
    >
      <td className="pl-3 pr-1 w-7">
        <ChevronRight size={13} className="text-zinc-700 group-hover:text-zinc-500" />
      </td>
      <td className="px-3 h-9 font-mono text-xs text-zinc-200 whitespace-nowrap">
        {mono(session.session_id, 22)}
      </td>
      <td className="px-3 h-9 font-mono text-[11px] text-zinc-500 whitespace-nowrap hidden md:table-cell">
        {session.project.tenant_id}
      </td>
      <td className="px-3 h-9 font-mono text-[11px] text-zinc-500 whitespace-nowrap hidden lg:table-cell">
        {session.project.workspace_id}
      </td>
      <td className="px-3 h-9 font-mono text-[11px] text-zinc-400 whitespace-nowrap">
        {mono(session.project.project_id, 16)}
      </td>
      <td className="px-3 h-9">
        <SessionPill state={session.state} />
      </td>
      <td className="px-3 h-9 text-center">
        {runCount > 0
          ? <span className="inline-flex items-center justify-center w-5 h-5 rounded bg-zinc-800 text-zinc-400 text-[10px] font-medium tabular-nums">{runCount}</span>
          : <span className="text-zinc-700 text-[11px]">—</span>}
      </td>
      <td className="px-3 h-9 text-[11px] text-zinc-500 whitespace-nowrap font-mono">
        {fmtTs(session.created_at)}
      </td>
    </tr>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function SessionsPage() {
  const { data: sessions, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ['sessions'],
    queryFn:  () => defaultApi.getSessions({ limit: 100 }),
    refetchInterval: 30_000,
  });
  const { data: allRuns } = useQuery({
    queryKey: ['runs'],
    queryFn:  () => defaultApi.getRuns(),
    staleTime: 30_000,
  });

  const list = sessions ?? [];
  const runCountFor = (id: string) => (allRuns ?? []).filter(r => r.session_id === id).length;
  const activeNow   = list.filter(s => s.state === 'open').length;

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Sessions</p>
        <div className="flex items-center gap-2">
          <button onClick={() => refetch()} className="flex items-center gap-1.5 rounded-md bg-zinc-900 border border-zinc-800 px-2.5 py-1.5 text-[11px] text-zinc-500 hover:bg-white/5 transition-colors">
            <RefreshCw size={11} className={clsx(isFetching && 'animate-spin')} /> Refresh
          </button>
          <button className="flex items-center gap-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 px-2.5 py-1.5 text-[11px] text-white font-medium transition-colors">
            <Plus size={11} /> New Session
          </button>
        </div>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-3 gap-3">
        <StatCard label="Total Sessions" value={list.length} accent="default" />
        <StatCard label="Active Now"     value={activeNow}   accent={activeNow > 0 ? 'blue' : 'default'} sub={activeNow > 0 ? `${activeNow} open` : 'none open'} />
        <StatCard label="Completed"      value={list.filter(s => s.state === 'completed').length} accent="emerald" />
      </div>

      {/* Table */}
      {isError ? (
        <div className="rounded-lg border border-zinc-800 p-6 text-center">
          <p className="text-sm text-zinc-400">{error instanceof Error ? error.message : 'Failed to load sessions'}</p>
        </div>
      ) : (
        <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
          {/* Column headers */}
          <div className="border-b border-zinc-800">
            <table className="w-full">
              <thead>
                <tr className="bg-zinc-950">
                  <th className="pl-3 pr-1 w-7" />
                  <th className="px-3 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Session ID</th>
                  <th className="px-3 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider hidden md:table-cell">Tenant</th>
                  <th className="px-3 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider hidden lg:table-cell">Workspace</th>
                  <th className="px-3 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Project</th>
                  <th className="px-3 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Status</th>
                  <th className="px-3 h-8 text-center text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Runs</th>
                  <th className="px-3 h-8 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider">Created</th>
                </tr>
              </thead>
            </table>
          </div>

          {/* Body */}
          {isLoading ? (
            <div className="flex items-center gap-2 px-4 h-12 text-zinc-600 text-xs">
              <Loader2 size={12} className="animate-spin" /> Loading sessions…
            </div>
          ) : list.length === 0 ? (
            <div className="px-4 py-10 text-center text-xs text-zinc-600">
              No sessions yet — POST to /v1/sessions to create one
            </div>
          ) : (
            <table className="w-full">
              <tbody>
                {list.map((session, i) => (
                  <SessionRow
                    key={session.session_id}
                    session={session}
                    runCount={runCountFor(session.session_id)}
                    even={i % 2 === 0}
                  />
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </div>
  );
}

export default SessionsPage;
