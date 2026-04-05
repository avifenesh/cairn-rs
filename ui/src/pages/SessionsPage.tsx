import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  ChevronDown,
  ChevronRight,
  MonitorPlay,
  ServerCrash,
  Loader2,
  Inbox,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { StateBadge } from '../components/StateBadge';
import type { RunRecord, SessionRecord, SessionState } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatTs(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month:  'short',
    day:    'numeric',
    hour:   '2-digit',
    minute: '2-digit',
  });
}

function truncate(s: string, n = 14): string {
  return s.length > n ? `${s.slice(0, n)}…` : s;
}

// ── Session state badge (separate from RunState) ──────────────────────────────

const SESSION_BADGE: Record<SessionState, string> = {
  open:       'bg-blue-950   text-blue-300  ring-1 ring-blue-800',
  completed:  'bg-emerald-950 text-emerald-400 ring-1 ring-emerald-800',
  failed:     'bg-red-950    text-red-400   ring-1 ring-red-800',
  archived:   'bg-zinc-800   text-zinc-500  ring-1 ring-zinc-700',
};

function SessionStateBadge({ state }: { state: SessionState }) {
  return (
    <span className={clsx(
      'inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 text-[10px] font-medium whitespace-nowrap',
      SESSION_BADGE[state] ?? SESSION_BADGE.open,
    )}>
      <span className={clsx(
        'w-1.5 h-1.5 rounded-full',
        state === 'open' ? 'bg-blue-400 animate-pulse' :
        state === 'completed' ? 'bg-emerald-400' :
        state === 'failed' ? 'bg-red-400' : 'bg-zinc-500',
      )} />
      {state.charAt(0).toUpperCase() + state.slice(1)}
    </span>
  );
}

// ── Run sub-table (shown when a session row is expanded) ──────────────────────

