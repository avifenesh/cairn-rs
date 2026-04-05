/**
 * EventLog — live tail of the last 50 runtime SSE events.
 *
 * Connects via useEventStream and renders a scrollable, auto-updating
 * log with event type badges, payload preview, and connection status.
 */

import { useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import { Radio, WifiOff, Loader2, Inbox } from 'lucide-react';
import { useEventStream, type StreamStatus } from '../hooks/useEventStream';

// ── Status indicator ──────────────────────────────────────────────────────────

const STATUS_CFG: Record<StreamStatus, { label: string; dot: string; icon: React.ComponentType<{ size?: number; className?: string }> }> = {
  connecting:   { label: 'Connecting',   dot: 'bg-amber-400 animate-pulse', icon: Loader2 },
  connected:    { label: 'Live',         dot: 'bg-emerald-400',             icon: Radio   },
  disconnected: { label: 'Disconnected', dot: 'bg-red-500',                 icon: WifiOff },
};

function ConnectionBadge({ status }: { status: StreamStatus }) {
  const cfg = STATUS_CFG[status];
  const Icon = cfg.icon;
  return (
    <span className={clsx(
      'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium ring-1',
      status === 'connected'    && 'bg-emerald-950 text-emerald-400 ring-emerald-800',
      status === 'connecting'   && 'bg-amber-950   text-amber-400   ring-amber-800',
      status === 'disconnected' && 'bg-red-950     text-red-400     ring-red-800',
    )}>
      <span className={clsx('w-1.5 h-1.5 rounded-full shrink-0', cfg.dot)} />
      <Icon size={11} className={status === 'connecting' ? 'animate-spin' : undefined} />
      {cfg.label}
    </span>
  );
}

// ── Event type → badge colour ─────────────────────────────────────────────────

function eventBadgeClass(type: string): string {
  if (type.includes('run'))      return 'bg-blue-950 text-blue-300 ring-blue-800';
  if (type.includes('task'))     return 'bg-indigo-950 text-indigo-300 ring-indigo-800';
  if (type.includes('approval')) return 'bg-violet-950 text-violet-300 ring-violet-800';
  if (type.includes('session'))  return 'bg-sky-950 text-sky-300 ring-sky-800';
  if (type.includes('provider') || type.includes('tool'))
                                 return 'bg-teal-950 text-teal-300 ring-teal-800';
  if (type.includes('checkpoint')) return 'bg-amber-950 text-amber-300 ring-amber-800';
  return 'bg-zinc-800 text-zinc-400 ring-zinc-700';
}

function EventTypeBadge({ type }: { type: string }) {
  return (
    <span className={clsx(
      'inline-block shrink-0 rounded px-1.5 py-0.5 text-[10px] font-mono font-medium ring-1 whitespace-nowrap',
      eventBadgeClass(type),
    )}>
      {type}
    </span>
  );
}

// ── Payload preview ───────────────────────────────────────────────────────────

function PayloadPreview({ payload }: { payload: unknown }) {
  let preview = '';
  try {
    const text = typeof payload === 'string'
      ? payload
      : JSON.stringify(payload);
    preview = text.length > 120 ? `${text.slice(0, 120)}…` : text;
  } catch {
    preview = String(payload);
  }
  return (
    <span className="font-mono text-[11px] text-zinc-500 break-all leading-relaxed">
      {preview}
    </span>
  );
}

// ── Timestamp ─────────────────────────────────────────────────────────────────

function Timestamp({ ms }: { ms: number }) {
  const d = new Date(ms);
  const time = d.toLocaleTimeString(undefined, {
    hour:   '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
  return (
    <span className="shrink-0 text-[10px] text-zinc-600 font-mono tabular-nums">
      {time}
    </span>
  );
}

// ── Main component ─────────────────────────────────────────────────────────────

interface EventLogProps {
  /** Override the SSE URL. */
  url?: string;
  /** Override the bearer token. */
  token?: string;
  /** Maximum number of events to display (default: 50). */
  maxEvents?: number;
  /** CSS class applied to the outer container. */
  className?: string;
}

export function EventLog({
  url,
  token,
  maxEvents = 50,
  className,
}: EventLogProps) {
  const { events, status } = useEventStream({ url, token });

  // Keep the list scrolled to top (newest first) on mount.
  const listRef = useRef<HTMLDivElement>(null);

  // Scroll to top when new events arrive.
  useEffect(() => {
    if (listRef.current) {
      listRef.current.scrollTop = 0;
    }
  }, [events.length]);

  const displayEvents = events.slice(0, maxEvents);

  return (
    <div className={clsx(
      'flex flex-col rounded-xl ring-1 ring-zinc-800 overflow-hidden bg-zinc-950',
      className,
    )}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800 shrink-0">
        <h3 className="text-xs font-semibold text-zinc-300 flex items-center gap-2">
          <Radio size={13} className="text-zinc-500" />
          Event Stream
          {displayEvents.length > 0 && (
            <span className="text-zinc-600 font-normal">
              ({displayEvents.length}{displayEvents.length === maxEvents ? '+' : ''})
            </span>
          )}
        </h3>
        <ConnectionBadge status={status} />
      </div>

      {/* Event list */}
      <div
        ref={listRef}
        className="flex-1 overflow-y-auto min-h-0"
        style={{ maxHeight: '420px' }}
      >
        {displayEvents.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-12 gap-2 text-zinc-700">
            {status === 'connected' ? (
              <>
                <Inbox size={24} />
                <p className="text-xs">Waiting for events…</p>
              </>
            ) : (
              <>
                <WifiOff size={24} />
                <p className="text-xs">
                  {status === 'connecting' ? 'Connecting to stream…' : 'Not connected'}
                </p>
              </>
            )}
          </div>
        ) : (
          <ul className="divide-y divide-zinc-800/50">
            {displayEvents.map((ev, i) => (
              <li
                key={`${ev.id}-${i}`}
                className="flex items-start gap-3 px-4 py-2.5 hover:bg-zinc-900/50 transition-colors"
              >
                {/* Timestamp */}
                <Timestamp ms={ev.receivedAt} />

                {/* Type badge */}
                <EventTypeBadge type={ev.type} />

                {/* Payload preview */}
                <div className="flex-1 min-w-0">
                  <PayloadPreview payload={ev.payload} />
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
