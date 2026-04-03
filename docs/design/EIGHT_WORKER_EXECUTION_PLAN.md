# Cairn Rust Rewrite: 8-Worker Execution Plan

Status: historical execution plan; superseded for active coordination by [`MANAGER_THREE_WORKER_REPLAN.md`](./MANAGER_THREE_WORKER_REPLAN.md)
Audience: product owner, architecture lead, parallel implementation workers  
Depends on: RFC 001 through RFC 014, `RUST_PRODUCT_REWRITE_PLAN.md`

## Purpose

This document turns the stabilized RFC set into an execution plan for 8 senior parallel workers.

It is not another architecture RFC.

It answers:

- who owns what
- what order work should land in
- what can run in parallel
- what must be gated on earlier contracts
- what must be true before the next phase starts

## Planning Assumptions

This plan assumes:

- one architecture owner keeps the contracts coherent
- 8 senior workers can execute in parallel
- the RFC set is now stable enough to implement against
- `glide-mq` is not rewritten first
- local mode and self-hosted team mode are first-class
- the first target is a credible self-hosted sellable v1, not a managed cloud launch

## Delivery Outcome

The target outcome is:

- a Rust backend/workspace that replaces the Go control-plane and runtime core
- compatibility where explicitly preserved
- intentional product breaks where already decided
- a self-hosted team deployment that is supportable and sellable

## Repo Outcome

By the end of the first major execution cycle, `cairn-rs` should contain:

- Rust workspace root
- core crates aligned to the RFC set
- migration tooling and compatibility fixtures
- operator-facing API/SSE surface for the preserved v1 contract
- bootstrap path for local mode and self-hosted team mode

## Operating Rules

### Rule 1: Contract First

No worker should invent state models or API semantics locally.

When an RFC is ambiguous:

- stop the local design drift
- resolve it in docs first
- then resume implementation

### Rule 2: One Owner Per Write Surface

Each worker owns a primary write surface.

Other workers may consume it, but they should not casually edit it without coordination.

### Rule 3: Daily Integration Beats Long-Lived Divergence

Every worker stream should integrate to `main` or an agreed integration branch frequently.

Long-running isolated branches are how contract drift comes back.

### Rule 4: Preserve The Wedge

Do not spend early execution time on:

- long-tail integrations
- marketplace layers
- personal overlay/profile behavior
- managed cloud operations
- polish-only UI work before runtime/operator flows exist

## Worker Ownership

### Worker 1: Contracts, Fixtures, Migration Harness

Owns:

- preserved route catalog enforcement
- SSE compatibility fixtures
- golden fixture harvesting
- migration verification harness
- compatibility-break tracking

Primary outputs:

- fixture corpus
- compatibility tests
- migration comparison scripts

Primary write surface:

- compatibility docs/tests
- migration harness code

### Worker 2: Domain, State Machines, Shared Types

Owns:

- `cairn-domain`
- IDs, commands, events, enums, policy types
- session/run/task/checkpoint lifecycle types
- tenancy primitives and ownership keys

Primary outputs:

- canonical Rust domain crate
- state transition tests
- typed contracts used by all other workers

Primary write surface:

- `cairn-domain`

### Worker 3: Store, Event Log, Synchronous Projections

Owns:

- `cairn-store`
- schema migrations
- event persistence
- synchronous read models
- tenancy/workspace/project scoping in storage

Primary outputs:

- Postgres schema
- SQLite local-mode schema path where required
- store APIs and projection layer

Primary write surface:

- `cairn-store`

### Worker 4: Runtime Spine

Owns:

- `cairn-runtime`
- sessions, runs, tasks, approvals
- checkpoints
- mailbox
- leases, recovery, pause/resume
- external-worker runtime boundary

Primary outputs:

- durable runtime services
- command handlers
- recovery/resume logic

Primary write surface:

- `cairn-runtime`

### Worker 5: Tools, Plugin Host, Isolation

Owns:

- `cairn-tools`
- tool permission model
- builtin tool host
- plugin host integration
- supervised vs sandboxed execution classes

Primary outputs:

- tool invocation layer
- permission enforcement
- plugin execution/runtime glue

Primary write surface:

- `cairn-tools`
- plugin host code

### Worker 6: Memory, Retrieval, Graph

Owns:

- `cairn-memory`
- `cairn-graph`
- ingest/chunk/embed/query flow
- retrieval diagnostics
- graph projections and graph-oriented read surfaces

