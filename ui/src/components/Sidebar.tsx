import {
  Bell,
  Cpu,
  GitBranch,
  Calculator,
  Coins,
  Database,
  FileText,
  FlaskConical,
  KeyRound,
  ServerCrash,
  Layers,
  LayoutDashboard,
  ListChecks,
  LogOut,
  MonitorPlay,
  BookOpen,
  Network,
  Play,
  Puzzle,
  Settings,
  ScrollText,
  Shield,
  Terminal,
  TestTube,
  User,
  BarChart2,
  Waves,
  Zap,
  CheckSquare,
} from 'lucide-react';
import { clsx } from 'clsx';
import { useQuery } from '@tanstack/react-query';
import { clearStoredToken } from '../lib/api';
import { usePresence, type PresenceEntry } from '../hooks/usePresence';
import type { ReactNode } from 'react';

export type NavPage =
  | 'dashboard'
  | 'workspaces'
  | 'sessions'
  | 'runs'
  | 'tasks'
  | 'workers'
  | 'orchestration'
  | 'approvals'
  | 'prompts'
  | 'traces'
  | 'memory'
  | 'costs'
  | 'cost-calc'
  | 'graph'
  | 'sources'
  | 'providers'
  | 'plugins'
  | 'credentials'
  | 'channels'
  | 'playground'
  | 'audit-log'
  | 'logs'
  | 'metrics'
  | 'evals'
  | 'api-docs'
  | 'test-harness'
  | 'settings'
  | 'deployment'
  | 'profile';

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
      { id: 'workspaces', label: 'Workspaces', icon: Layers         },
    ],
  },
  {
    label: 'Operations',
    items: [
      { id: 'sessions',  label: 'Sessions',  icon: MonitorPlay },
      { id: 'runs',      label: 'Runs',      icon: Play        },
      { id: 'tasks',     label: 'Tasks',     icon: ListChecks  },
      { id: 'workers',        label: 'Workers',        icon: Cpu       },
      { id: 'orchestration',  label: 'Orchestration',  icon: GitBranch },
      { id: 'approvals', label: 'Approvals', icon: CheckSquare },
      { id: 'prompts',   label: 'Prompts',   icon: FileText    },
    ],
  },
  {
    label: 'Observability',
    items: [
      { id: 'traces',    label: 'Traces',    icon: Waves         },
      { id: 'memory',    label: 'Memory',    icon: Database      },
      { id: 'sources',   label: 'Sources',   icon: Database      },
      { id: 'costs',     label: 'Costs',     icon: Coins         },
      { id: 'cost-calc', label: 'Calculator', icon: Calculator    },
      { id: 'evals',     label: 'Evals',     icon: FlaskConical  },
      { id: 'graph',     label: 'Graph',     icon: Network       },
      { id: 'audit-log', label: 'Audit Log', icon: Shield        },
      { id: 'logs',      label: 'Logs',      icon: ScrollText    },
      { id: 'metrics',   label: 'Metrics',   icon: BarChart2     },
    ],
  },
  {
    label: 'Infrastructure',
    items: [
      { id: 'providers',   label: 'Providers',   icon: Zap      },
      { id: 'plugins',     label: 'Plugins',     icon: Puzzle   },
      { id: 'credentials',  label: 'Credentials',  icon: KeyRound     },
      { id: 'deployment',   label: 'Deployment',   icon: ServerCrash  },
      { id: 'channels',     label: 'Channels',     icon: Bell     },
      { id: 'playground',    label: 'Playground',    icon: Terminal  },
      { id: 'test-harness',  label: 'Test Harness',  icon: TestTube  },
      { id: 'api-docs',    label: 'API Docs',    icon: BookOpen },
      { id: 'settings',    label: 'Settings',    icon: Settings },
    ],
  },
];

// ── Presence dots ─────────────────────────────────────────────────────────────

function PresenceDots({ entries }: { entries: PresenceEntry[] }) {
  if (entries.length === 0) return null;
  const shown = entries.slice(0, 3);
  const overflow = entries.length - shown.length;
  return (
    <span className="ml-auto flex items-center gap-0.5 shrink-0">
      {shown.map(e => (
        <span
          key={e.id}
          className="inline-block w-[5px] h-[5px] rounded-full"
          style={{ backgroundColor: e.color }}
          title="Another user is viewing this page"
          aria-hidden="true"
        />
      ))}
      {overflow > 0 && (
        <span className="text-[9px] text-zinc-600 font-mono leading-none">+{overflow}</span>
      )}
    </span>
  );
}

// ── Component ─────────────────────────────────────────────────────────────────

interface SidebarProps {
  current: NavPage;
  onNavigate: (page: NavPage) => void;
  mobileOpen?: boolean;
  onMobileClose?: () => void;
  onLogout?: () => void;
}

