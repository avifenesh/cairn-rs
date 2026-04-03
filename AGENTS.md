# cairn-rs Agent Rules

This repo is the execution workspace for the Rust rewrite of Cairn.

Use it to implement the product defined by the RFC set in [`docs/design/rfcs`](./docs/design/rfcs).

Do not treat this repo as an open-ended playground.

## Source Of Truth Order

When there is ambiguity, resolve in this order:

1. the relevant RFCs under [`docs/design/rfcs`](./docs/design/rfcs)
2. [`docs/design/EIGHT_WORKER_EXECUTION_PLAN.md`](./docs/design/EIGHT_WORKER_EXECUTION_PLAN.md)
3. compatibility docs under [`docs/design`](./docs/design)
4. the current Go implementation in [`../cairn`](../cairn) only where preserved behavior or fixtures need to be checked

If the docs disagree, fix the docs before inventing local behavior.

## Core Project Rules

- one codebase and one product binary
- local mode and self-hosted team mode are first-class in v1
- managed cloud and hybrid are later motions, not v1 foundations
- do not introduce a separate enterprise architecture fork
- do not move canonical runtime truth into `glide-mq`, queues, plugins, or transient workers
- do not bypass tenant/workspace/project scoping
- do not add hidden product forks through provider-native flags or entitlement-only config tricks
- do not re-open preserved route or SSE contracts casually

## Important Product Gotchas

- one-binary all-in-one execution is a supported convenience path, not the canonical team-mode production recommendation
- SSE replay minimum for the first sellable release is 72 hours
- workspace-level cross-project operational bulk actions are forbidden in v1
- tenant-level roll-up views are read-only for operational actions in v1
- owned retrieval replaces convenience external KB dependence in core flows
- chunk-level portability is advisory; receiving systems re-derive final chunking/indexing
- the first paid expansion after `team_self_hosted` is a narrow governance/compliance package
- early entitlement UX is inspection-oriented, not a purchasing or provisioning console

## Worker Coordination Rules

- each worker should primarily edit their owned surfaces from the execution plan
- before substantial work, update your worker mailbox file with current focus and blockers
- when you need another worker, leave a concise note in their mailbox file
- when a dependency lands, acknowledge it in your mailbox and update your next merge target
- if a change crosses worker boundaries, note the affected RFC and the reason in the mailbox entry

Use:

- [`docs/design/EIGHT_WORKER_EXECUTION_PLAN.md`](./docs/design/EIGHT_WORKER_EXECUTION_PLAN.md)
- [`docs/design/MILESTONE_BOARD_WEEKS_1_4.md`](./docs/design/MILESTONE_BOARD_WEEKS_1_4.md)
- [`docs/design/REPO_SCAFFOLDING_TASKS.md`](./docs/design/REPO_SCAFFOLDING_TASKS.md)
- [`.coordination/mailbox`](./.coordination/mailbox)
- [`.coordination/queue`](./.coordination/queue)

## Coordination Conventions

- append new mailbox notes at the top of the relevant section
- keep notes short and operational
- reference RFCs and files directly
- prefer "blocked by", "needs from", and "ready for review" language over long narrative updates
- use the queue bus for short active-task pacing, not for durable project truth
- queue multiple follow-on tasks when possible so workers can keep moving without waiting for a fresh manager pass

## When To Stop And Escalate

Stop and update the docs first if:

- an RFC is not specific enough to keep two workers aligned
- a preserved compatibility surface needs to break
- a worker wants to add a new runtime truth source
- a commercial/entitlement surface changes core product behavior
- a managed/hybrid assumption starts leaking into v1 implementation

## What To Keep Out Of Early Execution

- long-tail integrations
- marketplace work
- billing workflows
- in-product purchasing
- managed cloud operations
- enterprise admin sprawl beyond the agreed first paid package boundary

## Reference Docs

- [Rewrite Plan](./docs/design/RUST_PRODUCT_REWRITE_PLAN.md)
- [8-Worker Execution Plan](./docs/design/EIGHT_WORKER_EXECUTION_PLAN.md)
- [Milestone Board Weeks 1-4](./docs/design/MILESTONE_BOARD_WEEKS_1_4.md)
- [Repo Scaffolding Tasks](./docs/design/REPO_SCAFFOLDING_TASKS.md)
- [RFC Index](./docs/design/rfcs/README.md)
