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
- required capabilities
- disabled optional capabilities

Bindings are what evals and runtime route decisions should point to. They are the canonical unit for "which provider/model path did we use?"

### Binding Settings Normalization Rule

In v1, `settings` is for normalized product-facing tuning only.

That means:

- adapters may expose provider-specific tuning only after mapping it into a stable normalized field with clear semantics
- bindings must not become opaque bags of arbitrary provider-native flags
- if a provider feature cannot be expressed through a stable normalized field in v1, it should remain unsupported rather than leaking raw provider shape into the product contract

This keeps provider bindings comparable across evals, route policy, and operator surfaces.

### Provider Policy Baseline

Provider policy baselines are inherited configuration objects above the project layer.

They may exist at:

- tenant scope
- workspace scope

They define:

- allowed provider adapters
- preferred provider connections
- default route templates by operation kind where allowed
- budget or quota guardrails where defined elsewhere
- capability restrictions
- policy-level retry and timeout ceilings

Baselines are not runtime truth objects. They constrain and seed project route policy.

### Provider Route Template

A provider route template is the canonical higher-scope representation for an inherited default or selector-specific route before it becomes project runtime truth.

Templates may exist at:

- tenant scope
- workspace scope

Required fields:

- `provider_route_template_id`
- `scope`
- `operation_kind`
- `provider_connection_id`
- `provider_model_id`
- `template_name`
- `template_settings`
- `created_at`
- `updated_at`

Templates may also declare:

- required capabilities
- disabled optional capabilities
- timeout ceilings
- retry ceilings

Templates are allowed to refer to tenant- or workspace-scoped provider connections.

Templates must not directly reference project-scoped provider bindings.

Project-scoped runtime truth is always expressed through provider bindings, not inherited templates.

### Route Policy

Route policy is project-scoped and determines:

- which binding is the default for each operation
- which selector-specific overrides exist
- which fallback chain applies
- which policy constraints can veto a route

Route policy is product-owned configuration, not hidden runtime code.

### Route Attempt

A route attempt is the durable record of one candidate considered during runtime resolution.

Required fields:

- `route_attempt_id`
- `route_decision_id`
- `project_id`
- `operation_kind`
- `provider_binding_id`
- `route_selector_context`
- `attempt_index`
- `decision`
- `decision_reason`
- `created_at`

`decision` in v1:

- `selected`
- `vetoed`
- `failed`
- `skipped`

### Route Decision

A route decision is the summarized runtime outcome for one logical provider-routed request for one operation kind.

There is exactly one route decision per logical runtime request for one operation kind.

That one decision may contain:

- many route attempts
- zero or more dispatched provider calls

This means post-dispatch fallback remains part of one logical decision rather than creating a second decision object per dispatched candidate.

Required fields:

- `route_decision_id`
- `project_id`
- `operation_kind`
- `terminal_route_attempt_id` when at least one candidate reached dispatch
- `selected_provider_binding_id` when a route is selected
- `selected_route_attempt_id` when a route is selected
- `selector_context`
- `attempt_count`
- `fallback_used`
- `final_status`
- `created_at`

`final_status` in v1:

- `selected`
- `failed_after_dispatch`
- `no_viable_route`
- `cancelled`

Route decisions link the runtime event stream to evals, graph, and operator explanation surfaces.

### Provider Call

A provider call is the execution record for the actual provider dispatch made after route resolution.

Required fields:

- `provider_call_id`
- `route_decision_id`
- `route_attempt_id`
- `project_id`
- `operation_kind`
- `provider_binding_id`
- `provider_connection_id`
- `provider_adapter`
- `provider_model_id`
- `started_at`
- `finished_at`
- `latency_ms`
- `status`
- `input_tokens` where applicable
- `output_tokens` where applicable
- `cost`
- `error_class` where applicable

Every executed provider call must belong to exactly one route decision and exactly one dispatched route attempt.

Skipped and vetoed route attempts must not create provider-call records.

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

### Capability Ownership

Capability ownership is split across three layers:

- provider adapter
  - defines the canonical capability vocabulary and normalization rules for that provider family
- provider connection
  - records endpoint-specific availability and health-related capability constraints
- provider binding
  - defines which capabilities are required, preferred, disabled, or safe to use for this project/runtime path

This keeps capability truth from drifting between vendor adapters, endpoint observations, and project intent.

### Effective Capability Set

The effective capability set for a runtime call is computed from:

1. adapter-declared capability vocabulary
2. connection-declared availability
3. binding-required and binding-disabled capability rules
4. route-policy restrictions

The runtime may only dispatch a call when the effective capability set satisfies the request requirements.

This computation must be inspectable through the operator control plane.

## Scope Rules

Provider ownership must follow RFC 008.

### Tenant Scope

Tenant-scoped provider data includes:

- provider credentials
- tenant-wide provider connections
- tenant-wide provider policy baselines
- tenant-scoped provider route templates

