# RFC 015: Plugin Marketplace and Per-Project Scoping

Status: draft
Owner: plugin/runtime lead
Depends on: [RFC 001](./001-product-boundary.md), [RFC 007](./007-plugin-protocol-transport.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 010](./010-operator-control-plane-ia.md), [RFC 011](./011-deployment-shape.md)

## Summary

Cairn already has a working plugin host (RFC 007): JSON-RPC over stdio, `PluginManifest`, `PluginCapability`, out-of-process supervision, MCP bridge. What it is missing is the layer above: a **marketplace** that lets a team discover, install, and enable plugins per project, plus a **scoping model** so that agents in a project see only the tools, signals, and channels from plugins that project has explicitly enabled.

This RFC does not rebuild the plugin host. It adds:

- plugin registry sources (bundled catalog, remote registry, local file, URL)
- a **one-button activation** flow for plugins (install, credential wizard, manifest validation, optional ephemeral verification, enable for project) wired to existing `CredentialService`
- **per-project enable/disable** so a plugin is known to the host but only visible inside projects that opted in
- **per-run tool visibility scoping** that extends the existing `ToolTier` model with a "visible from plugin X" dimension
- a marketplace operator view (RFC 010) showing available, installed, and per-project enabled plugins
- **the first reference plugin is GitHub** (specified in RFC 017) — not as a first-class integration but as proof that a real team can one-button activate an external system

The product promise is not "cairn runs your GitHub issues." It is "a team can extend cairn with any external system via a plugin, and agents only see what that team has turned on."

## Why

### The real v1 gap

RFC 007 gave cairn a plugin protocol. The current product can accept a `PluginManifest` via `POST /v1/plugins`, spawn the plugin process, and bridge its tools/signals/channels through the plugin host. But:

1. **There is no discovery.** An operator must construct a manifest JSON and POST it. There is no catalog, no browse, no "install from this URL", no "one click".
2. **There is no per-project visibility gate.** Once a plugin is registered, its tools are visible to every agent in every project in the tenant. An agent running a customer-support workflow sees tools from a GitHub plugin it will never use.
3. **Credential provisioning is manual.** There is no wizard that knows "this plugin needs a GitHub App installation ID + private key" and guides the operator through it. The operator must know the plugin's credential shape, create entries in the credential store by hand, and set env vars for the plugin process.
4. **Health and lifecycle are opaque.** `plugin_health_monitor.rs` exists but there is no operator surface that says "GitHub plugin is healthy, webhook connection is live, last event received 4 minutes ago" from a marketplace-style view.

### Why this matters for the product positioning

Cairn is a control plane for **teams using AI**, not a coding agent. That means:

- teams differ — support, R&D, research, operations, content, customer success
- each team uses a different set of external systems — Jira, Linear, Zendesk, Intercom, Salesforce, Slack, Drive, Notion, Confluence, GitHub, GitLab, PagerDuty, DataDog
- agents should not be aware of systems that are irrelevant to their team's work
- operators must be able to say "this project uses Slack and Jira, that's it" without writing manifests or code

The plugin marketplace is how cairn becomes credibly useful for non-engineering teams. Until a non-technical operator can enable a plugin for their project in one click, cairn is an R&D toy.

### Why not hardcode integrations

Every integration cairn tries to own as first-class code becomes a maintenance obligation, a provider-lock surface, and a wall between cairn and teams that use something cairn has not integrated yet. The plugin mechanism is the only answer that scales to "every team, any system". Hardcoding GitHub would set a precedent that the next team with a Zendesk workflow expects cairn to own Zendesk, then Salesforce, then PagerDuty. That path ends in a vendor list, not a control plane.

## Product Goals

The marketplace layer must let a team operator:

1. **Discover** plugins: browse a catalog, search by system name, filter by capability (tool / signal / channel). (Eval-scorer filtering is reserved for a future RFC — see §Non-Goals.)
2. **Install and credential** a plugin in one interaction: click → credential wizard → optional credential verification → ready to enable. There is no separate "connect" step; process instances spawn on demand when the plugin is enabled for a project (see §"Plugin Lifecycle — Two Separate State Machines").
3. **Enable** a plugin on specific projects; a plugin installed at tenant level is invisible to projects that did not enable it. Enablement carries both a `tool_allowlist` and a `signal_allowlist`, each independently scoping what the project's runs may see from the plugin.
4. **Configure** a plugin's declared runtime settings (timeout, concurrency, declared-but-not-enforced network egress hints) without editing files. In v1 network egress hints are stored and displayed but not enforced — enforcement is deferred to a future RFC (see §"Network Egress").
5. **Inspect** plugin health per project (events received, last error, invocation count). On-demand credential verification is available via an ephemeral `POST /v1/plugins/:id/verify` action.
6. **Disable** or **uninstall** a plugin cleanly, revoking its credentials and removing its tools from every project's visible set.

The marketplace must NOT:

- require the operator to write a `PluginManifest` by hand (beyond pasting a plugin URL for advanced flows)
- require the operator to know the plugin's internal protocol
- expose per-plugin implementation details in the project operator workflow
- leak plugin credentials across projects
- make agents in project A aware of tools from plugins enabled only in project B

## Scope

### In scope for v1

- Marketplace registry sources (see §"Registry Sources")
- Plugin activation flow: discover → install → provide credentials → (optional ephemeral verify) → enable per project. There is no separate "connect" lifecycle step.
- Credential wizard contract: a plugin declares required credentials; the control plane guides the operator; the result is stored in `CredentialService` scoped to the credential's actual scope (tenant, workspace, or project) per the wizard
- Per-project plugin enable/disable state with `tool_allowlist`, `signal_allowlist`, and `signal_capture_override`, persisted in the event log
- Per-run tool visibility filter that composes with the existing `ToolTier` system; the same filter applies to `tool_search` so Deferred-tier plugin tools are gated identically
- Operator view extensions to the RFC 010 "Settings" and optional new "Plugins" section
- Signal knowledge capture: auto graph projection (opt-out) and opt-in memory ingestion, both running asynchronously off the durable signal event spine
- Events: `PluginListed`, `PluginInstalled`, `PluginCredentialsProvided`, `PluginCredentialsVerified`, `PluginInstanceReady`, `PluginInstanceStopped`, `PluginEnabledForProject`, `PluginDisabledForProject`, `PluginUninstalled`, `PluginInstallationFailed`, `SignalProjectedToGraph`, `SignalIngestedToMemory`
- A "bundled catalog" shipped with the binary containing curated known-good plugin descriptors

