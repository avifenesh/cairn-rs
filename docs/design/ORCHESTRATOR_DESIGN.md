# Cairn Orchestrator Design

Status: design draft  
Author: worker-1  
Date: 2026-04-07  
Depends on: RFC 002 (runtime event model), RFC 005 (task/session/checkpoint lifecycle)

---

## 1. Problem Statement

cairn-rs has a complete runtime spine — sessions, runs, tasks, approvals, checkpoints,
tool invocations, events — but nothing that _drives_ an agent through a goal.  A customer
needs a system that:

1. **Gathers** current context (memory, graph, events, operator settings).
2. **Decides** what to do next via an LLM (generate actions, tool calls, subagent spawns).
3. **Executes** approved actions through the existing task/approval/tool pipeline.
4. **Loops** until done, checkpoint-safe and resumable on restart.

This document designs the Rust orchestrator that closes that gap.  It is not a port of
the Go orchestrator; it is designed around existing cairn-rs types and contracts.

---

## 2. Guiding Principles

**Event-first**: every state change that matters (step started, tool invoked, subagent
spawned, checkpoint saved) must emit a `RuntimeEvent` through the store.  No hidden
in-memory mutations.

**Phase separation**: gather, decide, and execute are separate traits so each can be
tested, mocked, and replaced without touching the loop wiring.

**Checkpoint safety**: the loop must be interruptible at any phase boundary and resumable
from the last checkpoint with no data loss.

**Respect existing contracts**: the orchestrator is a _consumer_ of runtime services
(`RunService`, `TaskService`, `CheckpointService`, `ToolInvocationService`).  It does
not bypass them.

**Approval gate as a first-class pause point**: when a `DecideOutput` contains
`RequiresApproval`, the loop pauses the run (`waiting_approval`), returns control to the
caller, and resumes only when the approval resolves.

---

## 3. Architecture Overview

```
OrchestratorLoop
  │
  ├── GatherPhase  ───► GatherOutput  (context snapshot)
  │     uses: RetrievalService, DefaultsReadModel, EventLog (read),
  │           SessionReadModel, RunReadModel, CheckpointReadModel
  │
  ├── DecidePhase  ───► DecideOutput  (proposed actions)
  │     uses: GenerationProvider (brain), PromptResolver,
  │           ReflectionAdvisory
  │
  └── ExecutePhase ───► ExecuteOutcome (what actually happened)
        uses: TaskService, ApprovalService, ToolInvocationService,
              CheckpointService, SubagentSpawn (TaskServiceImpl::spawn_subagent),
              RunService
```

Each iteration of the loop:
1. Calls `GatherPhase::gather(ctx)` → `GatherOutput`
2. Calls `DecidePhase::decide(ctx, gather_output)` → `DecideOutput`
3. If approval required: pause run, await, resume
4. Calls `ExecutePhase::execute(ctx, decide_output)` → `ExecuteOutcome`
5. Saves checkpoint via `CheckpointService::save`
6. Emits `StepCompleted` SSE frame
7. Checks loop termination: `Done | Failed | MaxIterations | Timeout`

---

## 4. Core Types

### 4.1 `OrchestrationContext`

The immutable context threaded through every phase of a single iteration.

```rust
/// Immutable context for one orchestration step.
#[derive(Clone, Debug)]
pub struct OrchestrationContext {
    /// The project this execution belongs to.
    pub project:    ProjectKey,
    pub session_id: SessionId,
    pub run_id:     RunId,
    /// The task that owns the current execution lease (if any).
    pub task_id:    Option<TaskId>,
    /// Which iteration of the loop we are on (0-based).
    pub iteration:  u32,
    /// The original user goal / input message that started this run.
    pub goal:       String,
    /// Resolved prompt for this agent type, or None if using defaults.
    pub resolved_prompt: Option<ResolvedPrompt>,  // from cairn_agent::orchestrator
    /// Agent type label used for routing, logging, and confidence calibration.
    pub agent_type: AgentType,                    // from cairn_agent::orchestrator
    /// Wall-clock start of the entire run (for timeout enforcement).
    pub run_started_at_ms: u64,
}
```

### 4.2 `GatherOutput`

The context snapshot produced by `GatherPhase`.

