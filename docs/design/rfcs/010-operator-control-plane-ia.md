# RFC 010: Operator Control Plane Information Architecture

Status: draft  
Owner: product/UX lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 004](./004-graph-eval-matrix.md), [RFC 008](./008-tenant-workspace-profile.md)

## Summary

The v1 operator control plane must ship as one coherent product surface.

Minimum top-level operator views:

- overview
- runs
- approvals
- memory
- graph
- prompts
- evals
- policies
- sources and channels
- settings

If these surfaces do not exist in some usable form, the product will still read as infrastructure, not a control plane.

## Why

The rewrite now positions Cairn as:

- a self-hostable control plane for production agents

That promise is only credible if operators can:

- see what is happening
- approve or stop work
- inspect memory and retrieval
- compare prompt/eval outcomes
- inspect policy and permission outcomes
- manage sources and channels

## Minimum V1 Views

### Visual Fidelity Rule

V1 does not require every operator surface to be deeply visual, but it does require every named surface to be product-usable.

The canonical v1 rule is:

- table/detail-first is acceptable where the workflow is primarily operational
- explicitly visual views are required where the product promise depends on relationship or provenance understanding

In v1 specifically:

- `overview`, `runs`, `approvals`, `prompts`, `evals`, `policies`, `sources and channels`, and `settings` may ship as strong table/detail views first
- `memory` must include result/provenance inspection views, but may remain largely table/detail with targeted explanation panels
- `graph` must include a genuinely visual relationship view in v1, not only raw tables

This keeps the control plane credible without forcing every area into bespoke visual polish on day one.

### 1. Overview

Purpose:

- current system health
- active runs/tasks
- blocked approvals
- failed or degraded components
- recent critical events

### 2. Runs

Purpose:

- list and inspect runs
- see status, timings, failures, and linked tasks
- drill into tool activity and checkpoints

### 3. Approvals

Purpose:

- single inbox for human-in-the-loop decisions
- approve, deny, defer
- see why approval is required
- see affected project/run/task

### 4. Memory

Purpose:

- inspect corpora, documents, chunks, and retrieval behavior
- understand why a result was returned
- view source quality and ingestion health

### 5. Graph

Purpose:

- visualize execution provenance
- inspect relationships between tasks, prompts, tools, memories, and outcomes
- answer “why did this happen?”

### 6. Prompts

Purpose:

- manage prompt assets
- inspect versions and release tags
- compare releases
- roll forward and back

### 7. Evals

Purpose:

- inspect scorecards and matrices
- compare prompt/model/tool/policy outcomes
- review dataset-linked evaluation runs

### 8. Policies

Purpose:

- inspect effective permissions and guardrails
- see policy hits and denials
- understand why a tool/action was blocked or allowed

### 9. Sources and Channels

Purpose:

- inspect signal source health
- inspect channel routing and delivery health
- troubleshoot pollers and notification paths

### 10. Settings

Purpose:

- manage tenant/workspace/project configuration
- provider settings
- credentials metadata
- deployment and runtime settings that belong in-product

## Minimum V1 Operator Workflows

The v1 product must support these workflows end-to-end:

### Workflow A: Failed Run Triage

Operator can:

- open a failed run
- see related tasks, checkpoints, and tool activity
- see likely cause
- decide whether to retry, resume, or intervene

### Workflow B: Approval Decision

Operator can:

- see pending approvals in one place
- understand context and impact
- approve or deny
- watch the blocked work resume or terminate

### Workflow C: Retrieval Debugging

Operator can:

- inspect a retrieval result
- see source/provenance/scoring context
- understand why it was surfaced
- identify bad or stale inputs

### Workflow D: Prompt Release Comparison

Operator can:

- compare two prompt releases
- view eval impact
- choose rollout, rollback, or hold

### Workflow E: Policy Inspection

Operator can:

- see what policy blocked or allowed an action
- understand effective scope and reason
- identify misconfiguration

## Bulk Action Rule

V1 bulk actions are required only where they materially reduce operator toil in core workflows.

Required bulk actions in v1:

- approvals: approve, deny, or defer multiple compatible approval items
- sources/channels: retry or pause/resume multiple failing or degraded sources where the action semantics are safe
- prompts: archive or label multiple releases/versions where no activation-state conflict exists

Explicitly not required as bulk actions in v1:

- bulk rollback of prompt releases
- bulk retry/resume of arbitrary runs
- bulk policy rewrites across unrelated scopes

Where bulk actions exist, the UI must show:

- scope of effect
- affected item count
- whether conflicts or ineligible items were skipped

This keeps v1 operable without turning the first release into an admin automation suite.

## Visibility Scope Rule

The canonical v1 operator-visibility stance is:

- project-scoped operation is first-class
- workspace-level visibility is first-class for aggregation and drill-down
- tenant-level visibility exists primarily for settings, policy inheritance, credential/provider administration, and roll-up health

Cross-workspace operational control is not a primary v1 workflow.

That means:

- operators can aggregate across projects inside a workspace
- tenant/workspace surfaces may summarize project health and counts
- most actionable workflow views should drill into one project context before allowing state-changing actions

This keeps the first release aligned with RFC 008 scoping and reduces accidental cross-project blast radius.

### Tenant Roll-Up Mutability Rule

In v1, tenant-level roll-up views are read-only for operational workflow actions.

Allowed tenant-level mutations in v1 are limited to:

- settings administration
- policy-baseline administration
- credential and provider administration
- workspace or project provisioning actions where those belong in-product

Tenant-level roll-up views must not be the place for:

- bulk run retry or resume
- bulk prompt rollout or rollback
- bulk source/channel operational intervention across unrelated projects
- other broad operational mutations that bypass explicit project context

This keeps tenant scope aligned to administration and governance rather than becoming a high-blast-radius operations surface in the first release.

## Navigation Rule

The control plane should be product-first, not subsystem-first.

That means:

- do not create separate mini-products for runtime, memory, prompts, and channels
- use shared scoping, shared navigation, and shared audit context

## V1 Non-Goals

For v1, do not optimize for:

- custom dashboard builders
- arbitrary BI/reporting
- highly personalized navigation
- every operator workflow being deeply polished

Focus on the minimum workflows that make the product operable.

## Open Questions

1. Should workspace-level aggregated views in v1 allow a limited set of bulk operational actions across projects when all affected items share the same workspace policy boundary?

## Decision

Proceed assuming:

- the v1 control plane must include the minimum views and workflows listed above
- subsystem work should be shaped by these operator workflows, not only by backend elegance
- `graph` is the only surface that requires a genuinely visual relationship view in v1; the rest may be table/detail-first where appropriate
- bulk actions are required only for approvals, selected source/channel operations, and safe prompt housekeeping actions
- project-scoped and workspace-aggregated visibility are first-class; most operational mutations happen within an explicit project context
- tenant-level roll-up views are read-only for operational actions in v1, except for settings, policy, credential/provider, and provisioning administration
