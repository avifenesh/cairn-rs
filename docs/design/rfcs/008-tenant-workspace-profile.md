# RFC 008: Tenant, Workspace, Profile Separation

Status: draft  
Owner: architecture lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 002](./002-runtime-event-model.md)

## Summary

The Rust rewrite will use a four-layer ownership model:

- system
- tenant
- workspace
- project

Operator profile data is tenant-scoped but distinct from project/runtime data.

This RFC is intentionally concrete because storage, permissions, retrieval, graph, evals, and APIs cannot be safely built in parallel without one scoping contract.

## Why

The product is now explicitly team-facing, not a single-user agent environment.

That requires the rewrite to stop treating:

- personal profile data
- defaults
- runtime assets
- product-owned assets

as if they belong to the same scope.

## Scope Model

### System Scope

System scope is deployment-global and not customer-authored day-to-day.

Examples:

- builtin tool definitions
- plugin protocol definitions
- runtime binaries
- system feature flags
- product default templates shipped with the distribution

### Tenant Scope

Tenant is the top-level product ownership and security boundary.

Use tenant for:

- billing and licensing boundary later
- auth principal home
- credentials and provider configuration ownership
- tenant-wide policy baselines
- tenant-wide prompt libraries and datasets
- tenant-wide channel/provider/source configuration defaults

For v1:

- deployments may run in single-tenant mode
- but tenant IDs are still real and required in schema design

### Workspace Scope

Workspace is the team/environment boundary inside a tenant.

Use workspace for:

- team ownership
- environment separation
- access control
- workspace-level prompt libraries
- workspace-level policies
- workspace-level source subscriptions
- workspace-level channels and routing defaults

Suggested examples:

- `platform-prod`
- `support-ops`
- `research-lab`
- `staging`

### Project Scope

Project is the primary product unit for agent systems and runtime assets.

Use project for:

- agents
- sessions
- runs
- tasks
- checkpoints
- mailbox state
- memory corpora
- knowledge sources
- prompt releases
- eval runs
- routing overrides
- graph slices

Projects should be where operators feel they “run a thing.”

### Runtime Asset Rule

All runtime-owned execution assets must be project-scoped in v1.

This includes:

- sessions
- runs
- tasks
- approvals
- checkpoints
- mailbox messages
- tool invocation records
- project graph slices
- project eval runs
- project retrieval corpora and chunk projections

Workspace scope may own shared inputs, defaults, and policy sets, but not live execution truth.

Put differently:

- workspaces own shared configuration and team-level defaults
- projects own execution, memory, graph, prompt release, and eval reality

This resolves the remaining workspace-vs-project ambiguity for parallel workers.

## Operator Profile Model

Operator profile data is not product core logic and not project runtime state.

It should be modeled separately as tenant-scoped operator data.

Examples:

- user preferences
- communication defaults
- operator-specific working conventions
- notification preferences

This replaces the current informal mixing of profile and runtime concerns.

### Operator-Local Preference Rule

Operator-local preferences in v1 are limited to ergonomic or presentation-level behavior that must not silently change canonical project/runtime outcomes.

Safe operator-local preferences in v1 include:

- notification delivery preferences
- UI display preferences
- personal view/layout preferences
- non-canonical comparison or filter presets

Not safe as operator-local overrides in v1:

- effective prompt release selection
- effective policy/guardrail enforcement
- provider routing decisions
- project runtime permission grants
- canonical source or channel behavior

Those must remain controlled by system, tenant, workspace, or project-scoped product policy.

## Defaults Model

Defaults are layered and overrideable.

Order of precedence:

1. system defaults
2. tenant defaults
3. workspace defaults
4. project overrides
5. operator-local preferences where allowed

This rule must apply consistently to:

- prompts
- policies
- channels
- source settings
- tool permission presets
- starter skills

### Default Prompt-Library Placement Rule

V1 uses this canonical default:

- workspace-scoped prompt libraries are the default authoring and sharing layer for day-to-day team prompt work
- tenant-scoped prompt libraries are reserved for broadly shared or centrally governed prompt assets

That means:

- new prompt assets created through normal project/team workflows should default to workspace scope
- tenant scope should be used when the prompt library is intentionally shared across multiple workspaces or governed centrally

This keeps the default authoring experience aligned with team ownership while preserving tenant scope for real cross-workspace reuse.

