# RFC 012: Onboarding and Starter Templates

Status: draft  
Owner: product/UX lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 006](./006-prompt-registry-release-model.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 010](./010-operator-control-plane-ia.md), [RFC 011](./011-deployment-shape.md)

## Summary

V1 must define a concrete path from installation to first value.

Cairn should ship with:

- a canonical bootstrap flow
- a small set of product-owned starter templates
- import paths for prompts, documents, and memory corpora
- explicit separation between shipped defaults and customer-authored data

The goal is that a new team can install Cairn, create a first project, run a useful agent flow, and inspect it in the control plane without founder-guided setup.

## Why

The product is now defined as:

- a self-hostable control plane for production agents

That is not enough by itself. A product with strong internals still fails adoption if teams cannot:

- get it running quickly
- understand what to configure first
- reach a meaningful first run
- see a coherent example of prompts, policies, memory, and tools working together

This RFC exists so onboarding does not drift into:

- bespoke operator setup
- personal-agent assumptions
- ambiguous starter content
- inconsistent import behavior

## Product Requirement

V1 onboarding is successful only if a new technical team can:

1. install the product locally
2. create an initial tenant/workspace/project
3. choose a starter template
4. configure at least one provider and one operator
5. import or create initial prompt and knowledge assets
6. run a first useful workflow
7. inspect the result in the operator control plane

If one of those steps requires ad hoc founder knowledge, onboarding is incomplete.

## Canonical V1 Onboarding Flows

### 1. Local Quickstart Flow

Purpose:

- prove value fast
- support evaluation and internal exploration

The local quickstart must:

- install and run with the local mode assumptions from RFC 011
- create one tenant, one default workspace, and one starter project
- provision starter prompts, starter policies, and starter views
- guide the operator to a first successful run

The quickstart is the shortest path to “I see Cairn working.”

### 2. Team Bootstrap Flow

Purpose:

- get a real team deployment into an operable first state

The team bootstrap must:

- assume self-hosted team mode from RFC 011
- initialize Postgres-backed product state
- create the first tenant/workspace/project structure
- establish at least one operator account and one provider connection
- let the team choose starter templates intentionally
- make the first operator workflows from RFC 010 reachable without custom data migration work

This is the shortest path to “our team can operate this for real.”

### 3. Existing Asset Import Flow

Purpose:

- let teams bring in useful material instead of starting from empty state

V1 must support import paths for:

- prompt assets and versions
- documents and knowledge sources
- memory corpora or curated knowledge packs

Import does not need to cover every old system in v1, but it must exist for the most important product surfaces.

## Canonical First Value

V1 first value is not “the UI opened.”

The canonical first value moment is:

- the team runs a starter agent workflow against real or imported knowledge,
- sees the runtime in the control plane,
- and can inspect prompts, memory/retrieval, and run outcomes in one system.

That means starter content must exercise:

- runtime and tasking
- prompt registry
- retrieval or memory
- operator visibility

## Shipped Starter Templates

V1 should ship with a small, opinionated set of templates.

Do not ship a giant catalog in v1.

### Required Starter Template Categories

#### 1. Knowledge Assistant

Purpose:

- demonstrate owned retrieval, prompts, and operator inspection

Should include:

- a retrieval-aware agent configuration
- starter prompts
- starter memory/retrieval policy defaults
- a minimal evaluation dataset or example traces where feasible

#### 2. Approval-Gated Worker

Purpose:

- demonstrate runtime durability, approvals, and operator control

Should include:

- an agent or workflow that can request approval
- starter approval policy
- visible checkpoints and run inspection

#### 3. Multi-Step Operator Workflow

Purpose:

- demonstrate orchestration, tools, and control-plane visibility

Should include:

- a workflow with at least two meaningful stages
- starter tool permission presets
- run/task visibility in the control plane

These templates should be enough to show the product shape without requiring dozens of starter packs.

## Starter Asset Types

Starter templates may include:

- prompt assets and prompt versions
- starter prompt releases
- policy presets
- starter knowledge/source configuration examples
- starter dashboards and operator views
- starter skill packs where those are product defaults

Starter templates must not silently include user-specific profile content.

## Separation Between Product Defaults and Customer Data

RFC 001 and RFC 008 already separate product defaults from customer-owned data. RFC 012 makes the onboarding consequence explicit.

### Product-Owned Defaults

These may ship with the product:

- starter templates
- starter prompts
- starter policies
- starter dashboards
- starter skill packs
- starter profile/identity templates as reusable assets

### Customer-Owned Runtime Data

These must be created, selected, imported, or edited by the customer:

- tenant prompts and prompt releases beyond shipped defaults
- imported documents and corpora
- source subscriptions and connection credentials
- team-specific policies
- project-specific identity/profile assets

The bootstrap flow must make this distinction clear.

### Canonical Materialization Rule

Shipped starter templates and shipped starter assets are system-scoped defaults.

They must not become live customer runtime state by reference alone.

The canonical rule in v1 is:

1. the product ships immutable system-scoped starter definitions
2. template selection records which starter template was chosen
3. bootstrap materializes customer-scoped state from that template into tenant, workspace, or project scope as appropriate
4. runtime and operator workflows operate on the materialized customer-scoped objects, not on the shipped system template objects

This prevents starter content from behaving like hidden global state.

### Materialization Rules By Asset Type

#### Prompts

- shipped starter prompt assets and versions may exist at system scope as product defaults
- if a team chooses a starter template, any prompt assets that are expected to become customer-managed library content must be copied into tenant or workspace scope
- any prompt used at runtime must be represented through project-scoped prompt releases per RFC 006
- shipped system prompt assets must not be mutated in place by customer actions