```rust
/// Gathered context ready to be injected into a decision prompt.
#[derive(Clone, Debug)]
pub struct GatherOutput {
    /// Relevant memory chunks retrieved by semantic + lexical search.
    /// Source: cairn_memory::RetrievalService::query(RetrievalQuery { ... })
    pub memory_chunks: Vec<RetrievalResult>,      // cairn_memory::retrieval::RetrievalResult

    /// Recent events from the current run (tool calls, approvals, checkpoints).
    /// Source: EventLog::read_by_entity(EntityRef::Run(run_id), after, limit)
    pub recent_events: Vec<StoredEvent>,           // cairn_store::event_log::StoredEvent

    /// Graph neighbourhood: nodes/edges linked to this run and session.
    /// Source: cairn_graph::InMemoryGraphStore::neighbors(run_id, depth=2)
    pub graph_context: Vec<GraphNode>,             // cairn_graph types

    /// Operator settings that apply to this project/tenant.
    /// Source: DefaultsReadModel::list_by_scope(Scope::Project, project_id)
    pub operator_settings: Vec<DefaultSetting>,   // cairn_domain::DefaultSetting

    /// Latest checkpoint data (for resume awareness).
    /// Source: CheckpointService::latest_for_run(&run_id)
    pub checkpoint: Option<CheckpointRecord>,     // cairn_store::projections::CheckpointRecord

    /// Prior step summaries (summarized from recent_events, not raw events).
    pub step_history: Vec<StepSummary>,
}

/// A compressed record of what happened in a prior step.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepSummary {
    pub iteration:   u32,
    pub action_kind: String,          // "tool_call", "subagent", "continue", etc.
    pub summary:     String,          // human-readable or LLM-generated
    pub outcome:     StepOutcome,     // from cairn_agent::orchestrator
}
```

### 4.3 `DecideOutput`

The proposed action set produced by `DecidePhase`.

```rust
/// One or more proposed actions from the LLM decide phase.
#[derive(Clone, Debug)]
pub struct DecideOutput {
    /// The raw LLM response text (stored for audit/replay).
    pub raw_response: String,
    /// Structured actions the LLM proposed (parsed from response).
    pub actions: Vec<ProposedAction>,
    /// Confidence the model assigned to this decision [0.0, 1.0].
    pub predicted_confidence: f64,
    /// Whether the runtime must gate any action through an approval.
    pub requires_approval: bool,
    /// Model and latency metadata for observability.
    pub model_id: String,
    pub latency_ms: u64,
}

/// A single proposed action.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposedAction {
    /// Call a tool and observe the result.
    ToolCall {
        tool_name: String,
        arguments: serde_json::Value,
        /// ExecutionClass drives the approval gate check.
        /// Source: cairn_domain::ExecutionClass
        execution_class: ExecutionClass,
    },
    /// Spawn a subagent to handle a sub-task.
    SpawnSubagent {
        agent_type: AgentType,
        goal: String,
        /// Whether the parent run should wait for the child to complete.
        blocking: bool,
    },
    /// Send a message to the operator mailbox.
    SendMailbox {
        recipient: String,
        body: String,
    },
    /// Declare the run complete.
    Complete {
        summary: String,
    },
    /// Declare the run failed.
    Fail {
        reason: String,
    },
}
```

### 4.4 `ExecuteOutcome`

What actually happened after executing `DecideOutput`.

```rust
#[derive(Clone, Debug)]
pub struct ExecuteOutcome {
    /// Actions that were executed and their individual results.
    pub executed: Vec<ActionResult>,
    /// Whether execution of this step is fully done.
    pub loop_signal: LoopSignal,      // from cairn_agent::react
}

#[derive(Clone, Debug)]
pub struct ActionResult {
    pub action: ProposedAction,
    pub status: ActionStatus,
    /// For tool calls: the raw tool output.
    pub tool_output: Option<serde_json::Value>,
    /// The ToolInvocationId recorded in the event log (for replay linkage).
    pub invocation_id: Option<ToolInvocationId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionStatus {
    Succeeded,
    Failed { reason: String },
    /// Action was blocked by the approval gate; run enters waiting_approval.
    AwaitingApproval { approval_id: ApprovalId },
    /// Subagent was spawned; child_task_id is the schedulable unit.
    SubagentSpawned { child_task_id: TaskId },
}
```

---

## 5. Trait Definitions

### 5.1 `GatherPhase`