function RunsSubTable({ sessionId }: { sessionId: string }) {
  const { data: allRuns, isLoading, isError } = useQuery({
    queryKey: ['runs'],
    queryFn: () => defaultApi.getRuns(),
    staleTime: 30_000,
  });

  const runs: RunRecord[] = (allRuns ?? []).filter(
    (r) => r.session_id === sessionId,
  );

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 px-6 py-2 text-xs text-zinc-500">
        <Loader2 size={12} className="animate-spin" />
        Loading runs…
      </div>
    );
  }

  if (isError) {
    return (
      <p className="px-6 py-2 text-xs text-red-400">Failed to load runs.</p>
    );
  }

  if (runs.length === 0) {
    return (
      <p className="px-6 py-2 text-xs text-zinc-600 italic">
        No runs for this session.
      </p>
    );
  }

  return (
    <table className="w-full text-xs">
      <thead>
        <tr className="text-zinc-500 border-b border-zinc-800">
          <th className="px-6 py-2 text-left font-medium">Run ID</th>
          <th className="px-4 py-2 text-left font-medium">State</th>
          <th className="px-4 py-2 text-left font-medium">Parent</th>
          <th className="px-4 py-2 text-left font-medium">Created</th>
        </tr>
      </thead>
      <tbody className="divide-y divide-zinc-800/50">
        {runs.map((run) => (
          <tr key={run.run_id} className="hover:bg-zinc-800/30 transition-colors">
            <td className="px-6 py-2 font-mono text-zinc-300">
              {truncate(run.run_id, 18)}
            </td>
            <td className="px-4 py-2">
              <StateBadge state={run.state} compact />
            </td>
            <td className="px-4 py-2 font-mono text-zinc-500">
              {run.parent_run_id ? truncate(run.parent_run_id, 12) : '—'}
            </td>
            <td className="px-4 py-2 text-zinc-500">
              {formatTs(run.created_at)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Session row ───────────────────────────────────────────────────────────────

interface SessionRowProps {
  session: SessionRecord;
  runCount: number;
  expanded: boolean;
  onToggle: () => void;
}

function SessionRow({ session, runCount, expanded, onToggle }: SessionRowProps) {
  return (
    <>
      {/* Main row */}
      <tr
        onClick={onToggle}
        className={clsx(
          'cursor-pointer border-b border-zinc-800 transition-colors select-none',
          expanded ? 'bg-white/5' : 'hover:bg-white/5',
        )}
      >
        {/* Expand toggle */}
        <td className="pl-4 pr-2 py-2 w-8">
          {expanded
            ? <ChevronDown  size={14} className="text-zinc-400" />
            : <ChevronRight size={14} className="text-zinc-500" />
          }
        </td>

        {/* Session ID */}
        <td className="px-3 py-2 font-mono text-sm text-zinc-200">
          {truncate(session.session_id, 20)}
        </td>

        {/* Project */}
        <td className="px-3 py-2 text-sm text-zinc-400 font-mono">
          <span title={`${session.project.tenant_id}/${session.project.workspace_id}/${session.project.project_id}`}>
            {truncate(session.project.project_id, 16)}
          </span>
        </td>

        {/* State */}
        <td className="px-3 py-3">
          <SessionStateBadge state={session.state} />
        </td>

        {/* Run count */}
        <td className="px-3 py-2 text-center">
          {runCount > 0 ? (
            <span className="inline-flex items-center justify-center w-5 h-5 rounded bg-zinc-800 text-zinc-400 text-[10px] font-medium">
              {runCount}
            </span>
          ) : (
            <span className="text-zinc-600 text-xs">—</span>
          )}
        </td>

        {/* Created At */}
        <td className="px-3 py-2 text-sm text-zinc-500">
          {formatTs(session.created_at)}
        </td>
      </tr>

      {/* Expanded runs sub-table */}
      {expanded && (
        <tr className="bg-zinc-900/70 border-b border-zinc-800">
          <td colSpan={6} className="py-1">
            <RunsSubTable sessionId={session.session_id} />
          </td>
        </tr>
      )}
    </>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function SessionsPage() {
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const {
    data: sessions,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ['sessions'],
    queryFn: () => defaultApi.getSessions({ limit: 100 }),
    refetchInterval: 30_000,
  });

  // Pre-fetch runs so run counts are available without extra requests.
  const { data: allRuns } = useQuery({
    queryKey: ['runs'],
    queryFn: () => defaultApi.getRuns(),
    staleTime: 30_000,
  });

  function runCountFor(sessionId: string): number {
    return (allRuns ?? []).filter((r) => r.session_id === sessionId).length;
  }

  function toggle(id: string) {
    setExpandedId((prev) => (prev === id ? null : id));
  }

  // ── Loading ──────────────────────────────────────────────────────────────
  if (isLoading) {
    return (
      <div className="flex items-center justify-center min-h-48 gap-2 text-zinc-500">
        <Loader2 size={18} className="animate-spin" />
        <span className="text-sm">Loading sessions…</span>
      </div>
    );
  }

  // ── Error ────────────────────────────────────────────────────────────────
  if (isError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-48 gap-3 text-center p-8">
        <ServerCrash size={36} className="text-red-500" />
        <p className="text-zinc-300 font-medium">Failed to load sessions</p>
        <p className="text-sm text-zinc-500">
          {error instanceof Error ? error.message : 'Unknown error'}
        </p>
      </div>
    );
  }

  // ── Empty ────────────────────────────────────────────────────────────────
  if (!sessions || sessions.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center min-h-48 gap-3 text-center p-8">
        <Inbox size={36} className="text-zinc-700" />
        <p className="text-zinc-400 font-medium">No sessions yet</p>
        <p className="text-sm text-zinc-600">
          POST /v1/sessions to create the first session.
        </p>
      </div>
    );
  }

  // ── Table ────────────────────────────────────────────────────────────────
  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-zinc-200 flex items-center gap-2">
          <MonitorPlay size={15} className="text-indigo-400" />
          Sessions
          <span className="ml-1 text-xs text-zinc-500 font-normal">
            ({sessions.length})
          </span>
        </h2>
      </div>

      {/* Table */}
      <div className="rounded-lg border border-zinc-800 overflow-hidden">
        <table className="w-full">
          <thead className="bg-zinc-900 border-b border-zinc-800">
            <tr className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
              <th className="pl-4 pr-2 py-2.5 w-8" />
              <th className="px-3 py-2.5 text-left">Session ID</th>
              <th className="px-3 py-2.5 text-left">Project</th>
              <th className="px-3 py-2.5 text-left">State</th>
              <th className="px-3 py-2.5 text-center">Runs</th>
              <th className="px-3 py-2.5 text-left">Created</th>
            </tr>
          </thead>
          <tbody className="bg-zinc-950 divide-y divide-zinc-800/40">
            {sessions.map((session) => (
              <SessionRow
                key={session.session_id}
                session={session}
                runCount={runCountFor(session.session_id)}
                expanded={expandedId === session.session_id}
                onToggle={() => toggle(session.session_id)}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export default SessionsPage;
