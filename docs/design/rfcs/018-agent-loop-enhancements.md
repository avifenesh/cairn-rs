# RFC 018: Agent Loop Enhancements

Status: draft (rev 2 — incorporates Plan/Execute generic framing, exploration budget, shell_exec always-External, per-project Guardian inheritance)
Owner: orchestrator/runtime lead
Depends on: [RFC 002](./002-runtime-event-model.md), [RFC 005](./005-task-session-checkpoint-lifecycle.md), [RFC 009](./009-provider-abstraction.md), [RFC 015](./015-plugin-marketplace-and-scoping.md), [RFC 016](./016-sandbox-workspace-primitive.md), [RFC 019](./019-unified-decision-layer.md), [RFC 022](./022-triggers.md)

## Resolved Decisions (this revision)

The following were resolved during the question-by-question pass and are baked into this RFC:

- **Plan run budget**: Plan-mode runs draw from a **separate "exploration" budget**, distinct from the project's execute-run budget. The exploration budget defaults to 10% of the project's execute-run budget unless explicitly overridden. Configurable per project. Plan budgets are typically generous (planning is cheap; you want operators to use it freely) without coupling to production execution spend.
- **Guardian model configuration**: per-project with tenant-level default inheritance. Tenant admin sets a default Guardian model; projects can override or inherit. If neither is set, the Guardian resolver is not in the chain and approvals fall through to the human resolver. **No hardcoded default model** — cairn does not lock in on any provider.
- **`shell_exec` classification**: `shell_exec` is **always `External`** in v1. Even inside a sandbox, a shell command can escape it via network access, interaction with mounted paths, or long-running subprocesses. A team that needs read-only access to file content or search results should use dedicated Observational tools (`grep_search`, `file_read`, `glob_find`, etc.) which are always available in Plan mode. Dynamic per-invocation classification via an execpolicy rule engine (inspired by Codex CLI) is deferred to future work; it requires a per-invocation evaluation path that the v1 `ToolEffect` enum (which is per-tool, not per-invocation) does not support.
- **Tool effect classification**: every tool declares its `ToolEffect` (`Observational` / `Internal` / `External`). Plan mode shows only Observational + Internal tools to the agent. Examples are team-diverse (research, support, ops, content, security, code) — not coding-specific.

## Summary

The cairn orchestrator loop (`gather → decide → execute`) is structurally sound. What it is missing is a small set of control-plane patterns that move it from "runs the LLM in a loop" to "runs the LLM in a loop that knows when to pause, when to plan, when to compact, when to ask, and what it is allowed to see". Four enhancements:

1. **Per-run execution modes** — `Plan`, `Execute`, `Direct`. Plan is non-mutating and produces a markdown artifact. Execute consumes an approved plan. Direct is today's default behavior.
2. **Guardian approval resolver** — when an approval gate fires in an unattended context, a configurable resolver can spawn a sub-run through the provider abstraction (RFC 009) to make the decision. Fail closed on timeout, structured output, scoped to the project's budget.
3. **Inline context compaction** — when the message history exceeds a configured threshold, run a summarization turn and replace history with the summary. Provider-agnostic, runs through the same provider the main loop uses.
4. **Plugin visibility at prompt assembly** — the `VisibilityContext` from RFC 015 is consumed by the orchestrator's prompt builder so agents only see tools, signals, and system instructions for plugins enabled in their project.

Plus one smaller refinement:

- **Tool output token budget** — each tool result is truncated to a configured per-run cap before being appended to the message history, preventing a single large output from blowing the context window.

This RFC is explicitly **not** coding-specific. Every enhancement applies to any agent doing any work: research, writing, data analysis, incident triage, customer support, code, operations. The examples in this RFC use a range of team types intentionally.

**Not in this RFC**:

- memory pipeline (cairn-memory provides the full retrieval pipeline — ingest, chunking, embedding, retrieval with lexical/vector/hybrid modes, reranking, and graph expansion — exposed to agents via the `memory_search` and `memory_store` built-in tools. Internal pipeline stages like multi-hop deep search and entity extraction are invoked by the pipeline but are not separately agent-callable. No new memory system is needed.)
- `apply_patch` tool or anything else specific to code editing
- LSP feedback loops
- anything coupled to a specific LLM provider

## Why

### The current loop works for demos, not production

The orchestrator's gather→decide→execute loop is the right shape. It handles tool calls, it emits events, it respects approval gates. What it does not do:

- **Distinguish planning from execution.** Every run is "do the thing end-to-end". There is no safe mode where the agent can explore, think, and produce a plan for human review before touching anything. A research analyst agent that should produce a report before doing anything destructive has the same execution shape as a migration agent that should run the migration — and that is wrong.
- **Scale to multi-hour runs.** A sufficiently long run hits the model's context window and either fails or loses coherence. There is no compaction path.
- **Get unblocked without a human.** An approval gate in an overnight run pauses until a human resolves it; an 8-hour-old waiting approval for a trivial check is common and kills throughput.
- **Scope what the agent sees.** RFC 015 establishes per-project plugin visibility, but the orchestrator's prompt builder does not yet consume `VisibilityContext`. Agents see every registered tool regardless of project relevance.

