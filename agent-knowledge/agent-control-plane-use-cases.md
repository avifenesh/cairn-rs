# Agent Control Plane Use Cases: Real-World Production Patterns

> Research guide for cairn-rs product strategy and dogfooding.
> Sources: Anthropic engineering blog, Huyen Chip, Latent Space, Hamel Husain, LangChain/LangGraph docs, CrewAI docs, AutoGen/Microsoft Research, Langfuse, Letta, e2b-dev/awesome-ai-agents, and practitioner discussions.
> Date: 2026-04-08

---

## 1. Production Use Case Categories

### Tier 1: Proven in Production (Revenue-Generating)

**Customer Support Agents**
- Most mature production use case. Combines conversation with tool-enabled actions (data retrieval, refunds, ticket updates).
- Anthropic notes companies demonstrate viability through usage-based pricing.
- Requires: human-in-the-loop escalation, approval gates for write actions (refunds, account changes), session persistence across conversations, audit trail.
- **Cairn fit**: Strong. This is the canonical "long-running agent with approvals" scenario. The approval inbox, session/run lifecycle, and cost tracking are directly relevant.

**Coding Agents**
- Strongest product-market fit. Graham Neubig (Latent Space): "I use coding agents maybe 5-10 times a day."
- Three proven patterns: data analysis/visualization, new development using APIs, iterative improvement of existing code.
- Verifiable through automated tests, enabling iteration loops.
- Requires: tool execution (file/git/shell), checkpointing (save progress on long tasks), observability (what did the agent change and why?).
- **Cairn fit**: Good for the orchestration/observability layer. Less relevant for the coding logic itself.

**Data Pipeline / Analysis Agents**
- SQL generation, data extraction, report building, ETL orchestration.
- Requires: tool calling (DB queries, API calls), result validation, human review of outputs before downstream actions.
- **Cairn fit**: Good. Multi-step runs with validation gates.

### Tier 2: Growing Adoption

**Internal Tooling Agents**
- Engineering teams building agents for internal workflows: PR triage, incident response, documentation generation, onboarding.
- Often the "first hour" use case for teams evaluating agent infrastructure.
- Requires: integration with existing tools (GitHub, Slack, Jira), approval workflows, memory (learn from past incidents).
- **Cairn fit**: Excellent dogfood candidate. Low risk, high learning value.

**Research / Knowledge Agents**
- Ingest documents, answer questions, synthesize information across sources.
- Requires: document ingestion pipeline, retrieval with scoring, source attribution, memory.
- **Cairn fit**: Directly exercises cairn-memory, retrieval scoring, and the knowledge pipeline.

**Multi-Agent Coordination**
- Orchestrator delegates to specialized sub-agents (researcher, coder, reviewer).
- AutoGen: top performance on GAIA benchmark using specialized agent teams.
- CrewAI: sequential and hierarchical process models with task delegation.
- Requires: session/run hierarchy, sub-agent spawning, inter-agent messaging, result aggregation.
- **Cairn fit**: Exercises cairn-agent, cairn-channels, and the subagent model.

### Tier 3: Emerging / Experimental

**Autonomous Workflow Agents**
- Long-running agents that operate independently for hours/days (monitoring, data collection, scheduled tasks).
- Requires: durable execution, checkpoint/resume, recovery from failures, scheduled triggers.
- **Cairn fit**: Core thesis. This is where event-sourcing and recovery shine.

**Agentic Products (Customer-Facing)**
- AI features embedded in SaaS products where end-users interact with agents.
- Requires: multi-tenant isolation, cost metering per customer, approval policies, prompt versioning per use case.
- **Cairn fit**: Strong alignment with multi-tenant model and commercial packaging.

---

## 2. How Competitors Are Actually Used (Honest Assessment)

### LangGraph / LangSmith (LangChain)
- **What it is**: Agent runtime (LangGraph) + observability/eval platform (LangSmith).
- **Production features**: Fault-tolerant task queues, Postgres checkpointer, human-in-the-loop breakpoints, streaming, cron jobs, double-texting handling (reject/queue/interrupt/rollback).
- **Strengths**: Strong graph-based agent definition. LangGraph Cloud handles deployment infra. Used by Klarna, Rippling, Lyft, Harvey, Elastic.
- **Pain points**: LangChain (the library) is widely criticized for over-abstraction. Anthropic explicitly warns: "Start with raw LLM APIs rather than frameworks. Complex abstractions obscure the underlying prompts and responses, making them harder to debug." Many teams adopt LangGraph but avoid the rest of the LangChain ecosystem.
- **Self-hosted**: LangGraph is MIT-licensed. LangSmith Cloud is the default; self-hosting is possible but less documented.
- **Cairn overlap**: Direct competitor on the runtime side. Cairn's advantage is event-sourcing (replay/audit) and being a single coherent product vs. LangChain's sprawling ecosystem.

