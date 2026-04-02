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

## Persistence Rules

- commands are accepted through service boundaries
- validated events are durably recorded
- current-state tables are updated transactionally with event persistence
- projections may be rebuilt from durable state and event history

This implies:

- append-only event log for critical facts
- state tables for efficient operational reads
- idempotency keys for externally triggered commands

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

## What Not To Do

Do not:

- hide business-critical state only in transient async workers
- let route handlers write arbitrary state without command/event semantics
- make replay depend on unstructured logs
- conflate internal transient messages with durable state changes

## Open Questions

1. Which entities need full event history, and which only need durable current state plus audit events?
2. How much event replay should the product support in v1?
3. Which projections should be synchronous versus asynchronously materialized?
4. Should command handling be in-process only in v1, or should it already support external workers cleanly?

## Decision

Implement the runtime around explicit commands, durable events, and projections.

Do not require pure event sourcing everywhere, but do require replayable, inspectable state transitions for all critical runtime entities.
