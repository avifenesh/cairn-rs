# RFC 013: Artifact Import/Export Contract

Status: draft  
Owner: platform/product lead  
Depends on: [RFC 003](./003-owned-retrieval.md), [RFC 006](./006-prompt-registry-release-model.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 012](./012-onboarding-starter-templates.md)

## Summary

V1 needs one canonical import/export contract for structured product artifacts.

That contract must define:

- one bundle schema
- one identity and provenance envelope
- one import/export service model
- one reconciliation model for create, reuse, update, skip, and conflict outcomes

This RFC covers the structured import/export path for:

- prompt assets and prompt versions
- curated knowledge packs

It also defines how direct document import relates to the structured bundle path.

## Why

The current RFC set now defines:

- how prompts behave
- how onboarding bootstraps starter content
- how knowledge import should reconcile

What is still missing is the artifact contract that ties those together.

Without a canonical bundle/import/export model:

- prompt workers will invent one export format
- memory workers will invent a different knowledge-pack format
- API workers will expose mismatched import semantics
- UI workers will guess at conflict and provenance behavior

The result would be drift in one of the most adoption-critical product surfaces.

## Product Requirement

V1 must let a team:

- export prompt libraries and curated knowledge packs from one Cairn deployment
- import them into another deployment or scope
- preserve provenance and identity
- understand exactly what was created, reused, updated, skipped, or conflicted

This must work through one canonical service model even if the operator starts the flow from:

- CLI
- API
- control-plane UI

## Canonical Artifact Families

### Structured Bundle Families

V1 structured bundle import/export covers:

- `prompt_library_bundle`
- `curated_knowledge_pack_bundle`

These are the only first-class structured bundle families in v1.

### Physical Format Rule

V1 uses one canonical physical structured bundle format:

- one JSON document
- one canonical bundle envelope
- one `bundle_type` discriminator
- one `artifacts` array carrying typed artifact entries

This means prompt-library bundles and curated-knowledge-pack bundles are sibling bundle types inside one physical format family, not two unrelated physical formats.

Operators and workers should be able to reason about:

- one parser
- one validator
- one provenance envelope
- one import/export service contract

Variation belongs in typed bundle and artifact contents, not in separate physical file formats.

### Direct File Import

Direct document/file import remains supported for onboarding and knowledge ingest, but it is not itself the canonical portable bundle format.

Direct file import should feed the same import service and provenance model where applicable.

## Canonical Bundle Envelope

Every structured bundle must have one canonical envelope.

Required top-level fields:

- `bundle_schema_version`
- `bundle_type`
- `bundle_id`
- `bundle_name`
- `created_at`
- `created_by`
- `source_deployment_id` where available
- `source_scope`
- `artifact_count`
- `artifacts`
- `provenance`

`bundle_type` in v1:

- `prompt_library_bundle`
- `curated_knowledge_pack_bundle`

`source_scope` must identify the originating scope:

- tenant
- workspace
- project where applicable

`provenance` must include enough metadata to explain where the bundle came from and how it was produced.

### Canonical Serialization

V1 canonical serialization is:

- UTF-8 JSON
- one top-level bundle object
- stable field names as defined by this RFC

Compression, archive wrapping, or signed envelope layers may be added later, but they are not the canonical v1 format.

### Integrity Rule In V1

Content hashing is the canonical integrity mechanism in v1.

That means:

- bundle-level and artifact-level integrity rely on the canonical identity and hash fields defined by this RFC
- signed bundle envelopes are not required for the first sellable release
- stronger trust layers may be added later as additive envelope features without changing the core bundle model

## Canonical Identity and Provenance Envelope

Every artifact inside a bundle must carry one canonical identity/provenance envelope.

Required fields:

- `artifact_kind`
- `artifact_logical_id`
- `artifact_display_name`
- `origin_scope`
- `origin_artifact_id` where available
- `content_hash`
- `source_bundle_id`
- `origin_timestamp`
- `metadata`

Optional but recommended:

- `lineage`
- `tags`
- `source_refs`

### Identity Rules

The import contract must distinguish:

- portable logical identity
- source-system object identity