### Explicitly out of scope for v1

- A hosted marketplace service operated by cairn (the marketplace view reads from a catalog, but the catalog is static configuration in v1)
- Payments, licensing, or paid plugins
- Automated plugin updates from remote registries (the operator must explicitly trigger an update in v1)
- In-product plugin authoring tools (operators must still build plugins via the RFC 007 SDK)
- Plugin code-signing verification (future RFC)
- A "recommended plugins" ranking system

## Canonical Model

### Plugin Lifecycle — Two Separate State Machines

The marketplace layer tracks **catalog state** per plugin. The plugin host tracks **process-instance state** per `(plugin_id, credential_scope_key)`. These are independent lifecycles at different levels of abstraction; the marketplace has no opinion about whether any process is currently running.

**Marketplace lifecycle (per plugin, tenant-scoped)**:

```
[Listed] ──install()──▶ [Installing]
                           │
                    success│   failure
                           │         \
                      [Installed]   [InstallationFailed]
                           │
                 provide_credentials()
                           │
                  enable_for_project()
                           │
                  [EnabledForProject A]
                  [EnabledForProject B]
                  (not visible in Project C — no enablement record)
                           │
           disable_for_project(A)
                           │
                  [EnabledForProject B]
             (previous A config retained 30d for re-enable)
                           │
                    uninstall()
                           │
                    [Uninstalled]
```

There is **no `Connected` state in the marketplace lifecycle** and **no `POST /v1/plugins/:id/connect` endpoint**. A plugin that has been installed and had credentials provided is ready to be enabled; its process instances are spawned by the plugin host on demand (see below). Earlier drafts modeled `Connecting` / `Connected` / `ConnectionFailed` as durable marketplace states, but the Q2 resolution (per-project credentials → per-scope processes, see §"Process Instances by Credential Scope") made a single authoritative "is this plugin connected" answer meaningless: the same plugin can have zero, one, or several running process instances simultaneously, each with its own credential scope. A singular marketplace `Connected` flag cannot represent that without lying.

**Process-instance lifecycle (per `(plugin_id, credential_scope_key)`, host-side)**:

```
   [Spawning] ──ready──▶ [Ready] ──drain──▶ [Draining] ──shutdown──▶ [Stopped]
        │                                                                 ▲
        │ failure                                                         │
        ▼                                                                 │
     [Failed] ◀──────────────────── restart ─────────────────────────────┘
```

Process instances follow the existing `PluginState` enum (`Spawning`, `Handshaking`, `Ready`, `Draining`, `Stopped`, `Failed`) from RFC 007, keyed per scope key. See §"Process Instances by Credential Scope" for the spawn/drain rules.

**Spawn policy**:

- **Tool-only plugins** (no `SignalSource` capability declared in the manifest): process instances are spawned **lazily** on first tool invocation in a given `(plugin_id, credential_scope_key)` pair. There is no pre-warm after restart; the next inbound tool call triggers the spawn.
- **Signal-producing plugins** (at least one `SignalSource` capability declared): the **tenant-default** credential-scope instance is spawned **eagerly** at the first `EnablePluginForProject` call and stays resident so it can receive webhook or SSE ingress. Project-specific credential-scoped instances (distinct from the tenant-default) still spawn lazily on first tool invocation in that project. This exception exists because inbound signals must land in a running process — a webhook arriving after a restart with nothing listening would either 504 at ingress or be lost.
- **Credential verification** is an **ephemeral action**, not a lifecycle state. An operator can trigger `POST /v1/plugins/:id/verify` which spawns a transient process, runs the declared `post_install_health_check` method, and shuts down. The result surfaces in the operator view as "credentials verified at T, last check OK" and nothing more — no persistent "Connected" status is committed to the marketplace event log. Verification is a best-effort diagnostic, not an activation step.

**State ownership**:

- Marketplace states (`Listed`, `Installing`, `Installed`, `InstallationFailed`, `Uninstalled`) are **tenant-scoped**. One marketplace record per `(tenant_id, plugin_id)`.
- `EnabledForProject` / `DisabledForProject` is **project-scoped** (installing does not enable).
- Process-instance states (`Spawning`, `Ready`, `Draining`, `Stopped`, `Failed`) are **scope-key-scoped**. Zero or more instances per `(tenant_id, plugin_id)` at any moment, each bound to a `credential_scope_key`.

The two lifecycles are independent. A plugin can be `EnabledForProject A` at the marketplace layer while its process instance for that project's scope key is `Draining` at the host layer during a restart, and the marketplace view reflects both facts separately.

### Plugin Distribution Model: External Binaries Only

**Plugins are external artifacts.** Cairn-app does not bundle, embed, or ship any plugin binaries inside its release archive. A plugin is an independent executable (Rust, Go, Python, anything) that speaks the RFC 007 stdio JSON-RPC protocol. cairn-app references plugins by absolute path or URL and spawns them as subprocesses when invoked. The boundary is strict: no shared libraries, no shared memory, no shared code, no `arg0` subcommand pattern.

This is the "callback" model: cairn knows about a plugin via a descriptor, calls out to its binary as a black-box subprocess, receives signals and tool results back over stdio. Plugins can be added at runtime without touching cairn-app's release. Plugins can be updated independently. Plugins can be authored in any language by anyone.

### Registry Sources

A plugin descriptor (metadata pointing at an external binary) can reach the marketplace through four sources, in order of trust:

| Source | Trust | Writable | V1 |
|---|---|---|---|
| **Bundled catalog** (descriptor only, no binary) | High (curated metadata shipped with cairn-app) | No | Yes |
| **Local file** (manifest + path to local binary) | High (operator-owned) | Yes (operator) | Yes |
| **Remote URL** (manifest + binary download URL) | Medium (operator pastes a URL) | Yes (operator) | Yes |
| **Remote registry** | Configured per deployment | Pull only | Post-v1 |

