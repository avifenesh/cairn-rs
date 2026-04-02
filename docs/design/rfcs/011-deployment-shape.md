# RFC 011: Deployment Shape

Status: draft  
Owner: architecture/product lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 007](./007-plugin-protocol-transport.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 009](./009-provider-abstraction.md), [RFC 010](./010-operator-control-plane-ia.md)

## Summary

Cairn v1 is a self-hosted-first product with two first-class deployment modes:

- local mode for development, personal use, and evaluation
- self-hosted team mode for production use inside the customer environment

The architecture must remain hybrid-capable, but hybrid deployment is not a first-class supported operating model in v1.

Fully managed multi-customer control plane operation is explicitly out of scope for v1.

## Why

The product boundary in RFC 001 says Cairn is:

- a self-hostable control plane for production agents

That promise needs a concrete deployment stance because deployment assumptions directly shape:

- auth
- tenancy
- secrets and credentials
- plugin isolation
- local mode expectations
- network boundaries
- operator workflows

If deployment shape remains abstract, workers will make incompatible assumptions about where state lives, which trust boundary owns secrets, whether plugins can run remotely, and whether hosted control plane behavior is required in v1.

## Product Goals

The deployment model must let a technical team:

- run Cairn fully inside their own environment
- operate it without a managed control-plane dependency
- use the same product in local mode for development and evaluation
- grow from a single-node setup to a team production setup without changing the product contract

The architecture should still leave room for later hybrid and managed offerings without forcing v1 to act like a hosted SaaS product.

## Canonical V1 Deployment Modes

### 1. Local Mode

Local mode is first-class for:

- development
- personal use
- design validation
- small-scale evaluation

Local mode characteristics:

- one tenant
- one operator or a very small trusted group
- SQLite allowed
- local filesystem storage allowed
- plugins launched locally over stdio
- local or hosted model providers allowed
- reduced scale and availability expectations

Local mode is a real product mode, not a toy shell. It must preserve product-owned behavior, but it is not the scale or operability target for production teams.

### 2. Self-Hosted Team Mode

Self-hosted team mode is the primary production mode for v1.

Characteristics:

- deployed inside the customer environment
- Postgres required as the canonical system of record
- `pgvector` available for owned retrieval
- one customer organization per deployment
- tenant/workspace/project boundaries still modeled in-product
- plugins remain out-of-process and customer-side
- credentials remain under customer control
- channels, sources, pollers, and provider traffic originate from the customer environment unless explicitly proxied by the customer

This is the mode Cairn should optimize for operationally in v1.

## Not First-Class In V1

### Hybrid Deployment

Hybrid deployment means some control-plane or management surface is hosted outside the customer environment while runtime and sensitive execution stay customer-side.

The Rust architecture must remain hybrid-capable, but hybrid is not a first-class supported operating mode in v1.

That means:

- do not hard-code assumptions that prevent hybrid later
- do not require hybrid to make v1 work
- do not promise hybrid-specific operational guarantees in v1

### Managed Multi-Customer Control Plane

Managed multi-customer control plane deployment is out of scope for v1.

Do not shape v1 around:

- shared hosted tenancy as the default operating assumption
- provider credential escrow by Cairn-operated infrastructure
- remote plugin meshes as a required architecture element
- product assumptions that only make sense for SaaS-first deployment

The product may leave room for that future, but it is not the deployment contract workers should target today.

## Canonical Topology

### V1 Deployment Units

The product should support a single binary with separable roles.

Canonical roles:

- API/control-plane role
- runtime worker role
- scheduler/poller role
- plugin host role

Small deployments may run all roles together.

Team/production deployments should be able to split roles across processes or instances without changing product semantics.

### Canonical Shared Dependencies

Local mode:

- SQLite
- local filesystem
- local plugin processes

Self-hosted team mode:

- Postgres
- `pgvector`
- object/blob storage only when needed for artifacts later
- optional message-queue substrate only where transitional or operationally justified

The product contract must not require a managed external control-plane dependency for either first-class mode.

