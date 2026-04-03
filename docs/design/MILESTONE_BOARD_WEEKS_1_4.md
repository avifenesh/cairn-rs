# Cairn Rust Rewrite: Milestone Board Weeks 1-4

Status: historical milestone board for the original 8-worker execution pass
Audience: architecture owner, release integrator, 8 parallel workers  
Companion docs:

- [`EIGHT_WORKER_EXECUTION_PLAN.md`](./EIGHT_WORKER_EXECUTION_PLAN.md)
- [`MANAGER_THREE_WORKER_REPLAN.md`](./MANAGER_THREE_WORKER_REPLAN.md)
- [`REPO_SCAFFOLDING_TASKS.md`](./REPO_SCAFFOLDING_TASKS.md)

## Purpose

This board turns the 8-worker execution plan into concrete week-by-week delivery targets for the first four weeks.

The goal of weeks 1-4 is not feature completeness.

The goal is to establish:

- a working Rust workspace
- the runtime spine
- executable compatibility fixtures
- the first usable cross-worker integration path

## Week 1: Workspace And Contract Freeze

### Week 1 Outcome

- workspace builds
- crate boundaries exist
- preserved compatibility inventory is executable
- no unresolved architecture blockers remain

### Worker Deliverables

#### Worker 1

- convert preserved route and SSE docs into initial test fixture inventory
- create migration harness layout
- record preserved vs transitional vs intentionally broken surfaces in test form

#### Worker 2

- scaffold `cairn-domain`
- define base IDs, commands, events, tenancy types, and lifecycle enums
- add state-transition unit test skeletons

#### Worker 3

- scaffold `cairn-store`
- create initial migration layout
- add event-log and sync-projection interfaces

#### Worker 4

- scaffold `cairn-runtime`
- define service boundaries for session/run/task/checkpoint/mailbox handling
- wire runtime crate to domain and store interfaces

#### Worker 5

- scaffold `cairn-tools`
- define tool invocation traits and permission boundary interfaces
- stub plugin host integration points

#### Worker 6

- scaffold `cairn-memory` and `cairn-graph`
- define ingest/query/graph projection interfaces
- align storage needs with Worker 3

#### Worker 7

- scaffold `cairn-agent` and `cairn-evals`
- define prompt asset/version/release interfaces
- define eval row and matrix service boundaries

#### Worker 8

- scaffold `cairn-api`, `cairn-signal`, and `cairn-channels`
- define API/SSE boundary modules
- add preserved SSE contract test harness hooks

### Week 1 Gate

- workspace compiles
- every worker has a primary write surface
- compatibility fixtures can run, even if mostly pending/failing

## Week 2: Runtime Truth And Compatibility Shell

### Week 2 Outcome

- core runtime entities persist
- event log works
- initial HTTP/SSE compatibility shell exists

### Worker Deliverables

#### Worker 1

- harvest first golden fixtures from `../cairn`
- turn preserved SSE names and payload families into assertions

#### Worker 2

- finalize session/run/task/checkpoint/mailbox command and event types
- lock shared error and failure-class enums

#### Worker 3

- implement initial Postgres schema and migration runner
- implement sync projections for runtime-critical entities

#### Worker 4

- implement create/start/advance flows for session, run, task
- persist approvals, checkpoints, mailbox records through store layer

#### Worker 5

- implement durable tool-invocation record shape
- integrate permission-decision events with runtime/store contracts

#### Worker 6

- implement document and graph entity persistence skeletons
- align retrieval and graph storage requirements to schema reality

#### Worker 7

- implement prompt release and selector type usage against domain contracts
- define agent-runtime hooks against runtime services

#### Worker 8

- expose initial runtime read endpoints
- expose initial SSE endpoint using preserved event names

### Week 2 Gate

- session/run/task/checkpoint/mailbox/approval state persists end-to-end
- SSE shell emits preserved names from Rust runtime state
- migration harness can compare at least one full preserved runtime flow

## Week 3: Tool Boundary, Recovery, And Owned-Core Foundations

### Week 3 Outcome

- runtime recovery path is credible
- tool execution is durable and permissioned
- retrieval and graph implementation starts moving beyond skeletons

### Worker Deliverables

#### Worker 1

- expand fixture coverage for recovery, pause/resume, and tool-call visibility
- mark any remaining gaps explicitly as pending or intentionally broken

#### Worker 2

- finalize timeout, pause/resume, and external-worker shared types
- lock event shapes used by recovery and replay

#### Worker 3

- implement replay and rebuild support for runtime-critical projections
- add SQLite local-mode support where required by current crate surface

#### Worker 4

- implement recovery, lease, timeout-classification, and pause/resume semantics
- integrate external-worker reporting boundary

#### Worker 5

- implement builtin tool execution path and permission enforcement
- wire plugin host to protocol contract
- implement `supervised_process` and `sandboxed_process` selection path

#### Worker 6

- implement ingest pipeline skeleton
- implement retrieval query path and diagnostics skeleton
- implement graph projection flow for provenance/execution rows

#### Worker 7

- implement prompt registry persistence flow
- implement eval scorecard row creation path
- begin agent runtime execution on top of runtime spine

#### Worker 8

- add operator-facing runtime read models for runs, approvals, and mailbox visibility
- wire source/channel service boundaries to API contracts

### Week 3 Gate

- recovery and replay of runtime truth work in the Rust spine
- tool calls are persisted, replayable, and permissioned
- owned retrieval path exists as code, even if incomplete

## Week 4: Integration Wave And First Internal Alpha Slice

### Week 4 Outcome

- first integrated alpha slice exists
- owned retrieval replaces placeholder dependence in at least one core flow
- prompt/eval and operator surfaces are visible enough to validate direction

### Worker Deliverables

#### Worker 1

- run first broad compatibility comparison against Rust slice
- publish mismatch report and classify each as bug, planned break, or deferred

#### Worker 2

- stabilize shared interfaces based on week-3 integration pain
- make only contract-level changes approved by architecture owner

#### Worker 3

- stabilize migrations and projection correctness
- document any required data backfill/migration assumptions

#### Worker 4

- drive end-to-end runtime slice from command through replay/recovery
- close blocking lifecycle or mailbox defects

#### Worker 5

- close end-to-end tool and plugin host path for one representative plugin category

#### Worker 6

- complete first owned retrieval flow for supported document floor
- expose graph-backed provenance data for runtime/operator use

#### Worker 7

- complete first prompt release + eval + agent execution slice
- make scorecards queryable through product-facing services

#### Worker 8

- expose minimum operator backend slice for:
  - overview
  - runs
  - approvals
  - prompts/evals visibility
- make bootstrap path visible for local or team-mode bring-up

### Week 4 Gate

- first internal alpha slice runs end-to-end
- compatibility mismatches are classified
- owned retrieval, runtime, prompts/evals, and operator API surfaces all exist in one integrated path

## Cross-Week Non-Negotiables

- no worker silently changes RFC-defined semantics
- no new source of canonical runtime truth appears outside Rust runtime/store
- no paid/commercial surface changes core technical behavior without doc update
- every worker updates mailbox status when blocked or after landing a major change

## Recommended Review Rhythm

- Monday: week target lock and dependency review
- Wednesday: integration checkpoint and blocker burn-down
- Friday: gate review, mismatch report, and scope-cut decisions