These are control-plane gaps, not tool gaps. Cairn-rs has ~31 built-in tool handler files in `crates/cairn-tools/src/builtins/` and a plugin host that can add unlimited more — but **only 6 of those handlers are currently wired into the orchestrate registry** (see §"v1 Tool Registration Pre-requisite" below). The question is how the loop **decides which tools are visible, when to pause for a plan, when to compact, and when to ask a human versus a guardian** — and the answer depends on the tool registration gap being closed first.

### Why these four and not more

The research produced a long list of potential enhancements (memory pipeline extraction, apply-patch, ACI pagination, session forking, learned tool rules, etc.). Most are either coding-specific, already built in cairn-rs, or outside the "v1 dogfood demo" critical path. The four in this RFC are the minimum set that makes the loop viable for a real team task. Everything else is deferred to later work.

## Enhancement 1: Run Execution Modes

### The shape

Every team that uses cairn has the same workflow split: **do the work inside the system** (gather information, form an opinion, draft something for review), then **act on the outside world** (post, send, publish, merge, notify, deploy). The split exists for research analysts, customer success, operations, security, marketing, support, and code — it is not a coding concept.

Cairn needs to let a team agent perform the first half safely and have a human or automated gate between it and the second half.

Three run modes:

```rust
pub enum RunMode {
    /// Default. The agent gathers context, decides, and acts in one continuous
    /// loop, including external actions. This is today's behavior and the
    /// right choice when the team trusts the agent to complete an end-to-end
    /// task autonomously (e.g. posting a low-risk comment, updating an
    /// internal dashboard).
    Direct,

    /// The agent does inside-the-system work — it observes, reads context,
    /// writes notes, drafts artifacts, queries memory — but it does not
    /// perform any action that changes the outside world. The run terminates
    /// when the agent emits a `<proposed_plan>` markdown block summarizing
    /// what it found and what it intends to do. The plan becomes a durable
    /// run artifact.
    Plan,

    /// Execution of a previously-approved plan. The new run opens with the
    /// approved plan artifact as its initial context and is allowed to take
    /// external actions. Created explicitly by an operator after reviewing a
    /// Plan-mode run's artifact.
    Execute { plan_run_id: RunId },
}
```

Default is `Direct`. Nothing changes for callers that do not set the mode.

### What "inside" and "outside" mean

To enforce Plan mode consistently across team types, every tool is classified by what it touches:

```rust
pub enum ToolEffect {
    /// Observation only. Reads cairn-internal data or external data without
    /// changing anything.
    /// Examples: memory_search, grep_search, file_read, github.get_issue,
    /// web_fetch (GET), graph_query, list_runs, get_approvals, tool_search
    /// (read-only discovery of available tools).
    Observational,

    /// Internal work. Writes to cairn-owned state — scratch pad, memory
    /// store, planning notes, sandbox filesystem inside the run's sandbox —
    /// but never touches a system outside cairn's boundary.
    /// Examples: scratch_pad, memory_store, file_write (into the run's
    /// sandbox), create_task (cairn's own task queue).
    Internal,

    /// External action. Touches a system outside cairn. Any outbound API
    /// call that creates or changes state, any notification delivered to a
    /// human, any subprocess that can escape the sandbox, any write to a
    /// shared resource.
    /// Examples: shell_exec, http_request (POST/PUT/DELETE),
    /// github.create_pull_request, github.comment_on_issue, notify_operator,
    /// slack.send_message, any plugin tool that is not explicitly observational.
    External,
}
```

The classification is **per tool**, set by the tool's author:

- every built-in cairn tool declares its `ToolEffect` in its `ToolHandler` impl
- every plugin tool declares it in the plugin manifest (new field on `PluginCapability::ToolProvider`)

**Plan mode's rule**: the prompt builder includes only `Observational` and `Internal` tools. The LLM does not see `External` tools, so it cannot propose to use them. The agent can still do real work — it can search memory, query a database, read files in the sandbox, draft notes in the scratch pad, accumulate findings — and then summarize everything into a plan artifact. It just cannot push anything outward.

### Why this framing fits every team

The `Observational / Internal / External` split is not a coding abstraction. It holds across:

**Research analyst**: Observational = `memory_search`, `web_fetch`, `grep_search`. Internal = `scratch_pad` (draft a literature review), `memory_store` (save a finding). External = `notify_operator` (send the final report via Slack). Plan mode lets the analyst accumulate findings and draft the survey. Execute mode delivers it.

**Customer success**: Observational = `get_customer_history`, `search_tickets`. Internal = `scratch_pad` (draft a response). External = `reply_to_ticket`, `escalate_to_human`. Plan mode lets the agent find the right answer and draft a reply. Execute mode sends it.

**Operations / incident response**: Observational = `query_metrics`, `get_log_slice`, `list_incidents`. Internal = `scratch_pad` (build an incident timeline). External = `run_migration`, `restart_service`, `notify_pager`. Plan mode produces a root-cause analysis and a proposed runbook. Execute mode runs it.