`artifact_logical_id` is the portable identity used for reconciliation across deployments.

`origin_artifact_id` is the source-system object ID when known, but it must not be the only identity key because those IDs are not portable.

### Provenance Rules

Import/export provenance must remain inspectable after materialization.

Operators must be able to answer:

- where this artifact came from
- which bundle introduced it
- whether it was copied, reused, updated, skipped, or conflicted on import

## Canonical Artifact Entry Shape

Every entry in the `artifacts` array must use one canonical typed shape:

- shared identity/provenance envelope fields from this RFC
- one `payload` object whose schema is determined by `artifact_kind`

Required entry fields:

- `artifact_kind`
- `artifact_logical_id`
- `artifact_display_name`
- `origin_scope`
- `origin_artifact_id` where available
- `content_hash`
- `source_bundle_id`
- `origin_timestamp`
- `metadata`
- `payload`

The outer entry shape must stay consistent across bundle types. Variation belongs inside the typed `payload`.

## Prompt Library Bundle

### Contents

A `prompt_library_bundle` may contain:

- prompt asset definitions
- prompt versions
- prompt metadata
- optional release recommendations as non-canonical hints

V1 prompt bundles must not directly carry live project-scoped prompt releases as portable authoritative runtime state.

Project releases are deployment-local runtime choices per RFC 006.

### Canonical Prompt Artifact Kinds

V1 prompt-library bundles may contain these artifact kinds:

- `prompt_asset`
- `prompt_version`

They may also carry non-canonical advisory records such as release recommendations, but those must not replace the canonical artifact kinds above.

### Canonical `prompt_asset` Entry Payload

A `prompt_asset` artifact entry must use this payload shape:

- `name`
- `kind`
- `status`
- `library_scope_hint`
- `metadata`

Rules:

- `name` and `kind` must align with RFC 006 prompt-asset semantics
- `library_scope_hint` may be `tenant` or `workspace`
- this payload defines portable library identity, not deployment-local runtime release state

### Canonical `prompt_version` Entry Payload

A `prompt_version` artifact entry must use this payload shape:

- `prompt_asset_logical_id`
- `version_number`
- `format`
- `content`
- `metadata`

Rules:

- `prompt_asset_logical_id` must point to the logical identity of the corresponding `prompt_asset` artifact
- `version_number` is portable version ordering inside the bundle lineage, not a license to mutate an existing version in place
- `content_hash` on the outer artifact entry remains the canonical integrity and dedup key
- project release data must not be embedded here as if it were portable authoritative runtime state

### Prompt Advisory Payloads

Bundles may optionally include non-canonical prompt advisory payloads such as:

- `release_recommendation`
- `import_target_hint`

These must be clearly typed as advisory and must not be treated as runtime truth on import.

### Materialization Rule

When imported:

- prompt bundle contents materialize into tenant- or workspace-scoped prompt assets and prompt versions
- any project release creation is a separate explicit import option or post-import action

This keeps portable prompt content separate from runtime deployment state.

## Curated Knowledge Pack Bundle

### Contents

A `curated_knowledge_pack_bundle` may contain:

- knowledge-pack metadata
- curated documents or document references
- structured metadata
- chunk or chunking hints where useful
- provenance metadata
- optional retrieval hints

V1 knowledge-pack bundles are for curated knowledge import/export, not arbitrary full raw datastore export.

### Canonical Curated Knowledge Artifact Kinds

V1 curated-knowledge-pack bundles may contain these artifact kinds:

- `knowledge_pack`
- `knowledge_document`

### Canonical `knowledge_pack` Entry Payload

A `knowledge_pack` artifact entry must use this payload shape:

- `name`
- `description`
- `target_scope_hint`
- `metadata`

Rules:

- `target_scope_hint` may indicate the intended project or corpus style, but it is advisory and must not override explicit import target selection
- this payload defines the portable curated pack identity, not the final ingested runtime state

### Canonical `knowledge_document` Entry Payload

A `knowledge_document` artifact entry must use this payload shape:

- `knowledge_pack_logical_id`
- `document_name`
- `source_type`
- `content`
- `metadata`
- `chunk_hints` where available
- `retrieval_hints` where available

