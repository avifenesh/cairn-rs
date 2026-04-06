# cairn-rs Developer Guide

Practical reference for contributing to the cairn-rs codebase.

---

## 1. Architecture Overview

### Workspace layout

```
crates/
  cairn-domain    — types, events, IDs, lifecycle state machines
  cairn-store     — InMemoryStore, EventLog trait, projections
  cairn-runtime   — SessionService, RunService, TaskService, providers
  cairn-api       — shared HTTP types (request/response shapes)
  cairn-app       — axum binary: router, handlers, AppState
  cairn-memory    — document ingestion, retrieval, chunking
  cairn-graph     — graph projection, RFC 004 query service
  cairn-evals     — prompt evaluation framework
  cairn-tools     — tool invocation adapter
  cairn-channels  — notification channels (webhook, Slack, email)
  cairn-signal    — external signal ingestion
ui/               — Vite + React 19 + Tailwind v4 operator dashboard
```

### Event sourcing

Every state change appends an immutable `StoredEvent` to the log.
Projections (read models) are derived by replaying the log.

```
POST /v1/events/append
  -> EventEnvelope<RuntimeEvent>
  -> InMemoryStore::append()
     -> updates projections (RunReadModel, TaskReadModel, …)
     -> broadcasts to SSE channel
     -> notifies WebSocket subscribers
```

Key types:

```rust
// cairn-domain/src/events.rs
pub enum RuntimeEvent {
    SessionCreated(SessionCreated),
    RunCreated(RunCreated),
    RunStateChanged(RunStateChanged),
    TaskCreated(TaskCreated),
    ApprovalRequested(ApprovalRequested),
    // … 40+ variants
}

pub struct EventEnvelope<E> {
    pub event_id:     EventId,
    pub causation_id: Option<CausationId>,
    pub payload:      E,
}
```

Projections live in `cairn-store/src/projections/` and implement
`IntoProjection` or are updated inside `InMemoryStore::apply_projection`.

---

## 2. Adding a New API Endpoint

### Step 1 — define request/response types

Add structs to `crates/cairn-api/src/` or inline in `main.rs` for
one-off shapes:

```rust
#[derive(Deserialize)]
struct CreateWidgetBody {
    name:    String,
    kind:    String,
    tenant_id: Option<String>,
}

#[derive(Serialize)]
struct WidgetRecord {
    widget_id:  String,
    name:       String,
    created_at: u64,
}
```

### Step 2 — write the handler

Add the async handler function to `crates/cairn-app/src/main.rs`.
Follow the existing conventions:

```rust
/// `POST /v1/widgets` — create a new widget.
async fn create_widget_handler(
    State(state): State<AppState>,
    Json(body): Json<CreateWidgetBody>,
) -> impl axum::response::IntoResponse {
    let tenant_id = body.tenant_id.as_deref().unwrap_or("default");
    // … business logic using state.runtime …
    (StatusCode::CREATED, Json(WidgetRecord {
        widget_id:  format!("wgt_{}", uuid::Uuid::new_v4()),
        name:       body.name,
        created_at: now_unix_ms(),
    }))
}
```

### Step 3 — register the route

Find `fn build_router(state: AppState) -> Router` and add:

```rust
.route("/v1/widgets", post(create_widget_handler))
```

Routes requiring auth are automatically covered — the `auth_middleware`
guards all `/v1/*` paths (except `/v1/stream`, `/v1/ws`).

### Step 4 — add to the UI API client

In `ui/src/lib/api.ts`, inside `createApiClient`:

```typescript
createWidget: (body: { name: string; kind: string }): Promise<WidgetRecord> =>
  post("/v1/widgets", body),
```

### Step 5 — write a test

Inside the `#[cfg(test)]` block in `main.rs`:

```rust
#[tokio::test]
async fn create_widget_returns_201() {
    let state = make_state();
    let app   = make_app(state);
    let body  = serde_json::json!({ "name": "my-widget", "kind": "basic" });
    let resp  = authed_post(app, "/v1/widgets", body).await;
    assert_eq!(resp.status(), 201);
    let json: serde_json::Value = parse_body(resp).await;
    assert_eq!(json["name"], "my-widget");
}
```

---

## 3. Adding a New UI Page

### Step 1 — create the page component

```
ui/src/pages/WidgetsPage.tsx
```

