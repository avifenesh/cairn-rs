# Frontend Stack for Rust/Axum Dashboards

**Research date:** 2026-04-04
**Context:** cairn-rs already has a mature SSE publisher (`cairn-api/src/sse.rs`, `sse_publisher.rs`) with 16 named event types, `lastEventId`-based replay, and an axum backend with tower-http. The dashboard needs to show live agent runs, streaming logs, cost charts, and approval workflows.

---

## 1. Framework Comparison for Real-Time Dashboards

### React (recommended for cairn-rs)

**Why it wins for this use case:**
- Largest ecosystem of dashboard-specific libraries (shadcn/ui, Recharts, TanStack)
- `useSyncExternalStore` is purpose-built for subscribing to external data sources like SSE
- shadcn/ui provides production-ready dashboard templates out of the box
- Vite builds produce small static bundles ideal for embedding in a Rust binary
- Hiring/contribution pool is unmatched

**Concerns:**
- Virtual DOM overhead vs fine-grained reactivity frameworks
- Bundle size larger than Svelte/Solid (~40-60KB min+gzip for React+ReactDOM)

**Verdict:** Best choice given cairn-rs needs a dashboard quickly, with rich SSE integration and charting.

### Svelte / SvelteKit

**Strengths:**
- Compiler-based: no runtime shipped, smallest bundles
- Reactivity via runes (`$state`, `$derived`, `$effect`) feels natural for real-time data
- Used by Spotify, NYT, IKEA in production
- SvelteKit provides SSR, static generation, and adapter-based deployment

**Weaknesses:**
- Smaller component ecosystem than React
- No equivalent to shadcn/ui (though skeleton.dev and melt-ui exist)
- Charting library ecosystem thinner (must use vanilla JS libs like ECharts)

**Verdict:** Excellent if team has Svelte experience. Second-best choice.

### SolidJS

**Strengths:**
- Fine-grained reactivity without virtual DOM (closest to "reactive spreadsheet" model)
- React-like JSX syntax but with true reactivity (no re-renders)
- Very small bundle (~7KB)
- Supported by TanStack Query

**Weaknesses:**
- Smallest ecosystem of the four
- Component libraries are immature
- Community is niche

**Verdict:** Technically superior reactivity model, but ecosystem too thin for a dashboard that needs charts, tables, and complex UI fast.

### Leptos (Rust WASM)

**Strengths:**
- Full-stack Rust: shared types between backend and frontend (no codegen needed)
- Fine-grained reactive signals similar to SolidJS
- Server functions eliminate REST boilerplate
- Single language for entire stack
- `view!` macro with JSX-like syntax
- Hot-reloading and dedicated LSP support

**Weaknesses:**
- WASM bundle size typically 200-500KB (larger than JS alternatives)
- Ecosystem dramatically smaller: few charting libraries, no shadcn equivalent
- Compile times are slow (Rust WASM adds significant build overhead)
- Debugging WASM is harder than debugging JS
- Fewer developers can contribute
- The "plotters" crate exists for Rust charting but is far behind D3/ECharts

**Verdict:** Tempting for a Rust-native project, but the charting/component ecosystem gap makes it a poor fit for a dashboard that needs cost charts, sparklines, and data tables. Consider for a future v2 if the Leptos ecosystem matures.

### Dioxus (Rust WASM alternative)

**Strengths:**
- Cross-platform (web, desktop, mobile) from one codebase
- 24.5k GitHub stars, used by Airbus and ESA
- React-like component model in Rust

**Weaknesses:**
- Same ecosystem gaps as Leptos for dashboard components
- Same WASM bundle size and compile time issues

**Verdict:** Same assessment as Leptos. Not ready for dashboard-heavy use cases.

### Recommendation

**React + Vite + shadcn/ui** is the pragmatic choice. The ecosystem advantage for dashboards is decisive.

---

## 2. SSE Consumption Patterns

### The EventSource API (MDN)

**Source:** MDN Web Docs - EventSource API

```javascript
const evtSource = new EventSource('/v1/stream');

// Named events (matches cairn-rs SseEventName variants)
evtSource.addEventListener('task_update', (e) => {
  const data = JSON.parse(e.data);
  // Update task state
});

evtSource.addEventListener('approval_required', (e) => {
  const data = JSON.parse(e.data);
  // Show approval dialog
});

evtSource.addEventListener('ready', (e) => {
  const data = JSON.parse(e.data);
  console.log('Connected, clientId:', data.clientId);
});
```

