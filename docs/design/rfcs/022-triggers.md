# RFC 022: Triggers — Binding Signals to Runs

Status: draft
Owner: runtime/orchestrator lead
Depends on: [RFC 002](./002-runtime-event-model.md), [RFC 005](./005-task-session-checkpoint-lifecycle.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 015](./015-plugin-marketplace-and-scoping.md), [RFC 018](./018-agent-loop-enhancements.md), [RFC 019](./019-unified-decision-layer.md)

## Summary

A **Trigger** is a project-scoped declarative rule that says: "when a signal of type X arrives matching condition Y, create a run from template Z." It is the missing entity between cairn's signal ingestion (RFC 015 plugin marketplace + RFC 017 GitHub reference plugin) and run creation (RFC 005 / RFC 018). Without it, every plugin would have to call `POST /v1/runs` directly, leaking run-creation policy into plugin code and giving operators no central place to control how external events become work.

A **RunTemplate** is the parallel concept: a reusable run configuration (mode, initial system prompt, tool allowlist, default policies, default budget) that a Trigger references. Templates are the cleanest place to record "what kind of agent should respond to this kind of signal" without coupling that decision to either the plugin (which doesn't know about cairn's runs) or the orchestrator (which doesn't know which signal types matter).

This RFC defines:

- The `Trigger` entity, lifecycle, and event model
- The `RunTemplate` entity, lifecycle, and event model
- The trigger evaluator: how signals are matched to triggers and how runs are created
- A small condition language for matching (key-equals + label-set membership in v1; expanding later if needed)
- Loop prevention to keep one trigger from cascading into a feedback storm
- Runaway protection (per-trigger rate limits, per-project trigger budgets)
- Operator surface (project settings → triggers tab)

This RFC does not define new tools, new providers, new sandbox concepts, or new protocol surfaces. Everything in it is composition over existing primitives.

## Why

### The dogfood demo critical path needs this

The simplest credible demo (from `agent-knowledge/control-plane-requirements.md`) is:

> Repo with failing test → issue labeled `cairn-ready` → webhook → agent clones, fixes, opens draft PR

The path from `webhook arrives` to `agent starts working` requires a binding: someone has to say "when this kind of signal happens in this project, create a run with this template." Today, RFC 015 routes signals to subscribers but the subscribers are not specified. RFC 017 normalizes GitHub events into signals but stops there. RFC 005 and RFC 018 define what a run is, but nothing creates one in response to a signal.

The Trigger is that binding. Without it, the dogfood demo doesn't close.

### Why a first-class entity, not a tool or a plugin config

Three alternative locations for the rule were considered and rejected:

- **Project settings table** (the simplest option): "when signal type X arrives, run template Y". Works for the simplest case but has no lifecycle, no event log, no per-trigger health monitoring, no runaway protection, no operator audit beyond a config diff. Treats triggers as configuration, not as state. Becomes painful as soon as the operator has more than a few of them and wants to ask "which trigger fired this run?"
- **Plugin manifest declares triggers**: forces plugins to know about cairn's runs. Plugins are supposed to be black boxes that emit signals and answer tool calls. Adding "create runs of these shapes" to the plugin's responsibility couples plugin code to product logic.
- **Plugin creates runs directly via API**: plugin authors implement run creation themselves. Worse than the manifest option — every plugin reinvents the wheel, tools cannot enforce per-project policy uniformly, no audit trail for "who configured this auto-creation".

The Trigger as a first-class event-sourced entity is the right line. It fits naturally next to existing concepts (Approvals, Sessions, Runs, Tasks): a project-scoped, lifecycle-managed, event-emitting, operator-visible thing.

### What makes this RFC small

Triggers do not require new tools, new providers, new sandbox features, or protocol changes. They compose entirely over what cairn already has plus the Phase 2 RFCs. Implementation is mostly schema, projection, and one new background worker that listens on the signal router and fans out to runs.

## Scope

### In scope for v1

- `Trigger` entity, persisted in the event log
- `RunTemplate` entity, persisted in the event log
- Trigger evaluator: a runtime worker that subscribes to the signal router (RFC 015) and creates runs for matching triggers
- Condition matching: key-equals on payload fields plus label-set membership (covers GitHub label-based flows and most webhook scenarios)
- Loop prevention: signals carry `source_run_id` (sealed RFC 017); the evaluator resolves the source run's chain depth and refuses to fire when `depth + 1` exceeds the trigger's `max_chain_depth`
- Runaway protection: per-trigger rate limits (max N runs per minute), per-project trigger budget (max M runs per hour from triggers)
- Triggers integrate with RFC 019's decision layer: creating a run from a trigger is a decision request that goes through policy + budget + (optionally) approval before proceeding
- Operator surface: project settings → triggers tab; create, edit, enable, disable, inspect hits, see most recent fires
- Events for every state transition
- Integration with RFC 015's signal router for delivery
- Integration with RFC 005 / RFC 018 for the actual run creation

### Explicitly out of scope for v1

- A general-purpose policy DSL (cairn's existing guardrail rules apply at the decision layer; this RFC does not add a parallel rule engine)
- Trigger composition (a trigger that watches multiple signal types) — v1 is one signal type per trigger
- Trigger templates that themselves spawn child triggers
- Triggers that mutate other triggers
- Cross-project trigger fan-out (a signal received in project A creating a run in project B)
- Triggers that fire based on internal cairn events (e.g. "when run X completes, fire trigger Y") — that's a workflow concept; this RFC stays at the external-signal boundary
- Scheduled triggers (cron) — these are valuable but live in their own future RFC
- Temporal correlation (e.g. "when signal A arrives within 5 minutes of signal B")
- A graphical trigger builder UI in the operator dashboard

## The Trigger Entity

```rust
pub struct Trigger {
    pub id: TriggerId,
    pub project: ProjectKey,
    pub name: String,                      // operator-facing display name
    pub description: Option<String>,
    pub signal_pattern: SignalPattern,     // which signals this trigger matches
    pub conditions: Vec<TriggerCondition>, // additional match conditions on the payload
    pub run_template_id: RunTemplateId,    // which template to instantiate
    pub state: TriggerState,
    pub rate_limit: RateLimitConfig,
    pub max_chain_depth: u8,               // prevent infinite cascades
    pub created_by: OperatorId,
    pub created_at: u64,
    pub updated_at: u64,
}

pub enum TriggerState {
    Enabled,
    Disabled { reason: Option<String>, since: u64 },
    Suspended { reason: SuspensionReason, since: u64 },
}

pub enum SuspensionReason {
    RateLimitExceeded,
    BudgetExceeded,
    RepeatedFailures { failure_count: u32 },
    OperatorPaused,
}

pub struct SignalPattern {
    /// Signal type (e.g. "github.issue.labeled"). Exact match in v1.
    /// Wildcards (e.g. "github.issue.*") are deferred to a later version.
    pub signal_type: String,

    /// Optional plugin ID restriction. If set, only signals from this plugin
    /// match. If None, signals of this type from any plugin match (rare —
    /// most v1 deployments will set this).
    pub plugin_id: Option<String>,
}

pub enum TriggerCondition {
    /// JSON path equals value: payload.action == "labeled"
    Equals { path: String, value: serde_json::Value },

    /// JSON path's array contains a value: payload.labels[].name contains "cairn-ready"
    Contains { path: String, value: serde_json::Value },

    /// JSON path is non-null
    Exists { path: String },

    /// Negate a child condition
    Not(Box<TriggerCondition>),
}

pub struct RateLimitConfig {
    pub max_per_minute: u32,        // hard cap
    pub max_burst: u32,             // token bucket capacity
}
```

**Why this small DSL and not a general expression language**: cairn does not need CEL or JSONPath in v1. The vast majority of trigger use cases reduce to "this signal type, this label, this repo". A handful of `Equals` + one `Contains` covers GitHub issue labeling, PR comments mentioning specific phrases, Slack message keywords, Linear ticket priorities, etc. The DSL is intentionally narrow so the operator UI can render it as a form rather than a code editor. Future versions can add more operators (`Matches` for regex, `In` for set membership, `GreaterThan` for numeric) without breaking existing triggers.

### Trigger lifecycle

```
[Initial] ── create() ──▶ [Enabled]
                            │
                  ┌─────────┴─────────┐
                  │                   │
        disable() │                   │ rate limit /
                  ▼                   │ failures
            [Disabled]            [Suspended]
                  │                   │
        enable()  │                   │ resume() / cooldown
                  └────────┬──────────┘
                           ▼
                       [Enabled]
                           │
                       delete()
                           │
                           ▼
                       [Deleted]
```

States:

- **Enabled**: actively listening for matching signals
- **Disabled**: explicitly turned off by an operator; not listening
- **Suspended**: automatically turned off due to rate limit, budget, or repeated failures; waits for cooldown or manual operator intervention
- **Deleted**: tombstone record; the trigger is gone from active state but the event log preserves history

### Trigger fire lifecycle

```
Signal arrives in signal router
  ↓
Signal router fans out to enabled subscribers (RFC 015).
**Note**: per sealed RFC 015, the signal router's per-project `signal_allowlist` filter runs BEFORE the trigger evaluator. A signal type not in the project's `signal_allowlist` is dropped at the router and never reaches the evaluator. Operators must ensure the signal types their triggers match are included in the project's plugin enablement `signal_allowlist`.
  ↓
Trigger evaluator (a runtime worker) sees the signal
  ↓
For each enabled trigger in the signal's project:
    1. Match SignalPattern (exact signal type + optional plugin_id)
    2. Match all TriggerConditions (JSON path checks against payload)
    2a. **Fire ledger dedup check**: look up `(trigger_id, signal_id)` in the durable `TriggerFireLedger`. If already fired (from a webhook retry, signal replay, or evaluator restart), emit `TriggerSkipped { reason: AlreadyFired }` and skip. This is a DIFFERENT layer from RFC 017's webhook-ingress dedup (`WebhookDeliveryReceived`): ingress dedup prevents duplicate signals from entering the router; fire-ledger dedup prevents the same signal from creating duplicate runs through the same trigger.
    3. If both match and fire ledger says not-yet-fired, attempt to fire:
       a. Check rate limit (token bucket)
       b. Check max_chain_depth (prevent feedback loops)
       c. Submit a DecisionRequest to RFC 019's decision layer
          (kind: TriggerFire, scope: project)
       d. If decision is Approved: create run from template,
          emit TriggerFired event with run_id
       e. If decision is Denied: emit TriggerDenied event
       f. If decision requires approval: emit TriggerPendingApproval event,
          run is not created until human/guardian approves
       g. If rate limit exceeded: emit TriggerRateLimited event,
          do not create run
    4. Move to next trigger
```

A single signal can fire multiple triggers in the same project — they fan out. Triggers do not know about each other; the evaluator processes them independently.

### Loop prevention via `trigger_chain_depth`

Every run carries a `trigger_chain_depth: u8` field, default 0:

- A run created manually (operator clicks "Run") has depth 0
- A run created by a trigger fired from an external signal has depth 1
- If a run produces an event that the external system observes and webhooks back, the resulting trigger fire would create a run at depth 2
- If that run does the same, depth 3
- The trigger checks: resolve `source_run_id` from the signal envelope → look up the source run's `trigger_chain_depth` → if `source_depth + 1 > trigger.max_chain_depth`, refuse to fire

**Source attribution uses `source_run_id`**, the standard field on the signal normalization envelope defined in sealed RFC 017 (line 249). Plugins set `source_run_id` when they can detect the signal's origin is a cairn action (e.g. the GitHub plugin checks bot-user authorship per sealed RFC 017 §Signal normalization). The evaluator resolves the prior run's depth from durable run state and computes `next_depth = prior_depth + 1`.

For signals with no identifiable source (`source_run_id: null` — a human pushed a commit, an unrelated webhook), depth is 1 (external origin).

The default `max_chain_depth` is 5 (configurable per trigger, hard cap 10). This is enough for a normal work cycle (issue → trigger → run → PR → review-comment → trigger → revision-run → PR-update → trigger → final-run → feedback) without enabling indefinite cascades.

### Runaway protection

Triggers can fire too often. Two layers of protection:

1. **Per-trigger rate limit** (token bucket): `max_per_minute` and `max_burst`. If a trigger would fire more often than its bucket allows, the excess fires emit `TriggerRateLimited` events and are dropped (not queued — queueing would just delay the storm). Default: 10/min, burst 20.

2. **Per-project trigger budget**: a project-wide budget on total runs created by triggers per hour, defaulting to `100 runs/hour` (operator configurable). If exceeded, all triggers in the project enter `Suspended { reason: BudgetExceeded }` until a cooldown (default 15 minutes) or operator intervention.

If a trigger enters `Suspended { reason: RepeatedFailures }` after 5 consecutive run-creation failures (e.g. the template references a deleted template, or the project's run quota is exhausted), the operator is notified via the existing notification channels.

## The RunTemplate Entity

```rust
pub struct RunTemplate {
    pub id: RunTemplateId,
    pub project: ProjectKey,
    pub name: String,
    pub description: Option<String>,

    /// Default run mode (Direct, Plan, Execute) per RFC 018
    pub default_mode: RunMode,

    /// Initial system instruction the agent receives
    pub system_prompt: String,

    /// Optional initial user message; if None, the trigger may inject one
    /// derived from the signal payload
    pub initial_user_message: Option<String>,

    /// Allowlist of plugins this run is allowed to use (composes with the
    /// project's enabled plugins from RFC 015)
    pub plugin_allowlist: Option<Vec<String>>,

    /// Allowlist of tools (subset of plugin_allowlist's combined tools)
    pub tool_allowlist: Option<Vec<String>>,

    /// Default budget caps for runs created from this template
    pub budget: TemplateBudget,

    /// Default approval policy (which actions need human/guardian approval)
    pub approval_policy_id: Option<ApprovalPolicyId>,

    /// Sandbox policy hint (RFC 016) — what kind of sandbox to provision
    pub sandbox_hint: Option<SandboxPolicyHint>,

    /// Fields that must be present in the signal payload for substitution.
    /// If any required field is missing, the trigger emits
    /// TriggerSkipped { reason: MissingRequiredField } and does not fire.
    pub required_fields: Vec<String>,

    pub created_by: OperatorId,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct TemplateBudget {
    pub max_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_iterations: Option<u32>,
    pub exploration_budget_share: Option<f32>,  // RFC 018: how much of the budget Plan mode uses
}
```

A template is just configuration; it has no runtime state. The lifecycle is `create → updates → delete`.

When a trigger fires, the evaluator copies the template into a fresh `RunRecord` (RFC 005), substituting variables from the signal payload into the system prompt and initial user message. Substitution is simple `{{path.to.field}}` syntax — no template engine, no logic.

### Templates are project-scoped

A template lives in exactly one project. Cross-project sharing is out of scope for v1. If two projects need similar templates, the operator copies the configuration. This avoids cross-project security questions about whose policies apply.

## Variable Substitution

Trigger evaluators substitute placeholders in the run template's prompt fields with values from the signal payload before creating the run:

```text
System prompt template:
"You are responding to a {{action}} on issue #{{issue.number}} in {{repository.full_name}}.
The issue title is: {{issue.title}}
Labels: {{issue.labels[].name}}"

Signal payload (GitHub issue.labeled):
{
  "action": "labeled",
  "issue": { "number": 42, "title": "Fix login bug", "labels": [{"name": "bug"}, {"name": "cairn-ready"}] },
  "repository": { "full_name": "org/dogfood" }
}

Expanded:
"You are responding to a labeled on issue #42 in org/dogfood.
The issue title is: Fix login bug
Labels: bug, cairn-ready"
```

Substitution rules:

- `{{path.to.field}}` reads a JSON path from the payload
- Arrays are joined with `, ` when accessed via `[]` (e.g. `{{issue.labels[].name}}`)
- Missing paths render as empty strings; the trigger optionally fails if any required path is missing (declared in the template via `required_fields: ["issue.number"]`)
- No conditionals, no loops, no functions in v1 — keep substitution dumb so operators can read templates without learning a DSL

## Integration With Other RFCs

### With RFC 015 (Plugin Marketplace)

The signal router from RFC 015 is the producer; the trigger evaluator is one of its subscribers. When a signal arrives at the router, it fans out to:

1. Existing project subscribers (logged for observability)
2. The trigger evaluator (this RFC)

The evaluator looks up triggers by `(project, signal_type, plugin_id)` in a fast index, evaluates conditions, and proceeds to fire eligible triggers. The router does not need to know about triggers — they are just another subscriber.

### With RFC 005 (Task / Session / Checkpoint Lifecycle)

A trigger fire creates a `Run` record per RFC 005. The run starts in `pending` and is picked up by the orchestrator on its normal cycle. Nothing about RFC 005's state machine changes; triggers are just one of several sources of run creation (alongside operator-clicks-Run and the existing API).

**Trigger-origin fields on the run record**: when a run is created by a trigger, the `RunStarted` event (RFC 005) carries two additional fields:

- `created_by_trigger_id: Option<TriggerId>` — the trigger that caused this run. `None` for operator-initiated or API-initiated runs.
- `trigger_chain_depth: u8` — the depth in the trigger chain (1 for external-signal-triggered, incremented for each loop-back). `0` for non-trigger runs.

These fields are declared HERE (in RFC 022) because they depend on the Trigger entity, but they are physically carried on RFC 005's `RunStarted` event and the `RunRecord` projection. Implementation note: RFC 005's `RunStarted` event shape gains these two optional fields as part of RFC 022's implementation — they are backward-compatible additions (both `Option` / default-zero).

### With RFC 018 (Agent Loop Enhancements)

The `RunTemplate.default_mode` selects the run's `RunMode` (Direct, Plan, Execute). A trigger configured for "high-risk operations" can create runs in Plan mode by default, forcing every triggered run to produce a plan for human approval before any external action. A trigger for low-risk routine work (auto-comment on stale issues) can use Direct mode.

The exploration budget from RFC 018 Plan mode is configurable per template via `TemplateBudget.exploration_budget_share`.

### With RFC 019 (Unified Decision Layer)

Trigger fires go through the decision layer as a `DecisionRequest` of kind `TriggerFire`. This means:

- A guardrail rule that says "no auto-runs from external signals after midnight" applies to triggers
- The decision cache (sealed RFC 019) deduplicates equivalent trigger-fire decisions per its `TriggerFire` decision kind's sealed key derivation — this RFC does not define its own cache key
- Approval flows for triggers work the same way as approvals for any other action — Guardian or Human resolver can approve or deny
- Operators see trigger fires in the same Decisions audit view as everything else

A common pattern: a project has a guardrail that says "trigger fires require approval for the first 5 instances, then auto-allow." This is implemented entirely with existing RFC 019 machinery — no trigger-specific approval logic.

### With RFC 020 (Durable Recovery)

Triggers and run templates are first-class entities in the event log; their state survives crashes the same way runs and approvals do. The trigger evaluator is a runtime worker that resumes on cairn-app restart and re-subscribes to the signal router.

A signal that arrived between crash and recovery is delivered after recovery via the signal router's existing replay mechanism (RFC 015). No signals are lost because the router's storage is durable; the trigger evaluator catches up on missed deliveries.

## Operator Surface

RFC 010's Project Settings gains a Triggers tab:

### Triggers list

- table of all triggers in the project
- columns: name, signal type, state, last fired, fires per hour, plugin source
- click to drill into a trigger detail page

### Trigger detail

- the trigger's full configuration (signal pattern, conditions, template ref)
- recent fires: signal received, decision outcome (allowed/denied/pending), resulting run ID
- rate limit status (current bucket level)
- actions: enable, disable, edit conditions, edit template ref, delete

### Run templates

- separate sub-page for managing run templates
- list of templates, columns: name, default mode, default budget, used by N triggers
- create/edit form: name, description, mode picker, system prompt textarea, plugin allowlist, tool allowlist, budget fields, approval policy picker, sandbox hint

### Trigger creation flow

1. Click "New Trigger"
2. Select signal type from a dropdown of signal types declared by enabled plugins
3. Form for trigger conditions (key-value matchers, "label contains" checkbox, etc.)
4. Select an existing run template, or click "create new template"
5. Set rate limit (default 10/min)
6. Enable on save

### Trigger inspection from a run detail view

When viewing a run that was created by a trigger, the run detail page shows:

- "Created by trigger: {trigger_name} (id: ...)"
- The matching signal payload (with the matched fields highlighted)
- A link back to the trigger configuration

Operators can trace any auto-created run back to the trigger that started it.

### Operator HTTP Contract

Project-scoped CRUD routes for triggers and run templates:

**Triggers:**
```
GET    /v1/projects/:project/triggers?state=&signal_type=&cursor=
POST   /v1/projects/:project/triggers
GET    /v1/projects/:project/triggers/:trigger_id        (includes recent fire history)
PATCH  /v1/projects/:project/triggers/:trigger_id
DELETE /v1/projects/:project/triggers/:trigger_id
POST   /v1/projects/:project/triggers/:trigger_id/enable
POST   /v1/projects/:project/triggers/:trigger_id/disable
POST   /v1/projects/:project/triggers/:trigger_id/resume  (clear Suspended state)
```

**Run Templates:**
```
GET    /v1/projects/:project/run-templates?cursor=
POST   /v1/projects/:project/run-templates
GET    /v1/projects/:project/run-templates/:template_id  (includes "used by N triggers" count)
PUT    /v1/projects/:project/run-templates/:template_id
DELETE /v1/projects/:project/run-templates/:template_id  → 409 if any trigger references it
```

The trigger drill-in `GET` includes the last N `TriggerFired` / `TriggerSkipped` / `TriggerDenied` events so operators can debug "why didn't my trigger fire?" without joining across event-log endpoints.

### Durable Fire Ledger

The `TriggerFireLedger` is a durable projection over `TriggerFired` events, keyed by `(trigger_id, signal_id)`. It prevents the same signal from creating duplicate runs through the same trigger on webhook retry, signal replay, or evaluator restart. This is a **post-routing** dedup layer, distinct from the sealed RFC 017 webhook-ingress dedup (`WebhookDeliveryReceived`) which operates pre-routing.

On startup, the fire ledger is rebuilt from the event log as part of the projection replay step (step 2 of sealed RFC 020's startup dependency graph). Fire entries are retained for the trigger's rate-limit window plus a safety margin (default: 2× the rate-limit window, minimum 1 hour).

### Durable Rate-Limit and Budget State

Per-trigger rate-limit bucket state and per-project trigger budget consumption are **durable projection state**, not in-memory-only token buckets. After restart, the bucket is **reconstructed** from `TriggerFired` and `TriggerRateLimited` events in the event log:

- Per-trigger bucket: count `TriggerFired` events for the trigger in the last `60 / max_per_minute` seconds to derive the current token count
- Per-project budget: count `TriggerFired` events across all triggers in the project in the last hour

Both become event-log-backed projections (`TriggerBucketProjection`) that survive restart. If the in-memory token bucket is used for fast-path decisions at runtime, it is **hydrated** from the projection on startup, not reset to full.

Add to the skip reason enum: `AlreadyFired` for fire-ledger dedup hits.

### Startup / Readiness Integration (RFC 020 carry)

The trigger evaluator must NOT begin consuming signals until the following projections are warm (cross-reference to sealed RFC 020's startup dependency graph, step 2):

- `TriggerProjection`: trigger entity state (enabled/disabled/suspended/deleted)
- `RunTemplateProjection`: template entities
- `TriggerFireLedger`: durable (trigger_id, signal_id) dedup set
- `TriggerBucketProjection`: per-trigger rate-limit state + per-project budget state

Readiness (`/health/ready`) should stay 503 until these projections are rebuilt to the event-log high-water mark. Only then does the trigger evaluator subscribe to the signal router and begin processing new and replayed signals.

**Implementation note for sealed RFC 020**: the projection enumeration in RFC 020's step 2 should be extended to include `TriggerProjection`, `RunTemplateProjection`, `TriggerFireLedger`, and `TriggerBucketProjection`. The sealed RFC 020 commit text already contains "Any new RuntimeEvent variant added by a future or sealed RFC should be assessed" — these projections fall under that guidance.

### Integration With RFC 004 (Graph and Eval Matrix Model)

Trigger events are projected into `cairn-graph` via `cairn-graph::event_projector`, completing the Signal → Trigger → Run provenance chain:

- `TriggerCreated` → `GraphNode(Trigger)`: node_id `trigger:<trigger_id>`, project scope attribute
- `TriggerFired` → edges: `Signal` --`matched_by`--> `Trigger`, `Trigger` --`fired`--> `Run` (with `chain_depth` attribute on the edge)
- `TriggerDeleted` → tombstone on the Trigger node

Operators query: "which triggers created this run" (reverse `fired` from `Run`), "which signals matched this trigger in the last 7 days" (reverse `matched_by` from `Trigger`). No new query APIs — pure event projection following the same pattern as sealed RFC 015 Signal Knowledge Capture, sealed RFC 016 sandbox provenance, and sealed RFC 019 decision provenance.

This completes the three-part provenance chain across the Phase 2 RFC set:
- RFC 015: `GraphNode(Signal)` — signal arrival and knowledge capture
- RFC 022 (this RFC): `GraphNode(Trigger)` — signal-to-run binding
- RFC 005 existing: `GraphNode(Run)` — run lifecycle
- RFC 016: `GraphNode(Sandbox)` → `GraphNode(RepoBase)` — execution environment

### Integration With RFC 019 (Decision Layer) — clarification

Trigger fires go through the sealed RFC 019 decision layer as `DecisionRequest { kind: TriggerFire { trigger_id, signal_type } }`. The decision cache key is derived per RFC 019's sealed `cache_on_fields` mechanism — this RFC does NOT define its own cache key tuple. RFC 019's `TriggerFire` decision kind (sealed at RFC 019 line 100) with its declared policy defaults (1h TTL, Project scope, CacheIfApproved, 4h max) is the authoritative contract.

The trigger fire ledger (`TriggerFireLedger`) and the RFC 019 decision cache serve **different purposes**: the fire ledger prevents duplicate runs from the same (trigger, signal) pair; the decision cache prevents re-evaluation of equivalent trigger fire decisions. Both may hit for the same signal — a cached "Approved" decision from the decision layer does not bypass the fire-ledger dedup check.

## Events

```rust
pub enum TriggerEvent {
    TriggerCreated {
        trigger_id: TriggerId,
        project: ProjectKey,
        signal_pattern: SignalPattern,
        run_template_id: RunTemplateId,
        created_by: OperatorId,
        created_at: u64,
    },
    TriggerUpdated {
        trigger_id: TriggerId,
        updated_by: OperatorId,
        changes: Vec<TriggerFieldChange>,
        updated_at: u64,
    },
    TriggerEnabled  { trigger_id: TriggerId, by: OperatorId, at: u64 },
    TriggerDisabled { trigger_id: TriggerId, by: OperatorId, reason: Option<String>, at: u64 },
    TriggerSuspended {
        trigger_id: TriggerId,
        reason: SuspensionReason,
        at: u64,
    },
    TriggerResumed { trigger_id: TriggerId, at: u64 },
    TriggerDeleted { trigger_id: TriggerId, by: OperatorId, at: u64 },

    TriggerFired {
        trigger_id: TriggerId,
        signal_id: SignalId,
        signal_type: String,
        run_id: RunId,             // the resulting run
        chain_depth: u8,
        fired_at: u64,
    },
    TriggerSkipped {
        trigger_id: TriggerId,
        signal_id: SignalId,
        reason: SkipReason,        // ConditionMismatch | ChainTooDeep | AlreadyFired | MissingRequiredField
        skipped_at: u64,
    },
    TriggerDenied {
        trigger_id: TriggerId,
        signal_id: SignalId,
        decision_id: DecisionId,
        reason: String,
        denied_at: u64,
    },
    TriggerRateLimited {
        trigger_id: TriggerId,
        signal_id: SignalId,
        bucket_state: RateLimitBucket,
        rate_limited_at: u64,
    },
    TriggerPendingApproval {
        trigger_id: TriggerId,
        signal_id: SignalId,
        approval_id: ApprovalId,
        pending_at: u64,
    },

    RunTemplateCreated {
        template_id: RunTemplateId,
        project: ProjectKey,
        name: String,
        default_mode: RunMode,
        created_by: OperatorId,
        created_at: u64,
    },
    RunTemplateUpdated {
        template_id: RunTemplateId,
        updated_by: OperatorId,
        changes: Vec<TemplateFieldChange>,
        updated_at: u64,
    },
    RunTemplateDeleted {
        template_id: RunTemplateId,
        by: OperatorId,
        at: u64,
    },
}
```

Every state transition is in the event log; no separate audit path. The Decisions view (RFC 019) shows trigger-related decisions; the Triggers view (this RFC) shows the trigger-side history.

## Non-Goals

For v1, explicitly out of scope:

- a general-purpose workflow engine (cairn is not Temporal; triggers are simple signal-to-run bindings)
- triggers that fire on internal cairn events (run completed, task failed, etc.) — these are workflow patterns and live in a future RFC
- scheduled triggers (cron) — separate concept, separate RFC
- temporal correlation between signals
- triggers that mutate other triggers
- cross-project trigger fan-out
- a regex or general expression language for conditions
- trigger composition (one trigger watching multiple signal types)
- A/B testing of trigger configurations
- triggers that fire LLM-evaluation-based decisions before creating runs (a small LLM call at trigger time to decide whether to fire). This is interesting but premature; v1 is purely declarative

## Open Questions

1. **NEEDS DISCUSSION: JSON path syntax for conditions and substitution.** Two common options: dot notation (`issue.user.login`) or JSONPath (`$.issue.user.login`). Proposal: dot notation with `[]` for arrays, no leading `$`. Simpler, easier for operators to type, sufficient for the v1 use cases.

2. **NEEDS DISCUSSION: Should triggers fire on signal arrival OR on signal acknowledgment by the project subscriber list?** If a project has triggers AND subscribers (manual operator-click handlers via the API), do they both see the signal? Proposal: yes, both. The trigger evaluator is just one subscriber among potentially many; the router fans out to all of them in parallel.

3. **NEEDS DISCUSSION: What happens when a `RunTemplate` is deleted while triggers reference it?** Proposal: deletion is blocked if any trigger references the template. The operator must update the triggers first (point them at a new template) or delete them. Forces explicit decision, prevents dangling references.

4. **NEEDS DISCUSSION: Trigger chain depth in practice.** The default of 3 may be too low for some workflows (e.g. a multi-revision PR loop with 5+ rounds of review). Should the default be higher, or should operators configure per trigger? Proposal: default 5, configurable per trigger up to 10 (hard cap).

5. **NEEDS DISCUSSION: Variable substitution failure mode.** If a trigger template references `{{issue.number}}` but the signal payload doesn't have that field, what happens? Proposal: by default, missing fields render as empty string and the trigger still fires. If the template declares `required_fields: ["issue.number"]` and the field is missing, the trigger emits `TriggerSkipped { reason: MissingRequiredField }` and does not fire. Confirm.

6. **NEEDS DISCUSSION: Per-project trigger budget default.** Proposed 100 runs/hour from triggers. Is that too low for a busy team or too high for a small team? Proposal: scale with the project's overall budget — default to 10% of the project's `max_runs_per_hour`, or 100 if not configured. Operators can override.

7. **NEEDS DISCUSSION: Cooldown semantics for `Suspended { reason: RateLimitExceeded }`.** When a trigger is auto-suspended for hitting its rate limit, how long does it stay suspended? Proposal: 5 minutes default, exponential backoff if it re-suspends within an hour (5min → 15min → 1h → operator intervention required).

8. **NEEDS DISCUSSION: Should the trigger evaluator be a separate worker process or run inside cairn-app?** Proposal: inside cairn-app, as a tokio task subscribed to the in-process signal router. Splitting it into its own process is over-engineering for v1.

9. **NEEDS DISCUSSION: Templates as a separate top-level entity vs. nested inside Triggers.** Proposed separate (so multiple triggers can share one template). Alternative: inline templates per trigger, simpler model, more duplication. Decide.

10. **Resolved**: Signal source attribution for chain depth uses `source_run_id`, the standard field on the signal normalization envelope per sealed RFC 017 (line 249). Plugins set it when they can detect the signal's origin is a cairn action (e.g. the GitHub plugin checks bot-user authorship). The trigger evaluator resolves `source_run_id` to the source run's chain depth from durable run state. (No further discussion needed; sealed by RFC 017.)

## Decision

Proceed assuming:

- `Trigger` and `RunTemplate` are first-class event-sourced entities, project-scoped, persisted in the runtime event log
- the trigger evaluator is a runtime worker (in-process tokio task) that subscribes to the signal router and fans out to matching triggers
- conditions use a small DSL (Equals / Contains / Exists / Not) on payload fields with dot-notation paths
- variable substitution uses `{{path.to.field}}` syntax with no logic
- trigger fires go through RFC 019's decision layer as a `DecisionRequest { kind: TriggerFire }`, so policy, budget, guardrail, and approval all apply
- chain depth is tracked per run via `source_run_id` (sealed RFC 017 standard envelope field); evaluator resolves prior run depth from durable state; default cap 5, hard cap 10
- runaway protection has two layers: per-trigger token bucket and per-project trigger budget, both backed by durable event-log projections that survive restart (not in-memory-only)
- a durable `TriggerFireLedger` keyed by `(trigger_id, signal_id)` prevents duplicate runs on webhook retry, signal replay, or evaluator restart — separate from RFC 017 ingress dedup and RFC 019 decision cache
- trigger state projections (entity state, fire ledger, rate/budget state) must be warm before the evaluator begins consuming signals; integrated with sealed RFC 020 startup readiness
- trigger provenance projected into cairn-graph: `GraphNode(Trigger)` with `matched_by` and `fired` edges completing the Signal→Trigger→Run chain
- operator HTTP CRUD at `/v1/projects/:project/triggers` and `/v1/projects/:project/run-templates` with drill-in fire history, enable/disable/resume actions, and 409-on-delete-referenced-template
- a deleted template that is referenced by triggers blocks deletion until triggers are updated
- triggers fire events flow through the existing event log; operator surface is project settings → triggers tab
- open questions listed above must be resolved before implementation begins

## Integration Tests (Compliance Proof)

1. **Create + enable + fire**: operator creates a trigger for `github.issue.labeled` with condition `labels[].name contains cairn-ready` in project P1; webhook arrives; trigger fires; a new run is created with the template's mode and prompt; `TriggerFired` event is in the log
2. **Condition mismatch is silent**: a webhook with the wrong action emits no `TriggerFired` event but emits `TriggerSkipped { reason: ConditionMismatch }`
3. **Multiple triggers fan out**: two triggers in the same project both match a signal; both fire; two runs are created
4. **Cross-project isolation**: a webhook signals project P1; project P2 has an identical trigger; only P1's trigger fires
5. **Decision layer policy enforcement**: a trigger configured to require approval before firing emits `TriggerPendingApproval`; the run is not created until the approval is resolved; on approval the trigger fires and creates the run
6. **Decision layer cache hit vs fire-ledger dedup**: (a) a second delivery of the SAME signal (same `signal_id`) within the fire ledger window is skipped as `TriggerSkipped { reason: AlreadyFired }` by the fire ledger — the decision cache is never consulted. (b) A DIFFERENT signal with the same decision-layer shape (same `trigger_id + signal_type` per sealed RFC 019 `TriggerFire` key) but a different `signal_id` hits the decision cache, gets auto-approved without re-prompting the operator, and fires normally (new run created). This tests the two-layer dedup: fire ledger prevents duplicate runs from the same signal; decision cache prevents re-evaluation for equivalent but distinct signals.
7. **Rate limit drops excess**: a trigger with `max_per_minute: 5` receiving 10 signals in one minute fires 5 runs and emits `TriggerRateLimited` for the other 5
8. **Per-project budget suspends triggers**: a project exceeding its trigger budget enters a state where every trigger is `Suspended { reason: BudgetExceeded }` until the cooldown
9. **Chain depth cap prevents loops**: a trigger configured with `max_chain_depth: 3` does not fire when the source signal's `source_run_id` resolves to a run already at depth 3
10. **Variable substitution**: a template with `{{issue.title}}` produces a run whose initial user message contains the actual issue title from the signal payload
11. **Required fields**: a template declaring `required_fields: ["issue.number"]` skips the trigger when the signal payload lacks that field; emits `TriggerSkipped { reason: MissingRequiredField }`
12. **Template referenced by trigger blocks delete**: deleting a template referenced by a live trigger returns 409; operator must clear references first
13. **Recovery preserves trigger state**: cairn-app crashes mid-firing; on restart the trigger evaluator resumes and processes signals delivered during the recovery window
14. **Run carries trigger origin**: a run created by a trigger has `trigger_chain_depth >= 1` and a `created_by_trigger_id` reference; the operator can navigate from the run detail back to the trigger
