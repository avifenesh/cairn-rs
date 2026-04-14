# Fixture Harvesting Notes

These initial Phase 0 fixtures are seeded from:

- the local frontend contract in `../cairn-sdk/frontend/src/lib/api/client.ts`
- the local SSE client contract in `../cairn-sdk/frontend/src/lib/stores/sse.svelte.ts`
- the protocol notes in `../cairn-sdk/docs/design/FRONTEND_AGENT_BRIEF.md`

They should be treated as the first preserved-contract reference set.

Where live backend fixtures or direct handler captures become available later, these files may be tightened or replaced, but they should not drift casually because the UI compatibility contract depends on them.

## Provenance Levels

- `frontend_contract`
  - derived from concrete frontend route or SSE usage
- `protocol_doc`
  - reinforced by protocol/design docs in `../cairn-sdk`

The current seed fixtures generally use both.

## Current Constraint

The local `../cairn-sdk` checkout does not currently expose an obvious preserved Go
HTTP/SSE server implementation for the `/v1/*` surfaces Worker 1 is tracking.
Until direct handler-backed captures are available, the fixture set should stay
anchored to the frontend client plus protocol docs, with that upstream evidence
checked explicitly by script.
