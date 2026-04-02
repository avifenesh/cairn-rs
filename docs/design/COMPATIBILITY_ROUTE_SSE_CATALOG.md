# Compatibility Route and SSE Catalog

Status: draft  
Reference surface: `../cairn` `origin/main` frontend API client and SSE store  
Purpose: make Phase 0 compatibility concrete enough for parallel runtime/API workers

## Rule

This catalog covers the UI-referenced route and SSE surface from the current Cairn frontend.

Each surface is tagged:

- preserve
- transitional
- intentionally break

Preserve means preserve the operator-facing contract closely enough for the existing UI to function during migration.

## HTTP Route Catalog

| Surface | Current route family | Classification | Notes |
|---|---|---|---|
| Health | `/health` | Preserve | Needed for deploy/runtime health checks |
| Global stream | `/v1/stream`, `/v1/stream/ticket` | Preserve | Core operator live view transport |
| Dashboard/feed | `/v1/dashboard`, `/v1/feed*` | Preserve | Core control-plane overview workflows |
| Tasks | `/v1/tasks*` | Preserve | Core runtime/operator workflow |
| Approvals | `/v1/approvals*` | Preserve | Core product wedge |
| PR inbox | `/v1/prs*` | Transitional | Preserve initially if UI depends on it; may be folded into source/channel views later |
| Assistant message/send | `/v1/assistant/message` | Preserve | Core runtime entrypoint |
| Assistant session list/detail | `/v1/assistant/sessions*` | Preserve | Needed for chat/session continuity |
| Agent sessions | `/v1/agent-sessions*` | Preserve | Needed for agent run inspection |
| Session control | `/v1/sessions/:id/steer`, `/pause`, `/resume`, `/events` | Preserve | Core runtime control and observability |
| Voice/upload | `/v1/assistant/voice`, `/v1/upload` | Transitional | Keep if current UX depends on them; not product-defining |
| Memories | `/v1/memories*` | Preserve | Core memory/retrieval surface |
| Fleet | `/v1/fleet` | Transitional | Likely survives as overview/read-model wrapper |
| Skills | `/v1/skills*`, `/v1/skills/proposed`, `/v1/skills/suggestions*` | Transitional | Keep while defaults/profile/skill packaging settles |
| Soul/profile/config files | `/v1/soul*`, `/v1/user-profile*`, `/v1/agents-config*`, `/v1/memory-file*` | Transitional | Must be re-expressed as scoped assets, but wrappers can remain during migration |
| Costs/status | `/v1/costs`, `/v1/status`, `/v1/session/recap`, `/v1/journal`, `/v1/plugins` | Preserve | May move to cleaner read models later, but useful operator surfaces now |
| Config | `/v1/config`, `/v1/repos/suggestions` | Preserve | Required for in-product operator setup |
| Poll trigger | `/v1/poll/run` | Preserve | Core signal-plane operator action |
| Crons | `/v1/crons*` | Preserve | Core scheduling surface |
| User tools | `/v1/tools/user*` | Transitional | Depends on final plugin/tool install model |
| MCP connections | `/v1/mcp/connections*` | Transitional | Likely adapter layer over broader plugin model |
| Marketplace | `/v1/marketplace*` | Intentionally break | Not part of v1 product core; may return later behind a different ecosystem model |
| Agent activity | `/v1/agent/activity*` | Preserve | Required for operator observability |
| Auth/WebAuthn | `/v1/auth/*` | Preserve | Current operator auth baseline |
| Subagents | `/v1/subagents*` | Preserve | Core orchestration surface |
| Rules and executions | `/v1/rules*`, `/v1/rule-templates*` | Preserve | Needed for signal automation/control-plane workflows |
| Sources | `/v1/sources` | Preserve | Core signal-plane view |
| Agent types | `/v1/agent-types*` | Transitional | Preserve if used as primary UX; may later be subsumed by project-scoped agent definitions |
| Digest and actions | `/v1/digest*`, `/v1/actions/digest*`, `/v1/actions/exemplars*`, `/v1/actions/score` | Preserve | Directly tied to operator workflows and eval loops |
| Prompt patches | `/v1/patches*` | Preserve | Needed during prompt registry transition |

## SSE Event Catalog

Current UI-referenced event names from `/v1/stream`:

| Event | Classification | Notes |
|---|---|---|
| `ready` | Preserve | Stream bootstrap contract |
| `task_update` | Preserve | Core runtime status updates |
| `assistant_delta` | Preserve | Core interactive streaming |
| `assistant_end` | Preserve | Core interactive streaming completion |
| `assistant_reasoning` | Preserve | Important operator/debug surface |
| `assistant_tool_call` | Preserve | Required for tool/run inspection |
| `tool_executed` | Preserve | Required for tool/run inspection |
| `memory_proposed` | Preserve | Core memory review workflow |
| `memory_accepted` | Preserve | Core memory review workflow |
| `identity_patch_applied` | Transitional | Keep while scoped asset model replaces singleton files |
| `session_event` | Preserve | Core session/run observability |
| `agent_activity` | Preserve | Core operator observability |
| `agent_heartbeat` | Transitional | Preserve initially; final shape may become richer runtime health telemetry |
| `skill_installed` | Transitional | Depends on final skill/default packaging model |
| `skill_proposed` | Transitional | Depends on final skill/default packaging model |
| `mcp_connection` | Transitional | Adapter-era event; may be absorbed by generic plugin connection events |
| `subagent_started` | Preserve | Core orchestration surface |
| `subagent_progress` | Preserve | Core orchestration surface |
| `subagent_completed` | Preserve | Core orchestration surface |
| `pr_update` | Transitional | Preserve if PR inbox remains in v1 |
| `task_paused` | Preserve | Core runtime control/inspection |
| `task_resumed` | Preserve | Core runtime control/inspection |
| `rule_executed` | Preserve | Core signal automation/operator visibility |

## Compatibility Notes

- Preserve semantics first, not exact handler internals.
- Transitional surfaces must either:
  - have a compatibility wrapper in Rust, or
  - have a documented UI migration path before removal.
- Marketplace-specific routes are intentionally outside the v1 control-plane core.
- The scoped-asset transition must keep existing profile/identity UIs working long enough for operators to migrate content into tenant/workspace/project assets.

## Known Follow-Ups

- Add request/response fixture references for preserve and transitional routes.
- Add payload-shape notes for the highest-risk SSE events:
  - `task_update`
  - `assistant_delta`
  - `assistant_end`
  - `session_event`
  - `subagent_*`
