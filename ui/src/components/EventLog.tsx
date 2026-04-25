/**
 * EventLog — live tail of the last N runtime SSE events.
 *
 * Renders oldest→newest (auto-scrolls to bottom on new arrivals).
 * Accepts optional initialEvents from getRecentEvents so the log
 * is populated immediately — before the SSE connection opens.
 */

import { useRef, useEffect, useMemo } from 'react';
import { clsx } from 'clsx';
import { WifiOff, Loader2, Inbox, Activity } from 'lucide-react';
import { useEventStream, type StreamStatus, type StreamEvent } from '../hooks/useEventStream';
import type { RecentEvent } from '../lib/types';

// ── Status dot ─────────────────────────────────────────────────────────────────

function StatusDot({ status }: { status: StreamStatus }) {
  return (
    <span className="flex items-center gap-1.5 text-[11px] font-medium">
      <span className={clsx(
        'w-1.5 h-1.5 rounded-full shrink-0',
        status === 'connected'    && 'bg-emerald-400',
        status === 'connecting'   && 'bg-amber-400 animate-pulse',
        status === 'disconnected' && 'bg-red-500',
      )} />
      <span className={clsx(
        status === 'connected'    && 'text-emerald-400',
        status === 'connecting'   && 'text-amber-400',
        status === 'disconnected' && 'text-red-400',
      )}>
        {status === 'connected' ? 'Live' : status === 'connecting' ? 'Connecting…' : 'Disconnected'}
      </span>
    </span>
  );
}

// ── Event type badge colour ────────────────────────────────────────────────────

function typeBadgeClass(type: string): string {
  if (type.includes('run'))        return 'bg-blue-950  text-blue-300  ring-blue-800';
  if (type.includes('task'))       return 'bg-indigo-950 text-indigo-300 ring-indigo-800';
  if (type.includes('approval'))   return 'bg-violet-950 text-violet-300 ring-violet-800';
  if (type.includes('session'))    return 'bg-sky-950    text-sky-300   ring-sky-800';
  if (type.includes('checkpoint')) return 'bg-amber-950  text-amber-300 ring-amber-800';
  if (type.includes('provider') || type.includes('tool'))
                                   return 'bg-teal-950   text-teal-300  ring-teal-800';
  if (type.includes('eval') || type.includes('score'))
                                   return 'bg-pink-950   text-pink-300  ring-pink-800';
  return 'bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 ring-gray-300 dark:ring-zinc-700';
}

// ── Unified event row ──────────────────────────────────────────────────────────

interface NormalizedEvent {
  key: string;
  time: string;
  type: string;
  detail: string;
}

function toDetail(payload: unknown): string {
  if (!payload) return '';
  try {
    const s = typeof payload === 'string' ? payload : JSON.stringify(payload);
    return s.length > 90 ? `${s.slice(0, 90)}…` : s;
  } catch {
    return String(payload);
  }
}

