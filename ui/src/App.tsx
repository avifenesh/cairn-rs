import { useState, useEffect, lazy, Suspense } from 'react';
import { Loader2 } from 'lucide-react';
import { Layout } from './components/Layout';
import { ErrorBoundary } from './components/ErrorBoundary';
import { RequestLogProvider } from './components/RequestLogContext';
import { LoginPage } from './pages/LoginPage';
// ── Eagerly-loaded pages (always needed on first paint) ───────────────────────
import { DashboardPage } from './pages/DashboardPage';
import { RunsPage } from './pages/RunsPage';
import { SessionsPage } from './pages/SessionsPage';
import { TasksPage } from './pages/TasksPage';
import { ApprovalsPage } from './pages/ApprovalsPage';
import { EvalsPage } from './pages/EvalsPage';
// ── Lazily-loaded pages (loaded on first navigation) ─────────────────────────
const RunDetailPage      = lazy(() => import('./pages/RunDetailPage').then(m => ({ default: m.RunDetailPage })));
const SessionDetailPage  = lazy(() => import('./pages/SessionDetailPage').then(m => ({ default: m.SessionDetailPage })));
const EvalComparisonPage = lazy(() => import('./pages/EvalComparisonPage').then(m => ({ default: m.EvalComparisonPage })));
const PlaygroundPage     = lazy(() => import('./pages/PlaygroundPage').then(m => ({ default: m.PlaygroundPage })));
const WorkersPage        = lazy(() => import('./pages/WorkersPage').then(m => ({ default: m.WorkersPage })));
const TestHarnessPage    = lazy(() => import('./pages/TestHarnessPage').then(m => ({ default: m.TestHarnessPage })));
const MetricsPage        = lazy(() => import('./pages/MetricsPage').then(m => ({ default: m.MetricsPage })));
const OrchestrationPage  = lazy(() => import('./pages/OrchestrationPage').then(m => ({ default: m.OrchestrationPage })));
const DeploymentPage     = lazy(() => import('./pages/DeploymentPage').then(m => ({ default: m.DeploymentPage })));
const ApiDocsPage        = lazy(() => import('./pages/ApiDocsPage').then(m => ({ default: m.ApiDocsPage })));
const GraphPage          = lazy(() => import('./pages/GraphPage').then(m => ({ default: m.GraphPage })));
const PromptsPage        = lazy(() => import('./pages/PromptsPage').then(m => ({ default: m.PromptsPage })));
const TracesPage         = lazy(() => import('./pages/TracesPage').then(m => ({ default: m.TracesPage })));
const CostsPage          = lazy(() => import('./pages/CostsPage').then(m => ({ default: m.CostsPage })));
const CostCalculatorPage = lazy(() => import('./pages/CostCalculatorPage').then(m => ({ default: m.CostCalculatorPage })));
const MemoryPage         = lazy(() => import('./pages/MemoryPage').then(m => ({ default: m.MemoryPage })));
const ProvidersPage      = lazy(() => import('./pages/ProvidersPage').then(m => ({ default: m.ProvidersPage })));
const PluginsPage        = lazy(() => import('./pages/PluginsPage').then(m => ({ default: m.PluginsPage })));
const SourcesPage        = lazy(() => import('./pages/SourcesPage').then(m => ({ default: m.SourcesPage })));
const CredentialsPage    = lazy(() => import('./pages/CredentialsPage').then(m => ({ default: m.CredentialsPage })));
const ChannelsPage       = lazy(() => import('./pages/ChannelsPage').then(m => ({ default: m.ChannelsPage })));
const LogsPage           = lazy(() => import('./pages/LogsPage').then(m => ({ default: m.LogsPage })));
const AuditLogPage       = lazy(() => import('./pages/AuditLogPage').then(m => ({ default: m.AuditLogPage })));
const SettingsPage       = lazy(() => import('./pages/SettingsPage').then(m => ({ default: m.SettingsPage })));
const ProfilePage        = lazy(() => import('./pages/ProfilePage').then(m => ({ default: m.ProfilePage })));
import { NotFoundPage } from './pages/NotFoundPage';

