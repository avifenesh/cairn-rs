# RFC 021: Control Plane Protocols and Dynamic Discovery

Status: draft (rev 2 — SQ/EQ as additional protocol surface for v1 with UI migration post-v1, OTLP redaction default confirmed, scope-bound transport sessions)
Owner: protocol/runtime lead
Depends on: [RFC 007](./007-plugin-protocol-transport.md), [RFC 009](./009-provider-abstraction.md), [RFC 015](./015-plugin-marketplace-and-scoping.md), [RFC 018](./018-agent-loop-enhancements.md), [RFC 022](./022-triggers.md)

## Resolved Decisions (this revision)

- **SQ/EQ is an additional protocol surface for v1; bundled UI migrates post-v1**: v1 ships SQ/EQ as an additive interface for external clients (IDE extensions, scripts, alternate dashboards). The bundled UI continues to use existing HTTP + SSE routes in v1 — migrating the UI mid-v1 is risky scope creep. UI migration to SQ/EQ is committed for v1.1 or v2 as a follow-up once the protocol is proven by external clients + integration tests. Existing HTTP routes remain stable.
- **Transport sessions are scope-bound**: `Initialize` binds the SQ/EQ transport session to an authenticated scope (`ProjectKey` or `scopes: Vec<ProjectKey>`) derived from the bearer token. Events are server-side filtered by the bound scope before delivery. The transport session ID (`sqeq_session_id`) is distinct from any logical run/session/task ID in business payloads.
- **OTLP redaction default**: redacted by default. Exported spans include structural metadata (operation, model, tokens, latency, tool name, error class) but strip message content, tool args, and model responses. Operators who want full content (typically for debugging in Langfuse or Phoenix) set `redact_content = false` explicitly per deployment. Privacy-safe default; explicit opt-in for verbose mode.

## Summary

Cairn-rs must be able to talk to the rest of the agent ecosystem without becoming coupled to any specific vendor. This RFC specifies four protocol concerns, all additive on top of existing infrastructure:

1. **Formalize the Submission Queue / Event Queue (SQ/EQ) protocol** already implied by `RuntimeCommand` / `RuntimeEvent` and the SSE stream. Give it a versioned wire contract, capability negotiation, and a client library, so any external client (IDE, alternate dashboard, script) can drive cairn the same way the bundled UI does.

2. **Agent-to-Agent protocol (A2A) surface** via `/.well-known/agent.json` Agent Card and the A2A task submission endpoint. This lets external agent systems delegate work to cairn, and lets cairn delegate work to external agents via a standard shape.

3. **OpenTelemetry GenAI export** for every LLM call, tool invocation, and run. This unlocks every OTLP-compatible observability tool (Langfuse, Phoenix, Grafana Tempo, Jaeger, Datadog) as a plug-in backend without cairn having to integrate with each.

4. **Dynamic provider discovery from plugins.** RFC 015's plugin manifest gains a `PluginCapability::GenerationProvider` variant so plugins can register as LLM generation providers at runtime. Combined with RFC 009's existing provider router, this means an operator can add a new LLM provider by installing a plugin, with zero cairn-rs core changes.

What is already built and **not changed** by this RFC:

- MCP server (`cairn-tools/src/mcp_server.rs`) and MCP client (`cairn-tools/src/mcp_client.rs`)
- RFC 007 plugin protocol (JSON-RPC over stdio)
- RFC 009 provider abstraction with router, pool, health, binding, route resolver
- The existing SSE stream at `GET /v1/stream`
- Every built-in tool and plugin

## Why

### Cairn cannot be a closed product

A control plane for teams using AI is only useful if it composes with the tools those teams already use. That means:

- an IDE extension can drive a cairn run
- an external agent system (Devin, Factory, a competitor, a custom internal tool) can hand cairn a task and get structured results back
- operators can pipe cairn's telemetry into their existing observability stack
- a team can install a plugin that adds support for a new LLM provider cairn has never heard of

None of these require cairn to own the extension. They require cairn to speak standard protocols and expose stable interfaces.

### What is already solved

- **MCP** is already done in both directions. Cairn can expose its tools as an MCP server for external agents to consume, and cairn can call external MCP servers as clients. This is the most-used agent protocol as of 2026 and cairn already speaks it.
- **Plugin protocol (RFC 007)** covers the cairn-specific transport: JSON-RPC over stdio, capability declarations, supervised processes. Used for tools, signal sources, channel providers, eval scorers.
- **Provider abstraction (RFC 009)** covers model routing with fallback, health tracking, cost accounting. Works for OpenAI, Bedrock, Vertex, Ollama, OpenRouter, any OpenAI-compatible endpoint.