**Critical point**: the bundled catalog lives at `crates/cairn-plugin-catalog/catalog.toml` and contains **descriptors only**, not binaries. Each catalog entry includes a stable identifier, a download URL or git reference for the binary (or instructions for the operator to provide a path), and the manifest. Installation downloads the binary from the URL OR points cairn at an operator-provided local path.

The catalog is loaded at startup. Entries are **listed-but-not-installed** on first boot. Installation requires the operator to either accept the download URL (cairn fetches and verifies) or provide a local path.

### Plugin Descriptor (marketplace layer)

The existing `PluginManifest` describes the plugin process. The marketplace layer adds a `PluginDescriptor` that wraps the manifest with marketplace metadata:

```rust
pub struct PluginDescriptor {
    pub manifest: PluginManifest,          // existing RFC 007 type
    pub category: PluginCategory,          // for marketplace filters
    pub icon_url: Option<String>,          // displayed in marketplace view
    pub vendor: String,                    // "GitHub", "Slack", "Custom"
    pub required_credentials: Vec<CredentialSpec>,  // drives the wizard
    pub required_network_egress: Vec<String>,       // allowlist hint
    pub post_install_health_check: Option<HealthCheckSpec>,
    pub source: DescriptorSource,          // where this descriptor came from
}

pub enum PluginCategory {
    IssueTracker, ChatOps, Calendar, Files, CustomerSupport,
    Observability, DataSource, CommunicationChannel,
    // Reserved for forward-compat; not surfaced as a marketplace filter in v1.
    // See §Non-Goals — plugin-provided eval scorers are deferred to a future RFC.
    EvalScorer,
    Other(String),
}

pub struct CredentialSpec {
    pub key: String,                  // "github_app_private_key"
    pub display_name: String,         // "GitHub App Private Key"
    pub kind: CredentialKind,         // OAuth2 | ApiKey | AppInstallation | BasicAuth
    pub help_url: Option<String>,     // "where do I get this"
    pub validation: Option<CredentialValidation>,  // optional pre-store check
    pub scope: CredentialScopeHint,   // Tenant | Workspace | Project
}

pub struct HealthCheckSpec {
    pub method: String,               // RPC method on the plugin that proves connectivity
    pub timeout_ms: u64,               // hard cap
    pub success_criteria: serde_json::Value,  // e.g. {"ok": true}
}
```

The `CredentialSpec` is what drives the **credential wizard** — a plugin declares what it needs, the control plane asks the operator for it, stores it via `CredentialService`, and never exposes the raw value again.

### Per-Project Enable State

```rust
pub struct PluginEnablement {
    pub plugin_id: String,
    pub project: ProjectKey,
    pub enabled: bool,
    pub enabled_at: u64,
    pub enabled_by: OperatorId,
    pub tool_allowlist: Option<Vec<String>>,   // subset of plugin's tools
    pub signal_allowlist: Option<Vec<String>>, // subset of plugin's signal types
    pub signal_capture_override: Option<SignalCaptureOverride>,
    pub config_overrides: serde_json::Value,   // plugin-specific settings
}

pub struct SignalCaptureOverride {
    // Per-project override of the plugin's declared knowledge-capture hints.
    // None on a field = inherit the SignalSource capability default
    // (graph_projection default true, memory_ingest default false).
    pub graph_project: Option<bool>,
    pub memory_ingest: Option<bool>,
}
```

`tool_allowlist` lets a project enable a plugin but restrict which of its tools agents can see. If `None`, all tools from the plugin's manifest are visible. If `Some(list)`, only tools in the list are visible.

`signal_allowlist` is the same pattern for signal types: if `None`, every signal type declared in the plugin's `SignalSource` capabilities is routable to this project; if `Some(list)`, only signal types in the list are delivered. Signal scoping is enforced in `SignalRouter` at the per-project filter step alongside the existing enablement check, so a signal type outside the allowlist is dropped before reaching any of the project's subscribers, before trigger evaluation, and before knowledge-capture projection.

`signal_capture_override` allows an operator to override the plugin-declared capture behavior per project — for example, a compliance-sensitive project can force `graph_project: Some(false)` and `memory_ingest: Some(false)` to prevent any signal content from being embedded or projected, even when the plugin manifest declared both as on. See §"Signal Knowledge Capture".

This gives fine-grained control: a project that wants the GitHub plugin for issue ingestion but does not want agents to be able to create pull requests can enable the plugin with `tool_allowlist: Some(["github.get_issue", "github.list_comments"])`, a `signal_allowlist: Some(["github.issue.opened", "github.issue.labeled"])` so push events never reach the project's triggers, and a `signal_capture_override { graph_project: Some(false), memory_ingest: Some(false) }` so no signal content is captured.

Both `tool_allowlist`, `signal_allowlist`, and `signal_capture_override` are carried on the `EnablePluginForProject` command, the `PluginEnabledForProject` event, the HTTP `POST /v1/projects/:proj/plugins/:id` request payload, and the project-settings UI form. All four surfaces must expose the same fields — partial exposure (e.g. HTTP carries it but the UI does not) was a pre-review gap called out in F-W1-09.

### Per-Run Tool Visibility

Current prompt building calls `BuiltinToolRegistry::prompt_tools()` which returns tools filtered by tier (`Core + Registered`, exclude `Deferred`). This RFC adds a second filter: **plugin visibility by project**.

```rust
pub struct VisibilityContext {
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub enabled_plugins: HashSet<String>,   // plugin IDs enabled for this project
    pub allowlisted_tools: HashMap<String, Option<HashSet<String>>>,
    // plugin_id -> Some(tool names) if restricted, None if all allowed
}

impl BuiltinToolRegistry {
    pub fn prompt_tools_for(&self, ctx: &VisibilityContext) -> Vec<BuiltinToolDescriptor> {
        self.tools.iter()
            .filter(|(_, (_, tier))| matches!(tier, ToolTier::Core | ToolTier::Registered))
            .filter(|(name, (handler, _))| self.is_visible_in_context(name, handler, ctx))
            .map(|(_, (handler, tier))| /* build descriptor */ )
            .collect()
    }
}
```

