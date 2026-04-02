# Cairn Rust Product Rewrite Plan

Status: draft for planning and refinement  
Basis: code review of fetched `origin/main` at `72f6d37`, not the older local checkout  
Audience: product owner, architecture lead, parallel coding agents

## Purpose

This document defines a rewrite plan for turning Cairn into a coherent, sellable product by:

- rewriting the backend into Rust
- separating product code from profile/default overlay
- replacing product dependencies that are currently externalized out of convenience
- adding first-class graph and matrix/eval infrastructure
- creating workstreams that 8 senior parallel workers can execute with low conflict

This is not a porting plan. It is a contract-first product rewrite.

## Iteration 1 Reassessment

This section revises the initial plan using two inputs:

- deeper code review of fetched `origin/main`
- fresh external validation against current agent infrastructure products and documentation

### Fresh Market Observations

The market has become more opinionated and more segmented than a generic “agent platform” framing suggests.

Current leaders are specializing in slices of the stack:

- LangSmith Deployment is positioning around durable execution and long-running agent workloads
- Letta is positioning around stateful agents and memory
- Braintrust is positioning around observability, traces, evals, and prompt versioning
- Portkey is positioning around gateway, routing, and guardrails
- Vellum is positioning around prompt/workflow release management, approvals, and evaluation workflows

This means Cairn should not try to beat each of these point-for-point in v1.

The strongest wedge remains:

- self-hostable control plane
- long-running, multi-channel, tool-using agents
- approvals, checkpoints, mailbox, subagents, and recovery
- product-owned memory, retrieval, graph, and eval systems
- a coherent operator experience rather than five separate products glued together

### Revised Strategic Focus

The rewrite should optimize for:

- coherence over breadth
- product-owned core systems over convenience dependencies
- inspectability over hidden heuristics
- operator control over “framework magic”

The rewrite should explicitly deprioritize:

- becoming a full LLM gateway competitor on day one
- re-implementing every long-tail integration before the platform is coherent
- a separate graph database before graph use cases are stabilized
- a separate vector database before pgvector-backed product retrieval is proven insufficient

### Revised V1 Thesis

Sellable v1 is:

- a self-hostable agent operating system and control plane
- with durable runtime, approvals, memory, owned retrieval, graph-backed introspection, prompt/eval infrastructure, and multi-channel delivery

Sellable v1 is not:

- a generic all-in-one AI platform
- a pure gateway product
- a pure prompt IDE
- a pure vector database
- a pure memory SDK

### External Validation Notes

The reassessment above is informed by current official product/documentation surfaces:

- LangSmith Deployment emphasizes durable execution, checkpointing, replay, streaming, and human-in-the-loop pauses
- Letta emphasizes stateful agents, memory blocks, and multi-agent messaging
- Braintrust emphasizes traces, evals, datasets, prompt versioning, and deployment-quality feedback loops
- Portkey emphasizes gateway, routing, guardrails, budgets, and governance
- Vellum emphasizes prompt/workflow deployments, release tags, protected releases, and release reviews
- pgvector officially supports HNSW indexing in Postgres and remains viable for a Postgres-first retrieval strategy
- Qdrant is mature enough to remain a credible optional adapter later, but not required for v1 architecture

Implications for Cairn:

- durable runtime and checkpoints are table stakes, not differentiators by themselves
- memory must be first-class and inspectable, not just retrieval hidden behind prompts
- evals and prompt/version management are no longer “nice to have”
- we should not try to become a standalone gateway-first product in v1
- Postgres-first retrieval remains a strong default unless product benchmarks prove otherwise

## Product Goal

Build a self-hostable control plane for long-running, memoryful, multi-channel agents with:

- durable orchestration
- approvals, checkpoints, suspend/resume, mailbox, and recovery
- tool execution and permission models
- product-owned memory and retrieval
- graph-aware knowledge and execution introspection
- prompt/version/eval infrastructure
- live observability and replay
- channel and notification routing
- plugin-friendly extensibility

## Customer and Value Reassessment

Before RFCs, we should be explicit about who this is for and why they would care.

### Primary Customer

The most credible first customer is:

- engineering and platform teams inside software companies
- who are building internal or product-facing agents
- and need reliability, recoverability, approvals, memory, observability, and control
- without stitching together five different infrastructure products

These teams are typically:

- infra-capable
- security/compliance-aware
- comfortable self-hosting or hybrid deployment
- willing to invest in an agent platform if it reduces operational chaos

