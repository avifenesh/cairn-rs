import { useState, useEffect, useRef, type ReactNode } from 'react';
import { useIsFetching } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { Sidebar, type NavPage } from './Sidebar';
import { TopBar } from './TopBar';
import { CommandPalette } from './CommandPalette';
import { ConnectionStatus } from './ConnectionStatus';
import { type BreadcrumbItem } from './Breadcrumb';

// All top-level pages — must match NavPage union in Sidebar.tsx
const VALID_PAGES: NavPage[] = [
  'dashboard', 'workspaces',
  'sessions', 'runs', 'tasks', 'workers', 'orchestration', 'approvals', 'triggers', 'decisions', 'prompts', 'agent-templates',
  'traces', 'memory', 'sources', 'costs', 'cost-calc', 'evals', 'graph', 'audit-log', 'logs', 'metrics',
  'providers', 'plugins', 'skills', 'credentials', 'integrations', 'channels', 'deployment', 'playground', 'test-harness', 'api-docs', 'settings', 'profile',
];

// ── Route descriptor ──────────────────────────────────────────────────────────

/** A parsed hash route: either a named page or a dynamic segment. */
export type Route =
  | { kind: 'page'; page: NavPage }
  | { kind: 'run-detail'; runId: string }
  | { kind: 'session-detail'; sessionId: string }
  | { kind: 'eval-compare'; leftId: string; rightId: string }
  | { kind: 'project-dashboard'; projectId: string }
  | { kind: 'not-found'; hash: string };

export function parseRoute(hash: string): Route {
  const h = hash.replace(/^#/, '');
  if (h.startsWith('run/') && h.length > 4) {
    return { kind: 'run-detail', runId: h.slice(4) };
  }
  if (h.startsWith('session/') && h.length > 8) {
    return { kind: 'session-detail', sessionId: h.slice(8) };
  }
  if (h.startsWith('project/') && h.length > 8) {
    return { kind: 'project-dashboard', projectId: h.slice('project/'.length) };
  }
  if (h.startsWith('eval-compare/')) {
    const parts = h.slice('eval-compare/'.length).split('/');
    if (parts.length >= 2) {
      return { kind: 'eval-compare', leftId: parts[0], rightId: parts[1] };
    }
  }
  // Empty hash → dashboard; known page → page; anything else → 404
  if (h === '') return { kind: 'page', page: 'dashboard' };
  const page = h as NavPage;
  if (VALID_PAGES.includes(page)) return { kind: 'page', page };
  return { kind: 'not-found', hash: h };
}

export function currentRoute(): Route {
  return parseRoute(window.location.hash);
}

// ── Page metadata ─────────────────────────────────────────────────────────────

export const PAGE_TITLES: Record<NavPage, string> = {
  dashboard:          'Dashboard',
  workspaces:         'Workspaces',
  sessions:           'Sessions',
  runs:               'Runs',
  tasks:              'Tasks',
  workers:            'Workers',
  orchestration:      'Orchestration',
  approvals:          'Approvals',
  triggers:           'Triggers',
  decisions:          'Decisions',
  prompts:            'Prompts',
  'agent-templates':  'Agent Templates',
  traces:      'Traces',
  memory:      'Memory',
  sources:     'Sources',
  costs:       'Costs',
  'cost-calc': 'Cost Calculator',
  evals:       'Evaluations',
  graph:       'Knowledge Graph',
  'audit-log': 'Audit Log',
  logs:        'Request Logs',
  metrics:     'API Metrics',
  'api-docs':  'API Reference',
  providers:   'Providers',
  plugins:     'Plugins',
  skills:      'Skills',
  credentials:  'Credentials',
  deployment:   'Deployment Health',
  integrations: 'Integrations',
  channels:    'Channels',
  playground:      'Playground',
  'test-harness':  'Test Harness',
  settings:    'Settings',
  profile:     'Account',
};

const PAGE_GROUP: Partial<Record<NavPage, string>> = {
  workspaces:  'Overview',
  sessions:    'Operations',
  runs:        'Operations',
  tasks:       'Operations',
  workers:         'Operations',
  orchestration:   'Operations',
  approvals:          'Operations',
  triggers:           'Operations',
  decisions:          'Operations',
  prompts:            'Operations',
  'agent-templates':  'Operations',
  traces:      'Observability',
  memory:      'Observability',
  sources:     'Observability',
  costs:       'Observability',
  'cost-calc': 'Observability',
  evals:       'Observability',
  graph:       'Observability',
  'audit-log': 'Observability',
  logs:        'Observability',
  metrics:     'Observability',
  providers:   'Infrastructure',
  plugins:      'Infrastructure',
  skills:       'Infrastructure',
  credentials:  'Infrastructure',
  deployment:   'Infrastructure',
  channels:     'Infrastructure',
  playground:      'Infrastructure',
  'test-harness':  'Infrastructure',
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
  if (route.kind === 'not-found') return [{ label: 'Not Found' }];
  return [];
}

function activePage(route: Route): NavPage {
  if (route.kind === 'run-detail')        return 'runs';
  if (route.kind === 'session-detail')    return 'sessions';
  if (route.kind === 'eval-compare')      return 'evals';
  if (route.kind === 'project-dashboard') return 'dashboard';
  if (route.kind === 'not-found')         return 'dashboard';
  return route.page;
}

function PlaceholderPage({ page }: { page: NavPage }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-2 text-gray-400 dark:text-zinc-600">
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
  onLogout?: () => void;
}

export function Layout({ children, routeRenderer, onLogout }: LayoutProps) {
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
    <div className="flex h-screen w-screen overflow-hidden bg-white dark:bg-zinc-950 text-gray-900 dark:text-zinc-200 print-reflow">
      {/* Skip-to-content for keyboard users */}
      <a href="#main-content" className="skip-to-content no-print">Skip to content</a>
      {/* Global loading bar — fixed, above everything */}
      <LoadingBar />

      {/* Sidebar — hidden when printing */}
      <div className="no-print">
        <Sidebar
          current={page}
          onNavigate={navigate}
          mobileOpen={sidebarOpen}
          onMobileClose={() => setSidebarOpen(false)}
          onLogout={onLogout}
        />
      </div>

      <div className="flex flex-col flex-1 min-w-0 overflow-hidden print-full">
        {/* TopBar — hidden when printing */}
        <div className="no-print">
          <TopBar
            breadcrumbs={buildBreadcrumbs(route)}
            onMenuClick={() => setSidebarOpen(v => !v)}
          />
        </div>

        <main
          id="main-content"
          role="main"
          ref={mainRef}
          className="flex-1 overflow-hidden bg-gray-50 dark:bg-zinc-950 page-enter print-full"
          tabIndex={-1}
          aria-label="Main content"
        >
          {content}
        </main>
      </div>

      <div className="no-print">
        <CommandPalette onNavigate={navigate} />
        <ConnectionStatus />
      </div>
    </div>
  );
}
