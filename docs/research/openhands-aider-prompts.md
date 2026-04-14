# Agent Prompt Architecture Research: OpenHands, Aider, SWE-agent, Devin

Research conducted 2026-04-14 from actual source code in GitHub repos.

---

## Table of Contents

1. [OpenHands (CodeAct Agent)](#1-openhands-codeact-agent)
2. [Aider](#2-aider)
3. [SWE-agent](#3-swe-agent)
4. [Devin](#4-devin)
5. [Cross-Cutting Patterns](#5-cross-cutting-patterns)
6. [Design Recommendations for Cairn](#6-design-recommendations-for-cairn)

---

## 1. OpenHands (CodeAct Agent)

Source: `All-Hands-AI/OpenHands` (V0 legacy codebase, V1 migrating to Software Agent SDK)

### 1.1 System Prompt Structure

The system prompt is a Jinja2 template (`system_prompt.j2`) composed of clearly tagged XML-style sections. Each section is a self-contained behavioral module:

```
<ROLE> ... </ROLE>
<EFFICIENCY> ... </EFFICIENCY>
<FILE_SYSTEM_GUIDELINES> ... </FILE_SYSTEM_GUIDELINES>
<CODE_QUALITY> ... </CODE_QUALITY>
<VERSION_CONTROL> ... </VERSION_CONTROL>
<PULL_REQUESTS> ... </PULL_REQUESTS>
<PROBLEM_SOLVING_WORKFLOW> ... </PROBLEM_SOLVING_WORKFLOW>
<SECURITY> ... </SECURITY>
<SECURITY_RISK_ASSESSMENT> ... </SECURITY_RISK_ASSESSMENT>
<EXTERNAL_SERVICES> ... </EXTERNAL_SERVICES>
<ENVIRONMENT_SETUP> ... </ENVIRONMENT_SETUP>
<TROUBLESHOOTING> ... </TROUBLESHOOTING>
<DOCUMENTATION> ... </DOCUMENTATION>
<PROCESS_MANAGEMENT> ... </PROCESS_MANAGEMENT>
```

The opening line is minimal:
```
You are OpenHands agent, a helpful AI assistant that can interact with a computer to solve tasks.
```

#### Key Role Definition
```
Your primary role is to assist users by executing commands, modifying code, and solving
technical problems effectively. You should be thorough, methodical, and prioritize quality
over speed.
* If the user asks a question, like "why is X happening", don't try to fix the problem.
  Just give an answer to the question.
```

This is notable -- they explicitly separate "answering questions" from "taking action" to prevent the agent from doing unwanted modifications when a user just wants information.

### 1.2 Problem-Solving Workflow (5-Step)

This is the core cognitive loop, embedded directly in the system prompt:

```
1. EXPLORATION: Thoroughly explore relevant files and understand the context
   before proposing solutions
2. ANALYSIS: Consider multiple approaches and select the most promising one
3. TESTING:
   * For bug fixes: Create tests to verify issues before implementing fixes
   * For new features: Consider test-driven development when appropriate
   * Do NOT write tests for documentation changes, README updates, config files
   * If the repo lacks testing infrastructure, consult with the user first
4. IMPLEMENTATION:
   * Make focused, minimal changes to address the problem
   * Always modify existing files directly rather than creating new versions
   * Delete temporary files after confirming solution works
5. VERIFICATION: Test implementation thoroughly, including edge cases
```

### 1.3 Anti-Give-Up Mechanism (Troubleshooting Section)

```
If you've made repeated attempts to solve a problem but tests still fail:
  1. Step back and reflect on 5-7 different possible sources of the problem
  2. Assess the likelihood of each possible cause
  3. Methodically address the most likely causes, starting with the highest probability
  4. Document your reasoning process
```

And critically:
```
When you run into any major issue while executing a plan from the user, please don't try
to directly work around it. Instead, propose a new plan and confirm with the user before
proceeding.
```

### 1.4 Tool (Action Space) Design

OpenHands uses **function calling** (OpenAI-style tool use) with these tools:

| Tool | Purpose | Key Design Choice |
|------|---------|-------------------|
| `execute_bash` | Shell commands | Persistent session, soft 10s timeout, `is_input` for stdin interaction, `C-c`/`C-d` control |
| `str_replace_editor` | File view/create/edit | 5 subcommands: `view`, `create`, `str_replace`, `insert`, `undo_edit`. EXACT match required. |
| `think` | Structured reasoning | Logged but no side effects. For brainstorming, debugging hypotheses, architecture decisions. |
| `task_tracker` | Progress tracking | `view` and `plan` commands. todo/in_progress/done states. ONE active task at a time. |
| `finish` | Task completion | Must include summary of actions and results. |
| `request_condensation` | Memory management | Agent can request its own context to be compressed. |
| `edit_file` (LLM-based) | Fuzzy file editing | Can use `# ... existing code ...` to skip unchanged sections. For large files, uses line ranges. |
| `browser` | Web interaction | BrowseInteractiveAction for web pages. |

#### Bash Tool -- Detailed Description (verbatim from source)

```
Execute a bash command in the terminal within a persistent shell session.

### Command Execution
* One command at a time: You can only execute one bash command at a time.
  If you need to run multiple commands sequentially, use && or ; to chain them.
* Persistent session: Commands execute in a persistent shell session where environment
  variables, virtual environments, and working directory persist between commands.
* Soft timeout: Commands have a soft timeout of 10 seconds...

### Long-running Commands
* For commands that may run indefinitely, run them in the background and redirect
  output to a file, e.g. python3 app.py > server.log 2>&1 &
* For commands that may run for a long time, set the "timeout" parameter

### Best Practices
* Directory verification: Before creating new directories or files, first verify the
  parent directory exists and is the correct location.
* Directory management: Try to maintain working directory by using absolute paths

### Output Handling
* Output truncation: If the output exceeds a maximum length, it will be truncated
```

#### str_replace_editor -- Critical Requirements (verbatim)

```
CRITICAL REQUIREMENTS FOR USING THIS TOOL:

1. EXACT MATCHING: The old_str parameter must match EXACTLY one or more consecutive lines
   from the file, including all whitespace and indentation.

2. UNIQUENESS: The old_str must uniquely identify a single instance in the file:
   - Include sufficient context before and after the change point (3-5 lines recommended)

3. REPLACEMENT: The new_str parameter should contain the edited lines that replace the
   old_str. Both strings must be different.

Remember: when making multiple file edits in a row to the same file, you should prefer to
send all edits in a single message with multiple calls to this tool, rather than multiple
messages with a single call each.
```

#### Think Tool (verbatim)

```
Use the tool to think about something. It will not obtain new information or make any
changes to the repository, but just log the thought. Use it when complex reasoning or
brainstorming is needed.

Common use cases:
1. When exploring a repository and discovering the source of a bug, call this tool to
   brainstorm several unique ways of fixing the bug, and assess which change(s) are likely
   to be simplest and most effective.
2. After receiving test results, use this tool to brainstorm ways to fix failing tests.
3. When planning a complex refactoring, use this tool to outline different approaches and
   their tradeoffs.
4. When designing a new feature, use this tool to think through architecture decisions.
5. When debugging a complex issue, use this tool to organize your thoughts and hypotheses.
```

### 1.5 Security Risk Assessment

Every tool call includes a mandatory `security_risk` parameter (LOW/MEDIUM/HIGH):

```
- LOW: Read-only actions inside sandbox. Inspecting files, calculations, viewing docs.
- MEDIUM: Container-scoped edits and installs. Modify workspace files, install packages,
  run user code.
- HIGH: Data exfiltration or privilege breaks. Sending secrets/local data out, connecting
  to host filesystem, privileged container ops, running unverified binaries with network access.

Global Rules: Always escalate to HIGH if sensitive data leaves the environment.
```

### 1.6 Task Tracker (Long-Horizon Anti-Drift)

The long-horizon prompt variant adds a `<TASK_MANAGEMENT>` section requiring the agent to decompose work into trackable items. Key rules:

```
* Use task_tracker REGULARLY to maintain task visibility
* Update task status to "done" immediately upon completion of each work item.
  Do not accumulate multiple finished tasks before updating.
* For complex, multi-phase development work:
  1. Begin by decomposing the overall objective into primary phases
  2. Include detailed work items as necessary
  3. Update tasks to "in_progress" when commencing work
  4. Update tasks to "done" immediately after completing each item
  5. If you determine the plan requires substantial modifications, suggest revisions
     and obtain user confirmation before proceeding
```

### 1.7 In-Context Learning (Few-Shot Example)

OpenHands includes a single long worked example showing the full Observe-Think-Act cycle:

```
USER: Create a list of numbers from 1 to 10, and display them in a web page at port 5000.

A: Sure! Let me first check the current directory:
<function=execute_bash>
<parameter=command>pwd && ls</parameter>
</function>

USER: EXECUTION RESULT of [execute_bash]:
/workspace

A: There is no app.py file in the current directory. Let me create a Python file:
<function=str_replace_editor>
<parameter=command>create</parameter>
<parameter=path>/workspace/app.py</parameter>
<parameter=file_text>
from flask import Flask
app = Flask(__name__)
...
</parameter>
</function>

[...continues through error, dependency install, verification, modification, restart...]

<function=finish>
</function>
```

The example demonstrates: exploration first, error handling (missing flask), dependency installation, verification, modification cycle, and clean finish. The suffix enforces:

```
PLEASE follow the format strictly!
PLEASE EMIT ONE AND ONLY ONE FUNCTION CALL PER MESSAGE.
```

### 1.8 Context Injection Templates

Dynamic context is injected via additional Jinja2 templates:

- **`additional_info.j2`**: Repository info (name, directory, branch), runtime info (working dir, available hosts, date), custom secrets, conversation instructions
- **`microagent_info.j2`**: Triggered micro-agent knowledge injected based on keyword matching: `The following information has been included based on a keyword match for "{{ agent_info.trigger }}".`
- **`security_risk_assessment.j2`**: Risk policy (different for CLI mode vs sandbox mode)

### 1.9 Memory Condensation System

OpenHands has a sophisticated memory system for long conversations:

- **Condenser base class**: Abstract interface that takes event history and produces a compressed view
- **Implementations**: `amortized_forgetting_condenser`, `browser_output_condenser`, `conversation_window_condenser`, `llm_attention_condenser`
- **Agent-initiated**: The agent can call `request_condensation` tool when context gets too long
- **Persistence across condensation**: Task tracker state is preserved: "If you were using the task_tracker tool before a condensation event, continue using it after condensation"

### 1.10 Prompt Variants

OpenHands ships multiple system prompt variants composed via Jinja2 `{% include %}`:

| Variant | Adds |
|---------|------|
| `system_prompt.j2` | Base prompt with all sections |
| `system_prompt_long_horizon.j2` | Base + `<TASK_MANAGEMENT>` + `<TASK_TRACKING_PERSISTENCE>` |
| `system_prompt_interactive.j2` | Base + `<INTERACTION_RULES>` (explore before implementing, validate file existence, multilingual support) |
| `system_prompt_tech_philosophy.j2` | Base + Linus Torvalds engineering mindset (simplicity, backward compat, pragmatism, 5-layer analysis) |

---

## 2. Aider

Source: `paul-gauthier/aider`

### 2.1 Architecture: Fundamentally Different from OpenHands

Aider is NOT an autonomous agent loop. It is a **conversational code editor** that:
1. Receives a user request
2. Sends repo context + request to an LLM in a single call (or Architect + Editor pair)
3. Parses structured edits from the response
4. Applies edits to files
5. Auto-commits with git
6. Optionally runs lint/test and reflects on failures

There is no observe-think-act loop. The LLM makes one pass (with optional reflection on lint/test errors).

### 2.2 System Prompt Structure

Aider's prompts are Python class hierarchies. The base `CoderPrompts` class defines slots that subclasses fill:

```python
class CoderPrompts:
    main_system = ""          # Core system prompt
    example_messages = []     # Few-shot examples (user/assistant pairs)
    system_reminder = ""      # Appended at end of context as refresher
    files_content_prefix = "" # Introduces editable files
    repo_content_prefix = ""  # Introduces read-only repo map
    lazy_prompt = ""          # Anti-laziness injection
    overeager_prompt = ""     # Anti-overreach injection
```

### 2.3 Edit Formats

Aider's key innovation is multiple edit format strategies:

#### SEARCH/REPLACE (EditBlock) -- Most Used Format

System prompt (verbatim):
```
Act as an expert software developer.
Always use best practices when coding.
Respect and use existing conventions, libraries, etc that are already present in the code base.

Take requests for changes to the supplied code.
If the request is ambiguous, ask questions.

Once you understand the request you MUST:
1. Decide if you need to propose *SEARCH/REPLACE* edits to any files that haven't been
   added to the chat. You can create new files without asking! But if you need to edit
   existing files not already added to the chat, you *MUST* tell the user their full path
   names and ask them to *add the files to the chat*.
2. Think step-by-step and explain the needed changes in a few short sentences.
3. Describe each change with a *SEARCH/REPLACE block* per the examples below.

All changes to files must use this *SEARCH/REPLACE block* format.
ONLY EVER RETURN CODE IN A *SEARCH/REPLACE BLOCK*!
```

Format rules (system_reminder):
```
Every *SEARCH/REPLACE block* must use this format:
1. The *FULL* file path alone on a line, verbatim.
2. The opening fence and code language, eg: ```python
3. The start of search block: <<<<<<< SEARCH
4. A contiguous chunk of lines to search for in the existing source code
5. The dividing line: =======
6. The lines to replace into the source code
7. The end of the replace block: >>>>>>> REPLACE
8. The closing fence: ```

Every *SEARCH* section must *EXACTLY MATCH* the existing file content, character for
character, including all comments, docstrings, etc.

*SEARCH/REPLACE* blocks will *only* replace the first match occurrence.

Keep *SEARCH/REPLACE* blocks concise.
Break large *SEARCH/REPLACE* blocks into a series of smaller blocks.
Include just the changing lines, and a few surrounding lines if needed for uniqueness.
```

#### Unified Diff Format

```
For each file that needs to be changed, write out the changes similar to a unified diff
like `diff -U0` would produce.
```

Key instruction:
```
When editing a function, method, loop, etc use a hunk to replace the *entire* code block.
Delete the entire existing version with `-` lines and then add a new, updated version
with `+` lines. This will help you generate correct code and correct diffs.
```

#### Whole File Format

The simplest: return the entire updated file. Used when models can't reliably produce diffs.
```
To suggest changes to a file you MUST return the entire content of the updated file.
*NEVER* skip, omit or elide content.
```

### 2.4 Few-Shot Examples (EditBlock)

Two worked examples are included in every request:

**Example 1: Refactoring** -- Change `get_factorial()` to use `math.factorial`
- Shows: import addition, function deletion (empty REPLACE), call-site update
- Three separate SEARCH/REPLACE blocks for one logical change

**Example 2: Extract to new file** -- Move `hello()` into its own file
- Shows: creating a new file (empty SEARCH section), removing from old file and adding import
- Demonstrates the new-file pattern

### 2.5 Anti-Laziness / Anti-Overreach Prompts

Aider injects one of two behavioral modifiers based on model characteristics:

**Lazy models** get:
```
You are diligent and tireless!
You NEVER leave comments describing code without implementing it!
You always COMPLETELY IMPLEMENT the needed code!
```

**Overeager models** get:
```
Pay careful attention to the scope of the user's request.
Do what they ask, but no more.
Do not improve, comment, fix or modify unrelated parts of the code in any way!
```

### 2.6 Repository Map (Tree-Sitter Based Context)

Aider's repo map is a critical innovation for large codebases:

- Uses **tree-sitter** to parse every file into ASTs
- Extracts tags: function definitions, class definitions, method signatures
- Builds a **dependency graph** between files based on cross-references
- Uses **PageRank-style graph ranking** to select the most relevant files/symbols
- Compresses into a token budget (default 1,024 tokens, configurable via `--map-tokens`)
- Output format: file paths + class declarations + method signatures

Example repo map output:
```
aider/coders/base_coder.py:
  class Coder:
    def create(self, main_model, edit_format, io, ...)
    def get_edits(self, mode="update")
    def apply_edits(self, edits)
    ...
```

The repo map is introduced to the LLM with:
```
Here are summaries of some files present in my git repository.
Do not propose changes to these files, treat them as *read-only*.
If you need to edit any of these files, ask me to *add them to the chat* first.
```

### 2.7 Chat Message Assembly (ChatChunks)

Aider assembles the LLM request as ordered message chunks:

```python
class ChatChunks:
    system          # System prompt(s)
    examples        # Few-shot examples (user/assistant pairs)
    done            # Previous conversation history
    repo            # Repository map/context
    readonly_files  # Reference files (read-only)
    chat_files      # User-added editable files
    cur             # Current user message
    reminder        # System reminder (rules refresher at end)
```

The `system_reminder` is placed at the **end** of the context window to take advantage of recency bias -- the model pays more attention to text near the end of its context.

### 2.8 Architect Mode (Two-Model Split)

Aider's architect mode separates reasoning from formatting:

1. **Architect model** (e.g., o1, Claude Opus): Receives the full context and describes the solution in natural language. No formatting constraints.
2. **Editor model** (e.g., Claude Sonnet, DeepSeek): Receives the architect's plan and converts it into properly formatted SEARCH/REPLACE blocks.

This achieves 85% on benchmarks (o1-preview + o1-mini). The insight: forcing a reasoning model to also follow strict edit formatting degrades both capabilities.

### 2.9 Infinite Output (Prefill Continuation)

For models that support it, Aider uses **response prefilling** to overcome output token limits:

1. If model output is truncated mid-edit, Aider starts a new request
2. The truncated output is injected as the assistant's prefix
3. The model continues generating from exactly where it stopped
4. Heuristics join text across boundaries

This enables arbitrarily long edit outputs without loss.

### 2.10 Lint-Test-Reflect Loop

After applying edits, Aider can:
1. Run the project's lint tool
2. Run the project's test suite
3. If either fails, send the error output back to the LLM as a new user message
4. The LLM produces corrective edits
5. Repeat

This is the closest Aider gets to an "agent loop" -- but it's bounded (typically 1-2 reflection rounds).

---

## 3. SWE-agent

Source: `SWE-agent/SWE-agent`

### 3.1 Architecture

SWE-agent is a true agent loop like OpenHands, but with a distinctive focus on **Agent-Computer Interface (ACI) design** -- the idea that tool ergonomics matter as much as prompt engineering.

System prompt is minimal:
```
You are a helpful assistant that can interact with a computer to solve tasks.
```

The real work is in the **instance template** (per-task prompt):

```
I've uploaded a python code repository in the directory {{working_dir}}.
Consider the following PR description:

<pr_description>
{{problem_statement}}
</pr_description>

Can you help me implement the necessary changes to the repository so that the
requirements specified in the <pr_description> are met?

I've already taken care of all changes to any of the test files described in the
<pr_description>. This means you DON'T have to modify the testing logic or any of
the tests in any way!

Your task is to make the minimal changes to non-tests files in the {{working_dir}}
directory to ensure the <pr_description> is satisfied.

Follow these steps to resolve the issue:
1. As a first step, it might be a good idea to find and read code relevant to the
   <pr_description>
2. Create a script to reproduce the error and execute it with `python <filename.py>`
   using the bash tool, to confirm the error
3. Edit the sourcecode of the repo to resolve the issue
4. Rerun your reproduce script and confirm that the error is fixed!
5. Think about edgecases and make sure your fix handles them as well

Your thinking should be thorough and so it's fine if it's very long.
```

### 3.2 ACI Design Principles

SWE-agent's core contribution is four ACI design principles:

1. **Syntax validation on edit**: A linter runs automatically after every edit command, providing immediate feedback on syntax errors
2. **Specialized file viewer**: Shows ~100 lines at a time with scroll/search navigation, optimized for agent comprehension (not `cat` which dumps everything)
3. **Succinct search results**: Lists only files containing matches, avoids overwhelming context
4. **Explicit empty output**: Commands that produce no output explicitly say so: `"Your command ran successfully and did not produce any output."`

### 3.3 Output Handling

SWE-agent truncates long outputs and tells the agent:
```
Observation: {{observation[:max_observation_length]}}<response clipped>
<NOTE>Observations should not exceeded {{max_observation_length}} characters.
{{elided_chars}} characters were elided. Please try a different command that produces
less output or use head/tail/grep/redirect the output to a file.
Do not use interactive pagers.</NOTE>
```

And for timeouts:
```
The command '{{command}}' was cancelled because it took more than {{timeout}} seconds.
Please try a different command that completes more quickly.
Note: A common source of this error is if the command is interactive or requires user
input (it is impossible to receive user input in the current environment, so the command
will never complete).
```

### 3.4 Observation Format

Simple template-based:
```
OBSERVATION:
{{observation}}
```

### 3.5 Submit Review (Anti-Premature-Submission)

SWE-agent includes a review step before submission:
```
1. If you made any changes to your code after running the reproduction script, please
   run the reproduction script again.
2. Remove your reproduction script (if you haven't done so already).
3. If you have modified any TEST files, please revert them.
4. Run the submit command again to confirm.

Here is a list of all of your changes:
<diff>
{{diff}}
</diff>
```

### 3.6 Configuration-Driven Tools

SWE-agent tools are defined in YAML configs with tool bundles:
```yaml
tools:
  bundles:
    - path: tools/registry
    - path: tools/edit_anthropic
    - path: tools/review_on_submit_m
  enable_bash_tool: true
  parse_function:
    type: function_calling
```

Environment is carefully tuned:
```yaml
env_variables:
  PAGER: cat
  MANPAGER: cat
  LESS: -R
  PIP_PROGRESS_BAR: 'off'
  TQDM_DISABLE: '1'
  GIT_PAGER: cat
```

### 3.7 History Processing

```yaml
history_processors:
  - type: cache_control
    last_n_messages: 2
```

Only the last 2 messages get Anthropic-style cache control headers, keeping cost down while maintaining recent context freshness.

---

## 4. Devin

Source: Cognition AI (closed source), information from public blog posts and benchmarks.

### 4.1 Architecture

Devin uses a sandboxed environment with three core tools:
- **Shell** (terminal)
- **Code editor**
- **Browser**

It is built on "advances in long-term reasoning and planning" and can "plan and execute complex engineering tasks requiring thousands of decisions."

### 4.2 Key Capabilities

- **Context recall**: "recall relevant context at every step"
- **Learning**: "learn over time, and fix mistakes"
- **Real-time collaboration**: "reports on its progress in real time, accepts feedback, and works together with you through design choices"

### 4.3 SWE-bench Performance

Devin achieved 13.86% on SWE-bench (at time of launch), vs prior SOTA of 1.96%. This has since been surpassed by OpenHands and others (50%+ on verified).

### 4.4 Inferred Patterns (from public demonstrations)

- **Plan-first approach**: Creates an explicit plan before coding
- **Browser for research**: Uses browser to read documentation, Stack Overflow
- **Iterative debugging**: Runs code, observes errors, fixes in a loop
- **End-to-end deployment**: Can deploy to platforms (Netlify, etc.)

No public prompt text is available.

---

## 5. Cross-Cutting Patterns

### 5.1 What All Systems Share

| Pattern | OpenHands | Aider | SWE-agent | Devin |
|---------|-----------|-------|-----------|-------|
| Explore before implementing | Yes (explicit workflow step) | Yes (repo map provides context) | Yes (step 1 instruction) | Yes (plan first) |
| Reproduce bug before fixing | Yes (TESTING step) | No (not autonomous) | Yes (step 2: create reproduction script) | Yes (inferred) |
| Verify fix after implementing | Yes (VERIFICATION step) | Yes (lint/test reflect) | Yes (step 4: rerun script) | Yes (inferred) |
| Structured edit format | str_replace (exact match) | SEARCH/REPLACE or unified diff | Anthropic-style edit tool | Unknown |
| Think/reason step | Explicit `think` tool | No dedicated step | "Your thinking should be thorough" | Plan step |
| Task decomposition | task_tracker tool | No | No | Plan (inferred) |
| Memory management | Condenser system | Chat history truncation | History processors | Unknown |
| Security risk tracking | Per-tool risk enum | No | No | Unknown |

### 5.2 Anti-Give-Up Strategies

**OpenHands:**
- Troubleshooting section forces reflection on 5-7 possible causes
- Task tracker keeps the agent on rails
- "Don't try to directly work around [major issues]" -- forces replanning

**Aider:**
- `lazy_prompt`: "You are diligent and tireless! You NEVER leave comments describing code without implementing it! You always COMPLETELY IMPLEMENT the needed code!"
- Lint-test-reflect loop forces the model to actually fix errors
- Infinite output via prefill prevents truncation-induced laziness

**SWE-agent:**
- Submit review forces re-verification before completion
- 5-step instruction template with explicit verification step
- `max_requeries`: Retries on format errors, blocked actions, syntax errors (up to 3 times)
- "Your thinking should be thorough and so it's fine if it's very long"

### 5.3 Context Window Management

**OpenHands:**
- Multiple condenser strategies: sliding window, LLM-based attention, amortized forgetting
- Agent can explicitly request condensation
- Task tracker state survives condensation

**Aider:**
- ChatChunks ordering puts system reminder at END (recency bias)
- Repo map uses PageRank to compress whole-repo context into ~1K tokens
- Previous conversation is included as "done" messages
- Infinite output via prefill handles long responses

**SWE-agent:**
- Observation truncation at `max_observation_length` (100K chars)
- Cache control on last 2 messages only
- History processors for managing conversation length

### 5.4 Edit Mechanism Comparison

| Approach | Pros | Cons | Best For |
|----------|------|------|----------|
| str_replace (exact match) | Precise, verifiable, small diffs | Brittle to whitespace mismatches | Autonomous agents |
| SEARCH/REPLACE (Aider) | Same as above, well-tested | Same brittleness | Interactive coding |
| Unified diff | Familiar format, compact | Models struggle with line numbers | Strong models only |
| Whole file | Simple, no parsing needed | Expensive in tokens | Small files, weak models |
| LLM-based edit (fuzzy) | Handles large files with `# ... existing code ...` | Less predictable | Large files |
| Architect+Editor | Best reasoning + best formatting | Two LLM calls, higher cost | Complex tasks |

### 5.5 Few-Shot Example Patterns

**OpenHands**: One long worked example showing the full lifecycle: explore -> create -> error -> fix -> verify -> modify -> finish. Uses function-call XML format.

**Aider**: Two short focused examples showing edit mechanics only (factorial refactor, extract-to-file). Format-focused, not task-focused.

**SWE-agent**: Uses optional demonstration trajectories loaded from files. The default config doesn't include few-shot examples -- relies on the structured 5-step instruction instead.

---

## 6. Design Recommendations for Cairn

Based on this research, here are patterns to adopt for Cairn's GitHub-issue-to-PR agent:

### 6.1 System Prompt Architecture

Use OpenHands-style **tagged sections** with Jinja2/Handlebars templating:

```
<ROLE>You are Cairn Agent, an autonomous software engineer...</ROLE>
<WORKFLOW>
1. UNDERSTAND: Read the issue, explore the repo, understand the codebase
2. PLAN: Decompose into subtasks, identify files to modify
3. IMPLEMENT: Make minimal, focused changes
4. VERIFY: Run tests, check for regressions
5. SUBMIT: Create PR with clear description
</WORKFLOW>
<TOOLS>...</TOOLS>
<ANTI_SHORTCUTS>...</ANTI_SHORTCUTS>
```

### 6.2 Tool Design

Minimum viable tool set:
- **bash**: Persistent session, timeout handling, stdin interaction (OpenHands pattern)
- **file_read**: View files with line ranges (not raw `cat`)
- **file_edit**: str_replace with exact matching (OpenHands/Aider pattern)
- **think**: Structured reasoning log (OpenHands pattern)
- **task_tracker**: Decomposition and progress tracking (OpenHands pattern)
- **search**: grep/ripgrep with file-only output mode (SWE-agent ACI principle)
- **finish**: With mandatory summary

### 6.3 Anti-Give-Up Mechanisms

Layer multiple strategies:
1. **Lazy prompt injection** (Aider): "You NEVER leave comments describing code without implementing it!"
2. **Troubleshooting reflection** (OpenHands): Force 5-7 hypothesis brainstorm on failure
3. **Submit review** (SWE-agent): Verify before declaring done
4. **Task tracker persistence** (OpenHands): Track progress across context condensation
5. **Max retries on format errors** (SWE-agent): Don't give up on parsing failures

### 6.4 Context Management

1. **Repo map** (Aider): Use tree-sitter to build a compressed repository index. This is the single most impactful innovation for large repos.
2. **Observation truncation** (SWE-agent): Cap command output, tell the agent to use head/tail/grep
3. **Memory condensation** (OpenHands): LLM-based summarization of old conversation history
4. **System reminder at end** (Aider): Repeat critical formatting rules near the end of context

### 6.5 Issue-to-PR Workflow Template

Adapt SWE-agent's instance template for GitHub issues:

```
A GitHub issue has been filed in repository {{repo}}:

<issue>
{{issue_title}}

{{issue_body}}
</issue>

The repository has been cloned to {{workspace}}. You are on branch {{branch}}.

Follow these steps:
1. Read and understand the issue thoroughly
2. Explore the codebase to find relevant files (use search, file_read)
3. If this is a bug: create a reproduction script and confirm the bug
4. Plan your changes (use think tool to reason, task_tracker to decompose)
5. Implement the minimal changes needed
6. Run the project's test suite and fix any failures
7. Create a PR with a clear title and description

Your thinking should be thorough. Take your time exploring before implementing.
Do NOT describe what you would do -- actually do it.
Do NOT leave TODO comments -- implement the actual code.
```

### 6.6 Key Architectural Decisions

| Decision | Recommendation | Rationale |
|----------|---------------|-----------|
| Agent loop | ReAct-style observe-think-act | Industry standard, proven on SWE-bench |
| Edit format | str_replace (exact match) | Best precision/reliability tradeoff |
| Tool interface | Function calling (OpenAI format) | All major LLMs support it |
| Context strategy | Repo map + sliding window + condensation | Handles repos of any size |
| Anti-laziness | Multi-layered (prompt + reflection + review) | Single strategy is insufficient |
| Few-shot examples | One full worked example (OpenHands style) | Shows complete lifecycle |
| Multi-model | Optional Architect+Editor split (Aider) | 5-10% accuracy improvement |
| Security | Risk assessment per tool call (OpenHands) | Essential for production use |

### 6.7 What NOT To Do

1. **Don't use whole-file editing** for repos with large files -- token waste
2. **Don't rely on a single anti-laziness prompt** -- models habituate to it
3. **Don't skip the exploration phase** -- "thoroughly explore before proposing solutions" is in every system
4. **Don't dump entire files into context** -- use repo maps, line ranges, search results
5. **Don't use interactive commands** (pagers, editors that need stdin) -- all systems disable `PAGER`, `MANPAGER`, etc.
6. **Don't trust the first edit attempt** -- always verify with tests/lint
7. **Don't let the agent modify test files** when fixing bugs (SWE-agent explicitly forbids this)