function EventRow({ ev }: { ev: NormalizedEvent }) {
  return (
    <div data-event-row className="flex items-center gap-2.5 px-3 py-1.5 hover:bg-gray-100/40 dark:hover:bg-gray-100/40 dark:bg-zinc-800/40 transition-colors group">
      {/* Time */}
      <span className="shrink-0 text-[11px] text-gray-400 dark:text-zinc-600 font-mono tabular-nums w-[52px]">
        {ev.time}
      </span>

      {/* Type badge */}
      <span className={clsx(
        'shrink-0 rounded px-1.5 py-0.5 text-[10px] font-mono font-medium ring-1 whitespace-nowrap',
        typeBadgeClass(ev.type),
      )}>
        {ev.type.replace(/_/g, '\u202F')}
      </span>

      {/* Detail */}
      <span className="flex-1 min-w-0 text-[11px] text-gray-400 dark:text-zinc-500 font-mono truncate">
        {ev.detail}
      </span>
    </div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────────

interface EventLogProps {
  /** Pre-loaded events from getRecentEvents (shown before SSE connects). */
  initialEvents?: RecentEvent[];
  /** Override the SSE URL. */
  url?: string;
  /** Override the bearer token. */
  token?: string;
  /** Maximum rows to display (default 50). */
  maxEvents?: number;
  className?: string;
}

export function EventLog({
  initialEvents = [],
  url,
  token,
  maxEvents = 50,
  className,
}: EventLogProps) {
  const { events: liveEvents, status } = useEventStream({ url, token });
  const scrollRef = useRef<HTMLDivElement>(null);
  const prevRowCountRef = useRef(0);

  // Convert initial REST events → display format
  const seedRows: NormalizedEvent[] = useMemo(() =>
    initialEvents
      .slice(-maxEvents)
      .map((e, i) => {
        const ts = e.stored_at ?? (e.timestamp ? Date.parse(e.timestamp) : 0);
        return {
          key:    `seed-${i}`,
          time:   ts > 0
            ? new Date(ts).toLocaleTimeString(undefined, {
                hour: '2-digit', minute: '2-digit', second: '2-digit',
              })
            : '—',
          type:   e.event_type ?? 'unknown',
          detail: toDetail(e.message ?? e.data),
        };
      }),
    [initialEvents, maxEvents],
  );

  // Convert live SSE events → display format (newest first → reverse for oldest-first display)
  const liveRows: NormalizedEvent[] = useMemo(() => {
    const sorted = [...liveEvents].reverse(); // hook returns newest-first; we want oldest-first
    return sorted.slice(-maxEvents).map((e: StreamEvent) => ({
      key:    `live-${e.id}`,
      time:   new Date(e.receivedAt).toLocaleTimeString(undefined, {
        hour: '2-digit', minute: '2-digit', second: '2-digit',
      }),
      type:   e.type ?? 'unknown',
      detail: toDetail(e.payload),
    }));
  }, [liveEvents, maxEvents]);

  // Merge: when live events arrive, they supersede the seed.
  const rows = liveRows.length > 0 ? liveRows : seedRows;

  // Auto-scroll the inner log container to its bottom when new rows
  // arrive. Mutates `scrollTop` directly rather than using
  // `scrollIntoView`, which walks ancestors and would scroll the whole
  // page to bring the log into view — that caused the dashboard to
  // jump to the bottom on mount (F33).
  //
  // Skip the first effect fire so the initial seed render doesn't
  // trigger a scroll at all; only growth from live events does.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (prevRowCountRef.current === 0) {
      prevRowCountRef.current = rows.length;
      return;
    }
    if (rows.length > prevRowCountRef.current) {
      el.scrollTop = el.scrollHeight;
    }
    prevRowCountRef.current = rows.length;
  }, [rows.length]);

  return (
    <div className={clsx(
      'flex flex-col rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden bg-gray-50 dark:bg-zinc-900',
      className,
    )}>
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <span className="text-[11px] font-semibold text-gray-500 dark:text-zinc-400 flex items-center gap-1.5 uppercase tracking-wider">
          <Activity size={11} className="text-gray-400 dark:text-zinc-600" />
          Event Stream
          {rows.length > 0 && (
            <span className="text-gray-400 dark:text-zinc-600 font-normal ml-0.5">({rows.length})</span>
          )}
        </span>
        <StatusDot status={status} />
      </div>

      {/* Event list — compact Linear-style rows */}
      <div
        ref={scrollRef}
        className="overflow-y-auto"
        style={{ maxHeight: '280px' }}
        aria-live="polite"
        aria-atomic="false"
        aria-label="Live event stream"
        aria-relevant="additions"
        role="log"
      >
        {rows.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 gap-1.5 text-gray-300 dark:text-zinc-600">
            {status === 'connected' ? (
              <><Inbox size={20} /><p className="text-[12px]">Waiting for events…</p></>
            ) : status === 'connecting' ? (
              <><Loader2 size={20} className="animate-spin" /><p className="text-[12px]">Connecting…</p></>
            ) : (
              <><WifiOff size={20} /><p className="text-[12px]">Not connected</p></>
            )}
          </div>
        ) : (
          <div className="divide-y divide-gray-200 dark:divide-zinc-800/40">
            {rows.map(ev => <EventRow key={ev.key} ev={ev} />)}
          </div>
        )}
      </div>
    </div>
  );
}
