# RFC 009: Provider Abstraction and Gateway Non-Goals

Status: draft  
Owner: provider/runtime lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 004](./004-graph-eval-matrix.md), [RFC 006](./006-prompt-registry-release-model.md), [RFC 008](./008-tenant-workspace-profile.md)

## Summary

Cairn v1 will treat model access as a product-owned routing and observability layer with three first-class provider surfaces:

- generation providers
- embedding providers
- reranker providers

The product will own:

- provider capability modeling
- credentials and connection ownership
- route selection
- fallback policy
- cost and latency accounting
- eval linkage

The product will not try to become a general-purpose AI gateway.

## Why

The rewrite needs one provider contract because otherwise:

- runtime workers will route models differently
- memory workers will invent a separate embedding abstraction
- eval workers will score incompatible provider units
- UI workers will build a settings surface without one ownership model

The current Go reference already has real provider selection, fallback, and cost tracking. The Rust rewrite should preserve that useful behavior while expanding it into a product-owned control plane instead of leaving provider choice as ad hoc runtime wiring.

## Product Requirements

The provider layer must support:

- project-visible default model routing
- explicit fallback chains
- provider capability checks before runtime dispatch
- cost and token accounting per call
- latency and failure tracking per route
- eval linkage between prompt releases, task types, and provider choices
- local and hosted provider backends through the same abstraction
- tenant/workspace/project-aware credential and default resolution

It must remain simple enough that Cairn is still a control plane for agents, not a generic vendor proxy.

## Core Provider Surfaces

### Generation Provider

Generation providers are used for:

- assistant turns
- agent planning
- tool-using ReAct loops
- structured output generation
- review or critique passes

The normalized v1 generation contract must support:

- streaming text deltas
- final usage accounting
- tool call emission when supported
- structured output mode when supported
- request timeout and cancellation
- provider/model identity in the response metadata

### Embedding Provider

Embedding providers are used for:

- memory ingest
- document and chunk indexing
- query embedding
- graph-assisted retrieval expansion where embedding generation is needed

The normalized v1 embedding contract must support:

- single and batch input embedding
- explicit embedding dimensionality metadata
- model identity in the response metadata
- cost accounting where available

### Reranker Provider

Reranker providers are used for:

- retrieval reranking
- deep-search result ordering
- evidence prioritization before synthesis

The normalized v1 reranker contract must support:

- ordered candidate scoring
- score explanation fields where available
- model identity in the response metadata
- cost accounting where available

### Not Core In V1

The following are not part of the canonical v1 provider abstraction:

- speech-to-text providers
- text-to-speech providers
- generic image generation providers
- arbitrary vendor-specific APIs exposed as raw pass-through methods

Those may exist later as plugin or channel capabilities, but they are not the contract this RFC defines.

## Canonical Provider Objects

### Provider Adapter

A provider adapter is the system-scoped implementation that knows how to speak to one provider family.

Examples:

- `openai_compatible`
- `anthropic`
- `bedrock`
- `local_ollama`
- `cohere`

Provider adapters are product/runtime code or plugins. They are not tenant-authored data.

### Provider Credential

A provider credential is the secret-bearing object used to authenticate with a provider account or endpoint.

Provider credentials are:

- tenant-scoped by default
- optionally referenced by workspace-owned connections where access should be narrower

Credentials must never be project-scoped runtime truth objects.

### Provider Connection

A provider connection is a configured reachable provider target.

Required fields:

- `provider_connection_id`
- `scope`
- `provider_adapter`
- `credential_ref`
- `display_name`
- `base_url` or endpoint metadata where applicable
- `region` where applicable
- `status`
- `capabilities`
- `created_at`
- `updated_at`

`scope` in v1 may be:

- `tenant`
- `workspace`

Connections represent configured endpoints, not runtime routing decisions.

### Provider Binding

A provider binding is the deployable runtime selection unit.

Bindings are project-scoped and operation-specific.

Required fields:

- `provider_binding_id`
- `project_id`
- `operation_kind`
- `provider_connection_id`
- `provider_model_id`
- `binding_name`
- `status`
- `settings`
- `created_at`
- `updated_at`

`operation_kind` in v1:

- `generate`
- `embed`
- `rerank`

`settings` may include:

- temperature defaults for generation
- max token limits
- timeout budgets
- structured output preference
- provider-specific safe tuning fields explicitly normalized by the adapter

Bindings are what evals and runtime route decisions should point to. They are the canonical unit for "which provider/model path did we use?"

### Route Policy

Route policy is project-scoped and determines:

- which binding is the default for each operation
- which selector-specific overrides exist
- which fallback chain applies
- which policy constraints can veto a route

Route policy is product-owned configuration, not hidden runtime code.

## Capability Model

Provider capability checks must happen before runtime dispatch.

Generation bindings may declare capabilities such as:

- `streaming`
- `tool_use`
- `structured_output`
- `image_input`
- `reasoning_trace`
- `high_context_window`

Embedding bindings may declare:

- `batch_embedding`
- `dimension_override`

Reranker bindings may declare:

- `score_explanations`
- `batch_rerank`

The runtime may request required capabilities, but it must not assume every provider supports every feature.

## Scope Rules

Provider ownership must follow RFC 008.

### Tenant Scope

Tenant-scoped provider data includes:

- provider credentials
- tenant-wide provider connections
- tenant-wide provider policy baselines

### Workspace Scope

Workspace-scoped provider data includes:

- workspace-visible provider connections
- workspace default routing preferences where allowed

### Project Scope

Project-scoped provider data includes:

- provider bindings
- route policies
- route overrides
- provider-related eval outputs
- provider route decisions recorded during runtime

