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

1. Which of these surfaces must be fully visual in v1 versus acceptable as table/detail views first?
2. Which workflows must support bulk actions in v1?
3. How much multi-project and cross-workspace visibility is needed in the initial operator UX?

## Decision

Proceed assuming:

- the v1 control plane must include the minimum views and workflows listed above
- subsystem work should be shaped by these operator workflows, not only by backend elegance
