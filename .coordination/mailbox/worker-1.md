# Worker 1 Mailbox

Owner: Contracts, Fixtures, Migration Harness

## Current Status

- 2026-04-03 | Manager-owned only | No new Worker 1 feature implementation is planned from this seat. This mailbox now tracks compatibility, cross-worker drift, and quality gates while the other workers continue delivery.
- 2026-04-03 | Manager quality sweep | `cargo test --workspace` and `./scripts/check-compat-inventory.sh` both pass. Current quality attention is on cross-crate seam polish and warning cleanup, not red test failures.
- 2026-04-03 | Worker 1 / Manager | First seed fixture set complete | Minimum Phase 0 HTTP/SSE fixtures are in repo, inventory checker validates them, and the next step is tightening them against direct backend captures where possible.
- 2026-04-03 | Worker 1 / Manager | Mismatch report scaffold complete | Phase 0 fixture maps and generated mismatch report now track seeded coverage explicitly. Next step is replacing or confirming seeded fixtures with direct backend captures.
- 2026-04-03 | Worker 1 / Manager | Cross-worker repo check green | Current in-progress slices from Workers 2/3/4/6/7/8 compile together under `cargo test --workspace`, so we can keep moving without a coordination freeze.
- 2026-04-03 | Worker 1 / Manager | Local upstream is protocol-backed, not handler-backed | The local `../cairn` checkout exposes preserved API/SSE evidence through frontend code and protocol docs, but no concrete legacy server handler surface was found for direct capture. Worker 1 is validating against that upstream contract explicitly instead of stalling.
- 2026-04-03 | Worker 1 / Manager | Rust-side API catalog sync check complete | `cairn-api` now has an executable compatibility test that asserts the preserved route and SSE catalogs match `tests/compat/*` and continue to cover the Phase 0 required HTTP/SSE surfaces.
- 2026-04-03 | Worker 1 / Manager | Phase 0 fixture-shape checks complete | Seeded preserved HTTP/SSE fixtures now have executable minimum-shape assertions in `cairn-api`, so the harness catches accidental fixture drift in the UI-consumed fields instead of only checking file presence.
- 2026-04-03 | Worker 1 / Manager | SSE mapping follow-up identified | The workspace is green, but Worker 8's `sse_publisher` still needs preserved payload-shape alignment checks for task, approval, tool, and agent-progress events before Worker 1 can treat the SSE runtime surface as compatibility-locked.
- 2026-04-03 | Worker 1 / Manager | SSE runtime gap report added | Worker 1 now publishes an explicit Phase 0 SSE publisher gap report so the difference between event-name coverage and preserved payload-shape compatibility stays visible while Worker 8 tightens the publisher surface.
- 2026-04-03 | Worker 1 / Manager | HTTP endpoint gap report added | Worker 1 now publishes an explicit Phase 0 HTTP endpoint gap report so preserved routes with only catalog/fixture coverage stay visible until Worker 8 or later workers add real endpoint boundaries.
- 2026-04-03 | Worker 1 / Manager | Harness refresh is now automatic | `check-compat-inventory.sh` now regenerates the upstream contract report plus the HTTP/SSE gap reports before validating, so Worker 1 does not rely on stale report artifacts.
- 2026-04-03 | Worker 1 / Manager | Workspace integration sweep resolved | `cargo test --workspace` is green again. Worker 1 harness checks remain an independent guardrail, but the current repo risk has moved from red crates to cross-crate seam polish.
- 2026-04-03 | Worker 1 / Manager | SSE payload handoff published | Worker 1 now generates an explicit event-by-event payload handoff report for Worker 8 showing current runtime event sources, preserved fixture wrapper shapes, and the missing builder directions needed to lock SSE compatibility.
- 2026-04-03 | Worker 1 / Manager | Phase 0 owner map published | Worker 1 now generates an explicit owner map for preserved HTTP and SSE compatibility surfaces so route/SSE gaps are routed to Worker 4/6/7/8 intentionally instead of turning into orphaned compatibility TODOs.
- 2026-04-03 | Worker 1 / Manager | Worker slice health report added | Manager health is generated as `.coordination/WORKER_SLICE_HEALTH.md` and currently shows all worker-owned crates green in isolation.
- 2026-04-03 | Worker 1 / Manager | Upstream source pointers published | Worker 1 now generates exact upstream file-and-line pointers for each preserved Phase 0 HTTP route and SSE event, so the protocol-backed migration contract stays auditable even without legacy backend handler captures.
- 2026-04-03 | Worker 1 / Manager | SSE reports now track shaped-payload reality | The generated SSE gap and handoff reports no longer talk about raw runtime-event serialization. They now reflect the actual current state: `sse_payloads` exists, wrapper families are present, and the remaining work is field-level alignment to the preserved fixtures.
- 2026-04-03 | Worker 1 / Manager | Compatibility and workspace sweeps refreshed after latest integration cuts | `./scripts/check-compat-inventory.sh`, `./scripts/generate-worker-slice-health-report.sh`, and `cargo test --workspace` all pass cleanly again after the latest store/domain parity work. No current repo-level compatibility or integration blocker is surfaced by the Worker 1 manager sweep; the remaining quality issue is still the warning-level cleanup in `crates/cairn-tools/src/runtime_service_impl.rs`.

## Blocked By

- none

## Inbox

