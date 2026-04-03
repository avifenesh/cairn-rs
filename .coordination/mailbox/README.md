# Worker Mailbox Protocol

There is one mailbox file per worker:

- `worker-1.md` through `worker-8.md`

## Sections

- `Current Status`
- `Blocked By`
- `Inbox`
- `Outbox`
- `Ready For Review`

## Conventions

- prepend new entries near the top of a section
- keep each entry short
- include date and worker name
- reference RFCs, files, or PRs directly when possible
- move stale or resolved items into a short "resolved inline" note instead of letting files grow indefinitely

## Current Coordination Mode

Mailbox-first coordination is the active system right now.

- manager writes the next concrete cut directly into each worker mailbox
- workers report current status, blockers, handoffs, and ready-for-review notes in the mailbox
- do not rely on queue listeners, busywait refill, or auto-claim behavior for current execution

Completion rule:

- every finished cut should leave either concrete proof or a concrete blocker in the mailbox
- generic completion notes are not enough

The queue bus in [`../queue`](../queue) is paused and kept only as reference for a later redesign.

## Suggested Entry Format

`2026-04-03 | Worker 4 -> Worker 3 | Need sync projection shape for mailbox records before recovery patch lands.`