### Braintrust
- **What it is**: Eval platform with prompt versioning, tracing, and production monitoring.
- **Strengths**: Clean eval workflow, dataset management, prompt comparison.
- **Pain points**: Focused on eval/observability only. Not a runtime. Teams still need a separate execution layer.
- **Cairn overlap**: Cairn's eval framework + prompt registry compete directly. Cairn's advantage is integration with the runtime (eval tied to actual runs, not separate).

### Portkey
- **What it is**: AI gateway with routing, fallback, guardrails, cost tracking.
- **Strengths**: Clean provider abstraction, budget controls, request caching.
- **Pain points**: Gateway-only. No runtime, no memory, no agent orchestration.
- **Cairn overlap**: Cairn's provider abstraction covers similar ground but explicitly not trying to be a standalone gateway product.

### CrewAI
- **What it is**: Multi-agent orchestration framework (Python).
- **Production features**: Sequential/hierarchical process models, memory (short/long/entity), caching, rate limiting, replay from checkpoints.
- **Strengths**: Simple mental model (crews/agents/tasks), good for structured multi-agent workflows.
- **Pain points**: Python-only. Framework, not infrastructure. No built-in persistence, observability, or approval workflows. Teams must add their own production infrastructure.
- **Cairn overlap**: Cairn could be the infrastructure layer that CrewAI-style agents run on.

### AutoGen (Microsoft)
- **What it is**: Multi-agent conversation framework.
- **Strengths**: Flexible agent coordination, strong research backing (GAIA benchmark).
- **Pain points**: "Determining optimal multi-agent workflow configurations remains an open research question." Progress detection (stalling agents), debugging, and scaling are acknowledged challenges.
- **Self-hosted**: Open source, but no production infrastructure story.
- **Cairn overlap**: Same as CrewAI. Cairn is the control plane, not the agent logic.

### Letta (formerly MemGPT)
- **What it is**: Stateful agent platform focused on persistent memory.
- **Strengths**: Memory-first architecture, context management for long-running agents.
- **Pain points**: Narrow focus. Memory without runtime, evals, or operator control.
- **Cairn overlap**: cairn-memory competes. Cairn's advantage is memory integrated with runtime, graph, and evals.

### Langfuse
- **What it is**: Open-source LLM observability (traces, evals, prompt management).
- **Strengths**: MIT-licensed, self-hostable, hierarchical trace visualization, cost tracking, prompt versioning with A/B testing.
- **Pain points**: Observability-only. No runtime or execution.
- **Self-hosted**: Strong self-hosted story. Direct appeal to compliance-conscious teams.
- **Cairn overlap**: Cairn's event log and SSE streaming provide similar observability built into the runtime. Langfuse could be a complementary tool or a competitive reference for the observability UX.

### Vellum
- **What it is**: Prompt/workflow release management with approval workflows.
- **Strengths**: Protected releases, release reviews, deployment controls.
- **Pain points**: Prompt management only. No runtime or memory.
- **Cairn overlap**: cairn-evals prompt registry competes directly. Cairn integrates release management with actual execution.

---

## 3. Common Pain Points (What Breaks in Production)

### Infrastructure Fragmentation ("The Stitching Problem")
- Teams routinely use 3-5 separate tools: a framework (LangGraph/CrewAI), an observability tool (LangSmith/Langfuse), an eval platform (Braintrust), a gateway (Portkey), plus custom glue.
- Each tool has its own data model, auth, and deployment. Correlating a trace in Langfuse with an eval in Braintrust with a run in LangGraph requires manual effort.
- **Cairn's thesis**: One system instead of five. This is the core value proposition.