### Workspace Scope

Workspace-scoped provider data includes:

- workspace-visible provider connections
- workspace provider policy baselines
- workspace-scoped provider route templates

### Project Scope

Project-scoped provider data includes:

- provider bindings
- route policies
- route overrides
- route attempts
- route decisions
- provider calls
- provider-related eval outputs

This keeps runtime truth at the project level while still allowing credential and endpoint reuse.

## Canonical Inheritance Model

Provider routing and policy inheritance must be explicit product state.

V1 must not rely on implicit environment inheritance or hidden in-process defaults once a project exists.

### Inheritance Layers

The canonical inheritance chain is:

1. tenant provider policy baseline
2. workspace provider policy baseline
3. project route policy
4. explicit runtime override, when allowed by project policy

Each lower layer may narrow or override the previous layer, but it may not silently expand disallowed provider access.

### Effective Route Policy

For runtime use, Cairn should materialize an effective route policy read model per project and operation kind.

That read model must resolve:

- default binding
- selector-aware overrides
- fallback chain
- capability restrictions
- timeout and retry ceilings
- whether explicit runtime override is allowed

### Materialization Rule

Higher-scope defaults must not be used directly at runtime as if they were project bindings.

The canonical flow is:

1. tenant and workspace baselines point to route templates
2. project route policy inherits or overrides those templates
3. the effective route policy materializes project-scoped provider bindings from the resolved templates
4. runtime resolution selects only among project-scoped provider bindings

This keeps higher-scope policy inheritance compatible with RFC 008 while preserving project-scoped runtime truth.

Workers should build against the effective route policy view rather than recomputing inheritance ad hoc in multiple places.

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

Resolution first computes the effective route policy from the inheritance model above.

For a given operation kind, routing resolves in this order:

1. explicit runtime override if allowed by project policy
2. prompt release selector-specific route binding
3. project route policy override for `routing_slot`
4. project route policy override for `task_type`
5. project route policy override for `agent_type`
6. effective project default binding for the operation kind

Workspace and tenant defaults participate only through the effective route policy inheritance model above. Runtime resolution should not re-implement inheritance as a second algorithm.

This keeps provider routing aligned with prompt release selector precedence from RFC 006 while still supporting inherited defaults.

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

Every veto must be written as a route-attempt record with a concrete `decision_reason`. Vetoes must not exist only in ephemeral logs.

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

### Post-Dispatch Fallback Representation

Post-dispatch fallback is represented inside one route decision.

The canonical model is:

- one logical runtime request creates one `route_decision`
- every considered candidate becomes a `route_attempt`
- every dispatched candidate creates one `provider_call`
- if a dispatched candidate fails and the policy allows fallback, the next candidate becomes another `route_attempt` and may produce another `provider_call`
- the route decision closes only when one candidate succeeds, the runtime is cancelled, or no viable candidates remain

V1 must not create a new route decision object for each dispatched fallback candidate.

This keeps runtime, graph, eval, API, and UI workers aligned on one operator-visible story per logical request.

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
- `route_decision_id`
- `route_attempt_id`
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

Provider-call telemetry complements but does not replace route attempts and route decisions:

- route attempts explain candidate selection and veto
- route decisions explain the chosen binding and fallback path
- provider calls explain execution outcome and cost

### Canonical Linkage

One runtime dispatch must be explainable through this chain:

- one `route_decision`
- one or more `route_attempt` rows linked by `route_decision_id`
- zero or more `provider_call` rows linked to dispatched route attempts

Rules:

- every route attempt belongs to exactly one route decision
- every route decision may have many route attempts
- every executed provider call belongs to exactly one route attempt whose decision is `selected` or `failed`
- a dispatched route attempt must have exactly one provider call
- a route decision with final status `selected` or `failed_after_dispatch` must have at least one provider call
- a route decision resolved entirely by veto or skip must have no provider call
- if `final_status=selected`, `selected_route_attempt_id` and `selected_provider_binding_id` are required
- if `final_status=failed_after_dispatch`, `terminal_route_attempt_id` is required and `selected_route_attempt_id` must be null
- if `final_status=no_viable_route`, all route attempts must be `vetoed` or `skipped`

This linkage is canonical for graph, eval, audit, and operator explanation work.

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
2. Should a later release add provider-binding-level quotas after project and tenant budget controls have proven insufficient?

## Decision

Proceed with:

- three canonical provider surfaces: `generate`, `embed`, `rerank`
- provider credentials and connections owned above the project scope
- project-scoped provider bindings and route policies
- explicit selector-aware routing and fallback chains
- durable provider-call telemetry feeding eval and graph systems
- explicit non-goal of becoming a general AI gateway
- v1 relies on project and tenant budget controls rather than provider-binding-level quotas
- provider binding `settings` are limited to normalized product-facing tuning, not arbitrary provider-native flag bags
