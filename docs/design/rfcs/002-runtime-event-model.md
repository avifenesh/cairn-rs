# RFC 002: Runtime and Event Model

Status: draft  
Owner: runtime lead  
Depends on: [RFC 001](./001-product-boundary.md)

## Summary

The Rust rewrite should use an explicit command -> validation -> event -> projection model for the runtime.

This is not a pure event-sourcing mandate for every subsystem. It is a requirement that critical state transitions be:

- explicit
- durable
- replayable
- idempotent
- inspectable

## Why

The product depends on:

- long-running execution
- suspend/resume
- approvals
- checkpoints
- mailbox delivery
- replay and observability
- graph-backed execution introspection

Those are hard to make reliable if route handlers and runtime loops mutate state implicitly.

## Core Runtime Entities

The runtime model should include at least:

- session
- run
- task
- approval request
- checkpoint
- mailbox message
- tool invocation
- tool result
- signal event
- memory ingest job
- prompt release
- evaluation run

## Commands

Commands are intent records. They may be rejected before any event is emitted.

Examples:

- start_session
- append_user_message
- start_run
- spawn_subagent
- submit_task
- claim_task
- complete_task
- fail_task
- pause_task
- resume_task
- request_approval
- resolve_approval
- save_checkpoint
- load_checkpoint
- send_mailbox_message
- ingest_document
- create_prompt_release
- execute_evaluation

## Events

Events are the durable facts the system records after validation.

Examples:

- session_started
- run_started
- run_completed
- run_failed
- task_submitted
- task_claimed
- task_completed
- task_failed
- task_paused
- task_resumed
- approval_requested
- approval_granted
- approval_denied
- checkpoint_saved
- checkpoint_deleted
- mailbox_message_sent
- mailbox_message_received
- tool_invocation_started
- tool_invocation_completed
- tool_invocation_failed
- signal_ingested
- memory_document_ingested
- prompt_release_created
- evaluation_run_started
- evaluation_run_completed

## Projections

Projections build fast read models over events and durable tables.

Required projections:

- session summary
- task queue / task detail
- approval inbox
- checkpoint status
- mailbox inbox
- run timeline
- tool invocation timeline
- signal feed
- graph edges for execution and provenance
- evaluation scorecards

### Projection Classes

V1 distinguishes between:

- synchronous operational projections
- asynchronous derived projections

Synchronous operational projections are the read models the runtime needs in order to make correct next decisions.

Asynchronous derived projections are rebuildable views for analysis, graphing, search, and operator experience.

## Persistence Rules

- commands are accepted through service boundaries
- validated events are durably recorded
- current-state tables are updated transactionally with event persistence
- projections may be rebuilt from durable state and event history

This implies:

- append-only event log for critical facts
- state tables for efficient operational reads
- idempotency keys for externally triggered commands

### Canonical Event-History Classes

V1 uses two durability classes:

- `full_history`
- `current_state_plus_audit`

`full_history` means:

- every accepted event for the entity is retained in the canonical event log
- entity state may be reconstructed from event history plus snapshots if later introduced
- replay and timeline views are product features, not best-effort logging

`current_state_plus_audit` means:

- the canonical store keeps durable current state
- important transitions still emit durable audit events
- replay does not require reconstructing the entity solely from a complete append-only entity history

### Entity Classification

The following entities are `full_history` in v1:

- session
- run
- task
- approval request
- checkpoint
- mailbox message
- tool invocation

The following entities are `current_state_plus_audit` in v1:

- signal event
- memory ingest job
- prompt release
- evaluation run

This keeps the runtime spine fully replayable while allowing adjacent subsystems with their own RFCs to keep a simpler canonical state model where full entity-event history is not required for correctness.

## Ordering and Concurrency

Rules:

- ordering guarantees are per-entity, not global
- sessions, tasks, approvals, and prompt releases should each have monotonic versioning
- concurrent command handling must fail fast or retry on stale versions
- leases should be explicit for task claims and long-running work

## Checkpoints and Recovery

Checkpoints are first-class runtime state, not just debug artifacts.

Requirements:

- durable checkpoint persistence
- resume from explicit checkpoint state
- incomplete run recovery
- replay-friendly linking between checkpoints, tasks, and runs