### Secondary Customer

Secondary but still credible:

- AI product teams building customer-facing workflows that need:
  - human-in-the-loop approval
  - long-running execution
  - tool calling
  - memory and retrieval
  - live operator observability

### Tertiary Customer

Later, but not first:

- agencies and consultancies that build bespoke agent systems for clients
- advanced individuals running a personal agent OS

These users can be strong evangelists, but they should not define the first product boundary.

### Non-Target for V1

We should explicitly avoid optimizing v1 for:

- low-code hobby users
- teams wanting only a model gateway
- teams wanting only prompt management
- teams wanting only a vector DB or memory SDK
- teams who do not care about self-hosting, observability, or operational control

### Core Customer Value

The product value is not “we have lots of features.”

The customer value is:

- one operational system for durable agents instead of a fragmented stack
- inspectable long-running execution with approvals, pause/resume, and replay
- owned memory and retrieval instead of opaque external KB dependence
- graph-aware understanding of why the system acted as it did
- prompt and routing decisions that can be evaluated, versioned, and improved
- self-hosted or hybrid deployment for teams that care about control

### Product Wedge

The wedge should be described as:

- the self-hostable control plane for production agents

Not as:

- an everything-AI platform
- a personal assistant only
- an AI gateway
- a prompt IDE
- a memory SDK

This matters because the same code can be interpreted either as a coherent product or as a grab-bag of capabilities.

### Why This Can Win

The strongest reason this product can win is not that each individual subsystem is unique.

It is that the product can combine, in one opinionated system:

- durable runtime
- approvals and checkpoints
- owned memory and retrieval
- graph and eval visibility
- channels and notifications
- self-hostable control plane

The value is in the integration and operator coherence.

## The Extra Step Needed for Productization

The rewrite alone is not enough.

There is one additional transition the project must make:

- from “a very capable agent system” to “a product teams can adopt intentionally”

That extra step includes several product-level requirements.

### 1. Multi-Team Framing

The system must stop feeling like a single operator's agent OS and start feeling like a team product.

That means:

- tenants/workspaces/projects
- roles and permissions
- shared prompts, shared skills, shared datasets
- explicit ownership of agents, policies, and channels

### 2. Opinionated Onboarding

A powerful backend is not enough.

The product needs:

- installation path
- starter templates
- sample agents
- quickstart flows
- first-run success path
- import path for prompts, docs, or memory corpora

### 3. Operator UX

The product must make operators feel in control.

Needed surfaces include:

- run and task inspection
- approval inbox
- memory and retrieval health
- graph views
- eval scorecards
- prompt version and rollout controls
- source and channel health
- policy and permission inspection

### 4. Managed Product Decision

The product likely needs a clear stance on deployment shape:

- self-hosted only
- hybrid control plane
- or hosted control plane plus self-hosted execution

The strongest likely commercial option is:

- self-hosted or hybrid execution with an opinionated control plane

This should be decided deliberately, not left ambiguous.

### 5. Commercial Packaging

The rewrite should eventually map into a package story:

- open core / self-hosted base
- paid control-plane features
- advanced eval, policy, graph, or enterprise governance features
- managed deployment/help/support

This does not need to be fully decided before coding, but the architecture should leave room for it.

## Product vs Overlay

### Product Core

These belong to the product and should be rewritten as first-class Rust systems:

- control plane API, SSE, auth, admin
- session/task/checkpoint/mailbox runtime
- orchestrator, ReAct loop, subagents, approvals
- tool runtime, permission matrix, sandbox integration
- memory service, retrieval, reranking, KB, deep search
- signal plane, pollers, webhooks, digests
- channel adapters and notification router
- observability, event log, traces, replay
- plugin protocol and plugin host

### Product Defaults

These should ship with the product, but not be treated as core engine logic:

- bundled skills
- default prompts
- starter rules/policies
- starter identity/profile templates
- default dashboards and scorecards

### User / Tenant Overlay

These are not the core product and should live as runtime data:

- `SOUL.md`
- `USER.md`
- `AGENTS.md`
- `MEMORY.md`
- source subscriptions
- credentials and routing targets
- user-specific rules, skills, and thresholds
- tenant prompt versions and eval datasets

## Rewrite Principles

1. Do not translate Go packages line by line.
2. Freeze an explicit compatibility contract and break matrix before implementation.
3. Prefer explicit state machines over implicit mutation.
4. Keep plugins out of process and language-neutral.
5. Treat current Go code as a reference implementation and fixture source.
6. Product-owned retrieval, graphs, and evals must replace convenience dependencies.
7. Preserve room for refinement at every phase boundary.

