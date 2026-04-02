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

### Vector Layer

Default:

- `pgvector`
- HNSW indexes

Do not introduce a separate vector database in v1 unless measured product workloads prove Postgres-first retrieval insufficient.

### Lexical Layer

Default:

- Postgres full-text search
- metadata filtering in the product store

This is sufficient to build initial hybrid retrieval without operationally splitting the stack.

## Retrieval Pipeline

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

## Non-Goals

For v1, do not optimize for:

- operating a dedicated vector cluster
- every possible embedding backend
- every possible document parser
- internet-scale indexing

Focus on product-owned, inspectable retrieval for agent workloads.

## Open Questions

1. How much lexical sophistication is needed beyond Postgres full-text for v1?
2. Do we need a separate retrieval worker service in v1, or is an in-process service sufficient?
3. Which document types must be supported in the first sellable release?
4. How much of retrieval scoring should be configurable by operators in v1?

## Decision

Proceed with a Postgres-first owned retrieval stack using:

- product-owned ingest and chunking
- provider-abstracted embeddings
- `pgvector` + HNSW
- hybrid lexical + vector retrieval
- operator-visible scoring and reranking

Bedrock KB becomes optional and transitional, not foundational.
