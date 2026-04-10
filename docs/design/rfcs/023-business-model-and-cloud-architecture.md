# RFC 023: Business Model, Licensing, and Cloud Architecture

Status: draft
Owner: product / commercial lead
Depends on: [RFC 001](./001-product-boundary.md), [RFC 011](./011-deployment-shape.md), [RFC 014](./014-commercial-packaging-and-entitlements.md), [RFC 021](./021-control-plane-protocols.md)

## Summary

This RFC resolves the three open commercial questions that RFC 014 deferred:

1. **License**: BSL 1.1 (Business Source License), converting to Apache 2.0 after 4 years
2. **Revenue model**: self-hosted license keys (primary at launch) + hosted control plane with consumption pricing (primary at maturity)
3. **Cloud architecture**: control plane / data plane split where the hosted service NEVER touches customer credentials, repos, plugins, sandboxes, or LLM keys

The product is a self-hosted control plane for teams using AI. The business is selling operational reliability — either as license keys for teams who self-host, or as a hosted control plane surface for teams who don't want to operate Postgres, TLS, auth, and upgrades themselves.

## Resolved Decisions

- **License**: BSL 1.1. Additional Use Grant: internal production use, development, testing. Restriction: cannot offer cairn as a managed service to third parties. Change Date: 4 years from each release. Change License: Apache 2.0.
- **Cloud architecture**: mandatory control plane / data plane split. The hosted service runs cairn-server (event log, projections, decision cache, triggers, dashboard, API, OTLP). Customers run cairn-runner on their own infrastructure (sandboxes, plugin host, repo cache, credential store, tool execution). The split interface is the SQ/EQ protocol from RFC 021.
- **Security boundary**: hard-disabled, not feature-flagged. Hosted cairn-server does not instantiate SandboxService, RepoCloneCache, ProjectRepoAccessService, StdioPluginHost, CredentialService, or any tool execution path. Fails closed if any sensitive route is enabled in hosted mode.

## Why BSL 1.1

### Why not Apache 2.0

Apache 2.0 allows any company to take the cairn-rs source, host it as a managed service, and compete with zero contribution back. The 2023-2024 license migration wave (HashiCorp → BSL, Redis → proprietary, InfluxDB → BSL, Sentry → BSL) demonstrates that Apache 2.0 cannot sustain VC-backed or bootstrapped development of infrastructure software when a better-capitalized competitor can offer the same product as a service.

Cairn is a control plane — the entire product value IS the integrated software. Unlike a library or SDK that benefits from unrestricted embedding, a control plane is deployed as a system. Protecting the right to commercially operate that system is the minimum viable commercial defense.

Starting with BSL is significantly less damaging than switching from Apache later. HashiCorp's OpenTofu fork happened because they switched AFTER building a massive Apache community. Cairn has zero existing community — choosing BSL now creates no backlash.

### Why not fully closed

Self-hosted infrastructure buyers require source visibility. Teams deploying a control plane near their credentials, repos, and governance logic need to inspect the code. BSL provides full source visibility while protecting commercial rights. Closed source would eliminate cairn from evaluation by the majority of the self-hosted infrastructure market.

### What BSL allows

- Any company can read, clone, modify, and self-host cairn for their own internal use
- Development, testing, staging, and production use are all permitted
- Building products that USE cairn as infrastructure is permitted
- Contributing patches, filing issues, and participating in the community is welcome

### What BSL restricts

- Offering cairn as a hosted/managed service to third parties
- Reselling cairn as a product to other companies
- Building "Cairn Cloud" as a competitive service

### Conversion to Apache 2.0

After 4 years from each release, that release's code converts to Apache 2.0 with no restrictions. This means:

- Code released in 2026 becomes fully open source in 2030
- Each subsequent release gets its own 4-year clock
- The community can always see the source; they gain full freedom after the conversion window

## Revenue Model

### Tier Structure

