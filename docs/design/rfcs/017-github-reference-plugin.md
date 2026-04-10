# RFC 017: GitHub Reference Plugin

Status: draft
Owner: plugins/integrations lead
Depends on: [RFC 007](./007-plugin-protocol-transport.md), [RFC 011](./011-deployment-shape.md), [RFC 015](./015-plugin-marketplace-and-scoping.md), [RFC 016](./016-sandbox-workspace-primitive.md)

## Summary

The GitHub plugin is **the first plugin built through the marketplace mechanism defined in RFC 015**. It is a reference implementation, not a first-class cairn integration. It exists to prove three things:

1. the marketplace flow works end-to-end — discover, install, enable, use, uninstall
2. a real external system with realistic complexity (webhooks, GitHub App auth, rate limits, rich API, fine-grained credentials) can be made available to teams through the plugin protocol without any changes to cairn-rs core
3. the control plane remains fully in charge — per-project scoping, credential isolation, audit trail, and tool visibility all work when a non-trivial plugin is in play

This RFC specifies the GitHub plugin's capabilities, manifest, credential model, signal ingestion behavior, and the specific set of tools it exposes. It does not define any new cairn-rs infrastructure — every primitive it uses comes from RFCs 007, 015, 016, 019, or 022.

The GitHub plugin is **an external binary**, distributed independently of cairn-app per RFC 015's plugin distribution model. It lives in its own repository (separate release cadence) and is referenced from cairn's bundled catalog as a descriptor. The cairn-app release does not bundle, embed, or ship the plugin binary — operators install it via the marketplace flow which downloads or accepts a local path.

## Resolved Decisions

- **Webhook secret provisioning**: cairn generates a 32-byte secret during the marketplace install wizard, displays it once with copy-to-clipboard, and stores it via `CredentialService`. Operator pastes into GitHub's webhook settings UI. A "regenerate secret" button in the marketplace UI handles loss recovery.
- **Default `draft` for `github.create_pull_request`**: PRs are draft by default; agents must explicitly pass `draft: false` for ready-for-review. Aligns with the draft-PR-as-checkpoint pattern.
- **Plugin binary distribution**: external binary published independently of cairn-app. The cairn catalog descriptor points at the binary's download URL or a local path the operator provides. Not bundled, not embedded, not an `arg0` subcommand.

## Why

### The dogfood test proved the gap is the plugin flow, not the plugin itself

A 2-day dogfood run against `avifenesh/cairn-dogfood` produced zero progress on 18 open issues. Research showed the blocker was the absence of the **marketplace flow**, not the absence of GitHub-specific code. The existing plugin host can run a plugin today; what it could not do was give a team operator a button that says "activate GitHub" and have credentials, webhook subscription, tool exposure, and per-project enablement flow from that one interaction.

This RFC closes the dogfood gap by providing the one plugin the dogfood demo needs, built entirely on the generic mechanism. If the GitHub plugin works, the next five plugins (Slack, Linear, Jira, PagerDuty, Zendesk) are duplicated effort at the plugin level but zero new cairn-rs work.

### GitHub is not special

Nothing in RFCs 015 or 016 mentions GitHub. Nothing in cairn-rs's core crates mentions GitHub. If this RFC disappears and an operator builds their own GitHub plugin from scratch, the result should be functionally equivalent. The plugin in this RFC is "the one cairn ships so teams don't have to write it themselves", not "cairn's GitHub integration".

### Why a single plugin and not several

The simplest credible demo (from `control-plane-requirements.md`) is one team, one repo, one issue, one PR. A single GitHub plugin delivers that. Splitting into `github-issues`, `github-pr`, `github-actions`, etc., fragments the credential story and the operator mental model. The GitHub plugin is one plugin with multiple capabilities, matching how the GitHub App permission model actually works.

## Scope

### In scope for v1

- Plugin manifest declaring:
  - `SignalSource` capability for webhooks (`issue`, `pull_request`, `pull_request_review`, `pull_request_review_comment`, `issue_comment`, `workflow_run`, `check_run`, `check_suite`)
  - `ToolProvider` capability for a defined set of GitHub API tools (listed below)
