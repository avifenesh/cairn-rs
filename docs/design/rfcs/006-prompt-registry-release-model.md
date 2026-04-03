# RFC 006: Prompt Registry and Release Model

Status: draft  
Owner: evals lead  
Depends on: [RFC 004](./004-graph-eval-matrix.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 010](./010-operator-control-plane-ia.md)

## Summary

The Rust rewrite will treat prompts as first-class product assets with:

- library-owned prompt assets
- immutable prompt versions
- project-scoped prompt releases
- explicit rollout and rollback semantics
- approval-gated release transitions
- direct linkage to eval runs and runtime usage

This RFC exists to stop prompt/version/release semantics from drifting across eval, runtime, graph, API, and UI workers.

## Why

Prompts are part of the product control plane, not just strings embedded in agent code.

Without a concrete prompt registry model:

- runtime workers will reference prompts differently
- eval workers will score incompatible units
- graph workers will not know what to link
- UI workers will invent release semantics ad hoc

The prompt system must support the operator workflows promised in RFC 010:

- compare two prompt releases
- inspect eval impact
- choose rollout, rollback, or hold

## Core Objects

### Prompt Asset

A prompt asset is the stable logical identity for a prompt family.

Examples:

- `planner.system`
- `retrieval.answer`
- `coder.review`

Prompt assets are library objects. They are not mutable release state.

Required fields:

- `prompt_asset_id`
- `scope` (`tenant` or `workspace`)
- `name`
- `kind`
- `owner`
- `status`
- `created_at`
- `updated_at`

`kind` in v1 should support at least:

- `system`
- `user_template`
- `tool_prompt`
- `critic`
- `router`

### Prompt Version

A prompt version is an immutable snapshot of a prompt asset.

Required fields:

- `prompt_version_id`
- `prompt_asset_id`
- `version_number`
- `content`
- `format`
- `metadata`
- `created_by`
- `created_at`
- `content_hash`

Rules:

- versions are immutable
- a changed prompt body always creates a new version
- `content_hash` is used for integrity and dedup checks

`metadata` may include:

- model hints
- intended task types
- expected tools
- safety notes
- deprecation reason

### Prompt Release

A prompt release is the deployable runtime binding of a prompt version into a project.

Required fields:

- `prompt_release_id`
- `project_id`
- `prompt_asset_id`
- `prompt_version_id`
- `release_tag`
- `state`
- `rollout_target`
- `created_at`
- `created_by`

`state` in v1:

- `draft`
- `proposed`
- `approved`
- `active`
- `rejected`
- `archived`

`rollout_target` in v1 is a structured selector, not a loose label.

It must contain:

- `kind`
- `selector`

Supported `kind` values in v1:

- `project_default`
- `agent_type`
- `task_type`
- `routing_slot`

### Selector Justification

`project_default` exists to provide the baseline release when no narrower runtime context applies.

`agent_type` exists because teams often need one prompt behavior for a role such as planner, coder, or critic across many tasks.

`task_type` exists because some prompt choices are driven by workflow intent rather than the broad agent role.

`routing_slot` exists for deliberate runtime indirection such as:

- fallback chains
- explicit slot-based prompt routing
- controlled side-by-side runtime wiring where the slot name is part of the runtime contract

V1 keeps both `task_type` and `routing_slot` because they solve different sources of specificity:

- `task_type` is semantic workflow targeting
- `routing_slot` is explicit runtime wiring

Examples:

```json
{ "kind": "project_default", "selector": {} }
```

```json
{ "kind": "agent_type", "selector": { "agentType": "planner" } }
```

```json
{ "kind": "task_type", "selector": { "taskType": "review" } }
```

```json
{ "kind": "routing_slot", "selector": { "slot": "fallback_1" } }
```

### Prompt Release Action

Release transitions must be auditable.

Every rollout, approval, activation, rollback, or archive operation must emit a durable release action record with:

- `release_action_id`
- `prompt_release_id`
- `action_type`
- `actor`
- `reason`
- `created_at`
- optional `from_release_id`
- optional `to_release_id`

## Canonical Release Lifecycle

There is one canonical lifecycle field: `state`.

V1 must not introduce a second orthogonal `approval_state` field.

Approval is represented by the release entering either:

- `approved`
- `rejected`

This avoids split-brain semantics between lifecycle and approval.

### State Meanings