This keeps runtime truth at the project level while still allowing credential and endpoint reuse.

## Routing Model

### Canonical Resolution Inputs

Provider route resolution in v1 may use:

- `project_id`
- `operation_kind`
- `agent_type`
- `task_type`
- `routing_slot`
- explicit request override when allowed by policy

### Resolution Order

For a given operation kind, routing resolves in this order:

1. explicit runtime override if allowed by project policy
2. prompt release selector-specific route binding
3. project route policy override for `routing_slot`
4. project route policy override for `task_type`
5. project route policy override for `agent_type`
6. project default binding for the operation kind
7. workspace default if the project has no explicit binding
8. tenant default if workspace has none

This keeps provider routing aligned with prompt release selector precedence from RFC 006 while still supporting project-level defaults.

### Route Selection Unit

The selected route must always resolve to one concrete `provider_binding_id`.

Runtime code must not skip directly from task context to a raw vendor model string.

### Policy Veto

Before dispatch, route policy may veto a candidate for reasons such as:

- missing required capability
- disallowed provider family
- budget exhausted
- project policy restriction
- safety mode restriction

If vetoed, the runtime may continue to the next candidate in the fallback chain if one exists.

## Fallback Model

### Canonical Fallback Rules

Fallback is explicit and ordered.

Each project route policy may define a fallback chain for each operation kind and selector context.

A fallback chain is an ordered list of `provider_binding_id` candidates.

Fallback may be triggered by:

- transport failure
- timeout
- rate limit or temporary capacity failure
- policy veto
- explicit structured-output failure where the route requires valid structured output

Fallback must not be triggered silently for semantic dissatisfaction in v1. That belongs to evals, policy, or orchestrator behavior, not hidden provider routing.

### Operation Boundaries

Fallback is operation-specific.

V1 must not:

- fall back from generation to reranking
- fall back from embeddings to generation
- treat one provider surface as interchangeable with another

### Determinism

The runtime must record:

- the attempted binding order
- which candidate won
- why previous candidates were skipped or failed

This record is required for operator explanation and eval linkage.

## Runtime and Observability Contract

Every provider call must emit durable product state sufficient to answer:

- which binding was selected
- which provider/model actually served the request
- how long it took
- whether fallback happened
- what it cost
- whether it succeeded or failed

Minimum recorded fields per provider call:

- `provider_call_id`
- `project_id`
- `operation_kind`
- `provider_binding_id`
- `provider_connection_id`
- `provider_adapter`
- `provider_model_id`
- `task_id` or `run_id` where applicable
- `prompt_release_id` where applicable
- `started_at`
- `finished_at`
- `latency_ms`
- `status`
- `fallback_position`
- `input_tokens` where applicable
- `output_tokens` where applicable
- `cost`
- `error_class` where applicable

These records are product telemetry, not optional debug logs.

## Eval and Graph Linkage

RFC 004 requires provider choices to become inspectable product state.

### Eval Linkage

Provider metrics must feed eval matrices at least across:

- prompt release x provider binding x task type
- provider binding x structured-output validity
- provider binding x latency bucket
- provider binding x fallback rate
- provider binding x cost bucket

Eval workers must be able to compare:

- quality vs cost
- quality vs latency
- fallback frequency by binding
- route stability by task type and prompt release

### Graph Linkage

Provider route decisions should be graph-linkable to:

- runs
- tasks
- prompt releases
- eval runs
- retrieval operations

This is what makes "why did the system choose this model and what happened?" answerable in the control plane.

## Local and Hosted Modes

The abstraction must support both:

- hosted provider endpoints
- local/self-hosted model endpoints

The product must not treat local providers as second-class hacks. They use the same binding and route-policy model.

Local mode may support fewer capabilities in practice, but the contract stays the same.

## Compatibility and Migration From Current Cairn

The current Go reference behavior provides useful migration anchors:

- a default provider/model pair
- an optional fallback model
- retry and fallback wrapper behavior
- provider registry and model listing
- cost tracking

The Rust rewrite should preserve those practical capabilities while making them explicit product objects:

- raw provider config becomes provider credential + connection
- runtime default model becomes a project-scoped provider binding
- current fallback model becomes a first explicit fallback chain

The rewrite should not preserve the current implicitness where provider choice can live mostly in process wiring and environment variables.

## Operator Surfaces

The minimum v1 control plane must expose:

- configured provider connections
- project provider bindings
- route policy and fallback chain inspection
- provider health and recent failure visibility
- cost and latency views by provider binding
- eval comparisons involving provider choice

If operators cannot see and reason about provider behavior, the abstraction is incomplete.

## Non-Goals

For v1, Cairn is not trying to be:

- a generic LLM gateway product
- a full vendor-feature superset
- a raw pass-through proxy for arbitrary provider APIs
- a standalone centralized secrets broker
- a billing resale or cost-pass-through platform
- a universal abstraction over every multimodal API shape

The goal is product-owned routing for agent workloads, not gateway maximalism.

## Open Questions

1. Should v1 include speech recognition and synthesis under the same provider framework, or keep them outside the core provider abstraction?
2. Do we need provider-binding-level quotas in v1, or are project and tenant budget controls sufficient initially?
3. How much provider-specific tuning should be normalized into `settings` before the abstraction becomes too leaky?

## Decision

Proceed with:

- three canonical provider surfaces: `generate`, `embed`, `rerank`
- provider credentials and connections owned above the project scope
- project-scoped provider bindings and route policies
- explicit selector-aware routing and fallback chains
- durable provider-call telemetry feeding eval and graph systems
- explicit non-goal of becoming a general AI gateway
