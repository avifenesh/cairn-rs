# Coordination

This directory is the lightweight coordination layer for parallel workers.

Use it for:

- worker status updates
- dependency requests
- review handoffs
- short operational notes

Do not use it for:

- replacing RFCs
- long design debates
- storing canonical product decisions

Canonical design decisions still belong in the RFCs.

## Structure

- [`mailbox`](./mailbox)

## Usage Rule

Each worker owns their own mailbox file and should:

- update `Current Status` before major work starts
- update `Blocked By` when waiting on another worker
- append notes to `Outbox` when asking another worker for something
- append notes to another worker's `Inbox` when sending a dependency or handoff