- `draft`
  - release object exists but is not yet awaiting operator decision
- `proposed`
  - release is ready for operator review and not deployable yet
- `approved`
  - release is approved for activation but not currently active
- `active`
  - release is the live release for its selector
- `rejected`
  - release was reviewed and rejected for activation
- `archived`
  - release is retained only for history

### Allowed Transitions

- `draft -> proposed`
- `draft -> approved`
- `draft -> archived`
- `proposed -> approved`
- `proposed -> rejected`
- `proposed -> archived`
- `approved -> active`
- `approved -> archived`
- `active -> approved`
- `active -> archived`
- `rejected -> archived`

### Activation Rule

Activating a release:

- moves the selected release to `active`
- moves any currently active release for the same project/selector/prompt-asset tuple back to `approved`
- emits a release action recording the replacement

There is never more than one `active` release for the same:

- `project_id`
- `prompt_asset_id`
- `rollout_target`

### Rollout Model In V1

V1 rollout is explicit activation by selector target.

That means:

- a release becomes live only when activated for a concrete `rollout_target`
- there is no percentage-based traffic splitting in v1
- there is no automatic weighted prompt experimentation in the runtime path in v1

If operators want side-by-side comparison in v1, they do it through:

- separate eval runs
- separate releases on distinct selectors
- explicit promotion or rollback decisions

This keeps runtime resolution deterministic and keeps evaluation separate from live traffic routing complexity.

## Target Resolution Rules

Prompt release selection must be deterministic.

### Selector Precedence

When multiple active releases could match a runtime context, precedence is:

1. `routing_slot`
2. `task_type`
3. `agent_type`
4. `project_default`

This means the most specific matching selector wins.

### Uniqueness Rule

For a given project, prompt asset, and selector, there may be at most one active release.

This removes tie-breaking ambiguity at runtime.

### Runtime Resolution Inputs

Runtime prompt resolution may use:

- project ID
- agent type
- task type
- routing slot

If a more specific selector is absent, the runtime falls back by precedence order until a matching active release is found.

## Scope Rules

Prompt scoping must follow RFC 008.

### Library Ownership

Prompt assets and prompt versions may be:

- tenant-scoped
- workspace-scoped

They are not project-scoped library objects.

### Deployment Ownership

Prompt releases are project-scoped.

This is the key separation:

- assets/versions are shared library material
- releases are project runtime choices

### Operator Preferences

Operator-local prompt preferences must not override project releases silently.

If operator-specific prompt experimentation exists later, it must be explicit and non-canonical.

## Release Rules

### Creation

Creating a new prompt body creates a new prompt version under an existing prompt asset or creates the asset first if needed.

### Proposal

To affect runtime behavior, a version must be proposed as a prompt release for a project and rollout target.

### Approval

Approval is required before a prompt release may become `active` when:

- the rollout target is shared across the project
- the release replaces an existing active release
- the project policy requires approval for prompt changes

For v1, assume approval is required for all releases that leave `draft`, unless the project policy explicitly allows `draft -> approved` without review.

That shortcut is still represented through the same canonical lifecycle:

- the release moves directly from `draft` to `approved`
- no separate approval field is introduced
- the approving actor and reason still appear in release actions

### Default Approval Policy In V1

V1 defines one hard default approval stance:

- any release that will become eligible for runtime activation must require approval

This means the default project policy is:

- `draft -> proposed` for human review
- `proposed -> approved` or `proposed -> rejected`
- `approved -> active` by explicit activation

Projects may relax this only through an explicit project policy that allows trusted direct approval from `draft -> approved`.

Even when that shortcut is enabled:

- it must be explicit project configuration
- the approving actor must still be recorded
- release actions must still show that approval occurred

V1 must not support silent auto-promotion from `draft` to `active`.

### Approval Policy Scope

Approval policy is project-scoped in v1.

Tenant or workspace defaults may seed the project policy, but runtime enforcement happens against the effective project policy.

This keeps rollout governance aligned with the project-scoped nature of prompt releases.

Tenant or workspace policy may also forbid the `draft -> approved` shortcut for regulated projects in v1.

When that stricter policy is in effect:

- `draft -> approved` is not allowed for the affected project
- releases must pass through `draft -> proposed -> approved`
- the canonical lifecycle model does not change; only the allowed transition set becomes stricter for that project

This keeps governance hardening explicit without introducing a second approval model.

