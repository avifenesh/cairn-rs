# Queue Bus

This directory is the lightweight task queue and notification bus for worker coordination.

It does not replace:

- RFCs
- mailbox files
- canonical product decisions

Use it for short-lived execution pacing:

- manager queues follow-on tasks for workers
- workers claim and complete queued tasks
- listeners print notifications so workers and manager do not have to poll manually

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

## Main Scripts

- `scripts/coordination/init-queue.sh`
- `scripts/coordination/start-listeners.sh`
- `scripts/coordination/stop-listeners.sh`
- `scripts/coordination/listener-status.sh`
- `scripts/coordination/queue-worker-tasks.sh`
- `scripts/coordination/show-worker-queue.sh`
- `scripts/coordination/worker-claim-next.sh`
- `scripts/coordination/worker-complete-task.sh`
- `scripts/coordination/worker-listen.sh`
- `scripts/coordination/manager-listen.sh`

## Typical Flow

Manager:

```bash
./scripts/coordination/start-listeners.sh --all
./scripts/coordination/queue-worker-tasks.sh worker-8 \
  "Extend composed app coverage to one feed path" \
  "Harden assistant_end SSE assembled text"
```

Worker:

```bash
./scripts/coordination/worker-listen.sh worker-8
./scripts/coordination/worker-claim-next.sh worker-8
./scripts/coordination/worker-complete-task.sh worker-8 <task-id> --note "done in cairn-api tests"
```

Manager monitor:

```bash
./scripts/coordination/listener-status.sh
tail -f .coordination/queue/state/listeners/logs/manager.log
```