export function Sidebar({ current, onNavigate, mobileOpen = false, onMobileClose, onLogout }: SidebarProps): ReactNode {
  const server = (import.meta.env.VITE_API_URL ?? 'localhost:3000')
    .replace(/^https?:\/\//, '');

  const presenceByPage = usePresence(current);

  // Fetch server version from X-Cairn-Version header via the health endpoint.
  const { data: serverVersion } = useQuery({
    queryKey: ['server-version'],
    queryFn: async () => {
      const resp = await fetch('/health');
      return resp.headers.get('X-Cairn-Version') ?? null;
    },
    staleTime: 60_000,
    retry: false,
  });

  return (
    <>
      {/* Mobile backdrop */}
      {mobileOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/60 lg:hidden"
          onClick={onMobileClose}
          aria-hidden="true"
        />
      )}

      <aside
        aria-label="Main navigation"
        className={clsx(
          'flex flex-col h-screen',
          'bg-white dark:bg-zinc-950',
          'border-r border-gray-200 dark:border-zinc-800',
          // Mobile/tablet: fixed overlay; desktop: static in-flow
          'fixed inset-y-0 left-0 z-50 transition-transform duration-200 ease-in-out',
          'lg:static lg:z-auto lg:shrink-0',
          mobileOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0',
        )}
        style={{ width: 220 }}
      >
        {/* Logo / wordmark */}
        <div className="flex items-center gap-3 px-4 h-14 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          {/* Favicon-matched icon: four stacked stones */}
          <svg
            aria-hidden="true"
            width="22"
            height="22"
            viewBox="0 0 32 32"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
            className="shrink-0"
          >
            <rect x="3"  y="24" width="26" height="5" rx="2.5" fill="#4f46e5"/>
            <rect x="6"  y="17" width="20" height="5" rx="2.5" fill="#6366f1"/>
            <rect x="9"  y="10" width="14" height="5" rx="2.5" fill="#818cf8"/>
            <rect x="12" y="5"  width="8"  height="4" rx="2"   fill="#a5b4fc"/>
          </svg>
          <div className="flex flex-col min-w-0">
            <span className="text-[20px] font-semibold leading-tight text-gray-900 dark:text-zinc-100 tracking-tight">
              cairn
            </span>
            <span className="text-[10px] text-gray-400 dark:text-zinc-500 leading-none mt-0.5">
              control plane
            </span>
          </div>
        </div>

        {/* Navigation — grouped */}
        <nav role="navigation" aria-label="Main navigation" className="flex-1 overflow-y-auto py-2 px-2 space-y-4">
          {NAV_GROUPS.map((group) => (
            <div key={group.label}>
              <p className="px-3 pb-1 text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
                {group.label}
              </p>
              <div className="space-y-0.5">
                {group.items.map(({ id, label, icon: Icon }) => {
                  const active   = current === id;
                  const visitors = presenceByPage.get(id) ?? [];
                  return (
                    <button
                      key={id}
                      onClick={() => onNavigate(id)}
                      aria-current={active ? 'page' : undefined}
                      aria-label={label}
                      className={clsx(
                        'w-full flex items-center gap-2.5 px-3 py-1.5 rounded text-[13px] font-medium transition-colors relative',
                        active
                          ? 'bg-gray-100 dark:bg-zinc-800/80 text-gray-900 dark:text-zinc-100'
                          : 'text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800/50 hover:text-gray-900 dark:hover:text-zinc-100',
                      )}
                    >
                      {/* Left accent on active */}
                      {active && (
                        <span className="absolute left-0 inset-y-1 w-0.5 rounded-full bg-indigo-500" />
                      )}
                      <Icon
                        size={14}
                        className={clsx(
                          'shrink-0',
                          active ? 'text-indigo-500 dark:text-indigo-400' : 'text-gray-400 dark:text-zinc-500',
                        )}
                      />
                      <span className="truncate">{label}</span>
                      <PresenceDots entries={visitors} />
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
        </nav>

        {/* Footer */}
        <div className="px-3 py-3 border-t border-gray-200 dark:border-zinc-800 space-y-1">
          <div className="flex items-center justify-between px-1">
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono truncate" title={server}>
              {server}
            </p>
            {serverVersion && (
              <span className="ml-2 shrink-0 text-[9px] font-mono font-medium text-indigo-500 dark:text-indigo-400
                               bg-indigo-500/10 border border-indigo-500/20 rounded px-1.5 py-0.5"
                    title={`Server version ${serverVersion}`}>
                v{serverVersion}
              </span>
            )}
          </div>
          {/* Profile link */}
          <button
            onClick={() => onNavigate('profile')}
            aria-current={current === 'profile' ? 'page' : undefined}
            aria-label="Account"
            className={clsx(
              'w-full flex items-center gap-2 px-3 py-1.5 rounded text-[12px] font-medium transition-colors relative',
              current === 'profile'
                ? 'bg-gray-100 dark:bg-zinc-800/80 text-gray-900 dark:text-zinc-100'
                : 'text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800/50 hover:text-gray-900 dark:hover:text-zinc-100',
            )}
          >
            {current === 'profile' && (
              <span className="absolute left-0 inset-y-1 w-0.5 rounded-full bg-indigo-500" />
            )}
            <User
              size={13}
              className={clsx('shrink-0', current === 'profile' ? 'text-indigo-500 dark:text-indigo-400' : 'text-gray-400 dark:text-zinc-500')}
            />
            Account
          </button>
          <button
            onClick={() => {
              clearStoredToken();
              if (onLogout) {
                onLogout();
              } else {
                window.location.reload();
              }
            }}
            className="w-full flex items-center gap-2 px-3 py-1.5 rounded text-[12px]
                       text-gray-500 dark:text-zinc-600
                       hover:bg-gray-100 dark:hover:bg-zinc-800/50
                       hover:text-red-500 dark:hover:text-red-400
                       transition-colors group"
          >
            <LogOut size={12} className="shrink-0 group-hover:text-red-500 dark:group-hover:text-red-400 transition-colors" />
            Sign out
          </button>
        </div>
      </aside>
    </>
  );
}