| Tier | Target | Price | Deployment |
|---|---|---|---|
| **Local / Eval** | Solo developers, evaluation | Free | Self-hosted monolith, BSL |
| **Team Self-Hosted** | Teams (5-50 operators) | $29/operator/month | Self-hosted, license key |
| **Enterprise Self-Hosted (monthly)** | Growing teams (10-50 operators) | $99/operator/month, no minimum | Self-hosted, license key |
| **Enterprise Self-Hosted (annual)** | Large orgs (50+ operators) | $50K+/year with dedicated support + SLA | Self-hosted, license key, compliance |
| **Cloud** | Teams who don't want to operate infra | Consumption (events → est. runs/month) | Hosted control plane + BYO runner (Apache 2.0) |

### Free tier (Local / Eval)

The full runtime with all 8 Phase 2 RFC features, running as a single-node monolith with InMemory or SQLite store. Sufficient for:

- Solo developer running agents locally
- Team evaluating cairn before committing
- Development and testing environments
- Open source contributors

Limits: single tenant, up to 3 projects, community support only.

### Team Self-Hosted

Annual or monthly license key that unlocks:

- Postgres backend (production durability per RFC 020)
- Unlimited projects and workspaces
- Multi-operator access with basic RBAC (existing WorkspaceRole: Owner, Admin, Member, Viewer)
- Full marketplace + triggers + sandboxes (RFCs 015, 016, 022)
- Decision layer with learned rules (RFC 019)
- Plan/Execute/Direct agent modes (RFC 018)
- Email support with 48h response

### Enterprise Self-Hosted

Everything in Team, plus:

- SSO: SAML, OIDC, SCIM provisioning
- Advanced RBAC: custom roles beyond the four built-in roles
- Audit log export: S3, GCS, Azure Blob with configurable retention
- Fleet dashboards: cross-workspace agent health, cost roll-up, stalled/escalated views
- Advanced evals: A/B experiment management, bandit optimization
- Compliance policy packs: SOC 2, HIPAA, FedRAMP configuration templates
- Air-gapped deployment support with offline license activation
- Priority support: dedicated Slack channel, 4h response SLA, quarterly review calls

### Cloud (Cairn Cloud)

Hosted control plane with consumption-based pricing:

- Base: free tier (10,000 run orchestration events/month, 3 projects, 2 operators)
- Run events: $X per 1,000 events beyond free tier
- Decision evaluations: $X per 1,000
- OTLP spans: $X per 1,000
- Event log retention: $X per GB/month beyond 30 days

Operators connect their own LLM providers — cairn-server never handles provider API keys. The provider-agnostic architecture (RFC 009) is the selling point: cairn routes to the customer's providers, it doesn't intermediate them.

Cloud includes all Team features. Enterprise Cloud adds SSO, advanced RBAC, audit export, and SLA support at enterprise pricing.

## Cloud Architecture: Control Plane / Data Plane Split

### The problem

The current cairn-app binary runs everything in one process: the API server, the event log, the dashboard, the decision layer, the trigger evaluator, the sandbox provisioner, the plugin host, the credential store, and the tool executor. For self-hosted deployment this is a feature (single binary, simple ops). For cloud deployment it's a security disaster — hosting this binary means running untrusted customer code on shared infrastructure with access to customer credentials.

### The solution: cairn-server + cairn-runner

Split the monolith into two binaries:

**cairn-server** (hosted by Cairn Cloud):
- Event log + Postgres
- Sync projections (run state, task state, approval state, decision cache, trigger state, fire ledger)
- HTTP API + dashboard + SSE stream
- Decision layer evaluation (RFC 019)
- Trigger evaluator (RFC 022) — matches signals to triggers, creates run records
- Signal router (RFC 015) — routes signals to project subscribers
- OTLP exporter (RFC 021)
- SQ/EQ protocol endpoints (RFC 021)
- A2A Agent Card + task submission (RFC 021)
- Marketplace state (plugin catalog, enablement records) — but NOT plugin process management

