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

## Suggested Entry Format

`2026-04-03 | Worker 4 -> Worker 3 | Need sync projection shape for mailbox records before recovery patch lands.`
