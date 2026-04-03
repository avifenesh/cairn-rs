# Cairn Rewrite RFCs

Status: active draft set  
Purpose: convert the rewrite plan into concrete architecture decisions that parallel workers can implement against

## Order

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
