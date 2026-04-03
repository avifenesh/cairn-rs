# Worker 1 Mailbox

Owner: Contracts, Fixtures, Migration Harness

## Current Status

- 2026-04-03 | Worker 1 / Manager | Inventory + harness layout complete | Executable compatibility inventory, fixture directory layout, and migration harness structure are in repo. Next step is harvesting first golden fixtures.

## Blocked By

- none

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 1 | Week 1 focus: `tests/compat`, `tests/fixtures`, preserved route/SSE fixture naming, and initial migration harness shape.

## Outbox

- 2026-04-03 | Worker 1 / Manager -> Worker 2 | Week 2 target: publish the narrow runtime-critical domain cut for session/run/task/approval/checkpoint/mailbox advancement, shared error enums, external-worker reporting, and any required tool-invocation shared records. Land this cut early for Worker 3/4/5.
- 2026-04-03 | Worker 1 -> Worker 2 | Deliver stable base IDs, command/event enums, tenancy keys, and lifecycle types first. Worker 4, 5, 7, and 8 will build on those boundaries immediately.
- 2026-04-03 | Worker 1 -> Worker 3 | Deliver migration layout, event-log interfaces, and sync-projection boundaries early. Worker 4, 6, and 8 are blocked on store shape drifting.
- 2026-04-03 | Worker 1 -> Worker 4 | Stay at runtime service-boundary level until Worker 2/3 shared contracts settle. Avoid locking handler semantics locally.
- 2026-04-03 | Worker 1 -> Worker 5 | Keep tool/plugin work at interface level this week. Do not invent invocation/event shapes outside RFC 007 + Worker 2 shared types.
- 2026-04-03 | Worker 1 -> Worker 6 | Align retrieval/graph persistence assumptions with Worker 3 before implementing storage semantics. Use RFC 003/004/013 as hard contract.
- 2026-04-03 | Worker 1 -> Worker 7 | Keep prompt/eval/agent skeletons aligned to RFC 004 and RFC 006. Do not infer rollout or scorecard semantics from convenience behavior.
- 2026-04-03 | Worker 1 -> Worker 8 | Prioritize preserved API/SSE shell shape and bootstrap boundary only. Do not let operator/backend details outrun runtime/store contracts.

## Ready For Review

- 2026-04-03 | Worker 1 | Review `tests/compat/*`, `tests/fixtures/*`, and `scripts/check-compat-inventory.sh` for phase-0 compatibility harness baseline.