In v1, this stricter rule should be exposed as a simple policy preset rather than a lower-level advanced flag surface.

That means:

- operators can select a regulated-project approval preset
- the preset forbids the `draft -> approved` shortcut
- lower-level flag composition for this rule is deferred until a later governance/policy refinement pass

### Activation

Activation is only valid from `approved`.

Only one active prompt release may exist per project/prompt asset/selector tuple.

### Rollback

Rollback must not mutate old releases.

Rollback has one canonical v1 behavior:

- it re-activates a previously approved release for the same project/prompt asset/selector tuple
- it emits a release action with `action_type=rollback` and explicit `from_release_id` / `to_release_id`

Rollback does not create a synthetic new release object in v1.

Required rollback properties:

- auditable actor and reason
- explicit linkage to the replaced release
- no loss of historical state

If the target prior version does not already have an approved release for that selector, operators must create and approve a normal release first.

## Runtime Binding Rules

Runtime execution must reference prompt releases, not raw prompt bodies, wherever the prompt came from the registry.

At minimum, a run/task/tool invocation that uses a registry-managed prompt must record:

- `prompt_asset_id`
- `prompt_version_id`
- `prompt_release_id` when applicable

This is required for:

- graph linkage
- eval comparison
- rollback confidence
- operator inspection

## Relationship to Agent Types and Skills

### Agent Types

Agent types should reference prompt release slots or prompt asset names, not inline prompt blobs, whenever they are governed by the registry.

For v1:

- inline prompt content may remain as a transitional compatibility path
- project-governed agent behavior should migrate toward explicit prompt releases

### Skills

Skills may ship prompt defaults, but those do not bypass the prompt registry.

If a skill contributes a managed prompt:

- it creates or updates a prompt asset/version in the appropriate scope
- runtime use still goes through a project release decision

## Relationship to Evals

Every eval run that evaluates prompt behavior must reference:

- `prompt_asset_id`
- `prompt_version_id`
- `prompt_release_id` when a release was under test

Evals must be able to answer:

- which release performed best?
- what changed between release A and B?
- should a release be promoted, held, or rolled back?

This means prompt registry and eval storage must be directly linked, not joined only through heuristics.

## Relationship to Graph

Graph entities must include:

- prompt asset nodes
- prompt version nodes
- prompt release nodes

And edges for:

- `derived_from`
- `released_as`
- `used_prompt`
- `evaluated_by`
- `rolled_back_to`

This gives operators provenance from runtime behavior back to prompt decisions.

## Operator Surfaces Required for V1

Aligned with RFC 010, the prompt system must support:

- prompt asset list/detail
- version history
- release list/detail by project
- release comparison
- approval queue visibility
- rollback action visibility
- eval linkage from release detail

These do not all need bespoke visual polish in v1, but the read models must exist.

## Initial Implementation Rules

- prompt versions are immutable
- prompt releases are the project-scoped deployable unit
- one active release per project/selector/prompt-asset tuple
- rollback is an explicit action, not hidden mutation
- rollback re-activates a prior approved release; it does not create a new release object in v1
- runtime usage must record prompt release linkage when applicable
- evals and graph must link directly to prompt objects

## Non-Goals

For v1, do not optimize for:

- arbitrary live traffic splitting across many prompt variants
- consumer-style experimentation UIs
- cross-tenant prompt sharing
- prompt templating as a standalone product
- fully automatic prompt mutation without operator visibility

## Open Questions

1. Should a later governance release expose the regulated-project no-shortcut rule as a lower-level policy flag after the simple preset has proven sufficient?

## Decision

Proceed assuming:

- prompt assets and versions are scoped library objects
- prompt releases are project-scoped deployable bindings
- versions are immutable
- release approval is represented in the canonical lifecycle state machine
- selector precedence is deterministic
- rollback always re-activates a prior approved release
- prompt/eval/graph linkage is required product state, not optional observability
- rollout in v1 is explicit selector-based activation, not percentage traffic splitting
- both `task_type` and `routing_slot` are first-class selector kinds in v1
- approval is required by default before a release becomes activation-eligible, with only explicit project policy allowing `draft -> approved`
- tenant/workspace policy may explicitly forbid the `draft -> approved` shortcut for regulated projects without changing the canonical lifecycle model
- the regulated-project no-shortcut rule is exposed in v1 as a simple policy preset, not a lower-level advanced flag surface
