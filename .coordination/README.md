# Coordination

This directory is the lightweight coordination layer for parallel workers.

Use it for:

- worker status updates
- dependency requests
- review handoffs
- short operational notes
- manager-written next-task direction in mailbox files

Do not use it for:

- replacing RFCs
- long design debates
- storing canonical product decisions

Canonical design decisions still belong in the RFCs.

## Structure

- [`mailbox`](./mailbox)
- [`queue`](./queue)

Queue automation is currently paused.

- do not treat the queue bus as authoritative
- do not wait on listener or busywait automation
- use mailbox updates as the active coordination system
- keep the queue scripts only as reference for a later redesign

## Usage Rule

Each worker owns their own mailbox file and should:

- update `Current Status` before major work starts
- update `Blocked By` when waiting on another worker
- append notes to `Outbox` when asking another worker for something
- append notes to another worker's `Inbox` when sending a dependency or handoff

Active coordination rule:

- manager writes the next concrete cut directly into the worker mailbox
- workers update mailbox status/blockers/review handoffs directly
- mailboxes are the canonical status and dependency story
- if a worker finishes a cut, they should leave either concrete proof or a concrete blocker in the mailbox, not a generic completion note

The queue bus is paused and should not be used for current execution pacing.
