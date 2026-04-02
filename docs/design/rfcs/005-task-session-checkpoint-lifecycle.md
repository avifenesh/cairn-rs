# RFC 005: Task, Session, Checkpoint Lifecycle

Status: draft  
Owner: runtime lead  
Depends on: [RFC 002](./002-runtime-event-model.md), [RFC 008](./008-tenant-workspace-profile.md)

## Summary

This RFC defines the canonical runtime lifecycle for:

- sessions
- runs
- tasks
- checkpoints
- pause/resume
- subagent linkage

The goal is to give runtime, storage, API, and graph workers one state model.

## Core Entity Relationships

### Session

A session is the long-lived conversational or operational context.

- a session belongs to one project
- a session contains zero or more runs
- a session may exist before or after active execution

### Run

A run is a single execution attempt inside a session.

- a run belongs to one session
- a run may reference `parent_run_id` when spawned from another run
- a run may create tasks, approvals, tool invocations, and checkpoints
- a run is the primary execution unit for replay and runtime inspection

### Task

A task is a schedulable unit of work.

- a task belongs to one project
- a task may be linked to a parent run
- a task may spawn a child run for a subagent
- a task is the primary leased/claimable unit for asynchronous execution

### Checkpoint

A checkpoint is a durable recovery point for a run.

- a checkpoint belongs to one run
- a run may have many checkpoints over time
- one checkpoint may be marked latest for recovery

## Canonical State Machines

### Session States

- `open`
- `completed`
- `failed`
- `archived`

Rules:

- sessions start as `open`
- sessions do not use `paused`; pause belongs to runs/tasks
- session state is derived from run outcomes plus explicit close/archive actions

### Session Outcome Derivation

Session outcome is derived by these rules:

1. if the session is explicitly archived, state is `archived`
2. else if any run in the session is non-terminal, state remains `open`
3. else if the latest root run ended `failed`, the session is `failed`
4. else if the latest root run ended `completed` or `canceled`, the session is `completed`
5. else the session remains `open` until explicitly reconciled

`root run` means a run in the session with no `parent_run_id`.

Child runs created in child sessions do not directly derive the parent session outcome.
They affect the parent only through the parent run/task transition that records dependency success or failure.

This prevents child-run failures from silently overriding the parent session outcome while still allowing the parent run to fail explicitly when child work fails.

### Run States

- `pending`
- `running`
- `waiting_approval`
- `paused`
- `waiting_dependency`
- `completed`
- `failed`
- `canceled`

Rules:

- a run begins in `pending` or directly in `running`
- a run enters `waiting_approval` when blocked on a human decision
- a run enters `paused` when explicitly suspended for resume later
- a run enters `waiting_dependency` when blocked on subagent or task completion
- `completed`, `failed`, and `canceled` are terminal

### Task States

- `queued`
- `leased`
- `running`
- `waiting_approval`
- `paused`
- `waiting_dependency`
- `retryable_failed`
- `completed`
- `failed`
- `canceled`
- `dead_lettered`

Rules:

- `queued` is the default submission state
- `leased` means a worker owns the right to start or continue execution
- `running` means execution has actually begun
- `retryable_failed` is non-terminal and may return to `queued`
- `failed`, `completed`, `canceled`, and `dead_lettered` are terminal

### Checkpoint States

Checkpoints should not be modeled as a large public state machine.

Instead:

- checkpoints are immutable recovery records
- one checkpoint per run may be marked `latest`
- a latest checkpoint may later become `superseded`
- restored checkpoints are linked by restore events, not mutated into a new state bucket

## Leases and Heartbeats

Tasks are the only leased execution entity in v1.

Lease model:

- `lease_owner`
- `lease_expires_at`
- `lease_token` or monotonic version for stale-write protection

Rules:

- leasing moves a task from `queued` to `leased`
- the worker must either:
  - transition to `running`
  - heartbeat to extend the lease
  - or release/fail the task
- expired leased tasks are recovered by the runtime and may be requeued or failed based on policy

Heartbeat rule:

- heartbeat extends the lease on the canonical Rust-owned task state
- queue/sidecar heartbeat semantics must not be canonical

## Pause and Resume

Pause applies to runs and tasks.

Pause requires:

- reason
- actor/source
- optional resume trigger
- optional resume-after timestamp

Resume requires:

- trigger source
- linkage to the prior paused state

Rules:

- pausing a run does not destroy its checkpoints
- pausing a task preserves task identity and ownership linkage
- explicit resume transitions back to `pending`, `queued`, or `running` depending on entity and cause

## Recovery Rules

Recovery must handle:

- expired task leases
- interrupted runs
- incomplete checkpoints
- blocked-on-dependency states where the dependency has already finished

Recovery should produce explicit runtime events.

Recovery rules:

- task recovery is task-centric
- run recovery uses the latest checkpoint
- recovery must be idempotent
- recovery must never require the sidecar to be the source of truth

## Subagent Linkage

Subagent execution must be represented explicitly.

Rules:

- spawning a subagent creates:
  - a child task
  - a child session
  - optionally an initial child run
- the parent run records the linkage and enters `waiting_dependency` if blocked
- the child task is the schedulable ownership unit
- the child session is the conversational/execution context

Required linkage fields:

- parent_run_id
- parent_task_id where applicable
- child_task_id
- child_session_id
- child_run_id if created immediately

### Child Run Timing

Child run creation is not eager by default.

Canonical rule:

- subagent spawn synchronously creates:
  - child task
  - child session
- child run is created when the child task transitions from `leased` to `running`

Why:

- task ownership is the schedulable truth
- child run should represent actual execution, not merely intent
- this keeps recovery and replay aligned with real work start

Allowed exception:

- inline or foreground execution paths may create the child run immediately if the runtime also transitions the child task into execution in the same command flow

Even in that case, the conceptual rule still holds:

- child run creation coincides with actual execution start

## Synchronous vs Background Transitions

### Synchronous

- create session
- create run record
- submit task
- create approval request
- save checkpoint metadata
- pause request acceptance
- resume request acceptance

### Background / Worker-Driven

- task claim
- heartbeat
- task execution progress
- task completion/failure
- recovery sweeps
- dead-letter transitions
- dependency completion propagation

## Minimum Events

At minimum, the runtime must emit events for:

- session created
- run started/completed/failed/canceled
- task submitted/leased/running/completed/failed/retried/canceled/dead-lettered
- approval requested/resolved
- checkpoint saved/superseded/restored
- run paused/resumed
- task paused/resumed
- subagent spawned
- recovery attempted/completed

## Projections Required

- session list/detail
- run timeline
- task queue/detail
- approval inbox
- checkpoint history
- dependency/subagent view

## Non-Goals

For v1, do not optimize for:

- arbitrary workflow graph semantics beyond agent/runtime needs
- fully generic BPMN-style orchestration
- multiple lease models for different task classes

Focus on a clean runtime model for agent execution.

## Open Questions

1. Do we need explicit `timed_out` terminal states in v1, or should timeout collapse into `failed` plus reason?
2. Which pause/resume triggers must exist in v1 beyond human approval and explicit resume-after?

## Decision

Proceed with:

- sessions as long-lived containers
- runs as execution attempts
- tasks as leased schedulable work units
- checkpoints as immutable recovery records
- explicit subagent linkage through child task and child session records
- Rust-owned leases, heartbeats, recovery, and checkpoint truth
