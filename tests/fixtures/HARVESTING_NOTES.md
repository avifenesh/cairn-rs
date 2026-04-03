# Fixture Harvesting Notes

These initial Phase 0 fixtures are seeded from:

- the local frontend contract in `../cairn/frontend/src/lib/api/client.ts`
- the local SSE client contract in `../cairn/frontend/src/lib/stores/sse.svelte.ts`
- the protocol notes in `../cairn/docs/design/FRONTEND_AGENT_BRIEF.md`

They should be treated as the first preserved-contract reference set.

Where live backend fixtures or direct handler captures become available later, these files may be tightened or replaced, but they should not drift casually because the UI compatibility contract depends on them.

## Provenance Levels

- `frontend_contract`
  - derived from concrete frontend route or SSE usage
- `protocol_doc`
  - reinforced by protocol/design docs in `../cairn`

The current seed fixtures generally use both.