The **same visibility filter must also apply to `tool_search`**, which operates over the Deferred-tier inner registry (a separate code path from `prompt_tools_for`). `tool_search` is how agents discover tools not initially included in their prompt — without the same filter, an agent could search for and then invoke a tool from a plugin that is not enabled for its project:

```rust
impl ToolSearchTool {
    pub fn search(
        &self,
        ctx: &VisibilityContext,
        query: &str,
    ) -> Vec<BuiltinToolDescriptor> {
        // Deferred-tier inner registry — what prompt_tools_for deliberately excludes.
        self.deferred_registry.iter()
            .filter(|(name, (handler, _))| self.is_visible_in_context(name, handler, ctx))
            .filter(|(_, (desc, _))| desc.matches_query(query))
            .map(|(_, (handler, _))| /* build descriptor */ )
            .collect()
    }
}
```

Both `prompt_tools_for` and `ToolSearchTool::search` share the identical `is_visible_in_context` predicate, so a plugin tool is either visible everywhere its project allows or nowhere. There is no asymmetry between "in the prompt" and "findable via search".

Built-in cairn tools (file_read, grep_search, bash, etc.) are **always visible** — they are product-core, not plugin-provided. Visibility scoping applies only to tools registered from plugins.

**This is how "agents should not be aware of tools not relevant to them" is enforced**: at prompt assembly time, before the LLM ever sees the tool catalogue, and at tool-search time, before the LLM can even discover a tool by name.

### Events (Marketplace Layer)

Emitted into the existing event log:

```rust
pub enum MarketplaceEvent {
    PluginListed {
        plugin_id: String,
        source: DescriptorSource,
        listed_at: u64,
    },
    PluginInstallationStarted {
        plugin_id: String,
        initiated_by: OperatorId,
        started_at: u64,
    },
    PluginInstalled {
        plugin_id: String,
        manifest_hash: String,
        at: u64,
    },
    PluginInstallationFailed {
        plugin_id: String,
        reason: String,
        at: u64,
    },
    PluginCredentialsProvided {
        plugin_id: String,
        credential_ids: Vec<CredentialId>,
        provided_by: OperatorId,
        at: u64,
    },
    PluginInstanceReady {
        // Replaces the old PluginConnected event. Describes a specific running
        // process instance, not a tenant-wide "connected" state. Emitted by the
        // plugin host each time a (plugin_id, credential_scope_key) instance
        // reaches the Ready state — lazily on first tool call, or eagerly at
        // EnablePluginForProject for plugins declaring a SignalSource capability.
        plugin_id: String,
        credential_scope_key: CredentialScopeKey,
        reason: InstanceReadyReason,  // EagerSignalSource | LazyFirstInvocation | Restart
        at: u64,
    },
    PluginInstanceStopped {
        plugin_id: String,
        credential_scope_key: CredentialScopeKey,
        reason: InstanceStoppedReason,  // Drained | Failed { details } | Uninstalled
        at: u64,
    },
    PluginCredentialsVerified {
        // Result of an ephemeral POST /v1/plugins/:id/verify action.
        // Does NOT commit any persistent lifecycle state — advisory only.
        plugin_id: String,
        credential_scope_key: CredentialScopeKey,
        outcome: VerificationOutcome,  // Ok | Failed { reason }
        verified_by: OperatorId,
        at: u64,
    },
    PluginEnabledForProject {
        plugin_id: String,
        project: ProjectKey,
        enabled_by: OperatorId,
        tool_allowlist: Option<Vec<String>>,
        signal_allowlist: Option<Vec<String>>,
        signal_capture_override: Option<SignalCaptureOverride>,
        at: u64,
    },
    PluginDisabledForProject {
        plugin_id: String,
        project: ProjectKey,
        disabled_by: OperatorId,
        at: u64,
    },
    PluginUninstalled {
        plugin_id: String,
        uninstalled_by: OperatorId,
        credentials_revoked: Vec<CredentialId>,
        at: u64,
    },
    SignalProjectedToGraph {
        // Emitted after the signal router asynchronously projects a SignalReceived
        // into cairn-graph as a GraphNode(Signal) with provenance edges. Default ON
        // for plugins declaring SignalSource; can be disabled per plugin via the
        // manifest's graph_projection: false opt-out or per project via
        // PluginEnablement.signal_capture_override.
        signal_id: SignalId,
        plugin_id: String,
        project: ProjectKey,
        node_id: GraphNodeId,
        at: u64,
    },
    SignalIngestedToMemory {
        // Emitted after the signal router asynchronously submits declared payload
        // fields to cairn-memory::IngestService. Default OFF — requires a
        // memory_ingest hint on the plugin's SignalSource capability declaration.
        signal_id: SignalId,
        plugin_id: String,
        project: ProjectKey,
        source_id: String,          // plugin:{plugin_id}:signal:{delivery_id}
        chunks_created: u32,
        at: u64,
    },
}
```

Every state transition is audited by the existing event log infrastructure. No separate audit mechanism.

### Commands (Marketplace Layer)

```rust
pub enum MarketplaceCommand {
    InstallPlugin { plugin_id: String, initiated_by: OperatorId },
    ProvidePluginCredentials {
        plugin_id: String,
        credentials: Vec<(String, CredentialValue)>,
        provided_by: OperatorId,
    },
    VerifyPluginCredentials {
        // Ephemeral diagnostic: spawns a transient process instance for the
        // given scope key (defaults to tenant-default if omitted), runs the
        // declared post_install_health_check method, emits
        // PluginCredentialsVerified with the outcome, and shuts down.
        // This command does NOT transition the plugin to any persistent
        // "connected" state. It is a health probe, not a lifecycle step.
        plugin_id: String,
        credential_scope_key: Option<CredentialScopeKey>,
        verified_by: OperatorId,
    },
    EnablePluginForProject {
        plugin_id: String,
        project: ProjectKey,
        tool_allowlist: Option<Vec<String>>,
        signal_allowlist: Option<Vec<String>>,
        signal_capture_override: Option<SignalCaptureOverride>,
        enabled_by: OperatorId,
    },
    DisablePluginForProject {
        plugin_id: String,
        project: ProjectKey,
        disabled_by: OperatorId,
    },
    UninstallPlugin { plugin_id: String, uninstalled_by: OperatorId },
}
```

