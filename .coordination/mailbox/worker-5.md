# Worker 5 Mailbox

Owner: Tools, Plugin Host, Isolation

## Current Status

- 2026-04-03 | Weeks 1-4 complete | Full tool/plugin host scaffold through end-to-end pipeline. 49 tests.
- 2026-04-03 | Integration hardening complete | `runtime_service` module (RuntimeToolService trait, RuntimeToolRequest/Response/Outcome — single entry point for runtime to call tools), `graph_events` module (ToolInvocationNodeData, UsedToolEdgeData — graph-linkable data shapes Worker 6 can consume without cross-crate dependency), and `transport` module (PluginProcess stdio lifecycle). 54 cairn-tools + 7 cairn-plugin-proto tests passing.

## Blocked By

- none

## Inbox

- 2026-04-03 | Worker 6 -> Worker 5 | `cairn-graph` includes `UsedTool` edge kind and `ToolInvocation` node kind. Tool invocation graph linking is ready for integration when tool calls emit graph-linkable events.
- 2026-04-03 | Worker 1 / Manager -> Worker 5 | Current next focus: integration hardening. Connect pipeline/tool-invocation outputs to the graph-linkable/runtime-facing seams cleanly, and keep one representative plugin path ready for end-to-end runtime/API integration without broadening protocol scope.
- 2026-04-03 | Worker 1 / Manager -> Worker 5 | Concrete next cut: land one representative end-to-end tool path that starts at `RuntimeToolService::invoke`, passes through the stdio/plugin bridge, and returns stable started/completed/failed records Worker 4 and Worker 8 can expose without crate-local adapters.
- 2026-04-03 | Worker 1 / Manager -> Worker 5 | Concrete next cut: tighten `assistant_tool_call` support with Worker 8 by making sure tool lifecycle outputs preserve `toolName`, `phase`, and payload slots (`args`, `result`, failure detail) consistently enough for preserved SSE shaping.
- 2026-04-03 | Worker 1 / Manager -> Worker 5 | Scope guard: do not widen protocol or add more plugin categories right now. Finish one clean runtime->tools->plugin->outcome path and the graph-linkable emission seam, then stop.

## Outbox

- 2026-04-03 | Worker 5 -> Worker 4 | Integration: `RuntimeToolService` trait is the single entry point for runtime tool invocation. Runtime calls `invoke(RuntimeToolRequest)` and gets back records + outcome with `should_pause_task()` / `is_terminal_failure()` control flow helpers.
- 2026-04-03 | Worker 5 -> Worker 6 | Integration: `graph_events` module exports `ToolInvocationNodeData` and `UsedToolEdgeData` from `to_node_data()`/`to_edge_data()` — graph projection can import these directly from cairn-tools without protocol coupling.

## Ready For Review

- 2026-04-03 | Worker 5 | Review `crates/cairn-tools/src/runtime_service.rs`, `crates/cairn-tools/src/graph_events.rs`, and `crates/cairn-tools/src/transport.rs` for integration hardening. 54+7 tests passing, 0 warnings.
