# Learning Guide: Live Collaboration Between Terminal Agents

**Generated**: 2026-04-03
**Sources**: 24 resources analyzed
**Depth**: medium

---

## Prerequisites

- Familiarity with running CLI tools in a terminal (bash, shell scripting basics)
- Basic understanding of what an AI coding agent is (e.g., Claude Code, Codex CLI)
- Comfort reading JSON and Python code snippets
- Optional but helpful: tmux basics, git fundamentals

---

## TL;DR

- **Live agent collaboration** means two or more AI agents running concurrently in terminal sessions, sharing tasks, exchanging messages, and building on each other's outputs in real time rather than in sequence.
- The dominant patterns are **orchestrator/worker**, **peer-to-peer (swarm)**, **blackboard**, and **group chat** — each trading off control, flexibility, and coordination cost differently.
- **Communication channels** range from filesystem mailboxes (disk-based JSON/JSONL files) to tmux send-keys, message queues (Kafka/SQS), and standardized open protocols (Google's A2A).
- **Context drift** — agents diverging because they hold inconsistent state — is the #1 failure mode; guard against it with a shared task spec, scoped per-agent context, and an arbiter/coordinator role.
- Coordination gains plateau beyond **4 agents**; 3-5 teammates with 5-6 tasks each is the empirically optimal range.

---

## Core Concepts

### What "Live Collaboration" Means

Traditional agent workflows are sequential: one agent runs, finishes, its output feeds the next. Live collaboration is different — multiple agents are running simultaneously, can read each other's partial progress, send messages mid-task, claim work from a shared queue, and challenge each other's findings before anyone is done.

Key properties that distinguish live collaboration:

| Property | Sequential Workflow | Live Collaboration |
|----------|--------------------|--------------------|
| Timing | Strictly ordered | Concurrent / overlapping |
| Communication | Output-as-input | Direct messaging + shared state |
| Context | Passed by value | Shared or selectively synchronized |
| Task ownership | Pre-assigned | Self-claimed from shared list |
| Convergence | Deterministic | Emergent from negotiation |

Live collaboration adds coordination overhead, latency risk, and more complex failure modes. It pays off when tasks can genuinely be parallelized and when agents benefit from seeing each other's reasoning, not just final results.

---

### Collaboration Patterns

#### Orchestrator / Worker (Hierarchical)

A central **orchestrator** decomposes the goal, assigns tasks to **worker** agents, monitors progress, and synthesizes results. Workers focus on narrow tasks and report back. Workers do not communicate with each other directly.

```text
User
 └── Lead / Orchestrator
       ├── Worker A  (research docs)
       ├── Worker B  (write tests)
       └── Worker C  (implement feature)
```

**When to use**: Clear task decomposition, parallel work where workers don't need each other's intermediate outputs, tasks with known scope up front.

**Pitfall**: Orchestrator becomes a bottleneck. Synchronous waiting (orchestrator waits for each worker before proceeding) is simpler but slower. Anthropic's research system currently uses synchronous execution and acknowledges async as a future improvement.

#### Peer-to-Peer / Swarm

Agents communicate directly without a central controller. Each agent approaches the problem from its own perspective, shares findings, and refines based on what others publish. System behavior emerges from local interactions.

```text
Agent A  <-->  Agent B
   ^               ^
   |               |
   +---> Agent C <-+
```

**When to use**: Exploration tasks, competing hypotheses, scenarios where the optimal approach is unknown up front, creative divergence needed before convergence.

**Pitfall**: Agents can argue indefinitely. Require an explicit convergence rule (recursion limit, final-decision-maker node, or voting quorum).

#### Blackboard

All agents read from and write to a **shared knowledge repository** (the "blackboard"). A **control component** monitors the board and decides which agent activates next based on current board state. Agents don't communicate with each other — only with the board.

```text
            +-------------------+
            |    BLACKBOARD     |
            |  (shared state)   |
            +-------------------+
              ^    ^    ^    ^
              |    |    |    |
           KS-A  KS-B  KS-C  Control
         (agents / knowledge sources)
```

**Components**:
- **Blackboard**: structured document, vector DB, message queue, or key-value store
- **Knowledge Sources (KS)**: agents with a condition part (activation trigger) and action part (what to write)
- **Control**: monitors board changes, selects next KS via priority, focus, or opportunistic strategies

**When to use**: Complex problems requiring diverse expertise (speech recognition, code review, document analysis), incremental build-up of a solution, heterogeneous reasoning methods.

**Example** (document analysis):
- `EntityExtractor` activates when raw text exists on the board → writes entities
- `SentimentAnalyzer` activates when raw text exists → writes sentiment
- `SummarySynthesizer` activates only after both EntityExtractor and SentimentAnalyzer have written → writes final summary

**Pitfall**: Without a final-decision-maker, agents can overwrite each other indefinitely. Always set a recursion limit or supervisor veto.

#### Group Chat / Roundtable

All agents participate in a **shared accumulating conversation thread**. A chat manager decides which agent responds next. Human observers can inject at any point. All agents see the full history.

**Modes**:
- **Collaborative brainstorming**: free-flow, manager picks most relevant agent
- **Maker-checker loop**: maker creates, checker evaluates against acceptance criteria, cycles until pass or iteration cap hit

**When to use**: Decisions requiring debate and consensus, quality validation loops (code review, compliance), creative workflows needing multiple perspectives on shared material.

**Pitfall**: Conversation overhead is expensive. Limit to 3 agents max to maintain control. Not suited for real-time or latency-sensitive tasks.

#### Sequential Pipeline

Agents execute in a predefined linear order. Each agent processes the previous agent's output and passes its result forward. No parallelism; deterministic ordering.

```text
Input → Agent 1 → Agent 2 → Agent 3 → Output
```

**When to use**: Progressive refinement (draft → review → polish), data transformation with clear stage dependencies, workflows where order is critical.

**Pitfall**: Early stage failures propagate and worsen downstream. Not suitable when stages could be parallelized without loss.

#### Handoff / Routing

Control transfers dynamically from one agent to another based on current context. The receiving agent decides whether to handle the task itself or pass to a more appropriate peer. Agents execute one at a time (not parallel).

**When to use**: Customer support triage, dynamic routing where optimal agent is unknown at start, tasks where specialization only becomes apparent during processing.

---

### Technical Mechanisms for Communication

#### Filesystem Mailboxes (Disk-Based Message Passing)

The simplest coordination primitive: agents write JSON/JSONL messages to named files in a shared directory; other agents poll or watch those files.

**Claude Code agent teams** use:
- `~/.claude/teams/{team-name}/config.json` — runtime state, session IDs, tmux pane IDs
- `~/.claude/tasks/{team-name}/` — shared task list with file locking
- Per-agent inbox files (JSON arrays or JSONL) with automatic delivery

**OpenCode** (open-source re-implementation) uses:
- `team_inbox/<projectId>/<teamName>/<agentName>.jsonl` — append-only for O(1) writes
- `markRead` batching: file rewrites only on prompt loop completion, not per message
- Messages inject as synthetic user messages triggering `autoWake` on idle agents

**MCP Agent Mail** uses SQLite + Git:
- Persistent adjective+noun identities per project (e.g., `GreenCastle`, `BlueLake`)
- Messages stored outside context windows; full-text search via SQLite FTS5
- Git-backed archive: every message is a commit, enabling audit trails and replay
- File reservation leases on glob patterns with TTL expiration + optional pre-commit enforcement

**Atomic write pattern** (prevents partial reads):
```bash
# Write to temp file, then atomic rename
echo '{"from":"agent-a","to":"agent-b","body":"done with auth module"}' \
  > inbox/agent-b/msg-$(date +%s%N).json.tmp
mv inbox/agent-b/msg-$(date +%s%N).json.tmp \
   inbox/agent-b/msg-$(date +%s%N).json
```

**File locking** for task claiming (prevents race conditions):
```bash
# Use flock to claim a task atomically
(flock -x 200
  # read, modify, write task state
) 200>/tmp/tasks.lock
```

#### tmux Send-Keys (Terminal-Native IPC)

tmux panes are persistent processes that survive disconnects. Agents communicate by sending keystrokes to named windows/panes.

**Basic pattern**:
```bash
# Send a message to another agent's pane
tmux send-keys -t session:window "Your task: refactor auth.rs" Enter

# With timing wrapper for reliability
./send-claude-message.sh session:window "Your task: refactor auth.rs"
```

**Three-pane "Adventuring Party" layout**:
```text
┌──────────────────┬──────────────────┬──────────────────┐
│   Codex (Review) │  Claude (Implement) │   Shell (Utils) │
│                  │                  │                  │
│  Deep reviews    │  Main work agent │  git, tests,     │
│  code diffs      │  reads Codex     │  linters         │
│                  │  feedback        │                  │
└──────────────────┴──────────────────┴──────────────────┘
```

**Three-tier orchestration**:
```text
Orchestrator (top-level tmux session)
  ├── PM-1 (project manager window)
  │     ├── Engineer-1 (sub-window)
  │     └── Engineer-2 (sub-window)
  └── PM-2 (project manager window)
        └── Engineer-3 (sub-window)
```

Self-scheduling via cron-like script:
```bash
./schedule_with_note.sh 30 "Continue implementation of auth module"
```

#### Message Queues and Event Streaming

For distributed or high-throughput systems, agents publish to topics and subscribe as consumer groups.

**Kafka-based patterns**:
- **Orchestrator-Worker**: key-based partitioned topics; worker consumer groups auto-rebalance as agents scale
- **Blackboard**: shared topic as async knowledge repository; agents append findings, others subscribe
- **Market-Based**: separate bid/ask topics with a market-maker agent; eliminates O(N²) direct connections

**AWS native stack**:
- SQS/EventBridge for agent-to-agent messaging
- DynamoDB/S3/OpenSearch as blackboard state store
- Step Functions for lifecycle orchestration (timeouts, retries)

#### The Agent2Agent (A2A) Protocol

Google's open standard (April 2025, now Linux Foundation) for cross-vendor, cross-framework agent interoperability.

**Agent Card** (JSON capability advertisement):
```json
{
  "name": "SecurityReviewAgent",
  "description": "Reviews code changes for security vulnerabilities",
  "capabilities": ["code-analysis", "cve-lookup", "sast"],
  "endpoint": "https://agents.example.com/security-review",
  "authentication": { "type": "bearer" }
}
```

**Task lifecycle**:
```text
Client Agent  →  [Task: submitted]  →  Remote Agent
              ←  [Task: running]    ←
              ←  [Task: completed]  ←
```

**Transport**: HTTP + SSE (streaming updates) + JSON-RPC

**Version 0.3 additions**: gRPC support, signed Agent Cards, extended Python SDK

A2A is the emerging standard for **inter-organization** or **cross-framework** agent collaboration; for single-machine or single-codebase setups, filesystem mailboxes or tmux are simpler and lower-latency.

---

### Shared Memory and State Management

The memory layer is where most live collaboration breaks down. Three fundamental approaches:

#### Shared Memory (Common Pool)

All agents read/write the same store (Redis, DynamoDB, a shared file).

**Pros**: Easy knowledge reuse, single source of truth
**Cons**: Requires coherence mechanisms; without them, agents overwrite each other or read stale data

**Coherence rules**:
- Append-only writes where possible (JSONL, event log)
- File locking or `SELECT ... FOR UPDATE SKIP LOCKED` for exclusive access
- Versioned writes with timestamps; readers check version before acting on data
- TTL expiration on entries to prevent stale reads

#### Distributed Memory (Per-Agent + Selective Sync)

Each agent maintains its own local memory; synchronizes selectively with peers.

**Pros**: Isolation, scalability, no single bottleneck
**Cons**: Risk of state divergence; explicit sync protocols required

**Pattern**: Agent writes to its own outbox → coordination layer routes to relevant inboxes → recipients process and acknowledge

#### Three-Layer Memory Hierarchy (Research Model)

Proposed in 2026 arxiv paper (arXiv:2603.10062):

| Layer | Analogy | Contents | Latency |
|-------|---------|----------|---------|
| I/O | Peripherals | External inputs/outputs | Variable |
| Cache | CPU cache | Active context, KV cache, embeddings | Fast |
| Memory | RAM / disk | Dialogue history, vector DB, doc store | Slower |

**Key gap**: No principled KV cache sharing protocol across agents exists yet. Memory consistency (when writes become visible to other agents) remains the primary unsolved challenge.

---

### Framework Comparison

| Framework | Coordination Model | State Sharing | Best For |
|-----------|-------------------|---------------|----------|
| **Claude Code Agent Teams** | Lead + teammates; shared task list; mailbox | JSON task files + inbox files on disk | AI coding teams, parallel code exploration |
| **LangGraph** | Graph nodes + conditional edges; supervisor subgraphs | Typed state channels threaded through graph | Structured workflows with complex routing logic |
| **CrewAI** | Role-based; hierarchical manager + workers | `task.context[]` threading; shared memory store | Role-play inspired teams, progressive task chains |
| **AutoGen** | Group chat; round-robin or selective response | Accumulating conversation history | Conversational collaboration, human-in-the-loop |
| **OpenAI Agents SDK** | Handoff chains; triage + specialists | Conversation history passed through transitions | Customer support, dynamic routing |
| **OpenAI Swarm** | Same as SDK; deprecated | Stateless client-side | Education/prototyping only (replaced by Agents SDK) |
| **A2A Protocol** | Client/remote task-based; discovery via Agent Card | Message parts; task artifacts | Cross-vendor, cross-framework interoperability |

---

## Collaboration Pattern Decision Tree

```text
Is the task parallelizable?
  NO  → Use sequential pipeline or single agent
  YES → Do workers need to see each other's intermediate outputs?
          NO  → Orchestrator / Worker (clean, simple)
          YES → Do you need convergence through debate?
                  YES → Group chat (≤3 agents) or Peer-to-peer with convergence rule
                  NO  → Do agents contribute independently to shared knowledge?
                          YES → Blackboard
                          NO  → Concurrent fan-out/fan-in
```

---

## Code Examples

### Claude Code: Start an Agent Team

```bash
# Enable agent teams
export CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1
# Or in ~/.claude.json / settings.json:
# { "env": { "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS": "1" } }

# Start Claude Code and describe the team you want
claude
# Then type:
# "Create an agent team with 3 teammates:
#   - One reviews security implications of PR #142
#   - One checks performance impact
#   - One validates test coverage"
```

```bash
# Force in-process mode (works in any terminal, no tmux required)
claude --teammate-mode in-process

# Navigate teammates
# Shift+Down  — cycle to next teammate
# Ctrl+T      — toggle shared task list
# Escape      — interrupt current teammate turn
```

### LangGraph: Supervisor + Workers

```python
from langgraph.graph import StateGraph, END
from typing import TypedDict, Literal

class TeamState(TypedDict):
    task: str
    research_output: str
    code_output: str
    review_output: str
    next: Literal["researcher", "coder", "reviewer", "FINISH"]

def supervisor(state: TeamState) -> TeamState:
    # LLM call with structured output to decide next agent
    # Uses state to route: e.g. if research_output empty → researcher
    ...

def researcher(state: TeamState) -> TeamState:
    # Appends to state["research_output"]
    ...

def coder(state: TeamState) -> TeamState:
    # Uses state["research_output"], writes to state["code_output"]
    ...

builder = StateGraph(TeamState)
builder.add_node("supervisor", supervisor)
builder.add_node("researcher", researcher)
builder.add_node("coder", coder)
builder.add_node("reviewer", reviewer)

builder.set_entry_point("supervisor")
builder.add_conditional_edges(
    "supervisor",
    lambda s: s["next"],
    {"researcher": "researcher", "coder": "coder",
     "reviewer": "reviewer", "FINISH": END}
)
# Each worker routes back to supervisor
for worker in ["researcher", "coder", "reviewer"]:
    builder.add_edge(worker, "supervisor")

graph = builder.compile()
```

### CrewAI: Hierarchical Process with Delegation

```python
from crewai import Agent, Crew, Task, Process
from langchain_openai import ChatOpenAI

researcher = Agent(
    role="Senior Researcher",
    goal="Find comprehensive information on the topic",
    allow_delegation=False,
)

analyst = Agent(
    role="Data Analyst",
    goal="Synthesize research into actionable insights",
    allow_delegation=True,
    allowed_agents=["Senior Researcher"],  # constrained delegation
)

writer = Agent(
    role="Technical Writer",
    goal="Produce clear documentation",
    allow_delegation=False,
)

research_task = Task(
    description="Research the topic in depth",
    agent=researcher,
    expected_output="Detailed research notes",
)

writing_task = Task(
    description="Write documentation based on research",
    agent=writer,
    context=[research_task],  # threads research_task output into this task
    expected_output="Draft documentation",
)

crew = Crew(
    agents=[researcher, analyst, writer],
    tasks=[research_task, writing_task],
    process=Process.hierarchical,
    manager_llm=ChatOpenAI(model="gpt-4o"),
    memory=True,   # caches delegation context; ~30% cost reduction
    verbose=True,
)

result = crew.kickoff(inputs={"topic": "async Rust patterns"})
```

### Filesystem Mailbox: Minimal Coordination Protocol

```python
import json, os, time, fcntl
from pathlib import Path

MAILBOX_DIR = Path(".coordination/mailbox")

def send_message(to_agent: str, from_agent: str, body: dict):
    """Atomic write to prevent partial reads."""
    inbox = MAILBOX_DIR / to_agent
    inbox.mkdir(parents=True, exist_ok=True)
    msg = {"from": from_agent, "ts": time.time(), "body": body}
    tmp = inbox / f".tmp-{time.time_ns()}.json"
    final = inbox / f"msg-{time.time_ns()}.json"
    tmp.write_text(json.dumps(msg))
    tmp.rename(final)  # atomic on POSIX

def read_messages(agent_name: str) -> list[dict]:
    """Read and consume all pending messages."""
    inbox = MAILBOX_DIR / agent_name
    messages = []
    for f in sorted(inbox.glob("msg-*.json")):
        messages.append(json.loads(f.read_text()))
        f.unlink()  # consume
    return messages

def claim_task(task_id: str, agent_name: str) -> bool:
    """File-locked task claiming to prevent race conditions."""
    lock_file = MAILBOX_DIR / "tasks.lock"
    tasks_file = MAILBOX_DIR / "tasks.json"
    with open(lock_file, "w") as lf:
        fcntl.flock(lf, fcntl.LOCK_EX)
        tasks = json.loads(tasks_file.read_text())
        if tasks.get(task_id, {}).get("status") == "pending":
            tasks[task_id]["status"] = "in_progress"
            tasks[task_id]["owner"] = agent_name
            tasks_file.write_text(json.dumps(tasks))
            return True
        return False
```

### tmux: Cross-Pane Agent Messaging

```bash
#!/usr/bin/env bash
# send-agent-message.sh  <session:window>  <message>
# Wraps timing complexity for reliable delivery

SESSION_WINDOW="$1"
MESSAGE="$2"

# Send message to target pane
tmux send-keys -t "$SESSION_WINDOW" "$MESSAGE" Enter

# Brief pause to let the agent's prompt loop restart
sleep 0.5
```

```bash
# Start three-pane collaboration session
tmux new-session -d -s collab -n main
tmux split-window -h -t collab:main     # right pane: reviewer
tmux split-window -v -t collab:main.1  # bottom-right: shell

# Label panes
tmux rename-window -t collab:main "claude-implement"
# In right pane, start reviewer agent
tmux send-keys -t collab:main.1 "codex" Enter

# Send task to reviewer
./send-agent-message.sh collab:main.1 \
  "Review the changes in auth.rs for security issues. Report to left pane."
```

### A2A Protocol: Agent Card and Task Request

```json
{
  "name": "CodeReviewAgent",
  "version": "1.0.0",
  "description": "Reviews pull requests for security, performance, and correctness",
  "url": "https://agents.example.com/code-review",
  "capabilities": {
    "streaming": true,
    "pushNotifications": false
  },
  "skills": [
    { "id": "security-review", "name": "Security Review" },
    { "id": "perf-review",     "name": "Performance Analysis" }
  ]
}
```

```python
# Client agent sending a task to a remote agent
import httpx

async def delegate_review(pr_diff: str, remote_agent_url: str):
    async with httpx.AsyncClient() as client:
        response = await client.post(
            f"{remote_agent_url}/tasks/send",
            json={
                "message": {
                    "role": "user",
                    "parts": [{"type": "text", "text": f"Review this diff:\n{pr_diff}"}]
                }
            }
        )
        task = response.json()
        # Poll or use SSE stream for updates
        return task["result"]["parts"][0]["text"]
```

---

## Common Pitfalls

| Pitfall | Why It Happens | How to Avoid |
|---------|---------------|--------------|
| **Context drift** | Parallel agents hold inconsistent views of shared state | Central task spec + scoped per-agent context + coordinator/arbiter agent |
| **Race conditions on shared files** | Two agents claim or write the same resource simultaneously | File locking (`flock`), append-only JSONL, or database row locking |
| **Runaway coordination loops** | Agents debate without a convergence mechanism | Set recursion limits, iteration caps, and a final-decision-maker node |
| **Error amplification (17x)** | Decentralized "bag of agents" without central control plane | Centralized orchestrator or Byzantine consensus for critical decisions |
| **Context window overflow** | Long-running teams accumulate too much history | External memory with selective retrieval; summarize instead of storing full history |
| **Self-approval / circular review** | Implementor agent also acts as its own reviewer | Separate implementor from reviewer; evidence JSON tied to diffs; evidence goes stale on change |
| **Too many agents** | Coordination overhead exceeds parallelism benefit | Start with 3-5 agents; gains plateau beyond 4 |
| **Task granularity wrong** | Tasks too small (overhead exceeds benefit) or too large (no check-ins) | Aim for self-contained units producing a clear deliverable; 5-6 tasks per teammate |
| **Prompt injection via shared state** | Malicious content written to blackboard poisons other agents | Treat all content read from shared state as untrusted; sanitize before including in prompts |
| **No session recovery** | Agent restarts don't restore in-flight coordination | Use persistent storage (Git-backed archives, SQLite) for all messages and task state |

---

## Best Practices

1. **Start with the lowest complexity level** (direct model call → single agent → multi-agent). Multi-agent systems add coordination overhead, latency, and failure modes. (Source: Microsoft Azure Architecture Center)

2. **Use a central shared task spec** as the single living source of truth for all agents. Every agent checks this doc before making a decision that affects shared scope. (Source: Lumenalta, 8 Tactics)

3. **Scope context per agent role** — each agent receives only the files, rules, and history relevant to its function. Avoid broadcasting the full conversation history to all agents. (Source: Lumenalta, Claude Code best practices)

4. **Store coordination state outside context windows** — use files, SQLite, or Git-backed archives for messages, task lists, and decisions. Storing state outside context windows prevents token budget exhaustion and enables recovery across restarts. (Source: MCP Agent Mail, Claude Code docs)

5. **Use file locking or append-only writes** for any shared resource two agents might touch simultaneously. Silent overwrites are the most common failure mode. (Source: Claude Code docs, OpenCode architecture)

6. **Separate the implementor from the reviewer** and never allow self-approval. Tie approval evidence to specific code diffs; invalidate evidence automatically when the diff changes. (Source: Adventuring Party / tmux guide)

7. **Set explicit convergence rules** for any peer-to-peer or blackboard system — a recursion limit, iteration cap, or final-decision-maker prevents infinite loops. (Source: Microsoft Azure patterns, Galileo)

8. **Monitor and steer proactively**. Do not let a team run unattended for more than a few minutes. Check in, redirect failing approaches, and synthesize findings as they come in. (Source: Claude Code docs, Anthropic multi-agent research)

9. **Size teams appropriately** — 3-5 agents with 5-6 tasks each balances parallelism with manageable coordination. Research shows diminishing returns beyond 4 agents. (Source: Galileo, Claude Code docs)

10. **Use structured output for routing decisions**. When a supervisor or orchestrator must decide which agent runs next, require structured JSON output (not free-form text) to avoid parsing failures. (Source: LangGraph 2026 best practices)

---

## Tool and Framework Selection Guide

| Need | Recommended Tool |
|------|-----------------|
| AI coding team on a single machine | Claude Code Agent Teams |
| Structured workflow with complex routing | LangGraph |
| Role-based team inspired by human orgs | CrewAI |
| Conversational / human-in-the-loop | AutoGen or OpenAI Agents SDK |
| Cross-vendor agent interoperability | A2A Protocol |
| Long-running autonomous agents (24/7) | tmux orchestrator |
| Coordination without consuming context | MCP Agent Mail |
| High-throughput distributed agents | Kafka-based event streaming |

---

## Further Reading

| Resource | Type | Why Recommended |
|----------|------|-----------------|
| [Claude Code Agent Teams Docs](https://code.claude.com/docs/en/agent-teams) | Official Docs | Complete reference for Claude Code's native agent team feature |
| [AI Agent Orchestration Patterns - Azure](https://learn.microsoft.com/en-us/azure/architecture/ai-ml/guide/ai-agent-design-patterns) | Architecture Guide | Comprehensive pattern catalog with examples and when-to-avoid guidance |
| [How Anthropic Built Its Multi-Agent Research System](https://www.anthropic.com/engineering/multi-agent-research-system) | Engineering Blog | First-person account of production lessons from orchestrator-worker systems |
| [A2A Protocol Specification](https://a2a-protocol.org/latest/specification/) | Specification | Formal spec for cross-framework agent interoperability |
| [The Adventuring Party: tmux Orchestration](https://dev.to/alexivison/the-adventuring-party-from-sub-agents-to-multi-agent-orchestration-with-tmux-2edf) | Tutorial | Practical guide to multi-agent coordination via tmux panes |
| [Multi-Agent Memory Architecture (arXiv:2603.10062)](https://arxiv.org/abs/2603.10062) | Research Paper | Computer-architecture framing of shared vs. distributed memory for agents |
| [LangGraph Hierarchical Agent Teams](https://langchain-ai.github.io/langgraph/tutorials/multi_agent/hierarchical_agent_teams/) | Tutorial | Code walkthrough of nested supervisor/worker graphs |
| [Blackboard Architecture for Multi-Agent Systems](https://notes.muthu.co/2025/10/collaborative-problem-solving-in-multi-agent-systems-with-the-blackboard-architecture/) | Tutorial | Deep dive into the classic blackboard pattern with modern LLM examples |
| [Multi-Agent Communication Patterns That Actually Work](https://dev.to/aureus_c_b3ba7f87cc34d74d49/multi-agent-communication-patterns-that-actually-work-50kp) | Tutorial | Practical comparison of 5 communication patterns with anti-patterns |
| [8 Tactics to Reduce Context Drift](https://lumenalta.com/insights/8-tactics-to-reduce-context-drift-with-parallel-ai-agents) | Best Practices | Actionable drift prevention tactics for parallel agent systems |
| [MCP Agent Mail](https://github.com/Dicklesworthstone/mcp_agent_mail) | Tool | Lightweight coordination infrastructure (identity, messaging, file leases) for coding agents |
| [Four Design Patterns for Event-Driven Multi-Agent Systems](https://www.confluent.io/blog/event-driven-multi-agent-systems/) | Tutorial | Kafka-based patterns: orchestrator-worker, hierarchical, blackboard, market-based |
| [OpenCode Agent Team Architecture](https://dev.to/uenyioha/porting-claude-codes-agent-teams-to-opencode-4hol) | Deep Dive | Low-level implementation details of JSONL mailboxes, auto-wake, dual state machines |

---

*Generated by /learn from 24 sources.*
*See `resources/live-collaboration-between-terminal-agents-sources.json` for full source metadata.*
