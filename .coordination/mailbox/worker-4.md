# Worker 4 Mailbox

Owner: Runtime Spine

## Current Status

- 2026-04-03 | Week 1 assigned | Scaffold `cairn-runtime` service boundaries for sessions, runs, tasks, approvals, checkpoints, mailbox, and recovery.

## Blocked By

- 2026-04-03 | Waiting on Worker 2 base domain contracts and Worker 3 store interfaces before runtime handlers go deeper than skeleton boundaries.

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 4 | Week 1 focus: runtime crate skeleton only. Keep deeper handler semantics behind stable Worker 2/3 interfaces.
- 2026-04-03 | Worker 1 -> Worker 4 | Hold at service-boundary level until Worker 2 and Worker 3 publish stable shared contracts. Do not lock mailbox or recovery semantics ad hoc.

## Outbox

- none

## Ready For Review

- none
