# Operator Dashboard UI Patterns for AI Agent Platforms

**Research date:** 2026-04-05
**Sources:** 18 (Langfuse, Helicone, Phoenix, LangSmith, Laminar, AgentOps, Portkey, Tremor, shadcn/ui, TanStack, MDN, React docs, SWR, Recharts, tkdodo.eu)
**Context:** cairn-rs has a mature SSE publisher with 16 named event types, `lastEventId`-based replay, and an axum backend. The dashboard will be React + Vite + shadcn/ui embedded in the Rust binary via rust-embed.

---

## Table of Contents

1. [Core Layout Patterns](#1-core-layout-patterns)
2. [Real-Time Update Patterns](#2-real-time-update-patterns)
3. [Key Views Every Operator Dashboard Needs](#3-key-views-every-operator-dashboard-needs)
4. [Data Table Patterns](#4-data-table-patterns)
5. [Component Library & Assembly](#5-component-library--assembly)
6. [Dark Mode & Dense Information Display](#6-dark-mode--dense-information-display)
7. [Keyboard-First & Power-User Patterns](#7-keyboard-first--power-user-patterns)
8. [Cairn-Specific Implementation Notes](#8-cairn-specific-implementation-notes)

---

## 1. Core Layout Patterns

### 1.1 The Three-Panel Layout

Every production AI observability dashboard (Langfuse, Helicone, Phoenix, LangSmith, Laminar) uses a variant of the same three-panel layout:

```
+----------+-----------------------------+--------------------+
| Sidebar  | Main Content                | Detail Panel       |
| (nav)    | (table/grid/chart)          | (contextual info)  |
|          |                             |                    |
| 56-64px  | flex-1                      | 320-480px          |
| or       |                             | (conditional)      |
| 240-280px|                             |                    |
+----------+-----------------------------+--------------------+
```

**Sidebar** -- Always present, collapsible to icon-only (56-64px) or expanded (240-280px). Contains project selector, primary navigation, and user profile.

**Main Content** -- Takes remaining space. Holds the primary view: table, chart grid, or detail page. Always scrollable independently.

**Detail Panel** -- Conditional. Appears when a row is selected (trace detail, approval detail, run inspector). Slides in from the right or replaces part of the main content. In shadcn/ui, this is either a `Sheet` (overlay) or a `ResizablePanel` (inline).

### 1.2 Implementation with shadcn/ui

```tsx
// Root layout: SidebarProvider + ResizablePanelGroup
<SidebarProvider>
  <Sidebar collapsible="icon" variant="sidebar">
    <SidebarHeader>
      <ProjectSwitcher />
    </SidebarHeader>
    <SidebarContent>
      <SidebarGroup>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton asChild>
              <Link to="/dashboard"><LayoutDashboard /> Dashboard</Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton asChild>
              <Link to="/runs"><Play /> Runs</Link>
            </SidebarMenuButton>
            <SidebarMenuBadge>3</SidebarMenuBadge>
          </SidebarMenuItem>
          {/* ... more nav items */}
        </SidebarMenu>
      </SidebarGroup>
    </SidebarContent>
    <SidebarFooter>
      <UserNav />
    </SidebarFooter>
  </Sidebar>

  <SidebarInset>
    <header className="flex h-12 items-center border-b px-4">
      <SidebarTrigger />
      <Breadcrumb />
      <div className="ml-auto flex gap-2">
        <ConnectionStatusBadge />
        <CommandPaletteButton />
      </div>
    </header>

    <ResizablePanelGroup direction="horizontal">
      <ResizablePanel defaultSize={70} minSize={50}>
        <Outlet /> {/* Main content: table, charts, etc. */}
      </ResizablePanel>

      {selectedItem && (
        <>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={30} minSize={20}>
            <DetailPanel item={selectedItem} />
          </ResizablePanel>
        </>
      )}
    </ResizablePanelGroup>
  </SidebarInset>
</SidebarProvider>
```

### 1.3 Navigation Structure (Derived from Industry Patterns)

Every successful AI agent dashboard uses this information architecture:

| Nav Item | Icon | Purpose |
|---|---|---|
| **Dashboard** | `LayoutDashboard` | KPI cards + time-series charts |
| **Runs** | `Play` | Active and historical run list |
| **Tasks** | `ListChecks` | Task queue with state filters |
| **Approvals** | `ShieldCheck` | Pending + resolved approval queue |
| **Traces** | `GitBranch` | Span tree / waterfall debugger |
| **Sessions** | `MessageSquare` | Multi-turn conversation grouping |
| **Logs** | `ScrollText` | Raw event stream (live tail) |
| **Settings** | `Settings` | Project config, API keys, alerts |

The sidebar should show **badge counts** on Approvals (pending count) and Runs (active count) using `SidebarMenuBadge`. These update in real-time via SSE.

### 1.4 Responsive Behavior

On mobile (< 768px), the sidebar becomes an off-canvas drawer triggered by a hamburger button. The detail panel switches from inline `ResizablePanel` to a full-screen `Sheet` sliding from the right. Use `useSidebar().isMobile` from shadcn to detect this.

---

## 2. Real-Time Update Patterns

### 2.1 Architecture: SSE Store + React Query

The proven architecture for real-time operator dashboards uses two complementary data paths:

```
                         +-----------------+
   REST /v1/tasks  ----->| TanStack Query  |-----> Component tree
   (initial load,        | (cache, dedup,  |       (tables, cards)
    pagination,          |  pagination)    |
    search)              +---------^-------+
                                   |
                         invalidateQueries()
                                   |
   SSE /v1/stream  ----->| SseStore        |-----> Live indicators
   (real-time push)      | (useSyncExt.)   |       (badges, counters,
                         +-----------------+        toast notifications)
```

**Why two paths:** REST for the initial full dataset (paginated, sorted, filtered). SSE for incremental updates pushed by the server. The SSE store either invalidates the Query cache (simple) or directly patches it (lower latency).

### 2.2 The SSE Store Pattern

```typescript
// lib/sse-store.ts
import { useSyncExternalStore } from "react";

type Listener = () => void;

interface SseStoreState {
  connected: boolean;
  pendingApprovals: number;
  activeRuns: number;
  lastEventId: string | null;
  recentEvents: SseEvent[];
}

class SseStore {
  private listeners = new Set<Listener>();
  private eventSource: EventSource | null = null;
  private state: SseStoreState = {
    connected: false,
    pendingApprovals: 0,
    activeRuns: 0,
    lastEventId: null,
    recentEvents: [],
  };
  // Immutable snapshot for React -- only replaced when state changes
  private snapshot: SseStoreState = this.state;

  connect(url: string) {
    this.eventSource = new EventSource(url);

    this.eventSource.addEventListener("ready", (e) => {
      this.update({ connected: true });
    });

    this.eventSource.addEventListener("task_update", (e) => {
      const data = JSON.parse(e.data);
      this.pushEvent({ type: "task_update", data, id: e.lastEventId });
      // Invalidate TanStack Query cache for tasks
      queryClient.invalidateQueries({ queryKey: ["tasks"] });
    });

    this.eventSource.addEventListener("approval_required", (e) => {
      const data = JSON.parse(e.data);
      this.update({
        pendingApprovals: this.state.pendingApprovals + 1,
      });
      this.pushEvent({ type: "approval_required", data, id: e.lastEventId });
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    });

    this.eventSource.onerror = () => {
      if (this.eventSource?.readyState === EventSource.CLOSED) {
        this.update({ connected: false });
        // Reconnect with backoff
        setTimeout(() => this.connect(url), 3000);
      }
    };
  }

  private update(partial: Partial<SseStoreState>) {
    this.state = { ...this.state, ...partial };
    this.snapshot = this.state; // New reference triggers React re-render
    this.listeners.forEach((l) => l());
  }

  private pushEvent(event: SseEvent) {
    const events = [event, ...this.state.recentEvents].slice(0, 100);
    this.update({ recentEvents: events, lastEventId: event.id });
  }

  subscribe = (callback: Listener) => {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  };

  getSnapshot = () => this.snapshot;

  disconnect() {
    this.eventSource?.close();
    this.update({ connected: false });
  }
}

export const sseStore = new SseStore();

// Hooks
export function useSseState() {
  return useSyncExternalStore(sseStore.subscribe, sseStore.getSnapshot);
}

export function useConnectionStatus() {
  const state = useSseState();
  return state.connected;
}

export function usePendingApprovalCount() {
  const state = useSseState();
  return state.pendingApprovals;
}
```

### 2.3 Why useSyncExternalStore over useEffect

`useSyncExternalStore` is the React team's recommended primitive for external data sources. Advantages over `useEffect` + `useState`:

- **Concurrent-mode safe**: Avoids "tearing" where different components see different SSE states during a render
- **Automatic subscription lifecycle**: React manages mount/unmount cleanup
- **Immutable snapshot contract**: Forces the store to produce stable references, preventing infinite re-render loops
- **SSR support**: `getServerSnapshot` parameter provides fallback for server rendering and hydration

**Critical rule**: `getSnapshot` must return the same reference (`Object.is`) when data has not changed. Always replace the snapshot object entirely rather than mutating it.

### 2.4 TanStack Query Configuration for SSE-Driven Dashboards

When SSE provides live updates, configure TanStack Query to reduce unnecessary polling:

```typescript
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // SSE keeps data fresh, so don't refetch aggressively
      staleTime: 30_000,         // 30s -- SSE will invalidate sooner
      refetchOnWindowFocus: true, // Still useful when returning to tab
      refetchOnReconnect: true,   // Catch up after network loss
      retry: 3,
      retryDelay: (attempt) => Math.min(1000 * 2 ** attempt, 30000),
    },
  },
});
```

For the task list view, which needs pagination and server-side filtering:

```typescript
function useTaskList(filters: TaskFilters, page: number) {
  return useQuery({
    queryKey: ["tasks", filters, page],
    queryFn: () =>
      fetch(`/v1/tasks?${buildParams(filters, page)}`).then((r) => r.json()),
    // Keep previous page data visible during page transitions
    placeholderData: keepPreviousData,
  });
}
```

### 2.5 Optimistic Updates for Operator Actions

When an operator approves a task or resolves an approval, the UI should update immediately without waiting for the server round-trip:

```typescript
function useResolveApproval() {
  return useMutation({
    mutationFn: (args: { approvalId: string; decision: "approve" | "reject" }) =>
      fetch(`/v1/approvals/${args.approvalId}/resolve`, {
        method: "POST",
        body: JSON.stringify({ decision: args.decision }),
      }),
    onMutate: async ({ approvalId, decision }) => {
      // Cancel outgoing refetches
      await queryClient.cancelQueries({ queryKey: ["approvals"] });

      // Snapshot previous state for rollback
      const previous = queryClient.getQueryData(["approvals"]);

      // Optimistically remove from pending list
      queryClient.setQueryData(["approvals"], (old: Approval[]) =>
        old.filter((a) => a.approvalId !== approvalId)
      );

      return { previous };
    },
    onError: (_err, _vars, context) => {
      // Rollback on failure
      queryClient.setQueryData(["approvals"], context?.previous);
    },
    onSettled: () => {
      // Always refetch to ensure consistency
      queryClient.invalidateQueries({ queryKey: ["approvals"] });
    },
  });
}
```

### 2.6 Connection Status Indicator

Every real-time dashboard needs a visible connection indicator. Operators need to know when they are seeing stale data.

```tsx
function ConnectionStatusBadge() {
  const connected = useConnectionStatus();

  return (
    <Badge variant={connected ? "default" : "destructive"}>
      <span
        className={cn(
          "mr-1.5 inline-block h-2 w-2 rounded-full",
          connected ? "bg-green-500 animate-pulse" : "bg-red-500"
        )}
      />
      {connected ? "Live" : "Disconnected"}
    </Badge>
  );
}
```

Place this in the top-right of the header bar. When disconnected, optionally show a banner across the top of the main content area with a "Reconnecting..." message and a manual "Retry" button.

---

## 3. Key Views Every Operator Dashboard Needs

### 3.1 Overview Dashboard

The landing page. Shows aggregate health at a glance. Every AI agent platform (Langfuse, Helicone, Portkey, AgentOps) uses the same structure:

```
+------------------+------------------+------------------+------------------+
| Active Runs      | Pending          | Tasks Today      | Error Rate       |
| 12               | Approvals: 3     | 847              | 0.4%             |
| +8% from 1h ago  | URGENT           | +12% vs avg      | -0.1% vs avg     |
+------------------+------------------+------------------+------------------+
|                                     |                                     |
| Task Volume (line chart)            | Latency P50/P95 (line chart)        |
| [1h] [6h] [24h] [7d]               | [1h] [6h] [24h] [7d]               |
|                                     |                                     |
+-------------------------------------+-------------------------------------+
|                                     |                                     |
| Cost by Model (stacked bar)         | Recent Activity (event feed)        |
|                                     | 10:32 task_abc completed (2.1s)     |
|                                     | 10:31 approval_xyz APPROVED         |
|                                     | 10:30 run_def started               |
+-------------------------------------+-------------------------------------+
```

**Implementation notes:**

- **KPI cards**: Use shadcn `Card` with `size="sm"`. Show metric value, trend indicator (up/down arrow + percentage), and a sparkline.
- **Time-series charts**: Use Recharts `<AreaChart>` or `<LineChart>` inside shadcn chart wrappers. Time range selector as shadcn `Tabs` with `variant="line"` above each chart.
- **Activity feed**: A `ScrollArea` with fixed height. Each entry is a single line: timestamp + event icon + description + badge (state). New entries prepend with a subtle slide-in animation. The feed subscribes directly to the SSE store's `recentEvents`.

```tsx
// KPI card with trend
<Card size="sm">
  <CardHeader>
    <CardDescription>Active Runs</CardDescription>
    <CardTitle className="text-2xl tabular-nums">12</CardTitle>
    <CardAction>
      <Badge variant="outline" className="text-green-600">
        <ArrowUp className="h-3 w-3" /> 8%
      </Badge>
    </CardAction>
  </CardHeader>
  <CardContent>
    <Sparkline data={runCountHistory} className="h-8" />
  </CardContent>
</Card>
```

### 3.2 Run List View

A filterable, sortable, paginated table of agent runs. This is the most-visited view after the dashboard.

**Columns:**

| Column | Type | Notes |
|---|---|---|
| Status | Badge | `Running` (blue pulse), `Completed` (green), `Failed` (red), `Paused` (yellow) |
| Run ID | Monospace link | Click to open detail. Truncated with copy button. |
| Name/Label | Text | User-provided or auto-generated run name |
| Tasks | Count badge | `3/5` format (completed/total) |
| Duration | Duration | `2m 34s` format, live-updating for active runs |
| Cost | Currency | `$0.0043` format |
| Started | Relative time | `2m ago` with absolute tooltip |
| Agent | Text/Badge | Which agent configuration ran |

**Filters** (above the table):

- Status multi-select (checkboxes in a dropdown)
- Date range picker
- Agent selector
- Text search (run name, ID)
- Tag filter

### 3.3 Task Queue View

Similar to Run List but focused on individual tasks within runs. Critical columns:

| Column | Type | Notes |
|---|---|---|
| State | Badge | `Pending`, `Running`, `Completed`, `Failed`, `Paused`, `Cancelled` |
| Task ID | Monospace link | Click for detail panel |
| Title | Text | Human-readable task description |
| Parent Run | Link | Which run owns this task |
| Lease Owner | Text/Badge | Which worker claimed this task |
| Retry Count | Number | `0` default, red if > 2 |
| Created | Relative time | When the task entered the queue |

**Key interaction**: Clicking a task row opens the detail panel (right side) showing full task metadata, the span tree of operations within that task, inputs/outputs, and action buttons (cancel, retry, reassign).

### 3.4 Approval Queue View

The most operationally critical view. Pending approvals require human action.

**Layout**: Split into two sections via tabs:

- **Pending** (default, badge count): Items awaiting operator decision
- **Resolved**: Historical approvals with decision audit trail

**Pending approval card:**

```
+----------------------------------------------------------------+
| [ShieldAlert icon] Approve GitHub write action         URGENT   |
| Approval ID: appr_abc123                              2m ago    |
+----------------------------------------------------------------+
| Task: Draft weekly digest (task_xyz)                            |
| Agent wants to create a PR in repo cairn-rs/main                |
|                                                                 |
| Context:                                                        |
| - Files modified: 3                                             |
| - Estimated cost: $0.02                                         |
| - Risk level: Medium                                            |
+----------------------------------------------------------------+
| [Approve]  [Reject]  [View Details]                             |
+----------------------------------------------------------------+
```

**Key patterns from research:**

- Langfuse uses **Annotation Queues** for batch review -- items are queued, reviewers work through them sequentially
- Portkey uses **Guardrails** as automated approval gates with pass/fail verdicts
- No existing platform has first-class pause/approve/resume for live agents -- this is Cairn's differentiator

**Implementation**: Approval cards should be full-width in the main content area (not crammed into a table). Use shadcn `Card` with colored left border (yellow for pending, green for approved, red for rejected). The Approve/Reject buttons use `useMutation` with optimistic removal from the pending list.

### 3.5 Trace Detail / Span Tree View

The primary debugging view. Two-panel layout:

```
+---------------------------+------------------------------------+
| Span Tree (left)          | Span Detail (right)                |
|                           |                                    |
| v Root span (3.2s)        | Input:                             |
|   v LLM: claude (2.1s)   |   {"messages": [...]}              |
|     Tool: search (0.4s)   |                                    |
|     Tool: write (0.2s)    | Output:                            |
|   v LLM: claude (0.8s)   |   {"content": "..."}               |
|     Tool: submit (0.1s)   |                                    |
|                           | Model: claude-sonnet-4-5            |
|                           | Tokens: 1,234 in / 567 out         |
|                           | Cost: $0.0043                      |
|                           | Duration: 2.1s                     |
|                           |                                    |
|                           | [Copy Input] [Copy Output] [Replay]|
+---------------------------+------------------------------------+
```

**Span tree implementation:**

- Each node: icon (color-coded by span type) + name + duration badge
- Click to select -> populates right panel
- Expand/collapse for child spans
- Visual nesting via indentation (16px per level)
- Active/in-progress spans pulse with a subtle animation

**Color coding convention (derived from Langfuse, Phoenix, AgentOps):**

| Span Type | Color | Icon |
|---|---|---|
| LLM call | Purple/Blue | `Brain` or `Sparkles` |
| Tool invocation | Green | `Wrench` |
| Retrieval/RAG | Orange | `Search` |
| Custom/function | Gray | `Code` |
| Error | Red | `AlertTriangle` |
| Human annotation | Yellow | `User` |

### 3.6 Live Event Log (Tail View)

A scrolling feed of raw events as they arrive via SSE. Similar to Pydantic Logfire's "Live" view and Laminar's real-time search.

```tsx
function LiveEventLog() {
  const { recentEvents } = useSseState();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = 0; // Newest at top
    }
  }, [recentEvents, autoScroll]);

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between border-b px-4 py-2">
        <h2 className="text-sm font-medium">Live Events</h2>
        <div className="flex items-center gap-2">
          <Switch checked={autoScroll} onCheckedChange={setAutoScroll} />
          <span className="text-xs text-muted-foreground">Auto-scroll</span>
        </div>
      </div>
      <ScrollArea ref={scrollRef} className="flex-1">
        {recentEvents.map((event) => (
          <EventRow key={event.id} event={event} />
        ))}
      </ScrollArea>
    </div>
  );
}
```

Each event row should be a single dense line: `[timestamp] [event-type badge] [summary text] [id]`. Monospace font for IDs and timestamps. Event type badges use the same color coding as span types.

---

## 4. Data Table Patterns

### 4.1 TanStack Table + shadcn/ui DataTable

The industry standard for operator dashboards. shadcn/ui provides a guide (not a component) for building data tables with TanStack Table. The pattern:

**File structure:**

```
components/
  tasks/
    columns.tsx          # ColumnDef[] -- column definitions
    data-table.tsx       # Reusable DataTable shell
    data-table-toolbar.tsx  # Filters, search, view options
    data-table-pagination.tsx
    data-table-row-actions.tsx
  page.tsx               # Data fetching + composition
```

**Column definitions pattern:**

```typescript
// columns.tsx
export const columns: ColumnDef<TaskRecord>[] = [
  {
    id: "select",
    header: ({ table }) => (
      <Checkbox
        checked={table.getIsAllPageRowsSelected()}
        onCheckedChange={(value) => table.toggleAllPageRowsSelected(!!value)}
      />
    ),
    cell: ({ row }) => (
      <Checkbox
        checked={row.getIsSelected()}
        onCheckedChange={(value) => row.toggleSelected(!!value)}
      />
    ),
    enableSorting: false,
    enableHiding: false,
  },
  {
    accessorKey: "state",
    header: "Status",
    cell: ({ row }) => <TaskStateBadge state={row.getValue("state")} />,
    filterFn: (row, id, filterValues) =>
      filterValues.includes(row.getValue(id)),
  },
  {
    accessorKey: "taskId",
    header: "Task ID",
    cell: ({ row }) => (
      <code className="text-xs font-mono">
        {truncateId(row.getValue("taskId"))}
      </code>
    ),
  },
  {
    accessorKey: "title",
    header: ({ column }) => (
      <DataTableColumnHeader column={column} title="Title" />
    ),
  },
  {
    accessorKey: "duration",
    header: ({ column }) => (
      <DataTableColumnHeader column={column} title="Duration" />
    ),
    cell: ({ row }) => formatDuration(row.getValue("duration")),
  },
  {
    accessorKey: "createdAt",
    header: ({ column }) => (
      <DataTableColumnHeader column={column} title="Created" />
    ),
    cell: ({ row }) => <RelativeTime timestamp={row.getValue("createdAt")} />,
  },
  {
    id: "actions",
    cell: ({ row }) => <DataTableRowActions row={row} />,
  },
];
```

### 4.2 Key DataTable Features for Operator Dashboards

**Must-have:**

- **Server-side pagination**: Operator tables can have thousands of rows. Never load all data client-side. Use `manualPagination: true` with TanStack Table.
- **Column sorting**: Click header to sort. Use `DataTableColumnHeader` component with sort indicators.
- **Multi-select filtering**: Status filter as a multi-checkbox dropdown. Model filter similarly.
- **Row selection**: Checkbox column for batch actions (bulk approve, bulk retry, bulk cancel).
- **Column visibility toggle**: Let operators hide columns they don't need. Use the `DataTableViewOptions` pattern from shadcn.
- **Sticky header**: Table header stays visible while scrolling. Apply `sticky top-0 bg-background` classes.

**Nice-to-have:**

- **Column pinning**: Pin ID and Status columns to the left so they stay visible during horizontal scroll.
- **Row expansion**: Click a row to expand an inline detail section without navigating away.
- **Keyboard navigation**: Arrow keys to move between rows, Enter to open detail, Escape to close.
- **Virtualization**: For tables exceeding ~500 visible rows, use TanStack Virtual to only render visible DOM nodes.

### 4.3 Filter Bar Pattern

Place above the data table. Horizontal layout with filters left-aligned and view options right-aligned:

```
+------------------------------------------------------------------+
| [Status v] [Agent v] [Date Range] [Search...]  | [Columns] [Export] |
+------------------------------------------------------------------+
```

Each filter is a shadcn `Popover` with a `Command` list inside for searchable multi-select. Active filters show as removable `Badge` pills below the filter bar.

```tsx
<div className="flex items-center gap-2">
  <DataTableFacetedFilter
    column={table.getColumn("state")}
    title="Status"
    options={[
      { label: "Running", value: "running", icon: Loader2 },
      { label: "Completed", value: "completed", icon: CheckCircle },
      { label: "Failed", value: "failed", icon: XCircle },
      { label: "Paused", value: "paused", icon: Pause },
    ]}
  />
  <DataTableFacetedFilter
    column={table.getColumn("agent")}
    title="Agent"
    options={agentOptions}
  />
  <Input
    placeholder="Search tasks..."
    value={globalFilter}
    onChange={(e) => setGlobalFilter(e.target.value)}
    className="h-8 w-48"
  />
  <div className="ml-auto flex gap-2">
    <DataTableViewOptions table={table} />
    <Button variant="outline" size="sm">
      <Download className="h-3.5 w-3.5 mr-1" /> Export
    </Button>
  </div>
</div>
```

### 4.4 Status Badge Component

Reuse across every table and detail view:

```tsx
const stateConfig: Record<string, { label: string; variant: string; icon: any }> = {
  pending:   { label: "Pending",   variant: "outline",     icon: Clock },
  running:   { label: "Running",   variant: "default",     icon: Loader2 },
  completed: { label: "Completed", variant: "secondary",   icon: CheckCircle },
  failed:    { label: "Failed",    variant: "destructive",  icon: XCircle },
  paused:    { label: "Paused",    variant: "outline",     icon: Pause },
  cancelled: { label: "Cancelled", variant: "ghost",       icon: Ban },
};

function TaskStateBadge({ state }: { state: string }) {
  const config = stateConfig[state];
  const Icon = config.icon;
  return (
    <Badge variant={config.variant}>
      <Icon className={cn("h-3 w-3", state === "running" && "animate-spin")} />
      {config.label}
    </Badge>
  );
}
```

---

## 5. Component Library & Assembly

### 5.1 Recommended Stack

| Layer | Choice | Why |
|---|---|---|
| **Component primitives** | shadcn/ui (Radix UI + Tailwind) | Accessible, copy-paste, fully customizable. Used by Langfuse, Vercel. |
| **Data tables** | TanStack Table v8 | Headless, server-side pagination, sorting, filtering, virtualization. |
| **Charts** | Recharts (via shadcn chart wrapper) | React-native, declarative, composable. Covers line/area/bar/pie. |
| **Dense charts** | Tremor (optional) | 300+ pre-built blocks. KPI cards, sparklines, trackers. Tailwind-native. |
| **Command palette** | cmdk (via shadcn Command) | Fast keyboard-driven navigation and actions. |
| **Icons** | Lucide React | Consistent, tree-shakeable, used by shadcn. |
| **Date handling** | date-fns | Lightweight, tree-shakeable. `formatDistanceToNow` for relative times. |
| **Data fetching** | TanStack Query v5 | Caching, dedup, optimistic updates, SSR hydration. |
| **SSE state** | Custom store + useSyncExternalStore | Concurrent-safe, React-idiomatic. |

### 5.2 shadcn/ui Components Used in Operator Dashboards

**Layout:** `Sidebar`, `SidebarProvider`, `SidebarInset`, `ResizablePanelGroup`, `ResizablePanel`, `ResizableHandle`

**Navigation:** `Tabs` (view switching), `Breadcrumb` (location), `Command`/`CommandDialog` (command palette)

**Data display:** `Card` (KPI metrics), `Table` (data tables), `Badge` (status indicators), `Tooltip` (hover info), `ScrollArea` (scrollable panels)

**Interaction:** `Dialog` (confirmations), `Sheet` (mobile detail panels), `Popover` (filter dropdowns), `Select` (simple selects), `Button`, `Checkbox`, `Switch`

**Feedback:** `Skeleton` (loading states), `Alert` (error/warning banners), `Sonner`/toast (notifications)

### 5.3 Chart Patterns for Dashboards

**Time-series line/area chart (cost, latency, volume):**

```tsx
<Card>
  <CardHeader>
    <CardTitle>Task Volume</CardTitle>
    <CardAction>
      <Tabs defaultValue="24h" variant="line">
        <TabsList>
          <TabsTrigger value="1h">1h</TabsTrigger>
          <TabsTrigger value="6h">6h</TabsTrigger>
          <TabsTrigger value="24h">24h</TabsTrigger>
          <TabsTrigger value="7d">7d</TabsTrigger>
        </TabsList>
      </Tabs>
    </CardAction>
  </CardHeader>
  <CardContent>
    <ResponsiveContainer width="100%" height={200}>
      <AreaChart data={volumeData}>
        <defs>
          <linearGradient id="fillVolume" x1="0" y1="0" x2="0" y2="1">
            <stop offset="5%" stopColor="hsl(var(--primary))" stopOpacity={0.3} />
            <stop offset="95%" stopColor="hsl(var(--primary))" stopOpacity={0} />
          </linearGradient>
        </defs>
        <XAxis
          dataKey="time"
          tickFormatter={(t) => format(t, "HH:mm")}
          className="text-xs"
        />
        <YAxis className="text-xs" />
        <Tooltip content={<ChartTooltip />} />
        <Area
          type="monotone"
          dataKey="count"
          stroke="hsl(var(--primary))"
          fill="url(#fillVolume)"
        />
      </AreaChart>
    </ResponsiveContainer>
  </CardContent>
</Card>
```

**In-table sparklines:** For showing trends in compact table cells, render a tiny Recharts `<LineChart>` (64px wide, 24px tall) with no axes, grid, or tooltip. Just the line.

### 5.4 Tremor Blocks (Optional Accelerator)

Tremor provides 300+ copy-paste blocks specifically for dashboards. Relevant categories:

- **KPI Cards** (29 blocks): Metric + trend + sparkline in various layouts
- **Chart Compositions**: Area, bar, line with proper legends and tooltips
- **Filter Bars**: Pre-built date range and faceted filter layouts
- **Tables**: With pagination, actions, and expandable rows
- **Page Shells**: Complete dashboard page layouts with sidebar

Tremor requires React 18.2+ and Tailwind CSS v4. All blocks support light and dark modes. Use Tremor blocks as starting points, then customize to match the project's exact needs.

---

## 6. Dark Mode & Dense Information Display

### 6.1 Dark-First Design

Operator dashboards are used in long monitoring sessions. Dark mode reduces eye strain and is preferred by platform engineers. Ship dark as the default.

**Implementation with shadcn/ui + next-themes:**

```tsx
// components/theme-provider.tsx
import { ThemeProvider as NextThemesProvider } from "next-themes";

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  return (
    <NextThemesProvider
      attribute="class"
      defaultTheme="dark"       // Dark as default for operator tools
      enableSystem               // Respect OS preference
      disableTransitionOnChange  // No flash during theme switch
    >
      {children}
    </NextThemesProvider>
  );
}
```

For a Vite-based SPA (not Next.js), use the `class` attribute approach directly. Set `<html class="dark">` as the default and use a theme toggle that persists to `localStorage`.

**CSS variables approach:** shadcn/ui uses CSS custom properties for all colors. Define both light and dark palettes in your global CSS:

```css
@layer base {
  :root {
    --background: 0 0% 100%;
    --foreground: 240 10% 3.9%;
    --card: 0 0% 100%;
    --muted: 240 4.8% 95.9%;
    --primary: 240 5.9% 10%;
    /* ... */
  }

  .dark {
    --background: 240 10% 3.9%;
    --foreground: 0 0% 98%;
    --card: 240 10% 3.9%;
    --muted: 240 3.7% 15.9%;
    --primary: 0 0% 98%;
    /* ... */
  }
}
```

### 6.2 Dense Information Display Principles

Operator dashboards are not consumer products. Information density is a feature, not a bug. Platform engineers need to see as much relevant data as possible without scrolling.

**Typography:**

- Base font size: 13-14px (not the typical 16px)
- Table cell text: 12-13px with `tabular-nums` for aligned numbers
- Monospace for IDs, timestamps, JSON: `font-mono text-xs`
- Line height: 1.4 for body, 1.2 for table cells (tighter than consumer apps)

**Spacing:**

- Reduce default padding. shadcn components have reasonable defaults but operator UIs benefit from tighter spacing.
- Table row height: 32-36px (shadcn default is 48px -- override with `[&_tr]:h-8`)
- Card padding: `p-3` instead of `p-6`
- Gap between cards: `gap-3` instead of `gap-6`

```css
/* Operator density overrides */
.operator-dense {
  --card-padding: 0.75rem;
  font-size: 13px;
}

.operator-dense [data-slot="table"] tr {
  height: 2rem;
}

.operator-dense [data-slot="badge"] {
  font-size: 0.6875rem;
  padding: 0 0.375rem;
  height: 1.25rem;
}
```

### 6.3 Color Usage in Dark Mode

- **Background hierarchy**: Use 3-4 levels of background darkness to create depth. Base (`--background`), surface (`--card`), elevated (`--muted`), accent.
- **Status colors**: Must be distinguishable in both themes. Test with colorblind simulation. Use icons alongside colors (never color alone for status).
- **Borders**: Subtle (`border-border/50`) -- borders in dark mode should barely be visible, creating separation through shadow/elevation instead.
- **Text hierarchy**: Primary (`--foreground`), secondary (`--muted-foreground`), disabled (`--muted-foreground/50`).

### 6.4 Effective Dashboard Density Patterns

**KPI row**: 4 cards in a row, each 25% width, `size="sm"`. Shows metric value, trend badge, and micro-sparkline. Total height: ~80px.

**Chart row**: 2 charts side by side, each 50% width. Height: 200px. No legends -- use direct line labels or tooltips only.

**Table**: Occupies remaining vertical space. Sticky header. No wrapping in cells -- truncate with `truncate` class and show full content in tooltip.

**Scrolling strategy**: The page itself does not scroll. The sidebar is fixed. The main content area has a fixed header (page title + filters) and a scrollable body (table/content). This ensures filters and navigation are always visible.

```tsx
<div className="flex h-screen">
  <Sidebar /> {/* Fixed */}
  <div className="flex flex-1 flex-col overflow-hidden">
    <Header /> {/* Fixed, ~48px */}
    <div className="flex-1 overflow-auto p-4">
      <KpiRow />      {/* ~80px */}
      <ChartRow />    {/* ~200px */}
      <DataTable />   {/* Fills remaining space */}
    </div>
  </div>
</div>
```

---

## 7. Keyboard-First & Power-User Patterns

### 7.1 Command Palette

The single most impactful power-user feature. Maps to `Cmd+K` (Mac) / `Ctrl+K` (Windows). Opens a shadcn `CommandDialog` with:

- **Navigation**: Jump to any view (Dashboard, Runs, Tasks, Approvals, Settings)
- **Search**: Find runs/tasks by ID or name
- **Actions**: Approve pending items, retry failed tasks, toggle theme
- **Recent**: Show recently viewed items

```tsx
function CommandPalette() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((prev) => !prev);
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, []);

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Search or jump to..." />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>
        <CommandGroup heading="Navigation">
          <CommandItem onSelect={() => navigate("/dashboard")}>
            <LayoutDashboard className="mr-2 h-4 w-4" /> Dashboard
          </CommandItem>
          <CommandItem onSelect={() => navigate("/runs")}>
            <Play className="mr-2 h-4 w-4" /> Runs
          </CommandItem>
          {/* ... */}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Actions">
          <CommandItem onSelect={approveAllPending}>
            <ShieldCheck className="mr-2 h-4 w-4" /> Approve all pending
            <CommandShortcut>Ctrl+Shift+A</CommandShortcut>
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
```

### 7.2 Global Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Cmd+K` / `Ctrl+K` | Open command palette |
| `Cmd+B` / `Ctrl+B` | Toggle sidebar |
| `g` then `d` | Go to Dashboard |
| `g` then `r` | Go to Runs |
| `g` then `t` | Go to Tasks |
| `g` then `a` | Go to Approvals |
| `Escape` | Close detail panel / dialog |
| `j` / `k` | Navigate table rows (vim-style) |
| `Enter` | Open selected row's detail |
| `?` | Show keyboard shortcut help |

### 7.3 URL-Driven State

Every view state should be reflected in the URL. This enables:

- **Shareable links**: An operator can share a URL to a specific filtered view
- **Browser back/forward**: Navigating filter changes with browser history
- **Bookmarkable views**: Save common filter combinations

Encode filter state in URL search params: `/tasks?state=running,paused&agent=writer&page=2&sort=created_at:desc`

---

## 8. Cairn-Specific Implementation Notes

### 8.1 Mapping Cairn SSE Events to Dashboard Views

Based on the existing `SseEventName` variants and `map_event_to_sse_name` in `cairn-api/src/sse_publisher.rs`:

| SSE Event | Dashboard Update |
|---|---|
| `ready` | Set connection status to "Live", display `clientId` |
| `task_update` | Refresh task table, update KPI card count, update run progress |
| `approval_required` | Increment approval badge, prepend to approval queue, show toast |
| `assistant_tool_call` | Append to trace span tree, update tool call log |
| `agent_progress` | Update run timeline, refresh activity feed |
| `assistant_delta` | Stream text into chat/log view |
| `assistant_end` | Finalize streaming text block |

### 8.2 Reconnection with lastEventId

Cairn's SSE endpoint already supports `lastEventId`-based replay via `SseReplayQuery`. The frontend EventSource automatically sends the `Last-Event-ID` header on reconnection. The SSE store should track `lastEventId` from each received event so that:

1. On reconnect, the browser sends the last seen ID
2. The server replays events since that position
3. The store processes replayed events normally (dedup by checking if event ID is already seen)

```typescript
// In the SSE store connect() method:
this.eventSource.addEventListener("task_update", (e) => {
  const eventId = e.lastEventId;
  if (this.seenIds.has(eventId)) return; // Dedup replayed events
  this.seenIds.add(eventId);
  // ... process event
});
```

### 8.3 Build Pipeline

The dashboard frontend lives at `crates/cairn-app/frontend/` and embeds into the Rust binary via `rust-embed`:

```bash
# Development (two terminals)
cargo run --bin cairn-app                          # Backend on :3000
cd crates/cairn-app/frontend && npm run dev        # Vite on :5173, proxies /v1/* to :3000

# Production
cd crates/cairn-app/frontend && npm run build      # Output to dist/
cargo build --release                               # Embeds dist/ via rust-embed
./target/release/cairn-app                          # Dashboard at :3000/
```

### 8.4 File Structure

```
crates/cairn-app/frontend/
  src/
    components/
      layout/
        app-sidebar.tsx         # Sidebar nav with badge counts
        header.tsx              # Breadcrumb + connection status + command palette
        detail-panel.tsx        # Resizable right panel wrapper
      dashboard/
        kpi-cards.tsx           # 4-up metric cards
        volume-chart.tsx        # Task volume over time
        cost-chart.tsx          # Cost by model
        activity-feed.tsx       # Live event stream
      runs/
        columns.tsx             # TanStack Table column defs
        data-table.tsx          # DataTable shell
        run-detail.tsx          # Run inspector panel
      tasks/
        columns.tsx
        data-table.tsx
        task-detail.tsx
      approvals/
        approval-queue.tsx      # Card-based approval list
        approval-card.tsx       # Individual approval with actions
      traces/
        span-tree.tsx           # Collapsible span hierarchy
        span-detail.tsx         # Input/output/metadata viewer
      shared/
        status-badge.tsx        # Reusable state badge
        relative-time.tsx       # "2m ago" component
        connection-status.tsx   # Live/Disconnected indicator
        command-palette.tsx     # Cmd+K command menu
      ui/                       # shadcn/ui generated components
    hooks/
      use-sse.ts               # useSyncExternalStore + EventSource
      use-tasks.ts             # TanStack Query for /v1/tasks
      use-runs.ts              # TanStack Query for /v1/runs
      use-approvals.ts         # TanStack Query for /v1/approvals
    lib/
      sse-store.ts             # SSE state management
      api-client.ts            # REST client for /v1/*
      utils.ts                 # cn(), formatDuration, truncateId
    App.tsx
    main.tsx
    index.css                  # Tailwind imports + dark mode variables
  dist/                        # Vite build output (embedded by rust-embed)
  package.json
  vite.config.ts
  tsconfig.json
  components.json              # shadcn/ui config
```

---

## Sources

| # | Source | URL | Key Contribution |
|---|---|---|---|
| 1 | Langfuse Docs | langfuse.com/docs | Trace tree, sessions, annotation queues, dashboard metrics |
| 2 | Langfuse Tracing | langfuse.com/docs/tracing | Nested observation UI, timing/cost per span |
| 3 | Langfuse Analytics | langfuse.com/docs/analytics/overview | Cost/latency/volume dimensions, custom dashboards |
| 4 | Langfuse GitHub | github.com/langfuse/langfuse | Tech stack: Next.js, ClickHouse, monorepo |
| 5 | Helicone | helicone.ai | Real-time request logging, session debugging, cost tracking |
| 6 | Helicone GitHub | github.com/Helicone/helicone | Tech stack: Next.js, ClickHouse, Supabase, TypeScript 91% |
| 7 | Arize Phoenix | arize.com/docs/phoenix | Trace/span visualization, experiments, prompt playground |
| 8 | LangSmith | docs.langchain.com/langsmith | Run tree model, threads, observability concepts |
| 9 | shadcn/ui Data Table | ui.shadcn.com/docs/components/data-table | TanStack Table guide, column defs, pagination, filtering, sorting |
| 10 | shadcn/ui Sidebar | ui.shadcn.com/docs/components/sidebar | Collapsible sidebar, SidebarProvider, variants, mobile |
| 11 | shadcn/ui Dashboard Example | ui.shadcn.com/examples/dashboard | Full layout: sidebar, KPI cards, charts, data tables |
| 12 | shadcn/ui Dark Mode | ui.shadcn.com/docs/dark-mode/next | next-themes setup, CSS variables, system preference |
| 13 | TanStack Table | tanstack.com/table/latest | Headless table: sorting, filtering, pinning, virtualization |
| 14 | MDN EventSource | developer.mozilla.org/en-US/docs/Web/API/EventSource | SSE API: constructor, events, reconnection, readyState |
| 15 | React useSyncExternalStore | react.dev/reference/react/useSyncExternalStore | External store subscription, snapshot pattern, concurrent safety |
| 16 | TanStack Query + WebSockets | tkdodo.eu/blog/using-web-sockets-with-react-query | Query invalidation from real-time events, staleTime: Infinity |
| 17 | TanStack Query Overview | tanstack.com/query/latest | Polling, optimistic updates, window focus refetch |
| 18 | SWR Revalidation | swr.vercel.app/docs/revalidation | refreshInterval polling, focus revalidation, immutable data |
| 19 | Tremor | tremor.so | 300+ dashboard blocks, KPI cards, charts, filter bars |
| 20 | shadcn/ui Resizable | ui.shadcn.com/docs/components/resizable | ResizablePanelGroup for multi-panel layouts |
| 21 | shadcn/ui Command | ui.shadcn.com/docs/components/command | cmdk-based command palette, keyboard shortcuts |
| 22 | shadcn/ui Card | ui.shadcn.com/docs/components/card | Card composition, size variants, KPI layout |
| 23 | shadcn/ui Badge | ui.shadcn.com/docs/components/badge | 5 variants, icon support, status display |
| 24 | shadcn/ui Tabs | ui.shadcn.com/docs/components/tabs | View switching, line variant, vertical orientation |
| 25 | shadcn/ui Sheet | ui.shadcn.com/docs/components/sheet | Side panel overlay for mobile detail views |
| 26 | shadcn/ui ScrollArea | ui.shadcn.com/docs/components/scroll-area | Custom scrollbars for live event logs |
