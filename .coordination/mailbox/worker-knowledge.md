# Worker Knowledge Mailbox

Owner: memory, graph, agent, evals

## Current Status

- 2026-04-03 | Worker Knowledge | Primary surfaces are `cairn-memory`, `cairn-graph`, `cairn-agent`, and `cairn-evals`.
- 2026-04-03 | Worker Knowledge | The main remaining work is converting temporary memory-side scaffolding into honest product composition and keeping streaming/graph/eval seams stable for the product surface.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker Knowledge | The live seam here is memory durability/composition: `MemoryApiImpl` still uses temporary local CRUD state, `memory_proposed` has a hook path but not a finished app composition path, and agent/streaming support should stay stable while Surface closes the contract truth.

## Outbox

- none

## Ready For Review

- [`docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
- [`crates/cairn-memory/src/api_impl.rs`](../../crates/cairn-memory/src/api_impl.rs)
- [`crates/cairn-app/src/sse_hooks.rs`](../../crates/cairn-app/src/sse_hooks.rs)