Primary outputs:

- Bedrock-KB replacement
- owned retrieval service
- graph-backed introspection support

Primary write surface:

- `cairn-memory`
- `cairn-graph`

### Worker 7: Agent Runtime, Prompts, Evals

Owns:

- `cairn-agent`
- `cairn-evals`
- prompt asset/version/release model
- rollout evaluation
- scorecards and routing matrices

Primary outputs:

- ReAct/orchestration layer on top of runtime
- prompt release controls
- eval and comparison infrastructure

Primary write surface:

- `cairn-agent`
- `cairn-evals`

### Worker 8: API, SSE, Signals, Channels, Product Glue

Owns:

- `cairn-api`
- `cairn-signal`
- `cairn-channels`
- preserved API/SSE contract layer
- operator-facing API read models
- bootstrap and productization glue

Primary outputs:

- HTTP/SSE server
- source/channel integration path
- operator-surface backend contract

Primary write surface:

- `cairn-api`
- `cairn-signal`
- `cairn-channels`

## Shared Integration Roles

These are not separate workers. They are standing responsibilities.

### Architecture Owner

Owns:

- RFC amendments
- cross-stream conflicts
- phase gate approval
- “cut or defer” decisions when scope threatens the wedge

### Release Integrator

Owns:

- daily integration branch health
- fixture/test gate visibility
- merge sequencing when multiple streams land on the same day

This can be the same person as the architecture owner early on.

## Execution Waves

### Wave 0: Workspace Bootstrap And Freeze

Duration:

- week 1

Required outputs:

- Rust workspace skeleton
- crate layout stubbed
- CI skeleton
- compatibility fixtures started
- frozen RFC references linked from implementation tickets

Primary workers:

- Worker 1
- Worker 2
- Worker 3
- architecture owner

Gate to exit Wave 0:

- workspace builds
- domain/store crate boundaries are agreed
- preserved route/SSE fixture list is executable
- no unresolved architecture blockers remain in RFCs

### Wave 1: Runtime Spine

Duration:

- weeks 2-4

Primary focus:

- domain types
- store/event log
- runtime lifecycle
- initial API/SSE compatibility shell

Parallel streams:

- Worker 2 builds domain/state crates
- Worker 3 builds storage/event log/sync projections
- Worker 4 builds runtime handlers on top of Worker 2 and Worker 3 outputs
- Worker 8 builds compatibility shell against provisional projections
- Worker 1 converts preserved surfaces into executable fixtures

Gate to exit Wave 1:

- session/run/task/checkpoint/mailbox/approval flow works end-to-end
- SSE shell can emit preserved event names from runtime state
- migration harness can compare at least the core preserved flows against fixtures

### Wave 2: Tooling And Execution Boundary

Duration:

- weeks 3-5

Primary focus:

- tool invocation
- permissions
- plugin protocol integration
- isolation classes

Parallel streams:

- Worker 5 owns tool/plugin host
- Worker 4 integrates tool calls into runtime
- Worker 8 exposes tool/readout surfaces where required

Gate to exit Wave 2:

- tool calls are durable, permissioned, and replayable
- plugin execution class is selectable and observable
- no canonical runtime truth depends on sidecar/plugin-local state

### Wave 3: Owned Retrieval And Graph Foundation

Duration:

- weeks 4-7

Primary focus:

- ingest/query pipeline
- retrieval diagnostics
- graph projections
- import path alignment with RFC 013

Parallel streams:

- Worker 6 owns retrieval/graph
- Worker 3 supports storage/projection needs
- Worker 8 exposes source/status/read surfaces
- Worker 1 expands fixtures for retrieval-facing preserved behaviors where applicable

Gate to exit Wave 3:

- owned retrieval replaces Bedrock KB for core flows
- local-mode degraded retrieval path works
- graph-backed provenance/execution views can be queried through product-shaped APIs

### Wave 4: Prompts, Evals, Agent Runtime

Duration:

- weeks 6-9

Primary focus:

- prompt registry
- releases and rollout selectors
- eval matrices
- agent/orchestrator runtime on top of the new spine

Parallel streams:

- Worker 7 owns prompts/evals/agent loop
- Worker 4 supports runtime hooks
- Worker 6 supports graph/eval linkage
- Worker 8 supports operator/API read surfaces

