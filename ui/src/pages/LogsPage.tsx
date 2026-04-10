/**
 * LogsPage — live structured request log viewer.
 *
 * Polls GET /v1/admin/logs every 2 s. Each row is a structured log entry
 * written by the request-tracing middleware in cairn-app, colour-coded by level.
 */

import { useState, useRef, useEffect, useCallback } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, ServerCrash, Search, X, Pause, Play as PlayIcon,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import type { RequestLogEntry } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTimestamp(iso: string): string {
  try {
    const d = new Date(iso);
    return d.toLocaleTimeString(undefined, {
      hour12: false,
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    }) + '.' + String(d.getMilliseconds()).padStart(3, '0');
  } catch {
    return iso;
  }
}

function levelColors(level: 'info' | 'warn' | 'error'): {
  row: string; badge: string; dot: string;
} {
  switch (level) {
    case 'error': return {
      row:   'bg-red-950/20 hover:bg-red-950/30',
      badge: 'text-red-400 bg-red-400/10 border-red-400/20',
      dot:   'bg-red-400',
    };
    case 'warn': return {
      row:   'bg-amber-950/20 hover:bg-amber-950/30',
      badge: 'text-amber-400 bg-amber-400/10 border-amber-400/20',
      dot:   'bg-amber-400',
    };
    default: return {
      row:   'hover:bg-white/[0.02]',
      badge: 'text-sky-400 bg-sky-400/10 border-sky-400/20',
      dot:   'bg-sky-500',
    };
  }
}

function statusColor(status: number): string {
  if (status >= 500) return 'text-red-400';
  if (status >= 400) return 'text-amber-400';
  if (status >= 300) return 'text-sky-400';
  return 'text-emerald-400';
}

// ── Level toggle ──────────────────────────────────────────────────────────────

type Level = 'info' | 'warn' | 'error';

function LevelToggle({
  level, active, count, onClick,
}: {
  level: Level; active: boolean; count: number; onClick: () => void;
}) {
  const colors = levelColors(level);
  return (
    <button
      onClick={onClick}
      className={clsx(
        'flex items-center gap-1.5 px-2 py-1 rounded border text-[11px] font-medium transition-colors',
        active
          ? colors.badge
          : 'text-gray-400 dark:text-zinc-600 bg-gray-50 dark:bg-zinc-900 border-gray-200 dark:border-zinc-800 hover:border-gray-200 dark:border-zinc-700 hover:text-gray-500 dark:hover:text-zinc-400',
      )}
    >
      <span className={clsx('h-1.5 w-1.5 rounded-full', active ? colors.dot : 'bg-zinc-700')} />
      {level}
      <span className={clsx('tabular-nums', active ? '' : 'text-gray-300 dark:text-zinc-700')}>
        {count > 0 ? count : ''}
      </span>
    </button>
  );
}

// ── Log row ───────────────────────────────────────────────────────────────────

