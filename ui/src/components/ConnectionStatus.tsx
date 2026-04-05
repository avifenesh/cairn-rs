/**
 * ConnectionStatus — fixed bottom-right connection health indicator.
 *
 * Collapsed: single colored dot (green/yellow/red).
 * Expanded:  three rows — API, SSE stream, Ollama — each with status + latency.
 *
 * Health checks run every 30 s via useQuery.  The SSE status comes from the
 * existing useEventStream hook so we don't open a second SSE connection.
 */

import { useState, useEffect, useRef } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Wifi, WifiOff, Loader2, ChevronDown, ChevronUp, X, RefreshCw, CloudOff } from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useEventStream } from '../hooks/useEventStream';
import { onNetworkChange, isOnline, hasPendingUpdate, applyUpdate } from '../lib/registerSW';

// ── Types ─────────────────────────────────────────────────────────────────────

type ServiceStatus = 'ok' | 'degraded' | 'down' | 'checking';

interface ServiceState {
  status:     ServiceStatus;
  label:      string;
  detail?:    string;   // short status line
  latencyMs?: number;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function overallStatus(services: ServiceState[]): ServiceStatus {
  if (services.some(s => s.status === 'checking')) return 'checking';
  if (services.some(s => s.status === 'down'))     return 'down';
  if (services.some(s => s.status === 'degraded')) return 'degraded';
  return 'ok';
}

const DOT_CLASS: Record<ServiceStatus, string> = {
  ok:       'bg-emerald-500',
  degraded: 'bg-amber-500',
  down:     'bg-red-500 animate-pulse',
  checking: 'bg-zinc-500 animate-pulse',
};

const TEXT_CLASS: Record<ServiceStatus, string> = {
  ok:       'text-emerald-400',
  degraded: 'text-amber-400',
  down:     'text-red-400',
  checking: 'text-zinc-500',
};

const STATUS_LABEL: Record<ServiceStatus, string> = {
  ok:       'Connected',
  degraded: 'Degraded',
  down:     'Disconnected',
  checking: 'Checking…',
};

// ── Service row ───────────────────────────────────────────────────────────────

function ServiceRow({ svc }: { svc: ServiceState }) {
  return (
    <div className="flex items-center gap-2.5 py-1.5">
      <span className={clsx(
        'shrink-0 w-1.5 h-1.5 rounded-full',
        svc.status === 'checking' ? 'bg-zinc-600 animate-pulse' : DOT_CLASS[svc.status],
      )} />
      <span className="flex-1 text-[12px] text-zinc-300">{svc.label}</span>
      <span className={clsx('text-[11px] font-medium', TEXT_CLASS[svc.status])}>
        {svc.detail ?? STATUS_LABEL[svc.status]}
      </span>
      {svc.latencyMs !== undefined && (
        <span className="text-[10px] text-zinc-600 font-mono tabular-nums w-12 text-right">
          {svc.latencyMs}ms
        </span>
      )}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function ConnectionStatus() {
  const [expanded,    setExpanded]    = useState(false);
  const [dismissed,   setDismissed]   = useState(false);
  const [offline,     setOffline]     = useState(!isOnline());
  const [swUpdate,    setSwUpdate]    = useState(hasPendingUpdate());
  const containerRef = useRef<HTMLDivElement>(null);

  // Track browser online/offline state.
  useEffect(() => onNetworkChange(setOffline), []);

  // Listen for SW update-ready events.
  useEffect(() => {
    const handler = () => setSwUpdate(true);
    window.addEventListener('sw-update-ready', handler);
    return () => window.removeEventListener('sw-update-ready', handler);
  }, []);

  // ── API health check ────────────────────────────────────────────────────────
  const {
    data:      apiData,
    isLoading: apiLoading,
    isError:   apiError,
    dataUpdatedAt,
  } = useQuery({
    queryKey:       ['health-check'],
    queryFn:        async () => {
      const t0 = Date.now();
      const [health, status] = await Promise.all([
        defaultApi.getHealth(),
        defaultApi.getStatus(),
      ]);
      return { health, status, latency: Date.now() - t0 };
    },
    refetchInterval: 30_000,
    retry:           1,
    staleTime:       25_000,
  });

  // ── Ollama health ───────────────────────────────────────────────────────────
  const { data: ollamaData, isError: ollamaError } = useQuery({
    queryKey:        ['ollama-health'],
    queryFn:         async () => {
      const t0 = Date.now();
      const d = await defaultApi.getOllamaModels();
      return { models: d.count, latency: Date.now() - t0 };
    },
    refetchInterval:  60_000,
    retry:            0,
    staleTime:        55_000,
  });

  // ── SSE stream status (reuses the singleton hook) ──────────────────────────
  const { status: sseStatus } = useEventStream({ enabled: true });

  // Close on outside click when expanded.
  useEffect(() => {
    if (!expanded) return;
    function handler(e: PointerEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setExpanded(false);
      }
    }
    document.addEventListener('pointerdown', handler);
    return () => document.removeEventListener('pointerdown', handler);
  }, [expanded]);

  // Build service states.
  const apiService: ServiceState = apiLoading
    ? { status: 'checking', label: 'API' }
    : apiError
      ? { status: 'down',    label: 'API', detail: 'unreachable' }
      : {
          status:    apiData?.status.runtime_ok && apiData?.status.store_ok ? 'ok' : 'degraded',
          label:     'API',
          detail:    apiData?.status.runtime_ok && apiData?.status.store_ok ? 'healthy' : 'store issue',
          latencyMs: apiData?.latency,
        };

  const sseService: ServiceState = {
    status: sseStatus === 'connected'
      ? 'ok'
      : sseStatus === 'connecting'
        ? 'degraded'
        : 'down',
    label:  'SSE stream',
    detail: sseStatus,
  };

  const ollamaService: ServiceState = ollamaError
    ? { status: 'down',    label: 'Ollama', detail: 'not configured' }
    : !ollamaData
      ? { status: 'checking', label: 'Ollama' }
      : {
          status:    'ok',
          label:     'Ollama',
          detail:    `${ollamaData.models} model${ollamaData.models !== 1 ? 's' : ''}`,
          latencyMs: ollamaData.latency,
        };

  const services = [apiService, sseService, ollamaService];
  const overall  = overallStatus(services);

  // Don't render after explicit dismiss (until page refresh).
  if (dismissed) return null;

  const lastChecked = dataUpdatedAt
    ? new Date(dataUpdatedAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : null;

  return (
    <>
      {/* ── Offline banner (top of screen) ─────────────────────────────── */}
      {offline && (
        <div
          className="fixed top-0 inset-x-0 z-50 flex items-center justify-center gap-2
                     bg-red-950/95 border-b border-red-800/60 px-4 py-2 backdrop-blur-sm
                     no-print"
          role="alert"
          aria-live="assertive"
        >
          <CloudOff size={13} className="text-red-400 shrink-0" />
          <span className="text-[12px] font-medium text-red-300">
            You are offline — data may be stale
          </span>
        </div>
      )}

      {/* ── SW update banner (top of screen, below offline if both shown) ── */}
      {swUpdate && !offline && (
        <div
          className="fixed top-0 inset-x-0 z-50 flex items-center justify-center gap-3
                     bg-indigo-950/95 border-b border-indigo-800/60 px-4 py-2 backdrop-blur-sm
                     no-print"
          role="alert"
        >
          <RefreshCw size={12} className="text-indigo-400 shrink-0" />
          <span className="text-[12px] text-indigo-300">
            A new version of cairn is available.
          </span>
          <button
            onClick={applyUpdate}
            className="text-[12px] font-semibold text-white bg-indigo-600 hover:bg-indigo-500
                       rounded px-2.5 py-0.5 transition-colors"
          >
            Reload
          </button>
          <button
            onClick={() => setSwUpdate(false)}
            className="text-indigo-500 hover:text-indigo-300 transition-colors ml-1"
            aria-label="Dismiss update notification"
          >
            <X size={12} />
          </button>
        </div>
      )}

    <div
      ref={containerRef}
      className="fixed bottom-4 right-4 z-40 flex flex-col items-end gap-0"
      aria-live="polite"
      aria-label="Connection status"
    >
      {/* Expanded panel */}
      {expanded && (
        <div className="mb-1.5 w-64 rounded-xl bg-zinc-900 border border-zinc-800
                        shadow-2xl shadow-black/50 overflow-hidden
                        animate-[fadeIn_150ms_ease-out]">
          {/* Header */}
          <div className="flex items-center justify-between px-3 py-2.5 border-b border-zinc-800">
            <div className="flex items-center gap-2">
              {overall === 'checking'
                ? <Loader2 size={13} className="text-zinc-500 animate-spin" />
                : overall === 'ok'
                  ? <Wifi    size={13} className="text-emerald-400" />
                  : <WifiOff size={13} className="text-red-400" />
              }
              <span className={clsx('text-[12px] font-semibold', TEXT_CLASS[overall])}>
                {STATUS_LABEL[overall]}
              </span>
            </div>
            <button
              onClick={() => setDismissed(true)}
              aria-label="Dismiss connection status"
              className="p-0.5 rounded text-zinc-600 hover:text-zinc-300 transition-colors"
            >
              <X size={12} />
            </button>
          </div>

          {/* Service rows */}
          <div className="px-3 divide-y divide-zinc-800/60">
            {services.map(svc => <ServiceRow key={svc.label} svc={svc} />)}
          </div>

          {/* Footer */}
          {lastChecked && (
            <div className="px-3 py-2 border-t border-zinc-800/60 flex items-center justify-between">
              <span className="text-[10px] text-zinc-700">Last checked {lastChecked}</span>
              <span className="text-[10px] text-zinc-700">auto every 30s</span>
            </div>
          )}
        </div>
      )}

      {/* Collapsed trigger */}
      <button
        onClick={() => setExpanded(v => !v)}
        title={`Connection: ${STATUS_LABEL[overall]}`}
        aria-expanded={expanded}
        className={clsx(
          'flex items-center gap-1.5 rounded-full px-2.5 py-1.5 transition-all duration-200',
          'border shadow-lg shadow-black/30',
          'bg-zinc-900/90 backdrop-blur-sm',
          expanded
            ? 'border-zinc-700 pr-2'
            : 'border-zinc-800 hover:border-zinc-600',
        )}
      >
        {/* Animated status dot */}
        <span className={clsx(
          'w-2 h-2 rounded-full shrink-0 transition-colors duration-500',
          DOT_CLASS[overall],
        )} />

        {/* Label — always visible so users understand what the dot means */}
        <span className={clsx(
          'text-[11px] font-medium transition-colors duration-200',
          TEXT_CLASS[overall],
        )}>
          {overall === 'checking' ? 'Checking' : STATUS_LABEL[overall]}
        </span>

        {expanded
          ? <ChevronDown size={11} className="text-zinc-600" />
          : <ChevronUp   size={11} className="text-zinc-600" />
        }
      </button>
    </div>
    </>
  );
}
