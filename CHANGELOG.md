# Changelog

All notable changes to cairn-rs are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added

- **Traces page detail drawer + cross-entity links + scope-aware query
  (closes #241).** `TracesPage` now opens a right-side `Drawer` when an
  operator clicks (or keyboards onto) any row, surfacing the full
  metadata cairn persists for each provider call — trace id, model,
  provider, latency, cost, prompt/completion tokens, status, timestamp
  — plus deep links out to the owning session and run. The drawer is
  explicit about *not* rendering prompt or completion bodies, because
  cairn deliberately keeps that content off the trace read model, and
  points operators at the session/run pages for transcripts instead.
- **`session_id` and `run_id` as navigable links in the traces table.**
  Both ids were previously rendered as opaque `shortId` text; they are
  now `#session/:id` / `#run/:id` anchors so hopping from a slow
  provider call to its originating run or session no longer requires a
  manual URL edit. A new `Run` column also appears on md-and-wider
  viewports.
- **`defaultApi.getTraces` is now scope-aware.** The helper used to
  take a bare `limit: number`; it now accepts `{ limit?, tenant_id?,
  workspace_id?, project_id? }`, folds the current `useScope()` values
  in via `withScope()`, and `TracesPage`'s React-Query `queryKey`
  includes the scope tuple so switching tenants/workspaces/projects
  invalidates the cache instead of flashing stale rows from another
  project. Existing call sites in `DashboardPage`, `CostsPage`, and
  `GlobalSearch` were migrated to the new object form.

- **Workspace soft-delete via `DELETE /v1/admin/tenants/:t/workspaces/:w`
  (closes #218).** Before this there was no way to remove a workspace
  short of wiping the event log — every typo or stale dev workspace
  piled up on the operator's sidebar forever. The backend now accepts
  a `DELETE` on the workspace resource, emits a new
  `WorkspaceArchived` event, and stamps `archived_at` on the
  projection row. The list endpoint filters archived workspaces by
  default; passing `?include_archived=true` surfaces them again for
  audit. The in-memory, SQLite, and Postgres projections all track
  the new column (SQLite adds it via best-effort `ALTER TABLE` for
  existing dev databases; Postgres via a new `V020` migration).
  `WorkspacesPage` gains a per-card **Delete** action (protected by a
  confirm dialog, hidden on the currently-active workspace) wired to
  a new `defaultApi.deleteWorkspace` helper; `WorkspaceRecord` picks
  up an optional `archived_at` field on the UI side.

### Fixed

- **`cairn_http_*` Prometheus counters now advance with live traffic
  (closes #243).** The binary-side `metrics_prometheus_handler` was
  reading from a binary-local `AppMetrics` struct that no middleware
  ever wrote to — so `cairn_http_requests_total`,
  `cairn_http_latency_ms`, and `cairn_http_error_rate` all stayed at 0
  despite sustained traffic. The handler now reads from the lib-side
  `AppMetrics` that the `observability_middleware` populates on every
  request. The orphaned binary struct was removed and new public
  accessors (`http_total_requests`, `http_errors_by_status`,
  `http_requests_by_path`, `http_avg_latency_ms`,
  `http_latency_percentile`, `http_error_rate`) expose the existing
  histograms without duplicating the recording path. A new integration
  test (`test_http_metrics_middleware`) drives mixed 2xx/4xx traffic
  through the router and asserts the counters advance.
- **`GET /v1/admin/logs` returns a clean `{entries, limit, total}`
  response, and the LogsPage "Showing N of M buffered" footer now
  renders (closes #237).** The handler previously emitted five fields
  (`entries`, `limit`, `total`, `buffered`, `has_more`) while the UI
  typed only `buffered?`/`has_more?` as optional — the readable
  `entries.length` came through but the derived counts drifted. The
  handler is now trimmed to the three canonical fields (`total` is the
  ring buffer depth; clients derive "there is more" from
  `entries.length >= limit && total > entries.length`), and
  `LogsPage` + `RequestLogsResponse` were aligned so the footer
  surfaces real counts instead of the static ring-buffer fallback.
- **`GET /v1/skills/:id` returns 404 JSON for unknown ids even behind
  the binary's SPA catch-all (closes #236).** The route is registered
  in `build_catalog_routes` and the existing lib-level test covered
  it, but nothing asserted the bin-level composition
  (`catalog.merge(binary).fallback(serve_frontend)`) didn't accidentally
  drop the match. A new regression test in `test_http_skills` stands up
  the full bin-router shape in-process — catalog routes + empty binary
  merge + 200-text/html SPA fallback — and pins both the JSON 200 for a
  known id and the `application/json` 404 with the canonical
  `skill_not_found` envelope for an unknown one.
- **`GET /v1/tasks` + `GET /v1/runs` now honor the
  `tenant_id`/`workspace_id`/`project_id` query filters (closes
  #234).** `list_tasks_filtered` in the in-memory store was declared
  with underscore-prefixed scope arguments and returned every task in
  the store regardless of the caller's requested scope, so an admin
  querying `/v1/tasks?tenant_id=A&workspace_id=B&project_id=C` got
  tasks from other tenants mixed into the response — a cross-tenant
  leak on the operator list surfaces. The project key is now applied
  alongside the existing `run_id` and `state` narrowing predicates
  (mirroring the run-list contract from #184), with a new integration
  test seeding entities in two tenants and asserting neither leaks
  into the other's list. The run-list path was already wired
  correctly; the new test locks in that behavior too so a future
  refactor can't silently regress it the way #234 did.

- **DecisionsPage Cache tab crashed with "Objects are not valid as a
  React child" (closes #240).** The UI `CacheEntry.outcome` type
  declared `string`, but `GET /v1/decisions/cache` emits the same
  nested `{outcome, deny_reason?}` struct as `Decision.outcome`, so
  `OutcomePill` received an object and React bailed. `scope` was
  similarly a `ProjectScope` object being rendered directly. The
  `CacheEntry` type now mirrors the wire shape, the outcome column
  reads `r.outcome.outcome`, and `scope` is folded to a
  `tenant/workspace/project` label via `scopeLabel`. Row keys moved
  from the non-existent `key` field to `decision_id`.

- **AuditLog "Older" pagination permanently disabled (closes #239).**
  The backend returns `cairn_api::ListResponse<T>` which serializes
  via `#[serde(rename_all = "camelCase")]`, so the wire envelope is
  `{items, hasMore}`, but `AuditLogResponse` in `ui/src/lib/types.ts`
  and the reader in `AuditLogPage.tsx` both read `has_more`. The
  boolean was always `undefined`, so `Older` was always disabled.
  Types and the reader now use `hasMore` to match the wire
  contract.

- **Eval `dataset_id` lost on restart (closes #220).** `POST
  /v1/evals/runs` persisted an `EvalRunStarted` event so runs would
  survive a reboot, but the event only carried prompt-asset /
  version / release / created_by fields — not the dataset binding
  submitted alongside them. `replay_evals()` rebuilt each run from
  the event and silently dropped the dataset linkage, so after a
  reboot `GET /v1/evals/runs/:id` came back with `dataset_id: null`
  even though the operator had picked one in the form.
  `EvalRunStarted` now carries an optional `dataset_id` (defaulted
  via `#[serde(default)]` for backward-compatibility with pre-#220
  event log entries). The handler populates it from the request
  body and `replay_evals()` calls `set_dataset_id` on the
  in-memory eval service when the event carries one, so the
  binding round-trips through a full sigkill + replay cycle.

- **GraphPage — full node-kind coverage + 5 provenance query tabs
  (closes #151).** The simulation view previously collapsed all 22
  backend `NodeKind` variants down to `session` / `run` / `task` via a
  `toSimKind` filter, silently hiding approvals, checkpoints, triggers,
  mailbox messages, tool invocations, route decisions, provider calls,
  memories, documents, chunks, sources, ingest jobs, signals, prompt
  assets/versions/releases, eval runs, skills, and channel targets.
  The filter is gone; every emitted kind now renders with its own
  radius / fill / stroke, the legend rebuilds from live counts, and the
  simulation cap was raised from 100 to 150 nodes. A new **Queries**
  tab exposes thin wrappers around the five previously unused graph
  endpoints — `GET /v1/graph/execution-trace/:run_id`,
  `/v1/graph/dependency-path/:node_id`,
  `/v1/graph/retrieval-provenance/:run_id`,
  `/v1/graph/prompt-provenance/:release_id`, and
  `/v1/graph/multi-hop/:node_id` — each with a small form for its
  params (ID, max-depth/hops, and for multi-hop `min_confidence` +
  `direction`) and a render pane reusing the existing force-graph
  primitive. `GraphNodeKind` picked up the missing `trigger` variant
  and `GraphEdgeKind` picked up `matched_by` and `fired` so the
  TypeScript types match the Rust enum 1:1.

- **EvalsPage — real eval-run contract with dataset / rubric / baseline /
  prompt-release pickers (closes #138).** The "New Eval Run" popover used
  to collect only `evaluator_type` and `subject_kind`; the backend accepted
  the submission and returned a 201 with a run id, but the run was a
  no-op stub — no dataset, no rubric, no scorecard. `POST /v1/evals/runs`
  now additionally accepts (and validates against tenant state) optional
  `dataset_id`, `rubric_id`, and `baseline_id`. Dangling ids return 404
  instead of being silently ignored. New list endpoints
  `GET /v1/evals/rubrics` and `GET /v1/evals/baselines` (wrapping the
  existing `EvalRubricService::list` / `EvalBaselineService::list` — both
  tenant-scoped via `?tenant_id=`) back the pickers. The form now fetches
  datasets, rubrics, baselines, and — when `subject_kind=prompt_release`
  — prompt releases, letting the operator wire a real subject to a real
  eval configuration. A "Results" link on every row navigates to
  `#eval-results/:run_id`, which reuses `EvalComparisonPage` in
  single-run mode and hydrates metrics from `/v1/evals/compare`. Locked
  by integration test
  `cairn-app/tests/test_http_evals_full.rs::eval_run_full_contract_roundtrip`.

- **ChannelsPage — real `/v1/channels` CRUD UI (closes #139).** The
  `/v1/channels` runtime-channel API (create/list/send/consume/messages
  on `cairn-runtime::ChannelService`) had no operator UI. A new
  `ChannelsPage` now wires it up: project-scoped list with name /
  channel_id / capacity / created columns, a "New Channel" modal
  (name + capacity), a per-row "Send" modal (sender_id + body), and a
  per-row "Messages" drawer that polls `/v1/channels/:id/messages`.
  `api.ts` gains `listChannels`, `createChannel`, `sendToChannel`,
  `getChannelMessages`, `consumeChannelMessage`; `types.ts` gains
  `Channel`, `ChannelMessage`, `CreateChannelRequest`,
  `SendChannelMessageRequest`, `SendChannelMessageResponse` mirroring
  the `cairn_domain::{ChannelRecord, ChannelMessage}` Rust types. New
  integration test
  `crates/cairn-app/tests/test_http_channels.rs::channel_create_send_list_roundtrip`
  pins the create → send → list contract end-to-end against a live
  `cairn-app` subprocess.

- **LogsPage + AuditLogPage — time-range filter, page-size control, and
  cursor pagination (closes #163).** Both pages previously hardcoded a
  single fetch (500 / 200 entries) with no way to scroll into older
  history. The admin request-log handler now accepts `since_ms`; the
  audit-log handler now accepts `before_ms` (exclusive upper bound) in
  addition to the existing `since_ms`/`limit`, with the limit clamped to
  `[1, 1000]`. The UI gains last-15m / 1h / 24h / 7d / all time-range
  dropdowns, a 50/100/250/500 page-size picker, and — for the audit log
  — prev / next / jump-to-newest pagination driven by a `before_ms`
  cursor stack. The request-log response now also returns `buffered` +
  `has_more` so the footer can show "showing N of M" and surface a
  hint when the page was truncated.

- **`GET /v1/skills` + `GET /v1/skills/:id` — real skills catalog wiring.**
  Replaces the hard-coded empty stub
  (`list_skills_preserved_handler` in `handlers/memory.rs`) with a
  handler that reads a live `cairn_domain::skills::SkillCatalog` held
  on `AppState`. List returns the UI-expected
  `{items, summary, currently_active}` shape derived from the real
  `SkillSummary` records (`skill_id`, `name`, `description`,
  `version`, `tags`, `enabled`); `?tag=<tag>` filters by tag; detail
  endpoint returns the full `Skill` struct (with `entry_point`,
  `required_permissions`, `status`). `SkillsPage` now renders real
  skill metadata (skill id, version badge, tag pills) instead of
  dumping opaque `Record<string, unknown>` entries. The catalog
  starts empty; workers register skills via the domain API. The
  response body stays shape-compatible with the previous stub:
  `items`, `summary`, and both `currentlyActive` (camelCase, first)
  and `currently_active` (snake_case) keys are still emitted from a
  single shared list, so UI clients keyed on either name continue to
  work. `currently_active` includes a skill only when it is BOTH
  lifecycle-`Active` and `enabled` — the domain `disable()` only
  clears `enabled`, so gating on both avoids listing disabled skills
  under "Currently active".
  Integration tests at `crates/cairn-app/tests/test_http_skills.rs`
  cover list, tag-filter, detail, 404, disabled-skill handling, and
  empty-state paths. Closes #147.
- **`RunDetailPage` + `OrchestrationPage` — operator run-mutation actions.**
  Wires the 10 mutation endpoints under `/v1/runs/:id/*` that had no UI
  consumer: **pause**, **resume**, **recover**, **replay**, **claim**,
  **spawn subagent**, **children list**, **orchestrate**, **diagnose**,
  and **intervene** (plus `GET /v1/runs/:id/interventions` history).
  `RunDetailPage` gains an Operator Actions toolbar (pause/resume
  state-aware, orchestrate/diagnose drawer, intervene & spawn modals,
  recover/claim gated behind `window.confirm`), a Children Runs
  subtable that navigates to each child on click, and an Interventions
  history drawer. `OrchestrationPage` gains per-row quick-action icons
  (pause/resume/orchestrate/diagnose/intervene) that disable themselves
  based on run state and invalidate the live orchestration tree on
  success. Closes #166 and #173.
- **`defaultApi` — new run-scoped methods**: `recoverRun`, `replayRun`,
  `claimRun`, `spawnSubagentRun`, `listChildRuns`, `orchestrateRun`,
  `diagnoseRun`, `interveneRun`, `listRunInterventions`, plus widened
  `pauseRun` / `resumeRun` signatures that accept the full
  `PauseRunRequest` / `ResumeRunRequest` bodies (reason kind, actor,
  trigger, target). Matching TypeScript types in `lib/types.ts`
  (`PauseReasonKind`, `ResumeTrigger`, `RunResumeTarget`,
  `SpawnSubagentRequest`, `InterventionAction`, `InterveneRequest`,
  `InterventionRecord`, …) mirror the Rust DTOs in
  `crates/cairn-app/src/handlers/runs.rs`.
- **`test_http_run_operator_actions.rs`** integration test covering
  pause/resume endpoint wiring, `spawn → list_children` roundtrip, and
  `intervene → list_interventions` against the live HTTP server via
  `LiveHarness`.
- **`WorkersPage` now reads the real worker registry.** The page used to
  synthesise "workers" by grouping `GET /v1/tasks` rows by `lease_owner`,
  which reported zero workers whenever no task was currently leased —
  even with a dozen registered workers heartbeating. It now calls
  `GET /v1/workers` and `GET /v1/fleet` on mount (polling every 10 s),
  renders a fleet summary strip (total / active / healthy / suspended),
  a workers table (id + display name, tenant, status, active task count,
  last heartbeat, registered-at), and operator actions for Suspend /
  Reactivate wired to `POST /v1/workers/:id/suspend` and
  `/v1/workers/:id/reactivate`. Worker-detail navigation links to
  `#worker/<id>` as a stub — a dedicated detail page is a follow-up.
- **`defaultApi.listWorkers` / `getWorker` / `getFleet` /
  `suspendWorker` / `reactivateWorker`** on the TypeScript API client,
  with matching `WorkerRecord`, `WorkerHealth`, `FleetWorkerState`, and
  `FleetReport` types mirroring the `ExternalWorkerRecord`,
  `WorkerState`, and `FleetReport` Rust shapes.
- **`test_http_worker_registry.rs`** integration test covering register
  → list → get → suspend → fleet → reactivate against the live HTTP
  server via `LiveHarness`.
- **`ProjectReposPage` — attach/detach GitHub repos per project.** New
  operator page under the Infrastructure group that consumes the RFC 016
  `/v1/projects/:project/repos` surface (`GET` / `POST` / `GET :owner/:repo`
  / `DELETE`). Operators can now allowlist a repo against the current
  project scope straight from the UI, which unblocks the dogfood workflow
  of kicking off issue-sync runs against a real GitHub repo. The page is
  scope-aware (`useScope`), slash-path-encodes the project segment the
  same way `TriggersPage` does (PR #132), and invalidates the
  `["project-repos", projectPath]` query on every mutation. Four new
  typed API client methods — `listProjectRepos`, `attachProjectRepo`,
  `getProjectRepo`, `detachProjectRepo` — and matching `ProjectRepoEntry`
  / `ProjectRepoMutation` / `ProjectRepoDetail` TypeScript types land in
  `ui/src/lib/{api,types}.ts`. Integration coverage added at
  `crates/cairn-app/tests/test_http_project_repos.rs`
  (attach → list → get → detach → list-empty roundtrip + malformed-id
  400 contract).

### Changed

- **ChannelsPage → NotificationsPage rename (closes #139).** The
  legacy `ChannelsPage.tsx` was misnamed — it managed per-operator
  notification preferences at `/v1/admin/operators/:id/notifications`,
  not the `/v1/channels` CRUD surface. The file is renamed to
  `NotificationsPage.tsx`, the nav item splits into two entries under
  Infrastructure ("Channels" for runtime channels, "Notifications" for
  notification preferences), and the `NavPage` union in `Sidebar.tsx`
  / `Layout.tsx` gains a `notifications` key. Existing
  `/v1/admin/operators/:id/notifications` behaviour is unchanged.

### Removed

- **`list_skills_preserved_handler` stub (part of #147).** The
  hard-coded empty stub at
  `crates/cairn-app/src/handlers/memory.rs:1588-1603` that returned
  `{items: [], summary: {total: 0, enabled: 0, disabled: 0}}` for
  every `GET /v1/skills` request is deleted; the route is now served
  by the real handler in `handlers/skills.rs` backed by the domain
  `SkillCatalog`.

### Fixed

- **Backend: `EvalRunStarted` persists `dataset_id` / `rubric_id` /
  `baseline_id` (closes #223, supersedes #220).** `POST /v1/evals/runs`
  accepted the three linkage fields in the request body but dropped
  them before emitting the `EvalRunStarted` event, so the binding was
  invisible on `GET /v1/evals/runs/:id` and vanished entirely on
  event-log replay. The event now carries the three ids (all
  `Option<String>` with `#[serde(default)]` for backward-compat),
  `create_eval_run_handler` threads them from the request into the
  event and the in-memory `EvalRun` record via
  `set_dataset_id` / `set_rubric_id` / `set_baseline_id`, and
  `AppState::replay_evals` re-binds them on boot. Covered by
  `tests/test_http_evals_full.rs::eval_run_full_contract_roundtrip`
  and a new replay-survives-restart assertion.

- **Backend: `GET /v1/evals/rubrics` + `/v1/evals/baselines` return
  200 with the list shape the UI consumes (closes #223).** The routes
  are registered in the preserved catalog and the axum fold; this PR
  locks the contract with an integration test so they cannot regress
  to 405 again.

- **Backend: `POST /v1/prompts/releases/:id/request-approval` routed to
  the wrong handler (closes #222).** QA2 Slice 9 surfaced a 422 on every
  "Request Approval" click (`missing field approval_id`). Two routes
  registered the path — the later (and winning) one dispatched to the
  generic `request_approval_handler`, which deserializes the body into
  `RequestApprovalRequest { tenant_id, workspace_id, project_id, approval_id, … }`
  and 422'd on the empty body the UI posts. Fixed the duplicate route
  to use `request_prompt_release_approval_handler`, which takes only
  the path id and looks up the release's project server-side. The UI
  `requestPromptReleaseApproval` call now also wraps its body via
  `withScope(…)` for consistency with `createPromptVersion`,
  `createPromptRelease`, and `getPromptReleases`. `test_http_prompts.rs`
  now covers the full asset → version → release → request-approval
  roundtrip as a regression guard.

- **UI: `MemoryPage` — ingest `source_type` is now a dropdown of valid
  enum values (closes #219).** The ingest form shipped a free-text input
  with a placeholder suggesting `web, file, api, …` — none of which are
  valid; the backend rejected every submission with 422. Replaced with
  a `<select>` populated from the real `SourceType` enum (`plain_text`,
  `markdown`, `html`, `structured_json`, `json_structured`,
  `knowledge_pack`), defaulting to `plain_text`. The request field name
  is unchanged so the API contract is untouched.

- **Backend: `POST /v1/runs/:id/pause` returns 409 on invalid state
  (closes #216).** Pausing a run that was not yet claimable (pending
  state, no lease) previously crashed into a 500: FF's
  `ff_suspend_execution` rejects with `fence_required` /
  `execution_not_active`, which rolled up as `FabricError::Internal`
  and then `RuntimeError::Internal`. The fabric adapter now classifies
  the documented suspend-state rejection codes (`fence_required`,
  `partial_fence_triple`, `execution_not_active`, `lease_revoked`,
  `stale_lease`, `invalid_lease_for_suspend`, `already_suspended`,
  `waitpoint_not_token_bound`) as `RuntimeError::InvalidTransition`,
  and the HTTP error mapper returns 409 Conflict with code
  `invalid_state_transition` for every `InvalidTransition` variant
  (was 422). Regression covered end-to-end by
  `test_http_run_operator_actions::pause_on_pending_run_returns_409_not_500`.
- **Backend: duplicate credential for the same `(tenant, provider_id)`
  returns 409 (closes #217).** `POST /v1/admin/tenants/:t/credentials`
  silently accepted repeated posts with the same `provider_id`, both
  returning 201 and accumulating two active records in the read model.
  `CredentialServiceImpl::store` now rejects when an active credential
  already exists for the pair, surfacing `RuntimeError::Conflict`; the
  admin handler re-shapes this into a 409 with code
  `credential_exists` and a message naming the tenant and provider.
  Revoke-then-create still succeeds, preserving the rotation workflow.
  Regression covered by `test_http_credentials`.
- **UI: `PromptsPage` — create initial version alongside asset (#150).**
  `NewPromptForm` previously only posted to `/v1/prompts/assets`, leaving
  authors with an asset they could not release without a curl step. The
  form now accepts an optional "Initial version" textarea; on submit, the
  UI sequentially creates the asset and, when the body is non-empty,
  posts the first `PromptVersion` with a SHA-256 `content_hash` computed
  in the browser. Backend `POST /v1/prompts/assets/:id/versions` and
  `POST /v1/prompts/releases` now accept the request with
  `prompt_version_id`/`prompt_release_id` omitted and mint `pv_<uuid>`
  and `rel_<uuid>` respectively, so the UI no longer fabricates IDs
  client-side. `createPromptVersion` + `createPromptRelease` now route
  through `withScope(body)` (matching `createPromptAsset`) so the
  sequential flow inherits the active tenant/workspace/project scope.
  Asset-only creation (blank textarea) still works. Closes #150.
- **Backend: admin bypass on `POST /v1/runs/:id/spawn` and
  `POST /v1/runs/:id/intervene`.** Both handlers compared the run's
  `project.tenant_id` to the principal's tenant without an admin
  short-circuit, so the `admin` service account (hard-bound to
  `TenantId("default")`) returned a `404` with error code `"not_found"`
  and message `"run not found"` for any run in another tenant —
  inconsistent with `replay_to_checkpoint` and other
  sibling handlers that already honor `tenant_scope.is_admin`. Surfaced
  while wiring the run-detail operator pages (PR P / PR O'). Both
  handlers now read `tenant_scope.is_admin || run.project.tenant_id ==
  *tenant_scope.tenant_id()`, matching the existing pattern at
  `runs.rs:775`. Spawn keeps the strict parent-child project match on
  the child session lookup — admin does not enable cross-tenant child
  spawning. Downstream event-stamping and notification sites inside
  `intervene_run_handler` (5 sites: `append_run_intervention_event`,
  `OperatorIntervention.tenant_id` for `force_fail` / `force_restart`,
  and `notify_if_applicable`) now read the run's actual tenant from
  `run.project.tenant_id` instead of `tenant_scope.tenant_id()`, so
  an admin crossing tenants does not mislabel intervention events
  or misroute SSE notifications (fixed pre-merge after Cursor Bugbot
  flagged the regression on the first admin-bypass pass).
- **Backend: `event_message()` no longer renders Plan* events as
  `"unknown"`.** `PlanProposed`, `PlanApproved`, `PlanRejected`, and
  `PlanRevisionRequested` had entries in `event_type()` but fell into
  the catch-all fallthrough arm in `event_message()`, so every plan
  lifecycle event showed up in SSE frames and audit payloads with a
  message of `"unknown"`. Added dedicated arms that include the plan
  run id plus the relevant actor / reason / revision id so operators
  can read the message without decoding the event payload.

- **UI: `DashboardPage` — real widgets, no placeholders (#179).** Three
  bugs fixed in one pass: (A) the Runs / Tasks tabs rendered empty or
  dummy content — now render compact, live tables driven by
  `defaultApi.getRuns({ limit: 50 })` / `defaultApi.getAllTasks({
  limit: 50 })` filtered to active states, sorted newest-first, capped
  at 8 rows, auto-refreshing every 5 s, mirroring the row pattern from
  the existing `ActiveRunsWidget`. (B) `ProviderStatusWidget` hardcoded
  three rows (`Store / Events / Memory`) against `/v1/health/detailed`;
  it now iterates the real `components` array from `/v1/status`,
  normalizes status tokens (`ok`/`healthy`/`degraded`/`unhealthy`/
  `unconfigured`) and pretty-prints snake_case names. New components
  added server-side appear automatically with no UI change. (C) the
  `CostWidget` trend was dead code — the previous-snapshot query was
  `enabled: false` so the arrow never rendered. Replaced with a
  `useRef`-tracked prior total updated after each successful fetch;
  widget now shows an up / down / flat arrow plus a signed percent
  delta (`+12.3%` / `-4.1%` / `0.0%`) vs. the previous 30 s snapshot.
  The `displayedPrev` baseline only advances when `dataUpdatedAt`
  changes, so ordinary re-renders don't collapse the delta to flat.
  Also fixed the event-sparkline bucketing bug: `useHourlyEventCounts`
  now prefers numeric `stored_at` (backend truth) and falls back to
  ISO `timestamp` — previously it only read `timestamp`, dropping
  every `RecentEvent` that arrived with only the numeric field.
  Hash navigation from the Run / Task rows is now URL-encoded, to
  match the rest of the UI's routing pattern.
- **UI: `TestHarnessPage` "Run All Scenarios" no-op'd scenario cards (#143).**
  The page-level Run All handler ran its own private copy of each
  scenario's steps against the server and then bumped a `runAllKey`
  to remount cards. Cards themselves never executed — step rows
  stayed idle ("0/N steps") while the summary banner reported
  green. Rewired Run All so each `ScenarioCard` accepts a per-card
  `runNonce` prop; the parent increments nonces sequentially and
  awaits an `onComplete` callback from the card's real
  `runScenario` before moving to the next. Step rows now reflect
  live per-step progress during Run All, and the summary reflects
  what the cards actually saw. Closes #143.
- **UI: `TestHarnessPage` "Event log" step was a sham (#143).**
  The `event_log` step called `getRunEvents("__nonexistent__")`
  with a blanket `.catch(() => [])` and reported pass regardless
  of server state — it asserted nothing. Replaced with a real
  `GET /v1/events/recent?limit=5` probe via `getRecentEvents` that
  asserts the response is an array and each returned record has a
  non-empty `event_type`. Description updated to match.
- **UI: `AgentTemplatesPage` navigated after a 500 ms `setTimeout` (#161).**
  After `instantiateAgentTemplate` succeeded the page slept 500 ms before
  routing to the new run, hoping the backend had finished creating it —
  a pure race. The endpoint returns the `run_id` synchronously in its
  201 response, so the page now navigates immediately on success. No
  more stale-detail flash, no more missed-run when the handler is slow.
- **UI: `ProjectDashboardPage` rendered tenant-wide cost widgets as if
  they were project-scoped (#144).** `/v1/costs` is tenant-scoped on the
  backend (`SessionCostRecord` has `tenant_id` only; no workspace/project
  fields on the event or projection), so the "Total Spend" and "Provider
  Calls" cards aggregated across every workspace and project in the
  tenant while the labels suggested a single project. The cards are now
  labeled "Tenant Spend" / "Tenant Provider Calls" with a description
  that says "authenticated tenant — not project-scoped"; the Resources
  summary sub-label reads "tenant-wide". The copy avoids interpolating
  the UI's `tenantId` state because the backend derives the tenant from
  the bearer token (`TenantScope`) and ignores any UI-side tenant
  filter, so hardcoding a tenant id in copy would mis-attribute spend
  whenever the scope selector and the bearer token disagree. Proper
  project-scoped cost filtering needs a backend change (event shape
  extension + projection); tracked separately.
- **UI: `OrchestrationPage` re-processed the oldest buffered SSE event
  on each SSE update and leaked `setTimeout` callbacks on unmount
  (#177).** The stream effect depended on the whole `streamEvents`
  array and read `streamEvents[length - 1]`, but `useEventStream`
  prepends frames (newest-first), so each new stream frame re-handled
  the OLDEST buffered event — spurious refetches + fresh highlights
  per unrelated state change — and the 3 s fresh-clear `setTimeout`
  it scheduled had no cleanup (timeouts accumulated for the life of
  the tab). Fixed by iterating `streamEvents` in reverse so events
  are processed exactly once in causal order (dedupe via a
  `useRef<Set>` of server-assigned event ids, capped at
  `STREAM_BUFFER_MAX * 5` (currently 250) with
  oldest-insertion-order eviction), tracking pending highlight
  timeouts in a `useRef<Map>` that clears them on supersede +
  unmount, and coalescing refetches so an event burst triggers at
  most one `rSessions`/`rRuns`/`rTasks`. Payload ids read both
  snake_case and camelCase to match the pattern in `RunDetailPage`.
  Operator actions wired in PR P are untouched.
- **UI: `PlaygroundPage` model picker excluded Anthropic-native providers (#160).** The registry-derived model list filtered out providers whose `api_format` was `anthropic`, silently hiding every Anthropic-native connection even though the backend `chat/stream` handler routes through the native adapter. Removed the adapter-kind filter so all available registered providers contribute their models to the picker. Closes #160.
- **UI: `DecisionsPage` row-level Invalidate button was invisible (#153).**
  The per-row "Invalidate" button in the cache tab used Tailwind's
  `opacity-0 group-hover:opacity-100` reveal pattern, but the shared
  `DataTable` `<tr>` did not declare the `group` class, so the
  `group-hover:` variant never activated and the button stayed
  permanently hidden. Added an opt-in `rowClassName` prop to
  `DataTable` (keeps the shared component free of implicit Tailwind
  scopes) and pass `rowClassName="group"` from `DecisionsPage` for
  the cache tab so hover-revealed row actions become visible on
  hover. Closes #153.
- **UI: `CredentialsPage` Add-Credential modal missing toast on store error + stale `default` placeholder (#162).**
  The `storeCredential` mutation only declared `onSuccess`, so any
  failure (invalid provider, encryption key lookup miss, transport
  error) was only shown via the modal's inline `mutErr` text and did
  not surface through the shared toast — inconsistent with the rest
  of the app (ApprovalsPage, DecisionsPage) where operator-initiated
  mutations always raise a toast on failure. Added an `onError`
  handler that surfaces the error message via `useToast()`, matching
  the `ApprovalsPage` pattern. Also replaced a stale
  `placeholder="default"` string literal on the Tenant input with
  `DEFAULT_SCOPE.tenant_id` from `ui/src/lib/scope.ts` so the
  placeholder stays in sync with the canonical default tenant id
  (`default_tenant`) — the PR #132 cleanup missed this one field.
  Closes #162.
- **UI: `RunDetailPage` plan-mode detection used a loose substring match (#178).**
  The `isPlanMode` check in `ui/src/pages/RunDetailPage.tsx` used a loose
  substring match (`runModeType.includes("plan")`), so runs with names or
  mode strings that merely contained the substring `plan` (e.g.
  `"deploy-plan"`, `"reviewplan"`) triggered the Plan Mode review panel
  spuriously. Replaced with an exact match on the typed
  `cairn_domain::RunMode` discriminator (`runModeType === "plan"`), with
  the existing `hasPlan` fallback retained for legacy plan-artifact rows.
- **UI: `RunDetailPage` terminal-state set missed `dead_lettered` (#178).**
  The page-local `TERMINAL_STATES` set used for disabling operator
  actions, stamping the run-end on the Gantt chart, and choosing the
  "running"/"total" task stat label only listed `completed | failed |
  canceled`. Aligned with `cairn_domain::RunState::is_terminal()` and
  defensively added `dead_lettered` (bubbled up from
  `TaskState::DeadLettered` if a DLQ'd row is ever surfaced as a
  run-level state); `retryable_failed` is intentionally excluded
  because it is pending-retry, not terminal. The two inline duplicate
  literals lower in the file now reuse the named set so the three sites
  can't drift apart.
- **UI: `TriggersPage` swallowed backend failures on raw `fetch` calls (#154).**
  Replaced the 5 raw `fetch` calls (list triggers, list run-templates,
  enable/disable/delete trigger) with new `defaultApi.listTriggers` /
  `listRunTemplates` / `enableTrigger` / `disableTrigger` /
  `deleteTrigger` methods that route through `apiFetch` and throw on
  non-2xx. Added `onError` toasts to all three mutations so operators
  see the real backend reason instead of a lying "Trigger enabled."
  toast after a 4xx/5xx. DecisionsPage was already fixed in PR #131;
  this closes the TriggersPage half.

- **UI: `SessionsPage` per-row run count was O(N*M) per render (#180).**
  Replaced the per-row `allRuns.filter(...)` scan with a memoized
  `Map<session_id, count>` computed once from `allRuns`, collapsing
  per-render work from O(sessions * runs) to O(runs) build + O(1)
  lookup.
- **UI: `ApprovalsPage` 24h stats double-counted pending requests (#176).**
  The "Approved (24h)" and "Rejected (24h)" stat cards filtered resolved
  approvals by `created_at`, so approvals requested within the last 24h
  were counted regardless of when (or whether) they were decided, and
  approvals resolved recently but requested earlier were missed. Switched
  both filters to `updated_at`, which the backend `ApprovalRecord`
  projection stamps on every decision write — effectively the resolution
  timestamp for resolved records. `updated_at` was already serialized by
  the handler but was missing from the UI `ApprovalRecord` type and the
  OpenAPI schema; both are now aligned with the wire shape.
- **UI: `TasksPage` table rows were not clickable (#181).** Added
  `onRowClick` to the DataTable so clicking a task row navigates to the
  parent run detail page (`#run/:id`), mirroring the behaviour of the
  kanban task cards. Rows without a `parent_run_id` remain non-clickable.
- **UI: `TasksPage` kanban board omitted DLQ columns (#175).** Added
  `retryable_failed` ("Retryable Failed") and `dead_lettered`
  ("Dead Lettered") kanban columns so tasks that drop into the
  retry/dead-letter queues are visible alongside the other states. The
  Rust `TaskState` enum in `crates/cairn-domain/src/lifecycle.rs` already
  supports both variants; the UI now renders them.
- **UI: `ApiDocsPage` endpoint catalog had drifted (#148).** Audited
  every documented entry against `router.rs` + `bin_router.rs` and added
  17 missing real routes: run operator mutations
  (`orchestrate`/`diagnose`/`intervene`/`spawn`/`children`),
  `GET /v1/runs/:id/replay`, `POST /v1/runs/:id/replay-to-checkpoint`,
  `GET /v1/sessions/:id/runs`, workers/fleet
  (`/v1/workers`, `/v1/workers/:id`, `/v1/fleet`), project repo
  allowlist (`/v1/projects/:project/repos` GET/POST +
  `/v1/projects/:project/repos/:owner/:repo` GET/DELETE), the skills
  catalog (`/v1/skills`), and `/v1/metrics/prometheus`. No existing
  entries were fakes once cross-checked — the `/v1/events/stream` case
  flagged in the QA slice was already fixed in an earlier PR.

- **UI: `MetricsPage` percentiles were always zero (#159).** The API
  client's `getMetrics` fetched `/v1/metrics` (JSON counters-only,
  no histogram buckets), so `p50_latency_ms` / `p95_latency_ms` /
  `p99_latency_ms` were never populated. It now fetches
  `/v1/metrics/prometheus`, and `parsePrometheusMetrics` was extended
  (not rewritten) with branches for the four metric names the Rust
  handler actually emits — `cairn_http_latency_ms{quantile="0.50|0.95|0.99|avg"}`
  direct gauges, `cairn_http_requests_by_path_total`,
  `cairn_http_error_rate`, and `cairn_http_errors_by_status` — with the
  existing histogram-bucket path kept as a defensive fallback for
  non-cairn Prometheus feeds. Follow-up to #131's dual-name parser fix.
- **UI: `RunsPage` detail side-panel was dead code (#169).** A
  `DetailPanel` component was rendered against a `selected`/`setSelected`
  state pair that nothing ever set — clicking a row already navigates to
  `#run/<id>`, so the panel was guaranteed never to appear. The unused
  component, state, and all orphaned imports (`X`, `ChevronRight`,
  `FieldRow`, `SectionLabel`, `sectionLabel`, `card` preset) have been
  removed (~60 LOC).
- **UI: `RunsPage` batch create now issues one HTTP call (#174).** The
  `BatchCreateModal` fan-out of N sequential `POST /v1/runs` requests
  has been replaced by a single `POST /v1/runs/batch` round-trip via
  `defaultApi.batchCreateRuns`. On partial failure the toast now
  surfaces the first per-item error message from the backend's
  `{results: [{ok, error}…]}` body instead of a generic "failed" line.
  Plan-mode parity is preserved by extending the backend
  `CreateRunBody` with an optional `mode` field so batch callers can
  opt into `RunMode::plan` the same way single-run create already does.
- **UI: `RunDetailPage` rendered `$0.000000` for every run with no
  provider calls (#168).** `GET /v1/runs/:id/cost` returns `200` with a
  zero-valued `RunCostRecord` when no cost data has been recorded, so
  the page's `cost ? … : "—"` check always fell into the truthy branch
  and displayed a misleading exact-zero dollar amount. The Cost stat
  card now renders "—" when both `provider_calls` and
  `total_cost_micros` are zero, and only shows the formatted amount
  (with the "N provider call(s)" description) once real cost data
  exists.
- **UI: canceling a run on `RunDetailPage` did not refresh the page
  header until the next poll tick (#167).** `cancelRunMut.onSuccess`
  invalidated `["runs"]` and `["run-events", runId]` but not
  `["run-detail", runId]`, so the `StateBadge` kept showing "running"
  for up to 10 seconds after the cancel succeeded. The mutation now
  also invalidates `["run-detail", runId]`, matching the pattern
  established in #131 for plan approve / reject / revise.
- **UI: `PromptsPage` enum values now match the Rust domain (#150).** The
  kind dropdown and release-state badges used values (`user`, `assistant`,
  `pending_approval`, `released`, `rolling_out`, `rolled_back`) that the
  backend's `PromptKind` / `PromptReleaseState` enums do not recognize, so
  action buttons fired transition requests the server rejected. Kinds are
  now `system`, `user_template`, `tool_prompt`, `critic`, `router` and
  release states are `draft`, `proposed`, `approved`, `active`, `rejected`,
  `archived`, all exported as typed literal unions in `ui/src/lib/types.ts`
  and mirrored by state-driven buttons that only fire transitions allowed
  by `PromptReleaseState::can_transition_to` in `cairn-evals`.
- **UI: `SessionDetailPage` silently dropped runs past the first 500 (#170).**
  The page fetched `GET /v1/runs?limit=500` and filtered by
  `session_id` client-side, so on projects with more than 500 total
  runs older session runs were cut out before the filter ran and
  simply disappeared from the detail view. The page now calls
  `GET /v1/sessions/:id/runs` (which filters server-side at the
  projection layer) via a new `defaultApi.getSessionRuns` helper, and
  paginates through all runs with named caps
  (`SESSION_RUNS_PAGE_SIZE` = 500, `SESSION_RUNS_MAX_PAGES` = 40). If
  the 20k-run hard cap is reached the page surfaces an explicit
  truncation banner directing operators to session export. The page
  also reads `isError`/`error` from the runs query, renders a dedicated
  "Session not found" red card on 404 and a generic error card for
  other failures, and short-circuits retries on 404. Integration
  coverage in `crates/cairn-app/tests/test_http_session_detail.rs`
  asserts that a session's runs are returned in full and that
  sibling-session runs under the same project scope do not leak.
- **UI: `WorkspacesPage` polish — surface create failures + drop dead stat
  tiles.** The `createWorkspace` mutation only had an `onSuccess` handler,
  so any failed POST (duplicate ID, 422 validation, 5xx) was silently
  swallowed: the form dialog closed with no feedback and the operator had
  no idea their workspace never landed. Added an `onError` handler that
  surfaces the error message via the shared `useToast` hook, matching the
  pattern from `ApprovalsPage` and the rest of the codebase. Separately,
  each workspace card rendered three stat tiles — Projects / Sessions /
  Runs — that were permanently pinned to `0` because the list endpoint
  `GET /v1/admin/tenants/:tenant_id/workspaces` only emits the
  `WorkspaceRecord` fields (id, name, timestamps) and no per-workspace
  aggregates, and the `workspaces` `useMemo` builder never populated
  the counters. The tiles were therefore actively misleading
  ("this workspace has zero runs" when it actually has many). Rather
  than extend the store layer across three backends to aggregate
  sessions/runs/projects per workspace for a list page, the tiles (and
  the parallel aggregate summary strip) have been removed; the card now
  shows workspace ID, tenant, active badge, and last-activity timestamp.
  Backend-sourced stats can be reintroduced in a follow-up if the
  `list_workspaces_handler` starts emitting them. Closes #140.
- **Provider UX: register → use in one wizard.** Three dogfood-blocker bugs
  in the provider-connection path collapsed into one chain — operators
  registered OpenRouter with empty `supported_models`, picked a model in
  Playground, and got a misleading 503 "set `OPENROUTER_API_KEY`" error.
  (a) `ProvidersPage` Step 3 now auto-runs `GET
  /v1/providers/connections/:id/discover-models` right after registration
  when the operator leaves the manual model list blank, patches
  `supported_models` with the result, and surfaces a warning toast if the
  provider returns nothing. Every connection row also gets a **Discover**
  action and an amber "no models registered" warning so stale rows are
  recoverable in one click. (b) `chat_stream_handler` /
  `ollama_generate_handler` / `ollama_embed_handler` no longer hardcode
  `TenantId::new("default_tenant")` — they resolve the tenant from
  `?tenant_id=` (falling back to the default) the same way
  `list_provider_connections` does, which was silently serving the wrong
  tenant's providers to multi-tenant operators. (c) When the tenant has
  active connections but none supports the requested model,
  `chat_stream_handler` now returns `422 Unprocessable Entity` with an
  actionable body — `"No registered connection for tenant '<t>' supports
  model '<m>'. Active connections: [...]. Register with POST
  /v1/providers/connections with supported_models including '<m>', or
  call discover-models to refresh."` — instead of the old 503 that
  pointed at env vars. Closes #156. Closes #157. Closes #158.
- **UI: `ModelPicker` on SettingsPage filtered to reachable + registered
  models.** The picker used to list every registry catalog entry
  regardless of `available=true` and whether any registered connection
  supported the model. Operators picked phantom models and fell straight
  into the misleading-503 chain above. It now filters to `available:
  true` models that are served by at least one registered connection for
  the active scope; when no connection exists, it falls back to the
  catalog with an explicit "register a provider to use this model"
  disclaimer.
- **UI: `CostsPage` + `ProjectDashboardPage` stat cards no longer stuck
  at 0.** `GET /v1/costs` returns `{items, hasMore}` (the standard
  `ListResponse<T>` camelCase envelope — a list of per-session cost
  records); the UI was typed as a flat `CostSummary` and
  `total_cost_micros` was `undefined` on every page. Added a
  `CostListResponse` / `SessionCostRecord` pair to `types.ts`, a
  `summariseCostItems()` helper in `api.ts` that folds items into the
  legacy `CostSummary` shape client-side, and wired it through both
  pages. TestHarnessPage's "Cost summary" probe updated to assert `items`
  instead of the removed top-level field.
- **UI: `PluginsPage` per-project enable/disable (405 → 200).** The
  Marketplace tab called `POST /v1/projects/:id/plugins/:pluginId/enable`
  and `POST …/disable`, but the real routes in `marketplace_routes.rs`
  are `POST /v1/projects/:proj/plugins/:id` (enable) and
  `DELETE /v1/projects/:proj/plugins/:id` (disable) — no `/enable` or
  `/disable` suffix, and disable is DELETE, not POST. Every operator
  click therefore 405'd. Additionally the `:proj` path param is parsed
  as `"tenant/workspace/project"` and silently falls back to
  `default_tenant/default_workspace/<id>` for 1-segment input — the
  same cross-tenant leak PR #132 closed for `TriggersPage`. Fix: (a)
  `defaultApi.enablePluginForProject` / `disablePluginForProject` now
  use the correct URL and HTTP method and accept a `ProjectScope`
  (percent-encoded slash path, mirror of `attachProjectRepo`); (b)
  `PluginsPage` drops the free-text `project_id` input and reads the
  active scope from `useScope()`, matching `TriggersPage` /
  `ProjectReposPage`. Locked down by
  `test_http_plugin_lifecycle.rs` (catalog → install → enable →
  disable roundtrip plus negative assertions on the old URL shapes).
- **Admin-token reads of `GET /v1/sessions/:id` (and the session's
  activity / active-runs / cost / llm-traces / events subresources) no
  longer return a spurious 404 on non-default tenants.** The handlers
  filtered by `tenant_scope.tenant_id()` without honouring
  `TenantScope.is_admin`, so admin-token callers got 404 for every
  session whose tenant differed from the admin's default-tenant
  binding — cascading into SessionDetailPage rendering "No traces" for
  every session. Handlers now mirror the `is_admin || tenant_match`
  pattern already used in `tasks.rs`/`runs.rs`/`approvals.rs`. Closes
  #164.
- **Bare list calls to `GET /v1/runs`, `GET /v1/sessions`, and
  `GET /v1/tasks` no longer return 422 when `tenant_id` /
  `workspace_id` / `project_id` query params are missing or empty.**
  The three query structs now declare the scope fields as
  `Option<String>` with a `#[serde(default)]` and fall back to
  `DEFAULT_TENANT_ID` / `DEFAULT_WORKSPACE_ID` / `DEFAULT_PROJECT_ID`
  when absent. The incognito / first-load UI path (and any quick curl
  probe) now gets 200 with the default-scope results. Closes #165.
- **`TasksPage`: removed duplicate unstyled Refresh button.** A merge-conflict
  artifact left two Refresh buttons in the toolbar — one unstyled, one
  properly styled. The unstyled duplicate has been removed; the styled
  Refresh button (with auto-refresh interval selector) remains. Closes #172.
- **`TasksPage`: batch cancel now invalidates `run-tasks`.** The
  `cancelSelected` mutation invalidated `['tasks']` on success but not
  the `['run-tasks']` prefix key, so `RunDetailPage` showed stale task
  state after a batch cancel because queries like `['run-tasks', runId]`
  were not refreshed. Now mirrors the `claim` / `release` mutation
  pattern and invalidates both `['tasks']` and the `['run-tasks']`
  prefix key. Closes #171.
- **UI: global 401 interceptor.** When an operator's token is rotated
  (via `POST /v1/admin/rotate-token`) or expires mid-session, the app
  used to turn into a wall of red error badges on every page because
  the token check only ran once on mount. A new `QueryCache` /
  `MutationCache` `onError` observer in `main.tsx` inspects every
  TanStack Query and mutation failure: on `ApiError.status === 401`
  it clears the stored token and dispatches a `cairn:auth-expired`
  event that the App shell listens for to bounce the operator back
  to the LoginPage. One rotated-token = one trip through login, not
  dozens of failed polls.
- **UI: SessionsPage rows are now clickable.** The list rendered a
  `ChevronRight` drill-in affordance but had no `onRowClick` wired —
  operators could not navigate to session detail from the table.
  Matches the `window.location.hash = 'session/<id>'` pattern RunsPage
  already uses.
- **UI: DecisionsPage bulk-invalidate hit the wrong endpoint.** The
  page fired `POST /v1/decisions/cache/invalidate-all`, which does
  not exist in the router; the real endpoint is
  `POST /v1/decisions/invalidate`. Worse, the raw `fetch` call never
  checked `res.ok`, so a 404 still triggered the "All cache entries
  invalidated" success toast. Both raw-fetch mutations now wrap the
  response in an `assertOk` helper that throws `ApiError` on non-2xx
  (so the global 401 interceptor picks up auth-expired failures from
  this page too), surface the failure through a toast via `onError`,
  and normalize list responses through a local `unwrapList` helper
  (mirrors `api.ts::getList`).
- **UI: ApiDocsPage documented a nonexistent SSE path.** `GET
  /v1/events/stream` does not exist — the canonical runtime SSE
  path is `GET /v1/stream` (see `crates/cairn-app/src/router.rs:369`).
  Doc entry corrected.
- **UI: RunDetailPage plan-review mutations now invalidate the event
  timeline and the approvals list.** Previously `approvePlan` /
  `rejectPlan` / `revisePlan` only invalidated `["run-plan", runId]`
  and `["runs"]`, so the timeline tab and the Approvals tab stayed
  stale until the next 15s poll. Both keys now invalidate on success.
- **UI: TasksPage claim/release mutations now invalidate the per-run
  task list.** The `RowActions` claim/release only invalidated
  `["tasks"]`; RunDetailPage's `["run-tasks"]` list showed stale
  worker/lease state until reload. Both keys now invalidate.
- **UI: missing `onError` handlers across Plugins, Evals, and export
  buttons.** The four `CatalogCard` mutations (install / verify /
  enable / disable) and `EvalsPage::createEval` silently swallowed
  failures. `RunDetailPage::exportRun` and
  `SessionDetailPage::exportSession` called `.then(...)` with no
  `.catch(...)`, so a failed export logged to console and never
  reached the operator. Every site now surfaces a toast on failure.
- **UI: Prometheus parser dual-matches `cairn_`-prefixed histograms.**
  The `http_requests_total` match already accepted the `cairn_`
  prefix but the duration histogram, bucket, and gauge matches did
  not. If the backend ever uniformly prefixes metrics, p50/p95/p99
  silently showed 0. Same dual-match now applied to
  `http_request_duration_ms_{sum,count,bucket}` and to
  `active_runs_total` / `active_tasks_total`.
- **`IntegrationsPage` — derive pause state from the server, not local
  React state.** The "paused" flag used to live in `useState` and only
  flipped on mutation success, so a page reload or a failed pause call
  left the UI out of sync with the dispatcher. Pause/resume rendering is
  now driven by `queueData.dispatcher_running` — a newly emitted field
  on `GET /v1/webhooks/github/queue`.
- **`IntegrationsPage` SSE reuses the shared `useEventStream` hook.**
  The page previously opened its own bespoke `EventSource` against
  `/v1/stream` with no reconnect, back-off, or `Last-Event-ID` replay; a
  one-second network blip killed live updates until the component
  re-rendered. It now subscribes to the singleton stream
  (`useEventStream`) which handles reconnect with jittered back-off and
  gapless replay. `github_progress` was added to the hook's named event
  list so the frame reaches subscribers.
- **`GET /v1/webhooks/github/queue` now emits `max_concurrent` and
  `dispatcher_running`.** The UI was reading both fields via an untyped
  `as Record<string, unknown>` cast with silent defaults, hiding drift
  when the handler shape changed. The Rust handler now populates both
  fields explicitly (derived from the `queue_paused` + `queue_running`
  atomics), the TypeScript return type of `getGitHubQueue()` declares
  them, and the cast is gone.
- **`pauseMut` now invalidates the `github-queue` query and both pause
  and resume mutations show `onError` toasts.** Previously only
  `resumeMut` invalidated on success and `pauseMut` had no error path,
  so a failed pause call silently dropped the user back into a
  green-button UI that looked like everything had worked.

### Security

- **Plugged cross-tenant leak in `TriggersPage`.** The page was sending
  just `scope.project_id` as the `:project` route segment, which the
  backend (`crates/cairn-app/src/trigger_routes.rs`) parses as
  `tenant_id/workspace_id/project_id` and **silently falls back to the
  `DEFAULT_*` constants** when it cannot split on `/`. An operator on
  tenant `acme`, workspace `prod` was therefore reading and writing
  triggers in `default_tenant/default_workspace/acme-project-id` — the
  wrong tenant entirely. All six trigger endpoints (list, create, delete,
  enable, disable, run-templates) now send the full
  `tenant/workspace/project` slash path.
- Removed the dev-admin-token one-click hint from the login page now
  that the UI is reachable from untrusted networks. The placeholder
  hint inside the input still shows `dev-admin-token` when the client
  is pointed at a `localhost`/`127.*` server, but the prominent
  one-click autofill button — which used to broadcast the default credential
  to any visitor — is gone.

### Added

- **`cairn-harness-tools` crate** — adapter bridging the `@agent-sh/harness-*`
  Rust crates into cairn's `ToolHandler` surface. Ten built-in tools are
  now backed by the upstream harness implementations:
  `bash`, `bash_output`, `bash_kill` (from `harness-bash`);
  `read` (from `harness-read`); `grep` (from `harness-grep`);
  `glob` (from `harness-glob`); `write`, `edit`, `multiedit`
  (from `harness-write`); and `webfetch` (from `harness-webfetch`).
  This replaces cairn's in-house implementations of file I/O, shell
  exec, search, and web fetch with battle-tested upstream code while
  keeping cairn's permission / approval / RFC-018 classification pipeline
  intact.
- `ToolError::HarnessError { code, message, meta }` variant — structured
  pass-through for `harness_core::ToolErrorCode` (37 stable codes) so
  orchestrator retry / cache logic can pattern-match on the failure
  reason rather than string-parse the message.

### Fixed

- **Unified frontend "default scope" constants.** Two conflicting
  conventions were in use: `useScope.ts` defined
  `DEFAULT_SCOPE = { tenant_id: 'default_tenant', workspace_id: 'default_workspace', project_id: 'default_project' }`
  (matching the Rust `DEFAULT_*` constants in
  `crates/cairn-app/src/handlers/feed.rs`), but scattered fallbacks
  across the UI used `'default'` or `'default/default/default'`. Any
  fresh install or non-default-scope operator saw empty results or
  wrote to the wrong tenant. All fallbacks now reference the canonical
  `DEFAULT_SCOPE` constant (moved to `ui/src/lib/scope.ts` so
  non-React modules can import it without pulling React):
  `api.ts` (`searchMemory`, `resolveDefaultSetting`,
  notification helpers), `CredentialsPage.tsx`, `ChannelsPage.tsx`.
- **`MemoryPage` search now honours the active scope.** Previously
  `searchMemory` was called with no scope, and the `api.ts` fallback
  wrote `'default'` for all three IDs — searches in any non-default
  scope silently returned empty. `MemoryPage` now reads the current
  scope via `useScope()` and passes it explicitly.
- **Prompt assets (RFC 006) now scope-aware.** `getPromptAssets` and
  `createPromptAsset` in `api.ts` now flow through `withScope(...)`
  like the other project-scoped endpoints, so operators working in a
  non-default workspace no longer see empty lists or accidentally
  create assets in the default project.

### Removed

- **Replaced by harness-tools**: `cairn_tools::FileReadTool`,
  `FileWriteTool`, `GrepSearchTool`, `GlobFindTool`, `WebFetchTool`,
  `BashTool`. Callers should use
  `cairn_harness_tools::HarnessBuiltin::<cairn_harness_tools::HarnessX>::new()`
  instead. `HttpRequestTool` is preserved — its shape (arbitrary
  methods, JSON bodies) differs enough from `webfetch` to keep both.
- Built-in `git_operations` tool and the four `gh_*` CLI-wrapper tools
  (`gh_list_issues`, `gh_get_issue`, `gh_create_comment`, `gh_search_code`).
  Agents now invoke `git` and `gh` CLI directly through the renamed
  `bash` tool — more flexible, zero tool-schema bloat, no wrapper code to
  maintain. The `code-reviewer` agent template's `default_tools` now
  lists `bash` in place of `git_operations`.
- **Unaffected**: `github_api.*` tools (`gh_api_read_file`,
  `gh_api_write_file`, `gh_api_create_branch`, `gh_api_create_pr`,
  `gh_api_merge_pr`, `gh_api_list_contents`) remain — they wrap the
  GitHub App installation-token auth flow used by
  `cairn-integrations::prepare_tool_registry` for per-installation
  token scoping, which the `gh` CLI cannot replicate.

### Changed

- **Soak tests (`test_soak_5min`, `test_soak_30min`, `test_soak_1hr`)
  now assert post-warmup fd steady-state variance rather than a
  baseline→end %-growth bound.** A baseline→end % bound at small
  baselines (~16 fds post-Phase-D + harness-tools) conflates one-time
  startup fd cost with leak growth and fires spuriously. The new bound
  skips the warmup window (60 s for 5min, 150 s for 30min/1hr) and
  asserts `max(fd) - min(fd) <= 5` across steady-state samples —
  exactly what "no leak" semantically means. Motivated by the
  2026-04-22 30min run: 181 successful orchestrations, RSS stable,
  fd oscillated 19–22 post-warmup (delta=3) but the +6 absolute fixed
  startup cost tripped the old 30 % relative bound. Full sample trace
  remains in the panic message for diagnostics.
- **FlowFabric 0.3.2 → 0.3.4 lockfile refresh.** Workspace pins remain
  at the `"0.3"` caret (unchanged), only `Cargo.lock` moves. Picks up
  upstream hotfix for `FlowFabricWorker::connect_with` null
  `completion_backend_handle`; cairn still uses
  `FlowFabricWorker::connect(config)` (URL path) so the fix doesn't
  bite today, but cairn now carries it for the day it does. Build,
  clippy, and test baselines are clean.

- **Renamed `shell_exec` tool → `bash`.** Aligns with harness-tools
  upstream naming (battle-tested name wins). No alias retained; this
  is a clean rename. The built-in file moves from
  `crates/cairn-tools/src/builtins/shell_exec.rs` →
  `.../builtins/bash.rs`, and the Rust type renames from `ShellExecTool`
  → `BashTool`. Agents, prompts, and RFC documentation updated
  throughout. One source of truth for tool names across cairn and
  harness-tools upstream.

- **Phase D PR 2a — run / session / claim FF leaks closed via
  `ControlPlaneBackend`.** Extended the trait with 10 new methods
  (`create_run_execution`, `complete_run_execution`,
  `fail_run_execution`, `cancel_run_execution`, `suspend_run_execution`,
  `resume_run_execution`, `deliver_approval_signal`, `create_flow`,
  `cancel_flow`, `issue_grant_and_claim`) plus the
  `FlowCancelOutcome` / `FailExecutionOutcome` / `ExecutionCreated`
  mirror types and the `ExecutionLeaseContext` / `SuspendRunInput` /
  `CreateRunExecutionInput` / etc. request structs. `FabricRunService`
  (8 lifecycle methods), `FabricSessionService::create` + `::archive`,
  and `services::claim_common::issue_grant_and_claim` now delegate
  through the trait instead of reaching into `ff_core::keys::*` /
  `ff_core::partition::*` directly. Service-side grep audit
  (`git grep -nE '^use ff_core::(keys|partition)::' crates/cairn-fabric/src/services/{run_service,session_service,claim_common}.rs`)
  returns zero hits. `FabricSchedulerService` stays an intentional
  exception — `ClaimGrant` is a wire-contract type shared with
  ff-sdk workers and mirroring it cairn-side adds a conversion hop
  without real hiding; the exception is documented at the top of
  the file. `FabricTaskService` (11 methods, includes
  `declare_dependency` retry loop + `check_dependencies` envelope
  walk) is deferred to PR 2b to keep this PR's scope audit tight.
  Behaviour unchanged at every migrated call site, including the
  approval-waitpoint `signal_match_mode="any"` semantics that
  differ from the pause path's `len > 1 ? "all" : "any"` rule
  (regression pinned by `test_signal_delivery_resumes_waiter`).

### Added

- **Integration test coverage for `ff_renew_lease` (task heartbeat).**
  New `crates/cairn-fabric/tests/integration/test_heartbeat.rs` adds two
  live-harness tests: `test_heartbeat_extends_lease_expiry` (happy path —
  asserts `lease_expires_at_ms` strictly grows and the lease epoch is
  preserved) and `test_heartbeat_with_stale_epoch_is_rejected` (contract —
  asserts `FF_RENEW_LEASE` rejects a bogus epoch with `stale_lease` per
  `FlowFabric/lua/lease.lua:81-82`, and that the rejection does not
  corrupt the live lease). Closes the last zero-coverage FCALL in the
  Phase D PR 2 service-layer scope (run_service + task_service +
  session_service + claim_common). The other four COVERAGE.md §4 targets
  — `ff_suspend_execution`, `ff_resume_execution`, `ff_deliver_signal`,
  `ff_create_flow` / `ff_cancel_flow` — are already covered by
  `test_suspension.rs` and `test_session.rs`; their status in
  `COVERAGE.md` is refreshed in the same PR.

- **FF metrics surfaced on `/metrics`.** `cairn-fabric` now compiles
  `ff-observability` with the `enabled` feature (real OTEL → Prometheus
  exporter) and retains the shared `Arc<ff_observability::Metrics>` on
  `FabricRuntime`. `cairn-app`'s `/metrics` handler appends FF's
  Prometheus text-exposition to its own, so names like
  `ff_scanner_cycle_total`, `ff_scanner_cycle_duration_seconds`,
  `ff_cancel_backlog_depth`, `ff_lease_renewal_total`,
  `ff_claim_from_grant_duration_seconds`, and `ff_http_request_duration_seconds`
  now appear alongside cairn's metrics on the single scrape endpoint.
  See `docs/operations/metrics.md`. Closes the PR #117 follow-up.

### Changed

- **Engine trait decoupling — Phase D PR 1 (control-plane FCALLs +
  worker registry).** The FCALL-shaped control-plane operations
  (budget create / spend / release / status, quota create / admission
  check, waitpoint HMAC rotation) now flow through a new
  `ControlPlaneBackend` trait instead of importing FF key builders +
  partition helpers directly. The Valkey-backed impl lives in
  `engine/valkey_control_plane_impl.rs` and shares one
  `Arc<ValkeyEngine>` with the existing `Engine` trait (one struct,
  two traits). Cairn-native mirror types (`BudgetSpendOutcome`,
  `QuotaAdmission`, `BudgetStatusSnapshot`, `RotationOutcome`,
  `RotationFailure`, `WorkerRegistration`) sit on the trait boundary
  so FF wire enums (`ff_core::contracts::ReportUsageResult`, etc.)
  no longer leak through service signatures. Worker registry
  (register / heartbeat / mark-dead) folds into the existing `Engine`
  trait since the ops are HSET / SADD / PEXPIRE-shaped, consistent
  with Phase C's tag-write methods. `FabricBudgetService`,
  `FabricQuotaService`, `FabricRotationService`, and
  `FabricWorkerService` are now thin shims that delegate to the
  traits — 12 `ff_core::{keys,partition,contracts}` + `ff_sdk::task`
  imports removed from `crates/cairn-fabric/src/services/`. No
  caller-facing API change. Service-level type aliases
  (`BudgetStatus`, `AdmissionResult`, `RotateOutcome`, …) preserved
  so downstream imports keep working. PR 2 extends the same pattern
  to run/task/session lifecycle services (split along the natural
  fault line: FCALL-shaped vs. lifecycle-tangled with shared
  `claim_common.rs` helpers).
- **Engine trait decoupling — Phase C (tag writes).** Cairn services
  no longer call `ferriskey::Client::hset` on FF-owned hashes
  directly. Three new trait methods own the `cairn.*` namespace on
  the flow-core and execution-tags hashes:
  [`Engine::set_flow_tag`], [`Engine::set_flow_tags`] (bulk, single
  round-trip, all-or-nothing validation), and
  [`Engine::set_execution_tag`]. Keys are guarded against collision
  with FF's own hash fields via a `^[a-z][a-z0-9_]*\.` namespace
  rule (rejected as `FabricError::Validation`). `FabricSessionService`
  is the only caller migrated in this phase — its three direct
  `HSET cairn.project|session_id|archived` sites now route through
  the trait. No caller-facing API change; tag writes produce
  identical Valkey state. The `instance_tag_backfill` one-shot
  scanner keeps its direct `HSET` because it operates on raw scan
  keys rather than typed `ExecutionId`s (documented in
  `engine/mod.rs` scope notes as an accepted exception with a
  finite lifetime).
- **FlowFabric bumped to 0.3.2.** Closes #129 (FlowFabric family publish).
  Adopts RFC-012 Stage 1a: `EngineBackend` trait + `EngineError` crate-move
  from `ff-sdk` to `ff-core`. All seven FF crates consumed from crates.io at
  `"0.3"`, along with `ferriskey = "0.3"`.
- **Added `ff-observability` as a direct dependency.** Required because
  `Engine::start_with_completions(config, client, metrics, CompletionStream)`
  takes an `Arc<ff_observability::Metrics>`; `ff-engine` does not re-export
  the type. Replaces the previous `EngineConfig.completion_listener` wiring.
- **Consumes FF's upstream `ScannerFilter { namespace, instance_tag }`**
  (FF#122 / FF PR #127) for cross-instance isolation at the scanner and
  completion-subscriber layers. Per-instance isolation is now enforced by
  both the upstream filter AND cairn's `LeaseHistorySubscriber` client-side
  filter — layered defense, because the upstream filter does not cover the
  per-execution `:lease:history` stream XREAD path that the subscriber
  walks via the partition-global `lease_expiry` ZSET.
- `ff_sdk::task::read_stream` signature takes a `StreamCursor` enum instead
  of `&str`; call sites updated.

### Fixed

- **Cross-instance event leak in `LeaseHistorySubscriber`.** Two
  cairn-app instances sharing a Valkey previously saw each other's
  lease-expiry / lease-reclaim frames in their own `/v1/events` stream
  — `ff:idx:{fp:N}:lease_expiry` ZSETs are partition-global, not
  cairn-scoped, so the subscriber enumerated every cairn instance's
  leased executions on each partition and dispatched foreign frames
  into the local event log. Now every cairn execution carries a
  `cairn.instance_id` tag at create time and the subscriber drops any
  frame whose tag doesn't match `FabricConfig::worker_instance_id`.
  Fixes the `test_rfc020_recovery::clean_crash_recovery_restores_non_terminal_runs`
  flake (task #185) and the production cross-tenant leak. Docs:
  `docs/operations/cross-instance-isolation.md`. (#106)

- **`RoutePolicy.enabled` now plumbed through PG + SQLite projections.**
  The field was accepted on the wire and persisted in the event log, but
  both projection writers silently dropped it, so `GET /v1/providers/policies`
  always returned `enabled = true`. Adds the column to both backends and
  backfills existing rows. (#108)

- **`POST /v1/decisions/evaluate` added to `http_routes.tsv`.** Route
  handler existed and was reachable in production, but the compatibility
  catalogue did not list it, so the drift check could not detect
  regressions. Gap surfaced by #192. (#105)

- **Session projection read-after-write race closed for RFC 020 test
  #11.** `RecoverySummary` could be emitted before the session projection
  saw the preceding terminal transition, causing the compliance test to
  observe a non-terminal run during recovery enumeration. Recovery now
  reads from the authoritative projection head. (#100)

- **Per-harness sandbox base dir isolated.** Multiple `LiveHarness`
  instances in the same test binary previously shared the same sandbox
  base, producing flaky `SandboxBaseRevisionDrift` emissions when tests
  ran in parallel. Each harness now derives its own base path. (#99)

### Added

- **FF#122 `ScannerFilter` data-plane benchmark.** New
  `crates/cairn-fabric/tests/integration/test_scanner_filter_perf.rs`
  measures the wall-time cost of FF's per-candidate `HGET` against a
  live Valkey at candidate counts N ∈ {100, 1 000, 10 000}. Honest
  finding: per-candidate filter-ON p50 ≈ 40–55 µs (Valkey loopback
  RTT-dominated) with p95 up to ~400 µs under shared-instance
  contention; the filter-ON vs filter-OFF delta is noise-dominated
  (±15 % across three full-matrix runs) because `HGET` and `HEXISTS`
  share the same single-round-trip cost profile. Cairn continues to
  run filter-ON in production — the cross-instance isolation the
  filter buys (FF#122) dwarfs any measurable data-plane tax at
  cairn's scale. Reproduce: `CAIRN_TEST_VALKEY_URL=redis://127.0.0.1:6379/ cargo test -p cairn-fabric --test integration --release -- integration::test_scanner_filter_perf --nocapture`.

  | N | filter-OFF median | filter-ON median | per-cand p50 | per-cand p95 | delta vs OFF |
  |-----:|-----------:|-----------:|---------:|---------:|---------:|
  |   100 |  ~6–10 ms |  ~5–10 ms |  ~40–55 µs |   ~90–280 µs | noise (±21 %) |
  | 1 000 |  ~49–94 ms |  ~56–103 ms |  ~48–68 µs |  ~165–400 µs | noise (±16 %) |
  | 10 000 |  ~0.59–1.55 s |  ~0.51–1.51 s |  ~43–55 µs |  ~100–245 µs | noise (±12 %) |

- **9-table SQLite port (option B parity).** Ports `tenants`,
  `workspaces`, `projects`, `workspace_members`, `prompt_assets`,
  `prompt_versions`, `prompt_releases`, `route_decisions`, and
  `provider_calls` to the SQLite backend so team-mode deployments on
  single-node hardware can run without Postgres. Schema-parity check
  (`cargo test -p cairn-store --test schema_parity`) now passes for
  these tables. (#102)

- **`route_policies` ported to SQLite.** Completes option B parity
  for the 10-table block tracked by the schema-parity check. (#104)

- **Prompt schema hardened symmetrically in PG + SQLite.** New PG
  migration `V023` and parallel SQLite DDL tighten `prompt_assets` /
  `prompt_releases` FK and NOT-NULL constraints so both backends reject
  the same invalid inputs. (#103)

- **CI `--tests` allow-list extended to close silent coverage gap.**
  Integration tests in crates not previously in the allow-list (notably
  `cairn-api`) were never executed in CI. Audited the workspace for
  `tests/` directories and added every crate with non-empty integration
  coverage. Contributors adding a new `tests/` directory must extend
  the allow-list in the same PR. (#107)

- **`CAIRN_BACKFILL_INSTANCE_TAG=1`** — one-shot boot-time backfill
  that stamps `cairn.instance_id` onto every pre-existing exec-tag
  hash in Valkey that lacks it but carries `cairn.project`. Needed
  only for operators doing an in-place binary swap with `Running` /
  `WaitingApproval` executions that predate the filter; default off
  on fresh deploys. Idempotent across boots — a second pass is a
  no-op.

#### Durability — RFC 020 Tracks 1–4

- **Track 2 readiness gate** — `GET /health/ready` returns `503` with a
  per-branch progress JSON while recovery is in flight, and flips to
  `200` once every branch reports `complete`. Liveness (`/health`) stays
  `200` throughout so orchestrators can keep the process running across
  long replays. Shape documented in `docs/operations/rfc020-recovery.md`.
  (#73)
- **Track 1 RecoveryService** — startup pass enumerates non-terminal runs,
  applies the RFC 020 recovery matrix, and emits `RecoveryAttempted` /
  `RecoveryCompleted` events before readiness flips to `200`. Closes
  durability invariants 3 (non-terminal runs recovered before readiness)
  and 4 (recovery is idempotent). (#75)
- **Track 3 tool-call idempotency** — deterministic `ToolCallId` derivation,
  `ToolCallResultCache` projection consulted on every dispatch,
  `RetrySafety` three-tier enforcement (`IdempotentSafe` /
  `AuthorResponsible` / `DangerousPause`), and batched tool-event append
  (atomic `ToolInvocationRequested` + `ToolInvocationCompleted`). Closes
  invariants 6 (tool results cached) and 11 (batched append). (#82)
- **Track 4 dual checkpoint per iteration** — `Intent` checkpoint before
  tool dispatch and `Result` checkpoint after, plus `RecoverySummary`
  emitted once per boot and `DecisionCacheWarmup` event at startup. Closes
  invariant 5 (two checkpoints per iteration). (#84)

#### Sandbox recovery tripwires

- **`SandboxLost` emission** on recovery when the sandbox directory is
  missing on disk. Un-ignores RFC 020 compliance test #4. (#83)
- **`SandboxAllowlistRevoked` emission** on recovery when a sandbox's
  origin repo has been dropped from the project access allowlist. Un-ignores
  compliance test #3a. (#86)
- **`SandboxBaseRevisionDrift` emission** on recovery when an overlay
  sandbox's upper-layer base-revision has drifted from the clone cache's
  current `HEAD`. Un-ignores compliance test #3b. (#89)
- **Sandbox reattach test hook** — debug-gated `CAIRN_TEST_SEED_*`
  environment hook exercises the overlay reattach path end-to-end against
  a real sandbox fixture. Un-ignores compliance test #3. (#88)
- **RFC 020 sandbox recovery compliance tests #3 / #3a / #3b / #4**
  landed initially as tripwire `#[ignore]`d tests; each subsequent
  emission PR flips one of them live. (#80)

#### Decision cache durability

- **Decision cache persistence via event log + startup replay** — cached
  decisions survive a restart without re-approval. Closes invariant 9
  (decisions survive) and un-ignores compliance test #7. (#85)

#### Test infrastructure

- **LiveHarness SIGKILL + restart** — `sigkill()`, `restart()`, and
  `sigkill_and_restart()` helpers plus `setup_with_sqlite()` for
  durable-state-across-restart integration tests. Required fixture for
  every RFC 020 Track-3/4 compliance test. (#74)
- **Schema parity check between Postgres and SQLite** — new
  `cargo test -p cairn-store --test schema_parity` enumerates
  `CREATE TABLE` statements from both backends and asserts the table
  sets match. Currently ignored with 10 Postgres-only tables surfaced;
  will become a fail-on-merge gate when the gap closes. (#76)
- **RFC 020 compliance tests #7 (decision cache) and #12 (Postgres-only
  team mode)** as independent integration tests against a live
  cairn-app subprocess. (#77)
- **recovery_e2e migration to LiveHarness — PR 1 of 3**: promotes tests
  #6 (in-flight approval) and #11 (RecoverySummary emitted) from mocked
  unit tests to real SIGKILL-and-restart integration tests; deletes three
  unit-mocked tests whose contracts are now covered by the live suite.
  (#81)
- **recovery_e2e migration — PR 2 of 3**: deletes four additional
  Track-3-duplicated mocked tests whose coverage now lives in the Track 3
  LiveHarness suite. (#87)
- **Provider contract test against real OpenRouter** — live-provider
  chat-completion contract test against an OpenRouter free-tier model,
  gated on `OPENROUTER_API_KEY` so CI without the key skips cleanly. (#90)
- **OpenRouter fixture refresh** — refreshes the recorded fixture against
  the real API and swaps to a stable free-tier model so the offline path
  stays accurate. (#91)
- **Real-LLM soak test ladder against OpenRouter MiniMax** — 5-minute
  (#92), 30-minute (#98), and 1-hour (#101) live-provider soaks against
  the cairn-app subprocess, asserting no lease expiry / event-log drift /
  checkpoint divergence under sustained traffic. All three are gated on
  `OPENROUTER_API_KEY` and skip cleanly in CI without the key.
- **Chaos resilience suite** — SIGSTOP/SIGCONT, failed-append, and
  rapid-restart scenarios exercising cairn-app's durability guarantees
  under adverse conditions. (#95)
- **Reasoning-model response-shape contract test** — asserts that
  providers returning `content: null` with `finish_reason: length`
  (the reasoning-model truncation shape) are surfaced to the orchestrator
  as a typed error rather than an empty-string fallback. (#96)
- **recovery_e2e migration — PR 3 of 3 (post-Track-4 cleanup).**
  Deletes the final batch of Track-4-duplicated mocked tests whose
  coverage now lives in the LiveHarness Track-4 suite. (#97)

#### Operator documentation

- **`docs/operations/rfc020-recovery.md`** — operator-facing guide to
  readiness endpoints, startup sequence, store requirements, durability
  of state across crashes, and runbook entries for recovery situations.
  Summarises RFC 020; RFC is source of truth. (#78)
- **RFC 020 rev 3** — recovery ownership split (FF-owned operational
  state vs. cairn-owned run-level state), 15 gap resolutions, and the
  new durability invariant #12 (storage-transparent durability). (#79)

#### Pre-RFC-020 additions

- **Task dependency declaration now accepts `dependency_kind` and
  `data_passing_ref`.** `POST /v1/tasks/{id}/dependencies` surfaces
  both fields from FF 0.2's flow-edge FCALLs:
  - `dependency_kind` is an enum (today only `success_only`; unknown
    strings return 422 at the JSON extractor).
  - `data_passing_ref` is an opaque caller-supplied string stored on
    the FF edge and forwarded to the downstream task after upstream
    resolution. Cairn never dereferences it; downstream consumers are
    responsible for interpreting the value. Validated at the handler
    (length ≤ 256 bytes, charset `[A-Za-z0-9._:/-]`, empty string
    treated as absent). See `SECURITY.md` for the opaque-string
    contract.

  Existing callers that omit the fields get the previous defaults.
  `GET /v1/tasks/{id}/dependencies` now returns both fields on each
  blocker record.

### Changed

- **Dependency `edge_id` is now deterministic** (UUID-v5 over
  `flow_id || upstream_eid || downstream_eid`) instead of random. The
  replay path (`dependency_already_exists`) can read the staged edge
  directly via `HGETALL fctx.edge(edge_id)` and compare
  `(dependency_kind, data_passing_ref)` against the caller's values:
  identical replay is idempotent 201; a different kind or ref now
  returns **409 `dependency_conflict`** carrying both existing and
  requested values (previously returned 201 and silently kept the
  original). This also makes `BridgeEvent::TaskDependencyAdded`'s
  `edge_id` stable across caller retries, fixing a latent correlation
  gap for consumers of the bridge event stream.
- `TaskDependency` / `TaskDependencyRecord` now carry `dependency_kind`
  and `data_passing_ref` fields. Backward-compatible via
  `#[serde(default)]` so prior event-log records deserialise.
- **Fix**: `GET /v1/tasks/{id}/dependencies` now respects the admin
  token bypass; previously an admin-token call hit an open-coded
  tenant check that always returned 404 regardless of `is_admin`.
  Aligns with `load_task_visible_to_tenant` used by every other
  task-mutation endpoint.

- **FlowFabric bumped to 0.2**: `ff-core`, `ff-sdk`, `ff-engine`, `ff-scheduler`,
  `ff-script`, and `ferriskey` all move from `"0.1"` to `"0.2"`. FF 0.2 is
  behavior-compatible for claim / submit / complete paths — the 32
  cairn-fabric integration tests pass unchanged. The semver break is
  `ScriptError` gaining `#[non_exhaustive]`; cairn never matches
  exhaustively so no source change was required. `ferriskey::Value::BulkString`
  switched its inner type from `Vec<u8>` to `bytes::Bytes`; test fixtures
  in the new rotation service use `.to_vec().into()` accordingly.

- **RFC-011 Phase 2 closure**: per-session runs and tasks co-locate on the
  session's FlowId partition (`{fp:N}` hash tag). Runs are session-bound at
  the `RunService` trait; tasks remain `Option<&SessionId>` at `TaskService`
  to accommodate A2A protocol submissions (which have no session concept).
  The fabric adapter resolves session from the projection on every mutation:
  `TaskRecord.session_id` OR `TaskRecord.parent_run_id → RunRecord.session_id`.
  HTTP handlers no longer redundantly resolve session before calling
  `TaskService` — the adapter is the single source of truth. One exception:
  `create_task_handler` still resolves `parent_task_id → RunRecord.session_id`
  because neither the adapter nor the `TaskCreated` projection writer walks
  that edge, and leaving it out would silently route sub-sub-tasks to the
  solo partition.

### Added

- **`POST /v1/admin/rotate-waitpoint-hmac`** — admin-only endpoint that
  rotates the waitpoint HMAC signing kid across every execution
  partition without a restart. Delegates to FF 0.2's
  `ff_rotate_waitpoint_hmac_secret` FCALL. Request body:
  `{ new_kid, new_secret_hex, grace_ms? }`. Response body:
  `{ rotated, noop, failed[], new_kid }`. Idempotent on the same
  `(new_kid, new_secret_hex)` — replays report `noop` per partition.
  `grace_ms` (default 60_000) is the window in which the previously
  installed kid stays accepted for verification so in-flight
  waitpoints don't fail mid-rotation. Status mapping: 200 on any
  success, 400 on unanimous input-validation failure across all
  partitions (`invalid_kid`, `invalid_secret_hex`, `invalid_grace_ms`,
  `rotation_conflict`), 500 on whole-fleet transport failure, 503 when
  the fabric runtime is absent. See SECURITY.md → "Waitpoint HMAC
  secret rotation" for operator guidance. Closes #114.

- **`debug-endpoints` Cargo feature on `cairn-app`** (OFF by default).
  Enables `GET /v1/admin/debug/partition?kind=<run|task>&id=<id>` for
  RFC-011 co-location diagnostics. **SECURITY: this feature is intended
  for CI/development only.** Production release builds MUST be compiled
  without it. Turning it on adds FF-internal `ExecutionId` and Valkey
  partition-index disclosure (admin-gated) to the HTTP surface —
  information not otherwise reachable except through direct Valkey
  access. See `SECURITY.md` § "Debug endpoints feature" for the full
  threat model.

### Removed (breaking)

- **`in-memory-runtime` cargo feature deleted.** The feature existed as
  an "event-log-only courtesy backing" for `RunService` / `TaskService`
  / `SessionService` when Valkey wasn't available — local tinkering, CI
  escape hatch, some tests. Post the PR #66 FF dependency migration,
  Fabric is authoritative for all runs/tasks/sessions and the in-memory
  impls carried no correctness guarantees; keeping them meant every new
  event shape had to be taught to two runtimes or silently skipped on
  the in-memory side, and ~60 tests asserted behavior that might or
  might not match Fabric without re-testing against live Valkey.

  What goes:
  - `InMemoryServices::{new, with_store, with_fabric}` + `Default` impl
    + the three impl files `{run,task,session}_impl.rs`. The single
    factory `InMemoryServices::with_store_and_core(store, runs, tasks,
    sessions)` is now the only path.
  - 18 gated runtime tests + the orchestrator_e2e test — their
    coverage either already lives in `crates/cairn-store/tests/`
    (projection replay, sqlite adapter) or migrates to Fabric
    integration (see Added below).
  - 4 gated app mutation test files (bootstrap_smoke, e2e_lifecycle,
    full_workspace_suite, provider_lifecycle_e2e) + 19 mutating
    tests inside bootstrap_server.rs.
  - `#[cfg(test)]` modules across 5 tools builtins, quota_impl,
    signal_router_impl, execute_impl, lib.rs, main.rs, telemetry_routes,
    trigger_routes, repo_routes — all of which constructed
    `InMemoryServices::new()` to drive handler tests.
  - 3 feature-gated CI jobs (check feature arm, clippy feature arm,
    integration-tests). CI now runs a single-arm check/clippy/test
    plus the existing fabric-integration job.

  Upgrade path: production builds never enabled the feature, so there
  is no migration. Tests that were gated on `in-memory-runtime` are
  either deleted or ride the new `FakeFabric` read-only fixture under
  `crates/cairn-app/tests/support/`.

### Added

- **`AppState::new_with_runtime` + `AppBootstrap::router_with_injected_runtime`**
  — public constructors that build cairn-app's HTTP surface around a
  caller-provided `InMemoryServices`. Integration-test entry point used
  by the new `FakeFabric` read-only fixture.
- **`AppBootstrap::serve_prebuilt_router`** — serves a pre-built router
  on a listener, bypassing the `Self::router(config)` call inside
  `serve_with_listener` that constructs live Fabric from env.
- **`crates/cairn-app/tests/support/fake_fabric.rs`** — read-only
  stand-in for the production `Fabric{Run,Task,Session}ServiceAdapter`
  trio. Forwards every read method (`get`/`list_by_session`/…) to the
  projection store; returns `RuntimeError::Internal` on every
  mutation. Lets cairn-app handler tests boot `AppState` without a
  live Valkey while keeping the Fabric mutation surface honest.

### Changed

- **Task dependencies migrated to FF flow edges.** `declare_dependency`
  now issues `ff_stage_dependency_edge` + `ff_apply_dependency_to_child`
  on FF's flow partition instead of maintaining a cairn-side
  projection. `check_dependencies` reads live state via
  `ff_evaluate_flow_eligibility` + per-edge HGET on the child's dep
  hash. FF is the single source of truth; the cairn-side
  `TaskDependencyReadModel` trait is deleted.
  - **Breaking behavior (pre-public, no users)**: a failed or
    cancelled prerequisite now auto-skips its dependents
    (`TaskState::Failed` + `FailureClass::DependencyFailed`).
    Previously the dependent would stay `WaitingDependency`
    indefinitely. FF dispatches the skip via the completion listener
    (~RTT × depth) with a reconciler fallback at 15 s intervals.
  - **Breaking**: task dependencies now require both tasks to be in
    the same session. Cross-session and session-less-task declares
    return `Validation` before any FCALL. This matches FF's flow-
    membership contract; cross-flow edges are not representable.
  - **Scope**: edges use FF defaults (`dependency_kind=success_only`,
    `satisfaction_condition=all_required`). `data_passing_ref`
    (auto-copy upstream result to child payload) is not exposed yet
    — follow-up.
  - **Audit**: `RuntimeEvent::TaskDependencyAdded` is still appended
    to the EventLog on each declare, but no cairn projection reads
    it. Callers reconstructing "which deps resolved when" join
    against each prerequisite's `TaskStateChanged(Completed)`.
  - **Engine config**: `FabricRuntime::start` enables
    `CompletionListenerConfig` on the embedded `ff-engine`. Adds a
    third Valkey connection per runtime (main + lease-history tap +
    completion listener); the dedicated RESP3 client SUBSCRIBEs to
    `ff:dag:completions` and dispatches `ff_resolve_dependency`
    FCALLs per terminal transition.

- **RFC-011 Phase 3: `TaskCreated.session_id` / `TaskRecord.session_id`**
  —
  the task → session binding is now persisted directly on the event and
  projection row instead of being derived at resolve-time from
  `parent_run_id → RunRecord.session_id`. This removes a read-model
  round-trip from every task mutation hot path (claim, start, complete,
  heartbeat, release, cancel, fail) and closes the last window where a
  projection-lag parent-run lookup could silently degrade a
  session-scoped task to the solo ExecutionId mint path (wrong Valkey
  hash slot → unexplained Fabric 404).
  - **Schema**: `V021__add_task_session_id.sql` adds a nullable
    `tasks.session_id` column + partial index. Both Postgres and SQLite
    backends use `COALESCE` at insert time to pull the parent run's
    session when the event predates Phase 3 — no data backfill required
    for existing deployments.
  - **Event compat**: `TaskCreated.session_id` is
    `#[serde(default, skip_serializing_if = "Option::is_none")]`, so
    replaying pre-Phase-3 event streams still deserializes. The
    projection's COALESCE fallback handles the `None`-on-event case
    at replay time.
  - **Resolvers**: `resolve_session_for_task_record`,
    `load_task_with_session_for_tenant`, and
    `resolve_task_project_and_session` (fabric adapter) prefer
    `task.session_id` and only walk the parent run when it is `None`.
    The legacy fallback still propagates 500/404 from the Phase-2 fix.

### Added

- **`POST /v1/runs/:id/claim`** — activates a run's FlowFabric execution lease
  so downstream FCALLs (`enter_waiting_approval`, `pause`, signal delivery)
  accept it. NOT idempotent on the Fabric path: re-claiming an already-active
  run fails at FF's grant gate with `execution_not_eligible`. A second claim
  after a suspend/resume cycle dispatches through `ff_claim_resumed_execution`
  and is legitimate.

### Changed

<!--
  Note on "phase-2" nomenclature: "RFC-011 phase-2" refers specifically
  to the *second* mechanical-sweep slice of the FlowFabric co-location
  migration (RFC-011), and is unrelated to `docs/design/phase2-implementation-plan.md`,
  which tracks the separate RFC 015-022 batch. The two "phase 2" labels
  share a number by coincidence only.
-->
- **RFC-011 phase-2 session-scoped execution IDs** — `ExecutionId` for runs
  and tasks now derives from `session_id + run_id/task_id` via UUID-v5
  (`session_run_to_execution_id` / `session_task_to_execution_id`), replacing
  the previous `run_id`/`task_id`-only mints. All runs and tasks within the
  same session now co-locate on the session's `FlowId` Valkey partition,
  satisfying RFC-011's `{fp:N}:<uuid>` hash-tag invariant. **Breaking change,
  flag-day cutover:** any existing execution records in Valkey mint under
  the old scheme and will be unreachable post-upgrade. **Operator action
  required:** drain all in-flight runs and flush the FF Valkey namespace
  before deploying. Trait signatures on `RunService` / `TaskService` now
  thread `session_id` through all mutation methods (`claim`, `complete`,
  `fail`, `cancel`, `pause`, `resume`, `heartbeat`); `TaskService::submit`
  gains a trailing `session_id: Option<&SessionId>` parameter. `BridgeEvent::TaskCreated`
  gains `session_id: SessionId`. HTTP handlers resolve `session_id` from
  the store projection (task → parent run → session) on each call; no new
  round-trips in steady state (the HGETALL already carries the tag).

  **Migration procedure:**

  1. Stop accepting new runs (set the gateway to 503 or drain at the LB).
  2. Wait for in-flight runs to reach a terminal state (`Completed`,
     `Failed`, `Cancelled`). Monitor via `GET /v1/runs?state=running`.
  3. Flush the FF Valkey namespace: `redis-cli -n <db> FLUSHDB` against
     the Fabric Valkey instance. The event log (Postgres/SQLite) is
     authoritative and untouched — only the FF execution-state cache is
     invalidated.
  4. Deploy the new binary.
  5. Resume traffic.

  **Rollback:** revert the binary. The old scheme's execution IDs are
  deterministic from `run_id` alone, so a post-rollback Valkey is still
  reachable from the old code path. Any new runs created *after* the
  upgrade will have execution IDs derived from `session_id + run_id` and
  will be dead-lettered by the old binary — these must be re-issued.

  **Caveat:** historical events in the event log reference pre-upgrade
  `ExecutionId` values. Replay against a fresh Valkey will not find them;
  this is expected. Event-log semantics (durability, causality) are
  unaffected — only ephemeral FF state is scoped to the new mint.

- **RFC-011 phase-1 mechanical sweep** — FF rev bump `a098710` → `1b19dd10`
  (RFC-011 exec/flow hash-slot co-location, phases 1-3). Consumer-side
  adoptions in cairn-fabric:
  - `num_execution_partitions` renamed to `num_flow_partitions`; default
    raised 64 → 256. **Operator action required** if `FF_EXEC_PARTITIONS`
    is set: rename env var to `FF_FLOW_PARTITIONS` before deploying, or
    accept the new default of 256.
  - `ExecutionId` construction migrated to deterministic mint helpers
    (`deterministic_solo` / `for_flow`). The `::new()`, `::from_uuid()`,
    and `Default` constructors are removed upstream.
  - Parallel `parse_spend_result` deleted from `budget_service.rs`;
    replaced with `ff_sdk::task::parse_report_usage_result` (FF #16 closed).
  - Hardcoded `format!("ff:usagededup:…")` sites replaced with
    `ff_core::keys::usage_dedup_key` helper.
  - API-boundary validation added: run/session/project IDs now reject
    control characters at the HTTP handler layer.
  - `FabricError` detail stripping: 500 responses no longer leak Valkey
    key names or Lua error internals.

- **`TaskFrameSink` orchestrator integration** (#30) — orchestrator logs
  tool calls, tool results, LLM responses, and checkpoints through a
  non-consuming sink on the active `CairnTask`, removing the need to thread
  a separate `FrameSink` handle alongside the task. Lease-health gate aborts
  the loop before irreversible side effects when FF reports 3 consecutive
  renewal misses. Checkpoint-snapshot serialize failures degrade to a WARN
  log instead of aborting the step.

### Removed

- **`ActiveTaskRegistry`** (#29) — retired in favour of FlowFabric-owned lease
  state. `CairnTask` now carries the underlying `ClaimedTask` directly; the
  cairn-side registry was a cache of state FF already holds atomically, and
  kept drifting out of sync under lease expiry. Event-emission gate in the
  orchestrator now reads lease health through `TaskFrameSink::is_lease_healthy`
  (the worker-sdk accessor) rather than a cairn-local flag.

---

## [0.1.0] — 2026-04-05

First complete, test-verified milestone. The core control-plane infrastructure
is implemented and RFC-compliant across all ten specified contracts.

### Added

#### Runtime and domain

- **Event-sourced runtime** — 111 `RuntimeEvent` variants covering sessions, runs,
  tasks, approvals, checkpoints, provider calls, credentials, channels, evals,
  signals, knowledge, and commercial events. Every state change is an append;
  no in-place mutation.
- **RFC 002 event-log contract** — append-only log with monotonically ordered
  `EventPosition`, causation-ID idempotency, cursor-based replay, and a
  72-hour SSE replay window. `find_by_causation_id` prevents duplicate command
  application across retries.
- **RFC 005 approval blocking** — `ApprovalRequested` gates run/task progression.
  Pending approvals surface in the operator inbox; `ApprovalResolved` unblocks
  the run atomically and increments the approval record version.
- **RFC 006 prompt release lifecycle** — `draft → active` state machine with
  `PromptReleaseCreated` / `PromptReleaseTransitioned` events; per-asset
  scorecard aggregation across releases.
- **RFC 007 provider health** — `ProviderConnectionRegistered`,
  `ProviderHealthChecked`, `ProviderMarkedDegraded`, `ProviderRecovered` events
  drive the health read model; consecutive failure tracking and per-tenant
  isolation.
- **RFC 008 multi-tenant isolation** — all read-model queries are scoped to
  `ProjectKey` (tenant + workspace + project); cross-tenant data does not
  appear in any listing.
- **RFC 009 provider routing and cost** — `FallbackChainResolver` with
  capability checking; `RouteDecisionRecord` persisted with `fallback_used`
  flag; per-run and per-session cost accumulation in USD micros; derived
  `RunCostUpdated` / `SessionCostUpdated` events emitted into the log.
- **RFC 013 eval rubrics and bundles** — rubric scoring (ExactMatch, Contains,
  Similarity, Plugin); baseline comparison with 5 % regression tolerance;
  `BundleEnvelope` import/export with `PromptLibraryBundle` and
  `CuratedKnowledgePackBundle` discriminators.
- **RFC 014 commercial feature gating** — `ProductTier` (LocalEval,
  TeamSelfHosted, EnterpriseSelfHosted), `Entitlement` categories,
  `DefaultFeatureGate` with fail-closed unknown-feature semantics,
  `EntitlementOverrideSet` events for operator-applied overrides.
- **Durability class contract** — `EntityDurabilityClass::FullHistory` for
  Session/Run/Task (full replay required); `CurrentStatePlusAudit` for all
  other entities. Defined in `cairn-domain` so domain tests can reason about
  durability without depending on the store crate.

#### Storage backends

- **`InMemoryStore`** — full `EventLog` + 51 read-model trait implementations;
  synchronous `apply_projection` within the same lock as `append`; broadcast
  channel for SSE live delivery; `subscribe()` for real-time event fan-out.
- **`PgEventLog`** — durable Postgres append-only event log; events stored in
  `event_log` table with JSON payload; `find_by_causation_id` scans for
  idempotency.
- **`PgAdapter`** — Postgres read models for Session, Run, Task, Approval,
  Checkpoint, Mailbox, ToolInvocation (7 of 51; remainder tracked as gap list
  for follow-on work).
- **`PgSyncProjection`** — synchronous projection applier runs within the same
  Postgres transaction as the append; all new `RuntimeEvent` variants have
  no-op arms.
- **`PgMigrationRunner`** — 17 embedded SQL migrations (V001–V017); applied
  atomically within a transaction on first boot; migration history recorded in
  `_cairn_migrations`.

#### HTTP server (`cairn-app`)

- **16 routes** wired with axum 0.7:
  - `GET /health` — liveness probe (auth-exempt)
  - `GET /v1/stream` — SSE event stream with `Last-Event-ID` replay (auth-exempt)
  - `GET /v1/status` — runtime + store health; Postgres health check when configured
  - `GET /v1/dashboard` — active runs, tasks, pending approvals, system health
  - `GET /v1/runs` + `GET /v1/runs/:id` — run listing and lookup
  - `GET /v1/sessions` — active session listing
  - `GET /v1/approvals/pending` + `POST /v1/approvals/:id/resolve` — approval inbox and resolution
  - `GET /v1/prompts/assets` + `GET /v1/prompts/releases` — prompt asset and release listing
  - `GET /v1/costs` — aggregate cost summary (calls, tokens, USD micros)
  - `GET /v1/providers` — provider binding listing
  - `GET /v1/events` — cursor-based event log replay
  - `POST /v1/events/append` — idempotent event append with causation-ID guard
  - `GET /v1/db/status` — Postgres connectivity and migration state
- **Bearer token auth middleware** (RFC 008) — all `/v1/*` routes except `/v1/stream`
  require `Authorization: Bearer <token>`; `ServiceTokenRegistry` supports
  multiple concurrent tokens.
- **SSE protocol** — `connected` event on open; replay up to 1 000 events after
  `Last-Event-ID`; 15-second keepalive comments; SSE `id:` field carries log
  position for resume.
- **Postgres wiring** — `--db postgres://...` flag creates a `PgPool`, runs
  pending migrations, and enables dual-write: events appended to Postgres
  (durability) and InMemory (read models + SSE broadcast). `GET /v1/events`
  served from Postgres log when configured.
- **CLI flags** — `--mode`, `--port`, `--addr`, `--db`, `--encryption-key-env`.
  Team mode binds `0.0.0.0` and requires `CAIRN_ADMIN_TOKEN`.

#### Knowledge pipeline (`cairn-memory`)

- **Ingest pipeline** — `IngestPipeline<S, C>` with `ParagraphChunker`;
  normalization for PlainText, Markdown, Html; chunk deduplication by
  content hash; no-op `NoOpEmbeddingProvider` for tests.
- **Retrieval scoring** — lexical relevance, freshness decay (`e^(-age/decay_days)`),
  staleness penalty (linear beyond threshold), source credibility, corroboration,
  graph proximity from `InMemoryGraphStore` neighbor overlap.
- **`InMemoryRetrieval`** — `with_graph()` now actually wires the graph store
  and computes proximity; `explain_result()` returns a `ResultExplanation` with
  all scoring dimensions and a human-readable summary.
- **Source quality diagnostics** — `InMemoryDiagnostics` tracks chunk counts,
  retrieval hits, average relevance per source; `index_status()` aggregates
  across all sources for a project.
- **Bundle import/export** — `InMemoryImportService` validates `KnowledgeDocument`
  artifacts, deduplicates by content hash, infers `ImportOutcome` (Create/Skip).
  `InMemoryExportService` bundles documents with origin scope and provenance metadata.

#### Eval system (`cairn-evals`)

- **`EvalRunService`** — in-memory eval run lifecycle: Pending → Running →
  Completed/Failed; `complete_run()` stores `EvalMetrics`;
  `build_scorecard()` aggregates across releases per asset;
  `set_dataset_id()` links a dataset to a run post-creation.
- **`EvalBaselineServiceImpl`** — `set_baseline()`, `compare_to_baseline()`;
  regression detection with ±5 % tolerance band; `fallback_used` flag on locked
  baselines; `select_baseline()` prefers locked over most-recent.
- **`EvalRubricServiceImpl`** — rubric scoring across ExactMatch, Contains,
  Similarity, Plugin dimensions; `score_against_rubric()` requires a dataset
  link; `PluginRubricScorer` trait for custom scoring backends.
- **`BanditServiceImpl`** (GAP-013) — `EpsilonGreedy` and `UCB1` selection
  strategies; `record_reward()` updates `pulls` and `reward_sum`; `with_fixed_rng()`
  for deterministic testing; `list_by_tenant()` for per-tenant experiment views.
- **Provider binding cost stats** — `ProviderBindingCostStatsReadModel`
  implemented with real event-log scan (replaces the stub that returned `None`);
  `list_by_tenant()` groups by `provider_binding_id` via raw event scan.

#### Docs

- **`docs/api-reference.md`** — 769-line operator API reference: all 16 routes,
  request/response shapes, curl examples, auth guide, error codes, server
  configuration, route summary table.
- **`docs/deployment.md`** — Docker Compose, Postgres setup, environment
  variables, team/local mode, TLS, production hardening.

### Architecture

- **12 Rust crates** — `cairn-domain`, `cairn-store`, `cairn-runtime`,
  `cairn-api`, `cairn-app`, `cairn-memory`, `cairn-graph`, `cairn-evals`,
  `cairn-tools`, `cairn-signal`, `cairn-channels`, `cairn-plugin-proto`.
  No circular dependencies.
- **Event log + synchronous projections** — the same `apply_projection` logic
  drives both InMemory and Postgres backends; there is no dual-implementation
  drift. Appends within a transaction guarantee projection consistency.
- **RFC 002–014 compliance** — ten RFC contracts verified by executable
  integration tests. `rfc_compliance_summary.rs` in `cairn-store/tests/`
  contains one focused test per RFC verifying the single most critical MUST
  requirement against the real store backend.

### Test suite

| Category | Count | Failures |
|----------|-------|----------|
| Lib tests (all crates except cairn-app) | 796 | 0 |
| Integration tests (new this session) | ~230 | 0 |
| Previously-broken tests (fixed) | 33 | 0 |
| **Total** | **~1 059** | **0** |

**40+ integration test files** across cairn-store (15 files), cairn-runtime (3),
cairn-memory (8), cairn-evals (3), cairn-api (1), cairn-domain (3).

Notable integration suites:
- `rfc_compliance_summary.rs` — one test per RFC (6 tests)
- `entity_scoped_reads.rs` — RFC 002 entity-scoped event pagination
- `idempotency.rs` — causation-ID idempotency contract (7 tests)
- `event_log_compaction.rs` — 50-event scale proof with cursor pagination
- `approval_blocking.rs` — RFC 005 approval gate lifecycle
- `provider_routing_e2e.rs` — RFC 009 fallback chain with FallbackChainResolver
- `cost_aggregation_accuracy.rs` — per-call micros precision, zero-cost isolation
- `durability_classes.rs` — RFC 002 entity durability contract
- `product_tier_gating.rs` — RFC 014 commercial gating across all three tiers

### Fixed

- **9 pre-existing integration test failures** across cairn-evals
  (`baseline_flow`, `dataset_flow`, `rubric_flow`), cairn-runtime
  (`binding_cost_stats`), and cairn-memory (`ingest_retrieval_pipeline`,
  `entity_extraction`, `explain_result`, `graph_proximity`,
  `provenance_tracking`). Root causes: wrong-crate `EvalSubjectKind` imports,
  extra argument to `create_run`, missing `IngestRequest` fields added in
  later RFCs, stub `ProviderBindingCostStatsReadModel` returning `None`,
  missing `explain_result()` method on `InMemoryRetrieval`, missing graph
  proximity implementation.
- **`DashboardOverview` initializers** in `cairn-api/src/overview.rs` — four
  internal test constructors updated to include the six new RFC 010
  observability fields added during the GAP implementation phase.
- **`PgSyncProjection` non-exhaustive patterns** — `ApprovalPolicyCreated` and
  `PromptRolloutStarted` were missing no-op arms; added to resolve the
  `--features postgres` compile error.

---

*This changelog was generated at the close of the implementation session.*
*Session date: 2026-04-05. Workspace: cairn-rs.*
