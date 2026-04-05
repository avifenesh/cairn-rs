# AI Agent Orchestration Dashboards & Operator UIs

> Deep research compiled April 2026. Covers 20+ sources across real-time monitoring,
> approval workflows, cost tracking, SSE/streaming, run visibility, and open-source
> exemplars.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Real-Time Agent Monitoring UIs](#1-real-time-agent-monitoring-uis)
3. [Approval Workflow / Human-in-the-Loop UIs](#2-approval-workflow--human-in-the-loop-uis)
4. [Cost Tracking Dashboards](#3-cost-tracking-dashboards)
5. [SSE / Streaming UIs](#4-sse--streaming-uis)
6. [Run & Task Visibility](#5-run--task-visibility)
7. [Open-Source Exemplar Breakdown](#6-open-source-exemplar-breakdown)
8. [Synthesized UX Patterns](#7-synthesized-ux-patterns)
9. [Anti-Patterns](#8-anti-patterns)
10. [Tech Stack Survey](#9-tech-stack-survey)
11. [Recommendations for Cairn](#10-recommendations-for-cairn)
12. [Source Index](#source-index)

---

## Executive Summary

The AI agent observability space has converged on a small number of proven UI patterns.
Every major platform (Langfuse, Helicone, Arize Phoenix, OpenLIT, Portkey, LangSmith,
AgentOps, Laminar, Braintrust, Pydantic Logfire) shares a common skeleton:

- **Trace tree / waterfall** as the primary debugging view
- **Session grouping** for multi-turn agent interactions
- **Time-series cost/latency charts** on the overview dashboard
- **Annotation queues** for human review
- **Tag/metadata filters** for slicing data

The differentiation is in execution quality, not concept novelty.

---

## 1. Real-Time Agent Monitoring UIs

### 1.1 The Universal Trace View

Every platform centers on a **trace detail page** with two panels:

| Left Panel | Right Panel |
|---|---|
| Tree of spans (collapsible) | Selected span detail |
| Each node: icon + name + duration badge | Inputs, outputs, metadata, cost |
| Color-coded by span type (LLM, tool, retrieval, custom) | Token counts, model name, latency |

**Langfuse** shows nested observations: an initial model call, multiple tool executions,
and a final summarization step. Each observation includes timing, inputs, outputs, and
cost information. The tree supports collapse/expand for deep hierarchies.

**AgentOps** calls this the "Session Waterfall" -- its most powerful visualization:
- Left side: timeline showing LLM calls, action events, tool calls, and errors
- Right side: detailed event information (exact prompts, completions, parameters)
- Most data is automatically captured without manual logging

**Arize Phoenix** displays distributed traces that capture model calls, retrieval, tool
use, and custom logic so you can debug behavior and understand where time is spent.
Span-level scoring and human annotations appear directly in the interface.

**Laminar** adds a unique twist: a debugger that can **rerun agent execution at specific
steps** while preserving prior context. Live system prompt tuning with instant
reflection during saves.

### 1.2 Session / Conversation View

For multi-turn agents, platforms group traces into sessions:

- **Langfuse Sessions**: propagate a `sessionId` across traces. The UI shows a
  chronological "session replay" of the complete interaction flow. One session maps to
  many traces (1:N).
- **Helicone Sessions**: tracks multi-step interactions with visual grouping.
- **AgentOps Session Drawer**: enables quick navigation between past sessions with
  filtering, plus aggregate meta-analysis across all recorded sessions.
- **LangSmith Threads**: uses metadata keys (`session_id`, `thread_id`,
  `conversation_id`) to group traces representing ongoing dialogues.

### 1.3 Overview Dashboard

The landing page for every tool shows aggregate metrics:

- Request volume over time (line chart)
- P50/P95/P99 latency (line chart)
- Total cost (bar chart, segmented by model)
- Error rate (line chart with threshold alerts)
- Top models by usage (horizontal bar)
- Active users (counter or chart)

**Portkey** breaks this into tabs: Overview, Users, Errors, Cache Analytics, Feedback.
**Langfuse** segments by trace name, user, tags, release version.
**SigNoz** adds dynamic filtering through dashboard variables for service-specific views.

---

## 2. Approval Workflow / Human-in-the-Loop UIs

### 2.1 Current State of the Art

No platform has a first-class "pause agent, show approval dialog, resume on click"
workflow built into its monitoring UI. Human-in-the-loop is handled at the
**application layer**, not the observability layer. However, several patterns exist:

### 2.2 Annotation Queues (Langfuse)

Langfuse provides **Annotation Queues** to streamline reviewing larger batches of
traces, sessions, and observations. The workflow:

1. Configure **Score Configs** defining standardized scoring dimensions and criteria
2. Create a queue of items to review
3. Reviewers click "Annotate" on detail views, select score dimensions, assign values
4. Multiple team members provide diverse expertise
5. Summary metrics update live as annotation scores are added

This is **post-hoc review**, not real-time approval, but it is the closest pattern in
the observability space.

### 2.3 Guardrails as Automated Approval (Portkey)

Portkey implements "Guardrails on the Gateway" -- real-time verification of LLM
behavior before responses reach users:

- 20+ deterministic checks plus LLM-based ones
- Detection for: prompt injection, code presence, regex patterns, JSON schema
  violations, PII, gibberish
- **Actions on pass/fail**: deny request, allow request, append feedback
- Request status codes indicate guardrail outcomes (200 = pass, 246 = partial, 446 = blocked)
- Logs UI shows pass/fail counts per check with individual verdicts and latency

This is **automated approval** -- the guardrail acts as a programmatic gatekeeper.

### 2.4 Feedback Loops

Multiple platforms support attaching human feedback to individual runs:

- **LangSmith Feedback**: continuous or discrete scores from inline annotations,
  automatic evaluators, or online evaluators
- **Portkey Manual Feedback**: annotate logs for later filtering in feedback dashboards
- **Braintrust**: "Add human feedback and build datasets" from production traces
- **AgentOps**: chat-style viewers with full conversation history for manual review

### 2.5 Patterns for Building Real-Time Approval UIs

Based on the research, a production approval workflow should:

1. **Agent emits a `pending_approval` event** via SSE/webhook when hitting a
   high-stakes action
2. **Dashboard shows a notification badge** on the active run
3. **Approval detail view** shows: the proposed action, relevant context, risk score,
   estimated cost
4. **Operator clicks Approve/Reject/Modify** -- this sends a response back to the
   agent runtime
5. **Agent resumes or aborts** based on the decision
6. **Audit trail** records who approved what and when

This pattern does not exist as a turnkey product today. It must be built at the
application/orchestration layer (which is exactly what Cairn can provide).

---

## 3. Cost Tracking Dashboards

### 3.1 Portkey Analytics

Portkey's analytics dashboard delivers real-time monitoring across three dimensions:
**cost, latency, and accuracy**. Key features:

- **Overview tab**: cost expenditure, token consumption, mean latency, request volume,
  user demographics, top models
- **21+ key metrics** tracked automatically
- **Budget Limits**: set spending thresholds per provider API key; system blocks
  requests when exceeded
- **Metadata grouping**: segment costs by user, team, feature, environment
- **Cache analytics**: visualize latency improvements and cost savings from caching
- **Data retention**: 30 days (dev) to unlimited (enterprise)

### 3.2 Langfuse Cost Tracking

Langfuse breaks cost down by multiple dimensions:

- User, session, geography, feature, model, prompt version
- Quality, cost, and latency as the three metric pillars
- Exportable to PostHog and Mixpanel for business intelligence integration
- Volume metrics based on ingested traces and token consumption

### 3.3 Helicone Cost & Performance

- Token-level cost analysis across providers (OpenAI, Anthropic, Azure, Mistral)
- Cost tracking for 100+ LLM models
- Custom metrics export to PostHog
- Real-time streaming of request logs with cost per request

### 3.4 LiteLLM Gateway

- Multi-tenant cost tracking and spend management per project/user
- Per-project customization (logging, guardrails, caching)
- Virtual keys for access control tied to budget limits
- 8ms P95 latency at 1k RPS -- designed for production proxy

### 3.5 AgentOps

- Track spend with LLM foundation model providers
- Session-level cost aggregation
- Event-level cost breakdown within waterfall view

### 3.6 Key UX Pattern for Cost Dashboards

The most effective cost UIs share these elements:

| Element | Purpose |
|---|---|
| **Running total counter** (top of page) | Immediate awareness of spend |
| **Time-series cost chart** | Trend identification |
| **Model breakdown table** | Per-model cost comparison |
| **Per-user/per-feature segmentation** | Accountability and optimization |
| **Budget threshold line** on chart | Visual budget awareness |
| **Alert badges** when approaching limits | Proactive cost control |

---

## 4. SSE / Streaming UIs

### 4.1 Live Tailing Patterns

**Pydantic Logfire** provides a "Live" view for real-time log observation -- a
continuously updating stream of spans as they arrive.

**Laminar** built a "custom realtime engine" for trace visualization, emphasizing
ultra-fast full-text search over span data as it streams in.

**Helicone** logs requests in real-time, enabling developers to watch agent activity
as it happens.

### 4.2 SSE-Based Dashboard Architecture

For SSE-driven agent UIs, the established pattern is:

```
Agent Runtime
    |
    | SSE stream (events: run_started, tool_called, llm_response, error, completed)
    v
Event Bus / Message Queue
    |
    | Fan-out to subscribers
    v
Dashboard WebSocket/SSE endpoint
    |
    | Push to browser
    v
React UI with streaming state management
```

Key implementation details:
- **Event types** should be strongly typed (not free-form strings)
- **Reconnection logic** is essential (SSE auto-reconnects, WebSocket needs manual)
- **Backpressure handling**: buffer events client-side, batch DOM updates
- **Event ID sequencing** for resumable streams after disconnect
- **Heartbeat events** to detect stale connections

### 4.3 Streaming Trace Construction

The hardest UI problem is building a trace tree from a live stream:

1. Events arrive out of order (child span may start before parent metadata arrives)
2. The UI must handle **optimistic rendering** -- show partial tree, fill in gaps
3. Duration badges update in real-time as spans complete
4. Cost counters increment as token counts arrive
5. Error highlighting must propagate up the tree when a child span fails

**Pattern**: Use a client-side span buffer keyed by `trace_id + span_id`. Insert
spans as they arrive. Re-parent orphaned spans when their parent arrives. Use React
state updates batched at 60fps to avoid jank.

---

## 5. Run & Task Visibility

### 5.1 The Run Tree Model

**LangSmith** defines the canonical run tree:

- **Run**: a single unit of work (LLM call, chain, tool invocation, lambda)
- **Trace**: collection of runs grouped by a unique trace ID
- **Project**: organizational container holding multiple traces
- **Thread**: multi-turn conversation linking related traces

Runs have parent-child relationships. A chain run contains child runs for each step.
An agent run contains child runs for each tool call and LLM decision. Maximum 25,000
runs per trace.

**Braintrust** uses similar terminology with six span types:
- `eval`, `task`, `llm`, `function`, `tool`, `score`

Each captures: inputs, outputs, metadata, timing, token usage, costs, nested calls,
errors, and custom metadata.

### 5.2 Visualization Approaches

| Approach | Best For | Used By |
|---|---|---|
| **Indented tree** | Deep hierarchies, debugging | Langfuse, LangSmith, Phoenix |
| **Waterfall timeline** | Performance analysis, parallelism | AgentOps, Laminar |
| **Node-and-edge graph** | Complex agent workflows | Langfuse (agent graph) |
| **Chat replay** | User-facing conversation review | AgentOps, Langfuse sessions |
| **Flame chart** | Latency hotspot identification | Logfire, SigNoz |

### 5.3 Tool Invocation Display

Claude's tool use protocol provides a natural visualization model:

1. Agent sends message with `tool_use` content blocks
2. Each block has: `id`, `name`, `input` (structured JSON)
3. Application executes and returns `tool_result` with matching `id`
4. Loop continues until agent responds with text

In a dashboard, this maps to:
- **Tool call node** in the trace tree (collapsible)
  - Input parameters (JSON viewer)
  - Output result (JSON viewer or rendered preview)
  - Duration, cost, success/failure badge
- **Connecting lines** between LLM decision and tool execution
- **Re-execution button** (as in Portkey's "Replay" and Laminar's step debugger)

### 5.4 Multi-Modal Content in Traces

**Langfuse** supports rich media in traces:
- Images (PNG, JPG, WebP), audio (MPEG, MP3, WAV), attachments (PDF, text)
- Base64 data URIs auto-extracted by SDKs, uploaded to object storage, linked to trace
- External URLs rendered inline in the UI
- Reference tokens replace media in storage: `@@@langfuseMedia:type=...|id=...|source=...@@@`

This is increasingly important as agents interact with vision models, generate images,
or process audio.

---

## 6. Open-Source Exemplar Breakdown

### 6.1 Langfuse

| Attribute | Detail |
|---|---|
| **GitHub** | github.com/langfuse/langfuse |
| **License** | MIT (core), Enterprise extras |
| **Frontend** | Next.js + React |
| **Database** | ClickHouse (analytics), PostgreSQL (application) |
| **Architecture** | Web layer + Worker layer + Shared packages |
| **Deployment** | Docker Compose, Kubernetes/Helm, Terraform (AWS/Azure/GCP) |
| **Key UI Screens** | Trace detail, Sessions, Dashboard, Playground, Annotation Queues, Datasets, Experiments, Scores, Prompt Management |
| **Integrations** | 50+ (LangChain, OpenAI, Anthropic, LlamaIndex, etc.) |
| **Standout Feature** | Agent graph visualization (node-and-edge diagrams) |
| **Built on** | OpenTelemetry standards |

**What works well**: Clean trace tree with timing/cost per span. Session replay for
multi-turn debugging. Annotation queues for team review. Prompt versioning with
side-by-side comparison.

### 6.2 Helicone

| Attribute | Detail |
|---|---|
| **GitHub** | github.com/Helicone/helicone |
| **License** | Apache 2.0 |
| **Frontend** | Next.js + React |
| **Database** | ClickHouse (analytics) + Supabase (app + auth) + Minio (log storage) |
| **Architecture** | Web (NextJS) + Worker (Cloudflare Workers) + Jawn (Express+Tsoa) |
| **Language** | 91.2% TypeScript |
| **Key UI Screens** | Requests, Segments, Sessions, Users, HQL (custom query language) |
| **Standout Feature** | HQL -- custom query language for ad-hoc analysis |
| **Deployment** | Self-hostable via Docker or Kubernetes (Helm) |

**What works well**: Real-time request streaming. Tabbed navigation between data views.
Segment-based filtering. Mobile-responsive layout. Request-level cost visibility.

### 6.3 OpenLIT

| Attribute | Detail |
|---|---|
| **GitHub** | github.com/openlit/openlit |
| **License** | Apache 2.0 |
| **Frontend** | React (Primer design system components) |
| **Database** | ClickHouse |
| **Architecture** | SDK -> OpenTelemetry Collector -> ClickHouse -> Dashboard |
| **SDKs** | Python, TypeScript, Go |
| **Key UI Screens** | Analytics Dashboard, Exceptions Dashboard, Prompt Hub, Secrets Management |
| **Standout Features** | 11 automated evaluation types (hallucination, bias, toxicity, safety), GPU monitoring alongside LLM observability, OpAMP fleet management |

**What works well**: Vendor-neutral OpenTelemetry-native approach. Custom pricing for
fine-tuned models. Rule engine with AND/OR operators for dynamic context retrieval.
Exception monitoring as a dedicated dashboard.

### 6.4 Arize Phoenix

| Attribute | Detail |
|---|---|
| **GitHub** | github.com/Arize-ai/phoenix |
| **License** | Elastic License 2.0 |
| **Frontend** | React + TypeScript |
| **Architecture** | Python server + React frontend + OpenTelemetry/OpenInference |
| **Key UI Screens** | Traces, Experiments, Prompt Playground, Datasets, Evaluations |
| **Standout Features** | Jupyter notebook integration, side-by-side prompt comparison, framework auto-instrumentation (LangChain, LlamaIndex, DSPy) |

**What works well**: Step-by-step execution visualization. Prompt playground for
replay. Experiment comparison panel. Span-level human annotations.

### 6.5 Laminar

| Attribute | Detail |
|---|---|
| **GitHub** | github.com/lmnr-ai/lmnr |
| **License** | Apache 2.0 |
| **Frontend** | React + TypeScript (70.5% TS) |
| **Backend** | Rust (24.5%) -- custom query engine |
| **Database** | ClickHouse |
| **Key UI Screens** | Traces, Debugger, Session Replay, Signals, SQL Editor, Custom Dashboards |
| **Standout Features** | Step-level debugger with context preservation, browser session replay synced with traces, Signals pattern detection system, SQL editor for custom queries |
| **Performance** | Custom realtime engine, ultra-fast full-text search over span data |

**What works well**: The step debugger is unique and extremely powerful for agent
development. Browser session replay synced with traces is innovative. Rust backend
provides excellent query performance. Self-hostable with three-line Docker setup.

### 6.6 AgentOps

| Attribute | Detail |
|---|---|
| **GitHub** | github.com/AgentOps-AI/agentops |
| **License** | MIT |
| **Frontend** | React |
| **Key UI Screens** | Session Waterfall, Session Drawer, Session Overview, Chat Viewer |
| **Standout Features** | Two-line setup, automatic instrumentation, session-level aggregate analysis |

**What works well**: Lowest friction setup. Waterfall view clearly shows time
allocation. Chat viewer provides familiar interface for conversation review.
Framework-native integrations (CrewAI, AutoGen, LangChain).

---

## 7. Synthesized UX Patterns

### 7.1 Navigation Structure

Every successful tool uses this information architecture:

```
Top Nav:  [Project Selector] [Dashboard] [Traces] [Sessions] [Playground] [Datasets] [Settings]

Dashboard (landing):
  +------------------+------------------+
  | Cost (time chart) | Latency (time)  |
  +------------------+------------------+
  | Volume (time)    | Errors (time)    |
  +------------------+------------------+
  | Top Models table | Active Users     |
  +------------------+------------------+
  [Filter bar: date range, model, tags, user, environment]

Traces (list):
  +------------------------------------------------------------------+
  | Search/filter bar                                                 |
  +------------------------------------------------------------------+
  | Trace ID | Name | Status | Latency | Tokens | Cost | Time | Tags |
  | ...      | ...  | ...    | ...     | ...    | ...  | ...  | ...  |
  +------------------------------------------------------------------+

Trace Detail:
  +-------------------------+-----------------------------------+
  | Span Tree (left)        | Span Detail (right)               |
  | - Root span             | Input: { ... }                    |
  |   - LLM call (2.3s)    | Output: { ... }                   |
  |   - Tool: search (0.5s)| Model: claude-sonnet-4-5           |
  |   - LLM call (1.1s)    | Tokens: 1,234 in / 567 out       |
  |   - Tool: write (0.2s) | Cost: $0.0043                     |
  |                         | Duration: 2.3s                    |
  |                         | [Annotate] [Replay] [Share]       |
  +-------------------------+-----------------------------------+
```

### 7.2 Color Coding Convention

| Span Type | Common Color | Icon |
|---|---|---|
| LLM call | Blue/Purple | Brain/sparkle |
| Tool invocation | Green | Wrench/gear |
| Retrieval/RAG | Orange | Search/database |
| Custom/function | Gray | Code brackets |
| Error | Red | Exclamation |
| Human annotation | Yellow | Person/pencil |

### 7.3 Interaction Patterns

1. **Click-to-expand**: Trace tree nodes expand to show children
2. **Click-to-select**: Clicking a span shows its detail in the right panel
3. **Hover for preview**: Quick tooltip with key metrics
4. **Filter-down**: Clicking a tag, model, or user filters the trace list
5. **Copy-to-clipboard**: One-click copy of trace IDs, inputs, outputs
6. **Share via URL**: Each trace/span has a unique shareable permalink
7. **Replay in playground**: Open an LLM call in an interactive playground to modify
   and re-execute

### 7.4 Real-Time Update Patterns

1. **Polling with exponential backoff** for trace lists (every 5s -> 10s -> 30s)
2. **SSE/WebSocket push** for active trace detail views
3. **Optimistic UI updates** for annotation/scoring actions
4. **Live counters** for cost and token usage on active runs
5. **Toast notifications** for completed runs, errors, budget alerts

---

## 8. Anti-Patterns

### 8.1 Information Overload

- Showing raw JSON by default instead of a structured summary
- Displaying all spans expanded on initial load (unusable for deep trees)
- No default sort/filter on trace lists (newest traces buried)

### 8.2 Missing Context

- Showing tool inputs without the LLM reasoning that triggered the call
- Displaying cost without the model name (cost is meaningless without model context)
- Trace trees without timing information (cannot identify bottlenecks)

### 8.3 Poor Streaming UX

- Full page refresh to see new data (destroys context)
- No loading states during stream construction (blank screen anxiety)
- Jittering layout as new spans arrive (content jumping)

### 8.4 Weak Filtering

- Only supporting exact match (no partial, regex, or range queries)
- No saved filter views / bookmarks
- Losing filter state on page navigation

### 8.5 Cost Tracking Gaps

- Showing tokens without converting to dollars
- No per-model cost breakdown
- No budget alerting or threshold visualization
- Mixing development and production costs without environment separation

### 8.6 Approval Workflow Gaps

- No way to flag traces for review (no annotation/queue system)
- No audit trail of who reviewed what
- Scoring without configured dimensions (inconsistent data)
- No batch review capability (reviewing one trace at a time doesn't scale)

---

## 9. Tech Stack Survey

### 9.1 Frontend Frameworks

| Tool | Framework | UI Library |
|---|---|---|
| Langfuse | Next.js + React | Custom components |
| Helicone | Next.js + React | Custom components |
| OpenLIT | React | Primer (GitHub's design system) |
| Arize Phoenix | React + TypeScript | Custom components |
| Laminar | React + TypeScript | Custom components |
| AgentOps | React | Custom components |
| LangSmith | React (presumed) | Custom components |
| Portkey | Not disclosed | Not disclosed |

**Verdict**: React dominates. Next.js is the most popular meta-framework.

### 9.2 Databases

| Tool | Analytics DB | Application DB |
|---|---|---|
| Langfuse | ClickHouse | PostgreSQL |
| Helicone | ClickHouse | Supabase (PostgreSQL) |
| OpenLIT | ClickHouse | ClickHouse |
| Laminar | ClickHouse | Not disclosed |
| LiteLLM | PostgreSQL | PostgreSQL |
| SigNoz | ClickHouse | Not disclosed |

**Verdict**: ClickHouse is the overwhelming choice for trace/analytics storage.
PostgreSQL for application data. This separation is deliberate -- ClickHouse excels
at columnar analytics over time-series data; PostgreSQL excels at transactional
application state.

### 9.3 Observability Standards

| Standard | Adoption |
|---|---|
| OpenTelemetry | Langfuse, OpenLIT, Phoenix, Laminar, SigNoz, Logfire |
| OpenInference | Phoenix (extends OTel for LLM-specific semantics) |
| Custom SDK | AgentOps, LangSmith, Helicone, Portkey |

**Verdict**: OpenTelemetry is becoming the lingua franca. Platforms that don't use
it natively still accept OTel data via adapters.

### 9.4 Backend Languages

| Tool | Primary Backend Language |
|---|---|
| Langfuse | TypeScript (Node.js) |
| Helicone | TypeScript (Cloudflare Workers + Express) |
| Laminar | Rust |
| Phoenix | Python |
| LiteLLM | Python |
| SigNoz | Go |

**Verdict**: Mixed. TypeScript is most common for web-centric platforms. Rust and Go
chosen for performance-critical data pipelines.

---

## 10. Recommendations for Cairn

Based on this research, here are actionable insights for building Cairn's operator UI:

### 10.1 MVP Feature Set (Phase 1)

1. **SSE-driven live trace view** -- subscribe to a signal/run stream, render spans
   as they arrive in a collapsible tree
2. **Run list with filters** -- table of recent runs with status, duration, cost,
   model, tags; filterable and sortable
3. **Cost counter** -- running total on dashboard, per-run cost in trace detail
4. **Signal/event type badges** -- color-coded span types matching Cairn's domain
   events (CommandIssued, ToolInvoked, SignalEmitted, etc.)

### 10.2 Differentiator Features (Phase 2)

1. **Approval workflow** -- built into the event stream, not bolted on. When an agent
   emits a `PendingApproval` signal, the dashboard shows an actionable card with
   Approve/Reject/Modify buttons that send a response event back via the API
2. **Budget guardrails** -- real-time cost tracking with configurable thresholds that
   pause execution and surface approval requests
3. **Session replay** -- group related signals by session/conversation ID, show
   chronological replay

### 10.3 Tech Stack Recommendation

- **Frontend**: React + TypeScript (industry standard, largest talent pool)
- **State management**: Zustand or Jotai for streaming state
- **Data viz**: Recharts or Visx for time-series charts
- **Table**: TanStack Table for filterable/sortable trace lists
- **SSE client**: Native EventSource API with reconnection wrapper
- **Analytics DB**: ClickHouse (if self-hosted) or PostgreSQL with TimescaleDB
  extension (simpler ops)

### 10.4 Data Model Alignment

Cairn's existing domain events map naturally to the trace/span model:

| Cairn Concept | Observability Equivalent |
|---|---|
| Signal | Span (with type=signal) |
| Command | Span (with type=tool_invocation) |
| Run/Session | Trace |
| Agent | Service/Project |
| Event Stream (SSE) | Trace ingestion pipeline |

The key insight: Cairn already has the event-sourced data model. The dashboard is a
**projection** of the event store -- the same pattern used by the existing in-memory
and PostgreSQL projections.

---

## Source Index

| # | Source | URL | Key Contribution |
|---|---|---|---|
| 1 | Langfuse Docs | langfuse.com/docs | Trace UI, annotation queues, session replay |
| 2 | Langfuse GitHub | github.com/langfuse/langfuse | Tech stack (Next.js, ClickHouse) |
| 3 | Langfuse Tracing | langfuse.com/docs/tracing | Nested observation visualization |
| 4 | Langfuse Sessions | langfuse.com/docs/tracing-features/sessions | Session grouping UX |
| 5 | Langfuse Analytics | langfuse.com/docs/analytics/overview | Cost tracking dimensions |
| 6 | Langfuse Scores | langfuse.com/docs/scores/annotation | Annotation workflow |
| 7 | Langfuse Tags | langfuse.com/docs/tracing-features/tags | Filtering UX patterns |
| 8 | Langfuse Multi-Modal | langfuse.com/docs/tracing-features/multi-modality | Rich content in traces |
| 9 | Helicone | helicone.ai | Real-time request logging, cost tracking |
| 10 | Helicone GitHub | github.com/Helicone/helicone | Tech stack (Next.js, ClickHouse, Supabase) |
| 11 | Arize Phoenix Docs | arize.com/docs/phoenix | Trace/span visualization, evaluations |
| 12 | Arize Phoenix GitHub | github.com/Arize-ai/phoenix | Tech stack (React, Python, OTel) |
| 13 | OpenLIT GitHub | github.com/openlit/openlit | Dashboard features, ClickHouse, evaluations |
| 14 | Portkey Observability | portkey.ai/docs/product/observability | 21+ metrics, budget controls |
| 15 | Portkey Logs | portkey.ai/docs/product/observability/logs | Log viewer UI, replay, privacy |
| 16 | Portkey Analytics | portkey.ai/docs/product/observability/analytics | Cost/latency/accuracy dashboards |
| 17 | Portkey Guardrails | portkey.ai/docs/product/guardrails | Automated approval, safety controls |
| 18 | LangSmith Concepts | docs.langchain.com/langsmith/observability-concepts | Run tree model, traces, threads |
| 19 | LangSmith Overview | docs.langchain.com/langsmith | Platform capabilities |
| 20 | AgentOps Docs | docs.agentops.ai | Session waterfall, auto-instrumentation |
| 21 | AgentOps GitHub | github.com/AgentOps-AI/agentops | Tech stack (React, Python SDK) |
| 22 | Laminar | laminar.sh | Step debugger, session replay, Rust backend |
| 23 | Laminar GitHub | github.com/lmnr-ai/lmnr | Tech stack (Rust + React, ClickHouse) |
| 24 | Braintrust Docs | braintrust.dev/docs | Span types, human feedback workflows |
| 25 | Braintrust Tracing | braintrust.dev/docs/guides/tracing | Trace anatomy, span categories |
| 26 | LiteLLM GitHub | github.com/BerriAI/litellm | AI gateway, cost tracking, admin UI |
| 27 | Pydantic Logfire | pydantic.dev/docs/logfire | Live view, OTel-native, LLM panels |
| 28 | LlamaIndex Observability | developers.llamaindex.ai/.../observability | Integration landscape |
| 29 | SigNoz Blog | signoz.io/blog/llm-observability | 7 best practices, metrics framework |
| 30 | Anthropic Tool Use | platform.claude.com/docs/.../tool-use | Tool call structure for visualization |
| 31 | Humanloop | humanloop.com | Evaluation, prompt management (now part of Anthropic) |
| 32 | DeepEval | deepeval.com | Evaluation ecosystem, annotation |
| 33 | MCP Docs | modelcontextprotocol.io/docs | Protocol patterns for tool monitoring |
