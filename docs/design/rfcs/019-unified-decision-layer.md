# RFC 019: Unified Decision Layer

Status: draft (rev 2 — adopts cache_on_fields allowlist for decision keys, singleflight cache with Pending state, selective invalidation via policy-rule reference index)
Owner: runtime/policy lead
Depends on: [RFC 002](./002-runtime-event-model.md), [RFC 005](./005-task-session-checkpoint-lifecycle.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 015](./015-plugin-marketplace-and-scoping.md), [RFC 018](./018-agent-loop-enhancements.md), [RFC 022](./022-triggers.md)

## Resolved Decisions (this revision)

- **Decision key normalization**: each tool declares an explicit `cache_on_fields: Vec<String>` allowlist in its descriptor. The decision layer hashes only those fields when computing the cache key. Tools without `cache_on_fields` default to `cache_policy: NeverCache`. No custom normalization function — just a list of field paths the operator and tool author both understand. The body's `semantic_hash` is derived from `cache_on_fields` only (not from `tool_name + normalized_args + effect` as an earlier draft stated).
- **Cache invalidation on policy changes**: **selective via policy-rule reference index**. Each cached decision's `reasoning_chain` records which guardrail `rule_id`s contributed to the evaluation at step 3. A reverse index from `rule_id → set of cached_decision_ids` enables selective invalidation: when an operator edits a guardrail rule, only cached decisions that referenced that rule are invalidated. Scope-wide wipe is the fallback if the reverse index is too expensive for a specific deployment.
- **Cache concurrency (singleflight)**: the cache lookup result is one of `Miss`, `Pending { owner_decision_id, started_at }`, or `Resolved { decision_id, outcome, expires_at }`. On a `Miss`, the first requester atomically installs a `Pending` entry and proceeds to evaluation/resolver steps. Concurrent requesters seeing `Pending` wait on the first evaluation to resolve. When the first evaluation completes, the entry promotes to `Resolved` and all waiters replay the result. Stale `Pending` entries (evaluator crashed, resolver timed out) are recovered via a configurable `pending_timeout_ms` (default: 60s); after timeout the pending entry is cleared and the next requester re-evaluates.
- **Guardian resolver source**: when a guardian resolves a decision, the cached entry is tagged `DecisionSource::Guardian { model_id }` distinct from `DecisionSource::Human { operator_id }`. The operator audit view shows both kinds with a filter to see only human-authorized rules.
- **Triggers go through the decision layer**: every trigger fire (RFC 022) is a `DecisionRequest { kind: TriggerFire }`. The decision cache deduplicates equivalent trigger fires; the resolver chain handles approval flows; the audit chain captures everything.

## Summary

Cairn already has four separate services that decide whether an action is allowed: `GuardrailService` (policy rules), `ApprovalService` (human sign-off requests), `BudgetService` + `SpendAlertService` (spend limits), and the tool tier + `VisibilityContext` system (whether a tool is even visible). Each of these runs independently, produces its own events, and has its own decision shape. The cost of that fragmentation is:

- **Re-nudging**: an operator can be asked to approve "the same thing" twice in the same day because the guardrail layer and the approval layer do not share a memory of prior decisions
- **Split audit trail**: understanding why an action was allowed requires joining events from four services
- **No learned rules**: a decision made once cannot be reused for an equivalent future decision
- **Opaque ordering**: if budget says "no", guardrail says "yes, with approval", and the tier system says "allowed", which wins? Today the answer depends on the call site

This RFC does not replace any of those services. It introduces a **Unified Decision Layer** — a thin orchestration service that composes the existing services into a single, atomic evaluation with **one truth per decision**: the layer takes a request, consults the existing services in a canonical order, checks the decision cache for an equivalent prior decision, invokes the RFC 018 resolver chain when human or guardian input is needed, caches the result, and emits a single `DecisionRecorded` event.

The result: the same action evaluated twice produces the same outcome without re-nudging the operator. A decision an operator makes once propagates to every semantically equivalent future request within a configurable TTL and scope. Every decision has a single authoritative record in the event log.

