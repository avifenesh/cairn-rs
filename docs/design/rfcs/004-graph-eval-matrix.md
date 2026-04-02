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
- prompt
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

### Prompt Registry

Prompts must become first-class assets with:

- versions
- release tags
- approval state
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

## Initial Implementation Rules

- every important runtime entity must be graph-linkable
- every prompt release and eval run must be recorded as durable product state
- every matrix must have a stable backing schema, not just derived UI logic
- graph and matrix outputs must be accessible through API and UI

## Non-Goals

For v1, do not optimize for:

- arbitrary graph analytics as a product in itself
- a generic BI platform
- unbounded custom matrix builders
- external graph databases by default

Focus on product explanations, provenance, and decision-quality loops.

## Open Questions

1. Which graph queries matter enough to optimize in v1?
2. How flexible do prompt rollouts need to be in the first release?
3. Which eval metrics should be built in versus plugin-defined?
4. How much of matrix construction should be operator-configurable in v1?

## Decision

Proceed with:

- graph as a first-class product-owned model implemented in the main store first
- prompt registry and eval matrices as first-class product systems
- APIs and UI surfaces that make provenance and evaluation inspectable
