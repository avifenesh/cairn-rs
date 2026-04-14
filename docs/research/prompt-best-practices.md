# System Prompt Best Practices for Autonomous Coding Agents

Research compiled 2026-04-14. Sources: Anthropic documentation (platform.claude.com),
Anthropic engineering blog ("Building Effective Agents", "Writing Tools for Agents",
"Claude Code Best Practices"), OpenAI cookbook, SWE-agent (Princeton NLP), OpenHands,
Cursor, Lilian Weng's survey, and community research (2024-2025).

---

## Context: The Problem

An agent orchestrator sends a goal (e.g., "implement this GitHub issue") and the agent
must use tools (file read/write, shell, git, GitHub CLI) to accomplish it. The observed
failure mode: the agent calls `tool_search`, reads some files, then calls `complete_run`
instead of actually writing code and creating PRs. This document synthesizes research
into what causes this and how to fix it.

---

## 1. Generic vs. Specific Prompts

### The Research Says

Anthropic's prompting best practices are explicit: "Think of Claude as a brilliant but
new employee who lacks context on your norms and workflows. The more precisely you
explain what you want, the better the result." Their golden rule: "Show your prompt to a
colleague with minimal context on the task and ask them to follow it. If they'd be
confused, Claude will be too."

### When to Be Generic

- **Exploration/research phases** where you want the agent to discover the right
  approach. A prompt like "investigate the auth module" lets the agent decide what to
  read.
- **Tasks where the scope is genuinely unknown.** If you cannot describe the expected
  diff, a looser prompt avoids over-constraining.

### When to Be Specific (Prescriptive)

- **When the agent must produce concrete artifacts** (code changes, PRs, commits).
  Vague prompts like "fix the bug" lead to analysis without action.
- **When there is a defined workflow** the agent must follow (read issue, write code,
  run tests, commit, push, create PR).
- **When the agent has a history of premature termination.** Specificity counteracts the
  model's tendency to satisfy the request with analysis alone.

### Practical Guidance

The sweet spot is **specific about outcomes, flexible about approach**:

```
BAD (too vague):
  "Fix the login issue."

BAD (too prescriptive on implementation):
  "Open src/auth.rs, go to line 47, change the if condition to check
   for None, save the file."

GOOD (specific outcome, flexible approach):
  "Users report login fails after session timeout. Investigate the auth
   flow in src/auth/, especially token refresh. Write a failing test that
   reproduces the issue, fix it, run the test suite, and commit the fix."
```

Anthropic's Claude Code docs reinforce this with a concrete table showing that scoping
the task, pointing to sources, referencing existing patterns, and describing symptoms
all dramatically improve results over vague prompts.

---

## 2. Tool Presentation: Upfront vs. Discovery

### The Research Says

**Anthropic's position**: List all tools upfront in the `tools` parameter. The API
constructs a system prompt from tool definitions automatically. Claude has been trained
on thousands of successful trajectories with specific tool schemas (especially
Anthropic-schema tools like `bash` and `text_editor`), so listing them upfront gives
the model the best chance of using them correctly.

**SWE-agent (Princeton NLP)**: The Agent-Computer Interface (ACI) design is central.
They "spent more time optimizing tools than the overall prompt." Their tools are listed
upfront with detailed descriptions, not discovered dynamically.

**OpenAI**: Functions are defined in the `tools` parameter at request time. No discovery
mechanism; all available functions are presented upfront.

### Upfront Listing (Recommended)

**Pros:**
- Model knows full capability surface from the start
- Can plan multi-step workflows that chain tools
- No wasted tokens on discovery calls
- Trained-in schemas (bash, text_editor) are called more reliably

**Cons:**
- More input tokens per request (tool definitions add ~20-50 tokens each for simple
  tools, ~100-200 for complex ones)
- With many tools (50+), the model may have difficulty selecting the right one

### Dynamic Discovery

**Pros:**
- Reduces context pollution when you have hundreds of tools
- Allows the agent to focus on relevant tools for the current task

**Cons:**
- Adds round-trip latency (discovery call before real work)
- Model may not discover the tools it needs
- Creates the exact failure mode observed: agent discovers tools, reads files, then
  "completes" because it never found/understood the write tools

### Practical Guidance for Cairn

