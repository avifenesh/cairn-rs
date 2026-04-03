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
- first harvested fixtures for the minimum Phase 0 set