Each command goes through validation (operator authority, credential shape, target project exists) before emitting events. Note: there is **no `ConnectPlugin` command**. The earlier draft's `POST /v1/plugins/:id/connect` is replaced by `VerifyPluginCredentials` for operator-initiated diagnostics; actual process spawn is the plugin host's responsibility, triggered either lazily by tool invocation or eagerly by `EnablePluginForProject` for `SignalSource`-capable plugins (see §"Plugin Lifecycle — Two Separate State Machines").

## Operator Surfaces

RFC 010 lists the minimum v1 operator views. This RFC adds:

### Settings → Plugins (tenant view)

- list all known plugin descriptors grouped by category
- marketplace state per plugin (`Listed`, `Installing`, `Installed`, `InstallationFailed`, `Uninstalled`) plus a separate per-instance health column summarizing running `(plugin_id, credential_scope_key)` process instances (count Ready, count Failed, most recent state transition)
- per-plugin health (from `plugin_health_monitor.rs`)
- actions: install, provide credentials, **verify credentials** (ephemeral `POST /v1/plugins/:id/verify` diagnostic — spawns a transient instance, runs `post_install_health_check`, emits `PluginCredentialsVerified`, stops), uninstall
- a drill-in view per plugin showing: manifest, required credentials, supplied credentials (metadata only), recent plugin events including `PluginCredentialsVerified` outcomes, running process instances by scope key with their health, enabled projects

### Project → Settings → Enabled Plugins (project view)

- list plugins currently enabled for this project
- per-enablement editors for **`tool_allowlist`**, **`signal_allowlist`**, and **`signal_capture_override`** (graph_project toggle, memory_ingest toggle) — all three fields carried on the `EnablePluginForProject` command and the `POST /v1/projects/:proj/plugins/:id` route payload, so the UI must surface them for the contract to hold
- actions: enable, disable, edit tool allowlist, edit signal allowlist, edit signal capture override

### Overview strip (existing Overview view)

- count of installed plugins
- count of failed plugin **instances** (scope-keyed process instances in the `Failed` state, not a tenant-wide "connection" count) and count of failed recent credential verifications
- most recent plugin activation, instance failure, or credential verification result

The overview strip reports facts about running process instances and ephemeral verification outcomes — it does not imply a durable "connected" state. These additions do not require a separate "Plugins" top-level menu in v1. Settings is sufficient.

## Security Model

### Credential Isolation

A plugin's credentials are stored in `CredentialService` with a **scope key derived from the credential's actual scope**. The scope key may be `(tenant_id, plugin_id)`, `(tenant_id, workspace_id, plugin_id)`, or `(tenant_id, workspace_id, project_id, plugin_id)` — whichever the credential was provisioned at during the wizard flow. The plugin process instance identity follows the credential scope key exactly: **process instances and credential scope keys are 1:1**. Multiple enablements resolving to the same scope key share a single process instance (the singleton case described in §"Process Instances by Credential Scope"), while enablements with distinct scope keys get distinct instances. The plugin process receives its credentials via environment variables set at spawn time by the plugin host. The marketplace layer does NOT pass credentials through the request/response flow — only the host has access, and only when spawning the process.

When a plugin is uninstalled, every credential scoped to that plugin is revoked in the same transaction as the `PluginUninstalled` event.

### Per-Project Signal Routing

Signals emitted by a plugin (RFC 007 `SignalSource` capability) are tagged with the plugin ID at ingestion. The `signal_router_impl.rs` layer filters signals to project subscribers: a signal from a plugin not enabled for a project is dropped before reaching that project's subscribers. A project's `signal_allowlist` on `PluginEnablement` further narrows which signal types from the plugin are delivered to that project's subscribers; if `None`, all signal types declared in the manifest's `SignalSource` capabilities are allowed. This is consistent with the existing signal routing model.

### Signal Knowledge Capture

Plugin signals that pass project-routing filters enter the **signal event spine** (persisted as `SignalReceived` runtime events) and are then asynchronously projected into cairn-graph and optionally ingested into cairn-memory. Capture is a **derived projection** running off the durable event log, not a synchronous side-effect of webhook intake or plugin emit — the HTTP/webhook contract must stay fast and deterministic. This keeps signal knowledge capture consistent with RFC 002's event-sourcing invariants and with RFC 022's `(trigger_id, signal_delivery_id)` dedup ledger: a replayed webhook never double-projects or double-embeds.

Two capture tracks with **different defaults**:

- **Graph projection (default ON)** — every `SignalReceived` event is projected through `cairn-graph::event_projector` into a `GraphNode(Signal)` with provenance edges to the source plugin node and the target project node. This is cheap (a few node/edge inserts per signal, no embedding calls, no chunking) and useful for every provenance query regardless of whether the operator indexes payloads for retrieval. A plugin can opt **out** by declaring `graph_projection = false` on its `SignalSource` capability. An operator can override per-project via `PluginEnablement.signal_capture_override`.

- **Memory ingestion (default OFF)** — a plugin may declare a `memory_ingest` hint on a specific `SignalSource` capability, naming which payload fields to chunk, embed, and index. When the hint is present and the signal passes routing, `cairn-runtime` calls `cairn-memory::IngestService::submit` with a synthetic `source_id = plugin:{plugin_id}:signal:{delivery_id}` so the resulting chunks are deletable via the existing retention mechanisms. This is expensive (chunking + embedding + indexing per payload with real token/compute cost), so it is opt-in at the capability level and further gated per project via `PluginEnablement.signal_capture_override`.

The hints live on the `SignalSource` capability (not plugin-globally), so a plugin can declare different capture behavior per signal type — e.g. `github.issue.opened` ingested to memory but `github.pull_request.synchronize` skipped to avoid churning the index on every commit:

```toml
[[capabilities]]
type = "signal_source"
signals = ["github.issue.opened", "github.issue.labeled", "github.pull_request.opened"]
graph_projection = true   # default; listed here for clarity

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

# pull_request.synchronize is NOT listed — no memory_ingest for commit churn
```

