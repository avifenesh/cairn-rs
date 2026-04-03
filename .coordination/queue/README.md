# Queue Bus

This directory is historical task-queue and notification-bus material from the earlier 8-worker coordination experiment.

## Current Status

Queue automation is archived.

- do not use the queue bus as the active task system
- do not rely on listeners, busywait refill, or auto-claim behavior
- use the mailbox files in [`../mailbox`](../mailbox) as the current coordination system
- keep these scripts and files only as reference for a later redesign

It does not replace:

- RFCs
- mailbox files
- canonical product decisions

It was intended for short-lived execution pacing:

- manager queues follow-on tasks for workers
- workers claim and complete queued tasks
- listeners print notifications so workers and manager do not have to poll manually

Former listener posture:

- run `manager-listen.sh` from a dedicated long-lived manager shell
- run each `worker-listen.sh worker-<n>` from that worker's own long-lived shell
- treat `start-listeners.sh` as a local convenience helper, not the default control-shell flow

## Layout

- `tasks/worker-<n>/pending`
- `tasks/worker-<n>/claimed`
- `tasks/worker-<n>/done`
- `events/worker-<n>`
- `events/manager`
- `state`

## Rules

- queue items should be short operational tasks, not design debates
- mailbox files remain the durable narrative layer
- manager can queue multiple follow-on tasks at once
- workers should claim the next task before they go idle
- worker claim and completion scripts emit manager-facing events so refill can happen quickly
- if a worker claims the last pending task, manager gets `queue_empty` immediately
- if a worker completes work and nothing is pending, manager gets `queue_empty` again

## Main Scripts

- `scripts/coordination/init-queue.sh`
- `scripts/coordination/start-listeners.sh`
- `scripts/coordination/stop-listeners.sh`
- `scripts/coordination/listener-status.sh`
- `scripts/coordination/queue-worker-tasks.sh`
- `scripts/coordination/show-worker-queue.sh`
- `scripts/coordination/worker-claim-next.sh`
- `scripts/coordination/worker-complete-task.sh`
- `scripts/coordination/requeue-extra-claims.sh`
- `scripts/coordination/worker-listen.sh`
- `scripts/coordination/manager-listen.sh`

## Historical Flow

Manager:

```bash
./scripts/coordination/manager-listen.sh --interval 2
./scripts/coordination/queue-worker-tasks.sh worker-8 \
  "Extend composed app coverage to one feed path" \
  "Harden assistant_end SSE assembled text"
```

Worker:

```bash
./scripts/coordination/worker-listen.sh worker-8 --interval 2
./scripts/coordination/worker-claim-next.sh worker-8
./scripts/coordination/worker-complete-task.sh worker-8 <task-id> \
  --proof "patched crates/cairn-api/src/feed.rs" \
  --proof "cargo test -p cairn-api --test http_boundary_alignment"
```

If a task cannot be completed yet, workers must finish it as blocked with a concrete blocker:

```bash
./scripts/coordination/worker-complete-task.sh worker-8 <task-id> \
  --blocker "missing runtime-owned memory_proposed publisher seam in cairn-api/src/sse_publisher.rs" \
  --note "needs owner decision from Worker 6 or Worker 8"
```

If you want a detached local listener from an already long-lived shell:

```bash
nohup ./scripts/coordination/worker-listen.sh worker-8 --interval 2 \
  >> .coordination/queue/state/listeners/logs/worker-8.log 2>&1 &
```

Historical behavior:

- queuing a task emits a worker-facing `queued` event
- claiming a task removes it from `pending/` and moves it to `claimed/`
- `worker-claim-next.sh` now refuses to claim a second task while the worker already has one in `claimed/`, unless `--force` is used
- if that claim drains `pending/`, manager gets `queue_empty` immediately
- completing a task now requires at least one concrete `--proof` or `--blocker`
- generic notes like `done`, `no drift`, or `all tests green` are rejected
- completed or blocked tasks move from `claimed/` to `done/`
- `requeue-extra-claims.sh` can move extra claimed tasks back to `pending/` if a worker shell accidentally over-claims

Historical manager monitor:

```bash
./scripts/coordination/listener-status.sh
tail -f .coordination/queue/state/listeners/logs/manager.log
./scripts/coordination/audit-completions.sh --limit 20
```

These scripts remain in the repo as reference material for a future replacement, but they are not part of the active coordination contract.
