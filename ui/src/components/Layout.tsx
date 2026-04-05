import { useState, useEffect, type ReactNode } from 'react';
import { Sidebar, type NavPage } from './Sidebar';
import { TopBar } from './TopBar';
import { CommandPalette } from './CommandPalette';

// All top-level pages — must match NavPage union in Sidebar.tsx
const VALID_PAGES: NavPage[] = [
  'dashboard',
  'sessions', 'runs', 'tasks', 'approvals',
  'traces', 'memory', 'costs',
  'providers', 'playground', 'settings',
];

// ── Route descriptor ──────────────────────────────────────────────────────────

/** A parsed hash route: either a named page or a dynamic segment. */
export type Route =
  | { kind: 'page'; page: NavPage }
  | { kind: 'run-detail'; runId: string }
  | { kind: 'session-detail'; sessionId: string };

export function parseRoute(hash: string): Route {
  const h = hash.replace(/^#/, '');
  // Dynamic: #run/<runId>
  if (h.startsWith('run/') && h.length > 4) {
    return { kind: 'run-detail', runId: h.slice(4) };
  }
  // Dynamic: #session/<sessionId>
  if (h.startsWith('session/') && h.length > 8) {
    return { kind: 'session-detail', sessionId: h.slice(8) };
  }
  const page = h as NavPage;
  return { kind: 'page', page: VALID_PAGES.includes(page) ? page : 'dashboard' };
}

export function currentRoute(): Route {
  return parseRoute(window.location.hash);
}

// ── Page metadata ─────────────────────────────────────────────────────────────

export const PAGE_TITLES: Record<NavPage, string> = {
  dashboard:  'Dashboard',
  sessions:   'Sessions',
  runs:       'Runs',
  tasks:      'Tasks',
  approvals:  'Approvals',
  traces:     'Traces',
  memory:     'Memory',
  costs:      'Costs',
  providers:  'Providers',
  playground: 'Playground',
  settings:   'Settings',
};

function routeTitle(route: Route): string {
  if (route.kind === 'run-detail') return `Run ${route.runId.slice(0, 12)}…`;
  if (route.kind === 'session-detail') return `Session ${route.sessionId.slice(0, 12)}…`;
  return PAGE_TITLES[route.page];
}

function activePage(route: Route): NavPage {
  if (route.kind === 'run-detail') return 'runs';
  if (route.kind === 'session-detail') return 'sessions';
  return route.page;
}

function PlaceholderPage({ page }: { page: NavPage }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-2 text-gray-400 dark:text-zinc-700">
      <span className="text-xl font-semibold text-gray-400 dark:text-zinc-600">{PAGE_TITLES[page]}</span>
      <p className="text-[13px]">Coming soon.</p>
    </div>
  );
}

// ── Layout ────────────────────────────────────────────────────────────────────

interface LayoutProps {
  /** Render prop receives the current named page (for top-level nav). */
  children?: (page: NavPage) => ReactNode;
  /** Render prop for dynamic routes (run detail, etc.). */
  routeRenderer?: (route: Route) => ReactNode;
}

export function Layout({ children, routeRenderer }: LayoutProps) {
  const [route, setRoute]           = useState<Route>(currentRoute);
  const [sidebarOpen, setSidebarOpen] = useState(false);

  useEffect(() => {
    const onHashChange = () => {
      setRoute(currentRoute());
      setSidebarOpen(false); // close on any hash navigation (back button, row clicks)
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  function navigate(next: NavPage) {
    window.location.hash = next;
    setRoute({ kind: 'page', page: next });
    setSidebarOpen(false);
  }

  const page = activePage(route);

  let content: ReactNode;
  if (routeRenderer) {
    const dynamic = routeRenderer(route);
    if (dynamic !== null) {
      content = dynamic;
    } else if (children) {
      content = route.kind === 'page' ? children(route.page) : null;
    }
  } else if (children) {
    content = route.kind === 'page' ? children(route.page) : null;
  }
  content ??= <PlaceholderPage page={page} />;

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-white dark:bg-zinc-950 text-gray-900 dark:text-zinc-200">
      <Sidebar
        current={page}
        onNavigate={navigate}
        mobileOpen={sidebarOpen}
        onMobileClose={() => setSidebarOpen(false)}
      />

      <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
        <TopBar title={routeTitle(route)} onMenuClick={() => setSidebarOpen(v => !v)} />

        <main className="flex-1 overflow-hidden bg-gray-50 dark:bg-zinc-950">
          {content}
        </main>
      </div>

      <CommandPalette onNavigate={navigate} />
    </div>
  );
}
