import {
  LayoutDashboard,
  Play,
  MonitorPlay,
  CheckSquare,
  Zap,
  Database,
  Settings,
  ChevronRight,
} from 'lucide-react';
import { clsx } from 'clsx';

export type NavPage =
  | 'dashboard'
  | 'runs'
  | 'sessions'
  | 'approvals'
  | 'providers'
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
  { id: 'memory',     label: 'Memory',      icon: Database        },
  { id: 'settings',   label: 'Settings',    icon: Settings        },
];

interface SidebarProps {
  current: NavPage;
  onNavigate: (page: NavPage) => void;
}

export function Sidebar({ current, onNavigate }: SidebarProps) {
  return (
    <aside className="flex flex-col w-56 shrink-0 bg-zinc-950 border-r border-zinc-800 h-screen">
      {/* Wordmark */}
      <div className="flex items-center gap-2 px-4 py-5 border-b border-zinc-800">
        <div className="w-6 h-6 rounded bg-indigo-500 flex items-center justify-center">
          <ChevronRight size={14} className="text-white" />
        </div>
        <span className="text-zinc-100 font-semibold tracking-tight text-sm">cairn</span>
        <span className="ml-auto text-[10px] text-zinc-500 font-mono">v0.1</span>
      </div>

      {/* Navigation */}
      <nav className="flex-1 overflow-y-auto py-3 px-2 space-y-0.5">
        {NAV_ITEMS.map(({ id, label, icon: Icon }) => {
          const active = current === id;
          return (
            <button
              key={id}
              onClick={() => onNavigate(id)}
              className={clsx(
                'w-full flex items-center gap-3 px-3 py-2 rounded-md text-sm transition-colors',
                active
                  ? 'bg-zinc-800 text-zinc-100'
                  : 'text-zinc-400 hover:bg-zinc-900 hover:text-zinc-200',
              )}
            >
              <Icon size={16} className={active ? 'text-indigo-400' : undefined} />
              {label}
            </button>
          );
        })}
      </nav>

      {/* Footer hint */}
      <div className="px-4 py-3 border-t border-zinc-800">
        <p className="text-[10px] text-zinc-600 font-mono">localhost:3000</p>
      </div>
    </aside>
  );
}