Per-project overrides via `PluginEnablement.signal_capture_override`:

```rust
pub struct SignalCaptureOverride {
    pub graph_project: Option<bool>,  // None = inherit capability default
    pub memory_ingest: Option<bool>,  // None = inherit capability default
}
```

An operator can disable all capture for a project even when the plugin declared hints (e.g. for compliance-sensitive projects that should never have signal content embedded).

**Ordering invariant (cross-reference to RFC 020)**: the signal router must append the corresponding `SignalProjectedToGraph` / `SignalIngestedToMemory` events and the underlying ingest writes **before** emitting the public `SignalReceived` event to downstream subscribers, so subscribers that read memory or graph as part of their handling see a consistent state. The detailed recovery invariant is specified in RFC 020.

### Tool Invocation Isolation

Plugin tools invoked inside a run are evaluated against the `VisibilityContext` before the invocation reaches the plugin host. A run in project A cannot invoke a tool from a plugin enabled only in project B — the tool is not in the prompt and if the LLM somehow invents the name, the invocation is rejected at the orchestrator's execute phase.

### Network Egress

In v1, a plugin's `required_network_egress` list is **declared and displayed only** — it is stored in the descriptor, surfaced to the operator during the install flow, and visible in the per-plugin drill-in view so operators can audit what network endpoints a plugin intends to reach. **v1 does not enforce the allowlist.** Actual egress filtering at the plugin process level (landlock, seccomp, namespace, firewall rules) is out of scope for this RFC and deferred to a future RFC on plugin sandboxing. This matches RFC 016's v1 stance on network policy (recorded but not enforced) and avoids a contract mismatch between goal text and security model.

Operators needing stronger egress guarantees in v1 should run plugins under external process sandboxing (systemd units, container network policies, etc.) outside cairn-app's control.

## Relationship to Existing Systems

| Existing system | How the marketplace layer uses it |
|---|---|
| `cairn-tools/src/plugin_host.rs` | Unchanged. Marketplace commands call the host's existing spawn/supervise/shutdown APIs. |
| `cairn-runtime/src/services/plugin_host.rs` | Unchanged. Runtime service wraps the tools host and is the dispatch point. |
| `cairn-runtime/src/services/plugin_capability_registry.rs` | Extended to track marketplace state (`Listed`, `Installing`, `Installed`, `InstallationFailed`, per-project `EnabledForProject` / `DisabledForProject`, `Uninstalled`) per plugin ID. Process-instance state lives on the plugin host, keyed by `(plugin_id, credential_scope_key)`, and is surfaced separately from marketplace state. |
| `cairn-runtime/src/services/plugin_health_monitor.rs` | Unchanged. Marketplace operator view reads from this. |
| `CredentialService` (`credential_impl.rs`) | Plugin credentials stored here via existing API. Marketplace flow drives the wizard; storage is unchanged. |
| `SignalService` + `SignalRouter` | Signals tagged with plugin ID at ingestion; routing filters by project enablement and per-enablement `signal_allowlist`. |
| `BuiltinToolRegistry` + `ToolTier` | Visibility extension composes with existing tier filter. Built-in tools are always visible; plugin tools are gated by `VisibilityContext`. |
| Event log | Marketplace events are first-class runtime events via the existing pipeline. |
| `cairn-graph::event_projector` | Auto-projects every `SignalReceived` into `GraphNode(Signal)` with provenance edges (default on; plugin opt-out via `graph_projection = false`; per-project override via `signal_capture_override`). |
| `cairn-memory::IngestService` | Consumes signal payload hints when a plugin declares `memory_ingest` on a `SignalSource` capability (default off; per-project override via `signal_capture_override`). Submitted with synthetic `source_id = plugin:{plugin_id}:signal:{delivery_id}` for retention-compatible deletion. |
| `cairn-evals::EvalRunService` | Reserved for future plugin-provided eval scorer integration. **Out of scope in v1** — see §Non-Goals. |

**No existing code is replaced.** This is additive.

## Bundled Catalog (v1 entries)

The bundled catalog is **a curated list of plugin descriptors** — metadata that points at external plugin binaries. cairn-app ships with the catalog metadata but no plugin binaries. For v1 the catalog contains exactly one entry:

- **GitHub** — descriptor pointing at the cairn GitHub plugin (RFC 017). The plugin binary itself is built and released separately from cairn-app and downloaded on install. RFC 017 defines the plugin and its installation flow as the reference implementation of the marketplace mechanism end-to-end.

Additional catalog entries (Slack, Linear, Jira, etc.) are added in later work as separate plugin descriptors. Each entry is a small TOML record + a published binary URL (or a git repo with build instructions). Adding catalog entries does not require new RFCs — once the marketplace mechanism is stable, adding a descriptor is configuration, not architecture.

## Non-Goals

For v1, explicitly out of scope:

- a hosted marketplace service run by cairn
- plugin update notifications from a remote registry
- code-signing verification of plugin binaries
- a plugin publishing flow (publishing is out-of-band — operators get plugins from their vendor or git)
- ratings, reviews, or any community features
- "recommended for your team" plugin ranking
- plugin-to-plugin dependency resolution (plugins are standalone)
- automatic credential rotation triggered by the marketplace layer (the credential service handles rotation independently)
- **`PluginCapability::EvalScorer` is reserved but not implemented in v1.** The enum variant and `PluginCategory::EvalScorer` are declared for forward-compatibility, but v1 does not specify the RPC contract, does not wire plugin-provided scorers into `cairn-evals::EvalRunService`, and does not surface eval-scorer plugins in the marketplace catalog filter. The plugin host **rejects** manifests declaring `capability = "eval_scorer"` at install time with a clear error message: *"EvalScorer capability is reserved for a future RFC; v1 does not support plugin-provided eval scorers. Either remove this capability from the manifest or install an earlier cairn-app version."* The reserved enum variant exists solely so that when the future RFC lands it does not require a breaking rename. Internal eval scorers that ship as cairn-rs code (using `cairn-evals::PluginDimensionScore` and `PluginRubricScorer` directly) are unaffected by this Non-Goal.
- **Plugin network egress enforcement in v1.** Plugins declare a `required_network_egress` hint which is stored and displayed to the operator, but v1 does not enforce the allowlist at the host or sandbox layer. See §"Network Egress".

