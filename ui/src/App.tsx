import { useState } from 'react';
import { Layout } from './components/Layout';
import { ErrorBoundary } from './components/ErrorBoundary';
import { LoginPage } from './pages/LoginPage';
import { ApprovalsPage } from './pages/ApprovalsPage';
import { CostsPage } from './pages/CostsPage';
import { DashboardPage } from './pages/DashboardPage';
import { MemoryPage } from './pages/MemoryPage';
import { ProvidersPage } from './pages/ProvidersPage';
import { RunsPage } from './pages/RunsPage';
import { RunDetailPage } from './pages/RunDetailPage';
import { SessionDetailPage } from './pages/SessionDetailPage';
import { SessionsPage } from './pages/SessionsPage';
import { PlaygroundPage } from './pages/PlaygroundPage';
import { SettingsPage } from './pages/SettingsPage';
import { TasksPage } from './pages/TasksPage';
import { TracesPage } from './pages/TracesPage';
import { AuditLogPage } from './pages/AuditLogPage';
import { getStoredToken } from './lib/api';
import type { NavPage } from './components/Sidebar';
import type { Route } from './components/Layout';

/** Wrap a page element in an ErrorBoundary labelled by its name. */
function Guarded({ name, children }: { name: string; children: React.ReactNode }) {
  return <ErrorBoundary name={name}>{children}</ErrorBoundary>;
}

function renderRoute(route: Route): React.ReactNode {
  if (route.kind === 'run-detail') {
    return <Guarded name="Run Detail"><RunDetailPage runId={route.runId} /></Guarded>;
  }
  if (route.kind === 'session-detail') {
    return <Guarded name="Session Detail"><SessionDetailPage sessionId={route.sessionId} /></Guarded>;
  }

  const page = (route as { kind: 'page'; page: NavPage }).page;

  switch (page) {
    case 'dashboard':  return <Guarded name="Dashboard"><DashboardPage /></Guarded>;
    case 'runs':       return <Guarded name="Runs"><RunsPage /></Guarded>;
    case 'tasks':      return <Guarded name="Tasks"><TasksPage /></Guarded>;
    case 'sessions':   return <Guarded name="Sessions"><SessionsPage /></Guarded>;
    case 'approvals':  return <Guarded name="Approvals"><ApprovalsPage /></Guarded>;
    case 'providers':  return <Guarded name="Providers"><ProvidersPage /></Guarded>;
    case 'memory':     return <Guarded name="Memory"><MemoryPage /></Guarded>;
    case 'costs':      return <Guarded name="Costs"><CostsPage /></Guarded>;
    case 'traces':     return <Guarded name="Traces"><TracesPage /></Guarded>;
    case 'settings':   return <Guarded name="Settings"><SettingsPage /></Guarded>;
    case 'playground': return <Guarded name="Playground"><PlaygroundPage /></Guarded>;
    default:           return null;
  }
}

export default function App() {
  const [authenticated, setAuthenticated] = useState(() => !!getStoredToken());

  if (!authenticated) {
    return <LoginPage onLogin={() => setAuthenticated(true)} />;
  }

  return <Layout routeRenderer={renderRoute} />;
}
