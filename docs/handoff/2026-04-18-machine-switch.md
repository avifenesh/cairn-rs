# Handoff — 2026-04-18 — Machine Switch

Onboarding doc for resuming cairn-rs work on a different machine. Written for a new Claude Code session with zero prior context.

Current `main` tip: `4b407474` (fix(cairn-fabric): try_task helper, PR #38). Today's date: 2026-04-18.

---

## 1. Project Identity

- **cairn-rs** — Rust control plane for agent operations. Event-sourced, multi-tenant, provider-agnostic. Serves real engineering teams in production. Quality is the product.
- **FlowFabric (FF)** — Avi's own Valkey-native execution engine. Separate repo at `github.com/avifenesh/FlowFabric`. cairn-rs `cairn-fabric` crate bridges to FF for atomic lease enforcement, 6-dim execution state vectors, typed suspensions, 37 FCALL functions, 14 background scanners. FF is the execution source-of-truth; cairn-rs owns the audit/projection layer.
- **License:** BSL 1.1 (Business Source License). Badges + LICENSE + Cargo.toml + README footer all aligned.
- **Rust toolchain:** 1.95 (pinned via `rust-toolchain.toml`).
- **21-crate workspace.** See README `## Architecture` and the crate table.
- **`unsafe_code = "forbid"`** at workspace root.

### Hard rules (from CLAUDE.md)

1. Production quality only. Never cut corners.
2. Nothing ships without tests. Integration tests required for every feature.
3. Fix everything you touch (in-scope + old broken code).
4. Verify subagent work — never assume success.
5. Ask before designing. Validate with user before implementing.
6. Be proactive. Propose enhancements. Confirm direction before large changes.

### Source-of-truth order when docs disagree

1. RFCs under `docs/design/rfcs/`
2. `docs/design/` compat docs
3. `CLAUDE.md`
4. Go `cairn-sdk` only for preserved behavior / fixtures

---

## 2. Recently Merged (chronological, current session)

All merged via `gh pr merge --admin --squash` (CI credits exhausted on private repo; admin-merge is the standing practice).

| PR  | Merge SHA  | What |
|-----|------------|------|
| #29 | `10ae3534` | HIGH event-emission gate bug fix + retire `ActiveTaskRegistry`. Killed cairn-side state duplication. |
| #30 | `d0ab3673` | `TaskFrameSink` trait wires `CairnTask` into orchestrator loop. 4 per-iteration frames (`llm_response` → `tool_call` → `tool_result` → `checkpoint`), lease-health gate, `OrchestratorError::FrameSink` variant, integration test drives real `OrchestratorLoop::run` against Valkey. 7 review rounds on the stack. |
| #34 | `c2947400` | L2 tech-debt — broadened `test_orchestrator_stream.rs` payload asserts on all 4 frames. |
| #35 | `c2a31021` | `POST /v1/runs/:id/claim` HTTP endpoint. 12 commits, **7 review rounds** (R1→R7). Non-idempotent contract documented + Fabric tripwire test. |
| #36 | `b18a45f1` | Root cleanup: deleted `PROMPT.md` + `RALPH-PROGRESS.md` (Ralph-loop workflow obsolete). Added `runs.claim` prereq note to `docs/design/CAIRN-FABRIC-FINALIZED.md` §4.3. |
| #37 | `bd5de7fc` | L4 tech-debt (minimum) — documented `per_result_duration_ms` truncation semantics + `tracing::debug` on silent 0. Issue #33 **stays open** for the "Better fix" (plumbing per-call duration through `ActionResult`). |
| #38 | `4b407474` | L3 tech-debt — `try_task()` / `try_stream_writer()` helpers replace `CairnTask::task()` panic on consumed state. 5 log methods route through helpers; 3 observer sites retain panic (caller-context guarantees `Some`). |

### Review-round discipline used this session

- Every PR got minimum 1 R1 review. Big surface (PR #35) got 7 rounds.
- Reviewers rotated per round.
- Independent catches matter — when two reviewers hit the same thing, it's real.
- "0 bugs" is a valid and valuable answer; "looks good" is not.
- Workers dispute findings with evidence; manager resolves.
- R3 was always a self-audit from the author.
- Admin-merge body rewrites commit-msg archaeology when the original wording is wrong (e.g. PR #38's `FrameSink`-vs-`Execute` correction).

---

## 3. Open Follow-ups on cairn-rs

### Open GitHub issues

| # | Title | Where it goes next |
|---|-------|---|
| #21 | SurrealDB as embedded storage backend (v0.2) | Long horizon — see §7. |
| #33 | Per-result duration_ms loses signal (L4) | **Minimum fix landed**; "Better fix" (plumb duration through `ActionResult`) stays tracked here. |

### Pending work tracked outside GitHub (in manager task-list / memory)

- **Bridge-event completeness audit across all fabric service mutations.** Some FF service mutations don't yet emit `BridgeEvent`s; walk every mutation path in `crates/cairn-fabric/src/services/*.rs` and verify each has either (a) a `BridgeEvent` with a projection consumer, or (b) explicit documented silence on lean-bridge grounds (like `runs.claim` does — FF owns the state, no projection reader).
- **PR #27 cross-review round** — held over from a previous session; check git log for `feat/cairn-fabric-runtime-consolidation-v2` and whether all findings were addressed.

---

## 4. FF-side Items We're Waiting For

FF is Avi's own project; we file issues against `github.com/avifenesh/FlowFabric`. When FF ships these, corresponding cairn-rs wiring becomes possible.

### Open FF issues blocking cairn-rs roadmap work

| FF # | Title | Unblocks on cairn side |
|------|-------|---|
| #9 | Batch C — ff-sdk polish before crate publish | Allows publishing cairn-fabric without path-deps; enables external worker SDK. |
| #11 | RFC-009: cap-routing convergence — V2 options | Capability routing (`RoutingRequirements.required_capabilities`) wiring in `FabricSchedulerService`. Multi-worker deployments need this. |
| ~~#14~~ | ~~Publish `pub fn claim_from_grant`~~ | **CLOSED upstream.** Landed in cairn via the rev-1b19dd10 bump (RFC-011 phase 1, mechanical sweep). Consumer-side adoption — replacing the `insecure-direct-claim` feature-gated path with the real grant-flow — is still pending as RFC-011 phase 2. |
| ~~#15~~ | ~~Publish `pub fn claim_from_reclaim_grant`~~ | **CLOSED upstream.** Same status as #14. Grant-flow resume adoption tracked under RFC-011 phase 2. |
| ~~#16~~ | ~~Publish `parse_report_usage_result`~~ | **CLOSED upstream and adopted.** `budget_service.rs` now calls `ff_sdk::task::parse_report_usage_result` directly; the parallel parser was deleted in RFC-011 phase 1. |
| #21 | Crash-recovery scanner for `flow_id` / flow membership consistency (cairn P3.6 §5.5) | Documented timeout-split-brain reconciliation path. |
| #22 | perf-invest: cairn P3.6 bridge-event audit (reports) | Input to our bridge-event completeness audit. |

### FF roadmap items cairn-rs needs but not yet issued / bigger-scope

From prior sessions' memory (may need re-triage against current FF state):

- **Stream read ops** — `read_attempt_stream`, `tail_stream` — needed for checkpoint recovery in orchestrator. Status: earlier memory says wired (`#78` in old task-list marked done); confirm against current FF main before re-adding.
- **Waitpoint HMAC tokens** — multi-tenant approval gate security. Status: earlier memory says done (`#77` task-list); confirm.
- **`cancel_flow` chunking** — blocks `cairn archive()` on large sessions.
- **Cluster-safe SCAN in `budget_reconciler` + `retention_trimmer`** — blocks production Valkey cluster deploys.
- **Error kind flattening** — for proper cairn observability.
- **Partition shuffle or blocking claim** — scales idle workers.

### FF items nice-to-have, not blocking v1

- Budget/quota CRUD, scheduled executions, `detect_cycle` optimization, supervisor backoff.

### FF-internal names leaking into cairn-rs public surface

PR #35 review flagged but deferred: `ff_claim_resumed_execution`, `ff_issue_claim_grant`, `execution_not_eligible` appear in OpenAPI descriptions + docstrings. They're accurate but couple our public API wording to FF internals. Deferred follow-up: user-facing-abstraction sweep to rename these together in all docstrings.

---

## 5. Immediate Next Steps (pick up here on new machine)

### Priority order

**1. Bridge-event completeness audit.** Walk `crates/cairn-fabric/src/services/*.rs`. For each `pub async fn` that mutates FF state, answer: does it emit a `BridgeEvent`? Does that event have a projection consumer in `cairn-store`? If no consumer, is the silence documented with lean-bridge rationale? Output: short markdown audit table in `docs/design/` + file issue per real gap.

**2. Issue #33 "Better fix" — thread per-call duration through `ActionResult`.** The minimum fix landed in PR #37; the follow-up removes the averaging math entirely by carrying real per-call duration. Touches `crates/cairn-orchestrator/src/execute_impl.rs` + `loop_runner.rs` + test stubs (~10 call sites per the deferral reasoning in the PR #37 commit). Estimate: small refactor, worth doing in one tight commit with test updates.

**3. RFC-011 phase 2 — adopt grant-flow on the consumer side.** FF #14/#15/#16 all landed in cairn via the rev-1b19dd10 bump (phase 1, mechanical sweep). Phase 2 replaces cairn-fabric's `insecure-direct-claim` feature-gated path with the real `claim_from_grant` / `claim_from_reclaim_grant` scheduler-mediated flow. This is the biggest remaining win — unlocks a real external worker SDK and removes the feature gate we've been carrying.

**4. External worker SDK** (`worker_sdk.rs` in `cairn-fabric`). Blocked on #3. Goal: cairn-ergonomic wrapper around `ff-sdk` `FlowFabricWorker`. Pattern: `CairnWorker::claim_next() → CairnTask` with typed helpers. See earlier memory for the sketch.

**5. Bridge-event audit resolution.** Fix whatever #1 surfaces. Expect ~3-8 missing events plus ~2-5 justified silences to document.

---

## 6. Medium-horizon Plan

### Execution-layer completion (3-4 weeks of work, assuming FF asks land)

- Capability routing on scheduler (`FabricSchedulerService` matches worker capabilities against `RoutingRequirements.required_capabilities`). Blocked on FF #11.
- Timeout split-brain reconciliation scanner (or document as acceptable if reads always go through `run_service.get()` which reads fresh from FF).
- Priority dispatch via `ExecutionPolicy.priority` — currently `_priority` is ignored on task submission. Composite score `-(priority * 1T) + created_at_ms` for FIFO-within-priority.
- Lease epoch fencing re-audit: every terminal mutation must validate `lease_id + epoch + expiry` from fresh exec_core read.

### Operator surface polish

- 401/403 auth coverage on run-mutation endpoints — pre-existing gap across `/v1/runs/:id/{cancel,pause,resume,approve,reject,revise,claim}`. Single sweep.
- OpenAPI FF-internal-name abstraction sweep (see §4 last bullet).
- Consistency: claim_run_handler does get-first, cancel/pause/resume don't. Normalize either direction (lean toward "remove get-first everywhere" since the adapter already maps NotFound via `resolve_run_project`).

### UI / frontend items (lower urgency, check `ui/` if touching)

- Embedded via `rust-embed` in `cairn-app` binary — after `npm run build` the Rust build picks up new assets.
- Stale references to `cairn-sdk` (Go reference impl) — grep + scrub if any found.

---

## 7. Long-horizon

- **SurrealDB v0.2 backend** (Issue #21) — graph + doc + relational in one. BSL 1.1 compatible. Evaluated in prior session; fits the agent-control-plane data model. Plan: integrate after v1 stability test. Add as feature-gated store alongside Postgres / SQLite / InMemory.
- **Capability routing V2** (FF #11 long-tail) — post-v1 multi-worker deployment shape.
- **Managed-cloud and hybrid modes** — see RFC 023. Not v1 scope; first v1 motion is local mode + self-hosted team mode.
- **Plugin marketplace** (RFC 015) — catalog, publishing, reviews. Crate exists (`cairn-plugin-catalog`); operator UI is the next step.
- **GitHub App dogfood deployment on dolly** — earlier session set up Bedrock keys + GitHub App creds + admin token. See memory `reference_dolly_keys.md` on the session's machine; won't transfer. Re-issue credentials on the new machine if resuming that track.

---

## 8. Repository Layout (quick orientation)

```
/mnt/c/Users/avife/cairn-rs/                    ← bare repo (manager's context, no working tree)
/mnt/c/Users/avife/cairn-rs-work/               ← worker worktrees (current session; transient)
  w1-runs-claim/  ...  (feat/runs-claim-endpoint — merged; branch can be deleted)
  w2-issues/      ...  (fix/issue-32-try-task-helper — merged; branch can be deleted)
  w3-cleanup/     ...  (chore/root-cleanup-and-doc-drift — merged; branch can be deleted)
  handoff/        ...  (chore/handoff-doc — THIS BRANCH)
/mnt/c/Users/avife/cairn-rs-decision-sync       ← older worktree (codex/memory-contract merged in PR #23)
```

**For the new machine:** clone fresh (one working tree) or set up worktrees if you'll run the team-mode multi-agent pattern again. Worktree flow is documented in §9 below.

### Crate map (21 crates)

Read `README.md` architecture section for the full table. Key execution-layer crates:

- `cairn-domain` — pure types, no IO.
- `cairn-store` — append-only event log + projections. Backends: Postgres (default), SQLite, InMemory.
- `cairn-runtime` — service traits (RunService, TaskService, SessionService, etc.).
- `cairn-fabric` — **bridge to FF**. 37 FCALL builders, `CairnTask`, `CairnWorker`, adapters that implement cairn-runtime traits.
- `cairn-orchestrator` — agent loop, `OrchestratorLoop::run`, `TaskFrameSink`.
- `cairn-app` — Axum HTTP server. Handlers + routes + OpenAPI spec.
- `cairn-providers` — unified LLM abstraction (13 backends including Bedrock, Vertex, Ollama).

### RFCs (23 total)

Under `docs/design/rfcs/`. RFC 005 (task/session lifecycle), RFC 009 (provider abstraction), RFC 020 (durable recovery), RFC 022 (triggers), RFC 023 (cloud architecture) are the most load-bearing for execution-layer work.

---

## 9. Team-mode Multi-Agent Pattern (if resuming)

### Setup

Four-pane tmux session named `cairn-rs-team`: manager (pane %0) + worker-1/2/3 (panes %1/%2/%3). Watchers at `/home/avife/team-tmux/team-watch.sh` inject messages via `.coordination/mailbox/inbox/<agent>/`. See `CLAUDE.md` § Team Mode.

### Critical lessons learned this session

- **Use isolated worktrees, not shared-tree branch-switching.** Multiple workers on the same working tree with different branches checked out in-and-out produces commit corruption and stash-juggling thrash. Fix: one worktree per worker (`/mnt/c/Users/avife/cairn-rs-work/wN-<task>`), persistent for the duration of their work.
- **Worktrees need `npm install` in `ui/` once each.** Pre-push hook runs `npm run build`. Fresh worktree has no `node_modules`; run `cd ui && npm install` once per worktree; it persists across branch switches within that worktree.
- **Never `--no-verify` to skip pre-push.** Hard rule in CLAUDE.md. If the hook blocks on missing deps, install them; if it blocks on a real failure, fix it.
- **Manager's tree stays on `main`, hands-off.** Workers own their worktrees; manager only reads state + admin-merges.

### Communication

- `./scripts/team-send.sh <to> <from> "message"` — filesystem mailbox, injected into target's context.
- Workers must always report back via team-send. Pane-only output is invisible to manager.
- Every report: files changed, test counts, pass/fail, blockers. Specific.

### Cross-review protocol

- Rotate reviewers per round.
- Each reviewer gets a brief with specific audit criteria.
- Findings format: `[FILE:LINE] BUG/GAP/STYLE: description`.
- BUG = wrong behavior in production; GAP = missing functionality/error handling; STYLE = non-critical.
- 0-bug answer valid.
- Workers dispute with evidence; manager resolves.
- Multi-round until 0 bugs, extend on persistent findings. PR #35 went 7 rounds.

---

## 10. Build / Run / Test Commands

See `CLAUDE.md` for the canonical list. Quick reference:

```bash
# Build
cargo build --workspace

# Run locally (Postgres recommended)
DATABASE_URL=postgres://cairn:pass@localhost:5432/cairn \
CAIRN_ADMIN_TOKEN=dev-admin-token cargo run -p cairn-app

# In-memory fallback (dev only, ephemeral)
cargo run -p cairn-app -- --db memory

# Tests — all
cargo test --workspace

# Tests — specific arms
cargo test -p cairn-fabric --lib
cargo test -p cairn-fabric --test integration                       # default arm
cargo test -p cairn-fabric --test integration --features insecure-direct-claim  # +orchestrator-stream test
CAIRN_TOKEN=dev-admin-token ./scripts/smoke-test.sh                 # 81-check HTTP smoke

# UI
cd ui && npm install                                                # once per worktree
cd ui && npm run build                                              # must pass before cargo build for embedded assets
cd ui && npx playwright test                                        # 72 browser E2E tests
```

### Pre-push hook (`.githooks/pre-push`)

Runs: rust lib tests (`in-memory-runtime` feature), rust integration tests, **fabric integration both arms** (default + `insecure-direct-claim`), ui build, vitest. Playwright skipped if no server on :3000. All must pass.

---

## 11. FlowFabric Context (for the new session)

- **Repo:** `github.com/avifenesh/FlowFabric`
- **Branch we pin against:** `feat/execution-engine` (last pinned commit in cairn-rs session memory: `1b19dd10` — check `crates/cairn-fabric/Cargo.toml` for current git dep rev).
- **Relationship:** cairn-rs depends on FF via git dependency. When FF lands a needed fix, bump the rev in Cargo.toml; rebuild; test.
- **Filing FF issues:** `gh issue create --repo avifenesh/FlowFabric --title "..." --body "..."`.
- **FF internals to know:**
  - 37 Lua FCALL functions (`lua/*.lua` in FF repo).
  - 6-dimensional state vector: `lifecycle_phase`, `eligibility_state`, `attempt_state`, `ownership_state`, `public_state`, `terminal_outcome`.
  - 14 background scanners at varying intervals (1.5s lease-expiry → 60s retention).
  - `ff_issue_claim_grant` + `ff_claim_execution` is the two-step claim protocol.
  - `ff_claim_resumed_execution` dispatches ONLY from `attempt_interrupted` state (post-suspend). Re-claim of an already-active run fails at the grant gate (`execution_not_eligible`) — this is the non-idempotency contract documented in PR #35.

---

## 12. Environment Notes

- **Platform:** WSL2 on Windows, but repo lives on `/mnt/c/...` (Windows filesystem, slower than pure Linux FS — not blocking but known).
- **Git quirk in WSL:** `git config core.bare false` can reset; re-set if git ops behave oddly.
- **Admin merge:** Standing practice. CI credits on the private repo are exhausted. When flipping to public (see memory: user planned this after PR #29 merged), CI may or may not be re-enabled — check before merging next PR.
- **Provider:** Bedrock via Mantle. `CLAUDE_CODE_USE_MANTLE=1`. Model IDs without `us.` / `global.` prefix (e.g. `anthropic.claude-opus-4-7[1m]` for 1M context).

---

## 13. Working Style Reminders

- Depth over speed. No rubber-stamp reviews. Find real bugs.
- Every round must find something or prove absence. "0 bugs" with reasoning is a valid answer.
- Production quality only — no silent fallbacks (`Option` over magic defaults, propagate errors, validate config).
- No mutex `.unwrap()` — use `unwrap_or_else` for poison recovery.
- No `println!` / `eprintln!` / `dbg!` in production code.
- Default tenant/workspace/project values: `default_tenant`, `default_workspace`, `default_project` (NOT `default/default/default`).
- Git is rollback; no feature flags for new code paths. Direct replacement.
- Never `--no-verify`. Fix the underlying issue.

---

## 14. First Actions on New Machine

```bash
# 1. Clone
git clone git@github.com:avifenesh/cairn-rs.git
cd cairn-rs

# 2. Read context
cat CLAUDE.md                                        # project rules
cat docs/handoff/2026-04-18-machine-switch.md        # this file
cat docs/design/CAIRN-FABRIC-FINALIZED.md            # fabric contract (§4.3 has runs.claim note)

# 3. Verify build
cargo check --workspace
cd ui && npm install && npm run build && cd ..
cargo test -p cairn-runtime --lib
cargo test -p cairn-fabric --lib

# 4. If running fabric-integration:
#    - Start local Valkey: docker run -p 6379:6379 valkey/valkey:8.1
#    - CAIRN_TEST_VALKEY_URL=valkey://localhost:6379 cargo test -p cairn-fabric --test integration

# 5. Check FF state
gh issue list --repo avifenesh/FlowFabric --state open --limit 30

# 6. Pick from §5 Immediate Next Steps
```

---

## 15. Not In This Handoff (Intentionally)

- Ephemeral session coordination (`.coordination/` — gitignored, transient).
- Multi-agent tmux pane-level setup — machine-specific (`/home/avife/team-tmux/`).
- Dolly deployment credentials — machine-specific secrets.
- Auto-memory at `~/.claude/projects/.../memory/` — separate from repo, won't transfer, but covers similar ground.

Revive the tmux team + worktree pattern on the new machine if the work is multi-author or high-blast-radius. For a single-author track, one worktree on one branch is fine.

---

*End handoff. Push this doc, merge, then on the new machine pull main and start at §14.*