```typescript
import { useQuery } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";

export function WidgetsPage() {
  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["widgets"],
    queryFn:  () => defaultApi.getWidgets(),
    refetchInterval: 30_000,
  });

  if (isError) return (
    <ErrorFallback error={error} resource="widgets" onRetry={() => void refetch()} />
  );

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-zinc-800 shrink-0">
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
          Widgets
        </span>
      </div>
      {/* Content */}
      <div className="flex-1 overflow-y-auto p-5">
        {isLoading ? (
          <div className="flex items-center gap-2 text-zinc-600">
            <Loader2 size={14} className="animate-spin" />
            <span className="text-[13px]">Loading...</span>
          </div>
        ) : (
          <pre className="text-zinc-400 text-[12px]">
            {JSON.stringify(data, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}

export default WidgetsPage;
```

### Step 2 — add the route id to Sidebar.tsx

In `ui/src/components/Sidebar.tsx`:

```typescript
// NavPage union
| 'widgets'

// NAV_GROUPS — pick the right group
{ id: 'widgets', label: 'Widgets', icon: Boxes }
```

### Step 3 — register in Layout.tsx

```typescript
// VALID_PAGES array
'widgets',

// PAGE_TITLES
widgets: 'Widgets',

// PAGE_GROUP (breadcrumb)
widgets: 'Infrastructure',
```

### Step 4 — add the lazy import and route case in App.tsx

```typescript
const WidgetsPage = lazy(() =>
  import('./pages/WidgetsPage').then(m => ({ default: m.WidgetsPage }))
);

// inside renderRoute switch
case 'widgets': return <Guarded name="Widgets"><WidgetsPage /></Guarded>;
```

### Step 5 — add to CommandPalette

In `ui/src/components/CommandPalette.tsx` NAV_OPTIONS array:

```typescript
{ kind: 'page', id: 'widgets', label: 'Widgets',
  description: 'Widget management', icon: Boxes },
```

---

## 4. Adding a New Event Type

### Step 1 — add the payload struct to cairn-domain

In `crates/cairn-domain/src/events.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetCreated {
    pub project:   ProjectKey,
    pub widget_id: WidgetId,
    pub name:      String,
}
```

### Step 2 — add the variant to RuntimeEvent

```rust
pub enum RuntimeEvent {
    // … existing …
    WidgetCreated(WidgetCreated),
}
```

### Step 3 — handle in event_type_name (main.rs)

```rust
E::WidgetCreated(_) => "widget_created",
```

### Step 4 — update match exhaustiveness

The Rust compiler will point you to every `match` on `RuntimeEvent`
that needs a new arm. Add `E::WidgetCreated(_) => { ... }` to each.

Key locations: `apply_projection` in `cairn-store/src/in_memory.rs`,
`event_message` in `main.rs`, any audit/graph projectors.

### Step 5 — append the event in a handler

```rust
let envelope = EventEnvelope::for_runtime_event(
    EventId::new(format!("wgt_evt_{}", uuid::Uuid::new_v4())),
    None, // causation_id
    RuntimeEvent::WidgetCreated(WidgetCreated {
        project:   project.clone(),
        widget_id: widget_id.clone(),
        name:      body.name.clone(),
    }),
);
state.runtime.store.append(&[envelope]).await?;
```

---

## 5. Testing Strategy

### Unit tests

Pure logic in domain crates, no I/O. Run with `cargo test -p cairn-domain`.

```rust
#[test]
fn run_state_completed_is_terminal() {
    assert!(RunState::Completed.is_terminal());
    assert!(!RunState::Running.is_terminal());
}
```

### Integration tests (in-process, no network)

Most cairn-app tests spin up the full axum router backed by
`InMemoryStore`. Use the `make_state()` / `make_app()` helpers:

```rust
#[tokio::test]
async fn list_sessions_returns_created_session() {
    let state = make_state();
    let app   = make_app(state);

    // Create via append
    let _ = authed_post(app.clone(), "/v1/sessions", json!({
        "session_id": "s1", "tenant_id": "t", "workspace_id": "w", "project_id": "p"
    })).await;

    let resp = authed_get(app, "/v1/sessions").await;
    assert_eq!(resp.status(), 200);
    let body: Vec<serde_json::Value> = parse_body(resp).await;
    assert_eq!(body[0]["session_id"], "s1");
}
```

### End-to-end tests

`scripts/agent-sim.sh` exercises the full lifecycle against a running
server: session -> run -> tasks (claim/release) -> approval -> completion.

```bash
CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=<token> bash scripts/agent-sim.sh
```

`scripts/smoke-test.sh` covers HTTP contract compliance for every
registered route.

---

## 6. Local Development Setup with Ollama