**Key characteristics:**
- Unidirectional: server to client only
- Persistent connection with automatic reconnection
- `readyState`: CONNECTING (0), OPEN (1), CLOSED (2)
- HTTP/1.1 limit: 6 concurrent connections per domain (HTTP/2: ~100)
- `lastEventId` sent automatically on reconnect (cairn-rs already supports this via `SseReplayQuery`)

### Reconnection handling

The browser's EventSource handles reconnection automatically, but you can enhance it:

```javascript
evtSource.onerror = (e) => {
  if (evtSource.readyState === EventSource.CLOSED) {
    // Server closed connection, manual reconnect with backoff
    setTimeout(() => reconnect(), backoffMs);
  }
  // readyState === CONNECTING means browser is auto-reconnecting
};
```

The `retry:` field in SSE protocol controls auto-reconnect interval (server-side, via axum `Event::retry()`).

### React Hook Pattern: useSyncExternalStore

**Source:** React docs - useSyncExternalStore

This is the recommended React primitive for subscribing to external stores like SSE:

```typescript
import { useSyncExternalStore } from 'react';

// External store that accumulates SSE events
class SseStore {
  private listeners = new Set<() => void>();
  private tasks: Map<string, TaskData> = new Map();
  private eventSource: EventSource | null = null;
  private snapshot: TaskData[] = [];

  connect(url: string) {
    this.eventSource = new EventSource(url);
    
    this.eventSource.addEventListener('task_update', (e) => {
      const data = JSON.parse(e.data);
      this.tasks.set(data.task.taskId, data.task);
      this.snapshot = [...this.tasks.values()]; // Immutable snapshot
      this.emit();
    });
    
    this.eventSource.addEventListener('approval_required', (e) => {
      // Handle approval events
      this.emit();
    });
  }

  private emit() {
    this.listeners.forEach(l => l());
  }

  subscribe = (callback: () => void) => {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  };

  getSnapshot = () => this.snapshot;

  disconnect() {
    this.eventSource?.close();
  }
}

const sseStore = new SseStore();

// Custom hook
function useTaskUpdates() {
  return useSyncExternalStore(
    sseStore.subscribe,
    sseStore.getSnapshot
  );
}
```

**Why useSyncExternalStore over useEffect:**
- Concurrent-mode safe (avoids tearing)
- React manages subscriptions and cleanup
- Returns immutable snapshots (required by React)
- Works with server-side rendering via `getServerSnapshot`

### Alternative: TanStack Query + SSE

TanStack Query can complement SSE by managing REST-fetched data (initial load, pagination) while SSE handles live updates:

```typescript
// Initial data via REST + TanStack Query
const { data: tasks } = useQuery({
  queryKey: ['tasks'],
  queryFn: () => fetch('/v1/tasks').then(r => r.json()),
});

// Live updates via SSE invalidate the query cache
useEffect(() => {
  const es = new EventSource('/v1/stream');
  es.addEventListener('task_update', () => {
    queryClient.invalidateQueries({ queryKey: ['tasks'] });
  });
  return () => es.close();
}, []);
```

### cairn-rs SSE surface (existing)

The backend already emits these named SSE events that the frontend must handle:

| SSE Event Name | Source Domain Event | Dashboard Use |
|---|---|---|
| `ready` | Connection init | Show connected status |
| `task_update` | TaskCreated, TaskStateChanged, TaskLeaseClaimed, TaskLeaseHeartbeated | Task list, run timeline |
| `approval_required` | ApprovalRequested | Approval dialog/queue |
| `assistant_tool_call` | ToolInvocationStarted/Completed/Failed | Tool call log |
| `agent_progress` | ExternalWorkerReported, SubagentSpawned | Agent activity feed |
| `feed_update` | (mapped elsewhere) | Activity feed |
| `assistant_delta` | (streaming) | Chat/log stream |
| `assistant_end` | (streaming) | End of assistant turn |

The `lastEventId` replay mechanism is already implemented, meaning the frontend can reconnect and catch up seamlessly.

---

## 3. Real-Time Dashboard Architecture

### Recommended Architecture

