# Worker 7 Mailbox

Owner: Agent Runtime, Prompts, Evals

## Current Status

- 2026-04-03 | Week 1 assigned | Scaffold `cairn-agent` and `cairn-evals`, define prompt asset/version/release, selector, scorecard, and orchestrator module boundaries.
- 2026-04-03 | Worker 7 / Manager | `cairn-agent` and `cairn-evals` scaffolds complete | Prompt registry types (asset/version/release/action), selector/resolution types, eval metrics/matrices/scorecards, agent orchestrator/subagent/react/reflection boundaries are all in repo with passing tests.
- 2026-04-03 | Week 2 assigned | Implement prompt release lifecycle and selector resolution against domain contracts. Define agent-runtime hooks.
- 2026-04-03 | Worker 7 / Manager | Week 2 complete | PromptReleaseService (full RFC 006 lifecycle: create, transition with validation, activation deactivation, rollback with audit trail). SelectorResolver (precedence-based resolution: routing_slot > task_type > agent_type > project_default). RuntimeHookHandler and PromptResolver traits in cairn-agent connecting orchestrator to runtime services. Borrow checker fixes applied for Worker 1 flagged issues. 22 cairn-evals tests + 9 cairn-agent tests passing.
- 2026-04-03 | Week 3 assigned | Implement prompt registry persistence flow, eval scorecard row creation, and first agent-runtime execution slice on top of the runtime spine.
- 2026-04-03 | Worker 7 / Manager | Week 3 complete | EvalRunService (create/start/complete lifecycle, build_scorecard aggregation across releases). AgentExecutor with generic AgentDriver trait driving the ReAct loop via RuntimeHookHandler + PromptResolver. 25 cairn-evals tests + 11 cairn-agent tests passing.
- 2026-04-03 | Week 4 assigned | Complete first prompt release + eval + agent execution slice. Make scorecards queryable.
- 2026-04-03 | Worker 7 / Manager | Week 4 complete | Full prompt-as-product integration: create releases → activate with selector → resolve at runtime → run evals → build scorecard → promote based on results → rollback with audit. Multi-selector precedence test (agent_type overrides project_default). 27 cairn-evals tests (25 unit + 2 integration) + 11 cairn-agent tests passing.
- 2026-04-03 | Worker 7 / Manager | Integration blocker cleared | `cargo test -p cairn-evals` is green again. The next meaningful dependency handoff is the assistant streaming/output seam Worker 8 needs for `assistant_delta`, `assistant_end`, and `assistant_reasoning`.

## Blocked By

- none

## Inbox

- 2026-04-03 | Manager -> Worker 7 | Next pacing cut: turn the prompt/eval slice into a stronger downstream contract. Add one focused graph/API support proof that `GraphIntegration` plus scorecard/release data stay stable enough for Worker 8 to surface without re-deriving prompt/eval semantics.
- 2026-04-03 | Manager -> Worker 7 | Keep this narrow: one representative graph-linked prompt/eval path and one stable read seam are enough. Do not widen rollout, scorecard, or policy behavior unless the integration path actually requires it.
- 2026-04-03 | Manager -> Worker 7 | Current next focus: hold the agent/evals seam stable and support Worker 8’s streaming integration. Keep `cairn-agent` and `cairn-evals` green, and resist widening rollout or scorecard scope unless a real integration gap appears.
- 2026-04-03 | Architecture Owner -> Worker 7 | Week 1 focus: prompt/eval/agent skeletons and service interfaces that match RFC 006 and RFC 004.
- 2026-04-03 | Worker 1 -> Worker 7 | Keep week 1 focused on module boundaries and selector/scorecard interfaces. Do not expand rollout semantics beyond RFC 006.
- 2026-04-03 | Worker 2 -> Worker 7 | Prompt/provider/runtime shared IDs are stable in `cairn-domain`; eval and prompt crates can depend on those IDs immediately.
- 2026-04-03 | Worker 6 -> Worker 7 | `cairn-graph` includes `PromptAsset`, `PromptVersion`, `PromptRelease`, `EvalRun` node kinds and `EvaluatedBy`, `ReleasedAs`, `RolledBackTo`, `UsedPrompt` edge kinds. Graph projection and query interfaces are ready for prompt/eval integration.
- 2026-04-03 | Manager -> Worker 7 | The old `cairn-evals` blocker is resolved. Keep the crate green, support Worker 8 on the streaming seam, and route any future changes through narrow integration fixes instead of new feature breadth.
- 2026-04-03 | Worker 6 -> Worker 7 | Wave 4: `EvalGraphProjector` ready in `cairn-graph`. Call `on_asset_created`/`on_version_created`/`on_release_created`/`on_eval_run_created`/`on_release_rollback`/`on_prompt_used` from your prompt/eval services to wire graph linkage. Retrieval projector also available for eval-retrieval quality integration.
- 2026-04-03 | Manager -> Worker 7 | Concrete next cut: keep the `StreamingOutput` handoff stable for Worker 8 and add only the smallest follow-up needed if the API layer finds a mismatch. Do not broaden scorecard or rollout features unless integration truly requires it.

## Outbox

- 2026-04-03 | Worker 7 -> Worker 4 | `cairn-agent` exposes `AgentConfig`, `StepOutcome`, `StepContext`, `SpawnRequest`, `SubagentLink` types. Runtime can reference these for agent execution coordination.
- 2026-04-03 | Worker 7 -> Worker 6 | `cairn-evals` prompt registry types (`PromptAsset`, `PromptVersion`, `PromptRelease`) and graph-linkable IDs are available. Graph nodes for prompt_asset, prompt_version, prompt_release, eval_run can be built against these.
- 2026-04-03 | Worker 7 -> Worker 8 | `cairn-evals` exposes all prompt registry types, release lifecycle with RFC 006 transition rules, selector/resolution types, eval run/scorecard structures, and eval metrics. API surfaces for prompt management, release lifecycle, eval comparison, and scorecard views can build against these types.
- 2026-04-03 | Worker 7 -> Worker 8 | `StreamingOutput` types in cairn-agent: `AssistantDelta`, `AssistantReasoning`, `AssistantEnd`, `ToolCallRequested`, `ToolResult` with `sse_event_name()` matching preserved SSE catalog. Wire SSE publisher to these types for assistant output streaming.
- 2026-04-03 | Worker 7 -> Worker 6 | `GraphIntegration` in cairn-evals now wraps `EvalGraphProjector`. Calls `on_asset_created`, `on_version_created`, `on_release_created`, `on_rollback`, `on_eval_run_created`, `on_prompt_used`. Graph linkage is wired from the eval/release service layer.

## Ready For Review

- 2026-04-03 | Worker 7 | Review `crates/cairn-agent/*` for Week 1 agent scaffold: orchestrator, react loop, subagent linkage, and reflection boundaries.
- 2026-04-03 | Worker 7 | Review `crates/cairn-evals/*` for Week 1 evals scaffold: prompt registry (assets/versions/releases/actions), selectors, eval matrices, scorecards.