For a coding agent with a bounded tool set (file read/write, bash, git, GitHub CLI),
**list all tools upfront**. Use Anthropic's `tool_search` server tool or deferred
loading only if you have 50+ tools and need to reduce context.

For the `complete_run` tool specifically: do NOT list it as a regular tool alongside
others. Make it a special tool that the agent can only call after meeting explicit
criteria (see Section 5).

---

## 3. Structured Workflow vs. Free-Form

### The Research Says

Anthropic's "Building Effective Agents" blog distinguishes **workflows** (predetermined
code paths) from **agents** (dynamic tool use). Their recommendation: "Find the simplest
solution possible, and only increasing complexity when needed." They explicitly recommend
starting with workflows and graduating to agents only for open-ended problems.

Claude Code's best practices describe a four-phase workflow: **Explore, Plan, Implement,
Commit**. This is not free-form; it is a structured progression that the human or system
enforces.

OpenHands' CodeAct agent follows a similar structure: **exploration, analysis, testing,
implementation, verification**.

### The Spectrum

```
RIGID WORKFLOW                                    FREE-FORM AGENT
Step 1: Read issue    <-- Prescribed -->          "Here's the issue,
Step 2: Find files                                 fix it however
Step 3: Write code                                 you want."
Step 4: Run tests
Step 5: Commit
Step 6: Push + PR
```

### What Works for Coding Agents

**Hybrid: Structured phases with agent freedom within each phase.** The system prompt
defines the phases and their completion criteria. Within each phase, the agent has
freedom to choose which tools to use and in what order.

```
You are a software engineer. Your task is to implement the changes described below.

## Workflow

You MUST follow these phases in order. Do not skip phases.

### Phase 1: Understand
- Read the issue/task description carefully
- Explore the relevant codebase areas
- Identify the files that need to change

### Phase 2: Implement
- Write the actual code changes
- You MUST create or modify files. Reading files alone is not sufficient.

### Phase 3: Verify
- Run the test suite
- Fix any failures
- Run linters if available

### Phase 4: Deliver
- Commit changes with a descriptive message
- Push to a branch and create a pull request

Do not call complete_run until all four phases are done and you have
created a pull request.
```

### Why Free-Form Fails for This Use Case

Without phase structure, the model treats the entire task as a single "answer the
question" problem. Reading files and understanding the codebase feels like answering
the question, so the model calls `complete_run`. Phase structure makes it clear that
understanding is only Phase 1 of 4.

---

## 4. Examples and Few-Shot Prompting

### The Research Says

Anthropic's prompting guide: "Examples are one of the most reliable ways to steer
Claude's output format, tone, and structure. A few well-crafted examples (known as
few-shot or multishot prompting) can dramatically improve accuracy and consistency."

Their specific recommendations:
- **3-5 examples** for best results
- Examples should be **relevant** (mirror actual use case), **diverse** (cover edge
  cases), and **structured** (wrapped in `<example>` tags)
- For tool use specifically, Anthropic now supports `input_examples` on tool definitions
  that are schema-validated

Anthropic also notes that multishot examples work with thinking: "Use `<thinking>` tags
inside your few-shot examples to show Claude the reasoning pattern."

### Do Examples Help Tool-Use Patterns?

**Yes, significantly.** The key insight from the "Writing Tools for Agents" blog:
examples embedded in tool descriptions help the model understand when and how to use
each tool. But for the broader agent workflow, examples in the system prompt showing
the full trajectory (read -> write -> test -> commit) are even more powerful.

### Practical Example Format for Coding Agents

```xml
<examples>
<example>
<task>Add input validation to the /api/users endpoint</task>
<trajectory>
1. Read the issue to understand requirements
2. Read src/api/users.rs to understand current implementation
3. Read src/api/validation.rs to understand existing validation patterns
4. Edit src/api/users.rs to add validation middleware
5. Create tests/api/test_user_validation.rs with test cases
6. Run: cargo test -p api -- test_user_validation
7. Fix any test failures
8. Run: cargo clippy
9. Commit: "feat: add input validation to /api/users endpoint"
10. Push branch and create PR
</trajectory>
</example>

<example>
<task>Fix: dashboard chart renders empty when no data</task>
<trajectory>
1. Read the bug report
2. Read ui/src/components/Chart.tsx
3. Read ui/src/lib/api.ts to understand data fetching
4. Edit Chart.tsx to handle empty data state with a placeholder
5. Edit ui/src/tests/Chart.test.tsx to add empty-data test case
6. Run: cd ui && npm test -- Chart
7. Commit: "fix: show placeholder when dashboard chart has no data"
8. Push and create PR
</trajectory>
</example>
</examples>
```

