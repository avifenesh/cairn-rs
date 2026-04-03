# Worker 5 Mailbox

Owner: Tools, Plugin Host, Isolation

## Current Status

- 2026-04-03 | Week 1 complete | Permission model, builtin tool host, plugin host, execution-class modules. 15 tests.
- 2026-04-03 | Week 2 complete | Durable invocation service and permission-decision events. 25 tests.
- 2026-04-03 | Week 3 complete | Builtin executor, plugin bridge, execution-class selection, cairn-plugin-proto wire types. 45 tests.
- 2026-04-03 | Week 4 complete | End-to-end `pipeline` module closing full tool_provider path: manifest -> execution-class selection -> permission check -> invoke (builtin or plugin JSON-RPC) -> durable record lifecycle. `run_builtin_pipeline` produces 3 record snapshots (Requested->Started->Completed). `build_plugin_pipeline_request` produces record + selected config + JSON-RPC payload for plugin execution. 42 cairn-tools + 7 cairn-plugin-proto tests passing.

## Blocked By

- none

## Inbox

- 2026-04-03 | Worker 6 -> Worker 5 | `cairn-graph` includes `UsedTool` edge kind and `ToolInvocation` node kind. Tool invocation graph linking is ready for integration when tool calls emit graph-linkable events.
- 2026-04-03 | Worker 1 / Manager -> Worker 5 | Current next focus: integration hardening. Connect pipeline/tool-invocation outputs to the graph-linkable/runtime-facing seams cleanly, and keep one representative plugin path ready for end-to-end runtime/API integration without broadening protocol scope.

## Outbox

- 2026-04-03 | Worker 5 -> Worker 4 | Week 4: `run_builtin_pipeline` closes the end-to-end builtin tool path. `build_plugin_pipeline_request` prepares plugin tool invocation for runtime dispatch. Both produce durable `ToolInvocationRecord` snapshots for the store.
- 2026-04-03 | Worker 5 -> Worker 6 | Week 4: Pipeline produces record snapshots at each lifecycle stage. Graph projection can link tool invocations using records from `pipeline::PipelineResult.records`.
- 2026-04-03 | Worker 5 -> Worker 7 | Week 4: Plugin protocol is complete for `tool_provider` category. `eval.score` wire types in `cairn-plugin-proto` are ready for eval scorer plugin integration.

## Ready For Review

- 2026-04-03 | Worker 5 | Review `crates/cairn-tools/src/pipeline.rs` for Week 4 end-to-end tool_provider pipeline. 42+7 tests passing, 0 warnings.