`source_type` in v1 must use one of these canonical values:

- `text_plain`
- `text_markdown`
- `text_html`
- `json_structured`
- `external_ref`

Rules:

- `knowledge_pack_logical_id` must point to the logical identity of the corresponding `knowledge_pack` artifact
- `source_type` describes the canonical portable representation used in the bundle, not every possible source parser format from the originating system
- `content` is the portable document body or portable structured representation, not a pointer to live runtime state
- `chunk_hints` and `retrieval_hints` are advisory inputs to ingest, not a substitute for owned ingestion and retrieval processing
- document-level provenance must remain visible even if chunk-level dedup occurs during import

### Chunk Portability Rule In V1

In v1, chunk-level data in curated knowledge packs is portable only as advisory hints, not as canonical retrieval state.

That means:

- bundles may carry `chunk_hints` and retrieval-oriented hints
- import may use those hints to improve ingest consistency
- the receiving system still owns final chunking, deduplication, and retrieval indexing decisions
- imported knowledge must remain valid even if chunk boundaries are re-derived during import

This keeps portable bundles stable across product versions without turning chunk layout into a long-term compatibility surface.

### Canonical Inline Content Forms

If `source_type` is inline rather than external, `content` must use one of these canonical forms:

- `text_plain`
  - `content.kind = inline_text`
  - `content.text`
- `text_markdown`
  - `content.kind = inline_text`
  - `content.text`
- `text_html`
  - `content.kind = inline_text`
  - `content.text`
- `json_structured`
  - `content.kind = inline_json`
  - `content.value`

Rules:

- `inline_text` is the canonical v1 form for portable textual bodies
- `inline_json` is the canonical v1 form for portable structured document content
- exporters must normalize source-specific representations into one of these inline forms or into the canonical external-reference form below
- bundle producers must not invent additional inline content kinds in v1

### Source-Type Normalization Rule

The portable bundle contract is stricter than the full ingest/parser surface.

If an originating document type does not map cleanly to one of the canonical v1 `source_type` values:

- the exporter must normalize it into one of the canonical inline forms, or
- export it as `source_type = external_ref` using the canonical external-reference form

This keeps the portable artifact contract stable even while the retrieval system evolves to support more document parsers over time.

### Binary and External Content Rule

V1 canonical bundle entries should prefer portable UTF-8 JSON-safe content representations.

If content is too large or not naturally inline text:

- the bundle may carry a structured external content reference in `content`
- that reference must still participate in provenance and conflict handling

The canonical external-reference form is:

- `content.kind = external_ref`
- `content.ref_type`
- `content.uri`
- `content.media_type` where known
- `content.sha256` where known
- `content.bytes` where known

Rules:

- when `content.kind = external_ref`, `source_type` must be `external_ref`
- `ref_type` in v1 may be `file`, `object_store`, or `url`
- `uri` identifies the external content location or retrieval handle
- `sha256` is the canonical integrity field when external content is referenced
- `bytes` is advisory size metadata only
- external references must still preserve the same artifact identity, provenance, and reconciliation semantics as inline content
- import must not silently dereference remote content outside the operator-approved import flow

V1 does not require binary-perfect embedding of every possible source type inside the first structured bundle format.

### Materialization Rule

When imported:

- knowledge-pack contents materialize into project corpora or other explicitly selected knowledge targets
- ingest into owned retrieval pipelines is still required
- chunk dedup may happen during ingest, but document-level provenance must remain visible

## Canonical Import Service Contract

The system may expose import through multiple entrypoints, but the import service contract is one thing.

### Phases

V1 import should be modeled as:

1. validate
2. plan
3. apply
4. report

### Validate

Validation checks:

- bundle schema version
- bundle type
- required fields
- supported artifact kinds
- scope compatibility
- target selection completeness

### Plan

Planning produces an import plan without mutating product state.

The plan must classify each artifact as one of:

- `create`
- `reuse`
- `update`
- `skip`
- `conflict`

The plan must include reasons for each classification.

### Apply

Apply executes the approved plan and materializes scoped product state.

### Report