```rust
/// Collects context for a single decision step.
///
/// Implementations pull from:
/// - cairn_memory::RetrievalService  (semantic + lexical memory search)
/// - cairn_store::EventLog           (recent events for this run)
/// - cairn_graph::GraphStore         (execution/provenance neighbourhood)
/// - cairn_store::DefaultsReadModel  (operator settings for project/tenant)
/// - CheckpointService               (latest checkpoint state)
#[async_trait]
pub trait GatherPhase: Send + Sync {
    async fn gather(
        &self,
        ctx: &OrchestrationContext,
    ) -> Result<GatherOutput, OrchestratorError>;
}
```

**Concrete implementation** (`StandardGatherPhase`) uses:
- `RetrievalService::query(RetrievalQuery { project, query_text: ctx.goal, mode: Hybrid, limit: 10 })`
- `EventLog::read_by_entity(EntityRef::Run(ctx.run_id), after: last_event_position, limit: 20)`
- `DefaultsReadModel::list_by_scope(Scope::Project, project_id)`
- `CheckpointReadModel::latest_for_run(&ctx.run_id)` (via `CheckpointService`)
- Graph neighbours from `InMemoryGraphStore` (depth 1-2 from run_id node)

### 5.2 `DecidePhase`

```rust
/// Calls the brain LLM with gathered context and returns proposed actions.
///
/// Implementations use:
/// - cairn_domain::providers::GenerationProvider  (brain tier: gemma-4-31B)
/// - cairn_agent::orchestrator::ResolvedPrompt    (system prompt from PromptResolver)
/// - cairn_domain::providers::ProviderBindingSettings (temperature, max_tokens)
#[async_trait]
pub trait DecidePhase: Send + Sync {
    async fn decide(
        &self,
        ctx: &OrchestrationContext,
        gather: &GatherOutput,
    ) -> Result<DecideOutput, OrchestratorError>;
}
```

**Concrete implementation** (`LlmDecidePhase`):
1. Builds a system prompt: resolved prompt text + operator settings + tool catalogue
2. Builds a user turn: goal + memory chunks + step history + graph context
3. Calls `GenerationProvider::generate(model_id, messages, settings)` on the brain provider
4. Parses the response with a tool-call parser (JSON extraction for structured output)
5. Checks `ConfidenceCalibrator::calibrate(project)` to adjust `predicted_confidence`
6. Sets `requires_approval = true` when any action has `ExecutionClass::Sensitive`

### 5.3 `ExecutePhase`

```rust
/// Runs approved actions through the existing runtime pipeline.
///
/// Implementations use:
/// - ToolInvocationService   (record_start, record_completed, record_failed)
/// - TaskServiceImpl::spawn_subagent  (child task + SubagentSpawned event)
/// - ApprovalService         (create_request for gated actions)
/// - RunService              (complete/fail/pause on terminal outcomes)
/// - MailboxService          (send for SendMailbox actions)
/// - CheckpointService       (save after each successful tool call)
#[async_trait]
pub trait ExecutePhase: Send + Sync {
    async fn execute(
        &self,
        ctx: &OrchestrationContext,
        decide: &DecideOutput,
    ) -> Result<ExecuteOutcome, OrchestratorError>;
}
```

**Concrete implementation** (`RuntimeExecutePhase`):
- For `ToolCall`: calls `ToolInvocationService::record_start`, dispatches via `PluginHost`
  or inline handler, calls `record_completed` or `record_failed`.  Saves checkpoint after
  each successful tool call.
- For `SpawnSubagent`: calls `TaskServiceImpl::spawn_subagent(parent_run_id, parent_task_id,
  child_task_id, child_session_id, child_run_id=None)`.  If `blocking`, transitions parent
  run to `waiting_dependency` via `RunService`.
- For `SendMailbox`: calls `MailboxService::send`.
- For `Complete`: calls `RunService::complete(&ctx.run_id)`.
- For `Fail`: calls `RunService::fail(&ctx.run_id, FailureClass::AgentError)`.
- For gated actions when `requires_approval`: calls `ApprovalService::request` and
  transitions run to `waiting_approval`.

---

## 6. `OrchestratorLoop`

