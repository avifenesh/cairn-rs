# Worker 2 Mailbox

Owner: Domain, State Machines, Shared Types

## Current Status

- 2026-04-03 | Week 1 assigned | Scaffold `cairn-domain` and lock base IDs, commands, events, tenancy, lifecycle, and policy module boundaries.
- 2026-04-03 | Worker 2 / Manager | `cairn-domain` scaffold complete | Stable ID newtypes, tenancy keys, lifecycle enums/helpers, policy verdict types, and base runtime command/event envelopes are in repo with passing crate tests.
- 2026-04-03 | Week 2 assigned | Lock the minimal runtime-critical shared contract set needed by Workers 3, 4, and 5. Publish one review-ready cut, then stop expanding surface area.

## Blocked By

- none

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 2 | Week 1 focus: domain crate scaffold and base shared contracts that all other workers will depend on.
- 2026-04-03 | Worker 1 -> Worker 2 | Priority order: IDs, tenancy keys, lifecycle enums, command/event shells, policy types. Publish stable boundaries before adding depth.
- 2026-04-03 | Worker 1 / Manager -> Worker 2 | Week 2 priority order: (1) canonical command/event shapes for session/run/task/approval/checkpoint/mailbox state advancement, (2) shared command-validation/runtime conflict error enums, (3) external-worker reporting and lease-facing shared types, (4) any tool-invocation shared records Worker 5 needs. Keep this contract narrow and publish review-ready boundaries early so Worker 3/4/5 can code against them.

## Outbox

- 2026-04-03 | Worker 2 -> Worker 3 | `cairn-domain` now exposes stable ownership keys, lifecycle enums, and event/command envelopes. You can start store and sync-projection interfaces against those types.
- 2026-04-03 | Worker 2 -> Worker 4 | Session/run/task/checkpoint lifecycle enums and pause/resume/failure helpers are ready to consume from `cairn-domain`.
- 2026-04-03 | Worker 2 -> Worker 5 | Policy verdicts and execution-class enums are in `cairn-domain`; use those instead of inventing tool-isolation gating types locally.
- 2026-04-03 | Worker 2 -> Worker 7 | Prompt/provider/runtime shared IDs are stable in `cairn-domain`; eval and prompt crates can depend on those IDs immediately.
- 2026-04-03 | Worker 2 -> Worker 8 | API envelopes can now reference stable command/event IDs, ownership keys, and lifecycle states from `cairn-domain`.
- 2026-04-03 | Worker 2 -> Worker 3/4/5 | Next review-ready cut will lock minimal runtime-critical domain contracts. Treat anything beyond that cut as unstable until announced here.

## Ready For Review

- 2026-04-03 | Worker 2 | Review `crates/cairn-domain/*` for Week 1 shared-contract scaffold: IDs, tenancy keys, lifecycle helpers, policy types, and base runtime command/event envelopes.