**Security review**: Observational = `grep_search`, `file_read`, `graph_query`. Internal = `scratch_pad`, `memory_store` (record findings). External = `file_unrestricted_cve`, `notify_security_team`. Plan mode audits the code. Execute mode files.

**Marketing / content**: Observational = `memory_search`, `web_fetch`, `get_brand_guide`. Internal = `scratch_pad` (draft copy), `memory_store` (save style notes). External = `publish_to_cms`, `schedule_social_post`. Plan mode drafts. Execute mode publishes.

**Code**: Observational = `grep_search`, `file_read`, `github.get_issue`. Internal = `file_write` into sandbox, `scratch_pad`. External = `github.create_pull_request`, `shell_exec` against anything that leaves the sandbox. Plan mode reads code and writes design notes. Execute mode opens the PR.

The last example is the odd one: `file_write` counts as Internal because it writes to the run's sandbox, which is cairn-owned state. The sandbox is ephemeral, scoped to the run, and tied to RFC 016's lifecycle. The moment the agent pushes a branch or opens a PR, it leaves the sandbox boundary and becomes External.

This is the correct line. A research agent writing a draft into the scratch pad, a support agent drafting a reply, and a code agent editing files in its sandbox are all doing the same thing: **working privately inside cairn's boundary**. A plan run should allow all of it. An execute run is what takes the private work and pushes it outward.

### `shell_exec` classification

`shell_exec` is always `External`. Even inside a sandbox, a shell command can escape it (network access, interaction with mounted paths, long-running processes). A team that needs read-only shell access should use dedicated observational tools (`grep_search`, `file_read`, `glob_find`, `json_extract`, etc.) which are always available in Plan mode. If an operator genuinely wants an unattended research run that can spawn arbitrary shells, they use Direct mode and accept the risk.

Similarly, `http_request` is classified as `External` because it supports POST/PUT/DELETE. Agents in Plan mode that need to read web content use `web_fetch`, which is `Observational` and restricted to GET.

### State machine extension

RFC 005's run state machine is unchanged. `RunMode` is metadata on the run record, set at run creation and immutable. The restrictions the modes impose are enforced at the gather/decide/execute boundary (prompt construction and tool dispatch), not in the state machine.

Events gain an optional `run_mode: RunMode` field at run creation time.

### Plan artifact

The agent terminates a Plan run by emitting a `<proposed_plan>` block in its response:

```markdown
<proposed_plan>
# Plan: {{task goal}}

## What I found
...summary of observations and research...

## What I propose to do
1. Step one
2. Step two
3. Step three

## What I need approved
- Access to X
- Authorization to take action Y
- Budget of Z tokens

## Open questions for the operator
- ...
</proposed_plan>
```

The markdown is intentionally a free-text shape. A research agent can use it to present a literature review outline, a support agent can use it to show a drafted reply, an ops agent can use it to propose a runbook. The structure inside the block is guidance, not schema — whatever the agent needs to communicate to its operator.

The orchestrator parses `<proposed_plan>` blocks out of the agent's output, stores the markdown as a run artifact, and emits a `PlanProposed` event. The run terminates with state `Completed` and `outcome: plan_proposed`.

### Plan review flow

An operator reviewing the plan in the dashboard can:

- **Approve** — the operator explicitly creates a new run in `Execute` mode referencing the plan (no auto-creation; see Open Questions)
- **Revise** — the operator sends the plan back to a new `Plan`-mode run with their comments prepended to the initial context
- **Reject** — the plan is marked rejected; no execution run is created; the original run's artifact is preserved for audit

### Why modes are run-level, not turn-level

Codex CLI handles modes as per-turn switches inside one conversation. For cairn's multi-tenant, multi-user model, per-run is simpler and more auditable: a run is either a plan or an execution, never both, and the mode is immutable for the run's lifetime. If an agent needs to replan mid-execution, it creates a child run in Plan mode and the parent waits on its dependency (existing subagent model per RFC 005).

## Enhancement 2: Guardian Approval Resolver

### The gap

Approvals today are resolved by humans via the dashboard. A run in `waiting_approval` stays there until an operator clicks Approve or Deny. For unattended or high-volume runs this is a throughput killer — and even with reasonable response times, most approvals are obvious (e.g. "can the agent comment on this issue?" → yes, always). Asking a human for obvious things trains them to click approve without reading, which defeats the point of the gate.

### The shape

Introduce an `ApprovalResolver` trait with two implementations:

```rust
#[async_trait]
pub trait ApprovalResolver: Send + Sync {
    /// Produce a decision for the given approval request. Returns Ok(None)
    /// if this resolver cannot or should not handle this request (the caller
    /// will fall back to the next resolver in the chain).
    async fn resolve(
        &self,
        request: &ApprovalRequest,
        context: &ApprovalContext,
    ) -> Result<Option<ResolverDecision>, ResolverError>;
}

pub struct ResolverDecision {
    pub outcome: ApprovalOutcome,    // Approved | Denied
    pub rationale: String,
    pub risk_level: RiskLevel,       // Low | Medium | High
    pub resolved_by: ResolverId,     // "human:operator_id" | "guardian:model_id"
    pub ttl: Option<Duration>,       // how long to cache this decision
}
```

