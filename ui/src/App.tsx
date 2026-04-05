import { useState } from 'react';
import { Layout } from './components/Layout';
import { LoginPage } from './pages/LoginPage';
import { ApprovalsPage } from './pages/ApprovalsPage';
import { CostsPage } from './pages/CostsPage';
import { DashboardPage } from './pages/DashboardPage';
import { MemoryPage } from './pages/MemoryPage';
import { ProvidersPage } from './pages/ProvidersPage';
import { RunsPage } from './pages/RunsPage';
import { SessionsPage } from './pages/SessionsPage';
import { SettingsPage } from './pages/SettingsPage';
import { TracesPage } from './pages/TracesPage';
import { getStoredToken } from './lib/api';

export default function App() {
  // Initialise from localStorage so the login persists across page refreshes.
  const [authenticated, setAuthenticated] = useState(() => !!getStoredToken());

  if (!authenticated) {
    return <LoginPage onLogin={() => setAuthenticated(true)} />;
  }

  return (
    <Layout>
      {(page) => {
        switch (page) {
          case 'dashboard': return <DashboardPage />;
          case 'runs':      return <RunsPage />;
          case 'sessions':  return <SessionsPage />;
          case 'approvals': return <ApprovalsPage />;
          case 'providers': return <ProvidersPage />;
          case 'memory':    return <MemoryPage />;
          case 'costs':     return <CostsPage />;
          case 'traces':    return <TracesPage />;
          case 'settings':  return <SettingsPage />;
          default:          return null;
        }
      }}
    </Layout>
  );
}
