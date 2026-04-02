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

## Isolation Model

Plugins are isolated by process boundary first.

Required controls:

- explicit permission grants
- filesystem/network/tool restrictions passed in invocation context
- per-invocation timeout
- max concurrency per plugin
- host-enforced cancellation
- crash containment

Plugins must not have implicit access to:

- host secrets
- host DB handles
- unrestricted filesystem
- unrestricted shell

Those are granted only through explicit capability and invocation context.

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

## Non-Goals

For v1, do not optimize for:

- arbitrary plugin-to-plugin RPC
- remote service mesh for plugins
- language-specific SDK magic as the only integration path
- marketplace trust and signing as a prerequisite for local development

## Open Questions

1. Should the first bridge after stdio be HTTP or gRPC?
2. Which plugin categories need streaming responses beyond progress events in v1?
3. How much host-managed sandboxing should be mandatory versus deployment-configurable?

## Decision

Proceed assuming:

- JSON-RPC over stdio is the canonical v1 plugin transport
- plugins are out-of-process and supervised
- the host owns persistence, policy, retries, and canonical state
- plugin contracts must be explicit enough for tools, signals, channels, and eval workers to build against one surface