Two v1 resolvers:

- **`HumanResolver`** — blocks until a human provides a decision via the existing approval API. This is today's behavior.
- **`GuardianResolver`** — spawns a short sub-run through the provider abstraction (RFC 009) that asks the model to evaluate the request and return a structured decision. Fail closed on timeout, malformed output, or provider error.

A project's approval policy (per-category, per-tenant, or per-project) declares which resolvers are in the chain. A project that wants full-human gates sets `resolver_chain = ["human"]`. A project that trusts the guardian for low-risk decisions sets `resolver_chain = ["guardian", "human"]` — guardian tries first, falls through to human if the guardian returns `None` or fails.

### Guardian flow

```text
ApprovalRequest arrives
  ↓
resolve(request, context):
  1. Load guardian config for the project
     - model_id (configurable, no hardcoded default)
     - timeout_ms (default 60s)
     - max_context_tokens (default 8K messages + 8K tool history)
     - risk_ceiling (default: Low only; anything Medium+ falls through)
  2. Build transcript from context:
     - prior N messages (truncated to context budget)
     - prior tool calls (truncated to context budget)
     - the specific request (tool name, args, resource target)
  3. Construct a prompt with:
     - system message describing the guardian's role
     - transcript
     - structured output requirement
  4. Call provider (RFC 009) with a short timeout
  5. Parse response:
     - expect { outcome, rationale, risk_level }
     - if parse fails or timeout: return Ok(None) (fall through)
     - if risk_level > risk_ceiling: return Ok(None) (fall through)
     - otherwise: return Ok(Some(ResolverDecision { ... }))
  6. Emit GuardianResolved event with the decision summary for audit.
     The full guardian transcript (input prompt + model response) is
     persisted alongside the approval record in the event log; the
     GuardianResolved event itself carries the audit marker (approval_id,
     outcome, risk_level, rationale, token counts, duration) but NOT the
     full transcript — replay requires joining the event to the
     corresponding approval record.
```

The guardian's rationale is persisted in the approval record. Every guardian decision produces a `GuardianResolved` event linking to the approval record where the full input/output transcript is stored. A team can audit a week of guardian decisions by querying `GuardianResolved` events and joining to approval records for the full context.

### Why configurable and not hardcoded

Per user correction: cairn does not lock in on any provider. The guardian's model is configured per project with no default value; the operator sets it explicitly when enabling guardian mode. If no guardian model is configured, the guardian resolver is simply not in the chain and every approval goes to a human.

### Integration with RFC 019

RFC 019 defines the unified decision layer that evaluates policy, budget, guardrails, and approvals as one atomic flow. The guardian resolver is a plug point **inside** that flow — when the decision layer determines "human input is needed", it consults the resolver chain. A guardian-approved decision is cached per RFC 019's decision cache so repeated identical requests do not re-run the guardian.

### Why not a separate "AI reviewer" tool

Some systems model this as a tool the agent can call (`request_approval(reason)`). That's backwards — the agent should not be in charge of when to ask. The orchestrator's policy layer decides when an approval is needed, and the resolver chain decides who (or what) answers. The agent is not aware of the resolver; it just sees "my last tool call is waiting for approval" and pauses.

## Enhancement 3: Inline Context Compaction

### The gap

A run that spans many turns accumulates history. Even modest tasks can exceed a model's context window when tool outputs are large or the conversation is long. Today, cairn has no mitigation — the model eventually fails with a context-limit error and the run dies.

### The shape

Add a compaction pass to the gather phase:

```rust
// In cairn-orchestrator::gather
pub async fn assemble_context(
    &self,
    ctx: &OrchestrationContext,
    run_state: &RunState,
) -> Result<AssembledContext, GatherError> {
    let mut messages = load_message_history(ctx).await?;

    let token_count = count_tokens(&messages, ctx.model);
    let threshold = ctx.compaction_threshold
        .unwrap_or(compute_default_threshold(ctx.model));

    if token_count > threshold {
        messages = self.compact(messages, ctx).await?;
        emit_event(ContextCompacted {
            run_id: ctx.run_id.clone(),
            before_tokens: token_count,
            after_tokens: count_tokens(&messages, ctx.model),
            strategy: "inline_summarization",
        }).await?;
    }

    // ...continue assembling context as today
}
```

Compaction runs a summarization turn through the same provider the main loop uses (so it is provider-agnostic). The summarization prompt asks the model to produce a concise summary of the conversation so far, preserving:

- the original goal
- all decisions the agent has made
- key findings and facts
- the list of tools called and their results (names only, not full outputs)
- any open questions or unresolved state

The resulting summary replaces the full history as a single "summary message" in the context, anchored above the most recent user message so the agent sees it as fresh context.

### Configuration