### Planning Failures
- Huyen Chip: "Debate persists about whether autoregressive models can truly plan."
- Leading failure mode: agents hallucinate invalid tools or incorrect parameters.
- Fix: validate plans before execution, log everything, build lightweight heuristics.
- **Cairn implication**: Tool invocation recording and permission gates are essential, not optional.

### Observability Gaps
- Anthropic: "Explicitly expose agent planning steps and reasoning to maintain trust and debuggability."
- Hamel Husain: "You can never stop looking at data. No free lunch exists."
- Teams need to see: what the agent planned, what tools it called, what parameters it used, what results came back, why it made each decision.
- **Cairn implication**: The event log + SSE streaming + graph provenance are differentiators if the dashboard makes them accessible.

### Cost Overruns
- Reflection patterns multiply token costs. Multi-agent systems can run up costs quickly.
- Teams need per-run, per-session, per-tenant cost tracking.
- **Cairn implication**: Cost tracking at run/session/tenant level is table stakes.

### Recovery and Durability
- Long-running agents fail mid-execution. Without checkpointing, work is lost.
- LangGraph addresses this with Postgres checkpointer. Most frameworks don't.
- **Cairn implication**: Checkpoint/resume/recovery is a genuine differentiator for long-running workflows.

### Human-in-the-Loop Friction
- Anthropic: Build for human oversight. Define which operations require approval. Write actions especially need controls.
- Most frameworks treat HITL as an afterthought. Approval gates are either missing or bolted on.
- **Cairn implication**: First-class approval workflows are a real competitive advantage. The approval inbox with context and audit trail is the operator's primary interface for safety-critical agents.

### Eval Maturity Gap
- Hamel Husain's three-level hierarchy: unit tests → human/model eval → A/B testing.
- Most teams skip level 1 entirely. Generic eval frameworks underperform domain-specific ones.
- "Start simple. Use existing infrastructure before purchasing specialized tools."
- **Cairn implication**: The eval framework should be practical and incremental, not academic. Start with simple pass/fail assertions tied to runs, build toward scorecards.

---

## 4. Self-Hosted vs. Hosted Preferences

### Why Teams Self-Host
- **Compliance**: Regulated industries (finance, healthcare, government) cannot send data to vendor clouds.
- **Cost control**: Hosted platforms charge per-trace or per-seat. At scale, self-hosted is cheaper.
- **Data ownership**: Teams want full control over logs, traces, and training data.
- **Latency**: On-prem or VPC deployment reduces round-trip to model providers.
- **Customization**: Self-hosted allows deep integration with existing infrastructure.

### Why Teams Choose Hosted
- **Speed to start**: No deployment, no ops burden.
- **Managed upgrades**: Vendor handles infrastructure evolution.
- **Small team**: 1-3 person teams can't justify ops overhead.

### Langfuse's Model (Instructive for Cairn)
- MIT-licensed, full-featured self-hosted version.
- Cloud option for teams that prefer managed.
- No feature gating between self-hosted and cloud.
- This is essentially Cairn's RFC 014 model: one binary, self-hosted-first, paid differentiation through support/governance.

---

## 5. Adoption Patterns

### The Typical Journey
1. **Solo experiment** (hours): Developer tries a framework, builds a simple agent, sees it work.
2. **First real use case** (days): Wire the agent into an actual workflow. Hit the first production problems (no persistence, no observability, no cost tracking).
3. **Infrastructure discovery** (weeks): Realize you need observability, eval, and deployment tooling. Start evaluating LangSmith, Langfuse, Braintrust, etc.
4. **The stitching phase** (months): Integrate 3-5 tools. Build custom glue. Maintain it.
5. **Consolidation desire**: Teams start looking for "one system" that handles multiple concerns.

### The "First Hour" Problem
- Most agent platforms fail here. Complex setup, unclear getting-started path, no immediate value.
- Anthropic's advice: "Find the simplest solution possible, and only increase complexity when needed."
- **Cairn implication**: The solo dogfood must work in under 30 minutes. `cargo run -p cairn-app`, open dashboard, create a session, see something useful.

### The "Day 2" Problem
- After the demo works, teams ask: how do I deploy this? How do I add auth? How do I monitor costs? How do I add approval workflows?
- This is where most frameworks lose teams. The gap between "demo" and "production" is too wide.
- **Cairn implication**: Day 2 should feel like turning knobs, not rebuilding. Switch from SQLite to Postgres. Add team auth. Enable approval gates. Same product, different mode.