### Centrally Governed Prompt Categories

In v1, the following prompt categories should default to tenant scope when they are governed centrally for multiple workspaces:

- safety-critical system prompts
- centrally mandated compliance or policy prompts
- tenant-wide routing or escalation prompts used across multiple workspaces

This is an exception to the workspace-first authoring rule, not a replacement for it.

If a prompt asset is:

- primarily team-owned and team-operated, default it to workspace scope
- centrally governed and intended for reuse across workspaces, default it to tenant scope

## Legacy File Mapping

The current personal overlay files should map into the new model as follows:

- `SOUL.md`
  - becomes a default or project-level agent identity/profile asset
  - not a global architectural assumption
- `USER.md`
  - becomes tenant-scoped operator profile data
- `AGENTS.md`
  - becomes policy/default-operation guidance at system, tenant, workspace, or project scope depending on content
- `MEMORY.md`
  - becomes curated project knowledge or tenant-curated reference material, not a global singleton

## Required Scope Matrix

### System-Scoped

- builtin tool catalog
- plugin protocol definitions
- shipped default templates
- runtime feature definitions

### Tenant-Scoped

- operators/users
- auth principals and identity providers
- provider credentials
- channel credentials
- tenant policy baselines
- tenant prompt libraries
- tenant eval datasets
- tenant-wide source connection definitions

### Workspace-Scoped

- workspace memberships
- workspace policy sets
- workspace prompt libraries
- workspace routing defaults
- workspace channels and source subscriptions
- workspace-level skill catalogs

### Project-Scoped

- agents and agent types in use
- sessions
- runs
- tasks
- approvals
- checkpoints
- mailbox messages
- tool invocation records
- memory corpora
- documents and chunks
- prompt releases
- eval runs
- project-specific policies
- graph nodes and edges for execution and knowledge

### Mixed-Scope With Explicit Rules

- skills
  - may be shipped at system scope, installed at tenant/workspace scope, and activated per project
- prompts
  - library ownership may be tenant/workspace scoped; release objects are project scoped
- policies
  - baseline policy may be tenant/workspace scoped; effective policy is resolved at project runtime

## Phase 1 Requirement

The following entities must be tenancy-aware in Phase 1:

- tenant
- workspace
- project
- operator/user
- session
- run
- task
- approval
- checkpoint
- mailbox message
- credential
- prompt asset
- prompt release
- eval run
- memory corpus
- source connection

No Phase 1 schema should be introduced for these entities without explicit scope fields.

## API Rules

- every request operates in an explicit tenant/workspace/project context where applicable
- project-scoped APIs must not infer ownership from a single global deployment state
- system-scoped APIs must be clearly separated from tenant/workspace/project APIs

## Permission Model Implications

Permissions should be evaluated against:

- actor
- tenant membership
- workspace role
- project role
- capability requested

This means permission tables and policy evaluation must be scope-aware from the beginning.

## Retrieval and Graph Implications

Retrieval objects and graph entities must carry:

- tenant ID
- workspace ID where applicable
- project ID where applicable

This is required for:

- access control
- graph slicing
- retrieval filtering
- eval isolation
- operator reasoning

## Single-Tenant v1 Clarification

Single-tenant deployment is acceptable for v1 operations.

That does not mean “no tenant model.”

It means:

- product UX may expose one tenant by default
- schema and APIs still carry tenant semantics
- future multi-tenant expansion does not require a destructive redesign

## Non-Goals

For v1, do not optimize for:

- complex B2B hierarchical org structures
- cross-tenant asset sharing
- enterprise federation beyond what auth requires
- billing design

Focus on a clean tenant/workspace/project model that works for technical teams.

## Open Questions

1. Should tenant-scoped centrally governed prompt libraries in v1 support mandatory inheritance into workspace defaults, or remain opt-in at workspace/project adoption time?

## Decision

Proceed with:

- mandatory system / tenant / workspace / project distinction in architecture
- operator profile modeled separately from runtime assets
- layered defaults with explicit override order
- project-scoped ownership for all runtime execution assets
- Phase 1 tenancy-aware schema for all core runtime, retrieval, graph, and eval entities
- workspace-scoped prompt libraries as the default authoring layer, with tenant scope reserved for centrally governed or cross-workspace libraries
- operator-local preferences limited to ergonomic and presentation-level behavior, not canonical runtime or policy outcomes
