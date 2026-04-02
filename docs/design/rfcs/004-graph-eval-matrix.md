# RFC 004: Graph and Eval Matrix Model

Status: draft  
Owner: graph/evals lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 002](./002-runtime-event-model.md), [RFC 003](./003-owned-retrieval.md)

## Summary

Cairn v1 should include two first-class product systems:

- a graph layer for provenance, execution, and knowledge relationships
- an eval matrix layer for prompts, models, tools, policies, and routing

These should be explicit product capabilities, not hidden implementation details.

## Why

Without a graph, the system can act powerfully but remain opaque.

Without eval matrices, the system can evolve but not improve systematically.

These two layers are what turn:

- runtime facts into explanations
- retrieval facts into provenance
- prompt changes into measured decisions
- routing heuristics into product controls

## Graph Scope

### Graph Entity Types

Initial graph node categories:

- session
- run
- task
- approval
- checkpoint
- mailbox message
- tool invocation
- memory
- document
- chunk
- source
- prompt_asset
- prompt_version
- prompt release
- eval run
- skill
- channel target

### Edge Types

Initial edge categories:

- triggered
- spawned
- depended_on
- approved_by
- resumed_from
- sent_to
- read_from
- cited
- derived_from
- embedded_as
- evaluated_by
- released_as
- rolled_back_to
- routed_to
- used_prompt
- used_tool

### Initial Product Uses

The graph must support:

- explain why a result or action happened
- visualize task/subagent execution flow
- show provenance for memory and retrieval
- support graph-assisted retrieval expansion
- expose dependencies between prompts, skills, tools, and outcomes

### V1 Graph Queries To Optimize

V1 does not need arbitrary graph analytics, but it does need first-class support for these query families:

- execution trace for a session, run, or task
- subagent/task dependency path and rollback or resume lineage
- prompt provenance for a runtime outcome
- retrieval provenance from answer -> chunk -> document -> source
- tool and policy involvement for a runtime decision
- eval-to-asset lineage for prompt releases, provider routes, and retrieval policies

These query families are the optimization target for v1 graph read models and APIs.

### Graph Query Surface In V1

V1 exposes graph capabilities primarily through product-shaped read endpoints for the optimized query families above.

This means:

- operators and product APIs query named graph views aligned to product workflows
- the core v1 contract does not require a fully general graph traversal API
- internal graph storage and read models may still support richer traversal internally where needed

If a more general traversal API appears in v1, it is additive and non-canonical. Workers must not assume one exists for core product flows.

## Graph Storage Strategy

Initial decision:

- store graph data in the product-owned store
- use typed nodes and edges
- expose graph-oriented read models and query APIs

Do not introduce a separate graph database in v1 unless a clear product workload proves it necessary.

## Eval Matrix Scope

### Matrix Categories

Initial matrices should include:

- prompt x model x task-type
- provider routing matrix
- permission matrix by mode/tool/channel/tenant
- memory source quality matrix
- skill health / intervention matrix
- guardrail/policy outcome matrix

All matrix rows and graph entities that represent owned product resources must carry tenant/workspace/project scope where applicable.

### Matrix Ownership Rule

Each matrix in v1 must have:

- a canonical subject type
- a canonical row grain
- a canonical metric set
- a canonical scope model

Matrices are product state backed by stable schemas, not ad hoc UI calculations.

### Canonical Matrix Row Grain

V1 defines the canonical row grain for each initial matrix category as follows.

#### Prompt Comparison Matrix

Canonical subject:

- `prompt_release`

Canonical row grain:

- one row per evaluated `prompt_release_id` x `provider_binding_id` x effective selector context

Effective selector context in v1 must resolve to one of:

- `project_default`
- `agent_type`
- `task_type`
- `routing_slot`

This aligns the matrix with RFC 006, where runtime behavior is governed by project-scoped prompt releases with explicit selector targets.

#### Provider Routing Matrix

Canonical subject:

- `route_decision`

Canonical row grain:

- one row per `route_decision_id`

Supporting drill-down records may expose:

- linked `route_attempt` rows
- linked `provider_call` rows

But the operator-facing matrix row must summarize one logical routed request, not one attempted candidate and not one low-level call in isolation.

This aligns the matrix with RFC 009, where one logical request creates one `route_decision` and may contain many attempts and zero or more provider calls.

#### Permission Matrix

Canonical subject:

- permission decision family

Canonical row grain:

- one row per effective permission policy outcome for `mode x capability x scope`

#### Memory Source Quality Matrix

Canonical subject:

- memory source or source document family

Canonical row grain:

- one row per `source_id` or equivalent canonical memory-source unit within scope

#### Skill Health / Intervention Matrix

Canonical subject:

- skill