## Compatibility and Break Policy

The rewrite must not preserve current behavior blindly.

Before implementation starts, every externally visible surface must be placed in one of three buckets:

- preserve
  - runtime semantics that are core to the product wedge
  - task/checkpoint/approval/mailbox concepts
  - essential API and SSE concepts worth carrying forward
- intentionally break
  - personal-agent-specific assumptions
  - single-user overlays leaking into product behavior
  - accidental route shapes or state models that conflict with the team product boundary
- transitional
  - surfaces retained temporarily to simplify migration, with a documented replacement path

Examples of preserve:

- durable runs and tasks
- approvals
- checkpoints
- replay-friendly eventing
- subagent execution semantics

Examples of intentionally break:

- APIs or UI assumptions that treat profile data as product logic
- single-operator defaults exposed as architectural assumptions
- personal environment conventions baked into runtime behavior

Examples of transitional:

- glide-mq as an execution substrate
- selected HTTP/SSE compatibility wrappers around existing frontend expectations

Phase 0 must produce a written compatibility and break matrix. No worker should infer this ad hoc.

## What Exists Today

The fetched Go code already contains substantial product substrate:

- HTTP/SSE server
- task engine
- orchestrator and ReAct runtime
- checkpoints and mailbox
- tool registry and large builtin tool surface
- memory search and deep search
- signal plane
- Telegram / Slack / Discord integrations
- glide-mq sidecar integration
- identity/profile editing flows
- reflection, mutation, and error tracking seeds

This means the rewrite should preserve semantics, not rediscover product scope from zero.

## What Is Still Missing or Not Product-Grade Enough

These must be treated as part of the rewrite scope, not “later maybe”:

- product-owned retrieval backend replacing Bedrock Knowledge Base dependence
- unified graph model for knowledge, provenance, tasks, and tool execution
- first-class matrix/eval system
- prompt registry with versioning, release tags, rollback, and A/B evaluation
- clean tenant/workspace/project model
- stable audit/event schema for replay and debugging
- formal operator control plane for memory health, graph views, evals, policies, and routing
- explicit import/export/migration story
- stronger policy engine for guardrails, permissions, and escalation
- clearer separation between engine, defaults, and user profile

## Bedrock KB Replacement Plan

Current Bedrock KB usage is acceptable as a bridge, but not as the product core.

The rewrite should replace it with a product-owned retrieval stack:

- ingest pipeline
  - document import
  - parsing
  - chunking
  - metadata extraction
  - deduplication
- embedding pipeline
  - provider abstraction
  - local and hosted backends
  - batch and backfill workflows
- index layer
  - vector index
  - lexical index
  - hybrid retrieval
- scoring layer
  - freshness decay
  - credibility
  - corroboration
  - provenance weighting
  - access/staleness penalties
- reranking layer
  - MMR
  - optional learned reranker/provider reranker
- query layer
  - semantic search
  - filtered retrieval
  - deep search / multi-hop retrieval
  - graph-assisted expansion

Bedrock may remain an optional embedding provider, but not the source of truth for knowledge storage and retrieval.

## Graph and Matrix Requirements

### Graphs

The product should own a real graph layer, not just implicit relationships in tables.

At minimum:

- entity graph
  - people, repos, skills, prompts, tools, sources, memories, KB assets
- provenance graph
  - which fact came from where, when, and how strongly
- execution graph
  - tasks, subagents, tool calls, approvals, resumes, failures, recoveries
- knowledge graph
  - links between concepts, documents, procedures, and outcomes

Use cases:

- explain why a memory/result was surfaced
- inspect agent execution chains
- show “what led to this decision”
- support graph-assisted retrieval and deep search
- visualize subagent and task flow

### Matrices

The product should expose matrices, not just hidden scoring logic.

At minimum:

- prompt x model x task-type evaluation matrix
- provider routing matrix
- permission matrix by mode/tool/channel/tenant
- memory source quality matrix
- skill health / mutation / intervention matrix
- prompt A/B evaluation matrix
- guardrail/policy outcome matrix

Use cases:

- choose best prompt/model per workload
- compare providers and cost/quality tradeoffs
- explain why an action was or was not allowed
- identify weak skills and underperforming prompts

## Default Technical Decisions for Iteration 1

