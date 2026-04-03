# RFC 014: Commercial Packaging and Entitlements

Status: draft  
Owner: product / commercial lead  
Depends on: RFC 001, RFC 010, RFC 011

## Summary

Cairn v1 should commercialize as one product with one core binary and one underlying architecture.

The first sellable motion is self-hosted team deployments.

Paid differentiation should come from named product entitlements, support, deployment help, and later managed operating models, not from a separate enterprise binary or a hosted-only architecture fork.

## Why

The architecture now assumes:

- local mode and self-hosted team mode are first-class
- one product binary with separable roles
- product-defined auth, runtime, retrieval, graph, eval, and operator surfaces

That means the remaining ambiguity is not technical feasibility. It is commercial shape:

- what teams pay for first
- how paid features are activated
- how later cloud or hybrid offerings relate to the same product
- which commercial surfaces deserve product UX in the first sellable release

If this is left implicit, the business model will drift independently from the product model.

## Goals

- define the first sellable commercial motion
- define how paid differentiation works without a product fork
- define the minimum entitlement model
- define how later managed cloud and hybrid offerings fit the same product
- keep v1 supportable and legible to buyers

## Non-Goals

For v1, do not define:

- final pricing
- detailed contract packaging
- a full in-product billing system
- revenue recognition or finance workflows
- multi-product SKU sprawl

## Commercial Motions

### V1 Motions

The first release should support these motions:

- local evaluation and development
- self-hosted team deployment as the first sellable offer
- enterprise self-hosted expansion through named entitlements and support layers

### Later Motions

The product should leave room for:

- managed cloud
- hybrid control-plane operation
- additional compliance/governance packages
- marketplace or ecosystem monetization

Those later motions must build on the same product semantics rather than requiring a second architecture.

## Packaging Model

The v1 packaging contract is:

- one codebase
- one product binary
- one control-plane model
- one deployment-local product semantics model

This means Cairn should not ship:

- a separate enterprise binary
- a different hosted-only feature architecture
- hidden config-only forks that materially change product behavior

### Product Tiers In Practice

The practical v1 packaging posture is:

- `local_eval`
  - local mode for development, evaluation, and solo proving
- `team_self_hosted`
  - the first sellable deployment package
- `enterprise_self_hosted`
  - the same deployment model with additional entitlements, support, and governance/compliance layers

Later:

- `managed_cloud`
- `hybrid_control_plane`

These are commercial motions over the same product model, not separate products.

## Entitlement Model

Paid differentiation should use named product entitlements.

The canonical v1 contract is:

- entitlements are explicit deployment or tenant inputs
- entitlements gate named capabilities
- entitlements are inspectable in operator surfaces
- entitlement changes fail predictably and must not corrupt canonical product state
- entitlement absence must degrade by refusing gated operations, not by silently mutating prior state

### Entitlement Categories

V1 should think in a small number of entitlement categories:

- deployment and support tier
- governance and compliance
- advanced operator administration
- future managed-service rights

V1 should avoid a long list of tiny toggles that makes packaging hard to understand.

### What Entitlements Should Not Do

Entitlements should not:

- change runtime truth models
- fork storage semantics
- create hidden API behavior differences unrelated to named capabilities
- become an unstructured vendor-specific flag bag

## First Sellable Offer

The first sellable offer is `team_self_hosted`.

It is sold as:

- the self-hosted team-mode product
- supportable deployment guidance
- product-defined auth and secret-handling posture
- operator-visible control-plane workflows

It does not depend on:

- Cairn-operated cloud services
- hosted billing infrastructure
- a managed multi-customer control plane

### Included In The First Sellable Offer

The first sellable offer includes:

- the self-hosted team-mode deployment defined in RFC 011
- the operator control-plane floor defined in RFC 010
- the product core defined in RFC 001 and the downstream technical RFCs
- product-visible auth, entitlement, and deployment health status sufficient for team operation

### Explicitly Deferred Beyond The First Sellable Offer

The first sellable offer does not require:

