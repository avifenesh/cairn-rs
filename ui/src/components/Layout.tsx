import { useState, useEffect, type ReactNode } from 'react';
import { Sidebar, type NavPage } from './Sidebar';
import { TopBar } from './TopBar';
import { CommandPalette } from './CommandPalette';

function readPage(): NavPage {
  const hash = window.location.hash.replace('#', '') as NavPage;
  const valid: NavPage[] = [
    'dashboard', 'runs', 'tasks', 'sessions', 'approvals',
    'providers', 'costs', 'traces', 'memory', 'settings',
  ];
  return valid.includes(hash) ? hash : 'dashboard';
}

export const PAGE_TITLES: Record<NavPage, string> = {
  dashboard: 'Dashboard',
  runs:      'Runs',
  tasks:     'Tasks',
  sessions:  'Sessions',
  approvals: 'Approvals',
  providers: 'Providers',
  costs:     'Costs',
  traces:    'Traces',
  memory:    'Memory',
  settings:  'Settings',
  playground: 'Playground',
};

function PlaceholderPage({ page }: { page: NavPage }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-2 text-zinc-700">
      <span className="text-2xl font-semibold text-zinc-600">
        {PAGE_TITLES[page]}
      </span>
      <p className="text-[13px]">Coming soon.</p>
    </div>
  );
}

interface LayoutProps {
  children?: (page: NavPage) => ReactNode;
}

export function Layout({ children }: LayoutProps) {
  const [page, setPage] = useState<NavPage>(readPage);

  useEffect(() => {
    const onHashChange = () => setPage(readPage());
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, []);

  function navigate(next: NavPage) {
    window.location.hash = next;
    setPage(next);
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-zinc-950 text-zinc-200">
      <Sidebar current={page} onNavigate={navigate} />

      <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
        <TopBar title={PAGE_TITLES[page]} />

        {/* Pages own their own scroll and padding */}
        <main className="flex-1 overflow-hidden bg-zinc-950">
          {children ? children(page) : <PlaceholderPage page={page} />}
        </main>
      </div>

      <CommandPalette onNavigate={navigate} />
    </div>
  );
}