- 2026-04-03 | Architecture Owner -> Worker 1 | Week 1 focus: `tests/compat`, `tests/fixtures`, preserved route/SSE fixture naming, and initial migration harness shape.
- 2026-04-03 | Worker 8 -> Worker 1 | Preserved route catalog (30 entries) and SSE event catalog (16 entries) are codified as Rust types in `cairn-api::http::preserved_route_catalog()` and `cairn-api::sse::preserved_sse_catalog()`. Classification tags (preserve/transitional) match the compatibility catalog doc. Ready for fixture alignment.
- 2026-04-03 | Worker 8 -> Worker 1 | Week 2: SSE publisher maps all 20 RuntimeEvent variants to preserved SSE names. `build_sse_frame` + `parse_last_event_id` support the `/v1/stream?lastEventId=<id>` replay contract. Ready for SSE fixture assertions.
- 2026-04-03 | Manager -> Worker 1 mailbox | Current focus from this seat is coordination, not feature delivery. Keep the compatibility harness authoritative, refresh generated reports when worker changes land, and use this mailbox to route drift and quality issues to the right owner quickly.

## Outbox

- 2026-04-03 | Worker 1 / Manager -> Worker 2 | Week 2 target: publish the narrow runtime-critical domain cut for session/run/task/approval/checkpoint/mailbox advancement, shared error enums, external-worker reporting, and any required tool-invocation shared records. Land this cut early for Worker 3/4/5.
- 2026-04-03 | Worker 1 -> Worker 2 | Deliver stable base IDs, command/event enums, tenancy keys, and lifecycle types first. Worker 4, 5, 7, and 8 will build on those boundaries immediately.
- 2026-04-03 | Worker 1 -> Worker 3 | Deliver migration layout, event-log interfaces, and sync-projection boundaries early. Worker 4, 6, and 8 are blocked on store shape drifting.
- 2026-04-03 | Worker 1 -> Worker 4 | Stay at runtime service-boundary level until Worker 2/3 shared contracts settle. Avoid locking handler semantics locally.
- 2026-04-03 | Worker 1 -> Worker 5 | Keep tool/plugin work at interface level this week. Do not invent invocation/event shapes outside RFC 007 + Worker 2 shared types.
- 2026-04-03 | Worker 1 -> Worker 6 | Align retrieval/graph persistence assumptions with Worker 3 before implementing storage semantics. Use RFC 003/004/013 as hard contract.
- 2026-04-03 | Worker 1 -> Worker 7 | Keep prompt/eval/agent skeletons aligned to RFC 004 and RFC 006. Do not infer rollout or scorecard semantics from convenience behavior.
- 2026-04-03 | Worker 1 -> Worker 8 | Prioritize preserved API/SSE shell shape and bootstrap boundary only. Do not let operator/backend details outrun runtime/store contracts.

## Ready For Review

- 2026-04-03 | Worker 1 | Review `tests/compat/*`, `tests/fixtures/*`, and `scripts/check-compat-inventory.sh` for phase-0 compatibility harness baseline.
- 2026-04-03 | Worker 1 | Review first seed fixture set under `tests/fixtures/http` and `tests/fixtures/sse`; provenance is documented in `tests/fixtures/HARVESTING_NOTES.md`.
- 2026-04-03 | Worker 1 | Review `tests/fixtures/migration/phase0_mismatch_report.md` plus `tests/compat/phase0_*_fixture_map.tsv` for explicit seeded-coverage status.
- 2026-04-03 | Worker 1 | Review `scripts/generate-phase0-upstream-contract-report.sh` and `tests/fixtures/migration/phase0_upstream_contract_report.md` for the protocol-backed upstream evidence check.
- 2026-04-03 | Worker 1 | Review `scripts/generate-phase0-upstream-source-pointer-report.sh` and `tests/fixtures/migration/phase0_upstream_source_pointers.md` for exact upstream file-and-line evidence behind the preserved Phase 0 contract.
- 2026-04-03 | Worker 1 | Review `crates/cairn-api/tests/compat_catalog_sync.rs` for executable Rust-side route/SSE compatibility assertions against `tests/compat/*`.
- 2026-04-03 | Worker 1 | Review `crates/cairn-api/tests/phase0_fixture_shapes.rs` for executable minimum-shape checks on the seeded preserved HTTP/SSE Phase 0 fixtures.
- 2026-04-03 | Worker 1 | Review `scripts/generate-phase0-http-endpoint-gap-report.sh` and `tests/fixtures/migration/phase0_http_endpoint_gap_report.md` for the current Rust HTTP endpoint coverage assessment.
- 2026-04-03 | Worker 1 | Review `scripts/generate-phase0-sse-publisher-gap-report.sh` and `tests/fixtures/migration/phase0_sse_publisher_gap_report.md` for the current Rust SSE publisher compatibility gap assessment.
- 2026-04-03 | Worker 1 | Review `scripts/generate-phase0-sse-payload-handoff.sh` and `tests/fixtures/migration/phase0_sse_payload_handoff.md` for the concrete event-to-payload builder handoff Worker 8 can use to align SSE frames with preserved frontend shapes.
- 2026-04-03 | Worker 1 | Review `scripts/generate-phase0-owner-map.sh` and `tests/fixtures/migration/phase0_owner_map.md` for the current cross-worker ownership routing of preserved Phase 0 HTTP/SSE compatibility surfaces.
- 2026-04-03 | Worker 1 | Review `scripts/generate-worker-slice-health-report.sh` and `.coordination/WORKER_SLICE_HEALTH.md` for the current manager view of per-worker crate health.
- 2026-04-03 | Worker 1 | Review the refreshed manager sweep outputs after the latest integration pass: `.coordination/WORKER_SLICE_HEALTH.md`, `tests/fixtures/migration/phase0_*`, and the current `cargo test --workspace` run. Worker-level crate health and compatibility inventory are green, and the manager sweep is currently not surfacing a remaining repo-level blocker.
