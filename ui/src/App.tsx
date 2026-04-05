import { useState, useEffect } from 'react';
import { Loader2 } from 'lucide-react';
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
import { ProfilePage } from './pages/ProfilePage';
import { TasksPage } from './pages/TasksPage';
import { TracesPage } from './pages/TracesPage';
import { EvalsPage } from './pages/EvalsPage';
import { PluginsPage } from './pages/PluginsPage';
import { SourcesPage } from './pages/SourcesPage';
import { CredentialsPage } from './pages/CredentialsPage';
import { ChannelsPage } from './pages/ChannelsPage';
import { LogsPage } from './pages/LogsPage';
import { PromptsPage } from './pages/PromptsPage';
import { AuditLogPage } from './pages/AuditLogPage';
import { GraphPage } from './pages/GraphPage';
import { ApiDocsPage } from './pages/ApiDocsPage';
import { defaultApi, getStoredToken, clearStoredToken, ApiError } from './lib/api';
import type { NavPage } from './components/Sidebar';
import type { Route } from './components/Layout';

// ── Auth state ────────────────────────────────────────────────────────────────

/** 'checking' = existing stored token is being validated against /v1/status */
type AuthState = 'checking' | 'authenticated' | 'unauthenticated';

// ── Route renderer ────────────────────────────────────────────────────────────

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
    case 'dashboard':    return <Guarded name="Dashboard"><DashboardPage /></Guarded>;
    case 'runs':         return <Guarded name="Runs"><RunsPage /></Guarded>;
    case 'tasks':        return <Guarded name="Tasks"><TasksPage /></Guarded>;
    case 'sessions':     return <Guarded name="Sessions"><SessionsPage /></Guarded>;
    case 'approvals':    return <Guarded name="Approvals"><ApprovalsPage /></Guarded>;
    case 'prompts':      return <Guarded name="Prompts"><PromptsPage /></Guarded>;
    case 'providers':    return <Guarded name="Providers"><ProvidersPage /></Guarded>;
    case 'memory':       return <Guarded name="Memory"><MemoryPage /></Guarded>;
    case 'costs':        return <Guarded name="Costs"><CostsPage /></Guarded>;
    case 'traces':       return <Guarded name="Traces"><TracesPage /></Guarded>;
    case 'evals':        return <Guarded name="Evaluations"><EvalsPage /></Guarded>;
    case 'plugins':      return <Guarded name="Plugins"><PluginsPage /></Guarded>;
    case 'sources':      return <Guarded name="Sources"><SourcesPage /></Guarded>;
    case 'credentials':  return <Guarded name="Credentials"><CredentialsPage /></Guarded>;
    case 'channels':     return <Guarded name="Channels"><ChannelsPage /></Guarded>;
    case 'logs':         return <Guarded name="Logs"><LogsPage /></Guarded>;
    case 'graph':        return <Guarded name="Graph"><GraphPage /></Guarded>;
    case 'api-docs':     return <Guarded name="API Docs"><ApiDocsPage /></Guarded>;
    case 'audit-log':    return <Guarded name="Audit Log"><AuditLogPage /></Guarded>;
    case 'settings':     return <Guarded name="Settings"><SettingsPage /></Guarded>;
    case 'profile':      return <Guarded name="Account"><ProfilePage /></Guarded>;
    case 'playground':   return <Guarded name="Playground"><PlaygroundPage /></Guarded>;
    default:             return null;
  }
}

// ── Validating screen ─────────────────────────────────────────────────────────

function ValidatingScreen() {
  return (
    <div className="flex h-screen w-screen items-center justify-center bg-zinc-950">
      <div className="flex flex-col items-center gap-5">
        <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-indigo-600 shadow-lg shadow-indigo-600/30">
          <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
            <rect x="2"  y="2"  width="6" height="6" rx="1.5" fill="white" opacity="0.9"/>
            <rect x="10" y="2"  width="6" height="6" rx="1.5" fill="white" opacity="0.55"/>
            <rect x="2"  y="10" width="6" height="6" rx="1.5" fill="white" opacity="0.55"/>
            <rect x="10" y="10" width="6" height="6" rx="1.5" fill="white" opacity="0.9"/>
          </svg>
        </div>
        <div className="flex items-center gap-2 text-zinc-600">
          <Loader2 size={14} className="animate-spin" />
          <span className="text-[13px]">Verifying session…</span>
        </div>
      </div>
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

export default function App() {
  const [authState, setAuthState] = useState<AuthState>(() =>
    getStoredToken() ? 'checking' : 'unauthenticated'
  );

  // Validate stored token on mount by calling GET /v1/status.
  // 401  → token invalid/expired: clear it, show login.
  // Other error (network, 5xx) → assume token is valid; don't log the operator
  //   out just because the server had a momentary hiccup.
  useEffect(() => {
    if (authState !== 'checking') return;

    defaultApi.getStatus()
      .then(() => setAuthState('authenticated'))
      .catch((err: unknown) => {
        if (err instanceof ApiError && err.status === 401) {
          clearStoredToken();
          setAuthState('unauthenticated');
        } else {
          setAuthState('authenticated');
        }
      });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  function handleLogout() {
    clearStoredToken();
    setAuthState('unauthenticated');
  }

  if (authState === 'checking') {
    return <ValidatingScreen />;
  }

  if (authState === 'unauthenticated') {
    return <LoginPage onLogin={() => setAuthState('authenticated')} />;
  }

  return <Layout routeRenderer={renderRoute} onLogout={handleLogout} />;
}