```
cairn-rs binary
  |
  +-- axum router
  |     +-- /v1/stream          (SSE endpoint, existing)
  |     +-- /v1/tasks           (REST, existing)
  |     +-- /v1/approvals       (REST, existing)
  |     +-- /v1/runs            (REST, existing)
  |     +-- /                   (ServeDir or rust-embed for static files)
  |
  +-- embedded frontend (Vite build output)
        +-- index.html
        +-- assets/
              +-- main-[hash].js
              +-- main-[hash].css
```

### Data Flow

1. **Initial load:** Frontend fetches current state via REST (`/v1/tasks`, `/v1/runs`)
2. **Live updates:** Frontend connects to `/v1/stream` SSE endpoint
3. **Reconnection:** On disconnect, browser sends `Last-Event-ID` header; backend replays missed events
4. **State management:** `useSyncExternalStore` merges SSE events into local state
5. **Charts:** Cost/latency data fetched via REST, updated via SSE `agent_progress` events

### State Management Pattern

```
REST (initial) ---> React Query cache ---> Component tree
                          ^
SSE (live)    ---> SseStore ---> invalidateQueries() or direct state merge
```

Two viable patterns:
- **Query invalidation:** SSE triggers re-fetch via TanStack Query (simpler, slightly more latency)
- **Direct state merge:** SSE events directly update an external store (lower latency, more code)

For a monitoring dashboard, **direct state merge** is better because latency matters and the SSE payload already contains the full updated state.

---

## 4. Embedded vs Separate Frontend

### Option A: Embedded in Rust Binary (recommended)

**How it works:**
- Vite builds the frontend to `dist/` at build time
- `rust-embed` or `include_dir` embeds `dist/` into the Rust binary at compile time
- axum serves embedded files via a fallback route

**Pros:**
- Single binary deployment (like Grafana, Prometheus, Jaeger)
- No CORS configuration needed
- No separate web server
- Version-locked: frontend and backend always match
- Simple ops: `./cairn-app` and you have a full dashboard

**Cons:**
- Build pipeline needs Node.js + Rust
- Binary size increases (~1-5MB for a typical Vite build)
- Frontend changes require recompile (mitigated in dev mode)

**The Grafana model:** Grafana (Go binary) embeds its React frontend. TypeScript/React is built by Node, output is embedded via Go's `embed` package, served at `/`. Plugins extend the core. This is the proven pattern for developer tools.

### Option B: Separate Frontend (Next.js, standalone SPA)

**Pros:**
- Independent deploy cycles
- Hot reload without Rust recompile
- Can use Next.js SSR features

**Cons:**
- CORS configuration required
- Two deploy artifacts to manage
- Version skew between frontend and backend
- More ops complexity

### Option C: Hybrid (recommended for development)

**Development:** Vite dev server on `:5173` proxying API calls to axum on `:3000`
**Production:** Embedded in the Rust binary

This gives fast frontend iteration in dev with single-binary simplicity in production.

---

## 5. Embedding Frontend in a Rust Binary

### rust-embed (recommended)

**Crate:** `rust-embed` v8.11+ (MIT license)
**Key feature:** In debug mode, reads files from disk (fast iteration). In release mode, embeds files in the binary.

```toml
[dependencies]
rust-embed = { version = "8", features = ["axum"] }
```

```rust
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct FrontendAssets;
```

With the `axum` feature, rust-embed provides direct integration. For SPA routing (all paths fall back to index.html):

```rust
use axum::{Router, routing::get};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist"]
struct Assets;

async fn serve_frontend(uri: axum::http::Uri) -> impl axum::response::IntoResponse {
    let path = uri.path().trim_start_matches('/');
    
    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(axum::http::header::CONTENT_TYPE, mime.as_ref())], file.data).into_response()
        }
        None => {
            // SPA fallback: serve index.html for client-side routing
            let index = Assets::get("index.html").unwrap();
            ([(axum::http::header::CONTENT_TYPE, "text/html")], index.data).into_response()
        }
    }
}

let app = Router::new()
    .nest("/v1", api_routes())       // API routes first
    .fallback(get(serve_frontend));   // Frontend fallback
```

### include_dir (alternative)

**Crate:** `include_dir`
**Key feature:** `include_dir!()` macro embeds at compile time, always (no dev-mode disk fallback).

