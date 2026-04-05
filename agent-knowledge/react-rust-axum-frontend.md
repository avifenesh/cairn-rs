# Building a Production React Frontend for Rust/Axum Backends

**Research date:** 2026-04-05
**Context:** cairn-rs has an axum backend with SSE streaming (16 named event types, `lastEventId` replay), bearer token auth, and REST JSON endpoints. This guide covers building the operator dashboard frontend.

---

## Table of Contents

1. [Key Dependencies and Versions](#1-key-dependencies-and-versions)
2. [Project Structure](#2-project-structure)
3. [Project Scaffolding](#3-project-scaffolding)
4. [Vite Configuration](#4-vite-configuration)
5. [shadcn/ui and Tailwind CSS Setup](#5-shadcnui-and-tailwind-css-setup)
6. [API Client Setup](#6-api-client-setup)
7. [Auth Flow](#7-auth-flow)
8. [TanStack Query for Server State](#8-tanstack-query-for-server-state)
9. [SSE Event Consumption](#9-sse-event-consumption)
10. [Dark Mode Implementation](#10-dark-mode-implementation)
11. [Serving Static Files from Axum](#11-serving-static-files-from-axum)
12. [Build and Deployment](#12-build-and-deployment)
13. [Sources](#13-sources)

---

## 1. Key Dependencies and Versions

### Frontend (package.json)

```json
{
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "react-router": "^7.0.0",
    "@tanstack/react-query": "^5.62.0",
    "lucide-react": "^0.460.0",
    "recharts": "^2.15.0",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "tailwind-merge": "^2.6.0"
  },
  "devDependencies": {
    "vite": "^8.0.0",
    "@vitejs/plugin-react": "^4.4.0",
    "typescript": "^5.7.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@types/node": "^22.0.0",
    "tailwindcss": "^4.0.0",
    "@tailwindcss/vite": "^4.0.0"
  }
}
```

### Rust Backend (Cargo.toml additions)

```toml
[dependencies]
# Existing axum stack
axum = { version = "0.8", features = ["macros"] }
tower-http = { version = "0.6", features = ["cors", "fs"] }

# Frontend embedding
rust-embed = { version = "8", features = ["axum"] }
mime_guess = "2"
```

### Version Notes

- **Vite 8** (released 2025): Uses Rolldown bundler, requires Node.js >= 20.19 or >= 22.12
- **React 19**: Stable since Dec 2024. `use()` API for Suspense-based data fetching
- **TanStack Query v5**: Breaking changes from v4 -- `cacheTime` renamed to `gcTime`, query keys must be arrays
- **Tailwind CSS v4**: CSS-native cascade layers, no PostCSS config, `@tailwindcss/vite` plugin
- **shadcn/ui**: Not a library you install. Components are copied into your project via CLI

---

## 2. Project Structure

```
cairn-rs/
  crates/
    cairn-app/
      src/
        main.rs              # axum server + embedded frontend
      frontend/              # Vite + React project
        public/
          favicon.svg
        src/
          components/
            ui/              # shadcn/ui components (Button, Card, Table, etc.)
            dashboard/
              task-list.tsx
              approval-queue.tsx
              agent-feed.tsx
              cost-chart.tsx
            layout/
              sidebar.tsx
              header.tsx
              theme-toggle.tsx
          hooks/
            use-sse.ts       # SSE subscription hook
            use-auth.ts      # Auth state management
          lib/
            api-client.ts    # Typed fetch wrapper with auth
            sse-store.ts     # External SSE state store
            query-keys.ts    # TanStack Query key factories
          types/
            api.ts           # API response types (mirror Rust structs)
            sse.ts           # SSE event payload types
          App.tsx
          main.tsx
          index.css          # Tailwind entry + theme variables
          vite-env.d.ts      # Env var type declarations
        dist/                # Vite build output (git-ignored, embedded by rust-embed)
        index.html
        package.json
        tsconfig.json
        tsconfig.app.json
        vite.config.ts
        components.json      # shadcn/ui config
```

### Key Conventions

- `src/components/ui/` is owned by shadcn/ui CLI -- do not manually create files here
- `src/lib/` holds pure logic (no React imports except hooks)
- `src/types/` mirrors Rust domain types with `camelCase` field names
- `src/hooks/` contains custom hooks that compose React primitives

---

## 3. Project Scaffolding

### Step-by-step setup

```bash
# From the crate directory
cd crates/cairn-app
mkdir frontend && cd frontend

# Scaffold Vite + React + TypeScript
pnpm create vite@latest . -- --template react-ts

# Install core dependencies
pnpm add react-router @tanstack/react-query lucide-react recharts \
  class-variance-authority clsx tailwind-merge

# Install Tailwind CSS v4 (Vite plugin approach)
pnpm add tailwindcss @tailwindcss/vite

# Install dev dependencies
pnpm add -D @types/node

# Initialize shadcn/ui
pnpm dlx shadcn@latest init

# Add initial shadcn components
pnpm dlx shadcn@latest add button card table badge dialog \
  dropdown-menu tabs sidebar tooltip sheet command select
```

### TypeScript Path Aliases

**tsconfig.json:**
```json
{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["./src/*"]
    }
  }
}
```

**tsconfig.app.json:** Add the same `baseUrl` and `paths` configuration.

---

## 4. Vite Configuration

### vite.config.ts

```typescript
import path from "path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    port: 5173,
    proxy: {
      // Proxy all API and SSE requests to the axum backend
      "/v1": {
        target: "http://localhost:3000",
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    // Produce chunk hashes for cache-busting
    rollupOptions: {
      output: {
        manualChunks: {
          vendor: ["react", "react-dom"],
          query: ["@tanstack/react-query"],
          charts: ["recharts"],
        },
      },
    },
  },
});
```

### Key Proxy Behavior

- Requests to `http://localhost:5173/v1/tasks` are forwarded to `http://localhost:3000/v1/tasks`
- SSE connection to `http://localhost:5173/v1/stream` is proxied to `http://localhost:3000/v1/stream`
- No CORS issues during development because the browser sees one origin
- In production, frontend is embedded in the same binary -- no proxy needed

### Environment Variables

**`.env` (all modes):**
```bash
VITE_APP_TITLE=Cairn Dashboard
```

**`.env.development`:**
```bash
VITE_API_BASE_URL=http://localhost:3000
```

**`.env.production`:**
```bash
VITE_API_BASE_URL=
# Empty = same origin (embedded in binary)
```

**Type declarations (`src/vite-env.d.ts`):**
```typescript
/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_APP_TITLE: string;
  readonly VITE_API_BASE_URL: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
```

Only variables prefixed with `VITE_` are exposed to client code. Non-prefixed variables (like `DB_PASSWORD`) remain hidden.

---

## 5. shadcn/ui and Tailwind CSS Setup

### Tailwind CSS v4 Entry Point

**`src/index.css`:**
```css
@import "tailwindcss";

/* shadcn/ui theme variables */
@custom-variant dark (&:where(.dark, .dark *));

@theme inline {
  /* Base radius for all components */
  --radius: 0.625rem;

  /* Light mode colors (oklch for perceptual uniformity) */
  --color-background: oklch(1 0 0);
  --color-foreground: oklch(0.145 0.015 285.82);
  --color-card: oklch(1 0 0);
  --color-card-foreground: oklch(0.145 0.015 285.82);
  --color-primary: oklch(0.205 0.015 285.82);
  --color-primary-foreground: oklch(0.985 0 0);
  --color-secondary: oklch(0.97 0.005 285.82);
  --color-secondary-foreground: oklch(0.205 0.015 285.82);
  --color-muted: oklch(0.97 0.005 285.82);
  --color-muted-foreground: oklch(0.56 0.015 285.82);
  --color-accent: oklch(0.97 0.005 285.82);
  --color-accent-foreground: oklch(0.205 0.015 285.82);
  --color-destructive: oklch(0.577 0.245 27.33);
  --color-border: oklch(0.922 0.005 285.82);
  --color-input: oklch(0.922 0.005 285.82);
  --color-ring: oklch(0.708 0.015 285.82);

  /* Status colors for agent dashboard */
  --color-success: oklch(0.65 0.2 145);
  --color-warning: oklch(0.75 0.18 80);
  --color-info: oklch(0.65 0.15 250);
}

/* Dark mode overrides */
.dark {
  --color-background: oklch(0.145 0.015 285.82);
  --color-foreground: oklch(0.985 0 0);
  --color-card: oklch(0.205 0.015 285.82);
  --color-card-foreground: oklch(0.985 0 0);
  --color-primary: oklch(0.985 0 0);
  --color-primary-foreground: oklch(0.205 0.015 285.82);
  --color-secondary: oklch(0.269 0.015 285.82);
  --color-secondary-foreground: oklch(0.985 0 0);
  --color-muted: oklch(0.269 0.015 285.82);
  --color-muted-foreground: oklch(0.708 0.015 285.82);
  --color-accent: oklch(0.269 0.015 285.82);
  --color-accent-foreground: oklch(0.985 0 0);
  --color-destructive: oklch(0.577 0.245 27.33);
  --color-border: oklch(0.269 0.015 285.82);
  --color-input: oklch(0.269 0.015 285.82);
  --color-ring: oklch(0.439 0.015 285.82);

  --color-success: oklch(0.7 0.2 145);
  --color-warning: oklch(0.8 0.18 80);
  --color-info: oklch(0.7 0.15 250);
}
```

### Adding shadcn/ui Components

```bash
# Individual components
pnpm dlx shadcn@latest add button
pnpm dlx shadcn@latest add card
pnpm dlx shadcn@latest add data-table

# Components land in src/components/ui/
# You own them -- edit freely
```

### Usage Example

```tsx
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

function TaskCard({ task }: { task: TaskRecord }) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm font-medium">{task.title}</CardTitle>
        <Badge variant={task.state === "completed" ? "default" : "secondary"}>
          {task.state}
        </Badge>
      </CardHeader>
      <CardContent>
        <p className="text-xs text-muted-foreground">{task.description}</p>
      </CardContent>
    </Card>
  );
}
```

---

## 6. API Client Setup

### Typed Fetch Wrapper

**`src/lib/api-client.ts`:**
```typescript
type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

interface RequestOptions {
  method?: HttpMethod;
  body?: unknown;
  headers?: Record<string, string>;
  signal?: AbortSignal;
}

class ApiError extends Error {
  constructor(
    public status: number,
    public statusText: string,
    public body?: unknown,
  ) {
    super(`${status} ${statusText}`);
    this.name = "ApiError";
  }
}

// Token storage -- see Auth Flow section for full implementation
let accessToken: string | null = null;

export function setAccessToken(token: string | null) {
  accessToken = token;
}

export function getAccessToken(): string | null {
  return accessToken;
}

/**
 * Base fetch wrapper with auth, JSON handling, and error normalization.
 */
async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", body, headers = {}, signal } = options;

  const baseUrl = import.meta.env.VITE_API_BASE_URL || "";
  const url = `${baseUrl}${path}`;

  const requestHeaders: Record<string, string> = {
    "Content-Type": "application/json",
    ...headers,
  };

  // Attach bearer token if available
  if (accessToken) {
    requestHeaders["Authorization"] = `Bearer ${accessToken}`;
  }

  const response = await fetch(url, {
    method,
    headers: requestHeaders,
    body: body ? JSON.stringify(body) : undefined,
    signal,
  });

  // Handle 401 -- token expired or invalid
  if (response.status === 401) {
    setAccessToken(null);
    // Redirect to login or trigger refresh
    window.dispatchEvent(new CustomEvent("auth:unauthorized"));
    throw new ApiError(401, "Unauthorized");
  }

  if (!response.ok) {
    const errorBody = await response.json().catch(() => undefined);
    throw new ApiError(response.status, response.statusText, errorBody);
  }

  // Handle 204 No Content
  if (response.status === 204) {
    return undefined as T;
  }

  return response.json() as Promise<T>;
}

// Convenience methods
export const api = {
  get: <T>(path: string, signal?: AbortSignal) =>
    request<T>(path, { signal }),

  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "POST", body }),

  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "PUT", body }),

  patch: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: "PATCH", body }),

  delete: <T>(path: string) =>
    request<T>(path, { method: "DELETE" }),
};
```

### API Types (mirror Rust domain structs)

**`src/types/api.ts`:**
```typescript
// Mirrors cairn_store::projections::TaskRecord
export interface TaskRecord {
  taskId: string;
  project: ProjectKey;
  parentRunId: string | null;
  parentTaskId: string | null;
  state: TaskState;
  promptReleaseId: string | null;
  failureClass: string | null;
  pauseReason: string | null;
  resumeTrigger: string | null;
  retryCount: number;
  leaseOwner: string | null;
  leaseExpiresAt: number | null;
  title: string | null;
  description: string | null;
  version: number;
  createdAt: string;
  updatedAt: string;
}

export type TaskState =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "paused"
  | "cancelled";

export interface ProjectKey {
  tenantId: string;
  workspaceId: string;
  projectId: string;
}

export interface ApprovalRecord {
  approvalId: string;
  project: ProjectKey;
  runId: string | null;
  taskId: string | null;
  requirement: "required" | "optional";
  decision: "approved" | "rejected" | null;
  title: string | null;
  description: string | null;
  version: number;
  createdAt: string;
  updatedAt: string;
}

// List response envelope (matches cairn_api::http::ListResponse)
export interface ListResponse<T> {
  items: T[];
  total: number;
  offset: number;
  limit: number;
}

export interface ListQuery {
  limit?: number;
  offset?: number;
  status?: string;
}
```

---

## 7. Auth Flow

### Bearer Token Strategy

cairn-rs uses a `ServiceTokenRegistry` that maps bearer tokens to `AuthPrincipal` values. The frontend stores the token in memory (not localStorage for XSS protection) and optionally persists a refresh mechanism via httpOnly cookies.

### Token Storage Hierarchy (security vs. convenience)

| Storage | XSS Safe | CSRF Safe | Survives Refresh | Recommendation |
|---------|----------|-----------|-------------------|----------------|
| In-memory variable | Yes | Yes | No | Best for SPAs with short sessions |
| httpOnly cookie | Yes | Needs CSRF token | Yes | Best for persistent sessions |
| localStorage | No | Yes | Yes | Avoid for sensitive tokens |
| sessionStorage | No | Yes | No | Slightly better than localStorage |

### Recommended: In-Memory + httpOnly Cookie Refresh

```
1. User submits credentials via login form
2. Backend sets httpOnly cookie with refresh token
3. Backend returns access token in JSON response body
4. Frontend stores access token in memory (module-level variable)
5. All API requests attach: Authorization: Bearer <access_token>
6. On 401 response, frontend calls POST /v1/auth/refresh (cookie sent automatically)
7. Backend validates refresh cookie, returns new access token
8. Frontend updates in-memory token, retries failed request
9. On page refresh, frontend calls /v1/auth/refresh to get new access token
```

### Auth Hook

**`src/hooks/use-auth.ts`:**
```typescript
import { useState, useCallback, useEffect } from "react";
import { api, setAccessToken, getAccessToken } from "@/lib/api-client";

interface AuthState {
  isAuthenticated: boolean;
  isLoading: boolean;
  principal: AuthPrincipal | null;
}

interface AuthPrincipal {
  kind: "operator" | "service_account" | "system";
  operatorId?: string;
  name?: string;
  tenant: string;
}

interface LoginResponse {
  accessToken: string;
  principal: AuthPrincipal;
  expiresIn: number;
}

export function useAuth() {
  const [state, setState] = useState<AuthState>({
    isAuthenticated: !!getAccessToken(),
    isLoading: true,
    principal: null,
  });

  // Try to refresh on mount (recovers session after page reload)
  useEffect(() => {
    let cancelled = false;

    async function tryRefresh() {
      try {
        const data = await api.post<LoginResponse>("/v1/auth/refresh");
        if (!cancelled) {
          setAccessToken(data.accessToken);
          setState({
            isAuthenticated: true,
            isLoading: false,
            principal: data.principal,
          });
        }
      } catch {
        if (!cancelled) {
          setState({ isAuthenticated: false, isLoading: false, principal: null });
        }
      }
    }

    tryRefresh();
    return () => { cancelled = true; };
  }, []);

  // Listen for 401 events from the API client
  useEffect(() => {
    const handler = () => {
      setState({ isAuthenticated: false, isLoading: false, principal: null });
    };
    window.addEventListener("auth:unauthorized", handler);
    return () => window.removeEventListener("auth:unauthorized", handler);
  }, []);

  const login = useCallback(async (token: string) => {
    setAccessToken(token);
    try {
      // Validate token by fetching principal info
      const data = await api.get<{ principal: AuthPrincipal }>("/v1/auth/me");
      setState({
        isAuthenticated: true,
        isLoading: false,
        principal: data.principal,
      });
    } catch {
      setAccessToken(null);
      throw new Error("Invalid token");
    }
  }, []);

  const logout = useCallback(() => {
    setAccessToken(null);
    setState({ isAuthenticated: false, isLoading: false, principal: null });
  }, []);

  return { ...state, login, logout };
}
```

### Protected Route Pattern

```tsx
import { Navigate, Outlet } from "react-router";
import { useAuth } from "@/hooks/use-auth";

function ProtectedLayout() {
  const { isAuthenticated, isLoading } = useAuth();

  if (isLoading) {
    return <div className="flex h-screen items-center justify-center">Loading...</div>;
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" replace />;
  }

  return <Outlet />;
}
```

---

## 8. TanStack Query for Server State

### Setup

**`src/main.tsx`:**
```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";
import { BrowserRouter } from "react-router";
import { ThemeProvider } from "@/components/theme-provider";
import App from "./App";
import "./index.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 30,       // 30 seconds -- SSE handles real-time
      gcTime: 1000 * 60 * 5,      // 5 minutes garbage collection
      retry: 1,                    // Retry once on failure
      refetchOnWindowFocus: false, // SSE handles freshness
    },
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <ThemeProvider defaultTheme="dark" storageKey="cairn-ui-theme">
          <App />
        </ThemeProvider>
      </BrowserRouter>
      <ReactQueryDevtools initialIsOpen={false} />
    </QueryClientProvider>
  </StrictMode>,
);
```

### Query Key Factory

Consistent key structure makes invalidation predictable.

**`src/lib/query-keys.ts`:**
```typescript
export const queryKeys = {
  // Tasks
  tasks: {
    all: ["tasks"] as const,
    lists: () => [...queryKeys.tasks.all, "list"] as const,
    list: (filters: { status?: string; limit?: number; offset?: number }) =>
      [...queryKeys.tasks.lists(), filters] as const,
    details: () => [...queryKeys.tasks.all, "detail"] as const,
    detail: (id: string) => [...queryKeys.tasks.details(), id] as const,
  },

  // Approvals
  approvals: {
    all: ["approvals"] as const,
    lists: () => [...queryKeys.approvals.all, "list"] as const,
    list: (filters: { status?: string }) =>
      [...queryKeys.approvals.lists(), filters] as const,
  },

  // Sessions
  sessions: {
    all: ["sessions"] as const,
    lists: () => [...queryKeys.sessions.all, "list"] as const,
    detail: (id: string) => [...queryKeys.sessions.all, "detail", id] as const,
  },

  // Runs
  runs: {
    bySession: (sessionId: string) => ["runs", "session", sessionId] as const,
  },
} as const;
```

### Query Hooks

**`src/hooks/use-tasks.ts`:**
```typescript
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { queryKeys } from "@/lib/query-keys";
import type { TaskRecord, ListResponse, ListQuery } from "@/types/api";

export function useTasks(filters: ListQuery = {}) {
  return useQuery({
    queryKey: queryKeys.tasks.list(filters),
    queryFn: ({ signal }) => {
      const params = new URLSearchParams();
      if (filters.limit) params.set("limit", String(filters.limit));
      if (filters.offset) params.set("offset", String(filters.offset));
      if (filters.status) params.set("status", filters.status);
      const qs = params.toString();
      return api.get<ListResponse<TaskRecord>>(
        `/v1/tasks${qs ? `?${qs}` : ""}`,
        signal,
      );
    },
  });
}

export function useTask(taskId: string) {
  return useQuery({
    queryKey: queryKeys.tasks.detail(taskId),
    queryFn: () => api.get<TaskRecord>(`/v1/tasks/${taskId}`),
    enabled: !!taskId,
  });
}

export function useCancelTask() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (taskId: string) =>
      api.post(`/v1/tasks/${taskId}/cancel`),
    onSuccess: (_data, taskId) => {
      // Invalidate both the specific task and all task lists
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(taskId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() });
    },
  });
}
```

**`src/hooks/use-approvals.ts`:**
```typescript
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { queryKeys } from "@/lib/query-keys";
import type { ApprovalRecord, ListResponse } from "@/types/api";

export function useApprovals(filters: { status?: string } = {}) {
  return useQuery({
    queryKey: queryKeys.approvals.list(filters),
    queryFn: ({ signal }) => {
      const params = new URLSearchParams();
      if (filters.status) params.set("status", filters.status);
      const qs = params.toString();
      return api.get<ListResponse<ApprovalRecord>>(
        `/v1/approvals${qs ? `?${qs}` : ""}`,
        signal,
      );
    },
  });
}

export function useResolveApproval() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      approvalId,
      decision,
    }: {
      approvalId: string;
      decision: "approved" | "rejected";
    }) => api.post(`/v1/approvals/${approvalId}/resolve`, { decision }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.approvals.all });
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all });
    },
  });
}
```

---

## 9. SSE Event Consumption

### The EventSource Limitation

The browser `EventSource` API does not support custom headers (including `Authorization`). Three approaches to authenticate SSE connections:

| Approach | Security | Complexity |
|----------|----------|------------|
| Token in query param (`/v1/stream?token=xxx`) | Acceptable for internal tools | Low |
| Cookie-based auth (httpOnly) | Best | Medium |
| `fetch()` with ReadableStream | Full header control | High |

For an internal operator dashboard, **query param token** is the simplest. For production with external users, use **cookie-based auth**.

### SSE Store (External Store Pattern)

This uses `useSyncExternalStore` which is concurrent-mode safe and avoids the tearing issues that `useEffect` + `useState` can produce.

**`src/lib/sse-store.ts`:**
```typescript
import type { TaskRecord, ApprovalRecord } from "@/types/api";

// Event payload types matching cairn-rs SseFrame.data shapes
export interface SseTaskUpdate {
  task: TaskRecord;
}

export interface SseApprovalRequired {
  approval: ApprovalRecord;
}

export interface SseReadyEvent {
  clientId: string;
}

export interface SseAgentProgress {
  runId: string;
  message: string;
  timestamp: string;
}

export type ConnectionStatus = "connecting" | "connected" | "disconnected" | "error";

interface SseState {
  tasks: Map<string, TaskRecord>;
  approvals: Map<string, ApprovalRecord>;
  agentEvents: SseAgentProgress[];
  connectionStatus: ConnectionStatus;
  clientId: string | null;
  lastEventId: string | null;
}

// Immutable snapshots for React consumption
interface SseSnapshot {
  tasks: TaskRecord[];
  approvals: ApprovalRecord[];
  agentEvents: SseAgentProgress[];
  connectionStatus: ConnectionStatus;
  clientId: string | null;
}

class SseStore {
  private listeners = new Set<() => void>();
  private state: SseState = {
    tasks: new Map(),
    approvals: new Map(),
    agentEvents: [],
    connectionStatus: "disconnected",
    clientId: null,
    lastEventId: null,
  };
  private snapshot: SseSnapshot = this.buildSnapshot();
  private eventSource: EventSource | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectAttempts = 0;
  private maxReconnectAttempts = 10;

  connect(baseUrl: string = "", token?: string) {
    this.disconnect();

    let url = `${baseUrl}/v1/stream`;
    const params = new URLSearchParams();
    if (this.state.lastEventId) {
      params.set("lastEventId", this.state.lastEventId);
    }
    if (token) {
      params.set("token", token);
    }
    const qs = params.toString();
    if (qs) url += `?${qs}`;

    this.updateState({ connectionStatus: "connecting" });

    const es = new EventSource(url);
    this.eventSource = es;

    es.addEventListener("open", () => {
      this.reconnectAttempts = 0;
      this.updateState({ connectionStatus: "connected" });
    });

    // Named events matching cairn-rs SseEventName variants
    es.addEventListener("ready", (e: MessageEvent) => {
      const data: SseReadyEvent = JSON.parse(e.data);
      this.updateState({ clientId: data.clientId });
      if (e.lastEventId) {
        this.state.lastEventId = e.lastEventId;
      }
    });

    es.addEventListener("task_update", (e: MessageEvent) => {
      const data: SseTaskUpdate = JSON.parse(e.data);
      this.state.tasks.set(data.task.taskId, data.task);
      if (e.lastEventId) {
        this.state.lastEventId = e.lastEventId;
      }
      this.emitChange();
    });

    es.addEventListener("approval_required", (e: MessageEvent) => {
      const data: SseApprovalRequired = JSON.parse(e.data);
      this.state.approvals.set(data.approval.approvalId, data.approval);
      if (e.lastEventId) {
        this.state.lastEventId = e.lastEventId;
      }
      this.emitChange();
    });

    es.addEventListener("agent_progress", (e: MessageEvent) => {
      const data: SseAgentProgress = JSON.parse(e.data);
      // Keep last 200 events to bound memory
      this.state.agentEvents = [
        ...this.state.agentEvents.slice(-199),
        data,
      ];
      if (e.lastEventId) {
        this.state.lastEventId = e.lastEventId;
      }
      this.emitChange();
    });

    es.addEventListener("error", () => {
      if (es.readyState === EventSource.CLOSED) {
        this.updateState({ connectionStatus: "error" });
        this.scheduleReconnect(baseUrl, token);
      }
      // readyState === CONNECTING means browser is auto-reconnecting
    });
  }

  disconnect() {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
    this.updateState({ connectionStatus: "disconnected" });
  }

  private scheduleReconnect(baseUrl: string, token?: string) {
    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      return;
    }
    // Exponential backoff: 1s, 2s, 4s, 8s, ... capped at 30s
    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30_000);
    this.reconnectAttempts++;

    this.reconnectTimer = setTimeout(() => {
      this.connect(baseUrl, token);
    }, delay);
  }

  // --- React integration via useSyncExternalStore ---

  subscribe = (callback: () => void): (() => void) => {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  };

  getSnapshot = (): SseSnapshot => {
    return this.snapshot;
  };

  getServerSnapshot = (): SseSnapshot => {
    // For SSR: return empty state
    return {
      tasks: [],
      approvals: [],
      agentEvents: [],
      connectionStatus: "disconnected",
      clientId: null,
    };
  };

  // --- Internal ---

  private updateState(partial: Partial<SseState>) {
    Object.assign(this.state, partial);
    this.emitChange();
  }

  private emitChange() {
    this.snapshot = this.buildSnapshot();
    this.listeners.forEach((l) => l());
  }

  private buildSnapshot(): SseSnapshot {
    return {
      tasks: [...this.state.tasks.values()],
      approvals: [...this.state.approvals.values()],
      agentEvents: [...this.state.agentEvents],
      connectionStatus: this.state.connectionStatus,
      clientId: this.state.clientId,
    };
  }
}

// Singleton -- one SSE connection per app
export const sseStore = new SseStore();
```

### SSE Hook

**`src/hooks/use-sse.ts`:**
```typescript
import { useEffect, useSyncExternalStore } from "react";
import { sseStore } from "@/lib/sse-store";
import { getAccessToken } from "@/lib/api-client";

/**
 * Subscribe to live SSE events. Connects on mount, disconnects on unmount.
 * Returns the current snapshot of all SSE-delivered state.
 */
export function useSse() {
  useEffect(() => {
    const baseUrl = import.meta.env.VITE_API_BASE_URL || "";
    const token = getAccessToken() ?? undefined;
    sseStore.connect(baseUrl, token);
    return () => sseStore.disconnect();
  }, []);

  return useSyncExternalStore(
    sseStore.subscribe,
    sseStore.getSnapshot,
    sseStore.getServerSnapshot,
  );
}

/**
 * Convenience hook for just task updates from SSE.
 */
export function useSseTasks() {
  const { tasks } = useSse();
  return tasks;
}

/**
 * Convenience hook for just the connection status.
 */
export function useSseStatus() {
  const { connectionStatus } = useSse();
  return connectionStatus;
}
```

### Alternative: fetch-based SSE with Auth Headers

When you need full header control (e.g., external-facing dashboard with strict auth):

```typescript
async function connectSseWithAuth(
  url: string,
  token: string,
  onEvent: (name: string, data: string) => void,
  signal: AbortSignal,
) {
  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "text/event-stream",
    },
    signal,
  });

  if (!response.ok || !response.body) {
    throw new Error(`SSE connection failed: ${response.status}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? ""; // Keep incomplete line in buffer

    let currentEvent = "message";
    for (const line of lines) {
      if (line.startsWith("event: ")) {
        currentEvent = line.slice(7).trim();
      } else if (line.startsWith("data: ")) {
        onEvent(currentEvent, line.slice(6));
        currentEvent = "message";
      }
      // Blank line = end of event (already handled by data dispatch)
    }
  }
}
```

**Tradeoff:** fetch-based SSE loses automatic reconnection. You must implement retry logic yourself. Use `EventSource` when possible.

### Combining SSE + TanStack Query

Two patterns for keeping REST queries and SSE in sync:

**Pattern A: SSE Invalidates Query Cache (simpler)**

```typescript
// In your SSE event handler:
es.addEventListener("task_update", () => {
  // Re-fetch from REST to get the canonical state
  queryClient.invalidateQueries({ queryKey: queryKeys.tasks.all });
});
```

Pros: Single source of truth (REST). Simple.
Cons: Extra network round-trip per event.

**Pattern B: SSE Directly Updates State (lower latency)**

```typescript
// SSE delivers the full task record in its payload.
// The SseStore holds authoritative real-time state.
// TanStack Query is used only for initial load and paginated lists.
// Components read from useSyncExternalStore for live data.
```

Pros: Zero-latency updates. SSE payload already contains full state.
Cons: Two state sources to reconcile.

**Recommendation for cairn-rs:** Use Pattern B (direct state merge) because the SSE payloads from `build_sse_frame_with_current_state` already contain full `TaskRecord` and `ApprovalRecord` data. Use TanStack Query only for initial data load and paginated historical views.

---

## 10. Dark Mode Implementation

### ThemeProvider

**`src/components/theme-provider.tsx`:**
```tsx
import { createContext, useContext, useEffect, useState } from "react";

type Theme = "dark" | "light" | "system";

interface ThemeProviderProps {
  children: React.ReactNode;
  defaultTheme?: Theme;
  storageKey?: string;
}

interface ThemeProviderState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

const ThemeProviderContext = createContext<ThemeProviderState>({
  theme: "system",
  setTheme: () => null,
});

export function ThemeProvider({
  children,
  defaultTheme = "system",
  storageKey = "cairn-ui-theme",
  ...props
}: ThemeProviderProps) {
  const [theme, setTheme] = useState<Theme>(
    () => (localStorage.getItem(storageKey) as Theme) || defaultTheme,
  );

  useEffect(() => {
    const root = window.document.documentElement;
    root.classList.remove("light", "dark");

    if (theme === "system") {
      const systemTheme = window.matchMedia("(prefers-color-scheme: dark)")
        .matches
        ? "dark"
        : "light";
      root.classList.add(systemTheme);
      return;
    }

    root.classList.add(theme);
  }, [theme]);

  const value = {
    theme,
    setTheme: (newTheme: Theme) => {
      localStorage.setItem(storageKey, newTheme);
      setTheme(newTheme);
    },
  };

  return (
    <ThemeProviderContext.Provider {...props} value={value}>
      {children}
    </ThemeProviderContext.Provider>
  );
}

export const useTheme = () => {
  const context = useContext(ThemeProviderContext);
  if (context === undefined) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }
  return context;
};
```

### Theme Toggle Component

**`src/components/layout/theme-toggle.tsx`:**
```tsx
import { Moon, Sun } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useTheme } from "@/components/theme-provider";

export function ThemeToggle() {
  const { setTheme } = useTheme();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="icon">
          <Sun className="h-[1.2rem] w-[1.2rem] scale-100 rotate-0 transition-all dark:scale-0 dark:-rotate-90" />
          <Moon className="absolute h-[1.2rem] w-[1.2rem] scale-0 rotate-90 transition-all dark:scale-100 dark:rotate-0" />
          <span className="sr-only">Toggle theme</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => setTheme("light")}>Light</DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("dark")}>Dark</DropdownMenuItem>
        <DropdownMenuItem onClick={() => setTheme("system")}>System</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
```

### How Dark Mode Works

1. **Tailwind v4** uses `@custom-variant dark (&:where(.dark, .dark *))` to scope dark styles to elements with a `.dark` class ancestor
2. **ThemeProvider** adds/removes the `dark` class on `<html>` based on user preference
3. **CSS variables** in `index.css` swap values under `.dark {}` so all shadcn/ui components adapt automatically
4. **Components use semantic tokens**: `bg-background`, `text-foreground`, `border-border` -- never raw colors
5. **Persistence**: User preference saved to `localStorage` and restored on page load

### Preventing Flash of Unstyled Content (FOUC)

Add this inline script to `index.html` before any other scripts:

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <script>
      // Apply theme before first paint to prevent flash
      (function() {
        const theme = localStorage.getItem('cairn-ui-theme') || 'system';
        const dark = theme === 'dark' ||
          (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
        if (dark) document.documentElement.classList.add('dark');
      })();
    </script>
    <!-- ... other head elements -->
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

---

## 11. Serving Static Files from Axum

### Development: Vite Dev Server + Proxy

During development, the Vite dev server (`:5173`) handles the frontend with hot module replacement. API requests are proxied to axum (`:3000`) via `vite.config.ts` proxy configuration. No changes to the Rust code needed.

```bash
# Terminal 1: axum backend
cargo run --bin cairn-app

# Terminal 2: Vite dev server with proxy to backend
cd crates/cairn-app/frontend && pnpm dev
```

### Production: Embedded in Rust Binary

**Using `rust-embed` (recommended):**

```rust
use axum::{Router, http::Uri, response::IntoResponse, routing::get};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct FrontendAssets;

/// Serve embedded frontend files with SPA fallback.
async fn serve_frontend(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Try exact file match first
    if let Some(file) = FrontendAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            [(axum::http::header::CONTENT_TYPE, mime.as_ref().to_owned())],
            file.data.to_vec(),
        )
            .into_response();
    }

    // SPA fallback: serve index.html for client-side routing
    match FrontendAssets::get("index.html") {
        Some(index) => (
            [(axum::http::header::CONTENT_TYPE, "text/html".to_owned())],
            index.data.to_vec(),
        )
            .into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn app_router(api_routes: Router) -> Router {
    Router::new()
        .nest("/v1", api_routes)       // API routes take priority
        .fallback(get(serve_frontend)) // Everything else -> frontend
}
```

**Key behaviors:**

- `rust-embed` with the `axum` feature reads files from disk in debug mode (for fast iteration without Node) and embeds them in the binary in release mode
- `/v1/*` routes are handled by axum API handlers
- All other routes (e.g., `/tasks`, `/approvals`, `/settings`) serve `index.html` for React Router
- Static assets (`/assets/main-abc123.js`) are served with correct MIME types

### Production Alternative: `tower-http::ServeDir`

For filesystem-based serving without embedding (e.g., Docker deployments where frontend is a separate build artifact):

```rust
use tower_http::services::{ServeDir, ServeFile};

let spa_service = ServeDir::new("frontend/dist")
    .append_index_html_on_directories(true)
    .precompressed_gzip()
    .precompressed_br()
    .not_found_service(ServeFile::new("frontend/dist/index.html"));

let app = Router::new()
    .nest("/v1", api_routes)
    .fallback_service(spa_service);
```

This supports precompressed `.gz` and `.br` files for optimal transfer sizes.

---

## 12. Build and Deployment

### Development Workflow

```bash
# One-time setup
cd crates/cairn-app/frontend
pnpm install

# Daily development (two terminals)
# Terminal 1:
cargo run --bin cairn-app      # axum on :3000

# Terminal 2:
cd crates/cairn-app/frontend
pnpm dev                        # Vite on :5173, proxies /v1 -> :3000
```

### Production Build

```bash
# Build frontend (outputs to frontend/dist/)
cd crates/cairn-app/frontend
pnpm run build

# Build Rust binary (rust-embed includes frontend/dist/)
cd ../../..
cargo build --release

# Result: single binary
./target/release/cairn-app
# Dashboard:  http://localhost:3000/
# API:        http://localhost:3000/v1/
# SSE:        http://localhost:3000/v1/stream
```

### CI Pipeline (GitHub Actions)

```yaml
name: Build
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: pnpm/action-setup@v4
        with:
          version: 9

      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: crates/cairn-app/frontend/pnpm-lock.yaml

      - name: Build frontend
        working-directory: crates/cairn-app/frontend
        run: |
          pnpm install --frozen-lockfile
          pnpm run build

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2

      - name: Build release binary
        run: cargo build --release

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: cairn-app
          path: target/release/cairn-app
```

### Docker Deployment

```dockerfile
# Stage 1: Build frontend
FROM node:22-alpine AS frontend
WORKDIR /app/frontend
COPY crates/cairn-app/frontend/package.json crates/cairn-app/frontend/pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY crates/cairn-app/frontend/ .
RUN pnpm run build

# Stage 2: Build Rust binary
FROM rust:1.85-bookworm AS backend
WORKDIR /app
COPY . .
COPY --from=frontend /app/frontend/dist crates/cairn-app/frontend/dist
RUN cargo build --release --bin cairn-app

# Stage 3: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/cairn-app /usr/local/bin/
EXPOSE 3000
CMD ["cairn-app"]
```

### Environment Configuration

| Variable | Dev | Production | Purpose |
|----------|-----|------------|---------|
| `VITE_API_BASE_URL` | `http://localhost:3000` | (empty) | API base URL; empty = same origin |
| `VITE_APP_TITLE` | `Cairn Dev` | `Cairn` | Browser tab title |
| `CAIRN_LISTEN_ADDR` | `0.0.0.0:3000` | `0.0.0.0:3000` | Rust-side bind address |
| `CAIRN_SERVICE_TOKEN` | `dev-token` | (secret) | Bearer token for operator auth |

---

## 13. Sources

| # | Source | URL | Key Contribution |
|---|--------|-----|------------------|
| 1 | Vite Guide | https://vite.dev/guide/ | Scaffolding, v8 defaults, Node.js requirements |
| 2 | Vite Proxy Config | https://vite.dev/config/server-options.html | Dev server proxy to backend |
| 3 | Vite Env Variables | https://vite.dev/guide/env-and-mode.html | `VITE_` prefix, `.env` files, TypeScript typing |
| 4 | Vite Backend Integration | https://vite.dev/guide/backend-integration.html | Manifest, entry points, production build |
| 5 | Vite Build | https://vite.dev/guide/build.html | Output config, chunk splitting, asset handling |
| 6 | TanStack Query Overview | https://tanstack.com/query/latest/docs/framework/react/overview | v5 setup, QueryClientProvider |
| 7 | TanStack Query Keys | https://tanstack.com/query/latest/docs/framework/react/guides/query-keys | Key structure, hashing, factories |
| 8 | TanStack useQuery | https://tanstack.com/query/latest/docs/framework/react/reference/useQuery | Hook signature, options, TypeScript |
| 9 | TanStack useMutation | https://tanstack.com/query/latest/docs/framework/react/reference/useMutation | Mutation lifecycle, invalidation |
| 10 | shadcn/ui Vite Install | https://ui.shadcn.com/docs/installation/vite | Full install steps, path aliases |
| 11 | shadcn/ui Theming | https://ui.shadcn.com/docs/theming | CSS variables, tokens, dark mode |
| 12 | shadcn/ui Dark Mode (Vite) | https://ui.shadcn.com/docs/dark-mode/vite | ThemeProvider, ModeToggle components |
| 13 | Tailwind CSS Dark Mode | https://tailwindcss.com/docs/dark-mode | `@custom-variant`, class/data strategies |
| 14 | MDN EventSource | https://developer.mozilla.org/en-US/docs/Web/API/EventSource | SSE API, no custom headers, reconnection |
| 15 | MDN Fetch API | https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API/Using_Fetch | JSON, auth headers, AbortController |
| 16 | MDN ReadableStream | https://developer.mozilla.org/en-US/docs/Web/API/ReadableStream | fetch-based SSE alternative |
| 17 | React useEffect | https://react.dev/reference/react/useEffect | Subscription patterns, cleanup, stale closures |
| 18 | React Custom Hooks | https://react.dev/learn/reusing-logic-with-custom-hooks | Hook composition, naming, best practices |
| 19 | axum SSE | https://docs.rs/axum/latest/axum/response/sse/index.html | Sse, Event, KeepAlive |
| 20 | axum Middleware | https://docs.rs/axum/latest/axum/middleware/index.html | Auth middleware pattern |
| 21 | tower-http ServeDir | https://docs.rs/tower-http/latest/tower_http/services/struct.ServeDir.html | Static file serving, SPA fallback |
| 22 | cairn-rs sse.rs | crates/cairn-api/src/sse.rs | 16 SseEventName variants, SseFrame struct |
| 23 | cairn-rs sse_publisher.rs | crates/cairn-api/src/sse_publisher.rs | Event mapping, lastEventId replay |
| 24 | cairn-rs auth.rs | crates/cairn-api/src/auth.rs | ServiceTokenRegistry, AuthPrincipal, Authenticator trait |
| 25 | cairn-rs endpoints.rs | crates/cairn-api/src/endpoints.rs | REST endpoint signatures, ListQuery |
