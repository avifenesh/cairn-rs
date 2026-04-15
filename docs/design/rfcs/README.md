# Cairn Rewrite RFCs

Status: active RFC set for the cairn-rs rewrite  
Purpose: concrete architecture decisions for the cairn-rs control plane

## Phase 1 RFCs (implemented)

1. [RFC 001 - Product Boundary and Non-Goals](./001-product-boundary.md)
2. [RFC 002 - Runtime and Event Model](./002-runtime-event-model.md)
3. [RFC 003 - Owned Retrieval Replacing Bedrock KB](./003-owned-retrieval.md)
4. [RFC 004 - Graph and Eval Matrix Model](./004-graph-eval-matrix.md)
5. [RFC 005 - Task, Session, Checkpoint Lifecycle](./005-task-session-checkpoint-lifecycle.md)
6. [RFC 006 - Prompt Registry and Release Model](./006-prompt-registry-release-model.md)
7. [RFC 007 - Plugin Protocol and Transport](./007-plugin-protocol-transport.md)
8. [RFC 008 - Tenant, Workspace, Profile Separation](./008-tenant-workspace-profile.md)
9. [RFC 009 - Provider Abstraction and Gateway Non-Goals](./009-provider-abstraction.md)
10. [RFC 010 - Operator Control Plane IA](./010-operator-control-plane-ia.md)
11. [RFC 011 - Deployment Shape](./011-deployment-shape.md)
12. [RFC 012 - Onboarding and Starter Templates](./012-onboarding-starter-templates.md)
13. [RFC 013 - Artifact Import/Export Contract](./013-artifact-import-export-contract.md)
14. [RFC 014 - Commercial Packaging and Entitlements](./014-commercial-packaging-and-entitlements.md)

## Phase 2 RFCs (implemented)

15. [RFC 015 - Plugin Marketplace and Scoping](./015-plugin-marketplace-and-scoping.md) — VisibilityContext, plugin catalog, marketplace routes
16. [RFC 016 - Sandbox Workspace Primitive](./016-sandbox-workspace-primitive.md) — cairn-workspace crate, repo store, sandbox lifecycle
17. [RFC 017 - GitHub Reference Plugin](./017-github-reference-plugin.md) — marketplace reference plugin, GitHub App auth, webhook normalization
18. [RFC 018 - Agent Loop Enhancements](./018-agent-loop-enhancements.md) — RunMode (Plan/Execute/Direct), Guardian resolver, context compaction, tool visibility
19. [RFC 019 - Unified Decision Layer](./019-unified-decision-layer.md) — DecisionService, 8-step pipeline, singleflight cache, learned rules
20. [RFC 020 - Durable Recovery](./020-durable-recovery.md) — startup dependency graph, dual checkpoint, ToolCallId idempotency, RetrySafety enforcement
21. [RFC 021 - Control Plane Protocols](./021-control-plane-protocols.md) — SQ/EQ protocol, A2A Agent Card, OTLP GenAI export
22. [RFC 022 - Triggers](./022-triggers.md) — signal capture, trigger evaluation, fire-once semantics
23. [RFC 023 - Business Model and Cloud Architecture](./023-business-model-and-cloud-architecture.md) — BSL 1.1 licensing, cloud architecture, commercial packaging

## How To Use These RFCs

- Treat these as draft decision documents, not final law.
- Resolve open questions before large implementation branches diverge.
- When an RFC changes, update the rewrite plan and any dependent RFCs.
- Prefer tightening scope over adding parallel half-systems.
- Use the compatibility docs under `docs/design/` as required companion inputs for runtime/API migration work.

## Decision Sequence

- RFC 001 defines what the product is.
- RFC 002 defines how the runtime behaves.
- RFC 003 defines how knowledge and retrieval become product-owned.
- RFC 004 defines how graph and eval systems become first-class product surfaces.

## Rule For Follow-On RFCs

No new RFC should contradict these without explicitly amending them.