## Open Questions

1. **Per-plugin config migration when a manifest changes version.** If an operator updates a plugin to a new version and the manifest shape changed, how are per-project `config_overrides` migrated? Proposal: the plugin declares a `config_migration_hint` in the new manifest (either `compatible`, `reset`, or a `from_version` mapping table); the marketplace layer refuses to auto-upgrade when the hint is missing and forces the operator to re-configure per project. **NEEDS DISCUSSION** before implementation.

2. **Multi-version installs.** Can an operator install version 1.2 and 1.3 of the same plugin side by side in one tenant? Proposal: no in v1 — one installed version per plugin ID at a time. Upgrading replaces the previous version after draining. **NEEDS DISCUSSION.**

## Resolved Decisions (from earlier iteration)

The following were asked and resolved during initial drafting and are captured here for audit:

- **Plugin ID uniqueness (Q1)**: globally unique per tenant. A bundled catalog entry and a local-file entry sharing the same `PluginManifest.id` is a **conflict**. Installation is blocked with a clear error until one is renamed. Source is not part of the identity — there is one "github" plugin per tenant, full stop.

- **Per-project credentials for the same plugin (Q2)**: **supported**. Project A can use its own credentials for the GitHub plugin; project B can use different credentials for the same plugin ID. The marketplace layer records credential scope per enablement; the plugin host spawns **one process instance per distinct credential scope**, not one instance per plugin ID. See "Process Instances by Credential Scope" below. This is a change to the existing plugin host behavior and must be tracked as an explicit host-side work item.

- **OAuth flow ownership (Q3)**: **both supported**. The plugin descriptor declares which flow(s) the plugin accepts. The marketplace layer picks based on whether a public callback URL is configured in deployment settings. If yes, cairn exposes `GET /v1/plugins/:id/oauth/callback` and drives the full OAuth handshake; if no (air-gapped or local mode), the operator receives a "paste your token" step and the flow falls back to manual credential entry. Both paths store results in `CredentialService` through identical downstream code.

- **`tool_search` respects plugin visibility**: **yes**. Deferred-tier tools from plugins not enabled in the project's `VisibilityContext` are filtered out of `tool_search` results. Agents cannot discover tools from plugins they are not allowed to use, even via search.

- **Disable → re-enable grace period**: previous per-project `config_overrides` and `tool_allowlist` are **preserved for 30 days** after disable, then garbage-collected. Re-enabling within the grace period restores the previous config. The operator sees a "previous config available" prompt during re-enable.

- **Local mode marketplace**: the marketplace view shows the **same bundled catalog** as team mode. Credentials entered in local mode use the local encryption key (per RFC 011) and the marketplace view displays a persistent warning to that effect.

- **Plugin health cache**: 30-second TTL per plugin, busted immediately on any `MarketplaceEvent` for that plugin.

- **Uninstall drain timeout**: 60 seconds for in-flight tool calls to complete after the plugin enters the `Draining` marketplace state. After timeout, the host forces shutdown; incomplete tool calls emit `ToolInvocationFailed` with reason `plugin_uninstalled_during_call`.

## Process Instances by Credential Scope

This section resolves Q2 by specifying how the plugin host accommodates per-project credentials.

### Current plugin host behavior

The existing `StdioPluginHost` keyed by `plugin_id` spawns at most one process per registered plugin. When a plugin is connected, a single process runs and serves every project that uses the plugin. Credentials are injected into the process at spawn time via environment variables and are fixed for the lifetime of the process.

### V1 change

The plugin host must key managed processes by `(plugin_id, credential_scope_key)` where `credential_scope_key` is derived from the enablement's credential reference:

```rust
// Derived deterministically from the CredentialId(s) the enablement resolved to.
pub struct CredentialScopeKey(String);  // e.g. "cred_gha_abc123" or composite hash
```

**Lookup rule**: when a project invokes a plugin tool, the marketplace layer resolves the project's enablement record, extracts the `credential_scope_key`, and asks the host to dispatch to the process instance for `(plugin_id, credential_scope_key)`. If no such instance exists, the host spawns one using the credentials pointed to by that scope key. If the instance exists and is `Ready`, the tool call dispatches to it.

**Singleton case**: when every project uses the same credential scope (tenant-scoped credential, which is the default for most plugins), the scope key is identical for all enablements and only one process runs. This preserves the current behavior for the common case.

**Isolation case**: when project A and project B declare different credentials for the same plugin, the scope keys differ and the host spawns two processes. Their state, connections, and runtime caches are fully isolated.

**Lifecycle**:
- A process instance is spawned on first use for a scope key
- A process instance is drained and shut down when the last enablement referencing its scope key is disabled or the plugin is uninstalled
- Each process instance has independent health tracking (the marketplace view shows one health row per scope key with a human-readable label derived from the credential name)

**Resource limits**: the plugin manifest's `PluginLimits` apply per process instance. A plugin declaring `max_concurrency: 8` gets 8 concurrent slots per scope key, not 8 shared across all scope keys. This means multi-account deployments cost more resources, which is correct — they're serving more traffic.

### Events for scope-specific instances

The existing process lifecycle events (`PluginStateChanged`, etc.) are extended with an optional `credential_scope_key` field. When present, the event describes a specific instance rather than the plugin as a whole. `PluginInstanceReady` and `PluginInstanceStopped` (introduced in §Events, replacing the earlier `PluginConnected` event) carry `credential_scope_key` as a required field — every process-instance transition is attributed to exactly one scope key. Marketplace-layer events that operate per-project (`PluginEnabledForProject`, `PluginDisabledForProject`) associate to a scope key indirectly through the enablement's resolved credential reference.

### Why this is clean

Per-project credentials without this change would require installing the plugin twice under different IDs. That leaks credential-scoping concerns into the plugin ID namespace, making the marketplace view show "github-devteam" and "github-customer" as unrelated entries. Operators would have to rename plugins when their credentialing changes, and cross-project comparisons would break. Instance-by-scope keeps the plugin identity clean — there is one "GitHub" plugin, but its processes fan out by the credentials it is serving.

