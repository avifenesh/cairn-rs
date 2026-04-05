import { useEffect, useState } from 'react';
import { Menu } from 'lucide-react';
import { defaultApi } from '../lib/api';
import type { SystemStatus } from '../lib/types';
import { clsx } from 'clsx';

function formatUptime(secs: number): string {
  if (secs < 60)   return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
}

function Dot({ ok, label }: { ok: boolean; label: string }) {
  return (
    <span className="flex items-center gap-1.5 text-[11px] text-zinc-500">
      <span className={clsx('inline-block w-1.5 h-1.5 rounded-full shrink-0',
        ok ? 'bg-emerald-500' : 'bg-red-500')} />
      {label}
    </span>
  );
}

interface TopBarProps {
  title: string;
  /** Callback to open/close the sidebar on mobile/tablet. */
  onMenuClick?: () => void;
}

export function TopBar({ title, onMenuClick }: TopBarProps) {
  const [status, setStatus]   = useState<SystemStatus | null>(null);
  const [healthy, setHealthy] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const [h, s] = await Promise.all([defaultApi.getHealth(), defaultApi.getStatus()]);
        if (!cancelled) { setHealthy(h.ok); setStatus(s); }
      } catch {
        if (!cancelled) { setHealthy(false); setStatus(null); }
      }
    }
    poll();
    const t = setInterval(poll, 15_000);
    return () => { cancelled = true; clearInterval(t); };
  }, []);

  return (
    <header className="flex items-center h-11 px-4 bg-zinc-950 border-b border-zinc-800 shrink-0 gap-3">
      {/* Hamburger — visible on tablet/mobile only */}
      <button
        onClick={onMenuClick}
        className="lg:hidden -ml-1 p-1.5 rounded text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800 transition-colors shrink-0"
        aria-label="Open menu"
      >
        <Menu size={16} />
      </button>

      <h1 className="text-[13px] font-medium text-zinc-200 truncate">{title}</h1>

      <div className="ml-auto flex items-center gap-3 shrink-0">
        {/* Compact single dot on mobile */}
        {healthy !== null && (
          <span className={clsx(
            'lg:hidden w-2 h-2 rounded-full shrink-0',
            healthy ? 'bg-emerald-500' : 'bg-red-500 animate-pulse',
          )} />
        )}

        {/* Full health breakdown on desktop */}
        {healthy === null ? (
          <span className="hidden lg:flex items-center gap-1.5 text-[11px] text-zinc-600">
            <span className="w-1.5 h-1.5 rounded-full bg-zinc-700 animate-pulse inline-block" />
            connecting
          </span>
        ) : (
          <div className="hidden lg:flex items-center gap-4">
            <Dot ok={healthy}              label={healthy ? 'ok' : 'degraded'} />
            {status && <Dot ok={status.runtime_ok} label="runtime" />}
            {status && <Dot ok={status.store_ok}   label="store"   />}
            {status && (
              <span className="text-[11px] text-zinc-600 font-mono tabular-nums">
                {formatUptime(status.uptime_secs)}
              </span>
            )}
          </div>
        )}
      </div>
    </header>
  );
}