### Key Insight

The examples above show the agent that a valid trajectory **always includes writing
files, running tests, and creating a PR**. An agent that only reads files and returns
analysis has not matched any of the example patterns.

---

## 5. Preventing Early Termination

This is the core problem. The agent reads files, does analysis, then calls
`complete_run` without producing artifacts.

### Root Causes (from research)

1. **The completion tool is too easy to call.** If `complete_run` has no preconditions,
   the model will call it as soon as it has a plausible response.

2. **The system prompt does not distinguish "understanding" from "completing."** Without
   phase structure, the model treats analysis as completion.

3. **No verification step.** Without tests or checks, the model has no way to validate
   its own work, so it defaults to "I analyzed it, I'm done."

4. **Context awareness triggers wrap-up.** Anthropic's docs note: "Claude may sometimes
   naturally try to wrap up work as it approaches the context limit." If the agent reads
   many files, context fills up and the model starts wrapping up.

### Solutions (Ordered by Effectiveness)

#### 5a. Redesign the `complete_run` Tool

Make the completion tool require evidence of work done:

```json
{
  "name": "complete_run",
  "description": "Signal that the task is complete. ONLY call this after you have: (1) written or modified at least one file, (2) run tests or verification, and (3) committed changes. If you have not done all three, continue working instead of calling this tool. This tool requires a summary of changes made.",
  "input_schema": {
    "type": "object",
    "properties": {
      "files_modified": {
        "type": "array",
        "items": {"type": "string"},
        "description": "List of files you created or modified. Must be non-empty."
      },
      "tests_passed": {
        "type": "boolean",
        "description": "Whether tests passed after your changes."
      },
      "pr_url": {
        "type": "string",
        "description": "URL of the pull request you created."
      },
      "summary": {
        "type": "string",
        "description": "Summary of what you implemented and why."
      }
    },
    "required": ["files_modified", "tests_passed", "summary"]
  }
}
```

The key insight from SWE-agent's poka-yoke design principle: "Change the arguments so
that it is harder to make mistakes." By requiring `files_modified` as a non-empty array,
you make it structurally impossible to complete without having modified files.

#### 5b. Add Explicit Anti-Termination Instructions

From Anthropic's own guidance for long-horizon agents:

```
Do not call complete_run until you have made actual code changes. Reading and
analyzing files is not completing the task. You must:
1. Write or modify source code files
2. Run tests to verify your changes work
3. Commit and push your changes

If you find yourself wanting to complete without having edited any files,
STOP and ask yourself: "What code changes would solve this task?" Then make
those changes.
```

#### 5c. Use Context Awareness Prompting

From Anthropic's prompting best practices for agentic systems:

```
Your context window will be automatically compacted as it approaches its limit,
allowing you to continue working indefinitely. Do not stop tasks early due to
token budget concerns. Always be as persistent and autonomous as possible and
complete tasks fully. Never artificially stop any task early.
```

#### 5d. Harness-Level Enforcement

Beyond prompt engineering, enforce completion criteria in the orchestrator code:

```rust
// In your agent harness, when the agent calls complete_run:
if agent_response.tool == "complete_run" {
    let files_changed = get_git_diff_files();
    if files_changed.is_empty() {
        // Reject the completion and send back a tool_result error
        return tool_result_error(
            "You have not modified any files. The task requires code changes. \
             Please implement the solution before completing."
        );
    }
}
```

This is the most reliable approach because it does not depend on the model following
instructions. The harness physically prevents premature completion.

#### 5e. Separate the "Give Up" Path

Provide a different tool for when the agent genuinely cannot complete the task:

```json
{
  "name": "request_help",
  "description": "Call this ONLY if you are truly stuck and cannot make progress. Explain what you tried and what blocked you. This is different from complete_run, which requires actual code changes."
}
```

This gives the model an escape hatch that does not conflate "I'm stuck" with "I'm done."

---

## 6. Identity Framing

### The Research Says

Anthropic's prompting guide: "Setting a role in the system prompt focuses Claude's
behavior and tone for your use case. Even a single sentence makes a difference."