```toml
[orchestrator.compaction]
enabled = true
threshold_pct = 70   # trigger at 70% of the model's context window
min_messages   = 10  # don't compact fewer than 10 messages
keep_last      = 4   # always keep the most recent N messages verbatim
summary_token_budget = 2000
```

### Why not the model's native compaction endpoint

Codex CLI uses OpenAI's `/responses/compact` endpoint, which is OpenAI-specific. Cairn's inline approach works with any provider that supports text completion — which is every provider. No lock-in.

### Observability

Every compaction produces a `ContextCompacted` event with before/after token counts. The operator can see in the run timeline exactly when compactions happened and how much context was shed. This is important for debugging why an agent suddenly "forgot" something earlier in the conversation — the event log shows when.

### Examples

- **Long research run**: an analyst agent reads 50 papers, compaction kicks in after paper 30, the summary retains the key findings so the agent can continue synthesizing without blowing the context
- **Debugging session**: an ops agent runs 80 diagnostic commands; compaction retains the structure of what was tried and the relevant outputs
- **Multi-step writing**: a writing agent drafts a long document over many revisions; compaction summarizes the earlier drafts so the agent can work on the latest version without losing sight of the outline

## Enhancement 4: Plugin Visibility at Prompt Assembly

### The gap

RFC 015 establishes per-project plugin enablement and a `VisibilityContext` type. But the orchestrator's prompt builder today calls `BuiltinToolRegistry::prompt_tools()` which returns every registered tool in `Core + Registered` tiers, regardless of which plugins are enabled for the project.

### The shape

Extend `BuiltinToolRegistry` with a visibility-aware variant:

```rust
impl BuiltinToolRegistry {
    /// Existing tier-based filtering (unchanged).
    pub fn prompt_tools(&self) -> Vec<BuiltinToolDescriptor> { /* ... */ }

    /// New: tier-based filtering PLUS plugin visibility from RFC 015.
    pub fn prompt_tools_in_context(
        &self,
        ctx: &VisibilityContext,
    ) -> Vec<BuiltinToolDescriptor> {
        self.prompt_tools()
            .into_iter()
            .filter(|desc| self.is_visible_in_context(desc, ctx))
            .collect()
    }
}
```