Canonical row grain:

- one row per `skill_id` within scope

#### Guardrail / Policy Outcome Matrix

Canonical subject:

- policy or guardrail rule

Canonical row grain:

- one row per policy-rule outcome slice within scope and comparison window

### Matrix Scope Rule

Unless a matrix category explicitly says otherwise:

- runtime-facing comparison matrices are project-scoped in v1
- tenant/workspace views are aggregate read models built over project-scoped or library-scoped canonical rows

For v1 specifically:

- prompt comparison rows are project-scoped because prompt releases are project-scoped
- provider routing rows are project-scoped because route decisions are project-scoped runtime facts
- permission and policy matrices may aggregate across project rows, but must preserve the canonical underlying scope of each row

### Prompt Registry

Prompts must become first-class assets with:

- versions
- release tags
- release lifecycle state
- rollout target
- rollback linkage
- associated evaluation history

### Eval Runs

An eval run should include:

- subject under test
- dataset or trace source
- evaluator type
- metrics
- cost
- version linkage
- output artifacts

### Built-In vs Plugin-Defined Metrics

V1 distinguishes between:

- built-in canonical metrics
- plugin-defined supplemental metrics

Built-in canonical metrics must exist wherever applicable for:

- task success or outcome status
- latency
- cost
- policy outcome
- retrieval quality signals
- prompt or route comparison summaries

Plugin-defined metrics may add domain-specific measurements, but they must:

- declare metric names and value types explicitly
- attach to a canonical eval run
- not replace the built-in core metrics required for operator comparison

This keeps the product comparable out of the box while still allowing extension.

### Mandatory Retrieval-Quality Metrics In V1

For retrieval-oriented evals and matrices, the first sellable release must include built-in metrics for:

- hit-at-k style retrieval success
- citation or evidence coverage where the workflow expects cited answers
- source diversity or corroboration signal
- retrieval latency
- retrieval cost where provider-backed embedding or reranking contributes cost

Where a workflow does not use citations or explicit evidence packaging, the product may omit citation-specific reporting for that run, but it must still preserve the canonical metric vocabulary and mark non-applicable metrics explicitly.

## Product Surfaces

The operator should be able to answer:

- why did the agent choose this?
- which prompt/model combination works best for this task type?
- which memory sources are helping or hurting?
- which skills are decaying?
- which policies are blocking useful work or permitting bad work?
- what changed between prompt release A and B?

If the system cannot answer these, graph and matrix support is incomplete.

## Relationship Between Graph and Evals

These systems should reinforce one another:

- graph edges provide provenance and execution context to evals
- evals score prompts, tools, policies, and retrieval behavior
- graph views should surface which assets were involved in a poor or strong outcome

This should not be two isolated subsystems.

### Prompt Rollout Alignment

Prompt rollout flexibility in v1 is defined by RFC 006.

For graph and matrix purposes, assume:

- rollout is explicit selector-based activation
- no percentage rollout in v1
- no weighted live prompt traffic splitting in v1

Graph and eval surfaces should therefore compare:

- releases across explicit selectors
- releases across eval runs
- releases before and after promotion or rollback

They do not need to model live percentage traffic allocation in v1.

## Initial Implementation Rules

- every important runtime entity must be graph-linkable
- every prompt release and eval run must be recorded as durable product state
- every matrix must have a stable backing schema, not just derived UI logic
- graph and matrix outputs must be accessible through API and UI
- matrix configuration must be bounded by product-defined subjects, scopes, and metrics

### Operator Configurability Rule

Operators may configure in v1:

- which built-in matrix views are enabled for a project or workspace
- threshold and highlight policies for built-in metrics
- which supplemental plugin-defined metrics are visible or eligible in specific views
- comparison windows and grouping dimensions where the backing schema supports them

Operators may not define in v1:

- arbitrary new matrix subject types in the core product
- arbitrary new join logic between graph and eval systems
- matrices whose semantics are known only to one plugin without canonical eval-run linkage

## Non-Goals

For v1, do not optimize for:

- arbitrary graph analytics as a product in itself
- a generic BI platform
- unbounded custom matrix builders
- external graph databases by default

Focus on product explanations, provenance, and decision-quality loops.

## Open Questions

1. Should v1 add a non-canonical advanced graph traversal API for internal/admin use, or defer that entirely until after the first sellable release?

## Decision

Proceed with:

- graph as a first-class product-owned model implemented in the main store first
- prompt registry and eval matrices as first-class product systems
- APIs and UI surfaces that make provenance and evaluation inspectable
- the optimized v1 graph query families defined above
- built-in canonical metrics plus supplemental plugin-defined metrics
- bounded operator configurability over product-defined matrices rather than arbitrary matrix construction