OpenHands identifies its agent as: "You are OpenHands agent, a helpful AI assistant
that can interact with a computer to solve tasks."

Claude Code's own system prompt (per Anthropic docs) identifies the agent as an
autonomous coding environment that "explores, plans, and implements."

### "Senior Engineer" vs. "AI Assistant"

| Framing | Effect | When to Use |
|---------|--------|-------------|
| "You are a senior software engineer" | More decisive, writes code confidently, less likely to hedge or ask for permission | When the agent should act autonomously and make implementation decisions |
| "You are an AI assistant that helps with coding" | More cautious, more likely to explain and ask questions, may analyze without acting | When the agent should be collaborative and seek approval |
| "You are an autonomous coding agent" | Emphasizes independence, takes initiative, completes tasks end-to-end | When the agent runs without human supervision |

### Practical Recommendation

For an autonomous agent that must produce code and PRs without human interaction:

```
You are a senior software engineer working autonomously. You have been
assigned a task and must complete it end-to-end: understand the problem,
write the code, verify it works, and deliver it as a pull request.

You do not ask questions. You do not provide analysis without action.
You write code, test it, and ship it.
```

The identity should match the expected behavior. If you want the agent to write code,
frame it as an engineer who writes code, not an assistant who answers questions.

### Avoid Over-Identification

Do not spend many tokens on elaborate backstories. One to three sentences is enough.
The role serves as a behavioral anchor, not a persona simulation.

---

## 7. Adversarial vs. Collaborative Tone

### The Research Says

Anthropic's latest guidance (Claude 4.5/4.6 prompting best practices) is clear:
"Claude Opus 4.5 and Claude Opus 4.6 are more responsive to the system prompt than
previous models. If your prompts were designed to reduce undertriggering on tools or
skills, these models may now overtrigger. The fix is to **dial back any aggressive
language**. Where you might have said 'CRITICAL: You MUST use this tool when...', you
can use more normal prompting like 'Use this tool when...'."

This is a significant shift from earlier recommendations (2023-2024) where adversarial
emphasis was sometimes necessary to get models to follow instructions.

### Effectiveness Comparison

```
ADVERSARIAL (less effective with modern models):
  "CRITICAL: You MUST NOT call complete_run without writing code.
   FAILURE to write code is UNACCEPTABLE. You will be PENALIZED
   for early termination."

STRUCTURED (more effective):
  "## Completion Criteria
   Before calling complete_run, verify:
   - [ ] You have modified at least one source file
   - [ ] Tests pass
   - [ ] Changes are committed
   If any item is unchecked, continue working."
```

### Why Adversarial Tone Backfires

1. **Overtriggering.** Modern models are highly responsive to emphasis. "MUST" and
   "CRITICAL" cause the model to over-index on that specific instruction at the expense
   of others.

2. **Attention competition.** When everything is "CRITICAL," nothing is. The model
   cannot distinguish genuinely important constraints from noise.

3. **Behavioral distortion.** Threatening language ("you will be penalized") can cause
   the model to become defensive, hedging its outputs or adding unnecessary
   qualifications.

### When Emphasis is Warranted

Use measured emphasis (not adversarial tone) for genuine constraints:

```
GOOD: "IMPORTANT: Always use absolute file paths. Relative paths will fail."
BAD:  "CRITICAL WARNING: You MUST ALWAYS use absolute paths or EVERYTHING WILL BREAK."
```

The Claude Code CLAUDE.md guidance confirms this: "You can tune instructions by adding
emphasis (e.g., 'IMPORTANT' or 'YOU MUST') to improve adherence." Note the measured
language -- "IMPORTANT" and "YOU MUST" are fine; aggressive adversarial framing is not.

### Practical Approach

Use **structured requirements** (checklists, numbered steps, phase gates) instead of
adversarial emphasis. Structure is more reliable than tone for controlling behavior.

---

## 8. Anti-Patterns

### From Anthropic's Research

1. **The Kitchen Sink Session.** Mixing unrelated tasks in one context. Context becomes
   polluted with irrelevant information, degrading performance.

2. **Correcting Over and Over.** Failed corrections accumulate in context, making things
   worse. After two failed corrections, start fresh with a better prompt.

3. **The Over-Specified CLAUDE.md.** If instructions are too long, the model ignores
   half of them. "If Claude keeps doing something you don't want despite having a rule
   against it, the file is probably too long and the rule is getting lost."