Gate to exit Wave 4:

- prompt release lifecycle works end-to-end
- selector-based resolution works
- agent runtime can execute on the new Rust spine
- eval scorecards and matrix rows are queryable

### Wave 5: Signals, Channels, Operator Backplane

Duration:

- weeks 8-11

Primary focus:

- high-value signals
- channels and notification routing
- operator-facing API read models
- onboarding/bootstrap backend path

Parallel streams:

- Worker 8 owns signals/channels/API glue
- Worker 5 supports pluginized signal/channel boundaries where needed
- Worker 6 and Worker 7 support graph/eval/operator read surfaces

Gate to exit Wave 5:

- high-value source polling and channel delivery run through Rust core
- operator control-plane backend supports minimum v1 views
- bootstrap/import path is functional for local and team modes

### Wave 6: Hardening, Sellable-V1 Gate

Duration:

- weeks 10-14

Primary focus:

- fixture completion
- migration coverage
- deployment packaging
- auth/entitlement/deployment health surfaces
- cut/defer decisions

Parallel streams:

- all workers fix, simplify, or defer based on gate results
- Worker 1 drives compatibility and migration validation
- Worker 8 and architecture owner drive sellable-v1 surface cuts

Gate to exit Wave 6:

- self-hosted team mode is deployable and supportable
- operator floor is present
- owned retrieval, graph, prompts/evals, runtime, and API/SSE are integrated
- the system reads as one product

## Critical Dependencies

These dependencies should be treated as hard gates:

- Worker 4 depends on Worker 2 and Worker 3 for stable domain/store interfaces
- Worker 5 depends on Worker 4 for runtime invocation hooks
- Worker 6 depends on Worker 3 for storage primitives and on RFC 013-aligned import handling
- Worker 7 depends on Worker 4 for runtime hooks and Worker 6 for graph/eval linkage
- Worker 8 depends on Worker 3 and Worker 4 for product-shaped API read models

## Merge Order Rules

When multiple streams are ready simultaneously:

1. domain and schema contracts first
2. runtime truth second
3. tool/plugin and retrieval foundations third
4. prompts/evals/agent logic fourth
5. signals/channels/operator surfaces fifth
6. hardening and packaging last

## Weekly Cadence

Every week should produce:

- one architecture sync
- one cross-worker integration sync
- one fixture and regression review
- one scope-cut review against the sellable wedge

Every worker should maintain:

- a short ownership log
- current blockers
- next merge target
- tests/fixtures added this week

## Phase Gates

### Gate A: Runtime Spine Ready

Must be true:

- core runtime entities persist and recover correctly
- SSE compatibility shell exists
- migration fixtures cover core runtime flows

### Gate B: Owned Core Systems Ready

Must be true:

- retrieval is product-owned
- graph surfaces exist
- prompt/eval lifecycle exists
- agent runtime works on Rust core

### Gate C: Sellable-V1 Ready

Must be true:

- team self-hosted deployment path works
- entitlement/license visibility exists
- auth/credential health is operator-visible
- minimum operator control plane is usable
- no blocked dependency on managed cloud exists

## Definition Of “Done Enough To Sell”

The product is ready for a first serious self-hosted sale when:

- a team can deploy it without founder-only knowledge
- approvals/checkpoints/replay/recovery actually work
- owned retrieval replaces convenience external KB dependence in core flows
- prompt rollout and eval surfaces are present and legible
- operators can inspect and control the system through one coherent product surface
- commercial gating is explicit without changing the core product model

## What To Cut First If Schedule Slips

Cut in this order:

- long-tail integrations
- extra starter templates
- advanced graph visualization polish
- advanced entitlement administration UX
- richer managed/hybrid preparation work
- post-v1 trust-chain hardening beyond the first paid package boundary

Do not cut first:

- runtime durability
- approvals/checkpoints/recovery
- owned retrieval
- prompt release and eval basics
- operator control-plane minimum
- self-hosted team deployment supportability

## Recommended Next Artifact

After this execution plan, the next planning artifact should be a milestone board derived from this document:

- milestone 0: workspace + fixtures
- milestone 1: runtime spine
- milestone 2: tool/plugin boundary
- milestone 3: owned retrieval + graph
- milestone 4: prompts/evals/agent runtime
- milestone 5: signals/channels/operator floor
- milestone 6: sellable-v1 hardening