#### Policies

- shipped starter policies are system-scoped defaults
- effective policies used by a project must be materialized into workspace or project scope before enforcement
- customer edits must apply to the materialized scoped policy objects, not the shipped system defaults

#### Skills and Starter Packs

- shipped starter skill packs may remain system-scoped installable defaults
- project or workspace activation must be recorded as customer-scoped product state
- if a team customizes a shipped skill pack beyond activation settings, the customized result must become tenant/workspace/project-owned state rather than mutating the shipped default

#### Dashboards and Operator Views

- shipped starter views may remain system-scoped templates
- workspace or project enablement must still be recorded explicitly
- customized operator views must become customer-scoped state

### Upgrade Rule

Later product upgrades may ship improved starter defaults.

Those upgrades must not silently rewrite previously materialized customer-scoped prompt, policy, skill, or view state.

Applying updated starter content after bootstrap must be an explicit operator action.

## Bootstrap Objects

The bootstrap flow must create a minimal but real product state.

Minimum objects created during setup:

- tenant
- default workspace
- at least one project
- at least one operator identity
- at least one provider connection
- at least one provider binding or starter route template materialized into a project binding
- starter prompt assets and releases required by the chosen template
- starter policy set required by the chosen template

Bootstrap must not rely on hidden global singletons outside the scoped model from RFC 008.

Bootstrap must also persist starter provenance so the operator can answer:

- which starter template this project began from
- which materialized objects came from that template
- whether those objects still match the original shipped defaults or have diverged

## Import Paths

### Prompt Import

V1 prompt import must support:

- importing prompt assets and versions from files or structured export format
- mapping imported prompts into tenant or workspace libraries
- optionally creating project releases from imported versions

### Document and Knowledge Import

V1 knowledge import must support:

- local document/file import
- structured metadata capture
- ingest into owned retrieval pipelines

The onboarding flow must expose how imported knowledge becomes searchable and inspectable.

### Memory Import

V1 does not need arbitrary raw memory import from every prior system.

It should support:

- curated knowledge pack import
- project corpus seeding
- explicit operator review of imported content where needed

## Installer and Bootstrap Contract

### Canonical Bootstrap Operation

V1 may expose bootstrap through:

- CLI
- UI wizard
- API

But those surfaces must all drive one canonical bootstrap operation and one canonical bootstrap data model.

Workers should not invent separate semantics for:

- CLI-first bootstrap
- UI-first bootstrap
- API-first bootstrap

### Idempotence Rule

Bootstrap must be idempotent with respect to its target scope and template selection.

Running bootstrap twice against the same target with the same requested configuration must:

- reuse the same tenant/workspace/project where appropriate
- avoid duplicating starter assets and releases unnecessarily
- report what already exists versus what was newly materialized

If the operator wants a second project or a second independent starter instantiation, that must be an explicit create-new action, not an accidental side effect of rerunning bootstrap.

### Canonical Import Contract

V1 import may have multiple entry surfaces, but it should use one canonical import service model.

At minimum, the import model should support:

- structured bundle import for prompts and curated knowledge packs
- direct file/document import for knowledge sources

The same canonical import rules must apply regardless of whether the operator starts import from CLI, API, or control-plane UI.

### Local Mode

The local installer/bootstrap path should be:

- one install path
- one start command
- one bootstrap step or wizard

The operator should not need to manually construct database rows or edit multiple internal config files to reach first value.

### Team Mode

The self-hosted team bootstrap path should guide:

- database readiness
- first operator creation
- first provider connection
- first workspace/project creation
- starter template selection

It may be wizard-driven, CLI-assisted, or API-driven, but the product must define one canonical path.

## Control Plane Onboarding Surfaces

The operator control plane must include onboarding-aware surfaces, not only steady-state admin views.

Minimum v1 onboarding surfaces:

- bootstrap status or first-run checklist
- template selection view or flow
- provider setup entrypoint
- import entrypoint for prompts and documents
- first project status view

These surfaces may be simple in v1, but they must exist.

## Defaults and Overrides

Starter templates are defaults, not hidden mandatory behavior.

Rules:

- a team may start from a template and then replace shipped prompts, policies, and skill packs
- shipped defaults must be inspectable
- shipped defaults must be overrideable through the scoped model from RFC 008
- starter template usage must be recorded as product state so operators know what their project began from

## Packaging Implications

RFC 011 requires:

- one local bootstrap path
- one production self-hosted deployment path

RFC 012 adds:

- one canonical first project bootstrap path per mode
- one canonical starter-template system
- one canonical import path for core product assets

These should be treated as product requirements, not just documentation tasks.

## Non-Goals

For v1, do not optimize for:

- a large marketplace of starter templates
- drag-and-drop low-code onboarding
- arbitrary import from every agent framework
- zero-opinion setup with no starter defaults
- deeply personalized founder-style starter profiles

The goal is a small, opinionated path to first value.

## Open Questions

1. Which starter templates are mandatory in the first sellable release, and which can be deferred?
2. How much of onboarding should be UI-first versus CLI-assisted in v1?
3. Should prompt import and document import use one shared artifact import pipeline or separate paths in v1?

## Decision

Proceed assuming:

- v1 must include a canonical bootstrap path
- v1 must ship a small, opinionated starter-template set
- onboarding must reach a real first-value workflow, not only installation success
- shipped defaults remain distinct from customer-owned runtime data
