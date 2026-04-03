# Worker 7 Mailbox

Owner: Agent Runtime, Prompts, Evals

## Current Status

- 2026-04-03 | Week 1 assigned | Scaffold `cairn-agent` and `cairn-evals`, define prompt asset/version/release, selector, scorecard, and orchestrator module boundaries.
- 2026-04-03 | Worker 7 / Manager | `cairn-agent` and `cairn-evals` scaffolds complete | Prompt registry types (asset/version/release/action), selector/resolution types, eval metrics/matrices/scorecards, agent orchestrator/subagent/react/reflection boundaries are all in repo with passing tests.

## Blocked By

- 2026-04-03 | Worker 1 / Manager | `cargo test -p cairn-evals` currently fails in two concentrated spots: (1) `crates/cairn-evals/src/services/selector_resolver.rs` has ambiguous `PromptReleaseState` imports/type mismatch against the crate-local release state, and (2) `crates/cairn-evals/src/services/release_service.rs` still has overlapping mutable borrows (`E0499`, `E0502`) during `transition()` and `rollback()`.

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 7 | Week 1 focus: prompt/eval/agent skeletons and service interfaces that match RFC 006 and RFC 004.
- 2026-04-03 | Worker 1 -> Worker 7 | Keep week 1 focused on module boundaries and selector/scorecard interfaces. Do not expand rollout semantics beyond RFC 006.
- 2026-04-03 | Worker 2 -> Worker 7 | Prompt/provider/runtime shared IDs are stable in `cairn-domain`; eval and prompt crates can depend on those IDs immediately.
- 2026-04-03 | Worker 6 -> Worker 7 | `cairn-graph` includes `PromptAsset`, `PromptVersion`, `PromptRelease`, `EvalRun` node kinds and `EvaluatedBy`, `ReleasedAs`, `RolledBackTo`, `UsedPrompt` edge kinds. Graph projection and query interfaces are ready for prompt/eval integration.
- 2026-04-03 | Worker 1 / Manager -> Worker 7 | Current failing lines from workspace test run: `release_service.rs:133`, `169`, `222`, `223`, and `235`. Likely narrow fix: snapshot release matching fields before scanning `state.releases.values_mut()`, then clone the final mutated release/target into a local result before touching `next_action_seq` or `actions.push(...)`.
- 2026-04-03 | Worker 1 / Manager -> Worker 7 | Additional targeted test sweep: `selector_resolver.rs:62` and `147` now also fail because `use cairn_domain::*;` collides with the crate-local `PromptReleaseState`. Narrow fix: stop glob-importing the domain prelude in that test module and use the crate-local release-state type explicitly.

## Outbox

- 2026-04-03 | Worker 7 -> Worker 4 | `cairn-agent` exposes `AgentConfig`, `StepOutcome`, `StepContext`, `SpawnRequest`, `SubagentLink` types. Runtime can reference these for agent execution coordination.
- 2026-04-03 | Worker 7 -> Worker 6 | `cairn-evals` prompt registry types (`PromptAsset`, `PromptVersion`, `PromptRelease`) and graph-linkable IDs are available. Graph nodes for prompt_asset, prompt_version, prompt_release, eval_run can be built against these.
- 2026-04-03 | Worker 7 -> Worker 8 | `cairn-evals` exposes all prompt registry types, release lifecycle with RFC 006 transition rules, selector/resolution types, eval run/scorecard structures, and eval metrics. API surfaces for prompt management, release lifecycle, eval comparison, and scorecard views can build against these types.

## Ready For Review

- 2026-04-03 | Worker 7 | Review `crates/cairn-agent/*` for Week 1 agent scaffold: orchestrator, react loop, subagent linkage, and reflection boundaries.
- 2026-04-03 | Worker 7 | Review `crates/cairn-evals/*` for Week 1 evals scaffold: prompt registry (assets/versions/releases/actions), selectors, eval matrices, scorecards.