```rust
/// Ties gather → decide → execute together for a run.
///
/// Loop contract (per RFC 005):
/// - a single `OrchestratorLoop` instance owns one run's execution
/// - it is the only writer to run/task state for that run
/// - it emits all events through the runtime store (never direct store writes)
pub struct OrchestratorLoop<G, D, E> {
    gather: G,
    decide: D,
    execute: E,
    config: LoopConfig,
}

pub struct LoopConfig {
    /// Maximum iterations before failing with MaxIterations.
    pub max_iterations: u32,
    /// Wall-clock timeout for the entire run in milliseconds.
    pub timeout_ms: u64,
    /// Checkpoint strategy: after every N tool calls, or every step.
    pub checkpoint_strategy: CheckpointStrategy,
}

impl<G, D, E> OrchestratorLoop<G, D, E>
where
    G: GatherPhase,
    D: DecidePhase,
    E: ExecutePhase,
{
    /// Drive the full GATHER → DECIDE → EXECUTE loop for a run.
    ///
    /// Returns when the loop reaches a terminal state (Done, Failed, MaxIterations,
    /// Timeout) or when execution must pause for an approval or subagent dependency.
    pub async fn run(
        &self,
        ctx: OrchestrationContext,
    ) -> Result<LoopTermination, OrchestratorError>;
}

/// Why the loop stopped.
#[derive(Clone, Debug)]
pub enum LoopTermination {
    /// Agent declared itself done.
    Completed { summary: String },
    /// Agent failed or we hit an unrecoverable error.
    Failed { reason: String },
    /// Loop hit the iteration cap.
    MaxIterationsReached,
    /// Timed out. Runtime will mark run as failed with timed_out failure class.
    TimedOut,
    /// Run is now waiting_approval; loop will be resumed by approval resolution.
    WaitingApproval { approval_id: ApprovalId },
    /// Run is waiting for a child subagent. Loop will be resumed by dependency signal.
    WaitingSubagent { child_task_id: TaskId },
}
```

**Loop body pseudocode** (one iteration):

```
for iteration in 0..config.max_iterations {
    // 1. Check timeout
    if elapsed > config.timeout_ms {
        run_service.fail(&run_id, FailureClass::Timeout).await?;
        return Ok(LoopTermination::TimedOut);
    }

    // 2. Save iteration start checkpoint
    if should_checkpoint(iteration, config) {
        checkpoint_service.save(&project, &run_id, new_checkpoint_id()).await?;
    }

    // 3. GATHER
    let gather_output = gather.gather(&ctx).await?;

    // 4. DECIDE
    let decide_output = decide.decide(&ctx, &gather_output).await?;

    // Emit StepCompleted SSE frame (non-blocking)
    emit_sse_step_frame(&ctx, &decide_output);

    // 5. Approval gate
    if decide_output.requires_approval {
        let approval_id = approval_service.request(...).await?;
        run_service.transition_to_waiting_approval(&run_id).await?;
        return Ok(LoopTermination::WaitingApproval { approval_id });
    }

    // 6. EXECUTE
    let execute_outcome = execute.execute(&ctx, &decide_output).await?;

    // 7. Check loop signal
    match execute_outcome.loop_signal {
        LoopSignal::Done => {
            run_service.complete(&run_id).await?;
            return Ok(LoopTermination::Completed { ... });
        }
        LoopSignal::Failed { reason } => {
            run_service.fail(&run_id, FailureClass::AgentError).await?;
            return Ok(LoopTermination::Failed { reason });
        }
        LoopSignal::WaitSubagent { child_task_id } => {
            run_service.transition_to_waiting_dependency(&run_id).await?;
            return Ok(LoopTermination::WaitingSubagent { child_task_id });
        }
        LoopSignal::Continue => {
            ctx.iteration += 1;
            continue;
        }
    }
}
return Ok(LoopTermination::MaxIterationsReached);
```

---

## 7. Checkpoint and Resume Protocol

The loop must be resumable from a `CheckpointRecord` after restart.

### Checkpoint data schema

`CheckpointRecord.data` (a `serde_json::Value`) stores:

```json
{
  "iteration": 3,
  "step_history": [...],
  "goal": "original user goal",
  "last_action_kind": "tool_call",
  "last_tool_name": "search_memory",
  "gather_output_hash": "sha256:..."
}
```

### Resume path

When a task is re-claimed after a restart (via `RecoveryServiceImpl`):

