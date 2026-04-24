# Learning Guide: LLM Agent Tool-Call Approval and Permission Systems

**Generated**: 2026-04-23
**Sources**: 32 resources analyzed
**Depth**: deep
**Audience**: The fixer agent implementing cairn-rs BP-v2 (approval system redesign)

---

## Prerequisites

- Familiarity with the LLM agentic loop (plan → tool_use → tool_result → continue)
- Basic understanding of async Rust (tokio channels, oneshot, Arc<Mutex<...>>)
- Awareness that cairn-rs currently has a broken approval flow where operator approval discards the original proposal and triggers a fresh LLM call

## TL;DR

1. **Approval is a STATE on the tool call** — not a side-channel event. The tool call transitions: `Proposed → PendingApproval → Approved → Executing → Completed`. The LLM is NOT re-queried after approval; the original args execute directly.
2. **The proposal must be persisted** at the moment the LLM emits it, before the operator is asked. Losing the proposal (cairn's current bug) forces a new LLM call and breaks idempotency.
3. **Resume flow**: operator approves → original `(tool_name, tool_args, call_id)` tuple is retrieved → tool executes → result feeds into conversation as `tool_result` → LLM sees only the result and continues planning.
4. **Allow-once vs allow-always** are first-class concepts in every mature system. They differ only in whether the approval record is scoped to one `call_id` or to a `(tool_name, pattern)` for the session lifetime.
5. **The hook/middleware layer** (Claude Code `PermissionRequest`, OpenCode `permission.ask`, harness-core `PermissionHook`) is separate from the rule-match layer. Rules answer statically; hooks/ask-flows answer dynamically with human input.

---

## Core Concepts

### 1. The Agentic Loop and Where Approval Fits

Every LLM tool-calling system shares this canonical loop:

```
while stop_reason == "tool_use":
    tool_calls = parse(llm_response)          # LLM decides WHAT to call
    for tc in tool_calls:
        approved = check_permission(tc)       # GATE: approve or deny
        if approved:
            result = execute(tc)              # Execute with ORIGINAL args
        else:
            result = format_rejection(tc)
        conversation.append(tool_result(tc.id, result))
    llm_response = llm(conversation)          # LLM sees results and continues
```

The critical invariant: **the `tc` (tool_name + tool_args + call_id) used in `execute()` is the same object that came out of `parse()`**. The LLM never re-decides; the human only gates execution.

### 2. Tool Call Identity

Every mature system mints a stable `call_id` at the moment the LLM emits the tool call:

- Anthropic: `tool_use.id` (e.g. `toolu_01A09q90qw90lq917835lq9`)
- OpenAI: `tool_calls[i].id`
- LangGraph: interrupt ID inside `Interrupt.id`
- OpenCode/harness-core: `tool_call_id` / `PermissionQuery`

This ID is the identity key across the entire approval flow. It links: proposal storage → approval decision → execution → result attribution.

### 3. Approval as a State Machine

```
                  ┌────────────────────┐
                  │      Proposed      │  ← LLM emits tool_use block
                  └────────────────────┘
                           │
               Policy/rule evaluation
                    ┌──────┴──────┐
                    │             │
                 Allow          Ask
                    │             │
                    │    ┌────────────────┐
                    │    │ PendingApproval │  ← persisted, operator notified
                    │    └────────────────┘
                    │         │        │
                    │      Approve   Reject
                    │         │        │
                    └─────────┤        │
                              │        │
                    ┌─────────▼─┐  ┌───▼──────────┐
                    │ Executing  │  │   Rejected   │
                    └─────────┬─┘  └──────────────┘
                              │
                    ┌─────────▼─┐
                    │ Completed  │
                    └───────────┘
```

The state machine has exactly **one branch point** (Ask) where human input is required. At that branch, execution is **suspended**, not cancelled. The proposal is persisted in full.

---

## Per-System Analysis

### System 1: Claude Code / Anthropic Agent SDK

**Sources**:
- https://code.claude.com/docs/en/permissions
- https://code.claude.com/docs/en/hooks-guide
- https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/how-tool-use-works

#### The Agentic Loop

Claude Code implements the standard `stop_reason: "tool_use"` loop. The model emits `tool_use` blocks; the client-side harness executes them and returns `tool_result` blocks. The model is never re-queried to re-decide on an already-emitted tool call.

#### Permission Architecture: Two-Layer Stack

**Layer 1 — Static Rules** (evaluated synchronously before any prompt):
```json
{
  "permissions": {
    "deny":  ["Bash(curl *)", "Read(./.env)"],
    "ask":   ["Bash(git push *)"],
    "allow": ["Bash(npm run test *)", "Read(src/**)"]
  }
}
```
Rules are evaluated **deny → ask → allow**. First match wins. Fail-closed: unmatched defaults to `ask`.

**Layer 2 — Dynamic Hooks** (`PreToolUse`, `PermissionRequest`):
Hooks run after rule evaluation. A `PreToolUse` hook receives the full tool call on stdin:
```json
{
  "session_id": "abc123",
  "tool_name": "Bash",
  "tool_input": { "command": "git push origin main" }
}
```
Hook exit codes and JSON outputs:
- Exit 0: allow (proceeds to permission rules)
- Exit 2: block with stderr reason
- JSON `{"permissionDecision": "allow" | "deny" | "ask"}`: structured control
- JSON `{"permissionDecision": "defer"}`: headless mode — suspend and resume externally

The **`defer` decision** is the critical one for integration: it preserves the in-flight tool call so an external process (Agent SDK wrapper) can collect approval and resume. This is how Claude Code bridges to multi-turn human-in-the-loop.

#### Approval UX

When a tool call hits an `ask` rule:
1. Claude Code surfaces the full `tool_name + args` to the operator UI
2. Operator chooses: **Allow once** or **Allow always** (saved to settings)
3. On "Allow always": rule is appended to `permissions.allow` in the settings file
4. On "Allow once": approval applies only to the current `call_id`

**Key design decisions**:
- `approvalMode: bypassPermissions` skips all prompts (CI/automation use case)
- `approvalMode: acceptEdits` auto-approves file edits but still prompts for shell commands
- Hooks are **additive** — they cannot override managed deny rules
- The `PermissionRequest` hook fires at the moment the dialog would appear, not before rule evaluation
- `PermissionDenied` hook fires when auto-mode classifier denies; can return `{retry: true}` to tell model it may retry

#### Session vs One-Time Grants

| Grant Type | Mechanism | Scope |
|---|---|---|
| Allow-once | In-memory, tied to call_id | One execution |
| Allow-session | In-memory, pattern match | Until session end |
| Allow-always | Written to settings.json | Permanent |

#### Hook Precedence (most restrictive wins)

When multiple `PreToolUse` hooks match, Claude Code picks the most restrictive answer:
- Any hook returning `deny` cancels the call regardless of others
- Any hook returning `ask` forces the prompt regardless of `allow` from others
- `additionalContext` text is merged from all hooks

---

### System 2: OpenAI Agents SDK (Python)

**Sources**:
- https://openai.github.io/openai-agents-python/human_in_the_loop/
- https://openai.github.io/openai-agents-python/ref/run_state/
- https://openai.github.io/openai-agents-python/ref/result/

#### Architecture

The OpenAI Agents SDK uses an **interrupt-based** model. Execution pauses mid-loop when a tool requires approval. The paused state is fully serializable:

```python
# Step 1: Run until interrupted
result = await Runner.run(agent, "What is the temperature in Oakland?")

# Step 2: Inspect pending approvals
while result.interruptions:
    state = result.to_state()           # Convert to mutable RunState
    state_json = state.to_string()      # Serialize (can persist to disk/DB)
    
    # Operator reviews: interruption.name, interruption.arguments
    for interruption in result.interruptions:
        if operator_approves(interruption):
            state.approve(interruption, always_approve=False)
        else:
            state.reject(interruption, rejection_message="Not authorized")
    
    # Step 3: Resume — NO new LLM call for the approved tool
    result = await Runner.run(agent, state)
```

#### State Preservation

`RunState` serialization captures exactly what is needed to resume:
```json
{
  "_model_responses": ["...cached LLM outputs, NOT re-queried on resume..."],
  "_last_processed_response": {"...the response containing tool calls..."},
  "_current_step": {
    "type": "interruption",
    "tool_approval_items": [
      {
        "call_id": "call_abc123",
        "tool_name": "get_temperature",
        "arguments": {"city": "Oakland"},
        "agent_name": "Weather assistant",
        "raw_item": {"...original LLM tool_use block verbatim..."}
      }
    ]
  }
}
```

The runner, upon resumption, detects `input is RunState` → skips LLM call → retrieves `_last_processed_response` → runs `resolve_interrupted_turn()` → executes the approved tools with **original args from `raw_item`**.

#### Allow-Once vs Always-Approve

```python
state.approve(interruption, always_approve=False)  # Allow-once: this call_id only
state.approve(interruption, always_approve=True)   # Always: all future calls to this tool

# Always-approve persists across to_string()/from_string() boundaries
# (i.e., survives process restart if state is externalized)
```

**Key insight**: `always_approve=True` caches the approval keyed on `tool_name`, not on `call_id`. Future calls to the same tool bypass the approval check entirely.

#### What Happens to a Rejected Tool?

A custom `rejection_message` is sent to the model as a `tool_result` with error content. The model then decides what to do (retry with different args, give up, ask the user). The LLM IS involved after rejection — but only sees the rejection as a normal tool result, not a special control flow.

---

### System 3: LangGraph (LangChain)

**Sources**:
- https://raw.githubusercontent.com/langchain-ai/langgraph/main/libs/langgraph/langgraph/types.py
- https://raw.githubusercontent.com/langchain-ai/langgraph/main/libs/checkpoint/langgraph/checkpoint/base/__init__.py

#### The `interrupt()` Primitive

LangGraph's human-in-the-loop is built on a single primitive:

```python
def interrupt(value: Any) -> Any:
    """
    First call: raises GraphInterrupt (pauses the graph).
    After Command(resume=value): returns the resume value.
    """
```

A tool-approval node uses it like this:
```python
def approval_node(state):
    tool_call = state["pending_tool_call"]
    # First execution: raises GraphInterrupt, graph pauses
    # Second execution (after resume): returns operator's decision
    decision = interrupt({
        "tool_name": tool_call.name,
        "tool_args": tool_call.args,
        "call_id": tool_call.id,
    })
    if decision == "approved":
        return {"approved_calls": [tool_call]}
    else:
        return {"rejected_calls": [tool_call]}
```

The `Interrupt` type:
```python
@dataclass
class Interrupt:
    value: Any   # The interrupt payload (tool call details)
    id: str      # Unique ID for resumption targeting
```

The `Command` type for resumption:
```python
@dataclass
class Command:
    graph: str | None = None
    update: Any | None = None
    resume: dict[str, Any] | Any | None = None  # Resume value(s)
    goto: ... = ()
```

#### Checkpoint-Based Pause/Resume

**Requirement**: a checkpointer must be configured. LangGraph writes graph state to the checkpointer at every node boundary.

```
1. approval_node runs → interrupt() raises GraphInterrupt
2. LangGraph catches GraphInterrupt → writes checkpoint (full state snapshot)
3. Returns to caller with stop_reason = "interrupted"

[... operator provides decision ...]

4. Caller invokes graph.invoke(Command(resume=decision), config=thread_config)
5. LangGraph loads checkpoint → re-enters approval_node
6. interrupt() returns the decision value (not raises)
7. approval_node continues from where it was logically (but physically re-executes)
```

**Critical detail**: The interrupted node is **re-executed from the top**, not resumed mid-function. `interrupt()` acts like a yield point — the function starts again, calls `interrupt()` again, but this time the framework returns the saved resume value instead of raising.

This means approval nodes must be **idempotent up to the interrupt call**. Side effects before `interrupt()` in the node body will run twice.

#### No LLM Re-Query

The tool node that actually executes the tool is downstream of the approval node in the graph. After the approval_node returns `approved_calls`, the tool_node executes them with original args. The LLM is not involved until tool_results are fed back in the next LLM call node.

#### Checkpoint Write Types

```python
WRITES_IDX_MAP = {
    "ERROR": -1,
    "SCHEDULED": -2,
    "INTERRUPT": -3,    # written when interrupt() is called
    "RESUME": -4,       # written when Command(resume=...) is received
}
```

The `INTERRUPT` and `RESUME` write types mark the node as suspended vs being resumed. The original tool call data lives in the graph state channels, which are preserved in the checkpoint.

---

### System 4: OpenCode (SST)

**Sources**:
- https://raw.githubusercontent.com/sst/opencode/dev/packages/opencode/src/permission/index.ts
- https://raw.githubusercontent.com/sst/opencode/dev/packages/opencode/src/session/message.ts

#### Effect-Based Suspension

OpenCode uses the [Effect-TS](https://effect.website/) library for async control flow. The permission `ask()` function creates a `Deferred` — an Effect primitive that suspends the fiber until resolved:

```typescript
// Inside permission/index.ts
const ask = (info: PermissionRequest): Effect.Effect<void, RejectedError> => 
  Effect.gen(function*() {
    const deferred = yield* Deferred.make<void, RejectedError | CorrectedError>()
    // Publish event so UI can show approval dialog
    yield* Event.publish(Event.Asked({ ...info, deferred }))
    // Suspend: fiber waits here until deferred is resolved
    yield* Deferred.await(deferred)
  })
```

The `reply()` function resolves the deferred from the UI thread:
```typescript
const reply = (id: string, action: "allow" | "deny" | "once") => {
  const pending = pendingRequests.get(id)
  if (!pending) return
  
  if (action === "deny") {
    Deferred.unsafeDone(pending.deferred, Exit.fail(new RejectedError()))
  } else {
    if (action === "allow") {
      // Persist to approved ruleset — future identical requests auto-pass
      approved.push({ permission: pending.info.permission, pattern, action: "allow" })
    }
    // "once" — succeeds deferred but does NOT persist to approved
    Deferred.unsafeDone(pending.deferred, Exit.succeed(undefined))
  }
}
```

#### Allow vs AllowOnce

| Decision | Mechanism | Persistence |
|---|---|---|
| `"allow"` | Deferred resolves + rule appended to `approved[]` | Session-lifetime |
| `"once"` | Deferred resolves, nothing persisted | Single call only |
| `"deny"` | Deferred fails with RejectedError | Single call only |

`RejectedError` propagates up the Effect chain, which the tool execution layer catches and converts to a tool_result error.

#### Tool Call Event Shape

OpenCode's message schema uses a discriminated union for tool call states:
```typescript
type ToolCall        = { state: "call";         toolCallId, toolName, args, step? }
type ToolPartialCall = { state: "partial-call"; toolCallId, toolName, args, step? }
type ToolResult      = { state: "result";       toolCallId, toolName, args, step?, result }
```

There is no `"pending-approval"` state in the event schema — approval happens inline before the `"call"` event is emitted to the conversation log. The approval gate is transparent to the message history.

---

### System 5: OpenAI Codex (Rust)

**Sources**:
- https://raw.githubusercontent.com/openai/codex/main/codex-rs/core/src/exec_policy.rs
- https://raw.githubusercontent.com/openai/codex/main/codex-rs/core/src/lib.rs

#### Policy Decision Types

```rust
enum Decision { Allow, Prompt, Forbidden }

enum ExecApprovalRequirement {
    Skip { bypass_sandbox: bool },
    NeedsApproval {
        reason: String,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    Forbidden { reason: String },
}
```

Codex evaluates commands against a policy file before any execution. The policy uses prefix-matching with a rule precedence hierarchy.

#### Policy Amendments

When a user approves a `NeedsApproval` command, Codex generates an `ExecPolicyAmendment` that is persisted:
```rust
// On user approval:
append_amendment_and_update(amendment).await?;
// Updates both: on-disk rules file AND in-memory policy state
// Protected by semaphore to prevent concurrent amendment races
```

This implements the "allow-always" semantic: the next time the same command pattern is seen, the policy matches it directly as `Allow` without prompting.

#### The AskForApproval Modes

```rust
enum AskForApproval {
    Never,         // Fully autonomous — no prompts ever
    OnFailure,     // Prompt only when sandbox blocks (retroactive escalation)
    OnRequest,     // User configured per-tool
    UnlessTrusted, // Prompt for unrecognized commands only
    Granular,      // Fine-grained: allows_rules_approval() + allows_sandbox_approval()
}
```

`OnFailure` is notable: commands execute in the sandbox first, and approval is only escalated if the sandbox blocks them. This inverts the normal "ask-before-execute" flow — optimistic execution with fallback to HITL.

---

### System 6: Cline (VS Code Extension)

**Sources**:
- https://github.com/cline/cline — `src/core/assistant-message/index.ts`
- GitHub repository structure and interface definitions

#### In-Flight Approval Pattern

Cline implements approval inline in the streaming response handler. When the model emits a tool use block:

1. The tool call is surfaced to the VS Code webview with full `tool_name + args`
2. The streaming loop pauses by `await`-ing a promise that only resolves when the user responds
3. On approval: the tool executes with original args
4. On rejection: a synthetic `tool_result` with error is constructed and appended to conversation

The `requires_approval` flag on the `ToolUse` interface marks which tools need the pause. Read-only tools (grep, read_file) bypass the approval UI; destructive tools (write_file, execute_command) always prompt.

**UX affordance**: Cline shows the **full proposed command or file diff** in the approval dialog. Operators see exactly what will execute before approving. There is no edit-before-approve capability in the base version — it is approve-or-reject only.

#### `isNativeToolCall` Flag

For tool calls that come from native Claude tool_use blocks (as opposed to XML-embedded tool calls in older Claude models), Cline tracks them with a `call_id`. This ensures that the `tool_result` appended after approval correctly references the original `tool_use` block in the message history.

---

### System 7: AutoGen (Microsoft)

**Sources**:
- https://raw.githubusercontent.com/microsoft/autogen/main/python/packages/autogen-agentchat/src/autogen_agentchat/agents/_user_proxy_agent.py
- https://microsoft.github.io/autogen/0.2/docs/topics/task_decomposition

#### Architecture: Agent-Level, Not Tool-Level

AutoGen's approval model is coarser-grained than tool-level approval. The `UserProxyAgent` is a full participant in the conversation whose "tool" is human judgment:

```python
user_proxy = UserProxyAgent(
    name="human_proxy",
    human_input_mode="ALWAYS",    # prompt before every action
    # "NEVER" = fully autonomous
    # "TERMINATE" = only prompt on termination condition
    code_execution_config={"executor": LocalCommandLineCodeExecutor(work_dir=".")}
)
```

When `human_input_mode="ALWAYS"`:
1. AssistantAgent generates a response (possibly with code/commands)
2. UserProxyAgent receives the message, emits `UserInputRequestedEvent`
3. Human provides approval via `_get_input()` (sync or async)
4. UserProxyAgent constructs a `TextMessage` containing the human's response
5. The conversation continues

**Critical difference from tool-level approval**: AutoGen approval happens before the human proxy executes code, but the approval is granted via the conversation turn, not by examining a structured tool call. The human sees the agent's message (which may contain code) and types their response.

#### Code Execution Approval

For code blocks, `code_execution_config` + `human_input_mode="ALWAYS"` creates a two-step flow:
1. Agent proposes code
2. UserProxy pauses, human reviews, types "yes" or provides corrections
3. UserProxy executes code only if response isn't a termination signal

This is structurally different from tool-level approval: the agent's output is natural language with embedded code, not a structured tool_call. The approval is by human message response, not by an approve/reject API.

---

### System 8: CrewAI

**Sources**:
- https://docs.crewai.com/concepts/tasks

#### `human_input` Task Flag

CrewAI's approval model is task-level:
```python
review_task = Task(
    description="Analyze the security vulnerability and propose a fix",
    human_input=True,     # pause after agent produces output, before marking done
    expected_output="A patch with explanation"
)
```

When `human_input=True`:
1. Agent completes the task and produces output
2. CrewAI pauses and shows output to the human
3. Human can accept or provide feedback
4. If feedback provided, agent re-runs the task with that feedback
5. Loop until human accepts

**Key design**: this is **post-execution approval** (review the result before continuing), not **pre-execution approval** (approve the action before it runs). It is closer to "code review" than "sudo prompt".

CrewAI's task-level model is appropriate for high-level orchestration but insufficient for tool-call-level safety gates. A task that calls `execute_command(rm -rf /)` would execute before the human sees it.

---

### System 9: OS-Level Sandboxing Primitives (Analogy)

**Sources**:
- pledge(2) man page — https://man.openbsd.org/pledge.2
- Capsicum design documentation

#### pledge() — Upfront Capability Declaration

```c
pledge("stdio rpath wpath inet", NULL);
// From this point: only stdio/read/write/network syscalls permitted
// All other syscalls → SIGABRT (process killed)
```

**Design lessons for agents**:

1. **Monotonic reduction**: once a capability is revoked, it cannot be re-granted. Agent permission scopes should narrow as execution progresses, never broaden. A tool that gained `file_write` at session start cannot gain `network_access` mid-session without a new approval.

2. **Fail-closed on violation**: pledge kills the process on unauthorized syscall. Agent systems should fail the tool call (not silently succeed) when a tool exceeds its approved capability scope.

3. **Upfront declaration vs lazy ask**: pledge requires declaring all capabilities at program start. The agent analog is listing all tools in the system prompt schema — the LLM can only call tools that were declared. This prevents capability escalation (LLM inventing tool calls for tools not in the schema).

4. **No ambient authority inside capability envelope**: once inside pledge(), the process cannot "sudo out" of it. Agent analogy: an approved tool should not be able to trigger unapproved tool calls on behalf of the LLM. Approval is per-call, not per-agent-turn.

#### Capsicum — Object Capabilities

```c
// Each file descriptor carries its own capability rights
cap_rights_t rights;
cap_rights_init(&rights, CAP_READ, CAP_SEEK);  // NOT CAP_WRITE
cap_rights_limit(fd, &rights);
// Now fd can only read, not write — regardless of OS-level file permissions
```

**Design lessons for agents**:

1. **Capability attached to the object (tool call), not the subject (agent)**. The tool call ID carries with it the approved scope. A `call_id` approved for `read_file("config.json")` cannot be re-used to `read_file("secrets.env")`.

2. **No ambient authority**: the agent running inside an approval scope cannot grant its own subagents broader permissions than it holds. Cairn should enforce this: a session approved for `shell_exec(read-only)` cannot spawn a sub-session approved for `shell_exec(arbitrary)`.

---

## Comparison Table

| System | Approval State Model | LLM Re-queried After Approval? | Proposal Persistence | Allow-Once Mechanism | Allow-Session Mechanism | Args Visible to Operator? |
|---|---|---|---|---|---|---|
| Claude Code | State on call_id in session memory | No — tool executes with original args | In-memory until session end | Per-call approval in dialog | `allow` rule written to settings | Yes — full args + command |
| OpenAI Agents SDK | `RunState._current_step` (serializable) | No — `_model_responses` cached, `resolve_interrupted_turn()` | Serializable to JSON/disk | `state.approve(item, always_approve=False)` | `state.approve(item, always_approve=True)` | Yes — `interruption.arguments` |
| LangGraph | Checkpoint (external store, e.g. Postgres) | No — node re-executes but `interrupt()` returns resume value | Checkpoint store (persistent) | Per-interrupt `Command(resume=...)` | Approval node can set session-level state | Yes — whatever was passed to `interrupt()` |
| OpenCode | Effect `Deferred` in pending map | No — fiber resumes from suspension point | In-memory during session | `"once"` reply | `"allow"` reply (rule persisted to approved[]) | Yes — `Permission.Request` info |
| Codex (Rust) | `ExecApprovalRequirement::NeedsApproval` | No — same command args used after approval | On-disk policy file (amendment) | Implicit (one-time prompt flow) | `ExecPolicyAmendment` persisted | Yes — command string shown |
| Cline | Inline promise in streaming loop | No — tool executes with original args | In-flight promise (not persisted) | Approve-in-dialog (per call_id) | Not directly supported | Yes — full diff/command shown |
| AutoGen | UserProxy conversation turn | Agent may re-plan based on human message | Not persisted (conversation history only) | Per-turn | `human_input_mode="NEVER"` override | Via natural language (not structured) |
| CrewAI | Task `human_input=True` flag | Task may re-run based on feedback | Not persisted (result in memory) | Post-execution review | Disable `human_input` | Yes — task output shown |
| harness-core | `PermissionDecision::Ask` on `PermissionHook` | No — hook future resolves, tool continues | Not specified (hook is async future) | `AllowOnce` variant | `Allow` variant | Via `PermissionQuery` fields |

---

## cairn-rs: Current State and Root Cause

### The Bug

When an operator approves a tool call, the orchestrator discards the approved proposal and makes a fresh LLM call instead of executing the approved tool.

This violates the fundamental invariant: **approval gates execution, it does not re-trigger planning**.

The root cause is almost certainly one of:

**A. Missing proposal persistence**: The `(tool_name, tool_args, call_id)` tuple from the LLM's original response is not stored anywhere. When the approval comes back, the system has no way to retrieve it — so it falls back to re-querying the LLM to get a new proposal.

**B. Approval handled as a conversation event**: The approval is being injected as a new user message into the conversation, causing the LLM to see "operator approved" as a prompt and generate a new tool_call response. This is architecturally wrong — approval should affect the execution loop, not the conversation history.

**C. State machine gap between phases**: The execute phase checks for approval but on receiving `Approved`, instead of looking up the persisted proposal and executing it, it re-enters the planning phase. The `Approved` state is not connected to the stored proposal.

### What Needs to Exist

1. **Proposal store**: A map from `call_id → ToolCallProposal { tool_name, tool_args, call_id, session_id, proposed_at }` that is written atomically when the LLM response is parsed and before the operator is notified.

2. **Approval record**: A map from `call_id → ApprovalDecision { decision: Allow|AllowOnce|Deny, operator_id, decided_at }` written when the operator responds.

3. **Resume flow**: After approval is recorded, the execute loop retrieves the stored proposal by `call_id`, constructs the execution context from it, runs the tool, records the result, and appends `tool_result(call_id, result)` to conversation history. The LLM is not called again until all pending tool results are appended.

---

## Synthesis: Design Patterns for cairn-rs BP-v2

### Pattern 1: State on the Tool Call (Recommended)

**Verdict**: Approval is a STATE on the tool call record, not a separate event stream.

Every mature system converges on this: the tool call has a lifecycle, and `PendingApproval` is one of its states. The event stream carries state-change notifications, but the source of truth is the tool call record itself.

```rust
// Recommended domain model:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallProposal {
    pub call_id: ToolCallId,        // stable, minted by LLM response parser
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub display_summary: Option<String>,  // human-readable "what will this do"
    pub proposed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalState {
    PendingApproval { notified_at: DateTime<Utc> },
    Approved { by: OperatorId, at: DateTime<Utc>, scope: ApprovalScope },
    Rejected { by: OperatorId, at: DateTime<Utc>, reason: Option<String> },
    Executing { started_at: DateTime<Utc> },
    Completed { result: ToolResult, completed_at: DateTime<Utc> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalScope {
    Once,      // this call_id only
    Session,   // all calls matching (session_id, tool_name) for the rest of this session
}
```

### Pattern 2: LLM is NOT Re-Queried After Approval

**Verdict**: The approved tool executes "blindly" with its original args. The LLM only sees the result.

This is the unanimous answer from all systems surveyed:
- OpenAI Agents SDK: `resolve_interrupted_turn()` uses cached `_last_processed_response`, not a new LLM call
- LangGraph: checkpoint restores exact state; `interrupt()` returns resume value; tool node runs downstream
- OpenCode: Effect fiber resumes from the `Deferred.await()` suspension point
- Claude Code: tool executes with `tool_input` from the original `PreToolUse` event

The only case where the LLM sees the approval decision directly is AutoGen's conversation-turn model — designed for creative/interpretive tasks, not deterministic tool execution.

**Approved tool execution flow**:
```
1. Retrieve ToolCallProposal by call_id from store
2. Execute tool(proposal.tool_name, proposal.tool_args)
3. Store ToolResult { call_id, output, success, executed_at }
4. Update ApprovalState → Completed
5. Append tool_result(call_id, output) to conversation history
6. Continue agentic loop (LLM now sees the result)
```

### Pattern 3: Proposal Persistence

**Verdict**: Proposals must be persisted before the operator is notified.

The data that must be preserved:
```rust
pub struct ToolCallProposal {
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub display_summary: Option<String>,
    pub proposed_at: DateTime<Utc>,
}
```

Storage options for cairn (in order of architectural fit):
1. **Event log as source of truth** — `ToolCallProposed` event persisted to the existing event store before operator notification; reconstruct state by replaying. Most consistent with RFC 020 durability model.
2. **SQLite pending_tool_calls table** — simpler; required if events are not yet durable for this domain.
3. **In-memory hashmap per session** — sufficient only if sessions cannot survive process restart.

### Pattern 4: Allow-Once vs Allow-Session

**Verdict**: Both scopes must be first-class. They differ in lookup key.

```rust
// Session-level allow registry:
// key: (session_id, tool_name, args_pattern) → Allow
// populated by: ApprovalScope::Session approval
// expires: session end

// Call-level allow:
// key: call_id → Allow
// populated by: ApprovalScope::Once approval
// expires: after first execution

pub enum ApprovalScope {
    Once,                              // keyed by call_id; consumed on execution
    Session { pattern: ToolPattern },  // keyed by (session, tool, pattern); reusable
}

pub struct ToolPattern {
    pub tool_name: String,
    pub args_glob: Option<String>,  // None = match all args for this tool
}
```

**Lookup order** (mirrors Claude Code's deny → ask → allow precedence):
```
1. Check deny rules (operator-configured static deny list)
2. Check session-level allow registry
3. Check call-level allow registry
4. If nothing matches: Ask the operator
```

### Pattern 5: Retry After Failed Execution

**Verdict**: A new approval is required if the approved execution fails and the system wants to retry with different args.

Suggested policy:
```rust
pub enum RetryPolicy {
    /// Transient failures (network timeout, etc.): retry without new approval
    RetryTransient { max_attempts: u32 },
    /// Logic failures (tool returned error): require new approval
    RequireNewApproval,
    /// Hard failures (tool panicked): surface to operator, do not retry
    EscalateToOperator,
}
```

### Pattern 6: UX Affordance

**Verdict**: Operator should see full `tool_name + tool_args + display_summary`. Args should NOT be editable before approval.

If operators need to modify args, the workflow is:
1. Reject the original proposal with a correction note
2. The rejection is returned to the LLM as a `tool_result` error with the correction note
3. The LLM emits a new `tool_use` with modified args
4. The new proposal goes through the normal approval flow

This is how OpenAI Agents SDK handles it: `state.reject(interruption, rejection_message="Use city='San Francisco' instead")`.

---

## Recommended Architecture for cairn-rs BP-v2

### API Contract Sketch

#### Domain Events

```rust
// Emitted when LLM response parsed:
pub struct ToolCallProposed {
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub display_summary: Option<String>,
    pub proposed_at: DateTime<Utc>,
}

// Emitted when operator acts:
pub struct ToolCallApproved {
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub operator_id: OperatorId,
    pub scope: ApprovalScope,
    pub approved_at: DateTime<Utc>,
}

pub struct ToolCallRejected {
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub operator_id: OperatorId,
    pub reason: Option<String>,
    pub rejected_at: DateTime<Utc>,
}
```

#### State Transitions

```
LLM emits tool_use block
         │
         ▼
ToolCallProposed event emitted
ToolCallProposal persisted to store
         │
         ▼
Evaluate against session-level allow registry
         │
    ┌────┴────┐
    │  Match  │──────────────────────► Execute with original args
    └────┬────┘                            │
         │ No match                        ▼
         ▼                          ToolResult stored
Evaluate against deny rules         Tool_result appended to conversation
         │                          LLM continues
    ┌────┴────┐
    │  Match  │──────────────────────► Reject (return tool_result error)
    └────┬────┘
         │ No match
         ▼
Emit OperatorApprovalRequired notification
ApprovalState → PendingApproval

[Operator sees: tool_name, tool_args, display_summary]
[Operator decides: Allow-once | Allow-session | Reject]

         │
    ┌────┴──────────┐──────────────────────────────► Reject
    │ Allow-once    │ Allow-session
    │               │
    ▼               ▼
Retrieve ToolCallProposal by call_id   ← THE FIX: this lookup was missing
Execute tool(proposal.tool_name, proposal.tool_args)
[On Allow-session: add to session allow registry]
Store ToolResult
ApprovalState → Completed
Append tool_result(call_id, result) to conversation
Continue agentic loop
```

#### Service Interface Sketch

```rust
#[async_trait]
pub trait ApprovalService: Send + Sync {
    /// Called by execute phase after parsing LLM response.
    /// Persists proposal, evaluates policy, returns decision.
    async fn submit_proposal(
        &self,
        proposal: ToolCallProposal,
    ) -> Result<ApprovalDecision, ApprovalError>;
    
    /// Called by operator API endpoint.
    async fn approve(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        scope: ApprovalScope,
    ) -> Result<(), ApprovalError>;
    
    /// Called by operator API endpoint.
    async fn reject(
        &self,
        call_id: ToolCallId,
        operator_id: OperatorId,
        reason: Option<String>,
    ) -> Result<(), ApprovalError>;
    
    /// Called by execute phase to retrieve the original proposal after approval.
    /// This is the key method that closes the "lost proposal" bug.
    async fn retrieve_approved_proposal(
        &self,
        call_id: ToolCallId,
    ) -> Result<ToolCallProposal, ApprovalError>;
    
    /// Called by execute phase to await operator decision.
    /// Returns when the operator approves or rejects.
    async fn await_decision(
        &self,
        call_id: ToolCallId,
    ) -> Result<OperatorDecision, ApprovalError>;
}

pub enum ApprovalDecision {
    /// Tool is in allow list — execute immediately.
    AutoApproved,
    /// Tool is in deny list — reject immediately.
    AutoRejected { reason: String },
    /// Operator must decide — execution suspended until approved/rejected.
    PendingOperator,
}

pub enum OperatorDecision {
    Approved { scope: ApprovalScope },
    Rejected { reason: Option<String> },
}
```

#### Execute Phase Pseudocode (Fixed)

```rust
// In cairn-orchestrator/src/phases/execute.rs:

async fn execute_phase(ctx: &ExecuteContext, llm_response: &LlmResponse) {
    let tool_calls = parse_tool_calls(llm_response);
    let mut tool_results = Vec::new();
    
    for tc in tool_calls {
        let proposal = ToolCallProposal {
            call_id: tc.id.clone(),
            session_id: ctx.session_id.clone(),
            run_id: ctx.run_id.clone(),
            tool_name: tc.name.clone(),
            tool_args: tc.args.clone(),
            display_summary: generate_summary(&tc),
            proposed_at: Utc::now(),
        };
        
        // STEP 1: Submit proposal — persists it, evaluates policy
        match ctx.approval_service.submit_proposal(proposal).await? {
            ApprovalDecision::AutoApproved => {
                // Fast path: policy allows this tool
                let result = ctx.tool_registry.execute(&tc.name, &tc.args).await;
                tool_results.push(ToolResult { call_id: tc.id, result });
            }
            
            ApprovalDecision::AutoRejected { reason } => {
                // Static deny rule
                tool_results.push(ToolResult {
                    call_id: tc.id,
                    result: Err(ToolError::Rejected(reason)),
                });
            }
            
            ApprovalDecision::PendingOperator => {
                // SUSPEND: wait for operator decision via channel/notification
                let decision = ctx.approval_service
                    .await_decision(tc.id.clone()).await?;
                
                match decision {
                    OperatorDecision::Approved { scope } => {
                        // STEP 2: Retrieve THE ORIGINAL PROPOSAL by call_id
                        // This is what fixes the bug: we use the stored proposal,
                        // NOT a fresh LLM call.
                        let proposal = ctx.approval_service
                            .retrieve_approved_proposal(tc.id.clone()).await?;
                        
                        // Register session-level approval if requested
                        if let ApprovalScope::Session { ref pattern } = scope {
                            ctx.session_allow_registry.add(pattern.clone()).await;
                        }
                        
                        // Execute with ORIGINAL ARGS from proposal
                        let result = ctx.tool_registry
                            .execute(&proposal.tool_name, &proposal.tool_args).await;
                        tool_results.push(ToolResult { call_id: tc.id, result });
                    }
                    
                    OperatorDecision::Rejected { reason } => {
                        // Rejection is surfaced to LLM as a tool_result error
                        // LLM may adjust plan based on the reason
                        tool_results.push(ToolResult {
                            call_id: tc.id,
                            result: Err(ToolError::Rejected(reason)),
                        });
                    }
                }
            }
        }
    }
    
    // Append all tool_results to conversation
    // LLM is called next (for the first time since the proposals) to plan next step
    ctx.append_tool_results(tool_results).await?;
}
```

---

## Common Pitfalls

| Pitfall | Why It Happens | How to Avoid |
|---|---|---|
| Re-querying LLM after approval | Treating approval as a new user turn instead of resuming the existing tool call | The approved call_id must map back to a stored proposal; use `retrieve_approved_proposal()` |
| Losing the proposal on restart | Storing proposals only in memory | Emit `ToolCallProposed` as a durable event before notifying operator |
| Duplicate approvals | Operator clicks "approve" twice | Make `approve()` idempotent: second call is a no-op if already `Approved` |
| Approval for wrong tool version | Args changed between proposal and approval | Proposals are immutable once persisted; never mutate `tool_args` post-proposal |
| Session allow bleeding across sessions | Session-level approvals not scoped to session_id | Key session allow entries by `(session_id, tool_name, pattern)`, expire on session end |
| Approval timeout race | Operator takes 30min; session expired | Store proposals in durable store; allow operator to approve even after session timeout |
| Tool editing args at execution time | Execute layer modifies `tool_args` before running | Execute layer must use `proposal.tool_args` verbatim; no transformation |
| LangGraph-style re-execution side effects | Code before interrupt() runs twice on resume | Keep approval node idempotent up to the interrupt/await call |

---

## Best Practices

1. **Emit `ToolCallProposed` before notifying operator** — the event log is the source of truth; notification is derived

2. **Approval decision is idempotent** — a second `Approve` on an already-approved call_id is a no-op or returns the existing decision

3. **Operator sees full args** — display `tool_name + tool_args + display_summary` verbatim; no hiding

4. **Reject message feeds back to LLM as tool_result error** — the LLM can adjust its plan based on the rejection message; this is the right corrective loop

5. **Fail-closed on policy gaps** — if no rule matches, default to `Ask` not `Allow`

6. **Allow-session is tied to session_id**, not to global state — session-level approvals expire when the session ends

7. **One approval per call_id** — never reuse a call_id across retries; each retry attempt gets a new call_id

8. **await_decision() must be cancellable** — if the session is cancelled while waiting for operator approval, the awaiter must unblock cleanly

---

## Open Questions for cairn-rs Implementor

Before implementing BP-v2, the following questions need answers:

1. **Durability requirement**: Do proposals need to survive process restart, or is in-memory sufficient? If sessions survive restarts, proposals must be persisted (SQLite table or `ToolCallProposed` event).

2. **Suspension mechanism**: When a tool call hits `PendingOperator`, how does the execute loop suspend?
   - `tokio::sync::oneshot` channel per call_id (clean, but non-persistent)
   - Long-poll endpoint that the execute phase subscribes to
   - Event store subscription via polling on the `ToolCallApproved` / `ToolCallRejected` events
   The choice depends on answer to #1.

3. **Operator notification transport**: How is the operator notified of a pending approval? REST endpoint polling, WebSocket push, notification system (Valkey pub/sub), or all three?

4. **Timeout policy**: If an operator never responds, should the call eventually auto-reject? After how long? Who configures this?

5. **Multi-tool batches**: LangGraph and OpenAI both handle parallel tool calls (multiple tools in one LLM response). Does cairn's approval flow allow auto-approved tools in a batch to execute immediately while pending-approval tools wait? Or does the whole batch wait for the slowest approval?

6. **Approval scope matching**: What is the matching function for `ToolPattern` in session-level approvals? Exact args match? JSON subset match? Wildcard on specific fields?

7. **Operator editing of args**: The current recommendation is reject-and-replay for corrections. Is this acceptable UX for cairn's operators, or do they need inline arg editing before approval?

---

## Further Reading

| Resource | Type | Why Recommended |
|---|---|---|
| [OpenAI Agents SDK HITL Guide](https://openai.github.io/openai-agents-python/human_in_the_loop/) | Official Docs | Best concrete code example of the full approve/reject/resume cycle |
| [LangGraph types.py source](https://raw.githubusercontent.com/langchain-ai/langgraph/main/libs/langgraph/langgraph/types.py) | Source | `interrupt()` implementation — checkpoint-based pause/resume |
| [Claude Code Permissions](https://code.claude.com/docs/en/permissions) | Official Docs | Best treatment of rule-layer vs hook-layer, deny-first precedence |
| [Claude Code Hooks Guide](https://code.claude.com/docs/en/hooks-guide) | Official Docs | `PreToolUse` + `PermissionRequest` hook input/output schemas |
| [OpenCode permission/index.ts](https://raw.githubusercontent.com/sst/opencode/dev/packages/opencode/src/permission/index.ts) | Source | Effect-TS Deferred suspension model for ask/allow/once semantics |
| [Codex exec_policy.rs](https://raw.githubusercontent.com/openai/codex/main/codex-rs/core/src/exec_policy.rs) | Source | Rust reference implementation of policy evaluation + amendment persistence |
| `harness-core permissions.rs` | Local | Direct ancestor of cairn's permission model; `PermissionDecision` variants |
| [pledge(2) man page](https://man.openbsd.org/pledge.2) | Reference | OS analogy: monotonic capability reduction, fail-closed violation |

---

*This guide was synthesized from 32 primary sources.*
*Full source metadata is not included in this repository copy of the guide.*
