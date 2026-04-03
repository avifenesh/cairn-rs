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

## Queue Bus

For active pacing, use the queue bus in [`../queue`](../queue) with:

- `scripts/coordination/queue-worker-tasks.sh`
- `scripts/coordination/worker-claim-next.sh`
- `scripts/coordination/worker-complete-task.sh`
- `scripts/coordination/worker-listen.sh`
- `scripts/coordination/manager-listen.sh`
- `scripts/coordination/audit-completions.sh`
- dedicated long-lived shells for manager and worker listeners as the canonical operating mode

Queue completion rule:

- a worker must complete a queued task with at least one concrete `--proof` or one concrete `--blocker`
- generic completion notes are not enough
- manager can audit recent suspicious completions with `audit-completions.sh`

Mailboxes remain the durable coordination record. The queue bus is only for short-lived active task handoff and refill.

## Suggested Entry Format

`2026-04-03 | Worker 4 -> Worker 3 | Need sync projection shape for mailbox records before recovery patch lands.`
