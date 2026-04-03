# Coordination

This directory is the lightweight coordination layer for parallel workers.

Use it for:

- worker status updates
- dependency requests
- review handoffs
- short operational notes
- active task queueing and refill notifications via the queue bus

Do not use it for:

- replacing RFCs
- long design debates
- storing canonical product decisions

Canonical design decisions still belong in the RFCs.

## Structure

- [`mailbox`](./mailbox)
- [`queue`](./queue)

## Usage Rule

Each worker owns their own mailbox file and should:

- update `Current Status` before major work starts
- update `Blocked By` when waiting on another worker
- append notes to `Outbox` when asking another worker for something
- append notes to another worker's `Inbox` when sending a dependency or handoff

Manager and workers can also use the queue bus for short active-task pacing:

- manager queues multiple follow-on tasks at once
- workers claim and complete queued tasks
- background listeners print notifications so queue changes do not rely on manual polling
- the canonical listener posture is one long-lived manager shell plus one long-lived shell per worker listener
- `start-listeners.sh` is only a convenience helper for local use, not the default coordination contract
- queue tasks are not considered complete unless workers record concrete proof or a concrete blocker through `worker-complete-task.sh`

The queue bus is an execution assist, not the durable narrative layer. Mailboxes still carry the canonical status and dependency story.