import { defaultApi, getStoredToken, clearStoredToken, ApiError } from './lib/api';
import type { NavPage } from './components/Sidebar';
import type { Route } from './components/Layout';

// ── Auth state ────────────────────────────────────────────────────────────────

/** 'checking' = existing stored token is being validated against /v1/status */
type AuthState = 'checking' | 'authenticated' | 'unauthenticated';

// ── Page loader fallback ──────────────────────────────────────────────────────

function PageLoader() {
  return (
    <div className="flex h-full items-center justify-center bg-zinc-950">
      <Loader2 size={16} className="animate-spin text-zinc-600" />
    </div>
  );
}

// ── Route renderer ────────────────────────────────────────────────────────────

function Guarded({ name, children }: { name: string; children: React.ReactNode }) {
  return <ErrorBoundary name={name}>{children}</ErrorBoundary>;
}

function renderRoute(route: Route): React.ReactNode {
  if (route.kind === 'not-found') {
    return <NotFoundPage />;
  }
  if (route.kind === 'run-detail') {
    return (
      <Guarded name="Run Detail">
        <Suspense fallback={<PageLoader />}>
          <RunDetailPage runId={route.runId} />
        </Suspense>
      </Guarded>
    );
  }
  if (route.kind === 'session-detail') {
    return (
      <Guarded name="Session Detail">
        <Suspense fallback={<PageLoader />}>
          <SessionDetailPage sessionId={route.sessionId} />
        </Suspense>
      </Guarded>
    );
  }
  if (route.kind === 'eval-compare') {
    return (
      <Guarded name="Eval Comparison">
        <Suspense fallback={<PageLoader />}>
          <EvalComparisonPage leftId={route.leftId} rightId={route.rightId} />
        </Suspense>
      </Guarded>
    );
  }

  const page = (route as { kind: 'page'; page: NavPage }).page;

  // Eager pages — no Suspense needed.
  switch (page) {
    case 'dashboard':  return <Guarded name="Dashboard"><DashboardPage /></Guarded>;
    case 'runs':       return <Guarded name="Runs"><RunsPage /></Guarded>;
    case 'tasks':      return <Guarded name="Tasks"><TasksPage /></Guarded>;
    case 'sessions':   return <Guarded name="Sessions"><SessionsPage /></Guarded>;
    case 'approvals':  return <Guarded name="Approvals"><ApprovalsPage /></Guarded>;
    case 'evals':      return <Guarded name="Evaluations"><EvalsPage /></Guarded>;
    default: break;
  }

  // Lazy pages — wrapped in Suspense.
  const lazy_page = (() => {
    switch (page) {
      case 'workers':         return <WorkersPage />;
      case 'orchestration': return <OrchestrationPage />;
      case 'deployment':  return <DeploymentPage />;
      case 'prompts':     return <PromptsPage />;
      case 'providers':   return <ProvidersPage />;
      case 'memory':      return <MemoryPage />;
      case 'costs':       return <CostsPage />;
      case 'cost-calc':   return <CostCalculatorPage />;
      case 'traces':      return <TracesPage />;
      case 'plugins':     return <PluginsPage />;
      case 'sources':     return <SourcesPage />;
      case 'credentials': return <CredentialsPage />;
      case 'channels':    return <ChannelsPage />;
      case 'logs':        return <LogsPage />;
      case 'metrics':        return <MetricsPage />;
      case 'test-harness':  return <TestHarnessPage />;
      case 'graph':       return <GraphPage />;
      case 'api-docs':    return <ApiDocsPage />;
      case 'audit-log':   return <AuditLogPage />;
      case 'settings':    return <SettingsPage />;
      case 'profile':     return <ProfilePage />;
      case 'playground':  return <PlaygroundPage />;
      default:            return <NotFoundPage />;
    }
  })();

  if (lazy_page === null) return null;
  const label = page.charAt(0).toUpperCase() + page.slice(1).replace(/-/g, ' ');
  return (
    <Guarded name={label}>
      <Suspense fallback={<PageLoader />}>{lazy_page}</Suspense>
    </Guarded>
  );
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

  return (
    <RequestLogProvider>
      <Layout routeRenderer={renderRoute} onLogout={handleLogout} />
    </RequestLogProvider>
  );
}