## Storage Rules

### Local Mode Storage

Local mode may use:

- SQLite as the system of record
- local disk for artifacts and caches

This mode may be degraded in:

- concurrency
- retrieval scale
- durability expectations under operator error

But it must still preserve core product semantics.

### Team Mode Storage

Team mode must use:

- Postgres as the canonical store

Owned retrieval and graph/eval surfaces must assume Postgres-first production behavior.

SQLite is not a production team-mode target.

## Tenancy and Identity Implications

### V1 Customer Boundary

The first-class deployment boundary in v1 is:

- one customer organization per deployment

Inside that deployment, Cairn still models:

- tenant
- workspace
- project
- operator/user

This is compatible with RFC 008:

- single-tenant deployment is operationally first-class
- tenant IDs remain real product objects

### Auth Model Implications

V1 auth should support:

- local auth suitable for development/local mode
- self-hosted team auth suitable for one customer environment

The architecture must leave room for:

- external identity providers
- enterprise auth later

But v1 must not require a hosted identity control plane operated by Cairn.

### Minimum Auth Integrations For The First Sellable Release

The first sellable self-hosted release must support:

- built-in local auth for bootstrap and break-glass administration
- OIDC-based SSO for team mode
- scoped API or service tokens for automation

OIDC is the required external identity integration in v1.

SAML, LDAP, and SCIM may be added later, but they are not required to make the first sellable self-hosted release coherent.

### Auth Usage Rules

The canonical v1 auth stance is:

- local mode may use built-in local auth as the primary sign-in method
- self-hosted team mode should recommend OIDC as the primary team sign-in path
- built-in local auth remains available in team mode for bootstrap and controlled break-glass use
- service and automation access must use explicit scoped tokens rather than human session reuse

This keeps the first release deployable without overcommitting to every enterprise federation protocol on day one.

### Authorization Model

Permissions and policy evaluation remain in-product and scope-aware regardless of deployment mode.

The deployment shape must not collapse project/workspace/tenant permissions into one global superuser assumption.

## Secrets and Credential Ownership

### Canonical Rule

In first-class v1 modes, secrets and external-service credentials remain under customer control.

This includes:

- model/provider credentials
- channel credentials
- source connection credentials
- webhook secrets

### Implications

- Cairn may store encrypted credential metadata and references in-product
- Cairn must not require vendor-hosted secret custody to operate
- later hybrid or managed offerings may introduce alternative custody models, but those must be optional and additive

### Credential Encryption Requirement

Persisted credentials in self-hosted team mode must be encrypted at the application layer before storage.

This requirement applies to:

- provider credentials
- channel credentials
- source connection credentials
- webhook secrets
- any other persisted secret material used by the product

V1 self-hosted team mode must not store these secrets in plaintext in Postgres.

### Canonical Key Management Model

The canonical v1 model is deployment-controlled envelope encryption.

Required properties:

- one configured active key-encryption-key source for the deployment
- key version metadata stored with encrypted secrets
- support for re-encrypting stored secrets under a newer key version
- no Cairn-operated key escrow requirement

The active key-encryption-key source in v1 may be:

- an operator-supplied key from environment, file, or secret mount
- a deployment-local encryption service integration added later

V1 must not require a hosted external key-management dependency operated by Cairn.

### Local Mode Key Behavior

Local mode may use a product-generated local encryption key for convenience if the operator does not supply one.

That local key must still be treated as deployment state, not as a hardcoded product secret.

### Team Mode Key Behavior

Self-hosted team mode requires an explicit operator-controlled encryption key configuration before persisted credential features are considered production-ready.

If no valid key-encryption-key source is configured in team mode, the canonical v1 behavior is fail closed.

That means:

- persisted credential operations must be unavailable
- production bootstrap must remain incomplete until a valid key-encryption-key source is configured

For avoidance of doubt, this applies to:

- credential create/update/import operations
- credential decrypt/use operations that rely on persisted secret material
- secret rotation or re-encryption operations

### Rotation and Recovery Expectations