```rust
use include_dir::{include_dir, Dir};
static FRONTEND: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/frontend/dist");
```

**Tradeoff:** Simpler API but no dev-mode disk loading. Binary grows with asset size. For 620 files / 64MB, compile time increased from 1.5s to 5s and RAM from 200MB to 730MB. Fine for a typical Vite build (1-5MB).

### tower-http ServeDir (filesystem-only)

**For dev mode or non-embedded deployments:**

```rust
use tower_http::services::ServeDir;

let app = Router::new()
    .nest("/v1", api_routes())
    .fallback_service(
        ServeDir::new("frontend/dist")
            .append_index_html_on_directories(true)
            .precompressed_gzip()
            .precompressed_br()
    );
```

Supports precompressed assets (`.gz`, `.br`), directory index, and 404 fallback. No binary embedding -- reads from filesystem.

### Recommended Build Pipeline

```bash
# Build frontend
cd frontend && npm run build  # Outputs to frontend/dist/

# Build Rust binary (embeds frontend/dist/ via rust-embed)
cargo build --release

# Result: single binary with embedded dashboard
./target/release/cairn-app
# Dashboard at http://localhost:3000/
# API at http://localhost:3000/v1/
# SSE at http://localhost:3000/v1/stream
```

### cairn-app Cargo.toml addition

```toml
[dependencies]
rust-embed = { version = "8", features = ["axum"] }
mime_guess = "2"
```

---

## 6. Chart & Visualization Libraries

### Recharts (recommended for React)

**Source:** recharts.org, GitHub

- Built on React + D3, declarative component API
- Native SVG rendering, lightweight
- Composable: `<LineChart>`, `<BarChart>`, `<AreaChart>`, `<PieChart>`
- Easy to integrate with React state (re-renders on prop change)
- Good for: cost charts, latency over time, task counts

```jsx
<LineChart data={costData}>
  <XAxis dataKey="time" />
  <YAxis />
  <Tooltip />
  <Line type="monotone" dataKey="cost" stroke="#8884d8" />
</LineChart>
```

**Limitation:** Performance degrades with >10K points. Use downsampling for high-frequency data.

### Apache ECharts (best for heavy visualization)

**Source:** echarts.apache.org

- 20+ chart types out of the box
- Canvas and SVG rendering (switchable)
- Progressive rendering and stream loading: handles 10 million data points
- Professional data transforms (filtering, clustering, regression)
- Accessibility features (auto-generated descriptions, decal patterns)
- Larger bundle (~400KB min) but tree-shakeable

**Best for:** If the dashboard needs heatmaps, treemaps, sankey diagrams, or very large datasets. Overkill for basic line/bar charts.

### Chart.js

**Source:** chartjs.org

- 8 chart types, HTML5 Canvas
- Tree-shakeable: 48KB full, 14KB minimal
- Decimation plugin handles 1M data points
- Mixed chart types (bar + line)
- Animation framework for smooth transitions

**Good for:** Simple, lightweight charting. Less composable with React than Recharts.

### D3.js

**Source:** d3js.org

- Low-level visualization primitives (not a chart library)
- Direct DOM manipulation, no virtual DOM overhead
- 50+ map projections, force-directed graphs, treemaps
- Maximum flexibility, maximum learning curve
- Current version: 7.9.0

**Best for:** Bespoke visualizations (force graphs for agent relationships, custom timeline views). Not ideal for standard dashboard charts -- too much boilerplate.

### Recommendation for cairn-rs dashboard

| Chart Need | Library |
|---|---|
| Cost over time, latency sparklines | **Recharts** |
| Task status distribution (pie/bar) | **Recharts** |
| Agent run timeline | Custom with D3 or **Recharts** composed |
| High-volume metric streams (>10K pts) | **Apache ECharts** |
| Simple sparklines in table cells | **Recharts** `<Sparklines>` or raw SVG |

Start with **Recharts** for everything. Graduate individual charts to ECharts if performance demands it.

---

## 7. Tailwind + shadcn/ui Patterns

### shadcn/ui (recommended component system)

**Source:** ui.shadcn.com

**What it is:** Not a library you `npm install`. It is a collection of copy-paste React components built on Radix UI primitives, styled with Tailwind CSS. You own the code.

**Key characteristics:**
- Components copied into your project (`npx shadcn add button`)
- Full customization control (you edit the source directly)
- Built on Radix UI (accessible, keyboard-navigable)
- Styled with Tailwind CSS v4
- TypeScript-first

