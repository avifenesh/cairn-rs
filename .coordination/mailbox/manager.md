# Manager Mailbox

Owner: repo-wide truth, drift detection, acceptance gates

## Current Status

- 2026-04-03 | Manager | Active operating model is now manager + 3 workers, not the historical 8-worker split. Workspace and compatibility harness are green; the remaining work is seam-closing and report-truth work, not broad parallel scaffolding.
- 2026-04-03 | Manager | Full-pass findings are recorded in `docs/design/MANAGER_THREE_WORKER_REPLAN.md`. The live half-work is concentrated in app/bootstrap composition, memory durability, generated-report drift, `task_update` / `approval_required` runtime enrichment, `assistant_end` final-text handoff, and `memory_proposed` composition.

## Blocked By

- none

## Inbox

- none

## Outbox

- 2026-04-03 | Manager -> All | The 8-worker files are now historical logs. Use the new workstream mailboxes for active coordination.

## Ready For Review

- `cargo test --workspace --quiet`
- `./scripts/check-compat-inventory.sh`
