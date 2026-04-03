# RFC 007: Plugin Protocol and Transport

Status: draft  
Owner: plugin/runtime lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 002](./002-runtime-event-model.md), [RFC 008](./008-tenant-workspace-profile.md)

## Summary

The Rust rewrite will use an out-of-process plugin model with a single canonical v1 transport:

- JSON-RPC 2.0 over stdio for local plugins

The same protocol envelope may later be bridged over HTTP or another transport, but stdio is the only required transport for v1 workers.

Plugins may provide:

- tools
- signal sources
- channel adapters
- post-turn analyzers
- policy evaluators
- eval scorers

The protocol must be:

- language-neutral
- capability-declared
- permission-aware
- restartable
- observable

## Why

The product needs a coherent core and polyglot extension surface.

If plugin transport is left abstract:

- tool/runtime workers will invent incompatible contracts
- signal/channel workers will build one-off adapters
- marketplace or installation work will drift from runtime reality

This RFC gives all workers one extension contract.

## Decision

For v1:

- plugins run out of process
- plugins are launched and supervised by the Rust host
- host-to-plugin transport is JSON-RPC 2.0 over stdio
- plugin manifests are declarative and loaded before spawn
- every plugin capability is explicitly declared

Not in v1:

- in-process plugin loading as the primary extension model
- transport-specific business semantics
- ad hoc shell-script plugins without manifest and capability declaration

## Plugin Categories

Supported capability families:

- `tool_provider`
- `signal_source`
- `channel_provider`
- `post_turn_hook`
- `policy_hook`
- `eval_scorer`

One plugin may expose multiple families, but the manifest must declare them separately.

## Transport

### Canonical V1 Transport

- JSON-RPC 2.0
- UTF-8 JSON messages
- stdin/stdout
- stderr reserved for human-readable diagnostics only

Why stdio first:

- easiest cross-language baseline
- simple local supervision and restart
- avoids premature remote service complexity
- works well with sandboxed helper processes

### Deferred Transport

An HTTP or gRPC bridge may be added later, but it must speak the same protocol model.

That bridge is an adapter, not the canonical protocol definition.

## JSON-RPC Envelope Rules

Every protocol message must use a standard JSON-RPC 2.0 envelope.

### Request Envelope

```json
{
  "jsonrpc": "2.0",
  "id": "req_123",
  "method": "tools.invoke",
  "params": {}
}
```

Rules:

- `jsonrpc` must be `"2.0"`
- `id` must be a host-generated string for request/response correlation
- `method` must be one of the canonical method names in this RFC
- `params` must be an object

### Success Response Envelope

```json
{
  "jsonrpc": "2.0",
  "id": "req_123",
  "result": {}
}
```

### Error Response Envelope

```json
{
  "jsonrpc": "2.0",
  "id": "req_123",
  "error": {
    "code": -32000,
    "message": "timeout",
    "data": {
      "status": "timeout"
    }
  }
}
```

### Host-Readable Notifications

Plugins may emit notifications with:

```json
{
  "jsonrpc": "2.0",
  "method": "progress.update",
  "params": {}
}
```

Notifications must not mutate canonical host state directly.

### Canonical Plugin -> Host Notification Bodies

#### `log.emit`

```json
{
  "jsonrpc": "2.0",
  "method": "log.emit",
  "params": {
    "level": "info",
    "message": "cloned repository",
    "invocationId": "inv_123"
  }
}
```

Required `params` fields:

- `level`
- `message`
- `invocationId`

Optional:

- `fields`
- `timestamp`

#### `progress.update`

```json
{
  "jsonrpc": "2.0",
  "method": "progress.update",
  "params": {
    "invocationId": "inv_123",
    "message": "processed 5 of 10 items",
    "percent": 50
  }
}
```

Required `params` fields:

- `invocationId`
- `message`

Optional:

- `percent`
- `stage`
- `etaMs`

#### `event.emit`

```json
{
  "jsonrpc": "2.0",
  "method": "event.emit",
  "params": {
    "invocationId": "inv_123",
    "type": "signal.item_discovered",
    "payload": {}
  }
}
```

Required `params` fields:

- `invocationId`
- `type`
- `payload`

Optional:

- `externalId`
- `timestamp`

## Plugin Manifest

Every plugin must declare:

- plugin ID
- human name
- version
- executable command
- supported capability families
- declared tool names or provider namespaces where applicable
- required permissions
- concurrency limits
- timeout defaults
- health/readiness expectations

The manifest is loaded by the host before process spawn.

### Canonical Manifest Shape

The v1 manifest must be JSON and must support this shape:

```json
{
  "id": "com.example.git-tools",
  "name": "Git Tools",
  "version": "0.1.0",
  "command": ["plugin-binary", "--serve"],
  "capabilities": [
    {
      "type": "tool_provider",
      "tools": ["git.status", "git.diff"]
    }
  ],
  "permissions": [
    "fs.read",
    "fs.write",
    "process.exec"
  ],
  "limits": {
    "maxConcurrency": 4,
    "defaultTimeoutMs": 30000
  }
}
```

Required top-level fields:

- `id`
- `name`
- `version`
- `command`
- `capabilities`

Optional but strongly recommended:

- `permissions`
- `limits`
- `description`
- `homepage`

The manifest must be strict enough that the host can validate:

- what the plugin claims to do
- what it may access
- how it should be supervised

## Lifecycle

### 1. Discover

Host loads plugin manifest and validates:

- schema
- executable availability
- capability declarations
- permission declarations

### 2. Spawn

Host starts the plugin process under a supervised runtime boundary.

### 3. Handshake

Host sends `initialize`.

Plugin responds with:

- protocol version
- plugin ID/version
- effective capability set
- optional limits and feature flags

No operational calls may be made before a successful handshake.

### 4. Operate

Host invokes declared capabilities.

### 5. Shutdown

Host sends `shutdown`, then terminates if the plugin fails to exit gracefully.

## Core RPC Methods

### Host -> Plugin

- `initialize`
- `shutdown`
- `health.check`
- `tools.list`
- `tools.invoke`
- `signals.poll`
- `channels.deliver`
- `hooks.post_turn`
- `policy.evaluate`
- `eval.score`
- `cancel`

### Plugin -> Host

- `log.emit`
- `progress.update`
- `event.emit`

Plugins do not directly mutate core state. They return data to the host, which persists canonical truth.

## Canonical Wire Shapes

These are minimum required method bodies for v1. Workers may add optional fields, but must not remove or rename these fields without amending this RFC.

### `initialize`

Host request `params`:

```json
{
  "protocolVersion": "1.0",
  "host": { "name": "cairn", "version": "0.1.0" }
}
```

Plugin success `result`:

```json
{
  "protocolVersion": "1.0",
  "plugin": { "id": "com.example.git-tools", "name": "Git Tools", "version": "0.1.0" },
  "capabilities": [
    { "type": "tool_provider", "tools": ["git.status", "git.diff"] }
  ],
  "limits": { "maxConcurrency": 4, "defaultTimeoutMs": 30000 }
}
```

### `tools.list`

Plugin success `result`:

```json
{
  "tools": [
    {
      "name": "git.status",
      "description": "Return repo status",
      "inputSchema": { "type": "object" },
      "permissions": ["fs.read", "process.exec"]
    }
  ]
}
```

### `tools.invoke`

Host request `params`:

```json
{
  "invocationId": "inv_123",
  "toolName": "git.status",
  "input": {},
  "scope": { "tenantId": "t1", "workspaceId": "w1", "projectId": "p1" },
  "actor": { "operatorId": "u1" },
  "runtime": { "sessionId": "s1", "runId": "r1", "taskId": "t1" },
  "grants": ["fs.read", "process.exec"]
}
```

Plugin success `result`:

```json
{
  "status": "success",
  "output": { "text": "clean" },
  "events": []
}
```

### `signals.poll`

Host request `params`:

```json
{
  "invocationId": "inv_123",
  "source": { "kind": "rss", "id": "source_1" },
  "scope": { "tenantId": "t1", "workspaceId": "w1", "projectId": "p1" },
  "cursor": "opaque-cursor"
}
```

Plugin success `result`:

```json
{
  "status": "success",
  "events": [],
  "cursor": "next-cursor"
}
```

### `channels.deliver`

Host request `params`:

```json
{
  "invocationId": "inv_123",
  "channel": { "kind": "telegram", "id": "chan_1" },
  "message": { "subject": "Build failed", "body": "..." },
  "recipients": [{ "id": "user_1" }],
  "scope": { "tenantId": "t1", "workspaceId": "w1", "projectId": "p1" }
}
```

Plugin success `result`:

```json
{
  "status": "success",
  "deliveryIds": ["delivery_1"]
}
```

### `hooks.post_turn`

Host request `params`:

```json
{
  "invocationId": "inv_123",
  "scope": { "tenantId": "t1", "workspaceId": "w1", "projectId": "p1" },
  "runtime": { "sessionId": "s1", "runId": "r1", "taskId": "t1" },
  "turn": { "input": {}, "output": {}, "toolCalls": [] }
}
```

Plugin success `result`:

```json
{
  "status": "success",
  "findings": [],
  "patches": []
}
```

### `policy.evaluate`

Host request `params`:

```json
{
  "invocationId": "inv_123",
  "scope": { "tenantId": "t1", "workspaceId": "w1", "projectId": "p1" },
  "actor": { "operatorId": "u1" },
  "action": { "kind": "tool.invoke", "name": "git.status" },
  "context": {}
}
```