These are recommended defaults for the rewrite unless an RFC proves otherwise.

### Storage

- Postgres should be the primary system of record for product deployments
- SQLite should remain supported for local and single-user modes
- event log, projections, matrices, graph edges, and prompt versions should live in the product-owned store

### Vector Search

- use `pgvector` first
- use HNSW indexing and filtered retrieval as the default baseline
- do not introduce a dedicated vector database in v1 unless benchmarks show clear failure
- keep Qdrant or other vector stores as later optional adapters, not initial architecture anchors

Rationale:

- this keeps the system operationally simpler
- current `pgvector` capabilities are good enough to justify a Postgres-first design
- the product value is not “we run a bespoke vector cluster”

### Graph Layer

- implement graph capabilities in the product store first
- use typed edge tables, projections, adjacency indexes, and read models
- defer a dedicated graph database until real workloads prove the need

Rationale:

- the graph requirement is real
- a separate graph engine too early would add migration and operability cost
- most initial graph value is in provenance and execution introspection, not graph-algorithm novelty

### Retrieval Stack

- build owned ingest, chunking, embedding, retrieval, rerank, and deep-search services
- treat Bedrock KB as transitional only
- keep model/embedding/reranker providers pluggable

### Prompt and Eval Stack

- prompt registry, release tags, approvals, rollout controls, and scorecards are v1 systems
- prompt and workflow evaluation should be first-class, not a later observability add-on
- matrices should be backed by explicit product tables and APIs

### Plugin Transport

- use an out-of-process contract
- start with stdio JSON-RPC or a similarly small protocol
- design it so MCP compatibility is feasible, but do not block v1 on full MCP scope

### Gateway Scope

- support provider abstraction, routing rules, and policy enforcement inside Cairn
- do not try to become a standalone AI gateway product in v1
- observability should include model/provider behavior, but gateway breadth is not the first wedge

## Target Rust Workspace

### `cairn-domain`

Owns:

- IDs
- enums
- commands
- events
- policy inputs
- state machines
- shared types

Rules:

- no network calls
- no database code
- no provider-specific logic

### `cairn-store`

Owns:

- SQLite/Postgres persistence
- migrations
- repositories
- projections
- event log persistence
- import/export helpers

### `cairn-runtime`

Owns:

- sessions
- tasks
- approvals
- checkpoints
- mailbox
- leases
- pause/resume/recovery

### `cairn-agent`

Owns:

- ReAct loop
- orchestrator
- subagent management
- reflection
- compaction
- error pattern detection
- strategic/advisor flows

### `cairn-tools`

Owns:

- tool registry
- invocation context
- permission enforcement
- builtin tool adapters
- tool events

### `cairn-memory`

Owns:

- memory service
- embedding abstraction
- chunking
- hybrid retrieval
- reranking
- deep search
- KB ingest
- provenance scoring

### `cairn-graph`

Owns:

- entity graph
- provenance graph
- task/tool graph
- graph query APIs
- graph projections for UI and retrieval

### `cairn-evals`

Owns:

- prompt registry
- versioning and release tags
- prompt/model/tool evaluations
- A/B tests
- routing matrices
- scorecards
- eval datasets

### `cairn-signal`

Owns:

- source pollers
- scheduler
- dedup
- webhook ingest
- digest/event generation

### `cairn-channels`

Owns:

- Telegram
- Slack
- Discord
- notification router
- channel delivery policies

### `cairn-api`

Owns:

- HTTP API
- SSE
- auth
- WebAuthn
- admin/control plane routes
- frontend-facing read models

### `cairn-plugin-proto`

Owns:

- language-neutral plugin contract
- stdio and/or HTTP transport definitions
- tool provider protocol
- signal source protocol
- post-turn hook protocol
- guardrail/policy hook protocol

### `cairn-app`

Owns:

- binary wiring
- runtime config
- startup sequencing
- default package composition
- deployment packaging

## Plugin Strategy

Plugins should not be an in-process language-specific trap.

The plugin system should be:

- out of process
- protocol-driven
- capability-declared
- permission-aware
- observable
- restartable

Supported plugin categories:

- tool providers
- signal/poller providers
- channel providers
- post-turn analyzers
- policy/guardrail evaluators
- eval scorers

This keeps the Rust core coherent while allowing external plugins in any language.

## Migration Strategy

### Phase 0: Contract Freeze

Output:

- route inventory
- SSE event catalog
- canonical commands/events
- task/session/checkpoint state diagrams
- tool invocation schema
- memory and graph entity schema
- golden fixtures harvested from current Go behavior
- compatibility and break matrix
- glide-mq ownership boundary memo
- tenancy baseline schema decision

Definition of done:

- no major semantic changes without RFC
- all later phases reference frozen contracts or explicit planned breaks

Refinement checkpoint:

- validate whether the frozen contracts are worth preserving exactly or whether selected surfaces should be intentionally broken for a cleaner product model

### Phase 1: Core Spine

Build:

- `cairn-domain`
- `cairn-store`
- `cairn-runtime`
- initial `cairn-api`
- tenant/workspace/project model
- ownership and scoping model for runtime entities

Definition of done:

- sessions, tasks, approvals, checkpoints, mailbox, and recovery work end-to-end
- HTTP/SSE compatibility shell exists
- all core entities have tenant/workspace/project ownership semantics where applicable

Refinement checkpoint:

- re-evaluate whether runtime and API boundaries are too coupled
- re-evaluate tenant/workspace assumptions before graph/eval layers harden around them

### Phase 2: Tool Runtime and Permissions

Build:

- `cairn-tools`
- permission matrix model
- builtin file/git/shell/web/search/messaging tool adapters
- plugin protocol skeleton

Definition of done:

- tool execution is observable, permissioned, and replayable

### Phase 3: Product-Owned Memory and Retrieval

Build:

- `cairn-memory`
- in-house ingest/chunk/embed/index/query pipeline
- hybrid retrieval
- reranking
- deep search

Definition of done:

- Bedrock KB no longer required for core product retrieval
- local-mode retrieval contract is implemented and documented

Refinement checkpoint:

- benchmark pgvector-based retrieval against expected product workloads
- decide whether a dedicated retrieval service boundary is needed now or later

### Phase 4: Graph and Matrix Systems

Build:

- `cairn-graph`
- `cairn-evals`
- prompt registry
- A/B and scorecard infrastructure

Definition of done:

- graph-backed introspection exists
- evaluation and routing matrices are queryable and visible

Refinement checkpoint:

- confirm graph queries that matter most in the UI and operator workflows
- trim any graph ambition that is not connected to an actual product use case

### Phase 5: Agent Runtime

Build:

- `cairn-agent`
- ReAct loop
- orchestrator
- subagents
- reflection/error-tracking/advisor logic

Definition of done:

- core autonomous workflow works on top of the new runtime

### Phase 6: Signals and Channels

Build:

- `cairn-signal`
- `cairn-channels`
- notification router
- digest generation

Definition of done:

- high-value sources and channels operate through the Rust core

### Phase 7: Productization Pass

Build:

- defaults/profile separation
- operator UX read models
- import/export
- migration tools
- hardening and rollout path

Definition of done:

- sellable v1, not just technical parity

Refinement checkpoint:

- cut or defer anything that does not strengthen the sellable wedge
- verify the system reads as one product rather than several half-products

## 8 Parallel Worker Plan

### Worker 1: Contracts and Fixtures

Owns:

- behavior inventory
- API/SSE catalog
- compatibility fixtures
- state diagrams

### Worker 2: Domain and Event Schema

Owns:

- `cairn-domain`
- commands/events
- state machines
- policy types
- tenancy and ownership primitives

### Worker 3: Persistence and Projections

Owns:

- `cairn-store`
- migrations
- repos
- event log
- read models
- tenant/workspace/project scoping

### Worker 4: Runtime Core

Owns:

- sessions
- tasks
- approvals
- checkpoints
- mailbox
- recovery
- Rust-owned durable mailbox and recovery source of truth

### Worker 5: Tools, Sandbox, Plugins

Owns:

- `cairn-tools`
- permission matrix
- sandbox integration
- plugin protocol

### Worker 6: Memory, Retrieval, Graph

Owns:

- `cairn-memory`
- `cairn-graph`
- in-house KB pipeline
- reranking and deep search

### Worker 7: Agent and Evals

Owns:

- `cairn-agent`
- `cairn-evals`
- prompt registry
- A/B and routing matrices

### Worker 8: API, Signals, Channels, Productization

Owns:

- `cairn-api`
- `cairn-signal`
- `cairn-channels`
- tenant/workspace integration
- rollout glue

## Timeline

Assuming 8 senior parallel workers with a strong architecture owner:

- internal alpha: 12-16 weeks
- credible sellable v1: 16-24 weeks
- comfortable product-grade v1: 24-32 weeks

