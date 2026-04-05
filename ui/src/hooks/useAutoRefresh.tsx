/**
 * useAutoRefresh — configurable per-page polling interval.
 *
 * Persists the chosen interval to localStorage per page key.
 * The global `autoRefresh` preference in usePreferences acts as a master
 * kill-switch: when disabled, all pages behave as if interval = "off"
 * regardless of per-page settings.
 *
 * Usage:
 *   const { interval, setInterval, RefreshControl } = useAutoRefresh("runs");
 *   // Pass interval.ms as refetchInterval to useQuery (undefined = off)
 *   const { data } = useQuery({ ..., refetchInterval: interval.ms });
 *   // Render <RefreshControl /> in the toolbar
 */

import { useState, useCallback } from 'react';
import { RefreshCw } from 'lucide-react';
import { clsx } from 'clsx';

// ── Types ─────────────────────────────────────────────────────────────────────

export type RefreshOption = 'off' | '5s' | '15s' | '30s' | '60s';

export interface RefreshInterval {
  option: RefreshOption;
  /** Milliseconds to pass to refetchInterval, or undefined when off. */
  ms: number | false;
  label: string;
}

export interface UseAutoRefreshResult {
  interval:  RefreshInterval;
  setOption: (o: RefreshOption) => void;
  /** Convenience: use as `refetchInterval` prop on useQuery. */
  ms:        number | false;
}

// ── Constants ─────────────────────────────────────────────────────────────────

export const REFRESH_OPTIONS: RefreshInterval[] = [
  { option: 'off', ms: false,  label: 'Off'  },
  { option: '5s',  ms: 5_000,  label: '5 s'  },
  { option: '15s', ms: 15_000, label: '15 s' },
  { option: '30s', ms: 30_000, label: '30 s' },
  { option: '60s', ms: 60_000, label: '60 s' },
];

const OPTION_MAP = new Map<RefreshOption, RefreshInterval>(
  REFRESH_OPTIONS.map(r => [r.option, r]),
);

const LS_PREFIX = 'cairn_refresh_';
const LS_PREFS  = 'cairn_preferences'; // same key as usePreferences

// ── Storage helpers ───────────────────────────────────────────────────────────

function loadOption(pageKey: string): RefreshOption {
  try {
    const raw = localStorage.getItem(LS_PREFIX + pageKey);
    if (raw && OPTION_MAP.has(raw as RefreshOption)) return raw as RefreshOption;
  } catch { /* ignore */ }
  // Default: 15s for most pages
  return '15s';
}

function saveOption(pageKey: string, option: RefreshOption) {
  try { localStorage.setItem(LS_PREFIX + pageKey, option); } catch { /* ignore */ }
}

/** Read the global autoRefresh flag from the shared preferences store. */
export function loadGlobalAutoRefresh(): boolean {
  try {
    const raw = localStorage.getItem(LS_PREFS);
    if (!raw) return true;
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    // Default to true if the key is absent (preserves backwards-compat)
    return parsed['autoRefresh'] !== false;
  } catch { return true; }
}

// ── Hook ──────────────────────────────────────────────────────────────────────

/**
 * @param pageKey   Unique identifier used as localStorage key suffix.
 * @param defaultOption  Initial option if no saved preference exists (default: '15s').
 */
export function useAutoRefresh(
  pageKey: string,
  defaultOption: RefreshOption = '15s',
): UseAutoRefreshResult {
  const globalEnabled = loadGlobalAutoRefresh();

  const [option, setOptionState] = useState<RefreshOption>(() => {
    const saved = loadOption(pageKey);
    // If the file has no saved key yet, use the provided default
    try {
      if (localStorage.getItem(LS_PREFIX + pageKey) === null) return defaultOption;
    } catch { /* ignore */ }
    return saved;
  });

  const setOption = useCallback((o: RefreshOption) => {
    setOptionState(o);
    saveOption(pageKey, o);
  }, [pageKey]);

  // When global auto-refresh is disabled, always treat as "off"
  const effectiveOption: RefreshOption = globalEnabled ? option : 'off';
  const interval = OPTION_MAP.get(effectiveOption) ?? REFRESH_OPTIONS[0];

  return {
    interval,
    setOption,
    ms: interval.ms,
  };
}

// ── RefreshControl component ──────────────────────────────────────────────────

export interface RefreshControlProps {
  pageKey:    string;
  isFetching: boolean;
  onRefresh:  () => void;
  /** Default interval option (used before any localStorage preference exists). */
  defaultOption?: RefreshOption;
}

/**
 * Drop-in toolbar control: interval dropdown + manual refresh button.
 * Returns both the rendered element AND the interval ms for useQuery.
 *
 * @example
 *   const { ms, element } = useRefreshControl({ pageKey: "runs", isFetching, onRefresh: refetch });
 *   // use ms as refetchInterval, render element in toolbar
 */
export function useRefreshControl(props: RefreshControlProps): {
  ms:      number | false;
  element: React.ReactElement;
} {
  const { pageKey, isFetching, onRefresh, defaultOption = '15s' } = props;
  const { interval, setOption, ms } = useAutoRefresh(pageKey, defaultOption);
  const globalEnabled = loadGlobalAutoRefresh();

  const element = (
    <div className="flex items-center gap-1">
      {/* Interval dropdown */}
      <div className="relative">
        <select
          value={interval.option}
          onChange={e => setOption(e.target.value as RefreshOption)}
          disabled={!globalEnabled}
          className={clsx(
            "appearance-none rounded border bg-zinc-900 text-[11px] font-mono",
            "pl-5 pr-2 h-7 focus:outline-none focus:border-indigo-500 transition-colors",
            "disabled:opacity-40 disabled:cursor-not-allowed",
            interval.option === 'off'
              ? "border-zinc-700 text-zinc-600"
              : "border-zinc-700 text-zinc-400 hover:border-zinc-600",
          )}
          title={globalEnabled ? "Auto-refresh interval" : "Auto-refresh is disabled globally (see Preferences)"}
        >
          {REFRESH_OPTIONS.map(o => (
            <option key={o.option} value={o.option}>{o.label}</option>
          ))}
        </select>
        {/* Icon overlay */}
        <RefreshCw
          size={9}
          className={clsx(
            "absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none",
            isFetching ? "animate-spin text-indigo-400" : "text-zinc-600",
          )}
        />
      </div>

      {/* Manual refresh button */}
      <button
        onClick={onRefresh}
        disabled={isFetching}
        className="flex items-center gap-1 h-7 px-2 rounded border border-zinc-700 bg-zinc-900
                   text-[11px] text-zinc-500 hover:text-zinc-200 hover:border-zinc-600
                   disabled:opacity-40 transition-colors"
        title="Refresh now"
      >
        <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
        <span className="hidden sm:inline">Refresh</span>
      </button>
    </div>
  );

  return { ms, element };
}