1. `CheckpointService::latest_for_run(&run_id)` → `Some(checkpoint)`
2. Deserialize `checkpoint.data` → `ResumeCursor { iteration, step_history, goal }`
3. Build `OrchestrationContext { iteration: cursor.iteration, goal: cursor.goal, ... }`
4. Re-enter `OrchestratorLoop::run(ctx)` — gather will see current memory/events

This is idempotent: a re-gathered, re-decided step may repeat the last action, but
tool invocations have idempotency keys (`invocation_id`) that prevent double-execution
at the tool layer.

---

## 8. Approval Gate Integration

The approval gate is embedded in the loop transition, not in `ExecutePhase`.

Flow:
```
DecidePhase returns requires_approval = true
    │
    ├── ExecutePhase::execute skips execution (returns AwaitingApproval status)
    ├── OrchestratorLoop calls ApprovalService::request(run_id, requirement=Required)
    ├── RunService transitions run → waiting_approval
    ├── Loop returns WaitingApproval to its caller (HTTP handler or task worker)
    │
    [operator resolves via POST /v1/approvals/:id/resolve]
    │
    ├── ApprovalService emits ApprovalResolved event
    ├── Signal or task re-claim wakes the loop
    └── Loop re-enters: gather → decide → execute (now approved)
```

The `DecidePhase` sets `requires_approval` based on `ExecutionClass` of proposed actions:
- `ExecutionClass::Safe` → no gate
- `ExecutionClass::Sensitive` → requires_approval
- `ExecutionClass::Privileged` → always gates, regardless of settings

---

## 9. Subagent Spawning

Subagent spawning follows RFC 005 §Subagent Linkage exactly.

In `RuntimeExecutePhase::execute` for a `SpawnSubagent` action:

```rust
let child_task_id  = TaskId::new(ulid());
let child_session_id = SessionId::new(ulid());

// Uses TaskServiceImpl::spawn_subagent which emits:
// 1. TaskCreated { child_task_id, parent_run_id, parent_task_id }
// 2. SubagentSpawned { parent_run_id, child_task_id, child_session_id, child_run_id: None }
runtime.tasks.spawn_subagent(
    &ctx.project,
    ctx.run_id.clone(),
    ctx.task_id.clone(),
    child_task_id.clone(),
    child_session_id,
    None, // child run created when child task transitions leased → running
).await?;
```

If `blocking = true`, the parent run transitions to `waiting_dependency`.  When the child
task completes, a dependency resolution sweep (existing `TaskService::check_dependencies`)
re-queues the parent task.

---

## 10. Event Emissions

Per RFC 002, every observable state change emits a `RuntimeEvent`.

Events emitted by the orchestrator (on top of what services already emit):

| Trigger | Event | Emitted by |
|---------|-------|-----------|
| Step started | `TaskStateChanged { to: Running }` | `TaskService::start` |
| Step decide done | (SSE frame via `runtime_sse_tx`) | `OrchestratorLoop` |
| Tool call | `ToolInvocationStarted`, `ToolInvocationCompleted/Failed` | `ToolInvocationService` |
| Subagent spawn | `SubagentSpawned`, `TaskCreated` | `TaskServiceImpl::spawn_subagent` |
| Approval gate | `ApprovalRequested` | `ApprovalService` |
| Checkpoint saved | `CheckpointRecorded` | `CheckpointService::save` |
| Run complete | `RunStateChanged { to: Completed }` | `RunService::complete` |
| Run failed | `RunStateChanged { to: Failed }` | `RunService::fail` |
| Run waiting approval | `RunStateChanged { to: WaitingApproval }` | `RunService` |
| Run waiting dep | `RunStateChanged { to: WaitingDependency }` | `RunService` |

---

## 11. How Each Phase Uses Existing Services

### `StandardGatherPhase` dependencies

```rust
pub struct StandardGatherPhase<R, S, G> {
    retrieval: Arc<R>,         // cairn_memory::RetrievalService
    store: Arc<S>,             // cairn_store::EventLog + DefaultsReadModel + CheckpointReadModel
    graph: Arc<G>,             // cairn_graph::InMemoryGraphStore
    event_lookback: usize,     // how many recent events to include (default: 20)
    memory_results: usize,     // how many memory chunks to include (default: 10)
}
```

### `LlmDecidePhase` dependencies

