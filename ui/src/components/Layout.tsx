import { useState, useEffect, type ReactNode } from 'react';
import { Sidebar, type NavPage } from './Sidebar';
import { TopBar } from './TopBar';
import { CommandPalette } from './CommandPalette';

// ── Lightweight hash router ────────────────────────────────────────────────────
// No react-router-dom dependency — reads/writes window.location.hash.

function readPage(): NavPage {
  const hash = window.location.hash.replace('#', '') as NavPage;
  const valid: NavPage[] = [
    'dashboard', 'runs', 'sessions', 'approvals', 'providers', 'costs', 'memory', 'settings',
  ];
  return valid.includes(hash) ? hash : 'dashboard';
}

const PAGE_TITLES: Record<NavPage, string> = {
  dashboard:  'Dashboard',
  runs:       'Runs',
  sessions:   'Sessions',
  approvals:  'Approvals',
  providers:  'Providers',
  costs:      'Costs',
  memory:     'Memory',
  settings:   'Settings',
};

// ── Page placeholder ──────────────────────────────────────────────────────────

function PlaceholderPage({ page }: { page: NavPage }) {
  return (
    <div className="flex flex-col items-center justify-center flex-1 gap-3 text-zinc-600">
      <span className="text-4xl font-bold tracking-tight text-zinc-800">
        {PAGE_TITLES[page]}
      </span>
      <p className="text-sm">This view is coming soon.</p>
    </div>
  );
}

// ── Layout ────────────────────────────────────────────────────────────────────

interface LayoutProps {
  /**
   * Render function for the content area.
   * Receives the current NavPage and returns a ReactNode.
   * Defaults to a "coming soon" placeholder when omitted.
   */
  children?: (page: NavPage) => ReactNode;
}

export function Layout({ children }: LayoutProps) {
  const [page, setPage] = useState<NavPage>(readPage);

  // Keep hash in sync when the user navigates back/forward.
  useEffect(() => {
    function onHashChange() {
      setPage(readPage());
    }
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  function navigate(next: NavPage) {
    window.location.hash = next;
    setPage(next);
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-zinc-950 text-zinc-100">
      {/* Left sidebar */}
      <Sidebar current={page} onNavigate={navigate} />

      {/* Right panel: top bar + content */}
      <div className="flex flex-col flex-1 overflow-hidden">
        <TopBar title={PAGE_TITLES[page]} />

        {/* Content area — RunsPage manages its own scroll; others get a scrollable wrapper */}
        <main className="flex-1 overflow-hidden bg-zinc-950">
          {children ? children(page) : <PlaceholderPage page={page} />}
        </main>
      </div>

      {/* Command palette — Cmd+K / Ctrl+K — mounted at root to overlay everything */}
      <CommandPalette onNavigate={navigate} />
    </div>
  );
}