**Dashboard-relevant components:**
- `Card` -- metric cards, stat boxes
- `Table` / `DataTable` -- task lists, event logs
- `Dialog` / `Sheet` -- approval workflows, detail panels
- `Tabs` -- multi-view dashboards
- `Badge` -- status indicators (Running, Completed, Failed)
- `Select`, `Command` -- filtering, search
- `Chart` -- built-in chart components wrapping Recharts
- `Sidebar` -- navigation (collapsible)
- `Tooltip` -- hover info

**Dashboard template:** shadcn/ui ships a complete dashboard example with sidebar navigation, metric cards with trend indicators, data tables with pagination, multi-period chart views, and team management. This is the starting point.

### Tailwind CSS v4

- Utility-first CSS framework
- v4 uses CSS-native cascade layers, no PostCSS config needed
- Tiny runtime: only ships classes actually used
- Dark mode via `dark:` prefix
- Responsive via `sm:`, `md:`, `lg:` prefixes

### Tailwind Plus (paid, optional)

- 500+ production UI components ($299 one-time)
- Application UI blocks: tables, forms, navigation, modals, command palettes
- React, Vue, and HTML versions
- Not necessary if using shadcn/ui (which is free)

### Recommended Component Stack

```
shadcn/ui (free, copy-paste components)
  +-- Radix UI (accessible primitives)
  +-- Tailwind CSS v4 (styling)
  +-- Recharts (charts, via shadcn chart component)
  +-- cmdk (command palette)
  +-- TanStack Table (data tables)
```

---

## 8. Recommended Stack Summary

### Production Stack

| Layer | Choice | Rationale |
|---|---|---|
| **Framework** | React 19 + Vite | Ecosystem, SSE integration, component libraries |
| **Components** | shadcn/ui | Free, accessible, dashboard template included |
| **Styling** | Tailwind CSS v4 | Utility-first, tiny output, dark mode |
| **Charts** | Recharts (primary), ECharts (heavy viz) | React-native, declarative, composable |
| **Data fetching** | TanStack Query (REST) + useSyncExternalStore (SSE) | Best-in-class for both patterns |
| **State** | External SseStore + React Query cache | SSE events merge directly, REST data cached |
| **Embedding** | rust-embed (prod) + Vite dev server (dev) | Single binary deploy, fast dev iteration |
| **Routing** | React Router or TanStack Router | Client-side routing with SPA fallback |

### Project Structure

```
cairn-rs/
  crates/
    cairn-app/
      src/
        main.rs          # axum server + embedded frontend
      frontend/           # Vite + React project
        src/
          hooks/
            use-sse.ts    # useSyncExternalStore + EventSource
            use-tasks.ts  # TanStack Query for /v1/tasks
          components/
            dashboard/
              task-list.tsx
              cost-chart.tsx
              approval-queue.tsx
              agent-feed.tsx
            ui/            # shadcn/ui components
          lib/
            sse-store.ts   # External SSE state store
            api-client.ts  # REST client for /v1/*
          App.tsx
          main.tsx
        dist/              # Vite build output (embedded by rust-embed)
        package.json
        vite.config.ts
        tailwind.config.ts
```

### Development Workflow

```bash
# Terminal 1: Rust backend
cargo run --bin cairn-app

# Terminal 2: Frontend dev server (hot reload, proxies API to :3000)
cd crates/cairn-app/frontend && npm run dev
# Vite serves on :5173, proxies /v1/* to localhost:3000
```

### Production Build

```bash
cd crates/cairn-app/frontend && npm run build
cargo build --release
# Single binary: ./target/release/cairn-app
# Dashboard: http://localhost:3000/
```

---

## 9. Sources

1. **MDN - EventSource API** -- https://developer.mozilla.org/en-US/docs/Web/API/EventSource
   Complete reference for SSE consumption: constructor, readyState, named events, reconnection.

2. **React - useSyncExternalStore** -- https://react.dev/reference/react/useSyncExternalStore
   Recommended hook for subscribing to external data sources like SSE stores.

3. **docs.rs - rust-embed** -- https://docs.rs/rust-embed/latest/rust_embed/
   Embed static files in Rust binary; dev mode reads from disk, release embeds in binary. Axum feature flag.