### Replay Support In V1

V1 replay support means:

- the system can reconstruct `full_history` entities from durable event history
- synchronous projections may be rebuilt from the canonical event log
- asynchronous derived projections must be rebuildable from canonical runtime state and events
- operator surfaces may replay runtime facts and timelines from durable history
- SSE consumers may resume from durable event positions within the retained replay window

V1 replay support does not require:

- whole-deployment time travel rollback
- arbitrary historical reconstruction for every non-runtime subsystem
- infinite retention for every derived event feed

### Retention Rule

For `full_history` entities in v1:

- canonical runtime event history is retained for the life of the deployment unless an explicit product retention policy later says otherwise
- replayability must not depend on transient broker retention or UI caches

For SSE/event-feed replay in v1:

- the product must support a durable replay window sufficient for client resume and operator inspection
- the replay window may be implemented as a retained subset or cursorable view over canonical event history

## Mailbox Model

Mailbox messages should be durable runtime records with:

- sender
- recipient
- message body
- timestamps
- delivery status
- source run/task linkage where applicable

Mailbox is part of coordination, not just chat decoration.

Canonical ownership rule:

- mailbox durability belongs to the Rust runtime store
- any queue or sidecar transport is non-canonical

If glide-mq or another queue substrate is used, it may transport mailbox events but must not own mailbox truth.

## Tool Event Model

Every tool call should emit structured runtime facts:

- invocation requested
- invocation started
- invocation completed or failed
- permission decision
- execution metadata

This is required for:

- replay
- graph construction
- evals
- audits
- debugging

## API and SSE Relationship

HTTP APIs should submit commands or query projections.

SSE should expose runtime facts and projection updates, not ad-hoc UI-only events.

This gives the frontend and external operators one consistent model.

### Projection Timing Rules

The following must be synchronous with command/event commit in v1:

- entity version advancement
- current-state tables for session, run, task, approval request, checkpoint, mailbox message, and tool invocation
- any read model required to validate the next command against canonical runtime state

The following may be asynchronously materialized in v1:

- graph edges for execution and provenance
- evaluation scorecards and aggregate analytics
- broader signal feeds and cross-cutting operator digests
- search-oriented or reporting-oriented denormalizations

If an operator or API surface needs strict read-after-write correctness for workflow control, it must read from canonical state or synchronous projections, not from asynchronous derived views.

## Sidecar Boundary

The runtime may use glide-mq or another queue substrate during migration, but:

- command acceptance
- durable events
- runtime truth
- checkpoints
- mailbox state
- recovery state

must be owned by the Rust runtime and store.

Queue infrastructure may remain for async dispatch or streaming, but not as the canonical source of runtime truth.

### External Worker Rule

V1 supports external execution workers, but not external canonical command processors.

That means:

- canonical command validation and event persistence remain owned by the Rust runtime
- external workers may claim leased tasks, perform work, emit progress/heartbeats, and report outcomes through runtime-owned APIs
- external workers must not write canonical runtime events directly
- any sidecar, queue, or worker substrate remains transport/execution infrastructure, not a second source of truth

This keeps external execution possible without splitting the event model across multiple authorities.

## What Not To Do

Do not:

- hide business-critical state only in transient async workers
- let route handlers write arbitrary state without command/event semantics
- make replay depend on unstructured logs
- conflate internal transient messages with durable state changes

## Open Questions

1. Should v1 expose full event-history export for `full_history` entities directly, or rely on projections plus the canonical event log internally?
2. Should the SSE replay window be bounded only by retention policy, or should v1 define a smaller operational replay SLA for client resume?

## Decision

Implement the runtime around explicit commands, durable events, and projections.

Do not require pure event sourcing everywhere, but do require replayable, inspectable state transitions for all critical runtime entities.

Proceed assuming:

- the runtime spine uses the `full_history` vs `current_state_plus_audit` split defined above
- replay in v1 covers canonical runtime entities and rebuildable projections, not whole-system time travel
- synchronous projections are reserved for correctness-critical runtime state
- external workers may execute and report work, but canonical command handling and event persistence remain Rust-runtime-owned
