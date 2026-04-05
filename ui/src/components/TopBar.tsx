import { useEffect, useState } from 'react';
import { defaultApi } from '../lib/api';
import type { SystemStatus } from '../lib/types';
import { clsx } from 'clsx';

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

interface StatusDotProps {
  healthy: boolean;
  label: string;
}

function StatusDot({ healthy, label }: StatusDotProps) {
  return (
    <span className="flex items-center gap-1.5 text-xs text-zinc-400">
      <span
        className={clsx(
          'inline-block w-2 h-2 rounded-full',
          healthy ? 'bg-emerald-400' : 'bg-red-500',
        )}
      />
      {label}
    </span>
  );
}

interface TopBarProps {
  title: string;
}

export function TopBar({ title }: TopBarProps) {
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [healthy, setHealthy] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function poll() {
      try {
        const [health, sys] = await Promise.all([
          defaultApi.getHealth(),
          defaultApi.getStatus(),
        ]);
        if (!cancelled) {
          setHealthy(health.ok);
          setStatus(sys);
        }
      } catch {
        if (!cancelled) {
          setHealthy(false);
          setStatus(null);
        }
      }
    }

    poll();
    const interval = setInterval(poll, 15_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return (
    <header className="flex items-center justify-between h-12 px-5 bg-zinc-950 border-b border-zinc-800 shrink-0">
      {/* Page title */}
      <h1 className="text-sm font-medium text-zinc-100">{title}</h1>

      {/* Status indicators */}
      <div className="flex items-center gap-4">
        {healthy !== null && (
          <StatusDot
            healthy={healthy}
            label={healthy ? 'Healthy' : 'Degraded'}
          />
        )}

        {status && (
          <>
            <StatusDot healthy={status.runtime_ok} label="Runtime" />
            <StatusDot healthy={status.store_ok}   label="Store"   />
            <span className="text-xs text-zinc-500 font-mono tabular-nums">
              up {formatUptime(status.uptime_secs)}
            </span>
          </>
        )}

        {healthy === null && (
          <span className="flex items-center gap-1.5 text-xs text-zinc-600">
            <span className="inline-block w-2 h-2 rounded-full bg-zinc-600 animate-pulse" />
            Connecting…
          </span>
        )}
      </div>
    </header>
  );
}
