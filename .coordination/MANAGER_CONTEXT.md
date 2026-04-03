# Manager Context

Status: active handoff and operating context  
Audience: current manager and next manager  
Updated: 2026-04-03

This file is the manager's compact source of truth for:

- product/spec constraints that should not be re-litigated casually
- what the codebase already proves
- what is still honestly unfinished
- how to manage the current execution phase without recreating churn

Use this together with:

- [`../AGENTS.md`](../AGENTS.md)
- [`../docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
- [`mailbox/manager.md`](./mailbox/manager.md)

## 1. Source Of Truth Order

When deciding whether something is open, closed, or drifted, use this order:

1. RFCs in [`../docs/design/rfcs`](../docs/design/rfcs)
2. [`../docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
3. executable tests and generated compatibility artifacts
4. mailbox history only as context, not as final truth
5. `../cairn` only for preserved behavior checks

Manager rule:

- if code, tests, generated reports, and mailbox prose disagree, treat mailbox prose as the least trustworthy layer

## 2. Product And Architecture Constraints That Are Settled

These are stabilized enough that the manager should not reopen them casually.

### Product shape

- one codebase
- one product binary
- local mode and self-hosted team mode are first-class in v1
- managed cloud and hybrid are later motions on the same product model
- no separate enterprise architecture fork

### Runtime and ops

- runtime/event truth is canonical in Cairn, not in queue substrates or sidecars
- SSE replay floor is 72 hours
- tenant roll-up views are read-only for operations in v1
- workspace-level cross-project bulk operational actions are forbidden in v1

### Retrieval and graph

- owned retrieval is required in the product core
- lexical floor is Postgres full-text plus owned normalization/filtering/reranking
- PDF/Office extraction is additive, not part of the first sellable floor
- chunk-level portability is advisory; receivers re-derive final chunking/indexing

### Prompts, evals, and policy

- one canonical prompt lifecycle
- regulated-project no-shortcut is a policy preset, not a low-level flag surface
- prompt libraries are workspace-first
- tenant-scoped centrally governed prompt libraries are opt-in for adoption in v1

### Commercialization

- self-hosted-first is the first sellable motion
- paid differentiation is through named entitlements, not hidden forks
- first paid expansion is a narrow governance/compliance package

## 3. Current Execution Reality

The repo is no longer in broad scaffold mode.

What the current manager should assume:

- `cargo test --workspace --quiet` is green from the last full verification pass
- `./scripts/check-compat-inventory.sh` is green from the last full verification pass
- most crate-level work is present and usable
- the remaining work is seam-closing and truth-maintenance work

This means the manager's job is now:

- keep the remaining seams honest
- keep generated reports aligned with executable truth
- avoid inventing busywork
- avoid reopening already-closed arguments because of stale mailbox history

## 4. What Is Already Proven In Code

These points are important because stale notes previously kept reopening them.

### Closed enough to stop reassigning

1. Feed HTTP and `feed_update` SSE now align on the string-ID contract.

   Relevant files:

   - [`../tests/fixtures/http/GET__v1_feed__limit20_unread_true.json`](../tests/fixtures/http/GET__v1_feed__limit20_unread_true.json)
   - [`../tests/fixtures/sse/feed_update__single_item.json`](../tests/fixtures/sse/feed_update__single_item.json)
   - [`../crates/cairn-api/tests/http_boundary_alignment.rs`](../crates/cairn-api/tests/http_boundary_alignment.rs)
   - [`../crates/cairn-api/tests/sse_payload_alignment.rs`](../crates/cairn-api/tests/sse_payload_alignment.rs)

2. `assistant_tool_call` completed/failed runtime payloads preserve `taskId`, `toolName`, and `phase`.

   Relevant files:

   - [`../crates/cairn-domain/src/events.rs`](../crates/cairn-domain/src/events.rs)
   - [`../crates/cairn-runtime/src/services/tool_invocation_impl.rs`](../crates/cairn-runtime/src/services/tool_invocation_impl.rs)
   - [`../crates/cairn-api/src/sse_payloads.rs`](../crates/cairn-api/src/sse_payloads.rs)
   - [`../crates/cairn-api/tests/sse_payload_alignment.rs`](../crates/cairn-api/tests/sse_payload_alignment.rs)

3. Runtime recovery is no longer placeholder-only.

   Relevant files:

   - [`../crates/cairn-runtime/src/services/recovery_impl.rs`](../crates/cairn-runtime/src/services/recovery_impl.rs)

4. The old queue automation experiment is not part of the active execution model.

   Relevant files:

   - [`../README.md`](../README.md)
   - [`README.md`](./README.md)
   - [`queue/README.md`](./queue/README.md)

5. The active coordination model is manager + 3 workers, not the historical 8-worker split.

   Relevant files:

   - [`../docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
   - [`mailbox/worker-surface.md`](./mailbox/worker-surface.md)
   - [`mailbox/worker-core.md`](./mailbox/worker-core.md)
   - [`mailbox/worker-knowledge.md`](./mailbox/worker-knowledge.md)

## 5. Honest Open Seams

These are the real half-finished or still-explicit seams worth managing.

### A. App/bootstrap composition is not done

Evidence:

- [`../crates/cairn-app/src/main.rs`](../crates/cairn-app/src/main.rs)

Reality:

- the intended service wiring is still in comments
- the binary does not yet compose the app into a real running server path

### B. Memory API still uses temporary local CRUD state

Evidence:

- [`../crates/cairn-memory/src/api_impl.rs`](../crates/cairn-memory/src/api_impl.rs)

Reality:

- search is backed by retrieval service
- create/list/accept/reject still use a local in-memory map, generated IDs, and generated timestamps

This is acceptable scaffolding, but it is still half-work relative to a durable product path.

### C. Generated migration reports still lag live code

Evidence:

- [`../tests/fixtures/migration/phase0_http_endpoint_gap_report.md`](../tests/fixtures/migration/phase0_http_endpoint_gap_report.md)
- [`../tests/fixtures/migration/phase0_sse_publisher_gap_report.md`](../tests/fixtures/migration/phase0_sse_publisher_gap_report.md)
- [`../tests/fixtures/migration/phase0_owner_map.md`](../tests/fixtures/migration/phase0_owner_map.md)
- [`../tests/fixtures/migration/phase0_sse_payload_handoff.md`](../tests/fixtures/migration/phase0_sse_payload_handoff.md)

Specific known drift:

- memory search report wording is behind the current code/tests
- `memory_proposed` is still described as unmapped, but hook + builder code already exists

### D. Exact builder path vs generic runtime mapping is still unresolved for two SSE families

Evidence:

- [`../crates/cairn-api/tests/sse_payload_alignment.rs`](../crates/cairn-api/tests/sse_payload_alignment.rs)

Affected:

- `task_update`
- `approval_required`

Reality:

- exact dedicated builders already exist
- the generic runtime-event path is still thinner

The manager should treat this as a real open seam, not as “missing implementation everywhere”.

### E. `assistant_end` still needs caller-assembled final text

Evidence:

- [`../crates/cairn-api/src/sse_payloads.rs`](../crates/cairn-api/src/sse_payloads.rs)
- [`../crates/cairn-api/tests/sse_payload_alignment.rs`](../crates/cairn-api/tests/sse_payload_alignment.rs)
- [`../crates/cairn-api/tests/product_surface_composition.rs`](../crates/cairn-api/tests/product_surface_composition.rs)

Reality:

- enriched end builder exists
- current composition still depends on upstream code to pass the fully assembled final message

### F. `memory_proposed` is partially real but not fully composed

Evidence:

- [`../crates/cairn-app/src/sse_hooks.rs`](../crates/cairn-app/src/sse_hooks.rs)
- [`../crates/cairn-memory/src/api_impl.rs`](../crates/cairn-memory/src/api_impl.rs)
- [`../crates/cairn-app/src/main.rs`](../crates/cairn-app/src/main.rs)

Reality:

- hook path exists
- builder exists
- test exists for the hook in isolation
- app/bootstrap still does not wire this as a real product path

## 6. Manager Guidance: What Not To Reopen

Do not reopen these unless a failing test or RFC contradiction forces it.

- feed ID string contract
- `assistant_tool_call` completed/failed identity preservation
- runtime recovery as a “missing placeholder” story
- the queue bus as an active coordination system
- the 8-worker plan as the live execution model

If an older mailbox says one of those is still open, prefer code/tests over the mailbox note.

## 7. Active Workstreams

### Worker Surface

Owns:

- `cairn-api`
- `cairn-app`
- compatibility fixtures and generated reports

Best use:

- report truthfulness
- app composition
- `assistant_end` surface closure
- `memory_proposed` surface closure

### Worker Core

Owns:

- `cairn-domain`
- `cairn-store`
- `cairn-runtime`
- `cairn-tools`

Best use:

- provide honest runtime/store/read-model support for enriched API/SSE surfaces
- expose richer durable tool result/error detail if the product surface truly needs it

### Worker Knowledge

Owns:

- `cairn-memory`
- `cairn-graph`
- `cairn-agent`
- `cairn-evals`

Best use:

- remove temporary memory-side shortcuts
- support durable `memory_proposed` composition
- keep streaming/graph/eval surfaces stable while Surface closes truth gaps

## 8. Acceptance Standard

A seam is only closed when these all agree:

- code
- executable tests
- generated reports
- active coordination docs

If one of those is still behind, the seam remains open.

## 9. Manager Checklist

When taking over, do this in order:

1. run:

   - `cargo test --workspace --quiet`
   - `./scripts/check-compat-inventory.sh`

2. read:

   - [`../docs/design/MANAGER_THREE_WORKER_REPLAN.md`](../docs/design/MANAGER_THREE_WORKER_REPLAN.md)
   - this file
   - the 3 active worker mailboxes

3. ignore as active task sources:

   - `worker-1.md` through `worker-8.md`
   - queue artifacts

4. look for:

   - report truth lagging code
   - tests that explicitly say a gap is still open
   - bootstrap/composition work that is still only commented

5. assign only bounded seam-closing work

Avoid:

- busywork backlogs
- generic “support mode” churn
- reopening already-closed product decisions

## 10. Validation Commands

Primary commands:

- `cargo test --workspace --quiet`
- `./scripts/check-compat-inventory.sh`

Useful targeted checks:

- `cargo test -p cairn-api --test http_boundary_alignment --test sse_payload_alignment --test migration_report_consistency`
- `cargo test -p cairn-runtime --tests`
- `cargo test -p cairn-memory`

## 11. Historical Note

The repo contains a lot of execution history from the earlier 8-worker pass.

That history is useful for understanding how we got here, but it is no longer the operating model.

Manager principle:

- preserve history
- do not let history outrank current executable truth
