# Migration Harness Layout

This directory is Worker 1's compatibility and migration harness root.

## Goals

- keep preserved and transitional surfaces executable
- compare Rust behavior against fixture expectations harvested from `../cairn`
- give runtime/API workers a concrete contract instead of prose-only compatibility notes

## Layout

- `http_routes.tsv`
  - machine-readable inventory of UI-referenced HTTP routes
- `sse_events.tsv`
  - machine-readable inventory of UI-referenced SSE event names and minimum payload contracts
- `phase0_required_http.txt`
  - minimum Phase 0 request shapes that must have fixtures or executable coverage
- `phase0_required_sse.txt`
  - minimum Phase 0 SSE event set
- `../fixtures/http`
  - request/response fixture payloads
- `../fixtures/sse`
  - SSE payload fixtures
- `../fixtures/migration`
  - migration-comparison inputs and reports
- `phase0_http_fixture_map.tsv`
  - required preserved Phase 0 HTTP surfaces mapped to current fixture files
- `phase0_sse_fixture_map.tsv`
  - required preserved Phase 0 SSE surfaces mapped to current fixture files

## Naming Conventions

HTTP fixture files should prefer:

- `<METHOD>__<route-shape>__<scenario>.json`

Examples:

- `GET__v1_feed__limit20_unread_true.json`
- `POST__v1_assistant_message__with_session.json`

SSE fixture files should prefer:

- `<event-name>__<scenario>.json`

Examples:

- `feed_update__single_item.json`
- `assistant_delta__incremental_reply.json`

## First Worker 1 Deliverables

- inventory files above
- fixture directory layout
- executable inventory checker in `scripts/check-compat-inventory.sh`
- executable Rust-side API catalog sync test in `crates/cairn-api/tests/compat_catalog_sync.rs`
- first harvested fixtures for the minimum Phase 0 set
- generated `tests/fixtures/migration/phase0_mismatch_report.md`
- generated `tests/fixtures/migration/phase0_upstream_contract_report.md`

## Current Worker 1 Constraint

The local `../cairn` checkout currently provides preserved-route and SSE evidence
primarily through:

- frontend API usage in `frontend/src/lib/api/client.ts`
- frontend SSE usage in `frontend/src/lib/stores/sse.svelte.ts`
- protocol docs in `docs/design/FRONTEND_AGENT_BRIEF.md`
- protocol tables in `docs/design/pieces/09-server-protocols.md`

Worker 1 should prefer direct backend captures when they exist, but should not
block Phase 0 waiting for them. Until a concrete legacy server surface is
available locally, the compatibility harness treats the frontend + protocol docs
as the preserved upstream contract and validates against those sources
explicitly.
