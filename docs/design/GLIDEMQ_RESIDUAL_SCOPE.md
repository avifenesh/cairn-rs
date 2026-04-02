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

## Residual Capability Table

| Capability | Allowed in glide-mq during migration? | Canonical owner |
|---|---|---|
| Task queue transport | Yes | Rust runtime/store |
| Event fanout / stream transport | Yes | Rust runtime/event log |
| Mailbox persistence | No | Rust runtime/store |
| Checkpoint persistence | No | Rust runtime/store |
| Lease heartbeat truth | No | Rust runtime/store |
| Runtime dedup truth | No | Rust runtime/store |
| Background job dispatch helper | Yes | Rust runtime/store |
| Compatibility wrapper for legacy async flows | Yes | Rust runtime/store |

## Open Questions

1. Which specific current flows still justify temporary glide-mq transport use in v1?
2. Should stream fanout remain in glide-mq longer than queue dispatch, or should both be migrated together?