### What is not yet solved

Four gaps that block ecosystem interop:

1. **No formal SQ/EQ protocol.** The `RuntimeCommand` and `RuntimeEvent` types exist but are not exposed as a versioned wire protocol. An external client has to reverse-engineer the HTTP + SSE contract. There is no capability negotiation, no protocol version, no TypeScript generation.
2. **No A2A surface.** Cairn cannot advertise its capabilities to other agent systems in a standard shape. Other systems cannot delegate to cairn via a protocol.
3. **No OTLP export.** Cairn has internal traces and cost events, but an operator who wants them in Langfuse, Grafana, or Datadog has to write custom glue. The industry has converged on OTLP for this; cairn should speak it.
4. **Provider discovery is static.** Providers are configured in `config.toml` or via POST to the admin API. A plugin cannot register itself as a provider at runtime. This means every new provider is a cairn-rs code change or a bespoke config, not a plugin install.

This RFC closes those four gaps.

## Scope

### In scope for v1

- **SQ/EQ protocol spec**: versioned REST + SSE protocol for `Submission` (commands from client to server via POST) and `Event` (events from server to client via SSE), with scope-bound transport sessions and `correlation_id` threading. A client library in Rust (reference) and TypeScript (generated via `ts-rs`).
- **Capability negotiation** at connection time: the client sends an `Initialize` request listing its supported protocol versions and which event types it wants; the server responds with the negotiated version and a subscription filter applied.
- **A2A Agent Card** served at `GET /.well-known/agent.json` per the published A2A spec, describing cairn as an agent-of-agents that accepts task submissions.
- **A2A task submission endpoint** at `POST /v1/a2a/tasks` accepting the A2A task shape and mapping it into cairn's existing task model.
- **OTLP span export**: every `RuntimeEvent` that represents a meaningful operation (run start/end, tool call, LLM call, decision, sandbox lifecycle) is exported as an OTel span with the 2025 GenAI semantic convention attributes. Destination is configurable (OTLP gRPC or HTTP).
- **Plugin provider capability**: a new `PluginCapability::GenerationProvider` variant declaring that the plugin acts as an LLM generation provider. The plugin exposes RPC methods that cairn calls instead of making HTTP requests itself. The plugin is then discoverable by the RFC 009 provider router.
- **Dynamic provider registration**: when a provider plugin is enabled for a project, its provider entries appear in the project's provider catalog. The router can bind to them the same way it binds to config-declared providers.

### Explicitly out of scope for v1