```bash
# 1. Install Rust (stable toolchain)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Install Node 22
nvm install 22

# 3. Install Ollama
curl -fsSL https://ollama.ai/install.sh | sh
ollama serve &
ollama pull llama3.2        # chat model
ollama pull nomic-embed-text # embedding model

# 4. Start the server
CAIRN_ADMIN_TOKEN=dev OLLAMA_HOST=http://localhost:11434 \
  cargo run -p cairn-app -- --addr 0.0.0.0 --port 3000

# 5. Start the UI dev server (hot reload)
cd ui && npm install && npm run dev
# UI available at http://localhost:5173
# API proxied to http://localhost:3000
```

The `ui/vite.config.ts` proxy forwards `/health` and `/v1/*` to the
Rust backend, so the UI dev server needs the backend running.

---

## 7. Docker Development Workflow

### Build and run (single command)

```bash
docker compose up --build
```

This builds a three-stage image (Node for the UI, Rust for the binary,
Debian slim for the runtime) and starts both cairn and Ollama.

### Pull a model into the Docker Ollama instance

```bash
docker compose exec ollama ollama pull llama3.2
```

### Override the admin token

```bash
echo 'CAIRN_ADMIN_TOKEN=my-secret' > .env
docker compose up -d
```

### Persistent SQLite state

Uncomment the volume lines in `docker-compose.yml`:

```yaml
cairn:
  volumes:
    - cairn_data:/data
  command: ["--db", "/data/cairn.db"]
```

---

## 8. Common Patterns

### AppState — shared server state

`AppState` is `Clone` (cheap — all fields are `Arc`). Inject it into
handlers via the `State` extractor:

```rust
async fn my_handler(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.started_at.elapsed().as_secs();
    Json(json!({ "uptime": uptime }))
}
```

Fields of note:

| Field | Type | Purpose |
|---|---|---|
| `runtime` | `Arc<InMemoryServices>` | sessions, runs, tasks, approvals |
| `ollama` | `Option<Arc<OllamaProvider>>` | local LLM, None if unconfigured |
| `request_log` | `Arc<RwLock<RequestLogBuffer>>` | structured request ring buffer |
| `notifications` | `Arc<RwLock<NotificationBuffer>>` | operator notification queue |
| `metrics` | `Arc<RwLock<AppMetrics>>` | rolling latency percentiles |

### Auth middleware

All `/v1/*` routes go through `auth_middleware`. Exempt paths
(`/health`, `/v1/stream`, `/v1/ws`) skip it. Handlers needing the
resolved principal can pull it from extensions:

```rust
let principal = request.extensions().get::<AuthPrincipal>().cloned();
```

To add a new public (no-auth) path, extend `is_auth_exempt`:

```rust
fn is_auth_exempt(path: &str) -> bool {
    path == "/health"
        || path.starts_with("/v1/stream")
        || path == "/v1/ws"
        || path == "/v1/my-public-endpoint"  // add here
}
```

### SSE publishing

Events appended via `state.runtime.store.append()` are automatically
broadcast to all SSE subscribers. No extra work needed.

To subscribe in a handler (e.g., for a custom streaming endpoint):

```rust
let receiver = state.runtime.store.subscribe();
let stream   = BroadcastStream::new(receiver)
    .filter_map(|r| r.ok())
    .map(stored_event_to_sse);
Sse::new(stream.map(Ok::<_, Infallible>))
    .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
```

### Error responses

Use the shared helpers defined near the top of `main.rs`:

```rust
return Err(not_found(format!("run {id} not found")));
return Err(internal_error(e.to_string()));
return Err((StatusCode::BAD_REQUEST, Json(ApiError {
    code: "invalid_body",
    message: "field X is required".to_owned(),
})));
```

### Pagination

Most list handlers accept `PaginationQuery`:

```rust
async fn list_things_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl IntoResponse {
    let limit  = q.limit.min(500);  // q.limit defaults to 50
    let offset = q.offset;
    // …
}
```

---

## Quick reference

```bash
# Run all tests (excluding cairn-app integration suite which needs the binary)
cargo test --workspace --exclude cairn-app

# Run cairn-app tests
cargo test -p cairn-app

# Check only (no tests, fast)
cargo check --workspace

# Type-check the UI
cd ui && npx tsc --noEmit

# Build the UI for production
cd ui && npm run build

# Full smoke test against a running server
CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=dev bash scripts/smoke-test.sh

# Full agent simulation
CAIRN_URL=http://localhost:3000 CAIRN_TOKEN=dev bash scripts/agent-sim.sh
```
