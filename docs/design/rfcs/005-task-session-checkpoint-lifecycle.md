# RFC 005: Task, Session, Checkpoint Lifecycle

Status: stub  
Owner: runtime lead  
Depends on: [RFC 002](./002-runtime-event-model.md)

## Purpose

Define the canonical lifecycle for:

- sessions
- runs
- tasks
- checkpoints
- pause/resume/recovery

## Must Decide

1. State machine per entity
2. Lease and claim semantics
3. Recovery and replay expectations
4. Relationship between tasks, sessions, and subagents
5. Which transitions are synchronous versus background

## Blocking Reason

Parallel runtime and storage work should not proceed without a single lifecycle model.