4. **The Trust-Then-Verify Gap.** Accepting plausible-looking output without verification.
   Always provide tests, scripts, or screenshots for the agent to self-check.

5. **The Infinite Exploration.** Unscoped investigation that reads hundreds of files,
   filling context without producing artifacts.

### From "Writing Tools for Agents"

6. **Wrapping Every API as a Tool.** Creates confusion and context bloat. Consolidate
   related operations into fewer, more capable tools.

7. **Generic Tool Descriptions.** "Gets the stock price for a ticker" vs. a detailed
   description explaining when to use it, what it returns, and what it does not do.
   Vague descriptions cause incorrect tool selection.

8. **Returning Too Much Data.** Tool responses that dump full objects instead of the
   specific fields the agent needs. "Bloated responses waste context and make it harder
   for Claude to extract what matters."

9. **Overlapping Tools.** Multiple tools that do similar things confuse the model about
   which to use. Each tool should have a single, distinct purpose.

### From SWE-agent / Community Research

10. **No Error Recovery Path.** When a tool call fails, the agent has no guidance on
    what to try next. Always include error handling guidance in tool descriptions.

11. **Overly Rigid Completion Criteria.** Verification that only accepts exact string
    matches. Use semantic verification (does the test pass?) not syntactic (does the
    output match this exact string?).

12. **Prompt-Level Implementation Details.** Telling the agent which lines to change
    instead of what behavior to achieve. This prevents the agent from finding better
    solutions.

13. **Missing Context About Environment.** Not telling the agent what tools are
    installed, what the project structure looks like, or how to run tests. The agent
    wastes tokens discovering this information.

### Specific to the Early-Termination Problem

14. **Completion Tool Without Preconditions.** A `complete_run` or `finish` tool that
    takes no arguments or only a summary string. The model can call it with zero work
    done.

15. **No Distinction Between Phases.** When "analyze" and "implement" are the same
    phase, the model treats analysis as implementation.

16. **Analysis-Framed Goals.** "Investigate and fix the bug" may be interpreted as
    "investigate the bug" (done!) rather than "fix the bug." Use action verbs:
    "implement," "write," "create," not "investigate," "analyze," "look at."

---

## 9. Provider-Specific Guidance

### Anthropic (Claude)

**Tool Choice Parameter:**
- `auto` (default): Claude decides whether to call tools
- `any`: Must use one of the provided tools
- `tool`: Must use a specific tool
- `none`: Cannot use any tools

For agents, use `auto` but design tool descriptions to guide selection. Do not use
`any` or forced tool choice in an agent loop, as it prevents the model from deciding
when to stop using tools.

**Anthropic-Schema Tools (bash, text_editor):**
These are trained-in tool schemas. Claude has been optimized on thousands of successful
trajectories with these exact signatures. Using them gives significantly better results
than custom tools that do the same thing. If your agent needs to edit files and run
commands, use these schemas.

**Adaptive Thinking:**
For agentic coding workloads, use `thinking: {type: "adaptive"}` with `effort: "high"`.
This lets Claude dynamically decide when to reason deeply vs. act quickly.

**Parallel Tool Calls:**
Claude excels at parallel execution. Prompt for it explicitly: "If you intend to call
multiple tools and there are no dependencies between them, make all independent calls
in parallel."

**Context Window Management:**
Claude's context window fills fast during coding tasks. Use subagents for investigation
to keep the main context clean for implementation. Provide context compaction guidance
in the system prompt.

**Key Anthropic Insight:**
"Tool access is one of the highest-leverage primitives you can give an agent." Their
benchmarks show that adding even basic tools produces outsized capability gains on
SWE-bench and other coding benchmarks.

### OpenAI

**Function Calling:**
- Three control modes: `auto`, forced (`{"type": "function", "function": {"name": ...}}`),
  and `none`
- "Don't make assumptions about what values to plug into functions. Ask for clarification
  if a user request is ambiguous."
- Parallel function calling supported in GPT-4 and later

**System Prompt for Agents:**
OpenAI recommends using system messages to guide function selection. The system prompt
should explain the agent's role, available functions, and when to use them.

### Model-Agnostic Best Practices

These patterns work across providers:

1. **Detailed tool descriptions** (3-4+ sentences per tool, including when NOT to use it)
2. **Required parameters** that force evidence of work (files_modified, test_results)
3. **Error messages that guide recovery** (not just "failed" but "failed because X, try Y")
4. **Semantic identifiers** in tool responses (names, not UUIDs)
5. **Consolidated tools** (one tool with an action parameter vs. many overlapping tools)

---

## 10. Synthesis: Recommended System Prompt Architecture for Cairn

Based on all research, here is the recommended structure for an autonomous coding agent
system prompt in Cairn's orchestrator:

```
[IDENTITY]        1-3 sentences. Senior engineer, autonomous, ships code.
[ENVIRONMENT]     What tools are available, project structure, how to run tests.
[WORKFLOW]        Numbered phases with explicit completion criteria per phase.
[TOOL GUIDANCE]   When to use each tool category (read, write, shell, git).
[COMPLETION]      Explicit criteria for calling complete_run. Checklist format.
[ANTI-PATTERNS]   Short list of things NOT to do (do not complete without code changes).
[EXAMPLES]        2-3 few-shot trajectories showing full read-write-test-commit cycles.
[CONTEXT MGMT]    Instructions for handling large codebases and context limits.
```

### Concrete Template

```
You are a senior software engineer working autonomously on a codebase. You have
been assigned a task and must complete it end-to-end: understand the problem,
write the code, verify it works, and deliver it.

## Environment
- Project: {{project_name}} ({{language}})
- Build: {{build_command}}
- Test: {{test_command}}
- Available tools: file_read, file_write, bash, git, github_cli

## Workflow

Follow these phases in order.

### Phase 1: Understand
Read the task description. Explore relevant code. Identify what needs to change.

### Phase 2: Implement
Write the code changes. You MUST modify or create files. Reading alone is not
completing the task.

### Phase 3: Verify
Run tests: {{test_command}}
Fix any failures. Run linters if configured.

### Phase 4: Deliver
Commit with a descriptive message. Push to a branch. Create a pull request.

## Completion Criteria

Before calling complete_run, verify ALL of these:
- You have modified or created at least one source file
- Tests pass (or you have explained why tests cannot be added)
- Changes are committed and pushed
- A pull request exists

If any criterion is not met, continue working. Do not call complete_run.

## What NOT to Do
- Do not complete after only reading files. Reading is Phase 1; you must also
  do Phases 2-4.
- Do not provide analysis or recommendations without implementing them.
- Do not skip tests.
- Do not ask the user questions. You have the tools to find answers yourself.
```

### Harness-Level Reinforcement

In addition to the prompt, the Cairn orchestrator should:

1. **Validate `complete_run` calls** by checking git diff for actual changes
2. **Reject premature completions** with an error message guiding the agent back to work
3. **Track phase progression** (has the agent written any files? run any tests?)
4. **Set iteration limits** (max 50 tool calls) with clear guidance on what to do at
   the limit
5. **Provide `request_help`** as a separate tool for genuine blockers

---

## References

1. Anthropic, "Building Effective Agents" (2024)
   https://www.anthropic.com/engineering/building-effective-agents

2. Anthropic, "Writing Tools for Agents" (2025)
   https://www.anthropic.com/engineering/writing-tools-for-agents

3. Anthropic, "Prompting Best Practices" (2025)
   https://platform.claude.com/docs/en/docs/build-with-claude/prompt-engineering/claude-prompting-best-practices

4. Anthropic, "Claude Code Best Practices" (2025)
   https://code.claude.com/docs/en/best-practices

5. Anthropic, "Define Tools" (2025)
   https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/define-tools

6. Anthropic, "How Tool Use Works" (2025)
   https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/how-tool-use-works

7. Yang et al., "SWE-agent: Agent-Computer Interfaces Enable Automated Software
   Engineering" (2024), Princeton NLP
   https://arxiv.org/abs/2405.15793

8. OpenHands (formerly OpenDevin), CodeAct Agent architecture
   https://github.com/All-Hands-AI/OpenHands

9. OpenAI, "Function Calling" cookbook (2024)
   https://developers.openai.com/cookbook/examples/how_to_call_functions_with_chat_models

10. Lilian Weng, "LLM Powered Autonomous Agents" (2023)
    https://lilianweng.github.io/posts/2023-06-23-agent/

11. Cursor, "Prompt Design" (2024)
    https://www.cursor.com/blog/prompt-design