**cairn-runner** (runs on customer infrastructure):
- SandboxService + OverlayProvider + ReflinkProvider (RFC 016)
- RepoCloneCache + ProjectRepoAccessService (RFC 016)
- StdioPluginHost — spawns and manages plugin processes (RFC 007 / 015)
- CredentialService — stores and resolves credentials (RFC 015)
- Tool execution — all built-in tools run here
- Provider calls — LLM generation requests originate from the runner, using credentials the runner holds
- Connects to cairn-server via SQ/EQ protocol (RFC 021)

### The split interface: SQ/EQ

The cairn-runner connects to cairn-server using the SQ/EQ protocol defined in RFC 021:

- `POST /v1/sqeq/initialize` — runner authenticates and binds to a project scope
- `POST /v1/sqeq/submit` — runner reports tool results, sandbox events, checkpoint data
- `GET /v1/sqeq/events` — runner receives orchestration commands, decision outcomes, trigger fires

This means the cloud architecture was already designed in Phase 2 — RFC 021's SQ/EQ protocol IS the server/runner communication layer.

### Security boundary (non-negotiable)

The hosted cairn-server:

- Does NOT instantiate `SandboxService`, `OverlayProvider`, or `ReflinkProvider`
- Does NOT instantiate `RepoCloneCache` or `ProjectRepoAccessService`
- Does NOT instantiate `StdioPluginHost` or spawn any plugin processes
- Does NOT store or resolve credentials via `CredentialService`
- Does NOT execute any tools (built-in or plugin)
- Does NOT make LLM provider API calls
- Does NOT clone git repositories or access customer source code

These restrictions are enforced at the product boundary (the cairn-server binary simply does not compile these crates in), not via feature flags or runtime checks. If any sensitive capability is accidentally enabled, the server fails closed at startup.

### Multi-tenant isolation in cairn-server

Each tenant's data is isolated in Postgres via `ProjectKey` scoping (RFC 008). The existing query-filter-by-scope pattern applies identically in the hosted context. SSE streams are filtered by the SQ/EQ session's bound scope (RFC 021). No tenant can observe another tenant's events, decisions, or run state.

## Development Roadmap

### v1.0 GA — Week 0

Ship the self-hosted monolith with all 8 Phase 2 RFCs:

- BSL 1.1 LICENSE file in repository
- Full runtime: marketplace, sandbox, decision layer, triggers, agent loop modes, protocols, recovery
- InMemory + SQLite + Postgres store backends
- Operator dashboard (30+ views)
- Usage telemetry instrumented from day one (run counts, sandbox resource usage, plugin process lifecycle, decision cache hit rates, trigger fire frequency)
- 81-check smoke test + 3279 workspace tests
- Documentation: README, CLAUDE.md, 8 sealed RFCs, implementation plan

### v1.1 Hosted Operator Surface — Week 6

Ship a control-plane-only hosted product:

- cairn-server binary: cairn-app with SandboxService, RepoCloneCache, StdioPluginHost, CredentialService, and tool execution paths **hard-removed** (not compiled in)
- Multi-tenant Postgres with per-ProjectKey isolation
- Multi-tenant auth (bearer token per tenant, scoped sessions)
- Dashboard, decisions, approvals, event stream, trigger management, plan review — all operational surfaces
- Customers self-host the v1.0 monolith for execution; use Cairn Cloud for the operator surface
- Marketed as "Hosted Operator Surface" (not "Cairn Cloud") to set correct expectations

What this proves: demand for hosted operational experience. If teams sign up for hosted dashboards while self-hosting execution, cloud is validated.

### v1.1 Runner Alpha — Week 6 (parallel)

Internal prototype of cairn-runner:

- Extracts cairn-workspace + cairn-tools + cairn-plugin-proto into a standalone binary
- Connects to cairn-server via existing HTTP API (not yet SQ/EQ)
- Runner registration + heartbeat
- Basic sandbox provisioning + tool execution
- Internal dogfood only — not customer-facing