- gRPC support for the SQ/EQ protocol (HTTP + WebSocket is fine for v1; gRPC is a later optimization)
- A2A authentication negotiation beyond bearer tokens (the published A2A spec supports more; cairn picks the simplest)
- A2A signed Agent Cards (future work)
- Rewriting existing MCP client or server (no changes)
- A universal protocol translator that bridges MCP ↔ A2A ↔ SQ/EQ (these are different abstractions; cairn speaks all three, it does not unify them)
- OTLP log export (traces only in v1)
- OTLP metric export (future; cairn's internal cost tracking is sufficient for v1)
- Plugins as embedding providers or reranker providers (v1 supports generation providers only; RFC 009's embedding and reranker surfaces are deferred)
- Cross-tenant A2A task routing

## SQ/EQ Protocol

### The shape already exists

Cairn already has:

- `RuntimeCommand` — a union type the API layer translates from HTTP requests. Examples: `StartRun`, `PauseRun`, `ResolveApproval`, `CreateTask`.
- `RuntimeEvent` — a union type emitted onto the SSE stream and appended to the event log. Examples: `RunStarted`, `TaskClaimed`, `ApprovalRequested`.
- `GET /v1/stream` — an SSE endpoint that streams `RuntimeEvent`s with `Last-Event-ID` replay semantics.

What is missing is a protocol wrapper that names this as SQ/EQ, versions it, and gives clients capability negotiation.

### The versioned protocol

The SQ/EQ protocol is **versioned REST + SSE**, not a JSON-RPC 2.0 transport. Earlier drafts branded the protocol as JSON-RPC 2.0, but only the Submission envelope was JSON-RPC-shaped; Initialize is plain HTTP JSON and SSE events are `event:` + `data:` envelopes, not JSON-RPC notifications. The protocol is cleaner described honestly as what it is: versioned REST commands + SSE event streaming, with explicit correlation for cause-to-effect tracing.

```
Client                       Server
  |                            |
  |-- Initialize (POST)  --->  |
  |    {protocol_versions, scope, subscriptions}
  |                            |
  |  <-- InitializeResponse -- |
  |    {negotiated_version, sqeq_session_id, bound_scope, capabilities}
  |                            |
  |-- Submit (POST)  -------> |
  |    {method, params, correlation_id}
  |                            |
  |  <-- SubmissionAck ------- |
  |    {accepted, correlation_id, projected_event_seq}
  |                            |
  |  <-- Event (SSE) --------- |
  |    {seq, type, correlation_id?, payload}
  |  <-- Event (SSE) --------- |
  |  <-- SubmissionError (SSE)  |
  |    {correlation_id, error_code, message}
  |                            |
```

#### Initialize

```http
POST /v1/sqeq/initialize
Authorization: Bearer <token>
Content-Type: application/json

{
  "protocol_versions": ["1.0"],
  "client": { "name": "cairn-ide-ext", "version": "0.1.0" },
  "scope": { "tenant_id": "acme", "workspace_id": "eng", "project_id": "backend" },
  "subscriptions": {
    "event_types": ["run.*", "task.*", "decision.*", "memory.*", "graph.*"],
    "include_reasoning": "requested",
    "exclude_internal": true
  }
}
```

**Scope binding**: the `scope` field binds this transport session to a specific `ProjectKey` (or `scopes: [...]` for multi-project clients). The bearer token must authorize the declared scope; if the token does not have access to the scope, Initialize returns 403. Events from other projects in the same tenant are **not** delivered — server-side filtering is applied before SSE delivery.

**Subscription `include_reasoning`**: advisory, not guaranteed. The server applies a visibility filter based on the bearer token's permission level. An external client with read-only access requesting `include_reasoning: "requested"` may receive `include_reasoning: "denied"` in the response if the token lacks operator-level audit permission. Internal-only events (e.g. detailed guardrail rule evaluation) are filtered by `exclude_internal` regardless of `include_reasoning`.

Response:

```json
{
  "negotiated_version": "1.0",
  "sqeq_session_id": "sqeq_abc123",
  "bound_scope": { "tenant_id": "acme", "workspace_id": "eng", "project_id": "backend" },
  "include_reasoning": "granted",
  "capabilities": {
    "supported_commands": ["start_run", "pause_run", "resolve_approval", ...],
    "supported_events": ["run.started", "run.completed", "memory.ingested", ...],
    "supports_replay": true,
    "max_event_buffer": 10000
  }
}
```

`sqeq_session_id` is the **transport session handle** — used for reconnecting the SSE stream and correlating server-side session state. It is NOT a logical run ID, session ID, or task ID. Business payloads use `run_id`, `task_id`, `approval_id` etc. exclusively.

#### Submission (command)

```http
POST /v1/sqeq/submit
Authorization: Bearer <token>
Content-Type: application/json

{
  "method": "start_run",
  "correlation_id": "corr_req1_abc",
  "params": {
    "run_id": "run_abc",
    "mode": "direct"
  }
}
```

The server validates the command synchronously (auth, scope, schema). **All validation and authorization errors are returned synchronously** in the HTTP response — the client never has to look for validation failures in the SSE stream. If validation passes, the server emits the corresponding `RuntimeCommand` internally and returns an ack:

```json
{
  "accepted": true,
  "correlation_id": "corr_req1_abc",
  "projected_event_seq": 4821
}
```

The ack is immediate. Any downstream events (run lifecycle, tool calls, etc.) arrive via the SSE stream, each carrying the `correlation_id` so the client can trace cause-to-effect. **Async failures** (runtime rejections discovered after ack, resource exhaustion, provider errors) arrive as SSE events with the same `correlation_id`:

```
id: 4825
event: submission.error
data: {"correlation_id":"corr_req1_abc","error_code":"provider_unavailable","message":"Model gpt-5 not reachable","run_id":"run_abc"}
```

This establishes a clean framing rule:
- **Client → server**: REST POST only (`initialize`, `submit`)
- **Server → client synchronous**: HTTP JSON response for validation/acceptance
- **Server → client async consequences**: SSE events only, correlated by `correlation_id`

#### Event stream

```http
GET /v1/sqeq/events?sqeq_session_id=sqeq_abc123
Accept: text/event-stream
Last-Event-ID: 4820
```

Events arrive in SSE format, filtered by the bound scope from Initialize:

```
id: 4821
event: run.started
data: {"run_id":"run_abc","correlation_id":"corr_req1_abc","mode":"direct","started_at":1775760000000}

id: 4822
event: decision.recorded
data: {"decision_id":"dec_...","correlation_id":"corr_req1_abc","outcome":"allowed","source":"cache_hit"}

id: 4825
event: memory.ingested
data: {"signal_id":"sig_...","source_id":"plugin:github:signal:del_xyz","chunks_created":3}
```

Replay works via the existing `Last-Event-ID` mechanism (already implemented in cairn-app's SSE layer). The `correlation_id` threads through the entire causal chain from submission through all downstream effects, allowing clients to attribute any event to the submission that caused it.

### Why reuse the SSE endpoint

Cairn-app's existing `GET /v1/stream` already delivers `RuntimeEvent`s with Last-Event-ID. The SQ/EQ protocol is a layer on top that adds capability negotiation and a distinct namespace for subscribers who use the versioned protocol. The underlying delivery mechanism (HTTP long-poll SSE) is unchanged.

Implementation-wise, this is:

1. New route handlers at `/v1/sqeq/initialize`, `/v1/sqeq/submit`, `/v1/sqeq/events`
2. Initialize creates an `SqEqSession` with the negotiated capabilities
3. Submit validates the command against the session's declared capabilities and delegates to the existing runtime command handlers
4. Events delegates to the existing SSE broadcast with the session's subscription filter applied

### TypeScript client generation

Add `ts-rs` to `cairn-domain` and annotate the protocol types (`Initialize`, `Submission`, `EventEnvelope`, and every `RuntimeCommand`/`RuntimeEvent` variant) with `#[derive(TS)]`. During `cargo build`, generate TypeScript types into `ui/src/lib/protocol/`. The cairn UI consumes these generated types directly; external clients can vendor them.

### Capability negotiation effect on the bundled UI

Today, the bundled UI fetches events via `GET /v1/stream` and uses `api.ts` for commands. **In v1, the bundled UI continues to use these existing routes.** SQ/EQ is shipped as an additional protocol surface for external clients (IDE extensions, scripts, alternate dashboards). The UI migration to SQ/EQ is committed for **v1.1 or v2** as a follow-up once the protocol is proven by external clients + integration tests — migrating the UI mid-v1 is risky scope creep. Non-UI callers (curl scripts, tests) also continue to use the existing HTTP routes, which remain stable.

## A2A Agent Card and Task Submission

### Agent Card

Serve a JSON document at `GET /.well-known/agent.json` per the A2A v0.3 spec:

```json
{
  "a2a_version": "0.3",
  "agent_id": "urn:cairn:self-hosted:tenant:{tenant_id}",
  "name": "Cairn Control Plane",
  "description": "Self-hosted agent control plane for teams using AI",
  "endpoints": {
    "task_submission": "https://<cairn-host>/v1/a2a/tasks",
    "task_status": "https://<cairn-host>/v1/a2a/tasks/{task_id}",
    "task_cancel": "https://<cairn-host>/v1/a2a/tasks/{task_id}/cancel"
  },
  "auth": {
    "type": "bearer",
    "docs_url": "https://docs.cairn.dev/a2a/auth"
  },
  "capabilities": {
    "accepts_tasks": true,
    "delegates_tasks": true,
    "supports_streaming": true,
    "supports_push_notifications": false
  },
  "accepted_task_kinds": [
    "research",
    "code_edit",
    "incident_triage",
    "content_drafting",
    "data_analysis"
  ],
  "supported_input_formats": ["text/markdown", "application/json"],
  "supported_output_formats": ["text/markdown", "application/json"],
  "transport": ["https", "https-sse"],
  "version": "0.1.0"
}
```

The `accepted_task_kinds` is a free-form list of task category tags that the operator can declare in their deployment config. Task kinds are not protocol-mandated — they are a hint for external agents choosing delegation targets.

### Task submission endpoint

```http
POST /v1/a2a/tasks
Authorization: Bearer <token>
Content-Type: application/json

{
  "task": {
    "kind": "research",
    "input": {
      "content_type": "text/markdown",
      "content": "Summarize recent work on retrieval augmented generation..."
    },
    "metadata": {
      "requester_agent": "urn:factory:agent:12345",
      "priority": "normal",
      "deadline_ms": 1775759999999
    }
  }
}
```

The server:

1. Validates the task
2. Resolves the tenant/workspace/project context from the bearer token (the A2A auth layer honors RFC 008 scoping)
3. Creates an internal cairn task with `source: A2A { requester_agent, deadline_ms }`
4. Returns a task reference:

```json
{
  "task_id": "a2a_task_xyz",
  "status": "submitted",
  "status_url": "/v1/a2a/tasks/a2a_task_xyz"
}
```

Downstream, the A2A task flows into cairn's existing task model (RFC 005) with the `A2A` source. Nothing else in cairn treats it specially — the same runtime, the same policy, the same sandboxing, the same observability.

### Task status and streaming

```http
GET /v1/a2a/tasks/{task_id}
Accept: text/event-stream
```

Returns SSE updates with task state, partial outputs, and eventual completion. The format matches the A2A spec's streaming shape. Internally, this is a projection over the cairn task's events, filtered to the ones relevant to an external observer.

### Cairn delegating to external A2A agents

Outbound A2A is exposed as a **tool**: `a2a.delegate_task`. An agent in cairn can call this tool with an external Agent Card URL and a task, and cairn handles the HTTP + SSE lifecycle. Tool results include the external task's final output.

This is a built-in tool in `cairn-tools/src/builtins/a2a_delegate.rs` with `ToolEffect::External`. It is subject to the same policy layer as any other external tool.

## OpenTelemetry GenAI Export

### The standard

OpenTelemetry has finalized the GenAI Agent Application Semantic Convention (2025). It defines span attributes for:

- `gen_ai.operation.name` — chat, text_completion, tool_call
- `gen_ai.system` — anthropic, openai, bedrock, ollama, etc.
- `gen_ai.request.model` — the model ID
- `gen_ai.request.temperature`, `gen_ai.request.max_tokens`, etc.
- `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`
- `gen_ai.response.finish_reasons` — stop, tool_calls, length, content_filter
- `error.type` — for classified failures

Every OTLP-compatible observability tool consumes these attributes.

### The exporter

Add an OTLP exporter to cairn-runtime that subscribes to a selected set of runtime events and produces OTel spans:

```rust
// New in cairn-runtime/src/telemetry/otlp_exporter.rs
pub struct OtlpExporter {
    endpoint: String,          // OTLP gRPC or HTTP endpoint
    protocol: OtlpProtocol,    // Grpc | HttpBinary | HttpJson
    batch: BatchConfig,
    tracer: opentelemetry::global::BoxedTracer,
}

impl OtlpExporter {
    pub async fn export_event(&self, event: &RuntimeEvent) -> Result<(), ExportError> {
        match event {
            // Orchestrator loop events
            RuntimeEvent::RunStarted(e)          => self.start_run_span(e).await,
            RuntimeEvent::RunCompleted(e)        => self.end_run_span(e).await,
            RuntimeEvent::ToolInvocationStarted(e)   => self.start_tool_span(e).await,
            RuntimeEvent::ToolInvocationCompleted(e) => {
                // For memory_search: add gen_ai.operation.name = "retrieval" per
                // GenAI semantic conventions so observability tools (Langfuse,
                // Phoenix) distinguish RAG retrieval calls from generation calls.
                let override_attrs = if e.tool_name == "memory_search" {
                    Some(vec![("gen_ai.operation.name", "retrieval")])
                } else { None };
                self.end_tool_span_with(e, override_attrs).await
            },
            RuntimeEvent::LlmCall(e)             => self.llm_call_span(e).await,
            RuntimeEvent::Decision(e)            => self.decision_span(e).await,
            RuntimeEvent::ContextCompacted(e)    => self.context_compaction_span(e).await,
            // Sandbox events
            RuntimeEvent::SandboxProvisioned(e)  => self.sandbox_span(e).await,
            // Knowledge-layer events (sealed RFCs 015/016)
            RuntimeEvent::SignalIngestedToMemory(e)  => self.memory_ingest_span(e).await,
            RuntimeEvent::SignalProjectedToGraph(e)  => self.graph_projection_span(e).await,
            RuntimeEvent::RepoStoreRefreshed(e)      => self.repo_refresh_span(e).await,
            // Any new RuntimeEvent variant added by a future or sealed RFC should
            // be assessed for OTLP export relevance. Silently dropping a new event
            // means operators lose observability on that operation.
            _ => Ok(()),
        }
    }
}
```

Spans are nested using parent-child relationships derived from the event log's run → task → tool call hierarchy. An operator consuming the traces sees a full execution waterfall.

### Configuration

```toml
[telemetry.otlp]
enabled = false  # default off; operator opts in
endpoint = "http://localhost:4317"
protocol = "grpc"
service_name = "cairn-rs"
tenant_attribute = "tenant_id"  # which tenants should attribute spans to a custom tag
batch_max_events = 512
batch_max_delay_ms = 2000
```

### What OTLP unlocks without cairn code

An operator with OTLP configured can:

- **Langfuse**: point the OTLP endpoint at Langfuse's OTLP ingest; every LLM call, tool call, decision shows up in Langfuse's trace viewer, cost tracking, and eval linking — no cairn code changes required
- **Grafana Tempo + Grafana Cloud**: point OTLP at Tempo; correlate with existing infra metrics
- **Jaeger**: self-hosted trace viewer with no vendor cost
- **Datadog**: if an operator already uses Datadog, OTLP ingest is supported and cairn traces join everything else
- **Arize Phoenix**: self-hosted AI observability with OTel-native ingestion
- **Honeycomb, New Relic, any OTLP-compatible backend**

Cairn does not integrate with any of them specifically. It exports standard OTel spans. Each backend is a configuration change.

### Cairn's native trace view remains

The operator dashboard's runs + traces view (RFC 010) is unchanged. It reads from the event log, not from an external OTLP backend. OTLP export is for teams that want their AI traces in their existing observability stack alongside other services. The two views coexist.

### Privacy and PII

Spans may contain prompts, tool args, and model responses — which can include PII. The exporter has a configurable redaction layer:

- `redact_attributes: ["gen_ai.request.messages", "gen_ai.response.content"]` — elide content from exported spans but keep structural metadata
- `redact_tool_args: bool` — strip tool args from exported spans
- `max_attribute_value_bytes: 1024` — truncate large values

Cairn's native event log is not affected by these settings; redaction applies to the exported spans only.

## Dynamic Provider Discovery From Plugins

### The existing static model

RFC 009 defines cairn's provider abstraction. Providers are configured via `config.toml` or registered through the admin API. The router resolves a model ID against the configured providers and dispatches. Adding a new provider today means:

- writing Rust code for the provider (e.g. a new entry in `cairn-runtime/src/services/`)
- or configuring an existing provider (e.g. `openai_compat_provider.rs` with a custom base URL)

### The gap

A team with a proprietary or bespoke LLM provider cannot add it without editing cairn-rs source. Even an openai-compat provider requires config changes. There is no "install a plugin for provider X" story.

### The new capability

Extend `PluginCapability` with a new variant:

```rust
pub enum PluginCapability {
    ToolProvider { tools: Vec<String> },
    SignalSource { signals: Vec<String> },
    ChannelProvider { channels: Vec<String> },
    PostTurnHook,
    PolicyHook,
    EvalScorer,  // Reserved; v1 rejects manifests declaring this capability (sealed RFC 015 §Non-Goals)
    McpServer { endpoint: McpEndpoint },
    // NEW:
    GenerationProvider {
        /// Model IDs this provider advertises
        models: Vec<PluginProviderModel>,
    },
}

pub struct PluginProviderModel {
    pub id: String,                    // e.g. "acme-llm-v2"
    pub display_name: String,
    pub context_window: Option<u32>,
    pub supports_tool_calls: bool,
    pub supports_structured_output: bool,
    pub supports_streaming: bool,
    pub cost_per_million_input_tokens: Option<f64>,
    pub cost_per_million_output_tokens: Option<f64>,
}
```

### RPC contract

The plugin host adds new JSON-RPC methods plugins can implement for the generation provider capability:

- `provider.generate` — called by the provider router when a model ID matches one declared by the plugin
- `provider.stream` — streaming variant
- `provider.health_check` — called by the provider health tracker

These are plugin-internal RPC calls over the existing stdio transport; no new transport is needed. The request/response shapes match `cairn_runtime::services::provider::GenerationRequest` and `GenerationResponse`.

### Registration flow

1. Operator installs a plugin via RFC 015's marketplace (e.g. an "Acme LLM" plugin)
2. Plugin manifest declares `GenerationProvider { models: ["acme-llm-v2"] }`
3. Marketplace layer recognizes the provider capability and registers it with the provider router: `ProviderRouter::register_plugin_provider(plugin_id, models)`
4. The router's catalog now includes "acme-llm-v2" as a valid model ID
5. A run configured to use `acme-llm-v2` routes through the plugin via `provider.generate` RPC
6. Cost, latency, and health tracking use the existing `provider_health_impl.rs` and `run_cost_alert_impl.rs` machinery — the plugin provider looks identical to any other provider at the routing layer

**Restart safety**: on cairn-app startup, the provider router reconstructs plugin-provided model catalog entries from durable plugin enablement events during the event-log replay step (step 2 of the sealed RFC 020 startup dependency graph). `/v1/providers/models` is not ready — and readiness does not flip — until this replay completes. This ensures that a run resuming after a crash finds the same provider catalog it had before the crash, without depending on a live plugin reconnect race.

### Per-project scoping

Per RFC 015's per-project plugin enablement, a plugin provider is only discoverable in projects where the plugin is enabled. Two projects in the same tenant can each have a different set of plugin providers enabled. This aligns with the RFC 009 multi-tenant routing story.

### Credential flow

Provider plugins declare required credentials in their marketplace descriptor (RFC 015). The credential wizard collects them during install. At runtime the credentials are injected into the plugin process's environment. The plugin uses them to authenticate to the upstream LLM service. Cairn never sees the credential values.

### Why not just use `openai_compat_provider.rs`

A plugin can do things `openai_compat_provider` cannot:

- normalize a proprietary API shape that is not OpenAI-compatible
- handle custom auth flows (e.g. on-device token exchange)
- integrate with a bespoke LLM that has no HTTP API at all (shared-memory IPC to a local inference engine)
- implement provider-specific cost reporting that differs from pure token counting
- ship as a standalone binary that an operator can install without touching cairn-rs source

For OpenAI-compatible providers with a URL and a token, config.toml is still the right answer. Plugin providers are for the long tail that config cannot express.

## Non-Goals

- a unified protocol that bridges MCP / A2A / SQ-EQ under one abstraction
- gRPC for SQ/EQ in v1
- OTLP log and metric export
- Plugins as embedding or reranker providers. V1 supports `GenerationProvider` only. cairn-memory's embedding needs (for vector and hybrid retrieval modes per sealed RFC 018) are served by the RFC 009 **worker-tier provider**, configured via `config.toml` or `POST /v1/providers/connections`. Operators with custom embedding models (bge, nomic-embed-text) configure them as worker-tier providers, NOT as plugins. A future `PluginCapability::EmbeddingProvider` will allow plugin-based embedding backends; until then, embedding discovery is static, not dynamic.
- Cross-tenant A2A task delegation
- A2A push notifications (the spec supports them; cairn picks SSE only in v1)
- TypeScript SDK beyond auto-generated types (a real SDK with convenience APIs is future work)
- WASM plugin providers (stdio JSON-RPC only in v1)

## Open Questions

1. **Resolved**: SQ/EQ ships as an additional protocol surface for external clients in v1. The bundled UI continues to use existing HTTP + SSE routes. UI migration to SQ/EQ is committed for v1.1 or v2. (No further discussion needed; baked into the Resolved Decisions.)

2. **NEEDS DISCUSSION: A2A Agent Card authentication declaration.** The spec allows multiple auth schemes. Cairn declares `bearer` only in v1. Does cairn need to support OAuth for A2A specifically, or is bearer sufficient for self-hosted team deployments? Proposal: bearer only in v1.

3. **NEEDS DISCUSSION: OTLP redaction defaults.** Default to redacted (content elided) or verbose (content included)? Proposal: redacted by default — operators who want full content opt in explicitly because of PII risk.

4. **NEEDS DISCUSSION: Plugin provider health check frequency.** The existing provider health tracker polls configured providers. Plugin providers are polled via RPC. How often? Proposal: same cadence as configured providers, default 60s, plugin-overridable in the manifest.

5. **NEEDS DISCUSSION: What happens if a plugin provider dies mid-run?** A run depending on `acme-llm-v2` loses its provider. Proposal: the run transitions to `waiting_dependency` with reason `provider_unavailable` until the plugin recovers or the operator reconfigures the run to use a different provider. This is consistent with how RFC 009 already handles provider outages for configured providers.

6. **NEEDS DISCUSSION: A2A task status observability.** External A2A callers expect streaming updates. Does cairn's SSE endpoint for A2A tasks filter the event stream to a subset (protect internal decisions, guardrails, etc.) or show everything? Proposal: filter — A2A callers see run lifecycle and final output, not internal policy decisions. Operators can opt-in to expose more.

7. **NEEDS DISCUSSION: OTLP span parent resolution for concurrent tool calls.** If a run dispatches multiple tool calls in parallel (RFC 018 `supports_parallel`), each tool span has the run span as parent. Is that sufficient, or do we need a "batch" span grouping them? Proposal: sufficient for v1; operators can group in their viewer.

8. **NEEDS DISCUSSION: Dynamic provider unregistration.** When a plugin provider is disabled/uninstalled, in-flight runs using its models must handle the removal. Proposal: at uninstall time, the marketplace layer lists runs referencing the provider's models and marks them `waiting_dependency` until reconfigured or cancelled.

## Decision

Proceed assuming:

- the SQ/EQ protocol is defined as **versioned REST + SSE** (not JSON-RPC 2.0) with capability negotiation and scope binding at `POST /v1/sqeq/initialize`; transport sessions are bound to a `ProjectKey` scope; `correlation_id` threads through submissions and all downstream SSE events for cause-to-effect tracing; async failures arrive as `submission.error` SSE events correlated to the originating submission
- cairn exposes `GET /.well-known/agent.json` per A2A v0.3 and accepts task submissions at `POST /v1/a2a/tasks`
- OTLP export is opt-in, configurable per deployment, and emits GenAI semantic convention attributes
- `PluginCapability::GenerationProvider` is added to the plugin manifest enum, and provider plugins register with the existing RFC 009 router at enablement time; plugin-provided model catalog entries are reconstructed from durable enablement events during startup and must be warm before readiness flips (cross-reference to sealed RFC 020 startup dependency graph)
- the bundled UI continues to use existing HTTP + SSE routes in v1; SQ/EQ is an additional interface for external clients; UI migration is committed for v1.1 or v2
- MCP client and server are unchanged
- TypeScript types for the SQ/EQ protocol are generated via `ts-rs` and live under `ui/src/lib/protocol/`
- open questions listed above must be resolved before implementation branches diverge

## Integration Tests (Compliance Proof)

1. **SQ/EQ initialize + submit + stream**: an external client sends `Initialize` with `scope: ProjectKey` → receives `sqeq_session_id` (transport handle) + `bound_scope` → sends a `start_run` submission with `correlation_id` → receives an ack with the same `correlation_id` → receives `run.started` and downstream events via SSE with the `correlation_id` threaded through for cause-to-effect tracing; events are server-side filtered to the bound scope
2. **SQ/EQ capability negotiation**: a client requesting a future protocol version receives a version-downgrade ack with the highest cairn supports; a client requesting an unsupported event subscription receives a filtered subscription list
3. **SQ/EQ replay via Last-Event-ID**: a client disconnects, reconnects with `Last-Event-ID: N`, and receives events > N from the SSE buffer
4. **A2A Agent Card served**: `GET /.well-known/agent.json` returns a valid JSON conforming to the A2A v0.3 schema with cairn's endpoints
5. **A2A task submission creates internal task**: `POST /v1/a2a/tasks` with a valid task creates an internal cairn task with `source: A2A`, returns a status URL, and streams state updates
6. **A2A delegate tool**: a cairn run using `a2a.delegate_task` successfully submits a task to an external A2A agent (mock server) and receives the result
7. **OTLP export end to end**: with OTLP configured, running a simple agent loop produces spans in the receiving backend with the expected GenAI attributes (`gen_ai.operation.name`, `gen_ai.request.model`, `gen_ai.usage.*`)
8. **OTLP redaction**: with redaction enabled, exported spans do not contain message content or tool args; with redaction disabled, they do
9. **Plugin provider registration**: installing a plugin with `GenerationProvider` capability registers its models with the router; the models are discoverable via `GET /v1/providers/models` in projects where the plugin is enabled
10. **Plugin provider dispatch**: a run configured to use a plugin-provided model dispatches the generation call via the plugin RPC; cost and latency are recorded by the existing tracker
11. **Plugin provider per-project scoping**: a plugin provider enabled in project P1 is not discoverable in project P2
12. **Plugin provider uninstall**: uninstalling a plugin provider marks in-flight runs using its models as `waiting_dependency`; configured runs unblock when the operator reconfigures
13. **ts-rs generation**: running `cargo build` generates TypeScript types in `ui/src/lib/protocol/` matching the Rust `RuntimeCommand`/`RuntimeEvent` definitions
