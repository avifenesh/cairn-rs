#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/tests/fixtures/migration/phase0_owner_map.md"

{
  printf '# Phase 0 Compatibility Owner Map\n\n'
  printf 'Status: generated  \n'
  printf 'Purpose: keep preserved Phase 0 HTTP and SSE compatibility gaps routed to the right worker/module owners instead of turning Worker 1 reports into orphaned TODOs.\n\n'
  printf '## HTTP Surfaces\n\n'
  printf '| Surface | Current Boundary Signal | Likely Owner | Notes |\n'
  printf '|---|---|---|---|\n'
  printf '| `GET /v1/tasks?status=running&type=agent` | `cairn-api` route catalog + runtime/store read-model seam present | `Worker 8` with `Worker 4` + `Worker 3` support | Runtime-facing read endpoint; preserved contract mostly depends on operator/read-model shaping. |\n'
  printf '| `GET /v1/approvals?status=pending` | `cairn-api` route catalog + runtime/store read-model seam present | `Worker 8` with `Worker 4` + `Worker 3` support | Approval inbox/read-model shaping, not a new storage contract. |\n'
  printf '| `GET /v1/stream?lastEventId=<id>` | `cairn-api::sse_publisher` present | `Worker 8` with `Worker 4` / `Worker 6` / `Worker 7` depending on event family | Compatibility gap is mainly preserved payload shaping and event-family ownership. |\n'
  printf '| `GET /v1/feed?limit=20&unread=true` | `FeedEndpoints` trait present in `feed.rs` | `Worker 8` + `Worker 6` | Feed/list surface now has an explicit API seam; the remaining work is backing it with signal/channel aggregation and read-model data. |\n'
  printf '| `GET /v1/memories/search?q=test&limit=10` | `MemoryEndpoints::search` present in `memory_api.rs` | `Worker 6` + `Worker 8` | Retrieval/search boundary is explicit now; the remaining work is wiring it cleanly to `cairn-memory` retrieval services. |\n'
  printf '| `POST /v1/assistant/message` (with session) | `AssistantEndpoints::send_message` present in `assistant.rs` | `Worker 8` + `Worker 4` + `Worker 7` | API seam exists; the remaining work is runtime/agent execution backing and later streaming output families. |\n'
  printf '| `POST /v1/assistant/message` (without session) | `AssistantEndpoints::send_message` present in `assistant.rs` | `Worker 8` + `Worker 4` + `Worker 7` | Same as above, but bootstrap/create-session behavior still needs to stay explicit at the API/runtime seam. |\n'
  printf '\n## SSE Surfaces\n\n'
  printf '| Event | Current Boundary Signal | Likely Owner | Notes |\n'
  printf '|---|---|---|---|\n'
  printf '| `ready` | dedicated `build_ready_frame()` path exists | `Worker 8` | Already covered; preserve as-is. |\n'
  printf '| `task_update` | runtime event mapped via `sse_payloads`, field-level alignment still open | `Worker 8` + `Worker 4` | Wrapper family exists; remaining work is preserved task-field completeness. |\n'
  printf '| `approval_required` | runtime event mapped via `sse_payloads`, field-level alignment still open | `Worker 8` + `Worker 4` | Wrapper family exists; remaining work is preserved approval metadata completeness. |\n'
  printf '| `assistant_tool_call` | runtime event mapped via `sse_payloads`, phase semantics still open | `Worker 8` + `Worker 4` + `Worker 5` | Tool lifecycle is runtime-owned but completed/failed payload semantics still need preserved frontend alignment. |\n'
  printf '| `agent_progress` | runtime event mapped via `sse_payloads`, progress semantics still open | `Worker 8` + `Worker 4` + `Worker 7` | Minimal `{ agentId, message }` shape exists; richer agent/subagent progress semantics still need tightening. |\n'
  printf '| `feed_update` | explicit non-runtime publisher boundary present | `Worker 8` + `Worker 6` | Non-runtime publisher seam exists; remaining work is wiring it to feed/signal aggregation. |\n'
  printf '| `poll_completed` | explicit non-runtime publisher boundary present | `Worker 8` + `Worker 6` | Non-runtime publisher seam exists; remaining work is wiring it to signal/source polling output. |\n'
  printf '| `assistant_delta` | no runtime publisher mapping yet | `Worker 8` + `Worker 7` | Streaming assistant output ownership still needs to be made explicit. |\n'
  printf '| `assistant_end` | no runtime publisher mapping yet | `Worker 8` + `Worker 7` | Final assistant message event likely belongs with agent/output streaming boundary. |\n'
  printf '| `assistant_reasoning` | no runtime publisher mapping yet | `Worker 8` + `Worker 7` | Reasoning stream/event family needs explicit owner before publisher work. |\n'
  printf '| `memory_proposed` | no runtime publisher mapping yet | `Worker 8` + `Worker 6` | Memory proposal surface likely belongs with memory/retrieval proposal flow rather than generic runtime events. |\n'
} > "$OUT"

echo "generated $OUT"