### v1.2 Cloud GA — Week 10-14

Production BYO-runner architecture:

- cairn-runner binary published as a downloadable artifact
- Runner connects to cairn-server via SQ/EQ protocol
- Runner registration, lease/heartbeat, remote execution, event forwarding
- Credential handoff: runner holds all secrets, server holds none
- Recovery semantics across the network boundary
- Consumption-based pricing activated
- "Cairn Cloud" branding (replaces "Hosted Operator Surface")

### v1.3+ — Post Cloud GA

- Self-hosted enterprise features (SSO, advanced RBAC, audit export, fleet)
- Cloud enterprise tier
- Plugin marketplace revenue share
- Managed runner option (for teams that want cairn to operate execution too — requires additional security architecture)

## Open Questions

1. **Resolved**: Billing unit is **orchestration events**, presented to buyers as **estimated runs/month**. Events are the honest metric — they track actual control-plane work done (not idle wait time on slow providers, which run-minutes would misprice). Buyer-facing pricing shows "Your plan includes ~N runs/month" with an average-events-per-run multiplier in the calculator. Invoices show event counts for transparency. Average run is ~50-100 events, so the translation is straightforward.

2. **Resolved**: Free tier limits are based on **compute and network cost**, translated to users as a **run count cap** (e.g. N runs/month). Project count is NOT the limiter — a single project can be enormous. The cap reflects actual infrastructure cost (CPU time, memory, disk, network egress) but is presented to operators as a simple "runs per month" number they can reason about. The exact threshold is set based on the cost of hosting one free-tier tenant on the cloud infrastructure.

3. **Resolved**: **cairn-runner is Apache 2.0; cairn-server is BSL 1.1** (unanimous). The runner runs on customer infrastructure touching their credentials and repos — Apache 2.0 eliminates legal friction for customer security teams. The commercial protection cairn needs is around the hosted server product, not the execution layer. The runner has no standalone value without the server. Two licenses is a one-time docs cost; the trust signal on the sensitive component customers deploy is permanent. This matches the Temporal model (proprietary server, MIT SDK/workers).

4. **Resolved**: Cloud free tier is **permanent** with hard limits (unanimous). Time-limited cliffs kill conversion — enterprise procurement alone takes 90+ days. Freeloading is managed by the limits (capped runs/month, 2 operators), not by a clock. Cost per free-tier tenant is minimal (a few Postgres rows). Every successful infra product (GitHub, Vercel, Supabase, Neon) uses permanent free tiers.

5. **Resolved**: Two-tier enterprise structure (unanimous). **Entry**: $99/operator/month with no minimum ACV — enables bottom-up adoption via credit card. An eng manager starts with 5 operators at $495/month, no procurement needed. **Enterprise annual**: $50K+/year annual contract with dedicated support, SLA commitments, compliance assistance, and procurement-grade terms — offered as an upsell when teams grow past ~50 operators. The $50K floor emerges from growth, not from a gate. V1 motion is product-led growth; enterprise sales engages when usage proves demand.

## Decision

Proceed assuming:

- BSL 1.1 license from day one, converting to Apache 2.0 after 4 years per release
- Four-tier revenue model: Free (local/eval), Team Self-Hosted ($29/op/mo), Enterprise Self-Hosted ($99/op/mo or $50K+/yr), Cloud (consumption)
- Cloud architecture: cairn-server (hosted, control plane only) + cairn-runner (customer-side, all execution)
- SQ/EQ protocol (RFC 021) is the server/runner communication interface
- Security boundary: cairn-server never compiles sandbox/credential/plugin/tool execution crates
- v1.0 GA is the self-hosted monolith; v1.1 is hosted operator surface + runner alpha; v1.2 is cloud GA with BYO runner
- Enterprise features (SSO, RBAC, audit, fleet) are entitlement-gated in both self-hosted and cloud tiers
- Open questions above must be resolved before cloud pricing is finalized
