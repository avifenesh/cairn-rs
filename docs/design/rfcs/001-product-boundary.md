# RFC 001: Product Boundary and Non-Goals

Status: draft  
Owner: architecture lead  
Depends on: rewrite plan only

## Summary

Cairn v1 should be positioned and built as a self-hostable control plane for production agents.

It is not a generic all-in-one AI platform, not a prompt IDE, not a gateway-first product, and not a memory SDK.

The product boundary should prioritize:

- durable agent runtime
- approvals, checkpoints, suspend/resume, mailbox, and recovery
- tool execution with explicit permissions
- product-owned memory and retrieval
- graph-backed introspection
- prompt/version/eval infrastructure
- multi-channel delivery and notification routing
- operator visibility and control

## Why

The current codebase already contains strong infrastructure across runtime, orchestration, tooling, retrieval, channels, and control-plane surfaces.

What is missing is not mostly capability. It is:

- clearer product shape
- a stronger team-facing rather than single-operator-facing boundary
- product-owned subsystems where convenience dependencies still sit
- operator workflows that make the system adoptable by teams

## Primary Customer

Primary customer:

- engineering and platform teams inside software companies
- building internal or product-facing agent systems
- who need reliability, control, observability, and self-hosting or hybrid deployment

These customers will value:

- one operational system instead of multiple disconnected vendors
- inspectability and replay
- control over memory, policies, and channels
- ability to run long-lived agent workflows with human gates

## Secondary Customer

Secondary customer:

- AI product teams shipping agentic workflows into a customer-facing product

They care about:

- runtime reliability
- prompt/version controls
- evaluation and routing
- approval and escalation flows

## Non-Target for V1

Do not optimize v1 primarily for:

- low-code hobby users
- teams who want only an AI gateway
- teams who want only prompt management
- teams who want only a vector store or memory SDK
- teams who do not care about operational control

## Included In Product Core

These belong in the Rust rewrite as first-class product systems:

- API, SSE, auth, admin control plane
- session/task/checkpoint/mailbox runtime
- orchestrator, ReAct loop, subagents, approvals
- tool runtime, permission matrix, sandbox boundary
- memory service, retrieval, reranking, KB, deep search
- graph layer for provenance and execution
- prompt registry, rollout controls, eval matrices
- signal plane, pollers, webhooks, digests
- channel adapters and notification router
- observability, event log, traces, replay
- plugin protocol and plugin host

## Included As Product Defaults

These should ship with the product but not be treated as core engine logic:

- bundled skills
- starter prompts
- starter rules and policies
- starter identity/profile templates
- default dashboards and operator views

## Excluded From Product Core

These should be runtime data, not product logic:

- `SOUL.md`
- `USER.md`
- `AGENTS.md`
- `MEMORY.md`
- tenant prompts and prompt data
- source subscriptions
- credentials
- routing targets
- user-specific thresholds and policies

## Compatibility Policy

The Rust rewrite should preserve product-defining semantics, not every current surface.

Before implementation, every inherited surface must be tagged as:

- preserved
- intentionally broken
- transitional

Preserve:

- durable runtime concepts
- approvals
- checkpoints
- mailbox coordination
- replay and operator visibility

Intentionally break:

- personal-agent assumptions that leak into product behavior
- single-operator conventions baked into APIs or state models
- overlay-specific behavior that conflicts with a team platform

Transitional:

- selected compatibility routes
- glide-mq-backed execution paths that remain temporarily for migration

The product boundary is only real if the break policy is explicit.

## Deployment Stance

Default stance:

- self-hosted first
- local mode and self-hosted team mode are first-class in v1
- hybrid remains architecture-compatible but is not a first-class supported operating model in v1
- hosted or managed control plane remains a later option, not a prerequisite for architecture

This means the product must work well when deployed by a technical team in their own environment.

## Packaging Direction

Not fully decided yet, but architecture should leave room for:

- open core
- paid enterprise/operator features
- optional managed or hybrid deployment
- support and service layers

## Product Success Criteria

The product boundary is correct if:

- a team can understand what Cairn is in one sentence
- the product feels like one system, not multiple glued tools
- runtime, memory, graph, and evals reinforce one another
- operators can control and inspect the system without founder mediation

## Minimum Operator Product Shape

The first release must include a real operator control plane, not just backend APIs.

At minimum, the product shape includes:

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

These surfaces may be table/detail-first except where relationship visualization is required to make the product legible.

## Non-Goals

For v1, do not aim to be:

- a standalone gateway competitor
- a standalone prompt IDE competitor
- a standalone vector database
- a standalone workflow automation suite
- a consumer personal-assistant platform

## Open Questions

1. Which enterprise features should remain explicitly out of scope for the first sellable self-hosted release?
2. Should a later release introduce a first-class hybrid control-plane mode once the self-hosted operating model is proven?
3. Which post-v1 deployment and governance features merit promotion from architecture-compatible to first-class?

## Decision

Proceed with the rewrite assuming:

- Cairn is a self-hostable control plane for production agents
- product core excludes personal profile content
- product value comes from coherent runtime + memory + graph + eval + operator control
- local mode and self-hosted team mode are first-class in v1
- hybrid is architecture-compatible but not a first-class supported v1 operating mode
- tenant/workspace/project scoping is required in architecture even when deployments run effectively single-tenant
- the first release includes the minimum operator control-plane views and workflows defined by RFC 010