```rust
pub struct LlmDecidePhase<P, R> {
    provider: Arc<P>,         // cairn_domain::providers::GenerationProvider (brain)
    prompt_resolver: Arc<R>,  // cairn_agent::hooks::PromptResolver
    calibrator: Arc<ConfidenceCalibrator<...>>,  // cairn_runtime::services::confidence_calibrator
    tool_catalogue: ToolCatalogue,  // registered tool definitions (name, description, schema)
}
```

### `RuntimeExecutePhase` dependencies

```rust
pub struct RuntimeExecutePhase<S> {
    store: Arc<S>,             // EventLog for writing
    runtime: Arc<InMemoryServices>,  // runs, tasks, approvals, checkpoints, mailbox
    tool_registry: Arc<dyn ToolRegistry>,  // dispatches actual tool calls
    plugin_host: Arc<Mutex<StdioPluginHost>>,  // for plugin-backed tools
}
```

---

## 12. `OrchestratorError`

```rust
#[derive(Debug)]
pub enum OrchestratorError {
    Gather(String),
    Decide(String),
    Execute(String),
    Runtime(RuntimeError),           // from cairn_runtime::error
    Store(StoreError),               // from cairn_store::error
    MaxIterations { limit: u32 },
    Timeout,
    ApprovalDenied { approval_id: ApprovalId },
}
```

---

## 13. Where This Lives

New crate: `crates/cairn-orchestrator/`

```
crates/cairn-orchestrator/
  src/
    lib.rs           — re-exports
    context.rs       — OrchestrationContext, GatherOutput, DecideOutput, ExecuteOutcome
    phases/
      mod.rs
      gather.rs      — GatherPhase trait + StandardGatherPhase
      decide.rs      — DecidePhase trait + LlmDecidePhase
      execute.rs     — ExecutePhase trait + RuntimeExecutePhase
    loop_.rs         — OrchestratorLoop, LoopConfig, LoopTermination
    error.rs         — OrchestratorError
    tools/
      mod.rs         — ToolRegistry trait, ToolCatalogue
      builtin.rs     — built-in tools (search_memory, read_document, send_mailbox)
  Cargo.toml
    dependencies:
      cairn-domain
      cairn-runtime
      cairn-store
      cairn-memory
      cairn-graph
      cairn-agent     (for StepOutcome, AgentType, AgentConfig, hooks)
      cairn-evals     (for confidence calibration)
      async-trait
      serde, serde_json
      tokio
```

The HTTP handler (`POST /v1/runs/:id/execute` or similar) in `cairn-app/src/`
(routed via `bin_handlers.rs` and the `handlers/` directory) will:
1. Look up the run and task
2. Claim the task lease
3. Build `OrchestrationContext` from the run record
4. Instantiate `OrchestratorLoop` with concrete phases
5. Call `loop.run(ctx).await`
6. Release or complete the task lease based on `LoopTermination`

---

## 14. Open Questions for Avi

1. **Tool catalogue source**: Should tools be registered statically in `ToolCatalogue`,
   loaded from the plugin registry (`InMemoryPluginRegistry`), or both?  The plugin host
   already exists; suggest: built-ins statically, plugins dynamically from registry.

2. **Approval resume trigger**: Approval resolution currently emits `ApprovalResolved`.
   The loop needs a way to be notified.  Options: (a) the approval resolver calls
   `TaskService::claim` on the paused task to re-enter the loop; (b) a polling sweep;
   (c) a signal (`SignalService`).  Recommend (a) for v1.

3. **Blocking subagents**: When a subagent is `blocking`, the parent enters
   `waiting_dependency`.  The child completion must propagate back.  Should the
   `TaskService::check_dependencies` sweep handle this automatically, or does the
   child's executor explicitly call a "notify parent" API?  Recommend: extend the
   existing `check_dependencies` sweep in `RecoveryServiceImpl`.

4. **Streaming token output**: Should the decide phase stream tokens to the SSE
   channel in real time, or return only the final response?  Streaming requires the
   brain provider to support it.  Recommend: v1 returns complete response; streaming
   is a separate phase enhancement.

5. **Tool result injection**: After a tool call, the observation (tool output) must be
   fed back into the next `DecidePhase` call as a conversation turn.  This is part of
   `GatherOutput::recent_events` (the tool invocation completed event carries the
   result).  Confirm this is sufficient or if a dedicated `ToolObservation` field is
   needed in `GatherOutput`.
