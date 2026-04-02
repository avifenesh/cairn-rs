# Cairn Rewrite RFCs

Status: active draft set  
Purpose: convert the rewrite plan into concrete architecture decisions that parallel workers can implement against

## Order

1. [RFC 001 - Product Boundary and Non-Goals](./001-product-boundary.md)
2. [RFC 002 - Runtime and Event Model](./002-runtime-event-model.md)
3. [RFC 003 - Owned Retrieval Replacing Bedrock KB](./003-owned-retrieval.md)
4. [RFC 004 - Graph and Eval Matrix Model](./004-graph-eval-matrix.md)

## How To Use These RFCs

- Treat these as draft decision documents, not final law.
- Resolve open questions before large implementation branches diverge.
- When an RFC changes, update the rewrite plan and any dependent RFCs.
- Prefer tightening scope over adding parallel half-systems.

## Decision Sequence

- RFC 001 defines what the product is.
- RFC 002 defines how the runtime behaves.
- RFC 003 defines how knowledge and retrieval become product-owned.
- RFC 004 defines how graph and eval systems become first-class product surfaces.

## Rule For Follow-On RFCs

No new RFC should contradict these without explicitly amending them.