These ranges assume:

- no simultaneous rewrite of glide-mq
- no unbounded scope growth
- frozen contracts early
- aggressive parallelization with low ownership overlap
- product-owned retrieval, graph, and eval systems are included in scope

## What Should Not Be Rewritten First

- frontend
- bundled skills content
- personal identity/profile content
- marketplace ecosystem
- every long-tail integration
- glide-mq itself

Use existing UI and sidecar surfaces as stabilizers while the Rust product core becomes coherent.

## What We Still Need to Invent, Not Just Rewrite

The current code gives us substrate, but the product still needs genuinely new systems and sharper boundaries:

- graph-backed operator UX
- matrix/eval data model and scorecard UX
- prompt registry with release controls and approval workflows
- explicit tenant/workspace/project model
- policy authoring and inspection UX
- migration/import/export primitives for agents, memory, prompts, and datasets
- product-grade admin surfaces for retrieval health, graph integrity, source quality, and policy outcomes
- benchmark and regression framework for retrieval, orchestration, and prompt quality
- adoption surfaces: install path, starter templates, import flows, and operator onboarding

These are not optional polish items. They are part of what turns the rewrite into a product.

## Refinement Space

This plan intentionally leaves room for refinement in the following areas:

- exact graph storage strategy
  - relational projections only
  - relational + graph tables
  - dedicated graph engine later
- exact vector stack
  - pgvector only
  - pgvector + local ANN index
  - standalone retrieval service later
- exact plugin transport
  - stdio JSON-RPC
  - HTTP/gRPC
  - MCP-compatible bridge
- exact tenancy model
  - single-tenant first
  - workspace/project now
  - org/workspace/project later
- exact sandbox strategy
  - local process sandbox first
  - remote worker isolation later

These should be decided by short RFCs, not by drift during implementation.

## Required RFCs Before Phase 1

1. Product boundary and non-goals
2. Command/event schema and replay model
3. Task/session/checkpoint lifecycle
4. Retrieval architecture replacing Bedrock KB
5. Graph model and graph query scope
6. Prompt registry and eval matrix model
7. Plugin protocol and transport
8. Tenant/workspace/profile separation
9. Provider abstraction and gateway non-goals
10. Operator control-plane information architecture
11. deployment shape: self-hosted, hybrid, or managed control plane
12. onboarding and starter-template strategy

Initial RFC drafts live in [`docs/design/rfcs/`](./rfcs/README.md).

Supporting decision tables:

- [`COMPATIBILITY_BREAK_MATRIX.md`](./COMPATIBILITY_BREAK_MATRIX.md)
- [`COMPATIBILITY_ROUTE_SSE_CATALOG.md`](./COMPATIBILITY_ROUTE_SSE_CATALOG.md)
- [`GLIDEMQ_RESIDUAL_SCOPE.md`](./GLIDEMQ_RESIDUAL_SCOPE.md)

## glide-mq Boundary Decision

For the Rust rewrite, glide-mq is not the durable source of truth for:

- mailbox state
- checkpoints
- task state
- recovery state
- approval state

Those must live in the Rust-owned store and event model.

glide-mq may remain temporarily as:

- an execution queue substrate
- a fanout/stream transport
- a compatibility bridge for selected async workflows

If glide-mq is used in v1 runtime paths, it should be treated as:

- transport
- queueing infrastructure
- optional acceleration

and not as the canonical owner of product runtime state.

## Acceptance Criteria for Sellable v1

The rewrite is not “done” when parity exists. It is done when:

- the product no longer depends on Bedrock KB for core retrieval
- graph and matrix features are first-class, not hidden side logic
- defaults/profile are separated from engine code
- tasks, checkpoints, mailbox, and recovery are reliable
- prompt and routing decisions are inspectable
- permissions and guardrails are explicit and testable
- the UI can explain what happened, why, and what can be changed
- plugins can be written outside Rust without breaking the core
- a new team can install, onboard, and reach first value without bespoke founder support
- the product reads as a team platform, not as one person's personal agent environment

## Final Recommendation

Rewrite most of the backend in Rust, but do it as a product redesign with frozen contracts and explicit subsystem ownership.

Do not treat the current Go code as the target architecture.
Treat it as:

- semantic reference
- fixture source
- compatibility oracle
- scope inventory

The rewrite should produce a cleaner product than the current system, not a Rust-shaped copy of its current accidents.
