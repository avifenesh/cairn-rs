# Real-World Production Coding Agent Prompts: Research

Research date: 2026-04-14
Sources: Published/extracted system prompts, GitHub repos, Anthropic docs, academic papers

---

## Table of Contents

1. [Claude Code (Anthropic)](#1-claude-code-anthropic)
2. [Cursor Agent](#2-cursor-agent)
3. [Windsurf / Cascade (Codeium)](#3-windsurf--cascade-codeium)
4. [Devin AI (Cognition)](#4-devin-ai-cognition)
5. [Cline (Open Source)](#5-cline-open-source)
6. [Amp (Sourcegraph)](#6-amp-sourcegraph)
7. [Kiro (AWS)](#7-kiro-aws)
8. [GitHub Copilot / VS Code Agent](#8-github-copilot--vs-code-agent)
9. [SWE-agent (Princeton)](#9-swe-agent-princeton)
10. [Agentless (UIUC)](#10-agentless-uiuc)
11. [AutoCodeRover (NUS)](#11-autocoderover-nus)
12. [Cross-Cutting Patterns](#12-cross-cutting-patterns)
13. [Implications for Cairn](#13-implications-for-cairn)

---

## 1. Claude Code (Anthropic)

**Source**: Extracted system prompt (v1 and v2.0.0) from published collections; Anthropic best practices docs.

### Prompt Structure (Section Order)

1. **Identity & security policy** -- "You are an interactive CLI tool that helps users with software engineering tasks."
2. **Security guardrails** -- "Assist with defensive security tasks only. Refuse to create, modify, or improve code that may be used maliciously."
3. **Help/feedback links** -- How to get help, report issues.
4. **Tone and style** -- Conciseness rules, verbosity examples.
5. **Proactiveness policy** -- Balance between doing and not surprising.
6. **Professional objectivity** -- "Prioritize technical accuracy and truthfulness over validating the user's beliefs."
7. **Task management (TodoWrite)** -- Detailed instructions for task tracking with examples.
8. **Doing tasks** -- Recommended workflow steps.
9. **Tool usage policy** -- Prefer Task tool for search, batch parallel calls.
10. **Environment info** -- Working directory, platform, date, model name.
11. **Git commit workflow** -- Detailed multi-step protocol with safety rules.
12. **PR creation workflow** -- Step-by-step with `gh` CLI.
13. **Code references** -- `file_path:line_number` pattern.
14. **Tool definitions** -- Bash, Edit, Read, Write, Glob, Grep, Task, TodoWrite, WebFetch.

### Tool Presentation

Tools are presented as structured definitions with:
- Name and description
- Detailed usage notes (when to use, when NOT to use)
- Security/safety constraints inline with each tool
- Cross-references between tools ("Use Read instead of cat")

```
## Bash
Executes a given bash command in a persistent shell session...
IMPORTANT: This tool is for terminal operations like git, npm, docker, etc.
DO NOT use it for file operations - use the specialized tools for this instead.
```

### Key Patterns

**Anti-laziness / Completion enforcement**:
- "IMPORTANT: Always use the TodoWrite tool to plan and track tasks throughout the conversation."
- Detailed examples showing full task completion loops.
- "It is critical that you mark todos as completed as soon as you are done."

**Extreme conciseness enforcement** (v2.0 escalated this further):
- "You MUST answer concisely with fewer than 4 lines"
- "IMPORTANT: You should minimize output tokens as much as possible"
- Verbatim examples: `user: 2 + 2 / assistant: 4`
- "One word answers are best."

**Convention adherence**:
- "NEVER assume that a given library is available, even if it is well known."
- "When you create a new component, first look at existing components."
- "Always follow security best practices. Never introduce code that exposes or logs secrets."

**Defensive security** (appears twice -- top and bottom):
- Repeated for emphasis: "IMPORTANT: Assist with defensive security tasks only."

**Git safety protocol** (highly specific):
- "NEVER update the git config"
- "NEVER run destructive/irreversible git commands unless user explicitly requests"
- "NEVER skip hooks (--no-verify)"
- "NEVER commit changes unless the user explicitly asks"
- Specific authorship check before amending: `git log -1 --format='%an %ae'`
- Co-authored-by trailer required in v2.0.

**Task/system/user split**: Everything is in the system prompt. User messages carry the actual task. The system prompt injects `<system-reminder>` tags into tool results as side-channel context that is "NOT part of the user's provided input."

### Novel Patterns

1. **Model self-identification**: "You are powered by the model named Sonnet 4." -- the agent knows its own model.
2. **TodoWrite as planning primitive**: Not just tracking -- it IS the planning mechanism.
3. **Hooks as deterministic overrides**: Unlike CLAUDE.md (advisory), hooks guarantee execution.
4. **Subagents for context isolation**: Separate context windows prevent exploration from polluting the main session.
5. **Verbosity examples as calibration**: Showing `2+2 -> 4` trains the model's verbosity level more effectively than rules.

---

## 2. Cursor Agent

**Source**: Extracted system prompt (Agent Prompt 2025-09-03) from published collections.

### Prompt Structure (Section Order)

1. **Identity** -- "You are an AI coding assistant, powered by GPT-5. You operate in Cursor."
2. **Pair programming framing** -- "You are pair programming with a USER to solve their coding task."
3. **Agent persistence directive** -- "keep going until the user's query is completely resolved, before ending your turn"
4. **Communication spec** -- Markdown rules, no narration comments, state assumptions.
5. **Status update spec** -- Micro-narrative pattern with tense rules.
6. **Summary spec** -- End-of-turn summary requirements.
7. **Completion spec** -- Reconcile todo list, then summarize.
8. **Flow** -- Discovery pass, structured plan, status updates, reconciliation.
9. **Tool calling** -- Parallel tool calls, schema compliance, natural action descriptions.
10. **Context understanding** -- Semantic search as MAIN exploration tool.
11. **Maximize parallel tool calls** (dedicated section with emphasis).
12. **Grep spec** -- When to use grep vs codebase_search.
13. **Making code changes** -- Never output code to user, use edit tools.
14. **Code style** -- HIGH-VERBOSITY code, naming rules, control flow, comments.
15. **Linter errors** -- Run read_lints, max 3 fix loops.
16. **Non-compliance self-correction** -- Self-correct if rules violated.
17. **Citing code** -- Two methods: file references and fenced blocks.
18. **Markdown spec** -- Heading hierarchy, bold, backticks.
19. **Todo spec** -- Atomic items, verb-led, 14 words max.

### Key Patterns

**Autonomous completion mandate**:
```
keep going until the user's query is completely resolved, before ending
your turn and yielding back to the user. Only terminate your turn when
you are sure that the problem is solved.
```

**Parallel tool calls as primary directive**:
```
CRITICAL INSTRUCTION: For maximum efficiency, whenever you perform
multiple operations, invoke all relevant tools concurrently with
multi_tool_use.parallel rather than sequentially.
```
Dedicated section repeating this with many examples. Claims 3-5x speed improvement.

**Status update micro-narratives**:
- Must emit before every tool batch.
- Uses continuous tense: "I found X. Now I'll do Y."
- Non-compliance rule: "If a turn contains any tool call, the message MUST include at least one micro-update."

**Todo reconciliation gate**:
- "Before starting any new file or code edit, reconcile the todo list: mark newly completed tasks as completed."
- Creates a checkpoint mechanism preventing work on incomplete tasks.

**Self-correction enforcement**:
```
If you fail to call todo_write to check off tasks before claiming them done,
self-correct in the next turn immediately.
```

**HIGH-VERBOSITY code** -- opposite of concise responses:
- Full variable names (no 1-2 char names)
- Explicit type annotations for public APIs
- Guard clauses and early returns
- Comments explaining "why" not "how"

### Novel Patterns

1. **Communication conciseness vs code verbosity paradox**: Messages must be terse, but code must be verbose. This is the right tradeoff.
2. **Flow state machine**: Discovery -> Plan -> Execute -> Reconcile -> Summarize.
3. **Linter loop cap**: Max 3 attempts to fix linter errors, then ask user. Prevents infinite loops.
4. **Self-correction as explicit mechanism**: Not just "try to follow rules" -- explicit self-monitoring with correction protocol.

---

## 3. Windsurf / Cascade (Codeium)

**Source**: Extracted system prompt (Prompt Wave 11) from published collections.

### Prompt Structure (Section Order)

1. **Identity** -- "You are Cascade, a powerful agentic AI coding assistant designed by the Windsurf engineering team."
2. **User information** -- OS, workspace URIs, corpus names.
3. **Tool calling rules** -- 6 numbered rules for when/how to call tools.
4. **Making code changes** -- Never output code, use edit tools, immediately runnable.
5. **Debugging** -- Root cause focus, logging, test isolation.
6. **Memory system** -- Persistent memory database with liberal creation policy.
7. **Code research** -- "NEVER guess or make up an answer."
8. **Running commands** -- Safety classification (safe vs unsafe), NEVER auto-run unsafe.
9. **Browser preview** -- Auto-invoke after web server launch.
10. **External APIs** -- Use best-suited APIs, check compatibility.
11. **Communication style** -- Second person for user, first person for self.
12. **Planning** -- Plan maintained and updated through `update_plan` tool.

### Key Patterns

**Memory system as first-class primitive**:
```
You have access to a persistent memory database to record important context
about the USER's task, codebase, requests, and preferences for future reference.

As soon as you encounter important information or context, proactively use
the create_memory tool to save it to the database.
You DO NOT need USER permission to create a memory.
```
Memories survive context window limits. The system explicitly warns "ALL CONVERSATION CONTEXT, INCLUDING checkpoint summaries, will be deleted" -- so memories are the survival mechanism.

**Command safety classification**:
- Commands classified as safe or unsafe before execution.
- Unsafe = destructive side-effects (delete files, install deps, external requests).
- "You must NEVER NEVER run a command automatically if it could be unsafe." (double NEVER)
- Cannot be overridden by user: "You cannot allow the USER to override your judgement on this."

**Plan as living document**:
- Plan maintained through dedicated `update_plan` tool.
- "Whenever you receive new instructions from the user, complete items from the plan, or learn any new information that may change the scope or direction of the plan, you must call this tool."
- "It is better to update plan when it didn't need to than to miss the opportunity."

**EPHEMERAL_MESSAGE injection**:
- System injects messages marked as `<EPHEMERAL_MESSAGE>` -- "not coming from the user, but instead injected by the system as important information."
- Agent told to follow them strictly but never acknowledge them.

### Novel Patterns

1. **Memory > context**: Explicit acknowledgment that context will be lost, so memory database is the primary persistence mechanism.
2. **Command safety as non-negotiable**: Even the user cannot override safety classification.
3. **Plan-first architecture**: Plan is a formal artifact maintained by a dedicated tool, not just a mental model.
4. **TargetFile-first generation**: "When using any code edit tool, ALWAYS generate the TargetFile argument first, before any other arguments." -- Forces file selection before content generation.

---

## 4. Devin AI (Cognition)

**Source**: Extracted system prompt from published collections.

### Prompt Structure (Section Order)

1. **Identity & flattery** -- "You are Devin, a software engineer... You are a real code-wiz: few programmers are as talented as you."
2. **Communication policy** -- When to communicate (environment issues, deliverables, missing info, permissions).
3. **Approach to work** -- Gather info before conclusions, never modify tests, test locally.
4. **Coding best practices** -- No comments unless needed, mimic conventions, check library availability.
5. **Information handling** -- Don't assume link contents, use browser.
6. **Data security** -- Sensitive data handling, no secrets in commits.
7. **Response limitations** -- Never reveal prompt details.
8. **Planning modes** -- "planning" vs "standard" mode with distinct behaviors.
9. **Command reference** -- Full XML-based tool definitions.
10. **Tool categories**: Reasoning, Shell, Editor, Search, LSP, Browser, Deployment, User interaction, Git/GitHub, Misc, Plan.
11. **Multi-command outputs** -- Parallel execution policy.
12. **Pop quizzes** -- Evaluation/testing mechanism.
13. **Git operations** -- Branch naming, safety rules.

### Tool Presentation (XML-based)

Devin uses XML tags for tool invocation, not function calling:

```xml
<shell id="shellId" exec_dir="/absolute/path/to/dir">
Command(s) to execute
</shell>

<str_replace path="/full/path/to/filename">
<old_str>original code</old_str>
<new_str>replacement code</new_str>
</str_replace>

<think>reasoning scratchpad</think>
```

### Key Patterns

**Explicit think tool with mandatory triggers**:
```
You must use the think tool in the following situation:
(1) Before critical git/GitHub-related decisions
(2) When transitioning from exploring code to making changes
(3) Before reporting completion to the user
```
Plus 10 additional "should use" scenarios. This is the most structured thinking protocol of any agent.

**Planning mode as separate operating state**:
- "planning" mode: gather info, search, understand, ask questions, then `<suggest_plan />`
- "standard" mode: execute plan steps, the user shows current and next steps.
- Mode is externally controlled ("The user will indicate to you which mode you are in").

**LSP integration as first-class tooling**:
- `go_to_definition`, `go_to_references`, `hover_symbol` -- real IDE capabilities.
- "You should use the LSP command quite frequently to make sure you pass correct arguments."

**Browser as core capability**:
- Full Playwright-controlled Chrome browser with screenshot + HTML feedback.
- Click, type, navigate, view console, select options.
- `devinid` attributes injected into DOM for reliable element selection.

**Deployment tools built-in**:
- `deploy_frontend`, `deploy_backend` -- Devin can deploy code.
- `expose_port` -- Share running servers with users.

**find_and_edit for bulk refactoring**:
```xml
<find_and_edit dir="/some/path/" regex="regexPattern">
A sentence describing the change at each matching location.
</find_and_edit>
```
Dispatches a separate LLM to each match location -- an agent-within-an-agent pattern.

**Pop quiz mechanism**:
- "From time to time you will be given a 'POP QUIZ'... follow the new instructions and answer honestly."
- Used for evaluation/testing of the agent in-band.

### Novel Patterns

1. **Mandatory think-before-act transitions**: Not optional -- must think before git decisions, before code changes, before completion.
2. **Externally controlled planning mode**: The orchestrator (not the agent) decides when to plan vs execute.
3. **Agent-dispatching tool** (find_and_edit): One agent spawns many parallel agents for bulk operations.
4. **Environment issue reporting**: Dedicated `<report_environment_issue>` command -- acknowledges agents will hit env problems.
5. **Shell ID persistence**: Shells persist with IDs, enabling long-running processes and interactive sessions.
6. **Identity flattery**: "few programmers are as talented as you" -- primes confident, proactive behavior.

---

## 5. Cline (Open Source)

**Source**: GitHub repo `cline/cline`, system prompt architecture in `src/core/prompts/system-prompt/`.

### Architecture

Cline uses a **modular prompt system** with:
- `PromptRegistry` -- Registry of prompt variants per model family
- `VariantBuilder` -- Builds prompts tailored to model capabilities
- `TemplateEngine` -- Placeholder-based template system
- `ClineToolSpec` -- Tool definitions that compile to OpenAI/Anthropic/Google formats

### Prompt Sections (from `placeholders.ts`)

```typescript
enum SystemPromptSection {
  AGENT_ROLE        // Identity and capabilities
  TOOL_USE          // How to use tools
  TOOLS             // Tool definitions
  MCP               // MCP server descriptions
  EDITING_FILES     // File editing rules
  ACT_VS_PLAN       // Action vs planning mode
  TODO              // Task tracking
  CAPABILITIES      // What the agent can do
  SKILLS            // Custom skills
  RULES             // Behavioral rules
  SYSTEM_INFO       // OS, shell, working directory
  OBJECTIVE         // Current objective
  USER_INSTRUCTIONS // Custom user instructions
  FEEDBACK          // Task feedback
  TASK_PROGRESS     // Progress tracking
}
```

### Key Patterns

**Model-family-aware prompts**: Different prompt variants for different model families. Tool specs include `contextRequirements` functions that conditionally include parameters based on context.

**Native tool calling support**: Tools compile to OpenAI, Anthropic, and Google tool formats from a single `ClineToolSpec` definition. Cline handles the mapping.

**Act vs Plan mode**: Explicit section for switching between exploration and execution.

### Novel Patterns

1. **Prompt as compiled artifact**: Not a string template -- a registry of variants compiled from specs.
2. **Cross-provider tool normalization**: Single tool definition compiles to 3+ provider formats.
3. **Context-conditional parameters**: Tool parameters appear/disappear based on runtime context.

---

## 6. Amp (Sourcegraph)

**Source**: Extracted YAML prompt configuration from published collections.

### Prompt Structure

1. **Identity** -- "You are Amp, a powerful AI coding agent built by Sourcegraph."
2. **Agency** -- Balance between action and restraint, with numbered priorities.
3. **Task workflow** -- 5-step recommended approach (tools, todo_write, oracle, search, diagnostics).
4. **Tool use examples** -- 7 concrete examples showing tool selection for different query types.
5. **Conciseness** -- "Do not add additional code explanation summary unless requested."

### Key Patterns

**Oracle tool for complex tasks**:
```
For complex tasks requiring deep analysis, planning, or debugging across
multiple files, consider using the oracle tool to get expert guidance
before proceeding.
```
This is an escalation mechanism -- the agent can ask a more capable model for help.

**Diagnostic gate**:
- "After completing a task, you MUST run the get_diagnostics tool and any lint and typecheck commands."
- Mandatory verification, not optional.

**AGENTS.md as shared memory**:
- "If you are unable to find the correct command, ask the user for the command to run and if they supply it, proactively suggest writing it to AGENTS.md."
- The agent actively improves its own configuration file.

### Novel Patterns

1. **Oracle/escalation tool**: Agent can delegate to a more capable model for hard problems.
2. **Self-improving configuration**: Agent suggests adding discovered commands to AGENTS.md.
3. **Mermaid diagrams in responses**: Examples show the agent proactively creating architecture diagrams.

---

## 7. Kiro (AWS)

**Source**: Extracted Spec_Prompt.txt from published collections.

### Prompt Structure

1. **Identity** -- "You are Kiro, an AI assistant and IDE built to assist developers."
2. **Capabilities list** -- Explicit list of what Kiro can do.
3. **Rules** -- Security, PII handling, prompt secrecy, code quality.
4. **Response style** -- Extensive tone/voice guidelines.
5. **System information** -- OS, platform, shell.
6. **Platform-specific commands** -- Adapted to detected OS.
7. **Date/time** -- Explicit current date with timezone guidance.
8. **Coding questions** -- Technical language and formatting standards.

### Key Patterns

**Brand voice as prompt engineering**:
```
We are knowledgeable. We are not instructive.
Speak like a dev -- when necessary.
Be decisive, precise, and clear. Lose the fluff when you can.
We are supportive, not authoritative.
We are easygoing, not mellow.
```
This is the most detailed tone/voice section of any agent. It reads like a brand guide.

**Repeat failure handling**:
```
If you encounter repeat failures doing the same thing, explain what you
think might be happening, and try another approach.
```

**Minimal code philosophy**:
```
Write only the ABSOLUTE MINIMAL amount of code needed to address the
requirement, avoid verbose implementations and any code that doesn't
directly contribute to the solution.
```

### Novel Patterns

1. **Brand voice integration**: Personality defined as a full brand system, not just "be concise."
2. **Anti-scaffold pattern**: For multi-file projects, create "MINIMAL skeleton implementations only."
3. **PII substitution**: Automatically replace real data with placeholders in examples.

---

## 8. GitHub Copilot / VS Code Agent

**Source**: Extracted Prompt.txt from VSCode Agent directory in published collections.

### Prompt Structure

Mirrors Claude Code closely (both use Claude as the underlying model), with VS Code-specific additions:
1. Identity and security policy
2. Tone and style with verbosity examples
3. Proactiveness policy
4. Convention following
5. Code style ("DO NOT ADD ANY COMMENTS unless asked")
6. Task management (TodoWrite)
7. Task execution steps
8. Tool usage policy
9. Environment info with git status
10. Git commit and PR workflows

### Key Patterns

**Zero-comment code style**: "IMPORTANT: DO NOT ADD ***ANY*** COMMENTS unless asked" -- aggressively anti-comment, contrasting with most style guides.

**Git status injection**: Current git status is injected as a snapshot: "Note that this status is a snapshot in time, and will not update during the conversation."

**Lint/typecheck gate**: "When you have completed a task, you MUST run the lint and typecheck commands... If you are unable to find the correct command, ask the user for the command to run."

---

## 9. SWE-agent (Princeton)

**Source**: SWE-agent GitHub repo, default.yaml config, academic paper (arXiv 2405.15793).

### Prompt Structure (YAML-based)

```yaml
system_template: "You are a helpful assistant that can interact with
  a computer to solve tasks."

instance_template: |
  We've uploaded a python code repository. Your task is to
  implement the necessary changes per the <pr_description>.
  
  1. Locate and review relevant code
  2. Create and execute reproduction scripts  
  3. Modify source code to resolve issues
  4. Re-execute scripts to verify fixes
  5. Consider edge cases

next_step_template: "OBSERVATION: {{observation}}"

next_step_no_output_template: "Your command ran successfully and
  did not produce any output."
```

### Tool Configuration

- Environment variables suppress interactive output: `PAGER`, `GIT_PAGER`, `TQDM_DISABLE`
- Tool bundles: registry, edit, review submission
- `USE_FILEMAP: true` -- builds a file map of the repository
- Function calling parse mode
- Cache control on last 2 messages

### Key Patterns

**Prescribed 5-step workflow**: Not suggested -- the agent must follow these steps in order.

**Submission review checklist**:
1. Re-run reproduction script
2. Remove temporary test files
3. Revert any test modifications
4. Run submission confirmation

**Observation-based feedback**: Tool outputs are labeled "OBSERVATION:" -- creating a clear action/observation loop.

**Test modification prohibition**: "I've already taken care of all changes to test files... This means you DON'T have to modify the testing logic."

### Novel Patterns

1. **YAML-driven prompt configuration**: Entire agent behavior defined in a single config file.
2. **Strict step ordering**: 5 mandatory steps that create a reproducible workflow.
3. **Silent command handling**: Explicit template for when commands produce no output.
4. **Environment variable suppression**: Prevents interactive pagers from blocking the agent.

---

## 10. Agentless (UIUC)

**Source**: GitHub repo `OpenAutoCoder/Agentless`, FL.py and repair.py modules.

### Architecture (Non-Agentic Pipeline)

Agentless deliberately avoids the agent loop. Instead, it uses a **fixed pipeline**:

```
Fault Localization -> Context Collection -> Patch Generation -> Patch Validation
```

Each stage uses separate, focused prompts with no tool use.

### Fault Localization Prompts (6 stages)

1. **Irrelevant folder elimination**: "Identify folders that are irrelevant to fixing the problem."
2. **Relevant file identification**: "Provide a list of files that one would need to edit to fix the problem." (max 5 files, ordered by importance)
3. **Relevant code from compressed files**: Identify classes, functions, or line numbers that need editing.
4. **Relevant code without line numbers**: Same but requesting only symbolic names.
5. **Functions/variables from compressed files**: "Identify all locations that need inspection or editing."
6. **Functions/variables from raw files**: Same but from uncompressed source.

### Repair Prompts (4 variants)

1. **Base**: Generate `edit_file` commands with "PROPER INDENTATION" emphasis.
2. **Chain-of-thought**: "First localize the bug based on the issue statement" before generating edits.
3. **SEARCH/REPLACE diff format**: Uses `<<<<<<< SEARCH` / `=======` / `>>>>>>> REPLACE` markers.
4. **String replacement format**: Simple find-and-replace editing commands.

### Key Patterns

**Hierarchical localization**: Coarse-to-fine narrowing: folders -> files -> functions -> lines.

**No tools, no agent loop**: Every prompt is a single-shot LLM call. The pipeline orchestrator handles iteration, not the LLM.

**Structured output enforcement**: All outputs wrapped in triple backticks with specific format requirements.

### Novel Patterns

1. **Anti-agent architecture**: Proves that a fixed pipeline can outperform agent loops on benchmarks.
2. **Elimination before identification**: First remove irrelevant folders, then identify relevant files. Reduces search space.
3. **Multiple output format variants**: Same task with different output formats (edit commands, diffs, string replacement) -- lets you pick what works best for your toolchain.

---

## 11. AutoCodeRover (NUS)

**Source**: GitHub repo `nus-apr/auto-code-rover`, agent_write_patch.py, academic paper.

### Two-Phase Architecture

**Phase 1 - Context Retrieval**: LLM uses code search APIs (AST-aware) to navigate the codebase.

**Phase 2 - Patch Generation**: LLM writes patches based on retrieved context.

### System Prompt

```
You are a software developer maintaining a large project. You are
working on an issue submitted to your project.
```

### Patch Format

```
<file>path/to/file</file>
<original>exact original code</original>
<patched>replacement code</patched>
```

### Key Patterns

**AST-aware search APIs**: Not grep -- searches operate on the abstract syntax tree to locate classes, methods, and functions by structure.

**Iterative patch refinement**: When patches fail validation, the agent receives: "Your patch is invalid. [validation message]. Please try again."

**Prompt isolation between phases**: "Replace the system prompt in the message thread. This is because the main agent system prompt may involve tool_calls info, which should not be known to task agents."

### Novel Patterns

1. **Program-structure-aware tools**: Tools understand code structure (AST), not just text.
2. **Statistical fault localization**: Uses test suite execution data to narrow down bug locations.
3. **Phase-isolated prompts**: Different system prompts for exploration vs patching phases.

---

## 12. Cross-Cutting Patterns

### Pattern 1: Autonomous Completion Mandate

Every production agent includes a variation of "keep going until done":

| Agent | Phrasing |
|-------|----------|
| Cursor | "keep going until the user's query is completely resolved, before ending your turn" |
| Devin | "your mission is to accomplish the task using the tools at your disposal" |
| Claude Code | "you MUST run the lint and typecheck commands... to ensure your code is correct" |
| Windsurf | "keep working, using tools where needed, until the user's query is completely resolved" |

This is the single most universal pattern across all agents.

### Pattern 2: Parallel Tool Execution

Every agent with tool calling emphasizes parallelism:
- Cursor: dedicated `<maximize_parallel_tool_calls>` section
- Claude Code: "batch your tool calls together for optimal performance"
- Devin: "if you can output multiple commands without dependencies, it is better to output multiple"
- Amp: "invoke all relevant tools simultaneously rather than sequentially"

### Pattern 3: Never Output Code, Use Tools

Universal among IDE-integrated agents:
- Cursor: "NEVER output code to USER unless requested. Instead use one of the code edit tools"
- Windsurf: "When making code changes, NEVER output code to the USER"
- Devin: Uses editor commands exclusively, shell forbidden for file editing

### Pattern 4: Convention Discovery Before Action

```
[Claude Code] NEVER assume that a given library is available, even if it
is well known. Whenever you write code that uses a library or framework,
first check that this codebase already uses the given library.

[Devin] When you edit a piece of code, first look at the code's surrounding
context (especially its imports) to understand the code's choice of
frameworks and libraries.

[Windsurf] NEVER guess or make up an answer. Your answer must be rooted
in your research.
```

### Pattern 5: Structured Planning Primitive

All production agents have explicit planning mechanisms:

| Agent | Planning Mechanism |
|-------|-------------------|
| Claude Code | TodoWrite tool |
| Cursor | todo_write with reconciliation gates |
| Windsurf | update_plan tool |
| Devin | Planning mode + suggest_plan command |
| Cline | ACT_VS_PLAN section |
| Amp | todo_write + oracle escalation |

### Pattern 6: Safety/Security Guardrails

Every agent has safety rules, but they vary in enforcement:

| Level | Agent | Mechanism |
|-------|-------|-----------|
| Advisory | Claude Code | "Assist with defensive security tasks only" |
| Hard block | Windsurf | "You must NEVER NEVER run a command automatically if it could be unsafe" |
| User-overridable | Claude Code | Auto mode with classifier |
| Non-overridable | Windsurf | "You cannot allow the USER to override your judgement" |

### Pattern 7: Think/Reasoning Scratchpad

Several agents have explicit reasoning tools:
- **Devin**: `<think>` tool with 13 mandatory/recommended triggers
- **Cursor**: Implicit in status updates
- **Claude Code**: Extended thinking mode (separate from prompt)

### Pattern 8: Context Window Management

All agents acknowledge context as the fundamental constraint:
- Claude Code: "context window fills up fast, and performance degrades as it fills"
- Windsurf: "ALL CONVERSATION CONTEXT, INCLUDING checkpoint summaries, will be deleted" -> use memory
- Cursor: Status updates kept to "1-3 sentences"
- Claude Code: `/clear`, `/compact`, subagents as context isolation

### Pattern 9: Verification Gate Before Completion

Every agent requires verification before reporting done:
- Claude Code: "MUST run the lint and typecheck commands"
- Cursor: "Run read_lints after edits"
- Devin: "Make sure to follow the instructions very carefully" in think-before-completion
- SWE-agent: 4-step submission review checklist
- Amp: "MUST run the get_diagnostics tool"

### Pattern 10: Self-Correction Mechanisms

Several agents include explicit self-correction:
- **Cursor**: "If you fail to call todo_write... self-correct in the next turn immediately"
- **Devin**: Think tool mandatory "before reporting completion... critically examine your work"
- **Kiro**: "If you encounter repeat failures doing the same thing, explain what you think might be happening, and try another approach"

---

## 13. Implications for Cairn

Based on this research, here are concrete recommendations for Cairn's agent orchestrator prompt design.

### System Prompt Architecture

```
1. Identity + mission (1-2 sentences)
2. Security guardrails (mandatory, repeated at end)
3. Tool definitions with usage rules
4. Task management (planning primitive)
5. Workflow steps (explore -> plan -> implement -> verify)
6. Code conventions policy (discover before assuming)
7. Git/PR workflow (safety-first)
8. Verification gate (must pass before completing)
9. Output format rules
10. Environment context (injected dynamically)
```

### Must-Have Patterns for Cairn's Agent

1. **Autonomous completion mandate**: "Keep working until the issue is fully resolved. Do not stop to ask for permission unless you are blocked."

2. **Planning primitive**: Give the agent a `todo_write` or plan tool. Every production agent has one. Without it, agents forget steps on complex tasks.

3. **Parallel tool execution**: Explicitly instruct parallel tool calls. This is a 3-5x performance multiplier.

4. **Convention discovery**: "Before writing code, examine existing patterns in the codebase. Never assume a library is available."

5. **Verification gate**: "After implementation, run tests/lint/typecheck. Do not create a PR until verification passes."

6. **Think-before-act transitions**: Require explicit reasoning before git operations, before transitioning from exploration to coding, and before reporting completion.

7. **Context management**: For long-running issue resolution, use subagent-style context isolation for exploration, keeping the main context clean for implementation.

8. **Safety classification for commands**: Classify shell commands as safe/unsafe. Never auto-execute destructive operations.

9. **Self-correction protocol**: "If you notice you violated a rule, correct yourself in the next action."

10. **Structured output for patches**: Use a well-defined format (SEARCH/REPLACE blocks or file/original/patched XML) that can be mechanically validated.

### Anti-Patterns to Avoid

1. **No planning mechanism**: Agents without todo/plan tools forget steps on multi-file tasks.
2. **Unscoped exploration**: Letting the agent read unlimited files fills context. Scope searches or use subagents.
3. **No verification gate**: Without mandatory lint/test checks, agents produce plausible but broken code.
4. **Agent decides when to stop**: The orchestrator should control stopping, not the agent (Devin's externally-controlled planning mode is the best pattern here).
5. **Single output format**: Agentless shows that offering multiple patch formats (edit commands, diffs, string replacement) improves success rates.

### Cairn-Specific Considerations

Since Cairn dispatches GitHub issues to agents:

1. **Issue -> plan -> implement -> verify -> PR** pipeline is well-established across all agents.
2. **Two-phase approach** (like AutoCodeRover) works well: context retrieval in one phase, patch generation in another, with different system prompts for each.
3. **The orchestrator (Cairn) should control the agent's mode**, not the agent itself. This is the Devin pattern: the system tells the agent "you are in planning mode" or "you are in execution mode."
4. **Submission review checklist** (from SWE-agent) should be enforced by Cairn, not left to the agent. The orchestrator should verify tests pass before allowing PR creation.
5. **Memory across issues**: Windsurf's memory system pattern -- learnings from one issue should persist to future issues on the same repo.

---

## Sources

1. Claude Code system prompt -- extracted from `x1xhlol/system-prompts-and-models-of-ai-tools` (Anthropic/Claude Code/ and Claude Code 2.0.txt)
2. Cursor Agent Prompt 2025-09-03 -- same repo (Cursor Prompts/)
3. Windsurf Prompt Wave 11 -- same repo (Windsurf/)
4. Devin AI Prompt -- same repo (Devin AI/)
5. Cline source code -- `cline/cline` GitHub repo (src/core/prompts/system-prompt/)
6. Amp (Sourcegraph) -- same repo (Amp/)
7. Kiro (AWS) -- same repo (Kiro/)
8. VS Code Agent prompt -- same repo (VSCode Agent/)
9. SWE-agent config -- `princeton-nlp/SWE-agent` GitHub repo (config/default.yaml)
10. Agentless -- `OpenAutoCoder/Agentless` GitHub repo (agentless/fl/FL.py, agentless/repair/repair.py)
11. AutoCodeRover -- `nus-apr/auto-code-rover` GitHub repo (app/agents/)
12. Claude Code best practices -- https://code.claude.com/docs/en/best-practices