4. **docs.rs - include_dir** -- https://docs.rs/include_dir/latest/include_dir/
   Compile-time directory embedding. Performance data: 620 files/64MB adds ~4s compile, ~500MB RAM.

5. **docs.rs - tower-http ServeDir** -- https://docs.rs/tower-http/latest/tower_http/services/struct.ServeDir.html
   Static file serving with precompression, directory index, fallback routing.

6. **docs.rs - axum SSE** -- https://docs.rs/axum/latest/axum/response/sse/index.html
   Axum SSE types: Sse, Event, KeepAlive. Stream-based API with keep-alive support.

7. **axum SSE example** -- https://github.com/tokio-rs/axum/blob/main/examples/sse/src/main.rs
   Complete working SSE handler with stream::repeat_with, throttle, and KeepAlive.

8. **Leptos** -- https://leptos.dev/
   Full-stack Rust WASM framework. Reactive signals, server functions, view! macro.

9. **Svelte** -- https://svelte.dev/
   Compiler-based UI framework. Runes reactivity, no runtime, used by Spotify/NYT/IKEA.

10. **SvelteKit** -- https://svelte.dev/docs/kit/introduction
    Full-stack Svelte framework. SSR, static generation, adapter-based deployment.

11. **shadcn/ui** -- https://ui.shadcn.com/
    Copy-paste React components on Radix UI + Tailwind. Dashboard template included.

12. **shadcn/ui Dashboard Example** -- https://ui.shadcn.com/examples/dashboard
    Production dashboard template: sidebar, metric cards, data tables, charts, team management.

13. **Apache ECharts** -- https://echarts.apache.org/
    20+ chart types, Canvas/SVG, handles 10M data points, progressive rendering.

14. **Chart.js** -- https://www.chartjs.org/
    8 chart types, Canvas, 14-48KB, decimation plugin for 1M points.

15. **Recharts** -- https://recharts.org/ and GitHub
    React + D3 charting. Declarative, composable, SVG-based. Best React integration.

16. **D3.js** -- https://d3js.org/
    Low-level visualization primitives. v7.9. Maximum flexibility, steep learning curve.

17. **TanStack Query** -- https://tanstack.com/query/latest
    Async state management. Caching, background updates, SSR hydration. Works with React/Vue/Solid/Svelte.

18. **Vite** -- https://vite.dev/guide/
    Build tool. Fast HMR dev server, Rolldown-based production builds, static asset output.

19. **Dioxus** -- https://dioxuslabs.com/
    Rust cross-platform framework (web/desktop/mobile). 24.5k stars. Used by Airbus, ESA.

20. **Tailwind Plus** -- https://tailwindcss.com/plus
    500+ paid UI components ($299). Application UI, marketing, ecommerce blocks.

21. **Grafana Plugin Docs** -- https://grafana.com/docs/grafana/latest/developers/plugins/
    Grafana embeds React frontend in Go binary. TypeScript + Go plugin architecture.

22. **Rust WASM Web UI** -- https://robert.kra.hn/posts/2022-04-03_rust-web-wasm/
    Analysis of Yew, Dioxus, Sycamore for WASM SPAs. Tooling maturity assessment.

---

## 10. Decision Matrix

| Criterion | React+Vite | Svelte | SolidJS | Leptos (WASM) |
|---|---|---|---|---|
| Dashboard component ecosystem | 5/5 | 3/5 | 2/5 | 1/5 |
| SSE integration ease | 5/5 | 4/5 | 4/5 | 4/5 |
| Charting libraries | 5/5 | 3/5 | 2/5 | 1/5 |
| Bundle size | 3/5 | 5/5 | 5/5 | 2/5 |
| Runtime performance | 3/5 | 4/5 | 5/5 | 4/5 |
| Type safety with Rust backend | 3/5 | 3/5 | 3/5 | 5/5 |
| Developer pool / contributors | 5/5 | 3/5 | 2/5 | 1/5 |
| Time to working dashboard | 5/5 | 3/5 | 2/5 | 2/5 |
| Embed in single binary | 5/5 | 5/5 | 5/5 | 5/5 |
| **Total** | **39/45** | **33/45** | **30/45** | **25/45** |

**Winner: React 19 + Vite + shadcn/ui + Recharts, embedded via rust-embed.**
