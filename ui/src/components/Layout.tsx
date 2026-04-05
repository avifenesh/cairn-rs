import { useState, useEffect, useRef, type ReactNode } from 'react';
import { useIsFetching } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { Sidebar, type NavPage } from './Sidebar';
import { TopBar } from './TopBar';
import { CommandPalette } from './CommandPalette';
import { type BreadcrumbItem } from './Breadcrumb';

// All top-level pages — must match NavPage union in Sidebar.tsx
const VALID_PAGES: NavPage[] = [
  'dashboard',
  'sessions', 'runs', 'tasks', 'approvals',
  'traces', 'memory', 'costs', 'evals', 'graph', 'audit-log',
  'providers', 'playground', 'api-docs', 'settings',
];

// ── Route descriptor ──────────────────────────────────────────────────────────

/** A parsed hash route: either a named page or a dynamic segment. */
export type Route =
  | { kind: 'page'; page: NavPage }
  | { kind: 'run-detail'; runId: string }
  | { kind: 'session-detail'; sessionId: string };

export function parseRoute(hash: string): Route {
  const h = hash.replace(/^#/, '');
  if (h.startsWith('run/') && h.length > 4) {
    return { kind: 'run-detail', runId: h.slice(4) };
  }
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
  dashboard:   'Dashboard',
  sessions:    'Sessions',
  runs:        'Runs',
  tasks:       'Tasks',
  approvals:   'Approvals',
  traces:      'Traces',
  memory:      'Memory',
  costs:       'Costs',
  evals:       'Evaluations',
  graph:       'Knowledge Graph',
  'audit-log': 'Audit Log',
  'api-docs':  'API Reference',
  providers:   'Providers',
  playground:  'Playground',
  settings:    'Settings',
};

const PAGE_GROUP: Partial<Record<NavPage, string>> = {
  sessions:    'Operations',
  runs:        'Operations',
  tasks:       'Operations',
  approvals:   'Operations',
  traces:      'Observability',
  memory:      'Observability',
  costs:       'Observability',
  evals:       'Observability',
  graph:       'Observability',
  'audit-log': 'Observability',
  providers:   'Infrastructure',
  playground:  'Infrastructure',
  'api-docs':  'Infrastructure',
  settings:    'Infrastructure',
};

// ── Breadcrumb builder ────────────────────────────────────────────────────────

function shortId(id: string): string {
  return id.length > 16 ? `${id.slice(0, 12)}…` : id;
}

export function buildBreadcrumbs(route: Route): BreadcrumbItem[] {
  if (route.kind === 'page') {
    const { page } = route;
    if (page === 'dashboard') return [{ label: 'Dashboard' }];
    const group = PAGE_GROUP[page];
    if (group) return [{ label: group }, { label: PAGE_TITLES[page] }];
    return [{ label: PAGE_TITLES[page] }];
  }
  if (route.kind === 'run-detail') {
    return [
      { label: 'Operations' },
      { label: 'Runs', href: '#runs' },
      { label: shortId(route.runId) },
    ];
  }
  if (route.kind === 'session-detail') {
    return [
      { label: 'Operations' },
      { label: 'Sessions', href: '#sessions' },
      { label: shortId(route.sessionId) },
    ];
  }
  return [];
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

// ── Loading bar ───────────────────────────────────────────────────────────────

function LoadingBar() {
  const isFetching = useIsFetching();
  const [phase, setPhase] = useState<'idle' | 'loading' | 'done'>('idle');
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (isFetching > 0) {
      if (timerRef.current) clearTimeout(timerRef.current);
      setPhase('loading');
    } else if (phase === 'loading') {
      setPhase('done');
      timerRef.current = setTimeout(() => setPhase('idle'), 450);
    }
    return () => { if (timerRef.current) clearTimeout(timerRef.current); };
  }, [isFetching, phase]);

  if (phase === 'idle') return null;

  return (
    <div className="fixed top-0 left-0 right-0 z-[200] h-[2px] overflow-hidden pointer-events-none">
      <div
        className={clsx(
          'h-full bg-indigo-500',
          phase === 'loading' ? 'loading-bar-grow' : 'loading-bar-finish',
        )}
      />
    </div>
  );
}

// ── Layout ────────────────────────────────────────────────────────────────────

interface LayoutProps {
  children?: (page: NavPage) => ReactNode;
  routeRenderer?: (route: Route) => ReactNode;
}

export function Layout({ children, routeRenderer }: LayoutProps) {
  const [route, setRoute]             = useState<Route>(currentRoute);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const mainRef = useRef<HTMLElement>(null);

  useEffect(() => {
    const onHashChange = () => {
      setRoute(currentRoute());
      setSidebarOpen(false);
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  // Trigger page-enter animation on every route change.
  useEffect(() => {
    const el = mainRef.current;
    if (!el) return;
    el.classList.remove('page-enter');
    void el.offsetHeight; // force reflow so the animation replays
    el.classList.add('page-enter');
  }, [route]);

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
      {/* Global loading bar — fixed, above everything */}
      <LoadingBar />

      <Sidebar
        current={page}
        onNavigate={navigate}
        mobileOpen={sidebarOpen}
        onMobileClose={() => setSidebarOpen(false)}
      />

      <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
        <TopBar
          breadcrumbs={buildBreadcrumbs(route)}
          onMenuClick={() => setSidebarOpen(v => !v)}
        />

        <main
          ref={mainRef}
          className="flex-1 overflow-hidden bg-gray-50 dark:bg-zinc-950 page-enter"
        >
          {content}
        </main>
      </div>

      <CommandPalette onNavigate={navigate} />
    </div>
  );
}
