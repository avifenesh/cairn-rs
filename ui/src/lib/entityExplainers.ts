/**
 * entityExplainers — one-sentence descriptions of every first-class entity
 * surface in the operator UI (F32).
 *
 * Principle: an operator landing on a page cold should know what they are
 * looking at without clicking Help or switching to docs. Each explainer is a
 * single sentence (optionally followed by one distinguishing em-dash clause)
 * kept between roughly 60 and 160 characters. Accurate with respect to the
 * `cairn-domain` types — do not soften or restate a definition that drifts
 * from the data model.
 *
 * Rendering convention: a muted `<p>` at 11px, directly under the page
 * title (or inline in the page toolbar for dense layouts). Not a modal,
 * not a link, not collapsible — the text itself is the explanation.
 *
 * Tests in `ui/src/pages/__tests__/` lock each string so future refactors
 * do not silently delete them.
 */

export const ENTITY_EXPLAINERS = {
  // ── Scope hierarchy ────────────────────────────────────────────────────
  workspace:
    "Workspaces group Projects within a Tenant — each project has its own runs, memory, credentials, and policies.",
  project:
    "A Project is the innermost scope — every Run, Task, Memory, Credential, and Decision is owned by exactly one project.",

  // ── Execution surfaces ─────────────────────────────────────────────────
  run: "A Run is one orchestration session — an LLM loop dispatched against a model for this project. Different from Tasks (claimable units of work with leases).",
  runsList:
    "Orchestration sessions. Click a row to see its LLM calls, tool invocations, and state timeline.",
  session:
    "A Session groups multiple Runs under one conversation context — operators resume sessions to continue work across restarts.",
  sessionsList:
    "Conversation contexts that span Runs. Click a row to see its runs, messages, and cost rollup.",
  task: "Tasks are claimable units of work with leases — workers heartbeat to hold them. Different from Runs (orchestration sessions without leases).",

  // ── Decision layer ─────────────────────────────────────────────────────
  approval:
    "Approvals are operator-gated tool calls or plans that require human sign-off before execution. Different from Decisions (automatic policy outcomes).",
  decision:
    "Decisions record automatic policy outcomes (routing, admission, rate-limit). Different from Approvals (operator-gated tool calls and plans).",

  // ── Provider plumbing ──────────────────────────────────────────────────
  provider:
    "Provider Connections bind cairn to an LLM endpoint — one per (family, adapter, credential). Runs route through them via Settings defaults.",
  credential:
    "Credentials store secrets (API keys, tokens) per tenant. Provider Connections reference them by ID — the raw secret never leaves the server.",

  // ── Automation & agent config ──────────────────────────────────────────
  trigger:
    "Triggers fire runs on external events (GitHub webhook, schedule, signal). Each trigger is scoped to one project.",
  agentTemplate:
    "Agent Templates are reusable role definitions — prompt, tools, defaults — that Runs instantiate by ID.",
  prompt:
    "Prompt Releases are versioned prompt content bound to a project scope — runs reference releases by ID for reproducibility.",
  skill:
    "Skills are markdown instructions operators enable per-project — they become retrievable guidance the agent can invoke via the harness-skill tool.",

  // ── Knowledge & extension ──────────────────────────────────────────────
  memory:
    "Memory chunks are indexed, retrievable pieces derived from Sources — the agent retrieves them by relevance during a run.",
  source:
    "Sources are ingestable knowledge origins (URLs, files, repos) — cairn chunks and embeds them into Memory for retrieval.",
  plugin:
    "Plugins extend cairn with custom tools, integrations, and providers via the stdio JSON-RPC protocol.",
  integration:
    "Integrations connect cairn to external systems (GitHub, Linear, Notion, webhooks) — each surfaces its own tools and triggers.",
  eval: "Evals score prompt releases and agent behavior against fixed inputs — used to compare versions and detect regressions.",

  // ── Notification & observability ───────────────────────────────────────
  channel:
    "Channels deliver notifications to Slack, email, or webhooks — each channel binds to a project and a set of event filters.",
  notification:
    "Notifications are operator-visible events (stuck runs, approvals pending, budget alerts) routed through configured Channels.",
  trace:
    "Traces are per-run spans of LLM calls, tool invocations, and orchestrator steps — used for debugging and cost attribution.",
  auditLog:
    "Audit Log records every operator-visible mutation (approvals, credential writes, settings changes) with actor and timestamp.",
} as const;

export type EntityExplainerKey = keyof typeof ENTITY_EXPLAINERS;