- GitHub App credential model (App ID + private key + tenant-scoped webhook secret → short-lived installation access tokens per project)
- Credential wizard for App credential entry (no OAuth user-token flow in v1; the plugin uses GitHub App auth exclusively)
- Webhook signature verification (`X-Hub-Signature-256`)
- Webhook delivery deduplication via `X-GitHub-Delivery` ID backed by a **durable webhook-delivery dedup ledger** in the event log (not an in-memory cache); operates at webhook ingress before signal routing, independent of RFC 022's trigger-fire dedup
- Normalized signal emission into cairn-rs's signal router (tagged with plugin ID, carrying the original GitHub payload)
- Tool invocations with automatic installation-token refresh before each call
- Rate-limit awareness (the plugin surfaces `X-RateLimit-*` headers as tool result metadata and emits a `github.rate_limit.warning` signal when approaching the limit — a normal signal subject to per-project `signal_allowlist`)
- Health check at install/verify time: tenant-scoped App JWT auth against `GET /app` via `github.verify_app_auth` (not project-scoped installation tokens, since no project context exists at install time)
- Bundled in the v1 marketplace catalog as a "listed" entry at first boot

### Explicitly out of scope for v1

- GraphQL API support (REST only for v1; GraphQL is a later enhancement)
- GitHub Actions triggering from cairn (the plugin can read workflow runs, not start them)
- GitHub Packages API
- GitHub Enterprise Server (GHES) support (future work; the plugin targets github.com in v1)
- Merge queue management
- GitHub Projects / Milestones API
- Security advisories and Dependabot API
- Organization audit log retrieval

### Delegated to RFC 015 and RFC 016 (not restated here)

- marketplace lifecycle (discover / install / provide credentials / optional verify / enable per project) — no separate "connect" step per the sealed RFC 015 lifecycle
- per-project enablement, tool visibility, and signal_allowlist filtering
- credential storage encryption (via existing `CredentialService`)
- sandbox provisioning for runs that need to build or test code
- webhook routing across projects (handled by `signal_router_impl.rs`)

## Plugin Manifest

```toml
# crates/cairn-plugin-github/manifest.toml
id = "github"
name = "GitHub"
version = "0.1.0"
description = "Ingest GitHub issues, PRs, and events; expose GitHub API tools to agents"
homepage = "https://github.com/avifenesh/cairn-plugin-github"
vendor = "cairn"
category = "IssueTracker"
icon_url = "bundled://github.svg"
execution_class = "sandboxed_process"
command = ["cairn-plugin-github"]

[limits]
max_concurrency = 16
default_timeout_ms = 30000

[permissions]
network_egress = ["api.github.com", "uploads.github.com"]
filesystem_read = []
filesystem_write = []

[[capabilities]]
type = "signal_source"
signals = [
  "github.issue.opened",
  "github.issue.labeled",
  "github.issue.commented",
  "github.pull_request.opened",
  "github.pull_request.labeled",
  "github.pull_request.synchronize",
  "github.pull_request.review_submitted",
  "github.pull_request.review_comment",
  "github.workflow_run.completed",
  "github.check_run.completed",
  "github.rate_limit.warning",
]
# Graph projection is default-on per RFC 015 for all signals above.
# graph_projection = true  (implicit; listed here for reference-implementation clarity)

# Memory ingestion hints per RFC 015 §Signal Knowledge Capture.
# Only signals with high-value text content opt in; deliberately omitting
# pull_request.synchronize to avoid churning the memory index on every push.
[[capabilities.memory_ingest]]
signal_type = "github.issue.opened"
fields = ["payload.issue.title", "payload.issue.body"]
chunk_strategy = "paragraph"
source_label = "github-issue"

[[capabilities.memory_ingest]]
signal_type = "github.issue.labeled"
fields = ["payload.issue.title", "payload.issue.body"]
chunk_strategy = "paragraph"
source_label = "github-issue"

[[capabilities.memory_ingest]]
signal_type = "github.pull_request.opened"
fields = ["payload.pull_request.title", "payload.pull_request.body"]
chunk_strategy = "paragraph"
source_label = "github-pr"

[[capabilities]]
type = "tool_provider"
tools = [
  "github.get_issue",
  "github.list_issues",
  "github.create_issue",
  "github.comment_on_issue",
  "github.close_issue",
  "github.list_issue_comments",
  "github.get_pull_request",
  "github.list_pull_requests",
  "github.create_pull_request",
  "github.comment_on_pull_request",
  "github.get_pull_request_reviews",
  "github.request_pull_request_review",
  "github.get_pull_request_diff",
  "github.merge_pull_request",
  "github.list_commits",
  "github.get_file_contents",
  "github.list_files",
  "github.get_workflow_run",
  "github.get_rate_limit",
]

[marketplace]
required_credentials = [
  { key = "github_app_id",          display_name = "GitHub App ID",          kind = "ApiKey",          scope = "Tenant" },
  { key = "github_app_private_key", display_name = "GitHub App Private Key", kind = "AppInstallation", scope = "Tenant" },
  { key = "github_webhook_secret",  display_name = "Webhook Secret",         kind = "ApiKey",          scope = "Tenant",  generated = true },
  { key = "github_installation_id", display_name = "Installation ID",        kind = "AppInstallation", scope = "Project" },
]
# Note: github_webhook_secret is Tenant-scoped and auto-generated during install.
# It is used ONLY by the webhook intake endpoint for HMAC verification.
# github_installation_id is Project-scoped for API access scoping.
# This makes 4 credentials total: 3 tenant-scoped (app_id, private_key, webhook_secret)
# and 1 project-scoped (installation_id).

# v1 does NOT use OAuth user-token flows. The plugin uses GitHub App auth
# exclusively (App ID + private key → JWT → installation token). No PATs, no
# GitHub-user identity. The credential wizard is a manual-entry flow for the
# App fields. A future RFC may add a GitHub App installation callback flow
# that captures installation metadata via redirect, but v1 collects credentials
# through the wizard only.

[marketplace.health_check]
# Health check runs at install/verify time using tenant-scoped App credentials
# (App ID + private key → JWT). It does NOT use project-scoped installation_id
# because no project context exists yet at install time. The check calls
# GET /app (authenticated with the App JWT) to verify the App is reachable
# and authorized on github.com.
method        = "github.verify_app_auth"
timeout_ms    = 5000
success_criteria = { path = "id", predicate = "greater_than", value = 0 }
```

