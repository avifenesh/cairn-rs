# cairn-rs

Planning workspace for the Rust rewrite of Cairn.

This repo is intentionally separate from the current Go implementation in [`../cairn`](../cairn).  
Use `cairn` as the reference system for:

- current behavior
- route and SSE semantics
- runtime concepts
- migration fixtures
- scope inventory

Use this repo for:

- rewrite planning
- RFCs
- architecture decisions
- future Rust workspace scaffolding

## Current Docs

- [Rust Product Rewrite Plan](./docs/design/RUST_PRODUCT_REWRITE_PLAN.md)
- [8-Worker Execution Plan](./docs/design/EIGHT_WORKER_EXECUTION_PLAN.md)
- [Milestone Board Weeks 1-4](./docs/design/MILESTONE_BOARD_WEEKS_1_4.md)
- [Repo Scaffolding Tasks](./docs/design/REPO_SCAFFOLDING_TASKS.md)
- [RFC Index](./docs/design/rfcs/README.md)

## Coordination

- [AGENTS.md](./AGENTS.md)
- [Mailbox](./.coordination/mailbox)

## Relationship to Cairn

`cairn-rs` is not a line-by-line port target.

The goal is to build a cleaner, product-grade Rust implementation that preserves the right semantics from Cairn while fixing architecture, ownership boundaries, and product shape.