V1 does not need a full enterprise key-management suite, but it must support:

- key version tracking
- explicit rotation
- rewrap or re-encrypt flow for stored secrets

This keeps the deployment contract compatible with later Vault/KMS/HSM integrations without making them required on day one.

## Plugin Boundary Implications

RFC 007 establishes out-of-process plugins over JSON-RPC/stdio as the canonical v1 plugin model.

RFC 011 makes the deployment consequence explicit:

- in first-class v1 modes, plugins run inside the customer deployment boundary
- plugin execution is supervised by Cairn runtime roles inside that boundary
- remote plugin hosting is not a required product feature in v1

This keeps:

- trust boundaries understandable
- secret flow simple
- operational debugging local to the deployment

### Plugin Deployment Rule

V1 plugins are deployment-local components.

They may be:

- shipped with the product
- installed by the customer
- built in any language that implements the protocol

They are not treated as a remote marketplace fabric in v1.

## Provider and Channel Implications

RFC 009 establishes local and hosted provider backends behind one abstraction.

RFC 011 adds the deployment consequence:

- provider traffic may go to hosted vendor endpoints or local model endpoints
- either way, routing decisions happen inside the customer-controlled Cairn deployment in v1
- channel adapters and source pollers likewise run in the customer-controlled deployment for first-class modes

This means v1 does not assume:

- Cairn-operated outbound relays
- hosted provider proxying
- hosted channel delivery services

## Operator Surface Implications

RFC 010 requires a coherent operator control plane.

Deployment shape makes the minimum settings expectations concrete.

The control plane must let operators inspect and manage:

- deployment role status
- database/storage health
- plugin process health
- provider connection health
- poller/scheduler health
- channel delivery health
- credential metadata and scope
- auth provider status
- key-management configuration status
- secret re-encryption or rotation status where applicable

These surfaces must work in both first-class modes, even if local mode is simpler.

## Migration and Transitional Infrastructure

The deployment model must tolerate transitional infrastructure while the rewrite is in progress.

That may include:

- temporary sidecar or queue substrates
- compatibility APIs
- migration bridges

But those must not redefine the v1 deployment contract.

Specifically:

- the customer should still experience the product as self-hosted-first
- transitional components must not require Cairn-operated hosted infrastructure

## Packaging Rule

V1 packaging should support:

- one local/dev bootstrap path
- one production self-hosted deployment path

The product should not require a matrix of deployment recipes before teams can adopt it.

The first-time operator experience should answer:

- how do I run this locally?
- how do I deploy this for my team?
- what changes between those two?

## First-Class vs Experimental Matrix

### First-Class In V1

- local mode with SQLite
- self-hosted team mode with Postgres
- deployment-local plugins
- customer-controlled secrets
- customer-controlled outbound integrations

### Experimental / Future-Compatible

- hybrid control plane
- managed control plane
- remote plugin hosting
- multi-customer hosted tenancy
- Cairn-operated secret custody

## Non-Goals

For v1, do not optimize for:

- SaaS-first multi-customer hosting
- zero-trust remote plugin fabrics
- fully managed provider gateway operation
- complex split-cloud deployment topologies
- every enterprise deployment permutation

The goal is a deployable, operable self-hosted control plane first.

## Open Questions

1. Should v1 team mode support a one-binary all-in-one production deployment officially, or only as a convenience path?

## Decision

Proceed assuming:

- Cairn v1 is self-hosted-first
- local mode and self-hosted team mode are first-class
- hybrid remains architecture-compatible but not a first-class supported operating model in v1
- managed multi-customer control plane is out of scope for v1
- plugins, secrets, providers, channels, and runtime execution stay inside the customer deployment boundary in first-class modes
- self-hosted team mode requires operator-controlled credential encryption at rest
- OIDC plus built-in local auth and scoped service tokens form the minimum auth stack for the first sellable self-hosted release
- self-hosted team mode without a valid configured key-encryption-key source must fail closed for persisted credential features and cannot complete production bootstrap