Everything in the manifest is data consumed by RFC 015's marketplace layer. Nothing in cairn-rs core had to change for this manifest to be valid.

## Credential Model

The plugin uses a **GitHub App** with installation access tokens. Installation tokens are short-lived (1 hour) and scoped to specific repositories and permissions, which matches RFC 015's `CredentialSpec` model with `scope: Project` for the installation ID.

### Flow

1. Operator installs the plugin via the marketplace view (downloads the external binary from the catalog `download_url`)
2. Operator provides credentials through the wizard:
   - `github_app_id` — tenant-scoped, entered once
   - `github_app_private_key` — tenant-scoped, PEM format, stored encrypted via `CredentialService`
   - `github_webhook_secret` — **tenant-scoped**, auto-generated (32-byte random) during install, displayed once with copy-to-clipboard for the operator to paste into GitHub's webhook settings UI; stored via `CredentialService`; a "regenerate secret" button in the marketplace UI handles loss recovery. This credential is used **only** by the `POST /v1/plugins/github/webhook` intake endpoint for HMAC verification — it is never used for API access.
   - `github_installation_id` — project-scoped (per RFC 015 Q2 decision), entered per project during enablement
3. The plugin, on spawn, reads `github_app_id` and `github_app_private_key` from its env; on each tool call it looks up the `github_installation_id` from the run's project scope and mints a fresh installation token scoped to that installation's repositories
4. Installation tokens are cached in-memory per installation ID with a 55-minute TTL (5-minute safety margin before the 60-minute expiry) and refreshed proactively
5. On uninstall, all **four** credentials (`app_id`, `private_key`, `webhook_secret`, and all project-scoped `installation_id` entries) are revoked via `CredentialService::revoke_credential`

### Why installation tokens, not personal access tokens

- Installation tokens are already scoped to specific repositories, reducing blast radius
- They rotate automatically (no long-lived secret on disk)
- They support GitHub App permission granularity (e.g. "read issues, write pull requests, no access to secrets")
- They produce audit events on the GitHub side attributable to the App, not to a person

Personal access tokens (PATs) are not supported in v1. The manifest rejects a credential entry with `kind: ApiKey` alone unless it is accompanied by App fields. This is intentional: PATs are a tempting shortcut that leaks personal identity and lacks repository scoping.

### Per-project credential isolation

Because `github_installation_id` is project-scoped (RFC 015 Q2 decision), two projects in the same tenant can point at two different GitHub Apps installed against different repository sets. The plugin host spawns one plugin process per distinct `(plugin_id, credential_scope_key)` pair, per RFC 015's "Process Instances by Credential Scope" section. A run in project A talks to the process instance that has access to project A's GitHub repos, and a run in project B talks to a separate process instance with access to project B's repos. The repository ID space is isolated by construction.

