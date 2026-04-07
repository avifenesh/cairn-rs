/**
 * useEventStream — live SSE connection to /v1/streams/runtime
 *
 * Features:
 *  - Connects to the SSE endpoint with a bearer token
 *  - Parses structured { event_id, type, payload } frames
 *  - Auto-reconnects with exponential back-off (200ms → 30s, jitter)
 *  - Sends Last-Event-ID on reconnect for gapless replay
 *  - Exposes { events, status, lastEventId } via useSyncExternalStore
 */

import { useSyncExternalStore, useEffect } from 'react';

// ── Types ─────────────────────────────────────────────────────────────────────

export type StreamStatus = 'connecting' | 'connected' | 'disconnected';

export interface StreamEvent {
  /** SSE event id (position number from the server). */
  id: string;
  /** Event type string, e.g. "task_created", "run_state_changed". */
  type: string;
  /** Raw parsed payload from the server. */
  payload: unknown;
  /** Client-side receipt timestamp. */
  receivedAt: number;
}

interface StreamState {
  events: StreamEvent[];
  status: StreamStatus;
  lastEventId: string | null;
}

// ── Store ─────────────────────────────────────────────────────────────────────

const MAX_EVENTS = 50;
const BASE_DELAY_MS = 200;
const MAX_DELAY_MS = 30_000;

/** Shared singleton state — one SSE connection per page load. */
let state: StreamState = {
  events: [],
  status: 'disconnected',
  lastEventId: null,
};

type Listener = () => void;
const listeners = new Set<Listener>();

function setState(patch: Partial<StreamState>) {
  state = { ...state, ...patch };
  listeners.forEach((fn) => fn());
}

function subscribe(listener: Listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): StreamState {
  return state;
}

// ── Connection manager ────────────────────────────────────────────────────────

let es: EventSource | null = null;
let retryCount = 0;
let retryTimer: ReturnType<typeof setTimeout> | null = null;
let config: { url: string; token: string } | null = null;

function jitter(ms: number): number {
  return ms + Math.random() * ms * 0.3;
}

function retryDelay(): number {
  const delay = Math.min(BASE_DELAY_MS * 2 ** retryCount, MAX_DELAY_MS);
  return jitter(delay);
}

function connect() {
  if (!config) return;

  // Cancel any pending retry timer.
  if (retryTimer !== null) {
    clearTimeout(retryTimer);
    retryTimer = null;
  }

  // Close any stale connection.
  if (es) {
    es.close();
    es = null;
  }

  setState({ status: 'connecting' });

  // Build URL — append last-event-id as a query param because the browser
  // EventSource API doesn't let us set arbitrary headers. The server reads
  // it from the query string as well as the standard header.
  const url = new URL(config.url, window.location.origin);
  url.searchParams.set('token', config.token);
  if (state.lastEventId !== null) {
    url.searchParams.set('last_event_id', state.lastEventId);
  }

  // Use EventSource for automatic header-level reconnection, but we layer
  // our own retry to add back-off and Last-Event-ID replay.
  es = new EventSource(url.toString());

  es.addEventListener('open', () => {
    retryCount = 0;
    setState({ status: 'connected' });
  });

  // Generic message handler (catches all event types).
  es.addEventListener('message', handleMessage);

  // Also listen for named event types the server emits.
  const NAMED_TYPES = [
    'connected',
    'task_created', 'task_state_changed', 'task_update',
    'run_created', 'run_state_changed', 'agent_progress',
    'approval_required', 'approval_resolved',
    'session_created', 'session_state_changed',
    'tool_invocation_started', 'tool_invocation_completed', 'assistant_tool_call',
    'checkpoint_recorded', 'checkpoint_restored',
    'provider_call_completed', 'provider_health_checked',
    // Orchestration lifecycle events
    'orchestrate_started', 'gather_completed', 'decide_completed',
    'tool_called', 'tool_result', 'step_completed', 'orchestrate_finished',
    'operator_notification',
  ];
  for (const type of NAMED_TYPES) {
    es.addEventListener(type, handleMessage);
  }

  es.addEventListener('error', () => {
    es?.close();
    es = null;
    setState({ status: 'disconnected' });

    const delay = retryDelay();
    retryCount += 1;
    retryTimer = setTimeout(connect, delay);
  });
}

function handleMessage(raw: MessageEvent) {
  // Track last-event-id for replay on reconnect.
  if (raw.lastEventId) {
    state = { ...state, lastEventId: raw.lastEventId };
  }

  let parsed: StreamEvent;
  try {
    const data = JSON.parse(raw.data as string);
    parsed = {
      id:         raw.lastEventId || String(Date.now()),
      type:       raw.type === 'message' ? (data.type ?? 'unknown') : raw.type,
      payload:    data.payload ?? data,
      receivedAt: Date.now(),
    };
  } catch {
    parsed = {
      id:         raw.lastEventId || String(Date.now()),
      type:       raw.type,
      payload:    raw.data,
      receivedAt: Date.now(),
    };
  }

  // Skip the synthetic "connected" bookkeeping event from the log display.
  const next = parsed.type === 'connected'
    ? state.events
    : [parsed, ...state.events].slice(0, MAX_EVENTS);

  setState({ events: next });
}

function disconnect() {
  if (retryTimer !== null) {
    clearTimeout(retryTimer);
    retryTimer = null;
  }
  es?.close();
  es = null;
  setState({ status: 'disconnected' });
}

// ── Public API ─────────────────────────────────────────────────────────────────

export interface UseEventStreamOptions {
  /** Full URL of the SSE endpoint (default: /v1/streams/runtime). */
  url?: string;
  /** Bearer token passed as ?token= query param. */
  token?: string;
  /** Set to false to pause the connection. */
  enabled?: boolean;
}

export interface UseEventStreamResult {
  events: StreamEvent[];
  status: StreamStatus;
  lastEventId: string | null;
}

export function useEventStream({
  url = '/v1/streams/runtime',
  token = import.meta.env.VITE_API_TOKEN ?? 'dev-admin-token',
  enabled = true,
}: UseEventStreamOptions = {}): UseEventStreamResult {
  // Wire up or tear down the connection when deps change.
  useEffect(() => {
    if (!enabled) {
      disconnect();
      return;
    }

    const needsReconnect =
      config?.url !== url ||
      config?.token !== token ||
      state.status === 'disconnected';

    config = { url, token };

    if (needsReconnect) {
      connect();
    }

    return () => {
      // Don't tear down the shared singleton on component unmount —
      // other components may still be subscribed. Only disconnect when
      // `enabled` flips to false.
    };
  }, [url, token, enabled]);

  const snap = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  return {
    events:      snap.events,
    status:      snap.status,
    lastEventId: snap.lastEventId,
  };
}