The final report must include:

- requested target scope
- import actor
- bundle identity
- per-artifact outcome
- created/reused/updated/skipped/conflicted counts
- links to created or reused product objects where possible

## Canonical Reconciliation Rules

### Prompt Reconciliation

Prompt import reconciliation must follow RFC 012 and RFC 006.

Canonical order:

1. explicit import identifier match when present
2. otherwise logical identity plus target scope
3. content hash comparison

Rules:

- same logical identity plus same content hash in the same target scope -> `reuse`
- same logical identity plus different content hash in the same target scope -> `update` by creating a new prompt version
- same logical identity with incompatible target rules or conflicting intent -> `conflict`
- missing logical identity in target scope -> `create`

Prompt import must never silently mutate an existing prompt version.

### Knowledge Pack Reconciliation

Canonical order:

1. explicit import identifier match when present
2. otherwise source identity plus target scope
3. content hash comparison

Rules:

- same source identity and same content hash in same target scope -> `reuse`
- same source identity with changed content -> `update` through a new ingest/update event
- duplicate chunks may be deduplicated internally, but document-level import outcome must still be recorded
- ambiguous ownership or metadata collisions -> `conflict`

### Skip Rule

`skip` is allowed only when:

- the operator explicitly chooses not to import a valid artifact
- an artifact is intentionally excluded by target selection rules

`skip` must not be used as a silent substitute for conflict handling.

## Export Contract

Export must also use one canonical service model.

### V1 Export Sources

V1 should support exporting:

- tenant/workspace prompt libraries
- curated project knowledge packs

V1 does not need to support exporting every internal runtime object.

### Export Requirements

Export must:

- emit the canonical bundle envelope
- preserve identity/provenance metadata
- avoid leaking unrelated secrets or credentials
- explicitly declare omitted runtime-local state

## Secrets and Sensitive Data Rules

Bundles must not embed:

- provider credentials
- channel credentials
- source connection secrets
- operator auth secrets

If an artifact depends on external secrets or connections, the bundle may include:

- metadata references
- required capability declarations
- setup warnings

But not secret material itself.

## Scope Rules

Import/export must respect RFC 008.

### Allowed Portable Scope Shapes

V1 portable bundles may represent content originating from:

- tenant scope
- workspace scope
- project-scoped curated knowledge targets where explicitly supported

### Runtime State Exclusion

The portable bundle contract must not be used as an implicit runtime checkpoint export.

Excluded from this RFC:

- sessions
- runs
- tasks
- approvals
- checkpoints
- mailbox messages
- provider credentials
- live project prompt releases as authoritative deployment state

Those may need other backup/migration mechanisms later, but they are not part of the v1 portable artifact contract.

## Operator Surfaces

The control plane must be able to show:

- bundle metadata
- import plan preview
- per-artifact reconciliation decisions
- final outcome report
- provenance for imported artifacts

This does not need deep polish in v1, but it must be inspectable.

## API Contract Implications

V1 should expose one canonical import/export API model, even if exact route names are finalized later.

At minimum the model should support:

- validate bundle
- preview import plan
- apply import plan
- export selected artifacts into a bundle
- fetch import/export report

The API must return structured per-artifact outcomes, not only aggregate success/failure.

## Non-Goals

For v1, do not optimize for:

- arbitrary backup/export of the full database
- every historical artifact family
- binary-perfect round-trip of all runtime state
- cross-product marketplace packaging
- live secret transport inside bundles

The goal is a safe, inspectable, portable artifact contract for the highest-value product content.

## Open Questions

1. Which later trust and verification features, if any, should be layered on top of the v1 hash-based integrity model?

## Decision

Proceed assuming:

- v1 gets a dedicated import/export contract
- prompt libraries and curated knowledge packs are the first-class structured bundle families
- one physical JSON bundle format is canonical in v1
- one canonical bundle envelope, provenance model, and reconciliation model is required
- import/export remains separate from full runtime-state backup and restore
- chunk-level bundle data is advisory and re-derivable rather than canonical retrieval state
- content hashing is the canonical v1 integrity mechanism; signed bundle envelopes are additive later features