## Why

### The re-nudging problem is real and load-bearing

The user explicitly flagged this: "if was approved in one system, don't re-nudge the user. the system should be atomic and have one truth for everything."

Concretely, today:

- An operator approves `github.create_pull_request` for a specific run. Fine.
- Two hours later, a different run in the same project wants to do the same thing with a similar enough request. The approval service does not know about the previous decision; it asks again. The operator is either trained to click approve without reading (defeating the gate) or the run blocks waiting.
- Meanwhile the guardrail service checked the action against its rule set, the budget service checked spend, and the tier system checked visibility — each with its own evaluation, its own event, no coordination.

A single trusted record of "this decision was made, here's the key, here's the outcome, here's the TTL, here's who authorized it" is the fix.

### Why a thin layer, not a rewrite

Each existing service works. They were built in RFC 005, RFC 007, RFC 011, and earlier runtime work, and they are wired into cairn-app's routes, event log, and operator surfaces. Rewriting any of them to centralize decision-making would break a lot of downstream code without clear benefit. The unified layer is an orchestration primitive on top: it calls into each service, combines their answers, caches the result, and is the only place that emits `DecisionRecorded` events.

### What "atomic" means

One request flowing through the decision layer produces exactly one outcome. Either all the checks say yes and the action proceeds (`Allowed`), or the evaluation produces a `Denied` outcome at a specific step with a specific reason. In both cases the result is a single `DecisionRecorded` event with the full reasoning chain. There is no partial state — the action does not half-happen because budget said yes but guardrail said no. The layer is the gate.

## Scope

### In scope for v1

