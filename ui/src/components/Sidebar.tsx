import {
  Coins,
  LayoutDashboard,
  Play,
  MonitorPlay,
  CheckSquare,
  Zap,
  Database,
  Settings,
  LogOut,
} from 'lucide-react';
import { clsx } from 'clsx';
import { clearStoredToken } from '../lib/api';

export type NavPage =
  | 'dashboard'
  | 'runs'
  | 'sessions'
  | 'approvals'
  | 'providers'
  | 'costs'
  | 'memory'
  | 'settings';

interface NavItem {
  id: NavPage;
  label: string;
  icon: React.ComponentType<{ className?: string; size?: number }>;
}

const NAV_ITEMS: NavItem[] = [
  { id: 'dashboard',  label: 'Dashboard',  icon: LayoutDashboard },
  { id: 'runs',       label: 'Runs',        icon: Play            },
  { id: 'sessions',   label: 'Sessions',    icon: MonitorPlay     },
  { id: 'approvals',  label: 'Approvals',   icon: CheckSquare     },
  { id: 'providers',  label: 'Providers',   icon: Zap             },
  { id: 'costs',      label: 'Costs',       icon: Coins           },
  { id: 'memory',     label: 'Memory',      icon: Database        },
  { id: 'settings',   label: 'Settings',    icon: Settings        },
];

interface SidebarProps {
  current: NavPage;
  onNavigate: (page: NavPage) => void;
}

export function Sidebar({ current, onNavigate }: SidebarProps) {
  const server = (import.meta.env.VITE_API_URL ?? 'localhost:3000')
    .replace(/^https?:\/\//, '');

  return (
    <aside className="flex flex-col w-[220px] shrink-0 bg-zinc-950 border-r border-zinc-800/60 h-screen">
      {/* Wordmark */}
      <div className="flex items-center gap-2.5 px-4 py-4 border-b border-zinc-800/60">
        <span className="inline-flex h-6 w-6 items-center justify-center rounded-md bg-indigo-600 shrink-0">
          <span className="block h-2 w-2 rounded-full bg-white/90" />
        </span>
        <span className="text-zinc-100 font-semibold tracking-tight text-sm">cairn</span>
        <span className="ml-auto text-[10px] text-zinc-600 font-mono">v0.1</span>
      </div>

      {/* Navigation */}
      <nav className="flex-1 overflow-y-auto pt-2 pb-2 px-2 space-y-0.5">
        {NAV_ITEMS.map(({ id, label, icon: Icon }) => {
          const active = current === id;
          return (
            <button
              key={id}
              onClick={() => onNavigate(id)}
              className={clsx(
                'w-full flex items-center gap-2.5 px-3 py-2 rounded-md text-sm transition-colors relative',
                active
                  ? 'bg-zinc-800/50 text-zinc-100'
                  : 'text-zinc-500 hover:bg-white/5 hover:text-zinc-300',
              )}
            >
              {/* Active left border indicator */}
              {active && (
                <span className="absolute left-0 top-1 bottom-1 w-0.5 rounded-full bg-indigo-500" />
              )}
              <Icon
                size={16}
                className={clsx(
                  'shrink-0',
                  active ? 'text-indigo-400' : 'text-zinc-500',
                )}
              />
              {label}
            </button>
          );
        })}
      </nav>

      {/* Footer: server + version + logout */}
      <div className="px-3 py-3 border-t border-zinc-800/60 space-y-1.5">
        <p className="px-1 text-[11px] text-zinc-600 font-mono truncate" title={server}>
          {server}
        </p>
        <button
          onClick={() => { clearStoredToken(); window.location.reload(); }}
          className="w-full flex items-center gap-2 px-3 py-1.5 rounded-md text-[11px] text-zinc-600 hover:bg-white/5 hover:text-red-400 transition-colors group"
        >
          <LogOut size={13} className="shrink-0 group-hover:text-red-400 transition-colors" />
          Sign out
        </button>
      </div>
    </aside>
  );
}