## Signal Ingestion

### Webhook endpoint

The cairn-app exposes `POST /v1/plugins/:plugin_id/webhook` as a generic plugin webhook intake. For the GitHub plugin, this is `POST /v1/plugins/github/webhook`. The endpoint:

1. Verifies `X-Hub-Signature-256` against the **tenant-scoped** `github_webhook_secret` stored in `CredentialService`. The webhook secret is NOT project-scoped — one webhook URL per GitHub App installation serves all projects in the tenant. Verification happens once per delivery; project-level routing happens after verification.
2. Deduplicates by `X-GitHub-Delivery` ID via a **durable webhook-delivery dedup ledger** in the event log. On each accepted delivery, the intake endpoint appends a `WebhookDeliveryReceived { delivery_id, plugin_id, at }` event; on subsequent deliveries with the same `delivery_id`, the endpoint returns 200 (accepted, no-op) without dispatching to the plugin. On restart, the dedup set is rebuilt from the event log replay. This replaces the earlier 24-hour in-memory cache design, which was crash-unsafe: after a cairn-app restart the cache was empty and webhook retries could produce duplicate signals and duplicate trigger fires. The dedup operates at **webhook ingress, before signal routing** — it is a cairn-app / webhook-intake concern, not a trigger-evaluation concern. RFC 022's trigger-fire dedup (if any) operates at a different layer, after the signal has been routed to project subscribers and trigger evaluation has occurred.
3. Dispatches the raw payload to the plugin process via `PluginHost::dispatch_signal(plugin_id, signal_type, payload)`
4. The plugin normalizes the payload into a cairn signal and emits it via the plugin protocol; the runtime's `signal_router_impl.rs` filters by project enablement (only projects whose enablement declared this signal type in `signal_allowlist` receive it)

Cairn-app does not parse GitHub-specific payloads. It verifies the signature, deduplicates, and hands the raw payload to the plugin. GitHub-specific logic lives entirely inside the plugin process.

### Signal normalization

Every GitHub event becomes a cairn signal with this shape:

```json
{
  "plugin_id": "github",
  "signal_type": "github.issue.labeled",
  "occurred_at_ms": 1775759896876,
  "delivery_id": "12345678-1234-1234-1234-123456789012",
  "source_run_id": null,
  "project_hint": {
    "repo_full_name": "avifenesh/cairn-dogfood",
    "issue_number": 42
  },
  "payload": {
    "action": "labeled",
    "issue":  { "...": "full GitHub issue payload" },
    "label":  { "...": "label that was added" },
    "repository": { "...": "full repository payload" },
    "sender": { "...": "user who triggered" }
  }
}
```