## Decision

Proceed assuming:

- the plugin marketplace is a layer above the existing RFC 007 plugin host, not a replacement
- the marketplace lifecycle is `Listed → Installing → Installed → EnabledForProject → DisabledForProject → Uninstalled`; there is **no `Connected` state** and **no `POST /v1/plugins/:id/connect` endpoint**
- process instances follow a separate host-side lifecycle keyed per `(plugin_id, credential_scope_key)`; tool-only plugins spawn lazily on first invocation, `SignalSource`-declaring plugins eager-spawn the tenant-default scope at `EnablePluginForProject`
- credential verification is an ephemeral diagnostic action (`POST /v1/plugins/:id/verify`) that does not commit any persistent lifecycle state
- per-project enablement (including `tool_allowlist`, `signal_allowlist`, and `signal_capture_override`) is persisted in the event log as first-class runtime events
- per-run tool visibility composes with the existing `ToolTier` system via a `VisibilityContext`, and the same filter applies to `tool_search` against the Deferred-tier inner registry
- signal knowledge capture runs asynchronously off the durable signal event spine: graph projection default on, memory ingestion default off per plugin-declared hint, both overridable per project
- the first plugin in the bundled catalog is GitHub (RFC 017), used to prove the marketplace flow end-to-end
- built-in cairn tools (file_read, grep_search, bash, etc.) are always visible and are not subject to marketplace scoping
- `PluginCapability::EvalScorer` is reserved but not implemented in v1; manifests declaring it are rejected at install time
- plugin network egress hints are declared and displayed in v1 but not enforced; enforcement is deferred to a future RFC
- marketplace events flow through the existing event log with no separate audit path
- credentials are stored in the existing `CredentialService` and never travel through the marketplace request/response path
- in v1, the marketplace reads from a bundled static catalog plus operator-supplied local files and URLs; there is no hosted marketplace service
- open questions listed above must be resolved before implementation branches diverge

## Integration Tests (Compliance Proof)

The RFC is considered implemented when the following integration tests pass:

1. **Bundled catalog listing**: boot cairn, call `GET /v1/plugins/catalog`, confirm the GitHub entry appears with `state: listed`
2. **Install flow**: `POST /v1/plugins/:id/install` transitions the plugin to `Installing` and emits `PluginInstallationStarted`, then `PluginInstalled` on success
3. **Credential wizard**: plugin declares a credential spec; `POST /v1/plugins/:id/credentials` stores it in `CredentialService` and emits `PluginCredentialsProvided`
4. **Lazy spawn on first tool use for tool-only plugins**: enable a tool-only plugin for a project, confirm no process is spawned at `EnablePluginForProject` time, invoke a tool from the plugin in a run, confirm the host spawns a `(plugin_id, credential_scope_key)` instance and emits `PluginInstanceReady { reason: LazyFirstInvocation }` before the tool dispatches. A second project enabling the same plugin with the same credential reuses the instance; a second project enabling with a distinct credential spawns a second instance and emits a second `PluginInstanceReady`.
4a. **Eager spawn for SignalSource plugins**: enable a plugin declaring at least one `SignalSource` capability, confirm the tenant-default scope-key instance is spawned immediately at `EnablePluginForProject` and emits `PluginInstanceReady { reason: EagerSignalSource }` **before** the enablement command returns 200. Project-specific credential-scoped instances still spawn lazily on first tool invocation.
4b. **Ephemeral credential verification**: `POST /v1/plugins/:id/verify` spawns a transient instance, runs `post_install_health_check`, emits `PluginCredentialsVerified { outcome: Ok }`, and stops the transient instance. The plugin's marketplace state is unchanged — no persistent `Connected` flag is committed to the event log.
5. **Per-project enable**: `POST /v1/projects/:proj/plugins/:id` emits `PluginEnabledForProject` carrying `tool_allowlist`, `signal_allowlist`, and `signal_capture_override` if provided; tool listing for a run in that project includes the plugin's allowed tools
6. **Per-project isolation**: a run in a different project does not see the plugin's tools in its prompt or in `tool_search` results (both paths must filter by `VisibilityContext`)
7. **Tool allowlist**: enabling a plugin with a `tool_allowlist` subset restricts the tools visible to the project's runs
7a. **Signal allowlist**: enabling a plugin with a `signal_allowlist` subset drops signal types not in the list at `SignalRouter` before they reach any of the project's subscribers, triggers, or knowledge-capture projections
8. **Signal routing + capture defaults**: a signal emitted by the plugin (a) is only delivered to subscribers of enabled projects, (b) is projected to `cairn-graph` as a `GraphNode(Signal)` with provenance edges and emits `SignalProjectedToGraph` (graph_projection default on), (c) is NOT ingested to `cairn-memory` unless the plugin's `SignalSource` capability declared a `memory_ingest` hint for that signal type
8a. **Signal capture opt-in memory ingest**: a plugin declaring `memory_ingest` on a specific `SignalSource.signal_type` produces `SignalIngestedToMemory` events for matching signals with `source_id = plugin:{plugin_id}:signal:{delivery_id}`; deleting the source removes all ingested chunks via the existing retention path
8b. **Per-project signal capture override**: a project enabled with `signal_capture_override { graph_project: Some(false), memory_ingest: Some(false) }` receives the signal for trigger evaluation but produces neither a graph projection nor a memory ingest event for that project's copy of the signal
9. **Uninstall drains runs**: uninstalling a plugin while a tool call is in-flight waits for the call to complete (or times out) before shutting down **every** scope-key instance of that plugin and emitting `PluginInstanceStopped` for each
10. **Event log completeness**: every state transition is reflected in the event log with the correct event variant, including `PluginInstanceReady` and `PluginInstanceStopped` per scope key
11. **EvalScorer manifest rejection**: attempting to install a plugin whose manifest declares `capability = "eval_scorer"` fails at `POST /v1/plugins/:id/install` with a clear error message naming the reserved capability and v1 non-goal
