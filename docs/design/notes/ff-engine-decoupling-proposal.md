# FF Engine decoupling — phase roadmap

Ship plan for moving cairn off direct FlowFabric internals. One trait
(`crates/cairn-fabric/src/engine/mod.rs::Engine`), one impl
(`ValkeyEngine`), and a shrinking list of "not in scope (yet)" items
that shrinks by one phase per PR.

## Phase A — trait skeleton & `ValkeyEngine` (shipped, PR #72)

- `Engine` trait with `describe_execution`, `describe_flow`,
  `describe_edge`, `list_incoming_edges`.
- `ValkeyEngine` as the one impl — holds `ferriskey::Client`, does
  the `HGETALL` / `SMEMBERS` work, parses into typed snapshot structs.
- Every cairn-side read of FF state flipped to go through the trait.

## Phase B — targeted read (`get_execution_tag`) (shipped, PR #72)

- Added alongside Phase A to avoid the N+1 amplification that a
  full `describe_execution` would cause in loops like
  `check_dependencies`'s per-blocker resolve.

## Phase C — tag writes (shipped, this PR)

Goal: cairn services stop calling `ferriskey::Client::hset` on
FF-owned hashes.

Trait surface (3 methods):

| Method | Target hash | Use site |
|--------|------------|----------|
| `set_flow_tag(&FlowId, key, value)` | `fctx.core()` | `FabricSessionService::archive` |
| `set_flow_tags(&FlowId, &BTreeMap)` | `fctx.core()` | `FabricSessionService::create` (bulk — `cairn.project` + `cairn.session_id` in one round-trip) |
| `set_execution_tag(&ExecutionId, key, value)` | `ExecKeyContext::tags()` | (no caller in this phase; provided for Phase D/E symmetry + tested end-to-end) |

Invariants:

- **Namespace guard**: key must match `^[a-z][a-z0-9_]*\.`. Prevents
  collision with FF's own hash fields (which have no `.`). Rejected
  as `FabricError::Validation` at the trait boundary.
- **All-or-nothing bulk**: `set_flow_tags` validates every key before
  issuing the wire call; a partial write is never visible.
- **Empty map is a no-op**: `set_flow_tags` returns `Ok(())` without
  issuing a wire call.

### Scope exception — `instance_tag_backfill`

The one-shot backfill in `crates/cairn-fabric/src/instance_tag_backfill.rs`
keeps its direct `HSET cairn.instance_id` and is NOT routed through
the trait. Reason: it operates on raw `ff:exec:*:tags` scan-key
strings, not typed `ExecutionId`s. A trait method that took a raw
key would re-expose the Valkey key layout we're trying to hide.
Parsing the UUID back out of the scan key just to hand it to a trait
that re-derives the same key is pointless ceremony. The backfill is
a finite-lifetime migration utility gated on
`CAIRN_BACKFILL_INSTANCE_TAG=1` and will be removed once the
pre-filter fleet is fully aged out.

### Why no clippy `disallowed-methods` lint

A workspace-wide `disallowed-methods` entry on `ferriskey::Client::hset`
would flag cairn-owned keyspaces too (`worker_service`,
`quota_service`, `boot::seed_waitpoint_hmac_secret_if_configured`).
Those aren't layering violations — cairn owns those keys end-to-end.
The lint would either produce noise or require 3-4 `#[allow(...)]`
escape hatches, neither of which is a win. The module-docs in
`engine/mod.rs` pledge enforcement via code review. A future tight
lint is tracked as a follow-up in
`~/.claude/projects/-home-ubuntu/memory/project_ff_decoupling_followups.md`.

## Phase D — FCALL ARGV pre-reads (not started)

~12 `hget ctx.core(), "current_attempt_id"` sites in cairn's FCALL
build paths. Moves to `engine.read_current_attempt(&ExecutionId)` so
the Valkey layout isn't leaked into FCALL arg construction. Blocked
on nothing; ship when the Phase C dust settles.

## Phase E — typed error model (not started)

FCALL errors today arrive as `ferriskey::Value` envelopes parsed by
`helpers::check_fcall_success` et al. Phase E introduces a typed
`EngineError` enum returned by the trait, absorbing the parse step
into the engine boundary. Aligns cairn with RFC-012 Stage 1a, which
introduced `ff_core::EngineError` on the FF side.

## Phase F — swap-in upstream `describe_*` primitives

When FlowFabric#58 ships, `ValkeyEngine` shrinks to ~30 lines of
delegation; the typed snapshot structs become re-exports from the
`ff` umbrella crate. No caller-visible change.
