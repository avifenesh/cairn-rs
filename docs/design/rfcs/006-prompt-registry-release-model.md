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
- `approval_state`
- `created_at`
- `created_by`

`state` in v1:

- `draft`
- `proposed`
- `approved`
- `active`
- `rolled_back`
- `replaced`
- `archived`

`rollout_target` in v1:

- `project_default`
- `agent_type`
- `task_type`
- `routing_slot`

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

For v1, assume approval is required for all non-draft releases unless the project policy explicitly allows auto-activation.

### Activation

Only one active prompt release may exist per:

- project
- rollout target
- prompt asset

Activating a new release automatically marks the previous active release for the same tuple as `replaced`.

### Rollback

Rollback must not mutate old releases.

Rollback creates a new release action and re-activates a prior approved release or creates a rollback release that points at the prior prompt version.

Required rollback properties:

- auditable actor and reason
- explicit linkage to the replaced release
- no loss of historical state

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
- one active release per project/rollout-target/prompt-asset tuple
- rollback is an explicit action, not hidden mutation
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

1. Do we need percentage rollout or can v1 stop at explicit release activation per target?
2. Which rollout targets beyond `project_default` and `agent_type` are truly needed in v1?
3. Which approval policies should be hard defaults versus project-configurable rules?

## Decision

Proceed assuming:

- prompt assets and versions are scoped library objects
- prompt releases are project-scoped deployable bindings
- versions are immutable
- releases are auditable and approval-aware
- rollback is explicit and durable
- prompt/eval/graph linkage is required product state, not optional observability
