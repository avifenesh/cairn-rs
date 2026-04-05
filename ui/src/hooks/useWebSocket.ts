/**
 * useWebSocket — alternative real-time transport to SSE.
 *
 * Connects to GET /v1/ws?token=<token>, auto-reconnects with exponential
 * backoff, supports bi-directional JSON messages, and buffers sends while
 * the socket is reconnecting.
 *
 * Protocol (server → client):
 *   { type: "connected",  head_position: number }
 *   { type: "event",      position: number, event_type: string, event_id: string, payload: unknown }
 *   { type: "pong" }
 *   { type: "warn",       message: string }
 *
 * Protocol (client → server):
 *   { type: "subscribe",  event_types: string[] }   — filter future events
 *   { type: "ping" }                                 — request a pong
 */

import { useState, useEffect, useRef, useCallback } from 'react';

// ── Types ─────────────────────────────────────────────────────────────────────

export type WsStatus = 'idle' | 'connecting' | 'connected' | 'reconnecting' | 'failed';

export interface WsMessage {
  type: string;
  [key: string]: unknown;
}

interface UseWebSocketOptions {
  /** Override the base URL (defaults to VITE_API_URL). */
  baseUrl?: string;
  /** Override the token (defaults to localStorage cairn_token). */
  token?: string;
  /** Event types to subscribe to; empty/undefined = all events. */
  eventTypes?: string[];
  /** Called for every incoming message including "connected" and "pong". */
  onMessage?: (msg: WsMessage) => void;
  /** Whether to establish the connection at all. */
  enabled?: boolean;
  /** Max reconnect attempts before giving up (default: 10). */
  maxRetries?: number;
}

// ── Constants ─────────────────────────────────────────────────────────────────

const BASE_DELAY_MS = 1_000;
const MAX_DELAY_MS  = 30_000;

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useWebSocket({
  baseUrl,
  token,
  eventTypes,
  onMessage,
  enabled = true,
  maxRetries = 10,
}: UseWebSocketOptions = {}) {
  const [status, setStatus] = useState<WsStatus>('idle');
  const [lastMessage, setLastMessage] = useState<WsMessage | null>(null);

  const wsRef          = useRef<WebSocket | null>(null);
  const retryCount     = useRef(0);
  const retryTimer     = useRef<ReturnType<typeof setTimeout> | null>(null);
  const sendQueue      = useRef<WsMessage[]>([]);
  const onMessageRef   = useRef(onMessage);
  const eventTypesRef  = useRef(eventTypes);

  // Keep callback/option refs fresh without re-running effects.
  onMessageRef.current  = onMessage;
  eventTypesRef.current = eventTypes;

  // ── URL builder ────────────────────────────────────────────────────────────

  const buildUrl = useCallback((): string => {
    const base = (baseUrl ?? import.meta.env.VITE_API_URL ?? 'http://localhost:3000')
      .replace(/^https?/, (m: string) => (m === 'https' ? 'wss' : 'ws'));
    const tok = token ?? localStorage.getItem('cairn_token') ?? import.meta.env.VITE_API_TOKEN ?? '';
    return `${base}/v1/ws?token=${encodeURIComponent(tok)}`;
  }, [baseUrl, token]);

  // ── Core connect ───────────────────────────────────────────────────────────

  const connect = useCallback(() => {
    if (!enabled) return;
    if (wsRef.current?.readyState === WebSocket.OPEN) return;

    setStatus(retryCount.current === 0 ? 'connecting' : 'reconnecting');

    const ws = new WebSocket(buildUrl());
    wsRef.current = ws;

    ws.onopen = () => {
      retryCount.current = 0;
      setStatus('connected');

      // Send subscription filter if requested.
      const types = eventTypesRef.current;
      if (types && types.length > 0) {
        ws.send(JSON.stringify({ type: 'subscribe', event_types: types }));
      }

      // Flush offline-buffered messages.
      const queued = sendQueue.current.splice(0);
      queued.forEach((m: WsMessage) => ws.send(JSON.stringify(m)));
    };

    ws.onmessage = (e) => {
      let msg: WsMessage;
      try {
        msg = JSON.parse(e.data as string) as WsMessage;
      } catch {
        return;
      }
      setLastMessage(msg);
      onMessageRef.current?.(msg);
    };

    ws.onclose = () => {
      wsRef.current = null;
      if (!enabled) {
        setStatus('idle');
        return;
      }
      if (retryCount.current >= maxRetries) {
        setStatus('failed');
        return;
      }
      setStatus('reconnecting');
      const delay = Math.min(BASE_DELAY_MS * 2 ** retryCount.current, MAX_DELAY_MS);
      retryCount.current += 1;
      retryTimer.current = setTimeout(connect, delay);
    };

    ws.onerror = () => {
      // onclose fires immediately after, so no need to set status here.
      ws.close();
    };
  }, [enabled, maxRetries, buildUrl]);

  // ── Public API ─────────────────────────────────────────────────────────────

  /** Send a message; queues it if the socket is not yet open. */
  const send = useCallback((msg: WsMessage) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    } else {
      sendQueue.current.push(msg);
    }
  }, []);

  /** Send a ping and expect a pong back. */
  const ping = useCallback(() => send({ type: 'ping' }), [send]);

  /** Update the event-type subscription filter on the fly. */
  const subscribe = useCallback((types: string[]) => {
    send({ type: 'subscribe', event_types: types });
  }, [send]);

  /** Cleanly close the connection and cancel any pending reconnect. */
  const disconnect = useCallback(() => {
    if (retryTimer.current) clearTimeout(retryTimer.current);
    wsRef.current?.close();
    wsRef.current = null;
    retryCount.current = 0;
    setStatus('idle');
  }, []);

  /** Force an immediate reconnect (resets retry counter). */
  const reconnect = useCallback(() => {
    disconnect();
    retryCount.current = 0;
    setTimeout(connect, 50);
  }, [disconnect, connect]);

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  useEffect(() => {
    if (enabled) {
      connect();
    } else {
      disconnect();
    }
    return () => {
      if (retryTimer.current) clearTimeout(retryTimer.current);
      wsRef.current?.close();
    };
  }, [enabled]); // eslint-disable-line react-hooks/exhaustive-deps
  // ^ connect/disconnect are stable but adding them causes double-connects on
  //   strict-mode double-effect; enabled is the only meaningful trigger here.

  return { status, lastMessage, send, ping, subscribe, disconnect, reconnect };
}
