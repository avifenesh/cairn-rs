# RFC 003: Owned Retrieval Replacing Bedrock KB

Status: draft  
Owner: memory/retrieval lead  
Depends on: [RFC 001](./001-product-boundary.md), [RFC 002](./002-runtime-event-model.md)

## Summary

Cairn v1 should replace Bedrock Knowledge Base dependence for core product retrieval with a product-owned retrieval stack.

Bedrock may remain an optional provider for embeddings or reranking, but:

- knowledge storage
- ingest
- chunking
- indexing
- hybrid retrieval
- scoring
- reranking
- deep search

must belong to the product.

## Why

External KB dependence weakens the product in exactly the area where customers need control:

- inspectability
- provenance
- portability
- tuning
- operator trust
- graph integration
- eval integration

If retrieval is core to product value, it must be owned.

## Product Requirements

The retrieval system must support:

- multi-document ingest
- structured and unstructured sources
- chunking with provenance
- embeddings via provider abstraction
- lexical and vector retrieval
- metadata filters
- reranking
- freshness/credibility/corroboration scoring
- graph-assisted retrieval
- deep search / multi-hop retrieval
- operator visibility into retrieval quality

## Initial Architecture

### Source of Truth

Default:

- Postgres for canonical storage
- SQLite support for local mode where feasible

Local-mode contract:

- local and single-user mode may use a degraded but still product-owned retrieval path
- SQLite mode must not silently fall back to “no owned retrieval”
- Postgres is required for team and production-grade retrieval

### Vector Layer

Default:

- `pgvector`
- HNSW indexes

Do not introduce a separate vector database in v1 unless measured product workloads prove Postgres-first retrieval insufficient.

SQLite local mode:

- no HNSW requirement
- use a small-scale owned vector path suitable for local use
- acceptable implementations include brute-force or another simple local ANN strategy

### Lexical Layer

Default:

- Postgres full-text search
- metadata filtering in the product store

This is sufficient to build initial hybrid retrieval without operationally splitting the stack.

SQLite local mode should provide:

- FTS-backed lexical retrieval where feasible
- otherwise a documented degraded lexical path

## Retrieval Pipeline

### Service Shape In V1

V1 uses one canonical retrieval service shape:

- retrieval command handling and query execution run in-process with the main Cairn runtime/API roles
- heavier ingest, parse, chunk, embed, and reindex work may execute asynchronously as runtime-owned jobs
- those async jobs still report through runtime-owned command/event surfaces rather than acting as an independent retrieval authority

V1 does not require a separately deployed retrieval worker service in order to claim owned retrieval.

This keeps deployment shape aligned with RFC 011 while still allowing background ingest and reindex work.

### Ingest

Every ingestable asset should pass through:

- source registration
- normalization
- parsing
- chunking
- metadata extraction
- deduplication
- embedding generation
- index update

### Supported Document Types For The First Sellable Release

The first sellable release must support these canonical source types for owned retrieval ingest:

- plain text
- Markdown
- HTML
- structured JSON documents
- curated knowledge-pack imports defined by RFC 013

The first sellable release may additionally support extracted text from common office or PDF sources where a stable parser pipeline is available, but those formats are additive rather than required for the v1 product claim.

V1 must normalize supported source types into portable owned retrieval documents with explicit provenance rather than keeping parser-specific opaque blobs as the main retrieval unit.

### Chunk Model

Every chunk should retain:

- source document ID
- chunk ID
- source type
- tenant/workspace/project ownership
- timestamps
- provenance metadata
- credibility metadata where applicable
- graph linkage

### Embeddings

Embeddings should be provider-abstracted.

Supported modes:

- hosted provider embeddings
- optional local embeddings later

The architecture should not assume one vendor.

### Retrieval

The initial retrieval pipeline should support:

- lexical-only
- vector-only
- hybrid lexical + vector
- metadata-filtered retrieval
- deep search over multiple retrieval hops

### Reranking

Initial reranking should include:

- MMR
- optional provider-based reranker
- deterministic operator-visible scoring factors

## Scoring Model

The retrieval system should explicitly score on:

- base semantic relevance
- lexical relevance
- freshness decay
- staleness penalties
- source credibility
- corroboration
- graph proximity
- recency of use, where appropriate

These must be inspectable. Hidden heuristics are not enough.

### Configurability Rule

V1 splits scoring into:

- canonical scoring dimensions
- operator-tunable scoring policy

Canonical scoring dimensions are fixed by the product contract and must remain present in every compliant retrieval implementation:

- base semantic relevance
- lexical relevance
- freshness decay
- staleness penalties
- source credibility
- corroboration
- graph proximity
- recency of use where enabled

Operator-tunable scoring policy in v1 may control:

- per-project or per-workspace weight presets
- enable/disable of optional dimensions such as recency-of-use
- freshness and staleness decay parameters within bounded ranges
- retrieval mode selection defaults such as lexical-only, vector-only, or hybrid-first
- reranker enablement where configured

Operator-tunable scoring policy in v1 must not allow:

- removal of required provenance and inspectability
- arbitrary custom scoring code in the core runtime
- hidden provider-specific heuristics that cannot be surfaced in diagnostics

### Retrieval Diagnostics Requirement

For every retrieval request in v1, the product must be able to expose:

- the retrieval mode used
- the candidate-generation stages used
- the scoring dimensions that materially contributed
- the effective scoring policy or preset applied
- the reranker path used where applicable

## Deep Search

Deep search should become a first-class owned subsystem.

Requirements:

- query decomposition
- iterative retrieval
- quality gates
- graph expansion hooks
- synthesis inputs built from owned retrieval state

## Operator Surfaces

The product must expose:

- ingest status
- embedding/index status
- source quality views
- retrieval diagnostics
- top-hit inspection
- why-this-result explanations
- benchmark and eval views for retrieval quality

## Migration Path

### Transitional State

During migration:

- Bedrock KB can remain as an optional fallback or shadow reference
- owned retrieval becomes the primary path

### End State

For v1 product claims:

- Cairn should not require Bedrock KB for its main knowledge product story

## Local Mode Expectations

Local mode is supported for:

- development
- personal use
- small-scale evaluation

Local mode is not the performance target for:

- large corpora
- team concurrency
- heavy graph-assisted retrieval

The product should document this explicitly so SQLite support does not imply full production parity.

## Non-Goals

For v1, do not optimize for:

- operating a dedicated vector cluster
- every possible embedding backend
- every possible document parser
- internet-scale indexing

Focus on product-owned, inspectable retrieval for agent workloads.

## Open Questions

1. How much lexical sophistication is needed beyond Postgres full-text for v1?
2. Should extracted text from PDF/office sources be part of the first sellable release, or remain an additive parser package until parser quality is proven?

## Decision

Proceed with a Postgres-first owned retrieval stack using:

- product-owned ingest and chunking
- provider-abstracted embeddings
- `pgvector` + HNSW
- hybrid lexical + vector retrieval
- operator-visible scoring and reranking
- in-process canonical retrieval services with runtime-owned background ingest jobs
- the supported v1 document-type floor defined above
- fixed scoring dimensions with bounded operator-tunable scoring policy

Bedrock KB becomes optional and transitional, not foundational.