---

## 6. The Stitching Problem (Deep Dive)

A typical production agent stack today looks like:

| Concern | Tool | Pain |
|---------|------|------|
| Agent logic | LangGraph / CrewAI / custom | Framework lock-in, no standard |
| Observability | LangSmith / Langfuse | Separate deployment, separate data model |
| Evals | Braintrust / custom | No connection to runtime traces |
| Gateway/routing | Portkey / custom | Another hop, another system |
| Memory | Letta / pgvector / custom | Siloed from execution context |
| Prompt management | Vellum / custom | No connection to eval results |
| Approval workflows | Custom / nothing | Usually missing entirely |
| Cost tracking | Each tool's own | No unified view |

**The result**: 6-8 systems, 6-8 auth configs, 6-8 deployment concerns, no unified audit trail, no way to ask "why did the agent do this?" across the full stack.

**Cairn's answer**: One system that owns runtime + memory + evals + prompts + approvals + observability + cost tracking. The integration IS the product.

---

## 7. Implications for Cairn Dogfooding

### Recommended Solo Dogfood Scenario (First Few Hours)

**Use case: Internal Knowledge Agent**

Why this scenario:
- Exercises the distinctive parts of the product (memory + retrieval + approval + dashboard)
- Low risk (read-only agent, approval gates on any write actions)
- Immediately useful (answer questions about cairn-rs itself or another codebase)
- Tests the "first hour" experience

Steps:
1. Start cairn-rs locally (`cargo run -p cairn-app`)
2. Open the operator dashboard
3. Ingest a doc set via `POST /v1/memory/ingest` (use cairn-rs's own docs/RFCs)
4. Create a session, start a run that takes a user question
5. Agent retrieves from memory, proposes an answer
6. Add an approval gate: agent must get operator approval before responding with high-confidence claims
7. Use the dashboard to approve/reject, observe the event stream, check cost tracking
8. Deliberately break something (bad doc, wrong model config) and see how the dashboard surfaces the failure

**What to observe**:
- How long from `cargo run` to first useful interaction?
- Does the dashboard tell you what's happening without reading code?
- Can you debug a failed run from the dashboard alone?
- Does the approval workflow feel natural or ceremonial?
- What's missing that would make you trust this for a real workflow?

### Recommended Team Dogfood Scenario (Few Days)

**Use case: Multi-Agent Code Review Pipeline**

Why this scenario:
- Exercises multi-tenant (workspace per team member)
- Exercises sub-agents (analyzer agent, reviewer agent, summarizer agent)
- Exercises approval workflows (reviewer approves before merge recommendation)
- Exercises evals (compare different prompt versions for review quality)
- Requires Postgres (real persistence, concurrent users)
- Generates real cost data across multiple LLM calls

Steps:
1. Deploy with Postgres, team mode
2. Each team member gets a workspace
3. Agent ingests a PR diff, runs analysis, produces review
4. Operator approves or rejects the review
5. Compare review quality across different prompts using eval scorecards
6. Monitor costs per session/run

---

## 8. Key Takeaways for Cairn Product Strategy

1. **The wedge is real**: Teams ARE frustrated with tool fragmentation. The "one coherent system" pitch resonates.

2. **Customer support and internal tooling are the entry points**: These use cases are proven and directly exercise Cairn's strengths (approvals, memory, persistence, cost tracking).

3. **"Start simple" is non-negotiable**: Anthropic, Huyen Chip, and Hamel Husain all converge on this. The first experience must be simple. Complexity should be opt-in.

4. **Approval workflows are genuinely differentiated**: Most competitors don't have them. The ones that do (Vellum for prompts, LangGraph for breakpoints) treat them as features, not core product identity.

5. **Self-hosted is a real market**: Langfuse proves it. Compliance-driven teams will pay for a self-hosted control plane.

6. **Evals must be practical, not academic**: Hamel's advice — start with unit tests and manual inspection, not sophisticated scoring frameworks.

7. **The dashboard is the product**: For operators, the API is invisible. The dashboard's ability to answer "what happened and why?" is what makes Cairn a product vs. infrastructure.

8. **Don't compete on agent logic**: Cairn is the control plane, not the brain. Let teams bring their own agent patterns (ReAct, CrewAI-style, custom). Own the runtime, observability, and operational layer.