Plugin success `result`:

```json
{
  "decision": "allow",
  "reasons": [],
  "appliedPolicies": []
}
```

### `eval.score`

Host request `params`:

```json
{
  "invocationId": "inv_123",
  "scope": { "tenantId": "t1", "workspaceId": "w1", "projectId": "p1" },
  "target": { "kind": "prompt_release", "id": "pr_1" },
  "dataset": { "id": "ds_1" },
  "samples": []
}
```

Plugin success `result`:

```json
{
  "status": "success",
  "scores": [],
  "summary": {}
}
```

### `cancel`

Host request `params`:

```json
{
  "invocationId": "inv_123"
}
```

Plugin success `result`:

```json
{
  "status": "canceled"
}
```

## Isolation Model

Plugins are isolated by process boundary first.

Required controls:

- explicit permission grants
- filesystem/network/tool restrictions passed in invocation context
- per-invocation timeout
- max concurrency per plugin
- host-enforced cancellation
- crash containment
- allowlisted inherited environment only
- explicit working directory or workspace root assignment
- host-managed CPU/memory/process/file-descriptor limits

Plugins must not have implicit access to:

- host secrets
- host DB handles
- unrestricted filesystem
- unrestricted shell

Those are granted only through explicit capability and invocation context.

### Mandatory Isolation Floor

Every plugin execution in v1 must run under the minimum host-managed isolation floor:

- out-of-process supervision
- explicit permission grants
- bounded environment inheritance
- no direct access to product persistence handles or raw secret stores
- host-enforced timeout and cancellation
- host-enforced resource limits

This floor is mandatory in all deployment modes.

### Isolation Guarantees By Execution Class

The product distinguishes between:

- protocol- and host-level restriction
- OS- or sandbox-level confinement

Both execution classes must satisfy the mandatory isolation floor.

Only `sandboxed_process` is required to provide enforced OS-level confinement for filesystem and network access.

### Execution Classes

V1 has two canonical execution classes:

- `supervised_process`
- `sandboxed_process`

`supervised_process` means:

- plugin runs as a supervised child process
- isolation comes from process boundary, permission model, scoped inputs, allowlisted environment, and host-managed limits
- the host must not pass ambient credentials, unrestricted working directories, or unrestricted service handles into the process
- filesystem and network scope are policy-bounded by what the host provides and what the plugin is invoked with, but not guaranteed by an additional OS-level sandbox boundary
- suitable for local development and trusted deployment-local plugins when policy allows

`sandboxed_process` means:

- plugin runs under an additional host-managed sandbox boundary beyond simple child-process supervision
- filesystem, network, and privilege exposure are constrained by that sandbox boundary as well as by protocol permissions
- required for higher-risk plugin deployments in self-hosted team mode

### Canonical V1 Sandboxed Process Requirements

When `sandboxed_process` is used, the sandbox must provide all of the following:

- separate runtime boundary from the main Cairn host process
- read-only root filesystem by default, with only explicitly granted writable scratch or mounted paths
- network disabled by default, with only explicitly granted egress capability
- no ambient privilege escalation
- reduced kernel or host API exposure through a seccomp-equivalent or stronger syscall restriction model
- host-enforced resource controls for CPU, memory, process count, and file descriptors

The product contract defines these required properties even if the exact sandbox backend differs by deployment.

### Filesystem and Network Guarantee Boundary

In v1:

- `supervised_process` may use scoped paths, scoped credentials, and invocation policy to limit what the plugin can practically do
- `supervised_process` does not claim enforced OS-level filesystem or network isolation
- `sandboxed_process` is the only execution class that satisfies enforced filesystem/network confinement requirements

Any plugin that requires enforced confinement rather than trust-plus-policy must run as `sandboxed_process`.

### Deployment Mode Expectations

In local mode:

- `supervised_process` is sufficient as the default
- `sandboxed_process` is optional but supported where available

In self-hosted team mode:

- the product must support policy-enforced `sandboxed_process` execution
- customer-installed plugins and plugins granted high-risk capabilities should default to `sandboxed_process`
- `supervised_process` may still be allowed for explicitly trusted plugins if operator policy permits it
- policies that require enforced filesystem or network confinement must select `sandboxed_process`

### Canonical Backend Stance

V1 defines a sandbox contract, not a single mandatory sandbox technology.

For the first sellable self-hosted release:

- the product must document and support at least one concrete `sandboxed_process` backend
- that backend may be a rootless OCI/container-style runner or another deployment-local sandbox runtime
- stronger backends such as gVisor-compatible runtimes are additive, not required for baseline product coherence

This keeps the product contract stable while avoiding premature commitment to one isolation stack for every deployment.

