# glide-mq Residual Scope During Rust Rewrite

Status: draft  
Purpose: classify what glide-mq may still do during the Rust rewrite and what it must not own

## Canonically Rust-Owned

glide-mq must not be the source of truth for:

- session state
- run state
- task state
- approvals
- checkpoints
- mailbox durability
- recovery state
- lease/heartbeat truth
- runtime dedup decisions

These belong to the Rust runtime and store.

## Transitional Uses Allowed

glide-mq may remain temporarily for:

- async dispatch transport
- queueing substrate for selected background workloads
- stream/fanout transport where helpful
- compatibility bridging for selected current flows
- health probing for the transitional sidecar itself
- temporary hot-cache support for TTL-style ephemeral memory projections

These uses are acceptable only if the Rust runtime remains canonical.

## Explicitly Non-Canonical Even If Temporarily Used

If glide-mq is still involved in these areas, it must be transport-only:

- mailbox delivery
- task dispatch
- worker fanout
- event stream transport

Any durable truth about those must be persisted in Rust-owned state.

## Removed or Replaced Responsibilities

The rewrite should replace sidecar ownership of:

- mailbox persistence
- checkpoint persistence
- heartbeat/lease truth
- runtime dedup truth

with Rust-native implementations.

## Current Reference Inventory

The current `../cairn-sdk` reference uses sidecar-facing code for:

- `/health` probing
- queue job enqueue and queue counts
- SSE stream subscription and replay via `Last-Event-ID`
- mailbox routes
- heartbeat registry routes
- dedup check routes
- TTL memory cache routes

This inventory exists to stop workers from guessing what “residual scope” means.

## Residual Capability Table

| Capability | Allowed in glide-mq during migration? | Canonical owner |
|---|---|---|
| Sidecar health probe/status | Yes | Rust control plane deployment health model |
| Task queue transport | Yes | Rust runtime/store |
| Queue counts/status hints | Yes, advisory only | Rust runtime/store |
| Event fanout / stream transport | Yes | Rust runtime/event log |
| SSE reconnect helper / `Last-Event-ID` bridge | Yes | Rust runtime/event log |
| Mailbox persistence | No | Rust runtime/store |
| Mailbox delivery transport | Transitional only | Rust runtime/store |
| Checkpoint persistence | No | Rust runtime/store |
| Lease heartbeat truth | No | Rust runtime/store |
| Heartbeat registry endpoint | No | Rust runtime/store |
| Runtime dedup truth | No | Rust runtime/store |
| Sidecar dedup check helper | No for canonical runtime decisions | Rust runtime/store |
| TTL memory hot cache | Yes, non-canonical only | Rust memory/store |
| Background job dispatch helper | Yes | Rust runtime/store |
| Compatibility wrapper for legacy async flows | Yes | Rust runtime/store |

## Open Questions

1. Which specific current flows still justify temporary glide-mq transport use in v1 beyond queue dispatch and SSE fanout?
2. Should stream fanout remain in glide-mq longer than queue dispatch, or should both be migrated together?
