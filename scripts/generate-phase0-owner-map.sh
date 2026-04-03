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
  printf '| `GET /v1/feed?limit=20&unread=true` | preserved fixture only, no explicit API boundary yet | `Worker 8` + `Worker 6` | Feed/list surface likely depends on signal/channel aggregation and read-model shaping. |\n'
  printf '| `GET /v1/memories/search?q=test&limit=10` | preserved fixture only, no explicit API boundary yet | `Worker 6` + `Worker 8` | Retrieval/search boundary should stay grounded in `cairn-memory` service traits. |\n'
  printf '| `POST /v1/assistant/message` (with session) | preserved fixture only, no explicit API boundary yet | `Worker 8` + `Worker 4` + `Worker 7` | API glue over runtime run/task creation with agent/prompt semantics. |\n'
  printf '| `POST /v1/assistant/message` (without session) | preserved fixture only, no explicit API boundary yet | `Worker 8` + `Worker 4` + `Worker 7` | Same as above, but bootstrap/create-session path must stay explicit. |\n'
  printf '\n## SSE Surfaces\n\n'
  printf '| Event | Current Boundary Signal | Likely Owner | Notes |\n'
  printf '|---|---|---|---|\n'
  printf '| `ready` | dedicated `build_ready_frame()` path exists | `Worker 8` | Already covered; preserve as-is. |\n'
  printf '| `task_update` | runtime event mapped, payload wrapper missing | `Worker 8` + `Worker 4` | Needs preserved `{ task }` wrapper over runtime task events. |\n'
  printf '| `approval_required` | runtime event mapped, payload wrapper missing | `Worker 8` + `Worker 4` | Needs preserved `{ approval }` wrapper over approval request events. |\n'
  printf '| `assistant_tool_call` | runtime event mapped, payload wrapper missing | `Worker 8` + `Worker 4` + `Worker 5` | Tool lifecycle is runtime-owned but shape should match preserved frontend payload. |\n'
  printf '| `agent_progress` | runtime event mapped, payload wrapper missing | `Worker 8` + `Worker 4` + `Worker 7` | External-worker/subagent progress needs preserved `{ agentId, message }` shaping. |\n'
  printf '| `feed_update` | no runtime publisher mapping yet | `Worker 8` + `Worker 6` | Likely non-runtime feed/signal aggregation surface. |\n'
  printf '| `poll_completed` | no runtime publisher mapping yet | `Worker 8` + `Worker 6` | Likely signal/source polling surface, not core runtime event log. |\n'
  printf '| `assistant_delta` | no runtime publisher mapping yet | `Worker 8` + `Worker 7` | Streaming assistant output ownership still needs to be made explicit. |\n'
  printf '| `assistant_end` | no runtime publisher mapping yet | `Worker 8` + `Worker 7` | Final assistant message event likely belongs with agent/output streaming boundary. |\n'
  printf '| `assistant_reasoning` | no runtime publisher mapping yet | `Worker 8` + `Worker 7` | Reasoning stream/event family needs explicit owner before publisher work. |\n'
  printf '| `memory_proposed` | no runtime publisher mapping yet | `Worker 8` + `Worker 6` | Memory proposal surface likely belongs with memory/retrieval proposal flow rather than generic runtime events. |\n'
} > "$OUT"

echo "generated $OUT"