**`source_run_id`** (RFC 022 chain-depth tracking): when the GitHub webhook was triggered by cairn's own agent (e.g. the agent opened a PR via `github.create_pull_request`, GitHub fires `pull_request.opened` back at cairn), the plugin detects this by checking whether the sender matches the cairn GitHub App's bot user (identifiable by the `[bot]` suffix and the App's user ID from credentials). If the sender IS the cairn App, the plugin extracts `source_run_id` from a standard marker embedded in the content (e.g. `<!-- cairn:run_id=run_xyz -->` in the PR description). If extraction fails or the sender is not the cairn App, `source_run_id` is `null` and the trigger evaluator treats the signal as depth 1 (external origin). This is what makes RFC 022's loop-prevention mechanism (`chain_depth`) functional for the reference plugin — without it, cairn cannot distinguish "a human opened a PR" from "cairn opened a PR" and would recurse.

The mutating tools `github.create_pull_request` and `github.comment_on_issue` embed the `<!-- cairn:run_id={run_id} -->` marker automatically in their output so that the loop-back detection works end-to-end.

### Project routing

One GitHub webhook delivery can fan out to many projects (the same repository may be enabled in project A and project B). The signal router uses the enablement table to deliver one signal per receiving project. The plugin instance that emitted the signal has no knowledge of which projects will receive it — the router handles that.

### Rate-limit awareness

The plugin tracks remaining `X-RateLimit-Remaining` across tool calls and webhooks. When the remaining budget drops below 10% of the limit, the plugin emits a `github.rate_limit.warning` signal with the current remaining count and reset time. This signal is a **normal signal subject to the per-project `signal_allowlist`** filter from RFC 015 — it is NOT implicitly delivered to every enabled project. Projects that want rate-limit awareness must include `github.rate_limit.warning` in their `signal_allowlist` (or leave the allowlist as `None` to receive all signals). Operators who want unconditional rate-limit observability regardless of per-project signal scoping should use the marketplace plugin-instance health UI, which surfaces the rate-limit budget from the plugin's health monitor without routing through the signal subsystem.

## Tool Catalog

The plugin exposes 19 tools. The list is deliberately smaller than "every endpoint GitHub offers" — it covers the operations needed for the simplest credible demo plus common follow-ups. Additional tools can be added in minor versions without an RFC change.

### Read-only tools (always safe)

| Tool | Purpose |
|---|---|
| `github.get_issue` | Fetch one issue by number |
| `github.list_issues` | Paginated list, supports state/label/assignee filters |
| `github.list_issue_comments` | Comments on an issue |
| `github.get_pull_request` | Fetch one PR by number |
| `github.list_pull_requests` | Paginated list |
| `github.get_pull_request_reviews` | Reviews on a PR |
| `github.get_pull_request_diff` | Unified diff of a PR |
| `github.list_commits` | Paginated commit list on a branch |
| `github.get_file_contents` | Raw file at a revision |
| `github.list_files` | Directory listing at a revision |
| `github.get_workflow_run` | CI workflow status |
| `github.get_rate_limit` | Current rate-limit budget |

### Mutating tools (policy-gated)

| Tool | Purpose | Notes |
|---|---|---|
| `github.create_issue` | Open a new issue | Routed through the decision layer (RFC 019); default require_approval in non-automated runs |
| `github.comment_on_issue` | Post a comment | Auto-approve by default |
| `github.close_issue` | Close an issue | Require approval by default |
| `github.create_pull_request` | Open a PR (draft by default) | Draft-PR-as-checkpoint pattern |
| `github.comment_on_pull_request` | Post a PR comment | Auto-approve by default |
| `github.request_pull_request_review` | Request a review | Auto-approve by default |
| `github.merge_pull_request` | Merge a PR | **Always require approval** — hardcoded, cannot be auto-approved even with a learned rule |

**The merge tool is the only permanently gated tool**. Everything else can be auto-approved via RFC 019's learned-rules mechanism after the first human approval. Merge always asks.

### Tool invocation semantics

Each tool call:

1. Looks up the run's project; confirms the GitHub plugin is enabled in that project (enforced by `VisibilityContext` before the LLM sees the tool name)
2. Resolves the installation ID for the project's credential scope
3. Mints or reuses a cached installation token
4. Calls the GitHub API with appropriate headers
5. Returns a normalized result to the agent; failures surface as `ToolInvocationFailed` events with the HTTP status and rate-limit metadata attached

Tool results are bounded by the orchestrator's `tool_output_token_limit` (defined in RFC 018) before being appended to the agent context. A giant diff does not blow up the context window.

## Sandbox Integration

The GitHub plugin itself does not need a sandbox — it is a stdio JSON-RPC process that makes API calls. It runs as a regular plugin process under the existing plugin host.

However, **runs that use the GitHub plugin** may need sandboxes. A run that clones a repository to inspect it, runs tests, and pushes a branch needs a `RunSandbox` (RFC 016). The plugin does not provision sandboxes — the run does, using RFC 016's `SandboxService`. The plugin's `github.get_file_contents` is a no-sandbox API call. If an agent wants to work on the file system, it uses the orchestrator's file tools against the sandbox the runtime provisioned for the run.

This separation is important: the GitHub plugin is about **the GitHub API surface**, not about **where the agent works on code**. Conflating them would force every agent using the plugin to go through the sandbox flow even for a simple issue-comment task.

## Marketplace Bundle Entry

The v1 bundled catalog (`crates/cairn-plugin-catalog/catalog.toml`) contains:

```toml
[[plugin]]
id           = "github"
manifest     = "bundled://plugins/github/manifest.toml"
# The plugin binary is NOT shipped inside cairn-app. The descriptor provides a
# download URL for the independently published binary. Operators can also
# override with a local path during the install wizard. This is the
# descriptor-only bundled catalog model from sealed RFC 015 §Plugin
# Distribution Model — no bundled binary, no embedded binary, no arg0.
download_url = "https://github.com/avifenesh/cairn-plugin-github/releases/latest/download/cairn-plugin-github-{arch}"
category     = "IssueTracker"
```

At first boot, the catalog loader emits a `PluginListed` event for the GitHub entry. The plugin is **listed** but **not installed**. An operator who never enables GitHub never sees it in any prompt, never has its process running, and never has its signal ingestion endpoint active. This is the default.

An operator who wants to use it visits Settings → Plugins, clicks Install (which downloads the binary from the `download_url` or accepts a local path), is walked through the credential wizard, then enables it per project.

## Simplest Credible Demo (the dogfood path)

This RFC plus RFCs 015 and 016 together enable the following end-to-end flow:

1. Operator boots cairn-app, sees GitHub listed in the marketplace
2. Operator installs the plugin (downloads the binary from the catalog `download_url` or accepts a local path)
3. Operator provides GitHub App credentials through the wizard: App ID, private key, webhook secret (auto-generated, operator pastes into GitHub's webhook settings), installation ID for the target repo
4. Operator optionally verifies credentials via ephemeral `POST /v1/plugins/github/verify` (uses tenant-scoped App JWT to call `GET /app`; does not commit a persistent "Connected" state)
5. Operator enables the plugin for their project (one click); since the plugin declares `SignalSource`, the tenant-default process instance is eagerly spawned per RFC 015's spawn policy
6. Operator adds the target repo (`avifenesh/cairn-dogfood`) to the project's RepoStore allowlist via `POST /v1/projects/:project/repos` (RFC 016) so sandboxes can provision against it
7. Operator creates an RFC 022 Trigger matching `github.issue.labeled` with condition `labels[].name contains "cairn-ready"`, bound to a RunTemplate specifying the agent's mode, system prompt, and sandbox policy
8. Operator configures a GitHub webhook on their repo pointing at `POST /v1/plugins/github/webhook` (documented as part of the plugin install flow)
9. A team member labels an issue with `cairn-ready`
10. GitHub delivers the webhook → cairn verifies HMAC signature against the tenant-scoped webhook secret → deduplicates via the durable webhook-delivery dedup ledger (`WebhookDeliveryReceived` event) → hands raw payload to the eagerly-spawned plugin process → plugin normalizes (including `source_run_id` detection for loop prevention) → signal router filters by project `signal_allowlist` and routes to enabled projects → the RFC 022 Trigger evaluator matches the signal → a run is created from the RunTemplate
11. The run provisions a sandbox (RFC 016) against the allowlisted repo, the agent uses the GitHub plugin tools to read the issue body, writes code in the sandbox, runs tests, opens a draft PR (with `<!-- cairn:run_id=... -->` marker for loop-back detection), posts comments as review feedback comes in, marks ready for review
12. A human merges the PR (RFC 017's hardcoded approval gate on `github.merge_pull_request` means the agent asks first and the operator clicks approve)
13. Every step is in the event log; every cost is attributed to the run; the operator can see the full trace in the dashboard

**Every step above uses primitives defined in other RFCs.** RFC 017's only contribution is the plugin that serves GitHub specifically.

## Non-Goals

For v1, explicitly out of scope:

- GraphQL API support — REST only
- GitHub Enterprise Server support
- GitHub Actions triggering (`workflow_dispatch`)
- Security advisories and Dependabot
- GitHub Projects / Milestones
- Merge queue management
- Organization audit log retrieval
- Scheduled polling as a fallback for webhooks (the plugin expects webhooks to work; sites behind NAT must configure a public webhook URL or they cannot use the plugin in v1)
- Authenticating as a GitHub user (OAuth user tokens) — Apps only
- SaaS hosting of the GitHub plugin — it runs in the customer deployment

## Open Questions

1. **Resolved**: Signal-to-run binding is handled by RFC 022's **Trigger** entity. A project creates a Trigger with `signal_filter` matching `github.*` event types and a condition (e.g. `labels[].name contains "cairn-ready"`), bound to a RunTemplate specifying agent mode, system prompt, and sandbox policy. The plugin does not create runs; the project's Trigger + RunTemplate configuration does. No parallel "signal subscription table" is needed. (No further discussion needed; resolved by RFC 022.)

2. **Resolved**: Webhook secret provisioning. The marketplace flow generates a 32-byte random `github_webhook_secret` during the install wizard, displays it once with copy-to-clipboard, and stores it via `CredentialService` as a **tenant-scoped** credential (scope = Tenant). The operator pastes the secret into GitHub's webhook settings UI. A "regenerate secret" button in the marketplace view handles loss recovery. The webhook secret is distinct from the per-project `github_installation_id` — it is used ONLY by the `POST /v1/plugins/github/webhook` intake endpoint for HMAC verification, never for API access. (No further discussion needed; baked into the manifest's `required_credentials`.)

3. **NEEDS DISCUSSION: Can a single GitHub installation span multiple cairn projects?** Yes by default (installation is per-repository, and one repo may be enabled in several cairn projects). Is that desired? Proposal: yes, but with a warning during per-project enablement that "this repository is already enabled in projects X and Y" so operators are aware of signal fan-out.

4. **Resolved**: Draft PRs by default. `github.create_pull_request` defaults to `draft: true` unless the agent explicitly passes `draft: false`. Aligns with the "draft PR as checkpoint" pattern. (No further discussion needed; baked into the RFC.)

5. **Resolved**: Rate-limit as a normal signal subject to `signal_allowlist`. `github.rate_limit.warning` is NOT implicitly delivered to every enabled project — it follows the same per-project `signal_allowlist` filtering as every other signal (per sealed RFC 015). Operators who want unconditional rate-limit observability should use the marketplace plugin-instance health UI, which surfaces the budget from the plugin's health monitor without going through the signal router. (No further discussion needed.)

6. **Resolved**: Plugin binary distribution is **external binary, published independently of cairn-app**, per sealed RFC 015 §Plugin Distribution Model. The cairn catalog holds a descriptor + `download_url` pointing at the plugin's independent release. Not bundled, not embedded, not an `arg0` subcommand. RFC 015 line 134 (sealed) explicitly forbids `arg0`: "The boundary is strict: no shared libraries, no shared memory, no shared code, no `arg0` subcommand pattern." This Open Question is closed by the sealed RFC and cannot be reopened without amending RFC 015. (No further discussion needed.)

## Decision

Proceed assuming:

- the GitHub plugin is a **reference implementation** of the RFC 015 marketplace mechanism, not a first-class cairn integration
- the plugin is an **external binary**, distributed independently of cairn-app per sealed RFC 015's Plugin Distribution Model; it is **not bundled, not embedded, and not an `arg0` subcommand**; the cairn catalog holds a descriptor + `download_url`
- the plugin uses GitHub App installation tokens, not personal access tokens; no OAuth user-token flow in v1
- the credential model has **4 credentials**: `github_app_id` (tenant), `github_app_private_key` (tenant), `github_webhook_secret` (tenant, auto-generated), and `github_installation_id` (project-scoped per enablement)
- the marketplace lifecycle follows sealed RFC 015: discover → install → provide credentials → optional ephemeral verify → enable per project; there is **no `connect` step** and **no `PluginConnected` event**
- the health check at install/verify time uses tenant-scoped App JWT auth (`GET /app`), not project-scoped installation tokens
- webhook endpoint is `POST /v1/plugins/github/webhook`, HMAC-verified against the **tenant-scoped** webhook secret, deduplicated via a **durable webhook-delivery dedup ledger** (`WebhookDeliveryReceived` events in the event log, rebuilt on restart), and handed as raw payload to the plugin process
- signal normalization includes `source_run_id` for RFC 022 chain-depth tracking; mutating tools embed `<!-- cairn:run_id=... -->` markers for loop-back detection
- `github.rate_limit.warning` is a normal signal subject to per-project `signal_allowlist`, not implicitly delivered
- the manifest declares `memory_ingest` hints on `SignalSource` for `issue.opened`, `issue.labeled`, and `pull_request.opened` (title + body), deliberately omitting `pull_request.synchronize` to avoid commit-churn
- the plugin exposes 19 tools (12 read-only, 7 mutating), with `github.merge_pull_request` as the only permanently approval-gated tool
- per-project credential isolation uses RFC 015 Q2's "process instance per credential scope" model, so two projects with different installations run two plugin process instances; the tenant-default instance is eagerly spawned at `EnablePluginForProject` because the plugin declares `SignalSource` (per sealed RFC 015 spawn policy)
- the GitHub plugin itself does not provision sandboxes; runs that need them use RFC 016's `SandboxService` independently
- signal-to-run binding is handled by RFC 022's Trigger entity; the plugin does not create runs
- the plugin is listed in the v1 catalog but not installed by default
- open questions listed above must be resolved before implementation begins

## Integration Tests (Compliance Proof)

The RFC is considered implemented when the following integration tests pass (many of these overlap with RFC 015's tests, intentionally — this RFC is the end-to-end proof):

1. **Catalog listing**: boot cairn-app, call `GET /v1/plugins/catalog`, confirm GitHub appears with `state: listed`, `category: IssueTracker`, `download_url` present (no `bundled://` URI)
2. **Install + credential wizard**: `POST /v1/plugins/github/install` → downloads binary from `download_url` → provide credentials via wizard (app_id, private_key, webhook_secret auto-generated, installation_id per project) → `PluginInstalled` event → (optional) `PluginCredentialsVerified { outcome: Ok }` via ephemeral `POST /v1/plugins/github/verify` using tenant-scoped App JWT → health check confirms App reachable
3. **Per-project enablement**: enable for project P1 with no allowlist → `PluginInstanceReady { reason: EagerSignalSource }` emitted (SignalSource-declaring plugin, per sealed RFC 015 spawn policy) → the 19 tools appear in a run's prompt for project P1 but not for project P2
4. **Tool allowlist**: enable for project P3 with `tool_allowlist: ["github.get_issue", "github.list_issues"]` → only 2 tools visible
5. **Webhook signature verification**: `POST /v1/plugins/github/webhook` with an invalid HMAC against the tenant-scoped webhook secret → 401; with valid signature → 202 accepted and signal emitted
6. **Webhook deduplication (durable)**: send the same `X-GitHub-Delivery` ID twice → signal is emitted once (deduplication via the durable `WebhookDeliveryReceived` event log, NOT an in-memory cache); restart cairn-app between the two deliveries → still deduplicated (the event log is replayed on startup to rebuild the dedup set)
7. **Per-project signal routing + signal_allowlist**: one webhook delivery for a repo enabled in projects P1 and P3 → two signals emitted, one per project; project P2 (GitHub not enabled) sees nothing. If P3 has `signal_allowlist: ["github.issue.opened"]` and the delivery is `github.issue.labeled`, P3 does not receive it.
8. **Installation token caching**: two successive tool calls for the same installation ID within 55 minutes share the same minted token; a call after token refresh gets a new one
9. **Per-project credential isolation**: project A uses installation 100 with access to repo `org/A`; project B uses installation 200 with access to repo `org/B`; a tool call from project A to fetch an issue from `org/B` fails with "no access"
10. **Merge approval gate**: an agent calls `github.merge_pull_request` → the decision layer (RFC 019) always creates an approval request; no learned rule can auto-approve it
11. **Draft PR default**: `github.create_pull_request` without explicit `draft` flag creates a draft PR; the PR description contains `<!-- cairn:run_id=run_xyz -->` marker for loop-back detection
12. **Rate-limit warning signal follows signal_allowlist**: mock GitHub responses with `X-RateLimit-Remaining: 50` below 10% of limit → `github.rate_limit.warning` signal is emitted; if a project's `signal_allowlist` does not include `github.rate_limit.warning`, the signal does NOT reach that project's subscribers (it is a normal signal, not implicitly delivered)
13. **Uninstall revokes all 4 credentials**: `DELETE /v1/plugins/github` → all four credentials (`app_id`, `private_key`, `webhook_secret`, `installation_id` per project) are revoked in `CredentialService`; next webhook delivery returns 404 (plugin not installed)
14. **End-to-end dogfood path**: open an issue, label it `cairn-ready`, confirm the full flow through to a draft PR created via the plugin (integration test runs against a mock GitHub server). Flow covers: webhook verification → durable dedup → signal normalization with `source_run_id: null` (external origin) → RFC 022 Trigger match → RunTemplate instantiation → sandbox provisioning (RFC 016) against allowlisted repo → agent reads issue body, writes code, opens draft PR with `<!-- cairn:run_id=... -->` marker → PR is created.
15. **Loop-prevention (source_run_id)**: cairn's agent opens a PR via `github.create_pull_request` → GitHub fires `pull_request.opened` webhook → plugin detects the sender is the cairn App's bot user → extracts `source_run_id` from the `<!-- cairn:run_id=... -->` marker → signal envelope carries `source_run_id: "run_xyz"` → RFC 022 trigger evaluator increments `chain_depth` and applies the project's depth limit → if depth limit reached, trigger does NOT fire (prevents recursive loops)
16. **Memory-ingest for GitHub signals**: a `github.issue.opened` signal with `memory_ingest` hint flows through the signal router → `cairn-memory::IngestService` ingests the issue title and body with `source_id = plugin:github:signal:{delivery_id}` → `SignalIngestedToMemory` event emitted → a subsequent `memory_search` call for the issue content returns results from the ingested chunks
