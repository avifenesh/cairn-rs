import {
  Coins,
  Database,
  LayoutDashboard,
  ListChecks,
  LogOut,
  MonitorPlay,
  Play,
  Settings,
  Square,
  Terminal,
  Waves,
  Zap,
  CheckSquare,
} from 'lucide-react';
import { clsx } from 'clsx';
import { clearStoredToken } from '../lib/api';

export type NavPage =
  | 'dashboard'
  | 'sessions'
  | 'runs'
  | 'tasks'
  | 'approvals'
  | 'traces'
  | 'memory'
  | 'costs'
  | 'providers'
  | 'playground'
  | 'settings';

// ── Nav structure ─────────────────────────────────────────────────────────────

interface NavItem {
  id: NavPage;
  label: string;
  icon: React.ComponentType<{ className?: string; size?: number }>;
}

interface NavGroup {
  label: string;
  items: NavItem[];
}

const NAV_GROUPS: NavGroup[] = [
  {
    label: 'Overview',
    items: [
      { id: 'dashboard',  label: 'Dashboard',  icon: LayoutDashboard },
    ],
  },
  {
    label: 'Operations',
    items: [
      { id: 'sessions',  label: 'Sessions',  icon: MonitorPlay },
      { id: 'runs',      label: 'Runs',      icon: Play        },
      { id: 'tasks',     label: 'Tasks',     icon: ListChecks  },
      { id: 'approvals', label: 'Approvals', icon: CheckSquare },
    ],
  },
  {
    label: 'Observability',
    items: [
      { id: 'traces', label: 'Traces', icon: Waves    },
      { id: 'memory', label: 'Memory', icon: Database },
      { id: 'costs',  label: 'Costs',  icon: Coins    },
    ],
  },
  {
    label: 'Infrastructure',
    items: [
      { id: 'providers',  label: 'Providers',  icon: Zap      },
      { id: 'playground', label: 'Playground', icon: Terminal },
      { id: 'settings',   label: 'Settings',   icon: Settings },
    ],
  },
];

// ── Component ─────────────────────────────────────────────────────────────────

interface SidebarProps {
  current: NavPage;
  onNavigate: (page: NavPage) => void;
}

export function Sidebar({ current, onNavigate }: SidebarProps) {
  const server = (import.meta.env.VITE_API_URL ?? 'localhost:3000')
    .replace(/^https?:\/\//, '');

  return (
    <aside
      className="flex flex-col shrink-0 bg-zinc-950 border-r border-zinc-800 h-screen"
      style={{ width: 220 }}
    >
      {/* Wordmark */}
      <div className="flex items-center gap-2.5 px-4 h-11 border-b border-zinc-800 shrink-0">
        <span className="inline-flex h-5 w-5 items-center justify-center rounded bg-indigo-500 shrink-0">
          <Square size={9} className="text-white fill-white" />
        </span>
        <span className="text-[13px] font-semibold text-zinc-100 tracking-tight">cairn</span>
        <span className="ml-auto text-[10px] text-zinc-600 font-mono">v0.1</span>
      </div>

      {/* Navigation — grouped */}
      <nav className="flex-1 overflow-y-auto py-2 px-2 space-y-4">
        {NAV_GROUPS.map((group) => (
          <div key={group.label}>
            <p className="px-3 pb-1 text-[10px] font-medium text-zinc-500 uppercase tracking-wider">
              {group.label}
            </p>
            <div className="space-y-0.5">
              {group.items.map(({ id, label, icon: Icon }) => {
                const active = current === id;
                return (
                  <button
                    key={id}
                    onClick={() => onNavigate(id)}
                    className={clsx(
                      'w-full flex items-center gap-2.5 px-3 py-1.5 rounded text-[13px] font-medium transition-colors relative',
                      active
                        ? 'bg-zinc-800/80 text-zinc-100'
                        : 'text-zinc-400 hover:bg-zinc-800/50 hover:text-zinc-100',
                    )}
                  >
                    {/* Left accent on active */}
                    {active && (
                      <span className="absolute left-0 inset-y-1 w-0.5 rounded-full bg-indigo-500" />
                    )}
                    <Icon
                      size={14}
                      className={clsx('shrink-0', active ? 'text-indigo-400' : 'text-zinc-500')}
                    />
                    {label}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </nav>

      {/* Footer */}
      <div className="px-3 py-3 border-t border-zinc-800 space-y-1">
        <p className="px-1 text-[11px] text-zinc-600 font-mono truncate" title={server}>
          {server}
        </p>
        <button
          onClick={() => { clearStoredToken(); window.location.reload(); }}
          className="w-full flex items-center gap-2 px-3 py-1.5 rounded text-[12px] text-zinc-600
                     hover:bg-zinc-800/50 hover:text-red-400 transition-colors group"
        >
          <LogOut size={12} className="shrink-0 group-hover:text-red-400 transition-colors" />
          Sign out
        </button>
      </div>
    </aside>
  );
}