The orchestrator builds a `VisibilityContext` at the start of every run (or when the run's context changes):

1. Load the run's `(tenant, workspace, project)` scope
2. Query the marketplace layer for plugins enabled in that project
3. For each enabled plugin, retrieve the `tool_allowlist` if any
4. Construct `VisibilityContext { project, enabled_plugins, allowlisted_tools }`
5. Cache for the run's duration (invalidated on plugin enable/disable events)

Built-in cairn tools (file_read, grep_search, shell_exec, memory_search, etc.) are **always visible** regardless of plugin enablement — they are product-core, not plugin-provided. Only tools that originate from a plugin capability are subject to this filter.

### `tool_search` also respects visibility

The `tool_search` tool discovers `Deferred`-tier tools when the agent explicitly asks. Per the RFC 015 Q2 decision, `tool_search` also filters by `VisibilityContext`: a deferred plugin tool from a plugin not enabled in the project is not returned. Agents cannot discover tools from plugins they are not allowed to use.

### Effect on prompt size

For a project with only the GitHub plugin enabled, a Direct/Execute-mode run sees ~30 built-in tools (see §"v1 Tool Registration Pre-requisite") + 19 GitHub tools = ~49 tools in the prompt. A Plan-mode run in the same project sees ~23 built-in tools (Observational + Internal only) + 12 GitHub read-only tools = ~35 tools. For a project with Slack + Linear enabled, it sees the built-in set + each plugin's tools. No project sees tools from plugins it does not use, which both reduces prompt noise and prevents cross-plugin context contamination.

## Enhancement 5: Tool Output Token Budget

### The gap

A single `shell_exec` or `github.get_pull_request_diff` call can produce tens of thousands of tokens of output. Appending that to the message history without bound eats the context window.

### The shape

Add a `tool_output_token_limit` configuration per run (with a sensible default, e.g. 2000 tokens). After a tool call completes, the orchestrator truncates the output to this limit before appending to context. Truncation preserves head and tail (first N/2, last N/2, with a clear `... [truncated: X tokens omitted] ...` marker in between).

```rust
// In cairn-orchestrator::execute
let tool_result = tool.invoke(args, ctx).await?;
let truncated = truncate_for_context(
    &tool_result,
    ctx.tool_output_token_limit,
    ctx.model,
);
append_to_history(ToolCallResult {
    tool_name: tool.name(),
    args,
    result: truncated,
    original_size_tokens: count_tokens(&tool_result, ctx.model),
    was_truncated: truncated.len() < tool_result.len(),
});
```

Tools that know their output will be large can proactively paginate (returning partial results with a continuation token) rather than rely on truncation. `file_read` in particular should support an explicit `max_lines` parameter for this.

The full untruncated output is still persisted in the tool invocation record (for operator debugging via the dashboard) — only the context-facing copy is truncated.

## v1 Tool Registration Pre-requisite

**Critical implementation dependency**: the running orchestrate registry at `crates/cairn-app/src/lib.rs` currently wires only **6 tools** into the prompt: `memory_search`, `memory_store`, `web_fetch`, `shell_exec`, `notify_operator`, `tool_search`. Approximately 25 additional tool handler files exist in `crates/cairn-tools/src/builtins/` but are **not registered** in the orchestrate registry. Plan mode's value depends on a rich set of `Observational` and `Internal` tools being available; Execute/Direct mode benefits from the full set.

Before Plan mode ships, the following tools **must** be wired into the orchestrate registry, classified by `ToolEffect`:

**Observational** (always visible in Plan mode — read-only observation):

| Tool | Source file |
|---|---|
| `memory_search` | builtins/memory_search.rs (already wired) |
| `web_fetch` | builtins/web_fetch.rs (already wired) |
| `tool_search` | builtins/tool_search.rs (already wired, Deferred tier) |
| `grep_search` | builtins/grep_search.rs |
| `file_read` | builtins/file_read.rs |
| `glob_find` | builtins/glob_find.rs |
| `json_extract` | builtins/json_extract.rs |
| `calculate` | builtins/calculate.rs |
| `graph_query` | builtins/graph_query.rs |
| `get_run` | builtins/get_run.rs |
| `get_task` | builtins/get_task.rs |
| `get_approvals` | builtins/get_approvals.rs |
| `list_runs` | builtins/list_runs.rs |
| `search_events` | builtins/search_events.rs |
| `wait_for_task` | builtins/wait_for_task.rs |

**Internal** (visible in Plan mode — writes to cairn-owned state only):

| Tool | Source file |
|---|---|
| `memory_store` | builtins/memory_store.rs (already wired) |
| `scratch_pad` | builtins/scratch_pad.rs |
| `file_write` | builtins/file_write.rs (sandbox-scoped) |
| `create_task` | builtins/create_task.rs |
| `cancel_task` | builtins/cancel_task.rs |
| `summarize_text` | builtins/summarize_text.rs |
| `delete_memory` | builtins/delete_memory.rs |
| `update_memory` | builtins/update_memory.rs |

**External** (Execute + Direct mode only — touches systems outside cairn):

| Tool | Source file |
|---|---|
| `shell_exec` | builtins/shell_exec.rs (already wired) |
| `notify_operator` | builtins/notify_operator.rs (already wired) |
| `http_request` | builtins/http_request.rs |
| `git_operations` | builtins/git_operations.rs |
| `resolve_approval` | builtins/resolve_approval.rs |
| `schedule_task` | builtins/schedule_task.rs |
| `eval_score` | builtins/eval_score.rs |

**Total**: ~15 Observational + ~8 Internal + ~7 External = **~30 built-in tools** once fully wired. Plan mode sees ~23 tools (Observational + Internal). Execute/Direct sees all ~30 plus plugin tools gated by `VisibilityContext`.

The registration task is implementation work, not design work — the handler files are written; they need a line in `build_tool_registry()`. But it is a **blocking pre-requisite** for Plan mode to deliver its stated value: an agent in Plan mode with only 6 tools (4 of which are Internal/External) has almost no Observational surface to work with.

## Plan Review Operator Contract

The plan review flow has three operator actions. Each maps to an HTTP route:

```
POST /v1/runs/:plan_run_id/approve
     Body: { "reviewer_comments": "optional string" }
     → emits PlanApproved { plan_run_id, approved_by, ... }
     → returns 200 with { "next_step": "create_execute_run" }
     The operator then creates the execute run as a SEPARATE action
     (per Open Question 5: separate click, no auto-creation).

POST /v1/runs/:plan_run_id/reject
     Body: { "reason": "string" }
     → emits PlanRejected { plan_run_id, rejected_by, reason, ... }
     → returns 200

POST /v1/runs/:plan_run_id/revise
     Body: { "reviewer_comments": "string" }
     → creates a NEW Plan-mode run with:
       - the original plan's system prompt + task goal
       - the reviewer_comments prepended as additional context
       - parent_plan_run_id set to the original plan's run_id
     → emits PlanRevisionRequested { original_plan_run_id, new_plan_run_id, ... }
     → returns 201 with the new plan run ID

POST /v1/runs
     Body: { "mode": { "type": "execute", "plan_run_id": "..." }, ... }
     → creates an Execute-mode run seeded with the approved plan markdown
     → the plan markdown is included in the execute run's initial context
```

The `PlanRevisionRequested` event is declared in the `OrchestratorEvent` enum alongside `PlanProposed`, `PlanApproved`, and `PlanRejected`.

## Dropped From Scope

The following were considered and dropped:

- **Cross-session memory pipeline auto-ingestion**: `cairn-memory` provides the retrieval pipeline exposed via `memory_search` and `memory_store`. **Automatic ingestion of agent-generated artifacts** (run summaries, plan artifacts, compacted history, sandbox diffs) into cairn-memory is **out of scope for v1**. Operators who want run knowledge to accumulate in cairn-memory must either (a) have the agent call `memory_store` explicitly during the run, or (b) invoke `POST /v1/memory/ingest` on selected run artifacts after the run completes. A future `PostRunIngestHook` (triggered on `RunCompleted` events, configurable per project, default off) is a small engineering task once the run artifact format stabilizes, but v1 does not commit to it.
- **Compacted-away message retrieval**: compacted messages remain in the event log for operator inspection, but the **agent** cannot retrieve them. If the agent compacted away detail it later needs, there is no tool to recover it within the run — `memory_search` queries cairn-memory (which the compacted content was never ingested into), not the run's own event history. Future enhancements may add (a) auto-ingestion of compacted content into cairn-memory, or (b) a `search_run_history` Observational tool that queries the run's event log. V1 ships neither — the compaction config (`threshold_pct`, `keep_last`, `summary_token_budget`) should be tuned to retain enough recent context that compaction rarely drops critical detail.
- **Dynamic per-invocation `shell_exec` classification (execpolicy)**: an execpolicy rule engine that evaluates `shell_exec` per-invocation against allow/deny prefix rules (inspired by Codex CLI's pattern) was explored but deferred. The v1 `ToolEffect` enum is per-tool, not per-invocation, so `shell_exec` is always `External`. Execpolicy requires a per-invocation evaluation path and is future work.
- **`apply_patch` tool**: coding-specific; out of scope for a control plane for teams using AI
- **LSP diagnostic feedback loop**: coding-specific
- **ACI-style `file_read` pagination**: useful but marginal; `file_read` already has tier controls and can be enhanced in a minor version without an RFC
- **Learned tool-call auto-approvals**: covered by RFC 019's decision cache, not this RFC
- **Session / run forking**: deferred; event log supports it in principle but the UI and semantics need their own design

## Events

New event variants (all flow through the existing runtime event log):

```rust
pub enum OrchestratorEvent {
    PlanProposed {
        run_id: RunId,
        plan_markdown: String,
        produced_at: u64,
    },
    PlanApproved {
        plan_run_id: RunId,
        execute_run_id: Option<RunId>,  // filled in when execute run is created
        approved_by: OperatorId,
        approved_at: u64,
    },
    PlanRejected {
        plan_run_id: RunId,
        rejected_by: OperatorId,
        reason: String,
        rejected_at: u64,
    },
    PlanRevisionRequested {
        original_plan_run_id: RunId,
        new_plan_run_id: RunId,
        reviewer_comments: String,
        requested_by: OperatorId,
        requested_at: u64,
    },
    ContextCompacted {
        run_id: RunId,
        before_tokens: u32,
        after_tokens: u32,
        strategy: String,   // "inline_summarization"
        compacted_at: u64,
    },
    GuardianResolved {
        approval_id: ApprovalId,
        run_id: RunId,
        model_id: String,
        outcome: ApprovalOutcome,
        risk_level: RiskLevel,
        rationale: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
        resolved_at: u64,
    },
    GuardianFellThrough {
        approval_id: ApprovalId,
        run_id: RunId,
        reason: String,   // "parse_failed" | "timeout" | "risk_ceiling" | "error"
        fell_through_at: u64,
    },
    ToolOutputTruncated {
        run_id: RunId,
        tool_invocation_id: ToolInvocationId,
        original_tokens: u32,
        truncated_tokens: u32,
        truncated_at: u64,
    },
}
```

## Non-Goals

- rebuilding the memory system
- provider lock-in (all enhancements work with any provider cairn is configured for)
- changing the run state machine (RFC 005) beyond adding `RunMode` metadata
- replacing the existing approval primitive (resolvers plug into the existing `ApprovalService`)
- introducing a new top-level entity (modes are metadata, guardian decisions are resolver output, compaction is a gather-phase pass)
- coding-specific features

## Open Questions

1. **Resolved**: `shell_exec` classification. `shell_exec` is **always `External`** in v1. Even inside a sandbox, a shell command can escape via network or long-running subprocesses. Teams that need read-only access should use dedicated `Observational` tools (`grep_search`, `file_read`, `glob_find`). Dynamic per-invocation classification via execpolicy is deferred to future work (see §"Dropped From Scope"). (No further discussion needed.)

2. **Resolved**: Plan-mode run billing. Plan-mode runs draw from a **separate exploration budget**, distinct from the project's execute-run budget. The exploration budget defaults to **10% of the project's execute-run budget** unless explicitly overridden. This is a default, not a hard cap — operators can configure the ratio per project. (No further discussion needed; baked into the Resolved Decisions at the top of this RFC.)

3. **Resolved**: Guardian model configuration location. **Per-project with tenant-level default inheritance.** Tenant admin sets a default Guardian model; projects can override or inherit. If neither is set, the Guardian resolver is not in the chain and approvals fall through to the human resolver. No hardcoded default model. (No further discussion needed; baked into the Resolved Decisions at the top of this RFC.)

4. **NEEDS DISCUSSION: Compaction frequency cap.** Should the orchestrator prevent runaway compaction (e.g. compacting every turn because the threshold is set too low)? Proposal: at most once per N turns (default 5), with a `CompactionThrottled` event if the threshold is hit during the cooldown.

5. **NEEDS DISCUSSION: Plan approval creating the execute run.** When an operator approves a plan, does cairn automatically create the execute run, or does the operator have to click "Execute" separately? Proposal: separate click. Auto-creation risks surprising operators who approve a plan without intending to trigger an execute.

6. **NEEDS DISCUSSION: Visibility invalidation on plugin disable.** If a plugin is disabled mid-run (RFC 015 allows this), what happens to the cached `VisibilityContext`? Proposal: the run continues with the cached context; new runs created after the disable event see the updated context. The disabled plugin's next tool invocation from the still-running run fails with `PluginDisabledMidRun`, which the agent can handle via the existing tool failure path.

7. **Resolved**: Plan-mode budget cap. Exploration budget defaults to 10% of the execute-run budget. See Q2 above. (No further discussion needed.)

## Decision

Proceed assuming:

- `RunMode` is new metadata on the run record with variants `Direct`, `Plan`, `Execute { plan_run_id }`
- tool side-effect classification is a new `ToolEffect` enum on `ToolHandler` with variants `Observational`, `Internal`, `External` (and plugin manifest field for plugin tools)
- `Plan` mode runs only see `Observational` and `Internal` tools; `External` tools are filtered out of the prompt so the agent cannot attempt to use them; the run produces a markdown plan artifact and terminates `Completed` with `outcome: plan_proposed`
- approval resolution is pluggable via `ApprovalResolver`; `HumanResolver` and `GuardianResolver` ship in v1; the chain is configurable per project
- guardian model is configurable per project with no default; if not configured, guardian is not in the chain
- inline context compaction runs through the main provider, is provider-agnostic, and emits `ContextCompacted` events for audit
- the orchestrator's prompt builder consumes `VisibilityContext` from RFC 015 so agents see only tools from enabled plugins
- `tool_output_token_limit` caps the tokens appended to context per tool call; full outputs remain in the tool invocation record for operator inspection
- `cairn-memory` is unchanged; no new memory pipeline is built. Automatic ingestion of run artifacts and compacted content is out of scope for v1 (see §"Dropped From Scope"). Agents accumulate knowledge in cairn-memory only through explicit `memory_store` calls.
- `shell_exec` is always `External` in v1; dynamic per-invocation classification via execpolicy is deferred to future work (see §"Dropped From Scope")
- the ~30 built-in tool handlers in `cairn-tools/src/builtins/` must be fully wired into the orchestrate registry before Plan mode ships (see §"v1 Tool Registration Pre-requisite" — this is a blocking implementation dependency, not a design change)
- open questions listed above must be resolved before implementation begins

## Integration Tests (Compliance Proof)

1. **Plan mode produces artifact**: a `Plan`-mode run terminates with a markdown plan in its artifacts; the run's outcome is `plan_proposed`; no mutating tool calls are in the event log for the run
2. **Plan mode excludes external tools**: during a `Plan` run, the prompt given to the LLM does not include tools classified as `External`; `tool_search` in a `Plan` run does not return external-effect tools; `Observational` and `Internal` tools remain fully available
3. **Plan approval flow**: an operator approves a plan via `POST /v1/runs/{plan_run_id}/approve` → `PlanApproved` event; operator creates execute run via `POST /v1/runs` with `mode: Execute, plan_run_id: ...` → the plan markdown is included in the execute run's initial context
4. **Guardian happy path**: an approval request in a project with guardian configured triggers a guardian call → the guardian returns `Approved` with a rationale → `GuardianResolved` event is emitted → the approval is resolved without human intervention
5. **Guardian fall-through**: a guardian call that returns `High` risk (above the configured ceiling) falls through to the human resolver → `GuardianFellThrough` event → approval remains pending for human
6. **Guardian timeout**: a guardian call that exceeds the timeout fails closed → no decision is recorded → the approval falls through to human
7. **Guardian caching**: a guardian decision with a TTL is cached per RFC 019's decision cache → a second identical request within the TTL is resolved from cache without spawning a new guardian call
8. **Inline compaction**: a run whose message history exceeds the threshold triggers compaction → `ContextCompacted` event shows before/after token counts → the run continues with the summarized history → the original messages remain in the event log for operator inspection
9. **Compaction throttle**: a run attempting to compact more often than the cooldown allows emits `CompactionThrottled` and continues without compacting
10. **Plugin visibility in prompt**: a run in project P1 with GitHub enabled sees the 19 GitHub tools in its prompt; a run in project P2 without GitHub does not; the difference is reflected in the LLM's system prompt
11. **`tool_search` respects visibility**: a run in project P2 calling `tool_search("issue")` does not return GitHub's `github.get_issue` because the plugin is not enabled for the project
12. **Tool output truncation**: a tool call whose output exceeds `tool_output_token_limit` is truncated in the message history → the full output is preserved in the tool invocation record → a `ToolOutputTruncated` event is emitted