function LogRow({ entry, even }: { entry: RequestLogEntry; even: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const colors = levelColors(entry.level);

  return (
    <div
      className={clsx(
        'border-b border-gray-200/40 dark:border-zinc-800/40 last:border-0 transition-colors cursor-pointer',
        even ? '' : 'bg-gray-50/30 dark:bg-zinc-900/30',
        colors.row,
      )}
      onClick={() => setExpanded(v => !v)}
    >
      {/* Compact row */}
      <div className="flex items-center gap-0 h-7 px-1">
        {/* Timestamp */}
        <span className="w-28 shrink-0 text-[10px] font-mono text-gray-400 dark:text-zinc-600 tabular-nums pl-2">
          {fmtTimestamp(entry.timestamp)}
        </span>

        {/* Level dot */}
        <span className={clsx('w-1.5 h-1.5 rounded-full shrink-0 mx-1.5', colors.dot)} />

        {/* Method */}
        <span className="w-10 shrink-0 text-[10px] font-mono text-gray-400 dark:text-zinc-500 uppercase">
          {entry.method}
        </span>

        {/* Status */}
        <span className={clsx('w-10 shrink-0 text-[11px] font-mono tabular-nums font-medium', statusColor(entry.status))}>
          {entry.status}
        </span>

        {/* Path */}
        <span className="flex-1 min-w-0 text-[11px] font-mono text-gray-700 dark:text-zinc-300 truncate">
          {entry.path}
          {entry.query && <span className="text-gray-400 dark:text-zinc-600">?{entry.query}</span>}
        </span>

        {/* Latency */}
        <span className={clsx(
          'w-16 shrink-0 text-right text-[10px] font-mono tabular-nums pr-2',
          entry.latency_ms > 1000 ? 'text-amber-400' :
          entry.latency_ms > 300  ? 'text-yellow-600' : 'text-gray-400 dark:text-zinc-600',
        )}>
          {entry.latency_ms}ms
        </span>
      </div>

      {/* Expanded detail */}
      {expanded && (
        <div className="px-3 pb-2 pt-1 border-t border-gray-200/40 dark:border-zinc-800/40 bg-white dark:bg-zinc-950/40">
          <div className="grid grid-cols-2 gap-x-6 gap-y-1 text-[11px] font-mono">
            <div>
              <span className="text-gray-400 dark:text-zinc-600">request_id </span>
              <span className="text-gray-500 dark:text-zinc-400">{entry.request_id}</span>
            </div>
            <div>
              <span className="text-gray-400 dark:text-zinc-600">latency    </span>
              <span className="text-gray-500 dark:text-zinc-400">{entry.latency_ms}ms</span>
            </div>
            <div>
              <span className="text-gray-400 dark:text-zinc-600">method     </span>
              <span className="text-gray-500 dark:text-zinc-400">{entry.method}</span>
            </div>
            <div>
              <span className="text-gray-400 dark:text-zinc-600">status     </span>
              <span className={statusColor(entry.status)}>{entry.status}</span>
            </div>
            <div className="col-span-2">
              <span className="text-gray-400 dark:text-zinc-600">path       </span>
              <span className="text-gray-500 dark:text-zinc-400">{entry.path}</span>
              {entry.query && <span className="text-gray-400 dark:text-zinc-600">?{entry.query}</span>}
            </div>
            <div className="col-span-2">
              <span className="text-gray-400 dark:text-zinc-600">timestamp  </span>
              <span className="text-gray-500 dark:text-zinc-400">{entry.timestamp}</span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function LogsPage() {
  const [search,     setSearch]     = useState('');
  const [paused,     setPaused]     = useState(false);
  const [activeLevels, setActiveLevels] = useState<Set<Level>>(
    new Set(['info', 'warn', 'error'] satisfies Level[]),
  );
  const listRef  = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

  // Build the level query param from active levels.
  const levelParam = (['info', 'warn', 'error'] as Level[])
    .filter(l => activeLevels.has(l))
    .join(',');

  const { data, isLoading, isError, error, isFetching } = useQuery({
    queryKey: ['request-logs', levelParam],
    queryFn:  () => defaultApi.getRequestLogs({ limit: 500, level: levelParam || undefined }),
    refetchInterval: paused ? false : 2_000,
    staleTime: 0,
  });

  const entries: RequestLogEntry[] = data?.entries ?? [];

  // Apply client-side search filter.
  const filtered = search.trim()
    ? entries.filter(e =>
        e.path.toLowerCase().includes(search.toLowerCase()) ||
        e.message.toLowerCase().includes(search.toLowerCase()) ||
        e.request_id.toLowerCase().includes(search.toLowerCase()) ||
        String(e.status).includes(search)
      )
    : entries;

  // Count by level for the toggles.
  const counts = {
    info:  entries.filter(e => e.level === 'info').length,
    warn:  entries.filter(e => e.level === 'warn').length,
    error: entries.filter(e => e.level === 'error').length,
  };

  // Auto-scroll to bottom when new entries arrive (unless user scrolled up).
  useEffect(() => {
    if (!paused && atBottom.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [filtered.length, paused]);

  const handleScroll = useCallback(() => {
    const el = listRef.current;
    if (!el) return;
    atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 32;
  }, []);

  function toggleLevel(level: Level) {
    setActiveLevels(prev => {
      const next = new Set(prev);
      if (next.has(level)) {
        if (next.size > 1) next.delete(level); // always keep at least one
      } else {
        next.add(level);
      }
      return next;
    });
  }

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-gray-700 dark:text-zinc-300 font-medium">Failed to load logs</p>
      <p className="text-[12px] text-gray-400 dark:text-zinc-500">
        {error instanceof Error ? error.message : 'Unknown error'}
      </p>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200 mr-1">
          Logs
          {!isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-600 font-normal tabular-nums">
              {filtered.length}{filtered.length !== entries.length && `/${entries.length}`}
            </span>
          )}
        </span>

        {/* Level toggles */}
        {(['info', 'warn', 'error'] as Level[]).map(level => (
          <LevelToggle
            key={level}
            level={level}
            active={activeLevels.has(level)}
            count={counts[level]}
            onClick={() => toggleLevel(level)}
          />
        ))}

        {/* Search */}
        <div className="relative ml-2">
          <Search size={11} className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
          <input
            type="text"
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder="Filter…"
            className="h-6 w-40 bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded pl-6 pr-6 text-[11px]
                       text-gray-700 dark:text-zinc-300 placeholder-zinc-700 focus:outline-none
                       focus:border-indigo-500 transition-colors"
          />
          {search && (
            <button
              onClick={() => setSearch('')}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400"
            >
              <X size={10} />
            </button>
          )}
        </div>

        <div className="ml-auto flex items-center gap-2">
          {/* Pause / resume */}
          <button
            onClick={() => setPaused(v => !v)}
            aria-pressed={paused}
            aria-label={paused ? "Resume auto-refresh" : "Pause auto-refresh"}
            className={clsx(
              'flex items-center gap-1 px-2 py-1 rounded text-[11px] transition-colors',
              paused
                ? 'bg-amber-500/10 text-amber-400 border border-amber-500/20 hover:bg-amber-500/20'
                : 'text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300',
            )}
          >
            {paused ? <PlayIcon size={10} /> : <Pause size={10} />}
            {paused ? 'Resume' : 'Pause'}
          </button>

          {/* Refresh indicator */}
          <span className={clsx(
            'flex items-center gap-1 text-[11px] text-gray-300 dark:text-zinc-700 transition-opacity',
            isFetching ? 'opacity-100' : 'opacity-0',
          )}>
            <RefreshCw size={10} className="animate-spin" />
          </span>
        </div>
      </div>

      {/* Column headers */}
      <div className="flex items-center gap-0 h-6 px-1 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="w-28 shrink-0 pl-2 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider">Time</span>
        <span className="w-3 shrink-0" />
        <span className="w-10 shrink-0 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider">Method</span>
        <span className="w-10 shrink-0 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider">Status</span>
        <span className="flex-1 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider">Path</span>
        <span className="w-16 shrink-0 text-right pr-2 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider">Latency</span>
      </div>

      {/* Log list */}
      <div
        ref={listRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto font-mono"
      >
        {isLoading ? (
          <div className="flex items-center justify-center min-h-32 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={14} className="animate-spin" />
            <span className="text-[12px]">Loading…</span>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-48 gap-2 text-center">
            <p className="text-[12px] text-gray-400 dark:text-zinc-600">
              {search ? 'No entries match the filter.' : 'No log entries yet — make a request to see it here.'}
            </p>
            {search && (
              <button
                onClick={() => setSearch('')}
                className="text-[11px] text-indigo-500 hover:text-indigo-400 transition-colors"
              >
                Clear filter
              </button>
            )}
          </div>
        ) : (
          filtered.map((entry, i) => (
            <LogRow
              key={`${entry.request_id}-${i}`}
              entry={entry}
              even={i % 2 === 0}
            />
          ))
        )}
        {/* Sentinel for auto-scroll */}
        <div className="h-1" />
      </div>

      {/* Footer: live/paused status + ring buffer note */}
      <div className="flex items-center gap-3 px-3 h-6 border-t border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className={clsx(
          'flex items-center gap-1.5 text-[10px]',
          paused ? 'text-amber-500' : 'text-emerald-500',
        )}>
          <span className={clsx(
            'h-1.5 w-1.5 rounded-full',
            paused ? 'bg-amber-500' : 'bg-emerald-500 animate-pulse',
          )} />
          {paused ? 'Paused' : 'Live (2s)'}
        </span>
        <span className="text-[10px] text-gray-300 dark:text-zinc-700 ml-auto">
          Ring buffer: last 2,000 requests
        </span>
      </div>
    </div>
  );
}

export default LogsPage;