- A new service `DecisionService` in `cairn-runtime/src/services/` that composes the existing services
- A canonical `DecisionRequest` type that wraps the thing being evaluated (tool call, mutation, spend-incurring action, approval need)
- A **decision cache** keyed by a deterministic `DecisionKey` derived from the request's semantic shape (not its syntactic form); backed by the existing event log with a periodic consolidation pass
- Integration with the RFC 018 resolver chain (`HumanResolver`, `GuardianResolver`)
- A **learned rules** mechanism: when a human or guardian approves a decision key, the outcome is cached for a TTL (configurable per decision type) and replayed for equivalent future requests without re-nudging
- A single new event variant `DecisionRecorded` that carries the full reasoning chain for audit
- Operator surface: a "Decisions" tab (extension of RFC 010's "Policies" view) showing recent decisions, their keys, their sources, and a way to invalidate a cached decision
- Scope-aware caching: decisions can be cached at `tenant`, `workspace`, `project`, or `run` scope, chosen by the decision type's declared defaults and optionally overridable by the policy layer

### Explicitly out of scope for v1

- Rewriting `GuardrailService`, `ApprovalService`, `BudgetService`, or any of the existing policy machinery
- A general-purpose policy DSL (the existing guardrail rules are sufficient for v1)
- Policy authoring UI (the existing policy configuration surfaces remain; the decision layer reads from them)
- Machine-learned adaptive thresholds (budget caps are manual in v1)
- Cross-tenant decision sharing (every decision is scoped to at most one tenant)
- Cryptographic signing of cached decisions (future work when multi-operator trust boundaries matter)

## Canonical Flow

### The request

Anything that needs to be evaluated as a "can I do this?" question becomes a `DecisionRequest`. The main call sites are:

- the orchestrator's execute phase, when invoking a tool
- the marketplace layer, when enabling a plugin that triggers policy
- the provider router, before making a provider call that will incur cost
- any explicit policy-gated action (uninstall plugin, delete credential, cancel a run)

```rust
pub struct DecisionRequest {
    pub kind: DecisionKind,                    // TagKind for dispatch + key derivation
    pub principal: Principal,                  // who is asking: run, operator, system
    pub subject: DecisionSubject,              // what is the target: tool call, resource, etc.
    pub scope: ProjectKey,                     // tenant/workspace/project context
    pub cost_estimate: Option<CostEstimate>,   // for budget pre-check
    pub requested_at: u64,
    pub correlation_id: CorrelationId,         // for tracing across events
}

pub enum DecisionKind {
    ToolInvocation { tool_name: String, effect: ToolEffect },
    ProviderCall { model_id: String, estimated_tokens: u32 },
    PluginEnablement { plugin_id: String, target_project: ProjectKey },
    WorkspaceProvision { strategy: SandboxStrategy, base: SandboxBase },
    CredentialAccess { credential_id: CredentialId, purpose: String },
    TriggerFire { trigger_id: TriggerId, signal_type: String },
    DestructiveAction { action: String, resource: ResourceRef },
    Other(String),
}
```

### The evaluation order

The decision layer evaluates in a fixed order. Each step can allow, deny, or escalate (require resolver input). The first deny wins. An allow from one service does not override a deny from another.

```
1. Scope check
   - Does the principal have access to the scope? (existing RFC 008 scoping)
   - If no: Deny { reason: scope_violation }

2. Tier + visibility check (RFC 015)
   - Is the tool/capability even visible in the current VisibilityContext?
   - If no: Deny { reason: not_in_context }   (this should be rare — the LLM should not see the tool)

3. Guardrail check (existing GuardrailService)
   - Does any guardrail rule match? What does it return?
   - If Deny: Deny { reason: guardrail_denied, matching_rule: ... }
   - If Escalate: record that the guardrail layer requested escalation; continue
   - If Allow: continue

4. Budget check (existing BudgetService)
   - Does the estimated cost fit within tenant/workspace/project/run budgets?
   - If over a hard cap: Deny { reason: budget_exceeded, scope: ... }
   - If over a soft alert threshold: continue but record an alert event
   - If fine: continue

5. Cache lookup (singleflight)
   - Derive DecisionKey from the request
   - Atomically check the cache entry state for this key + scope:
     - **Resolved** (unexpired): apply the cached outcome. Emit DecisionRecorded { source: CacheHit, ... }. Return.
     - **Pending** (another request is being evaluated for this key): wait on the pending resolution up to `pending_timeout_ms`. If the pending evaluation completes within the timeout, replay its outcome as a cache hit. If the pending evaluation times out or the evaluator crashed (stale Pending entry), clear the Pending state and fall through to step 6 as a fresh evaluation.
     - **Miss** (no entry): atomically install a `Pending { owner_decision_id, started_at }` entry for this key, then continue to step 6. This prevents a second concurrent request from independently escalating while the first is in-flight.

6. Approval resolution (if step 3 or explicit policy required escalation)
   - Invoke the RFC 018 ApprovalResolver chain
   - GuardianResolver may fire first; if it returns None (fall-through), HumanResolver handles it
   - HumanResolver blocks the run until an operator responds
   - On return: emit DecisionRecorded with the resolver's decision

7. Cache write (if step 5 was a Miss and step 6 produced a decision)
   - Promote the Pending entry to Resolved with the decision outcome and the TTL declared by the DecisionKind
   - Emit DecisionCacheUpdated event
   - All waiters from step 5 (Pending path) are notified and replay the Resolved outcome

8. Return the outcome
```

No step is skipped. Every allow is the result of steps 1 through 7. Every deny comes from a specific step with an attributable reason.

### The DecisionKey

The cache key is where the "atomic, one truth" constraint lives. Two decisions are equivalent (same key) if they differ only in details that should not change the outcome:

```rust
// Deterministic, stable across time and processes.
pub struct DecisionKey {
    pub kind_tag: &'static str,        // e.g. "tool_invocation"
    pub scope_ref: DecisionScopeRef,   // matches the declared DecisionCacheScope
    pub semantic_hash: String,         // stable hash of the semantic fingerprint
}

/// Concrete discriminated scope key that matches the DecisionCacheScope enum.
/// DecisionRequest.scope remains ProjectKey (execution context); the cache
/// entry's scope_ref is derived from the DecisionKind's default_scope and
/// the request's execution context.
pub enum DecisionScopeRef {
    Run { run_id: RunId, project: ProjectKey },
    Project(ProjectKey),
    Workspace { tenant_id: TenantId, workspace_id: WorkspaceId },
    Tenant { tenant_id: TenantId },
}
```

The `semantic_hash` is derived exclusively from the tool's declared `cache_on_fields` allowlist. No custom normalization function — the hash is `hash(sorted(field_path → value for each field_path in cache_on_fields))`. Fields not in the allowlist are excluded from the key.

Per-`DecisionKind` examples:

- **`ToolInvocation`** — the tool's `cache_on_fields` determines which argument fields contribute to the key. Example: `github.comment_on_issue` declares `cache_on_fields: ["repo", "effect"]` — a single approval for "commenting on issues in repo X" covers all subsequent comments regardless of issue number or comment text, because those fields are not in the allowlist. Tools without `cache_on_fields` default to `NeverCache`.

- **`ProviderCall`** — `hash(model_id + coarse_token_bucket)`, where the token count is rounded to a bucket (100, 1K, 10K, 100K). Approving one 4K-token call to gpt-5 auto-approves equivalent small calls; a 100K-token call is a different key.

- **`PluginEnablement`** — `hash(plugin_id + target_project_key)`. Enablement decisions are project-specific and do not share across projects.

- **`WorkspaceProvision`** — `hash(strategy + base_category)`, where `base_category` is Repo/Directory/Empty. **Deliberately not per-repo** — approving a repo-based sandbox for the project covers every repo the project can reach. Per-repo access control is handled by the sealed RFC 016's project-scoped `RepoStore` allowlist, which has its own decision-layer gate via `cairn.registerRepo` (`DecisionKind::ToolInvocation`). The two layers are complementary: `WorkspaceProvision` gates the **capability** (can the project use repo-based sandboxes?); `RepoStore` allowlist gates the **scope** (which repos?). An operator who approves `WorkspaceProvision` is approving the capability for the project, not for a specific repo.

- **`TriggerFire`** — `hash(trigger_id + signal_type)`. Deduplicates equivalent trigger fires within the TTL. The cache key is distinct from the webhook-level dedup in RFC 017 (which deduplicates at signal ingress); the decision cache deduplicates at the trigger-evaluation layer after routing.

- **`CredentialAccess`** — `hash(credential_id + purpose)`. Same credential, same purpose → same key.

Tools and decision kinds that should never be cached declare `cache_policy = NeverCache`. Example: `github.merge_pull_request` from RFC 017 is always a fresh decision, no caching allowed, even if an operator just approved one merge for the same repo a minute ago.

### TTL and scope of cached decisions

Each `DecisionKind` declares defaults:

```rust
pub struct DecisionPolicy {
    pub default_ttl: Duration,
    pub default_scope: DecisionCacheScope,
    pub cache_policy: CachePolicy,  // AlwaysCache | NeverCache | CacheIfApproved | CacheIfDenied
    pub max_ttl: Duration,          // hard cap the operator can't override
}

pub enum DecisionCacheScope {
    /// Cached decision applies only to the run that made the request
    Run,
    /// Cached decision applies to every run in the project
    Project,
    /// Cached decision applies to every run in the workspace
    Workspace,
    /// Cached decision applies to every run in the tenant
    Tenant,
}
```

Example defaults:

| DecisionKind | default_ttl | default_scope | cache_policy | max_ttl |
|---|---|---|---|---|
| ToolInvocation (Observational) | 24 h | Project | AlwaysCache | 7 days |
| ToolInvocation (Internal) | 24 h | Project | AlwaysCache | 7 days |
| ToolInvocation (External, low-risk) | 4 h | Project | CacheIfApproved | 24 h |
| ToolInvocation (External, high-risk e.g. merge) | 0 | — | NeverCache | 0 |
| ProviderCall | 1 h | Run | CacheIfApproved | 24 h |
| PluginEnablement | 0 | — | NeverCache | 0 |
| WorkspaceProvision | 24 h | Project | AlwaysCache | 7 days |
| TriggerFire | 1 h | Project | CacheIfApproved | 4 h |
| CredentialAccess | 1 h | Run | CacheIfApproved | 1 h |

The operator can configure overrides via the policy surface (narrowing, not widening — an operator can reduce a default but never exceed `max_ttl`).

### Learned rules

When an operator (or a guardian) makes a decision that is cached, the cached entry is a **learned rule**: a durable "yes, this is OK" or "no, this is not OK" that cairn remembers and applies automatically.

The operator can see the full list of learned rules for their project in the Decisions operator view, and can invalidate individual rules or whole categories. Invalidation emits a `DecisionCacheInvalidated` event. Every future equivalent request goes through the full evaluation again until the operator approves a new rule.

**Why this is not "blind auto-approval"**: every learned rule was created by an explicit human or guardian approval of a specific request. The rule caches that decision for equivalent future requests. It does not auto-approve things nobody ever approved.

### Example walk-through: the same thing approved twice

Today (without the decision layer):

1. Agent in project P1 tries to call `github.comment_on_issue` on issue #42 of repo `org/dogfood`
2. `GuardrailService` says "requires approval" based on a guardrail rule
3. `ApprovalService` creates an approval request, run blocks
4. Operator clicks Approve
5. Tool executes
6. Ten minutes later, a different agent in project P1 tries to call `github.comment_on_issue` on issue #43 of the same repo
7. `GuardrailService` says "requires approval" again
8. `ApprovalService` creates another approval request, this run also blocks
9. Operator has to click Approve again

With the decision layer:

1. Agent calls `github.comment_on_issue` on issue #42 of `org/dogfood`
2. `DecisionService::evaluate` runs the evaluation order
3. Step 3: Guardrail says "escalate" (requires approval)
4. Step 5: Cache miss
5. Step 6: `HumanResolver` creates an approval request, run blocks, operator approves
6. Step 7: Decision cached at `Project` scope with 4h TTL: `DecisionKey { kind: ToolInvocation, semantic_hash: hash("github.comment_on_issue", repo="org/dogfood", effect=External) }`
7. Ten minutes later, agent calls `github.comment_on_issue` on issue #43 of the same repo
8. `DecisionService::evaluate` runs again
9. Step 5: Cache hit — same `DecisionKey`
10. Step 8: Return Approved, emit `DecisionRecorded { source: cache_hit, original_decision_id: ... }`
11. Tool executes without re-nudging the operator

The audit trail shows two decisions, one by human, one by cache hit, both pointing at the same learned rule. The operator can see the chain.

## Integration With Existing Services

### Guardrail Service

`DecisionService` calls `GuardrailService::evaluate(request)` and reads its existing `GuardrailDecision` shape. No changes to `GuardrailService`. If a guardrail rule returns `Allow`, the decision layer records it. If `Deny`, the decision is denied immediately. If `Escalate`, the decision layer moves to the cache and then resolver steps.

### Approval Service

`DecisionService` does not replace `ApprovalService`; it uses it. `HumanResolver` (from RFC 018) delegates to `ApprovalService::create_approval_request` and waits for `ApprovalService::get_decision`. The decision layer observes the outcome, records the `DecisionRecorded` event, and caches the result.

### Budget Service

`DecisionService` calls `BudgetService::check_cost(scope, estimated)` before any resolver step. If the budget is exhausted at a hard-cap level, the decision is denied immediately with `reason: budget_exceeded`. Soft alerts (70%, 90%) continue the evaluation but record alert events.

### Spend Alert Service

Unchanged. The decision layer emits spend-adjacent events that feed the existing alert pipeline.

### Visibility Context (RFC 015)

`DecisionService` is a backstop against `VisibilityContext` failures. In normal operation, the orchestrator's prompt builder already hides tools that are not visible in the context, so the LLM should never try to call them. But if the LLM hallucinates a tool name it should not have access to, the decision layer catches it at step 2 and denies before the tool is invoked.

### RFC 018 Resolver Chain

The decision layer is the only call site for the resolver chain. `HumanResolver` and `GuardianResolver` are invoked from step 6. They return `ResolverDecision` which is recorded and (if cacheable) stored in the cache.

### cairn-graph

`DecisionRecorded` events flow through the event log and are available to `cairn-graph::event_projector` for future decision-provenance projection (e.g. `GraphNode(Decision)` with edges to the `Run`, `Tool`, and `GuardrailRule` nodes involved). No RFC 019-specific change is needed — the `DecisionRecorded` event shape carries all required fields (`reasoning_chain`, `resolved_by`, `scope`). The projection is future work and does not block v1.

## Events

New event variants in the existing runtime event log:

```rust
pub enum DecisionEvent {
    DecisionRecorded {
        decision_id: DecisionId,
        request: DecisionRequest,
        outcome: DecisionOutcome,     // Allowed | Denied
        reasoning_chain: Vec<StepResult>,  // full chain from steps 1-7
        source: DecisionSource,       // CacheHit | FreshEvaluation
        resolved_by: Option<ResolverId>, // human id or guardian model id
        cached_for: Option<CachedDecisionRef>,  // the cached rule, if step 7 wrote one
        decided_at: u64,
    },

    DecisionCacheUpdated {
        decision_id: DecisionId,
        decision_key: DecisionKey,
        scope: DecisionCacheScope,
        ttl: Duration,
        expires_at: u64,
        created_at: u64,
    },

    DecisionCacheHit {
        new_request_id: CorrelationId,
        cached_decision_id: DecisionId,
        decision_key: DecisionKey,
        hit_at: u64,
    },

    DecisionCacheInvalidated {
        decision_id: DecisionId,
        decision_key: DecisionKey,
        invalidated_by: ActorRef,      // OperatorId | SystemPolicyChange
        reason: String,
        invalidated_at: u64,
    },
}
```

**`DecisionRecorded` is the single canonical event for ALL decisions** — both `Allowed` and `Denied`. The `outcome: DecisionOutcome` field carries `Allowed` or `Denied { deny_step, deny_reason }`. Earlier drafts split denials into a separate `DecisionDenied` event, but that created two canonical records for the same concept and contradicted the "single truth per decision" design goal. The reasoning chain captures the output of every step (scope, visibility, guardrail, budget, cache, resolver), so a reviewer can replay the decision logic from a single event without joining across services.

There is no `DecisionRequiresApproval` event — approval requests flow through the existing `ApprovalService` (`ApprovalRequested` / `ApprovalResolved` from RFC 005/018). The decision layer observes the approval outcome, records it in `DecisionRecorded`, and caches the result.

## Operator Surface

RFC 010 lists "Policies" as a top-level operator view. The decision layer extends it with a **Decisions** tab that shows:

### Recent decisions list

- decision ID, kind, scope, principal, outcome, reasoning summary
- filterable by kind, scope, outcome, source (cache hit vs fresh)
- click to drill into the full reasoning chain

### Learned rules view

- list of cached decisions currently in effect
- columns: kind, scope, TTL remaining, original authorizer, hit count
- actions: invalidate one rule, invalidate by category, invalidate all in a scope

### Decision audit trail

- for any resource (tool, plugin, run), "show me every decision that affected this resource"
- uses the reasoning chain to join back to guardrail rules, budget checks, resolver outcomes

### Policy source attribution

- for a decision that required approval, show which guardrail rule triggered the escalation, which budget rule was consulted, which resolver handled it
- for a cached hit, show the original decision and when it will expire

This view is the one place an operator can see "who, what, when, why" for any allow/deny in the system. It replaces the scattered "approval audit log", "guardrail decision log", and "budget event log" that exist today with a single pane.

### Operator HTTP Contract

```
GET    /v1/decisions
       List recent decisions; filterable by kind, scope, outcome, source.
       Paginated. Returns DecisionRecorded event summaries.

GET    /v1/decisions/cache
       List currently active cached decisions (learned rules).
       Columns: decision_key, kind, scope, ttl_remaining, source, hit_count.

GET    /v1/decisions/:decision_id
       Drill-in: full DecisionRecorded event with reasoning chain.

POST   /v1/decisions/:decision_id/invalidate
       Body: { "reason": "string" }
       Invalidates a specific cached decision. Emits DecisionCacheInvalidated.
       The next equivalent request goes through full fresh evaluation.

POST   /v1/decisions/invalidate
       Body: { "scope": DecisionScopeRef, "kind": "ToolInvocation" | "all" }
       where DecisionScopeRef is one of:
         { "type": "run", "run_id": "...", "project": {...} }
         { "type": "project", "project": {...} }
         { "type": "workspace", "tenant_id": "...", "workspace_id": "..." }
         { "type": "tenant", "tenant_id": "..." }
       Bulk invalidation: all cached decisions in the scope matching the kind
       (or all kinds if "all"). Emits one DecisionCacheInvalidated per entry.

POST   /v1/decisions/invalidate-by-rule
       Body: { "rule_id": "guardrail_rule_id" }
       Selective invalidation via the policy-rule reference index: invalidates
       only cached decisions whose reasoning_chain step 3 referenced the given
       rule_id. This is the concrete mechanism behind the sealed resolved
       decision on "selective invalidation via policy-rule reference index."
```

## Non-Goals

For v1, explicitly out of scope:

- replacing `GuardrailService`, `ApprovalService`, `BudgetService`, or `SpendAlertService`
- a general-purpose policy DSL beyond the existing guardrail rules
- ML-based adaptive decision policies
- cross-tenant decision sharing
- cryptographic signing of decisions
- "revoke and undo" for decisions that turned out to be wrong (the event log preserves history; undoing the side effects is a separate operator action)
- a recommendation engine for operators to approve learned rules in bulk

## Open Questions

1. **Resolved**: Decision key normalization uses `cache_on_fields` allowlists declared per tool/kind. No custom normalization function — just `hash(sorted(field_path → value))` over the declared fields. A tool that does not declare `cache_on_fields` defaults to `NeverCache`. A test harness enforces key equivalence: "for every two requests that should match, the hash is identical; for every two that should not, they differ." Tool authors write tests. (No further discussion needed; baked into the Resolved Decisions.)

2. **NEEDS DISCUSSION: Decision cache storage.** The cache is logically an in-memory read model over `DecisionEvent`s in the event log. Should it be a dedicated table in the store, or a projection over events? Proposal: projection over events (consistent with the rest of cairn's read model architecture).

3. **Resolved**: Cache invalidation on policy changes uses the **policy-rule reference index**: each cached decision's `reasoning_chain` step 3 records which guardrail `rule_id`(s) contributed to the evaluation. A reverse index from `rule_id → cached_decision_ids` is maintained. When an operator edits a rule, only cached decisions referencing that rule are invalidated via `POST /v1/decisions/invalidate-by-rule`. Scope-wide wipe via `POST /v1/decisions/invalidate` is the fallback. (No further discussion needed; baked into the Resolved Decisions.)

4. **NEEDS DISCUSSION: Soft cap budget behavior.** When spend crosses a soft alert threshold, the decision continues but an alert is emitted. Should the cache TTL be shortened for decisions made above a soft cap? Proposal: yes, cap TTL at 1 hour when spend > 80% of budget to force re-evaluation as budget becomes tight.

5. **NEEDS DISCUSSION: Guardian decisions in the cache.** If a guardian approves a decision, is the cached outcome identified as guardian-authorized? Should operators see it distinctly in the learned rules view? Proposal: yes, `DecisionSource::Guardian { model_id }` is distinct from `DecisionSource::Human { operator_id }` and the view shows both. Operators can also filter to "only human-authorized rules" if they want to audit guardian behavior separately.

6. **NEEDS DISCUSSION: What happens when a cached decision is hit but the original context has changed.** E.g. a cached `ToolInvocation` for `github.comment_on_issue` in repo X, but repo X has been disabled in the project since. Proposal: the cache hit is shadowed by step 2 (visibility check) — visibility is evaluated fresh each time and catches the "plugin no longer enabled" case before the cache is consulted. This is already how the evaluation order works; confirm no code change is needed.

7. **NEEDS DISCUSSION: Cross-run decision dependencies.** If run A's decision unlocked a resource and run B's decision relies on it, a cache invalidation on the first decision may leave run B with stale state. Proposal: v1 does not model inter-decision dependencies; invalidations are local to the decision key. Operators must be aware.

8. **NEEDS DISCUSSION: How is `DecisionId` derived?** Proposal: `dec_<ulid>` with a random component, stored in the event log. Not derivable from the request — a single request can produce multiple decisions over time as policies change.

## Decision

Proceed assuming:

- a new `DecisionService` in `cairn-runtime/src/services/` composes the existing guardrail/approval/budget/visibility services into a single evaluation
- the evaluation order is: scope → visibility → guardrail → budget → cache lookup → resolver chain → cache write → return
- a cache miss that results in a fresh approval writes a `DecisionCacheUpdated` event with a `DecisionKey` derived semantically per kind
- a cache hit on an unexpired decision returns the prior outcome and emits `DecisionRecorded { source: CacheHit }` — no re-nudging
- a `DecisionRecorded` event is the single canonical audit record for both `Allowed` and `Denied` decisions — there is no separate `DecisionDenied` event
- concurrent identical requests are coalesced via singleflight: the first installs a `Pending` cache entry, concurrent requesters wait on it, stale Pending entries are recovered after `pending_timeout_ms`
- `TriggerFire` is a first-class `DecisionKind` variant so trigger fires flow through the same cache/resolver/audit path as tool invocations
- the operator surface adds a Decisions tab inside Policies (per RFC 010) showing recent decisions and learned rules, with per-rule invalidation
- no existing service is replaced; the decision layer is an orchestration primitive above them
- every `DecisionKind` declares a default TTL, scope, cache policy, and max TTL; operators can narrow but never widen past `max_ttl`
- `github.merge_pull_request` and any other explicitly-uncacheable action sets `cache_policy = NeverCache`; learned rules cannot auto-approve them
- open questions listed above must be resolved before implementation begins

## Integration Tests (Compliance Proof)

1. **Fresh approval, then cache hit**: operator approves a `github.comment_on_issue` call in project P1; a subsequent equivalent call in P1 within TTL returns Allowed via cache hit; no new approval request is created
2. **Cache miss by semantic difference**: an approved call for `github.comment_on_issue` on issue #42 does NOT auto-approve a call for `github.merge_pull_request` on PR #43 — different semantic hashes
3. **Scope isolation**: a decision cached at `Project` scope for project P1 does not apply to project P2 — different `DecisionKey.scope_key`
4. **NeverCache decision**: two consecutive `github.merge_pull_request` approvals both require explicit human input, even within TTL — the cache is never written
5. **Guardrail deny wins**: a request where guardrail returns Deny is denied immediately; the cache is not written; no resolver chain is invoked
6. **Budget deny wins**: a request whose estimated cost exceeds hard cap is denied at step 4; no resolver chain is invoked
7. **Visibility deny**: a tool call for a tool not in the `VisibilityContext` is denied at step 2 even if the LLM hallucinates the name
8. **Guardian decision cached distinctly**: a guardian-approved decision creates a cached rule tagged `DecisionSource::Guardian`; the operator view shows it distinct from human-approved rules
9. **Invalidation removes the rule**: an operator invalidates a cached decision via the operator view; the next equivalent request goes through fresh evaluation
10. **Cache expiry**: a cached decision past its TTL is not returned; the next request is a fresh evaluation
11. **Policy change invalidates**: an operator edits a guardrail rule that was part of a cached decision's reasoning chain; the cached decision is automatically invalidated
12. **Reasoning chain in audit**: every `DecisionRecorded` event includes the full reasoning chain (scope, visibility, guardrail, budget, cache, resolver) with per-step outputs; the operator view can render the chain from the event log