- first-class managed cloud
- first-class hybrid operation
- in-product billing or metering workflows
- a marketplace monetization layer
- per-binding or per-node commercial quota systems

## First Paid Expansion Areas

After the first sellable self-hosted release proves out, the first commercial expansions should come from areas that deepen enterprise value without forking the core product.

The most likely early paid expansion areas are:

- stronger governance/compliance controls
- enterprise administration and audit workflows
- higher-touch support and deployment assurance
- later managed or hybrid operating options

### First Paid Expansion Choice

The first paid expansion after `team_self_hosted` should be a governance/compliance package.

That package should focus on capabilities that increase enterprise trust and control without changing core runtime semantics.

The initial paid governance/compliance family should center on:

- advanced audit export and governance reporting
- stricter compliance-oriented policy packs and approval hardening
- stronger artifact trust and verification layers where required for regulated environments

This should be the first paid step-up because it:

- maps cleanly to self-hosted buyer concerns
- strengthens the operator/control-plane story rather than diluting it
- avoids forcing managed cloud to become the first monetization path
- creates a clear enterprise boundary without requiring a second product architecture

Examples of likely later paid capability families:

- stricter compliance-oriented policy packs
- stronger artifact trust/integrity layers
- advanced audit export and governance reporting
- directory lifecycle or enterprise identity administration beyond the v1 minimum auth floor
- managed upgrade/support envelopes

These are commercial expansion areas, not part of the v1 technical minimum.

## Managed Cloud And Hybrid Path

Managed cloud and hybrid should be treated as later business motions on top of the same product.

The rule is:

- do not block v1 success on running Cairn as a SaaS
- do not design SaaS as a separate product
- do keep the product architecture compatible with later managed and hybrid deployment

This means:

- one binary and one control-plane model now
- later managed packaging reuses the same runtime, entitlement, and operator semantics

## Operator UX For Commercial Surfaces

V1 should include only the commercial surfaces required to operate a supportable self-hosted product.

Required v1 operator-visible commercial surfaces:

- entitlement/license status
- capability availability where gated features appear
- deployment/auth/credential health where commercial supportability depends on it

### Early Post-V1 Entitlement UX

The first entitlement surfaces that deserve dedicated operator workflows after v1 are:

- entitlement status and scope inspection
- capability-to-entitlement mapping for gated features
- entitlement change/audit visibility for operators

These remain operational and administrative surfaces, not purchasing flows.

Not required in v1:

- in-product purchasing flows
- complex commercial analytics
- detailed billing dashboards

## Feature Rollout Rule

New features should roll out on the same product artifact.

The rollout rule is:

- experimental features may be hidden behind explicit preview flags or non-default entitlements
- general-release features should either become part of the base product contract or a named commercial entitlement
- paid features must be introduced as named capability surfaces, not as invisible behavior changes

This keeps roadmap, support, and packaging aligned.

## Supportability Rule

Commercial packaging should follow supportability, not wishful breadth.

That means:

- only a small number of supported deployment motions in v1
- only a small number of understandable entitlement categories in v1
- examples and reference manifests may exist without becoming first-class support commitments

## Open Questions

1. What evidence should trigger promotion of managed cloud or hybrid from later motion to first-class commercial offer?
2. How much of the governance/compliance package should be bundled together in the first paid step-up versus phased across later releases?
3. Which entitlement operator workflows should move from status/inspection into stronger administrative tooling in the first post-v1 release?

## Decision

Proceed assuming:

- Cairn commercializes first as a self-hosted team product
- one codebase, one binary, and one core product model apply across local and self-hosted modes
- paid differentiation in v1 comes from named entitlements, supportability, and deployment/compliance layers rather than a separate architecture fork
- the first sellable motion is self-hosted team deployment, not managed cloud
- managed cloud and hybrid are later business motions on the same product model
- v1 commercial UX includes entitlement/license visibility and gated capability awareness, not billing workflows
- the first paid expansion after the initial self-hosted release should be a governance/compliance package rather than a managed-cloud-first upsell
