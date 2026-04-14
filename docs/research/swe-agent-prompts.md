# SWE-Agent Prompt Architecture Research

Research conducted 2026-04-14 for Cairn's GitHub-issue-to-PR coding agent.

Sources: princeton-nlp/SWE-agent (GitHub), SWE-agent/mini-swe-agent,
All-Hands-AI/OpenHands, arxiv.org/abs/2405.15793, swe-agent.com docs.

---

## Table of Contents

1. [Executive Summary — Why Cairn's Agent Gives Up Early](#1-executive-summary)
2. [SWE-Agent System Prompt Architecture](#2-swe-agent-system-prompt-architecture)
3. [Mini-SWE-Agent (Successor) Prompt Architecture](#3-mini-swe-agent-successor)
4. [OpenHands/CodeAct Prompt Architecture (Comparison)](#4-openhandsodeact-comparison)
5. [How These Systems Prevent Early Termination](#5-preventing-early-termination)
6. [Agent Identity and Capability Framing](#6-agent-identity-framing)
7. [Tool Discovery and Presentation](#7-tool-discovery-and-presentation)
8. [Workflow Structuring: Step-by-Step vs Free-Form](#8-workflow-structuring)
9. [Demonstrations and In-Context Learning](#9-demonstrations)
10. [Submission and Quality Gates](#10-submission-gates)
11. [Key Design Principles from the ACI Paper](#11-aci-design-principles)
12. [Diagnosis: Why Cairn's Agent Calls complete_run Too Early](#12-cairn-diagnosis)
13. [Recommended Cairn Prompt Architecture](#13-recommended-architecture)

---

## 1. Executive Summary

The core finding: **SWE-agent, mini-swe-agent, and OpenHands all share a design
where the agent has NO easy "give up" path**. The completion/submit action is
either gated behind a multi-stage review, or framed as a final irreversible step
that requires explicit confirmation. Cairn's current prompt says "return
complete_run immediately" and "return complete_run with your best answer" -- this
actively encourages the agent to bail out instead of doing work.

Key takeaways:

- **Never mention the exit action in the workflow instructions.** SWE-agent's
  instance template describes 5 concrete steps (explore, reproduce, fix, verify,
  edge-cases) without ever mentioning "submit" until the agent has completed them.
- **Frame the agent as an autonomous programmer, not an assistant.** "You are an
  autonomous programmer working directly in the command line" (SWE-agent 0.7) vs
  Cairn's "You are a focused AI agent."
- **One action per turn with mandatory feedback.** Every system enforces a
  thought-action-observation loop. The agent must wait for tool output before
  deciding next steps.
- **Gate submission behind a review step.** SWE-agent's `review_on_submit_m`
  tool forces the agent to re-run reproduction scripts, revert test changes, and
  re-review its diff before the patch is actually submitted.
- **Include a worked demonstration.** SWE-agent 0.7 injects a full trajectory
  showing how to solve a real issue. OpenHands includes a multi-step Flask
  example with error recovery.

---

## 2. SWE-Agent System Prompt Architecture

SWE-agent's prompt is assembled from a YAML config with these template slots:

### 2.1 Template Variables

| Variable | Source |
|----------|--------|
| `{{command_docs}}` | Auto-generated from tool config YAML |
| `{{working_dir}}` | Set by environment (repo checkout path) |
| `{{problem_statement}}` | GitHub issue text |
| `{{open_file}}` | State command output (current file in editor) |
| `{{observation}}` | Last tool execution output |
| `{{WINDOW}}` | Lines per page in file viewer (default 100) |
| `{{demonstration}}` | Full trajectory JSON from a solved example |

### 2.2 System Template (SWE-agent 0.7 -- the original paper config)

This is the verbatim system prompt from `config/sweagent_0_7/07.yaml`:

```
SETTING: You are an autonomous programmer, and you're working directly in the
command line with a special interface.

The special interface consists of a file editor that shows you {WINDOW} lines
of a file at a time. In addition to typical bash commands, you can also use
the following commands to help you navigate and edit files.

COMMANDS:
{command_docs}

Please note that THE EDIT COMMAND REQUIRES PROPER INDENTATION.
If you'd like to add the line '        print(x)' you must fully write that
out, with all those spaces before the code! Indentation is important and code
that is not indented correctly will fail and require fixing before it can be
run.

RESPONSE FORMAT:
Your shell prompt is formatted as follows:
(Open file: <path>) <cwd> $

You need to format your output using two fields; discussion and command.
Your output should always include _one_ discussion and _one_ command field
EXACTLY as in the following example:
DISCUSSION
First I'll start by using ls to see what files are in the current directory.
Then maybe we can look at some relevant files to see what they look like.
```
ls -a
```

You should only include a *SINGLE* command in the command section and then
wait for a response from the shell before continuing with more discussion
and commands. Everything you include in the DISCUSSION section will be saved
for future reference.
If you'd like to issue two commands at once, PLEASE DO NOT DO THAT! Please
instead first submit just the first command, and then after receiving a
response you'll be able to issue the second command.
You're free to use any other bash commands you want (e.g. find, grep, cat,
ls, cd) in addition to the special commands listed above.
However, the environment does NOT support interactive session commands
(e.g. python, vim), so please do not invoke them.
```

Key observations:
- **Identity: "autonomous programmer"** -- not "assistant" or "agent"
- **Single command per turn** -- enforced explicitly and repeatedly
- **No mention of "give up" or "complete" in the system prompt**
- **Tools described inline** via `{command_docs}` auto-generated from YAML
- **Concrete format example** included directly in the system prompt

### 2.3 Instance Template (SWE-agent 0.7)

This is the per-issue message that follows the system prompt:

```
We're currently solving the following issue within our repository. Here's the
issue text:
ISSUE:
{problem_statement}

INSTRUCTIONS:
Now, you're going to solve this issue on your own. Your terminal session has
started and you're in the repository's root directory. You can use any bash
commands or the special interface to help you. Edit all the files you need to
and run any checks or tests that you want.
Remember, YOU CAN ONLY ENTER ONE COMMAND AT A TIME. You should always wait
for feedback after every command.
When you're satisfied with all of the changes you've made, you can submit
your changes to the code base by simply running the submit command.
Note however that you cannot use any interactive session commands (e.g.
python, vim) in this environment, but you can write scripts and run them.
E.g. you can write a python script and then run it with `python <script_name>.py`.

NOTE ABOUT THE EDIT COMMAND: Indentation really matters! When editing a file,
make sure to insert appropriate indentation before each line!

IMPORTANT TIPS:
1. Always start by trying to replicate the bug that the issues discusses.
   If the issue includes code for reproducing the bug, we recommend that you
   re-implement that in your environment, and run it to make sure you can
   reproduce the bug. Then start trying to fix it.
   When you think you've fixed the bug, re-run the bug reproduction script to
   make sure that the bug has indeed been fixed.

   If the bug reproduction script does not print anything when it successfully
   runs, we recommend adding a print("Script completed successfully, no
   errors.") command at the end of the file, so that you can be sure that the
   script indeed ran fine all the way through.

2. If you run a command and it doesn't work, try running a different command.
   A command that did not work once will not work the second time unless you
   modify it!

3. If you open a file and need to get to an area around a specific line that
   is not in the first 100 lines, say line 583, don't just use the
   scroll_down command multiple times. Instead, use the goto 583 command.
   It's much quicker.

4. If the bug reproduction script requires inputting/reading a specific file,
   such as buggy-input.png, and you'd like to understand how to input that
   file, conduct a search in the existing repo code, to see whether someone
   else has already done that.

5. Always make sure to look at the currently open file and the current working
   directory (which appears right after the currently open file). The currently
   open file might be in a different directory than the working directory!

6. When editing files, it is easy to accidentally specify a wrong line number
   or to write code with incorrect indentation. Always check the code after
   you issue an edit to make sure that it reflects what you wanted to
   accomplish. If it didn't, issue another command to fix it.

(Open file: {open_file})
(Current directory: {working_dir})
bash-$
```

Key observations:
- **Assumes the agent WILL solve the problem** -- "you're going to solve this
  issue on your own"
- **Submit is mentioned only as "when you're satisfied with ALL changes"**
- **6 concrete, actionable tips** about common failure modes
- **Tips prevent common agent mistakes**: not reproducing bugs, re-running
  failed commands, scrolling instead of jumping, wrong indentation
- **The prompt ends with a shell-like prompt** to ground the agent in the
  terminal context

### 2.4 Default Config (Current -- Anthropic-style function calling)

The current default (`config/default.yaml`) is simpler because it uses
function calling instead of text parsing:

```
system_template: |-
  You are a helpful assistant that can interact with a computer to solve tasks.

instance_template: |-
  <uploaded_files>
  {working_dir}
  </uploaded_files>
  I've uploaded a python code repository in {working_dir}. Consider the
  following PR description:

  <pr_description>
  {problem_statement}
  </pr_description>

  Can you help me implement the necessary changes to the repository so that
  the requirements specified in the <pr_description> are met?
  I've already taken care of all changes to any of the test files described
  in the <pr_description>. This means you DON'T have to modify the testing
  logic or any of the tests in any way!
  Your task is to make the minimal changes to non-tests files in the
  {working_dir} directory to ensure the <pr_description> is satisfied.
  Follow these steps to resolve the issue:
  1. As a first step, it might be a good idea to find and read code relevant
     to the <pr_description>
  2. Create a script to reproduce the error and execute it with
     `python <filename.py>` using the bash tool, to confirm the error
  3. Edit the sourcecode of the repo to resolve the issue
  4. Rerun your reproduce script and confirm that the error is fixed!
  5. Think about edgecases and make sure your fix handles them as well
  Your thinking should be thorough and so it's fine if it's very long.

next_step_template: |-
  OBSERVATION:
  {observation}

next_step_no_output_template: |-
  Your command ran successfully and did not produce any output.
```

Key difference: this config enables `function_calling` parsing + the
`review_on_submit_m` tool bundle which gates submission.

### 2.5 Observation Templates

SWE-agent never leaves the agent hanging. When a command produces no output:

```
Your command ran successfully and did not produce any output.
```

When output is too long (over `max_observation_length`):

```
The output of your last command was too long. Please try a different command
that produces less output. If you're looking at a file you can try use head,
tail or sed to view a smaller number of lines selectively.
```

When a command times out:

```
The command '{command}' was cancelled because it took more than {timeout}
seconds. It may have been waiting for user input or otherwise blocked.
Please try a different command.
```

### 2.6 History Processing

SWE-agent 0.7 uses `last_n_observations: 5` -- only the last 5 tool outputs
are kept in full; older ones are replaced with:

```
Old environment output: (N lines omitted)
```

This prevents context window exhaustion while keeping recent context rich.

---

## 3. Mini-SWE-Agent (Successor)

Mini-swe-agent supersedes SWE-agent as of 2026. It achieves >74% on SWE-bench
Verified with approximately 100 lines of agent code. Key difference: **bash
only, no custom tools**.

### 3.1 System Template

```
You are a helpful assistant that can interact with a computer.

Your response must contain exactly ONE bash code block with ONE command
(or commands connected with && or ||).
Include a THOUGHT section before your command where you explain your
reasoning process.
Format your response as shown in <format_example>.

<format_example>
Your reasoning and analysis here. Explain why you want to perform the action.

```mswea_bash_command
your_command_here
```
</format_example>

Failure to follow these rules will cause your response to be rejected.
```

### 3.2 Instance Template

```
Please solve this issue: {task}

You can execute bash commands and edit files to implement the necessary changes.

## Recommended Workflow

This workflow should be done step-by-step so that you can iterate on your
changes and any possible problems.

1. Analyze the codebase by finding and reading relevant files
2. Create a script to reproduce the issue
3. Edit the source code to resolve the issue
4. Verify your fix works by running your script again
5. Test edge cases to ensure your fix is robust
6. Submit your changes and finish your work by issuing the following command:
   `echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT`.
   Do not combine it with any other command.
   <important>After this command, you cannot continue working on this
   task.</important>

## Important Rules

1. Every response must contain exactly one action
2. The action must be enclosed in triple backticks
3. Directory or environment variable changes are not persistent. Every action
   is executed in a new subshell. However, you can prefix any action with
   `MY_ENV_VAR=MY_VALUE cd /path/to/working/dir && ...`
```

### 3.3 Format Error Template

When the agent produces malformed output:

```
Format error:

<error>
{error}
</error>

Here is general guidance on how to format your response:

Please always provide EXACTLY ONE action in triple backticks, found
{actions_count} actions.
If you want to end the task, please issue the following command:
`echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT`
without any other command.
```

### 3.4 Observation Template

```
{% if output.exception_info %}
<exception>{output.exception_info}</exception>
{% endif %}
<returncode>{output.returncode}</returncode>
{% if output.output | length < 10000 %}
<output>
{output.output}
</output>
{% else %}
<warning>
The output of your last command was too long.
...
</warning>
<output_head>
{output.output[:5000]}
</output_head>
<elided_chars>
{elided_chars} characters elided
</elided_chars>
<output_tail>
{output.output[-5000:]}
</output_tail>
{% endif %}
```

### 3.5 Architecture

The agent loop is dead simple:

```python
def run(self, task):
    self.messages = []
    self.add_messages(
        format_message(role="system", content=render(system_template)),
        format_message(role="user", content=render(instance_template)),
    )
    while True:
        try:
            self.step()  # query() + execute_actions()
        except InterruptAgentFlow as e:
            self.add_messages(*e.messages)
        if self.messages[-1].get("role") == "exit":
            break
```

The exit condition is **only** triggered when the environment detects the magic
string `COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT` in stdout. There is no
"complete_run" tool -- the agent must echo a specific string.

---

## 4. OpenHands/CodeAct Comparison

OpenHands (formerly OpenDevin) is the other top SWE-bench performer. Its
prompt architecture provides a useful contrast.

### 4.1 System Prompt (verbatim from system_prompt.j2)

```
You are OpenHands agent, a helpful AI assistant that can interact with a
computer to solve tasks.

<ROLE>
Your primary role is to assist users by executing commands, modifying code,
and solving technical problems effectively. You should be thorough,
methodical, and prioritize quality over speed.
</ROLE>

<EFFICIENCY>
Each action you take is somewhat expensive. Wherever possible, combine
multiple actions into a single action.
</EFFICIENCY>

<PROBLEM_SOLVING_WORKFLOW>
1. EXPLORATION: Thoroughly explore relevant files and understand the context
   before proposing solutions
2. ANALYSIS: Consider multiple approaches and select the most promising one
3. TESTING: For bug fixes: Create tests to verify issues before implementing
   fixes
4. IMPLEMENTATION: Make focused, minimal changes to address the problem
5. VERIFICATION: Test your implementation thoroughly, including edge cases
</PROBLEM_SOLVING_WORKFLOW>
```

Full prompt also includes sections on:
- `<FILE_SYSTEM_GUIDELINES>` -- never create duplicate files, always modify in place
- `<CODE_QUALITY>` -- minimal changes, imports at top
- `<VERSION_CONTROL>` -- git best practices
- `<TROUBLESHOOTING>` -- "Step back and reflect on 5-7 different possible sources"

### 4.2 Finish Tool Description

OpenHands defines `finish` as a tool with this description:

```
Signals the completion of the current task or conversation.

Use this tool when:
- You have successfully completed the user's requested task
- You cannot proceed further due to technical limitations or missing information

The message should include:
- A clear summary of actions taken and their results
- Any next steps for the user
- Explanation if you're unable to complete the task
```

### 4.3 Think Tool

OpenHands has a dedicated `think` tool:

```
Use the tool to think about something. It will not obtain new information
or make any changes to the repository, but just log the thought.

Common use cases:
1. When exploring a repository and discovering the source of a bug, call
   this tool to brainstorm several unique ways of fixing the bug
2. After receiving test results, use this tool to brainstorm ways to fix
   failing tests
3. When planning a complex refactoring, outline different approaches
4. When designing a new feature, think through architecture decisions
5. When debugging a complex issue, organize your thoughts and hypotheses
```

### 4.4 In-Context Learning Example

OpenHands includes a full worked example showing:
1. Check directory (`pwd && ls`)
2. Create a file (`str_replace_editor create`)
3. Run it (fails with missing dependency)
4. Install dependency (`pip3 install flask`)
5. Run again (succeeds)
6. Edit the file (`str_replace_editor str_replace`)
7. Verify the change works
8. Call `finish`

The example shows **error recovery** (step 3-4), which is critical for
teaching the agent not to give up when things fail.

---

## 5. Preventing Early Termination

This is the most relevant section for Cairn's problem.

### 5.1 Strategies Used by SWE-Agent

| Strategy | Implementation |
|----------|---------------|
| **No "give up" tool** | SWE-agent 0.7 has `submit` only. There is a `exit_forfeit` tool but it is in a separate bundle NOT included by default. |
| **Multi-stage submit** | `review_on_submit_m` forces the agent through a review checklist before the patch is emitted. First `submit` call shows the diff and asks the agent to re-verify. Second `submit` actually emits the patch. |
| **Workflow-first framing** | The instance template describes 5-6 concrete steps the agent should follow. Submit is mentioned only at the end as "when you're satisfied." |
| **Demonstration trajectory** | A full solved example shows the expected depth of work. |
| **Cost/step limits** | Termination happens via external limits, not agent choice. |
| **Retry on parse errors** | `max_requeries` allows re-asking the LLM when output is malformed. |
| **Reviewer agent** | Optional: a separate LLM scores the submission and can reject it, forcing the agent back to work. |

### 5.2 Strategies Used by Mini-SWE-Agent

| Strategy | Implementation |
|----------|---------------|
| **Magic string submission** | Agent must `echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT` -- not a tool call, a specific stdout string. Harder to accidentally trigger. |
| **Irreversibility warning** | `<important>After this command, you cannot continue working on this task.</important>` |
| **Exception-based flow** | The loop only exits on `role="exit"` messages -- the agent cannot return early from `step()`. |

### 5.3 Strategies Used by OpenHands

| Strategy | Implementation |
|----------|---------------|
| **Finish requires summary** | The `finish` tool requires a `message` parameter with a summary. |
| **Quality-over-speed framing** | System prompt says "prioritize quality over speed." |
| **Troubleshooting section** | Explicit instructions to "step back and reflect on 5-7 different possible sources of the problem" when stuck. |
| **Think tool** | Provides a structured way to reason without acting, preventing reflexive completion. |
| **Task tracker** | Long-horizon variant tracks tasks and requires marking each as "done." |

### 5.4 What Cairn Currently Does Wrong

Cairn's `build_system_prompt` in `decide_impl.rs` contains these lines:

```
5. When you have enough information to answer the goal, return complete_run
   immediately.
6. If memory is empty AND no other tool can help, return complete_run with
   your best answer.
```

This is the **exact opposite** of what works. It:
- Tells the agent to complete as soon as it has "enough information"
- Provides a fallback that says "if nothing works, just complete"
- Never describes the actual work the agent should do
- Mentions `complete_run` twice in the instructions but never mentions
  exploring code, writing code, creating branches, or opening PRs

---

## 6. Agent Identity Framing

### How each system frames agent identity:

| System | Identity |
|--------|----------|
| SWE-agent 0.7 | "You are an autonomous programmer, and you're working directly in the command line with a special interface." |
| SWE-agent default | "You are a helpful assistant that can interact with a computer to solve tasks." |
| Mini-SWE-agent | "You are a helpful assistant that can interact with a computer." |
| OpenHands | "You are OpenHands agent, a helpful AI assistant that can interact with a computer to solve tasks." |
| Cairn (current) | "You are a focused AI agent." / "You are the orchestrator." |

Observations:
- The strongest performer (SWE-agent 0.7) uses the most specific identity:
  **"autonomous programmer"** working in a **"command line"**
- All systems ground the agent in a **computer interaction** context
- None of them use abstract terms like "focused AI agent" or "orchestrator"
- The identity should match the task: if the agent is writing code, call it
  a programmer

---

## 7. Tool Discovery and Presentation

### 7.1 SWE-Agent Tool Documentation Format

Tools are defined in YAML config files per bundle:

```yaml
tools:
  find_file:
    signature: "find_file <file_name> [<dir>]"
    docstring: "finds all files with the given name or pattern in dir"
    arguments:
      - name: file_name
        type: string
        description: "the name of the file or pattern to search for"
        required: true
      - name: dir
        type: string
        description: "the directory to search in"
        required: false
```

These are rendered into `{command_docs}` via `generate_command_docs()`:

```
command_name:
  docstring: description text
  signature: command syntax
  arguments:
    - param_name (type) [required/optional]: description
```

For function-calling configs, each tool becomes a standard OpenAI function
schema via `get_function_calling_tool()`.

### 7.2 SWE-Agent Tool Bundles

The default config uses three bundles:
1. **tools/registry** -- environment variable management (internal)
2. **tools/edit_anthropic** -- `str_replace_editor` (view/create/replace/insert/undo)
3. **tools/review_on_submit_m** -- gated `submit` with diff review

SWE-agent 0.7 uses:
1. **tools/registry** -- env vars
2. **tools/windowed** -- file viewer (open/goto/scroll_up/scroll_down/create)
3. **tools/search** -- find_file, search_dir, search_file
4. **tools/windowed_edit_linting** -- line-range edit with lint validation
5. **tools/submit** -- simple submit

Plus `enable_bash_tool: true` for raw shell access.

### 7.3 The str_replace_editor Tool (Anthropic-style)

This is the primary edit tool, shared between SWE-agent and OpenHands:

```yaml
str_replace_editor:
  commands: view, create, str_replace, insert, undo_edit
  key constraint: old_str must match EXACTLY one or more consecutive lines
  state: persistent across calls
```

### 7.4 Tool Blocklist

SWE-agent blocks interactive commands: vim, nano, gdb, etc. This prevents the
agent from entering interactive sessions that would hang.

---

## 8. Workflow Structuring

### 8.1 SWE-Agent's Approach: Concrete Steps + Tips

The workflow is always presented as numbered steps:

1. Find and read relevant code
2. Create reproduction script and run it
3. Edit source code
4. Re-run reproduction script
5. Think about edge cases

Then followed by 6 "IMPORTANT TIPS" that address specific failure modes.

### 8.2 Mini-SWE-Agent: Steps + Command Examples

Steps 1-5 same as above, plus step 6 (submit). Then includes literal command
examples for: creating files (`cat <<'EOF'`), editing with sed, viewing with
`nl -ba`.

### 8.3 OpenHands: Problem-Solving Workflow

Uses a more abstract framework:
1. EXPLORATION
2. ANALYSIS
3. TESTING
4. IMPLEMENTATION
5. VERIFICATION

### 8.4 What Works Best

SWE-agent 0.7's approach (concrete steps + failure-mode tips) performs best on
benchmarks. The tips are particularly effective because they address **specific
mistakes agents actually make**:

- Not reproducing the bug first
- Re-running failed commands without modification
- Scrolling instead of jumping to a line
- Ignoring indentation
- Confusing open file path with working directory

**For Cairn**: the workflow should name the actual operations (clone repo, read
files, create branch, write code, run tests, commit, push, open PR) with tips
for each.

---

## 9. Demonstrations

### 9.1 SWE-Agent Demonstrations

SWE-agent 0.7 includes a full trajectory from solving `marshmallow-1867`:

```yaml
demonstrations:
  - trajectories/demonstrations/replay__marshmallow-code__marshmallow-1867__...
```

The demonstration is injected via:

```
Here is a demonstration of how to correctly accomplish this task.
It is included to show you how to correctly use the interface.
You do not need to follow exactly what is done in the demonstration.
--- DEMONSTRATION ---
{demonstration}
--- END OF DEMONSTRATION ---
```

### 9.2 OpenHands In-Context Example

OpenHands includes a Flask app example that shows:
1. `pwd && ls` (explore)
2. Create `app.py` (write code)
3. Run it (fails -- ModuleNotFoundError)
4. `pip3 install flask` (recover from error)
5. Run again (succeeds)
6. Edit file (modify code)
7. Run again (verify)
8. `finish` (complete)

The error-recovery step (3-4) is critical -- it teaches the agent to **keep
going when things fail**.

### 9.3 Implication for Cairn

Cairn should include a demonstration trajectory showing:
1. Read the GitHub issue
2. Clone/navigate the repo
3. Search for relevant files
4. Read the code
5. Create a branch
6. Make edits
7. Run tests (they fail)
8. Fix the failing test
9. Run tests again (pass)
10. Commit and push
11. Open PR
12. Complete

---

## 10. Submission and Quality Gates

### 10.1 SWE-Agent's Multi-Stage Submit

The `review_on_submit_m` bundle intercepts the first `submit` call:

```python
# First submit call shows review message:
SUBMIT_REVIEW_MESSAGES:
  - |
    Thank you for your work on this issue. Please carefully follow the steps
    below to help review your changes.

    1. If you made any changes to your code after running the reproduction
       script, please run the reproduction script again.
    2. Remove your reproduction script (if you haven't done so already).
    3. If you have modified any TEST files, please revert them to the state
       they had before you started fixing the issue.
       You can do this with `git checkout -- /path/to/test/file.py`.
    4. Run the submit command again to confirm.

    Here is a list of all of your changes:

    <diff>
    {diff}
    </diff>
```

The agent must run `submit` a **second time** after completing the review
checklist. The actual patch (`<<SWE_AGENT_SUBMISSION>>`) is only emitted
on the second call (or when `--force` is used).

### 10.2 SWE-Agent's Reviewer Agent

Optional but powerful: a separate LLM evaluates submissions:
- Samples N evaluations (default 5)
- Extracts numerical scores
- Averages scores with std-deviation penalty
- Penalizes cost-limit exits
- Can reject and force retry

### 10.3 Mini-SWE-Agent's Approach

No review gate -- but the completion string is deliberately awkward:
`echo COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT`

Plus the irreversibility warning: "After this command, you cannot continue
working on this task."

### 10.4 Recommended for Cairn

Implement a two-stage completion:
1. First `complete_run` shows the agent its diff and asks it to verify
2. Second `complete_run` actually completes the run

Or better: require the agent to call `open_pr` before `complete_run`.

---

## 11. Key Design Principles from the ACI Paper

The SWE-agent paper (arxiv.org/abs/2405.15793) introduced the Agent-Computer
Interface (ACI) concept. Core findings:

### 11.1 ACI > Raw Shell

A baseline agent with no ACI performs dramatically worse. The custom interface
(specialized viewer, editor, search tools) is the main performance driver.

### 11.2 Four Critical ACI Features

1. **Syntax validation** -- Lint code before accepting edits. Reject bad
   indentation with a clear error message.
2. **Specialized file viewer** -- Show approximately 100 lines at a time with
   line numbers. Do not dump entire files.
3. **Succinct search results** -- `find_file` and `search_dir` return file paths
   and match counts, not full content.
4. **Empty output handling** -- Always tell the agent when a command succeeded
   with no output. Silence causes confusion.

### 11.3 Interface Design = Prompt Engineering

The paper's key thesis: designing the tools and their output format IS prompt
engineering. The way you present information to the agent matters as much as
the system prompt text.

---

## 12. Diagnosis: Why Cairn's Agent Calls complete_run Too Early

Based on comparing Cairn's current prompts with the research:

### Problem 1: The prompt actively encourages early completion

```
5. When you have enough information to answer the goal, return complete_run
   immediately.
6. If memory is empty AND no other tool can help, return complete_run with
   your best answer.
```

This tells the LLM: "your job is to answer a question, and you should finish
ASAP." But the job is to **write code and open a PR**, which requires many
steps.

### Problem 2: No workflow description

The prompt lists tool usage rules but never describes the actual workflow:
- No mention of reading code
- No mention of writing code
- No mention of creating branches
- No mention of running tests
- No mention of opening PRs

### Problem 3: Generic identity

"You are a focused AI agent" gives no context about what the agent is supposed
to do. Compare with "You are an autonomous programmer working directly in the
command line."

### Problem 4: No error recovery guidance

No tips about what to do when commands fail, when tests break, when the repo
structure is unfamiliar.

### Problem 5: No demonstrations

The agent has never seen a successful trajectory, so it does not know what
"doing work" looks like.

### Problem 6: Memory-search-first bias

Rule 2 says "ALWAYS search memory first" -- but for a GitHub issue coding
task, memory is irrelevant. The agent should be exploring the repo.

---

## 13. Recommended Cairn Prompt Architecture

Based on this research, here is the recommended prompt structure for Cairn's
GitHub-issue-to-PR agent:

### 13.1 System Prompt

```
You are an autonomous software engineer working in a sandboxed environment.
You have access to a repository checkout and a set of tools for exploring
code, editing files, running commands, and interacting with GitHub.

Your job is to solve GitHub issues by writing code: understanding the
codebase, implementing fixes or features, and opening pull requests.

You operate in a loop: you propose ONE action at a time, observe the result,
then propose the next action. You must wait for each result before proceeding.

## Available Tools
{tool_docs}

## Response Format
Return a JSON array with exactly one action per response.

## Rules
1. ONE action per response. Wait for the result before your next action.
2. Only use tools listed in Available Tools. Do not invent tool names.
3. When a command fails, analyze the error and try a different approach.
   Do not re-run the same failing command.
4. Do not use interactive commands (vim, nano, python REPL).
5. Do not complete your run until you have opened a pull request with
   your changes.
```

### 13.2 Instance Template (per-issue)

```
## Your Task

Solve this GitHub issue by writing code and opening a pull request.

**Repository:** {repo_url}
**Issue:** {issue_title}

<issue_body>
{issue_body}
</issue_body>

## Workflow

Follow these steps in order. Do not skip steps.

1. **Explore the repository.** Use list_files and read_file to understand
   the project structure, find relevant code, and understand the codebase
   conventions.

2. **Understand the issue.** Read the relevant source files. Identify
   where the bug or feature gap is.

3. **Create a branch.** Create a descriptive branch name from the issue
   (e.g., `fix/issue-42-null-pointer` or `feat/issue-15-add-export`).

4. **Write the fix.** Edit the source files to implement the change.
   Make minimal, focused changes. Follow existing code style.

5. **Run tests.** If the project has tests, run them. If your change
   breaks tests, fix them. If there are no tests, consider adding one.

6. **Commit your changes.** Write a clear commit message referencing the
   issue number.

7. **Push and open a pull request.** Push your branch and create a PR
   that links to the issue.

8. **Complete your run.** Only after the PR is open, call complete_run
   with a summary of what you did.

## Important Tips

- Start by exploring. Do not try to write code until you understand the
  codebase structure and the problem.
- If a command fails, read the error carefully. Try a different approach.
  A command that failed once will fail again unless you change something.
- When reading large files, use line ranges instead of reading the entire
  file.
- Always verify your changes compile/pass before opening the PR.
- If you get stuck, search for similar patterns in the codebase. The
  existing code is your best guide.
```

### 13.3 Completion Gate

Do NOT include `complete_run` in the tool documentation as a "ready anytime"
action. Instead, frame it as the final step that can ONLY be called after a
PR is opened. Consider:

Option A: Remove `complete_run` from the tool list entirely. Use a magic
string like mini-swe-agent (`echo CAIRN_TASK_COMPLETE`).

Option B: Make `complete_run` require a `pr_url` parameter. The orchestrator
rejects calls without a valid PR URL.

Option C: Two-stage completion. First call shows the diff and asks for
verification. Second call actually completes.

### 13.4 Demonstration

Include a full worked example showing a real issue being solved:

```
--- DEMONSTRATION ---
[Step 1] list_files(".") -> shows repo structure
[Step 2] read_file("src/main.rs", lines=1-50) -> reads entry point
[Step 3] search_files("parse_config") -> finds relevant function
[Step 4] read_file("src/config.rs", lines=120-160) -> reads the buggy code
[Step 5] shell_exec("git checkout -b fix/issue-42-config-parse")
[Step 6] edit_file("src/config.rs", ...) -> applies the fix
[Step 7] shell_exec("cargo test") -> tests pass
[Step 8] shell_exec("git add -A && git commit -m 'fix: handle empty config'")
[Step 9] shell_exec("git push -u origin fix/issue-42-config-parse")
[Step 10] open_pr(title="fix: handle empty config", body="Fixes #42...")
[Step 11] complete_run(summary="Opened PR #43 fixing issue #42...")
--- END OF DEMONSTRATION ---
```

### 13.5 Key Differences from Current Cairn Prompts

| Current | Recommended |
|---------|-------------|
| "focused AI agent" | "autonomous software engineer" |
| "return complete_run immediately" | "do not complete until PR is open" |
| "search memory first" | "explore the repository first" |
| No workflow | 8-step workflow with concrete actions |
| No tips | Tips for common failure modes |
| No demonstration | Full worked example |
| complete_run mentioned 2x in rules | complete_run mentioned only as final step |
| Tool rules dominate | Workflow dominates, tools serve the workflow |

---

## Appendix A: Tool Configs from SWE-Agent

### str_replace_editor (edit_anthropic)

```yaml
str_replace_editor:
  commands: [view, create, str_replace, insert, undo_edit]
  arguments:
    - command (string, required): view|create|str_replace|insert|undo_edit
    - path (string, required): absolute file path
    - file_text (string): content for create
    - old_str (string): exact match for str_replace
    - new_str (string): replacement text
    - insert_line (integer): line number for insert
    - view_range (array[int]): [start, end] for view
  key: old_str must match EXACTLY, must be unique in file
```

### Windowed File Viewer

```yaml
open: 'open "<path>" [<line_number>]'  # opens file at line
goto: "goto <line_number>"              # jump to line
scroll_up: "scroll_up"                  # up WINDOW lines
scroll_down: "scroll_down"             # down WINDOW lines
create: "create <filename>"            # create new file
```

### Search Tools

```yaml
find_file: "find_file <file_name> [<dir>]"    # find by name/pattern
search_dir: "search_dir <search_term> [<dir>]" # grep in directory
search_file: "search_file <search_term> [<file>]" # grep in file
```

### Windowed Edit with Linting

```yaml
edit:
  signature: |
    edit <start_line>:<end_line>
    <replacement_text>
    end_of_edit
  docstring: >
    Replaces lines <start_line> through <end_line> (inclusive) with the given
    text in the open file. Please note that THIS COMMAND REQUIRES PROPER
    INDENTATION.
```

---

## Appendix B: OpenHands Tool Definitions

### Bash Tool

```
execute_bash:
  command (string, required): bash command to run
  is_input (enum[true,false]): stdin input mode
  timeout (number): hard timeout in seconds
  key: persistent shell session, one command at a time
```

### Think Tool

```
think:
  thought (string, required): the thought to log
  purpose: structured reasoning without action
```

### Finish Tool

```
finish:
  message (string, required): summary of actions taken
  when: task complete OR cannot proceed further
```

---

## Appendix C: Environment Variables Set by All Systems

All three systems disable pagination and progress bars in the sandbox:

```
PAGER=cat
MANPAGER=cat
LESS=-R
PIP_PROGRESS_BAR=off
TQDM_DISABLE=1
GIT_PAGER=cat
```

This prevents tools from entering interactive modes that would hang the agent.