## Permission Model

Plugin permissions must be expressed at two levels:

- install-time declared permissions
- invocation-time granted permissions

Examples:

- read repository files
- write workspace files
- open network connections
- invoke shell
- access tenant-scoped credentials
- access project-scoped memory

The host decides whether a call is allowed based on effective policy and scope.

## Tenancy and Scope

Every plugin invocation context must include:

- tenant ID
- workspace ID where applicable
- project ID where applicable
- actor/operator ID where applicable
- run ID / task ID if invoked from runtime execution

Plugins may not infer global singleton context.

## Observability Requirements

Every plugin call must carry:

- invocation ID
- plugin ID
- capability family
- tenant/workspace/project scope
- run/task/session linkage where applicable
- start time
- end time
- outcome
- structured error if failed

Plugin progress and logs must be attributable to the calling run/task in operator views.

## Error and Timeout Semantics

Required outcomes:

- success
- retryable failure
- permanent failure
- timeout
- canceled
- protocol violation

The host, not the plugin, classifies whether retries are allowed in the runtime.

Protocol violations may disable the plugin until operator intervention.

## Canonical Inner Schema Rules

To keep workers from inventing incompatible body shapes, these inner object rules apply across methods:

### Capability Objects

Capability objects must include:

- `type`

And may include one of:

- `tools`
- `signals`
- `channels`
- `hooks`
- `policies`
- `scorers`

### Scope Object

The `scope` object must always use:

- `tenantId`
- `workspaceId`
- `projectId`

`workspaceId` and `projectId` may be omitted only when the capability truly operates above that scope.

### Runtime Linkage Object

When runtime linkage is present, it must use:

- `sessionId`
- `runId`
- `taskId`

### Event Array Shapes

Any `events` array returned by plugins must contain structured objects with:

- `type`
- `payload`

Optional:

- `timestamp`
- `externalId`

### Scores Array Shapes

Any `scores` array returned by eval plugins must contain structured objects with:

- `metric`
- `value`

Optional:

- `label`
- `reason`
- `sampleId`

### Delivery Result Shapes

Any delivery result must use:

- `deliveryIds` as an array of opaque strings

Optional:

- `warnings`

### Policy Decision Shapes

Policy decisions must use:

- `decision` with one of `allow`, `deny`, `review`
- `reasons` as an array of strings
- `appliedPolicies` as an array

## Concurrency Rules

- each plugin declares max concurrency
- host may further restrict concurrency by policy
- invocations are independent units and must be cancelable
- long-running providers must emit progress or heartbeat updates if used in interactive flows

## Packaging Direction

For v1:

- local manifest + executable path is sufficient
- registry/marketplace integration is additive, not foundational

This keeps the runtime contract stable even if installation UX changes later.

### First Bridge After Stdio

If Cairn adds a second transport after stdio, the first bridge should be HTTP rather than gRPC.

Reasoning for v1 and near-post-v1 evolution:

- HTTP is simpler to debug and operate across deployment boundaries
- HTTP aligns better with heterogeneous plugin ecosystems and lightweight service wrappers
- the canonical contract can still remain JSON-RPC-shaped even if the transport later changes

gRPC may still be useful later for specialized high-throughput or strongly typed internal service cases, but it is not the first bridge to optimize for.

### Streaming Rule In V1

V1 does not require plugin categories to expose streaming result bodies beyond:

- progress updates
- heartbeat-style liveness updates

That means:

- tools do not need streaming stdout/result protocols in the plugin contract
- signal plugins do not need streaming result feeds through the plugin RPC surface
- channel plugins do not need streaming delivery-result bodies
- eval plugins do not need streaming score output

If a workflow needs richer streaming later, that should be introduced as an explicit extension rather than implied by the base v1 protocol.

## Non-Goals

For v1, do not optimize for:

- arbitrary plugin-to-plugin RPC
- remote service mesh for plugins
- language-specific SDK magic as the only integration path
- marketplace trust and signing as a prerequisite for local development

## Open Questions

1. Which post-v1 workflows, if any, justify richer streaming result bodies beyond progress and heartbeat updates?

## Decision

Proceed assuming:

- JSON-RPC over stdio is the canonical v1 plugin transport
- plugins are out-of-process and supervised
- the host owns persistence, policy, retries, and canonical state
- plugin contracts must be explicit enough for tools, signals, channels, and eval workers to build against one surface
- the minimum host-managed isolation floor is mandatory in all modes
- local mode may default to `supervised_process`
- self-hosted team mode must support policy-enforced `sandboxed_process` execution
- HTTP is the first bridge to consider after stdio, not gRPC
- progress and heartbeat updates are the only required streaming-style plugin outputs in v1
- only `sandboxed_process` provides enforced filesystem/network confinement in v1
