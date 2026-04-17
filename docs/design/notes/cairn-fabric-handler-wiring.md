# cairn-fabric → cairn-app Handler Wiring Plan

**Status**: PREP. No code changes yet. This maps every Run/Task/Session/Budget/Quota call site in `crates/cairn-app/src/**` to its FabricServices replacement and flags gaps.

**Scope**: handlers + bin_* modules. Does NOT cover `cairn-orchestrator`, `cairn-agent`, or cross-crate runtime callers (tracked separately).

**Source versions**: main @ 1284a54d (cairn-rs), cairn-fabric 18 commits ahead of origin/main, FF B4 idempotency ARGV pending.

---

## 1. Overall findings

| Category | Count |
|---|---|
| Total call sites (Run/Task/Session/Budget/Quota) | **70** |
| Production call sites (exclude `bin_seed`, `lib.rs` tests) | **53** |
| Call sites with drop-in Fabric method | **~15** |
| Call sites blocked by signature mismatch (ProjectKey not available at call site) | **~35** |
| Call sites blocked by missing Fabric method | **~10** |
| API-surface gaps requiring Fabric additions or adapter layer | **5** |

**Critical finding**: FabricServices keep `ProjectKey` as a mandatory argument for every read/write (required by FF partitioning). Current handler call sites use bare IDs (`runs.get(&run_id)`) and resolve project from the returned record. Flipping to FabricServices requires an **adapter trait** (`RunServiceAdapter` etc.) that:
- Looks up `ProjectKey` from the cairn-store projection first, then calls Fabric, OR
- Defers all read-by-id operations to `cairn-store` projections (keep store reads) and only routes **mutations** through Fabric.

The second is simpler and aligns with the event-bridge design (FF = mutation, store = query).

---

## 2. RunService call sites

Handler file → line → Fabric replacement → gap

| File | Line | Handler | Call | Fabric equivalent | Auth? | Gap |
|---|---|---|---|---|---|---|
| `handlers/runs.rs` | 391 | `create_run_handler` | `sessions.get(&session_id)` | `FabricSessionService::get(project, session_id)` | yes | **SIG**: needs project; resolve via `SessionReadModel::get` first |
| `handlers/runs.rs` | 400 | `create_run_handler` | `runs.get(&parent_run_id)` | `FabricRunService::get(project, &run_id)` | yes | **SIG**: project comes from body.project |
| `handlers/runs.rs` | 414-423 | `create_run_handler` | `runs.start(&project, &session_id, run_id, parent_run_id)` | `FabricRunService::start(...)` | yes | ✅ direct match |
| `handlers/runs.rs` | 529 | `get_run_audit_trail_handler` | `runs.get(&run_id)` | `FabricRunService::get` | yes | **SIG**: project from tenant_scope→RunReadModel |
| `handlers/runs.rs` | 672 | `replay_run_handler` | `runs.get(&run_id)` | `FabricRunService::get` | yes | **SIG** |
| `handlers/runs.rs` | 707 | `replay_run_to_checkpoint_handler` | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 774 | `list_run_interventions_handler` | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 810 | `intervene_run_handler` | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 821 | `intervene_run_handler` ForceComplete | `runs.complete(&run_id)` | `FabricRunService::complete(project, run_id)` | yes | **SIG** |
| `handlers/runs.rs` | 882 | `intervene_run_handler` ForcePause | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 935 | `intervene_run_handler` ForceResume | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 1018 | `cancel_run_handler` | `runs.cancel(&run_id)` | `FabricRunService::cancel(project, run_id)` | yes | **SIG** |
| `handlers/runs.rs` | 1040 | `pause_run_handler` | `runs.pause(&RunId, reason)` | `FabricRunService::pause(project, run_id, reason)` | yes | **SIG** |
| `handlers/runs.rs` | 1224 | `list_due_run_resumes_handler` | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 1290 | `spawn_subagent_run_handler` | `runs.get(&parent_run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 1300 | `spawn_subagent_run_handler` | `sessions.get(&child_session_id)` | `FabricSessionService::get` | yes | **SIG** |
| `handlers/runs.rs` | 1351 | `list_child_runs_handler` | `runs.get(&parent_run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 2003 | `recover_run_handler` | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/runs.rs` | 2031 | `get_run_recovery_status_handler` | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/tasks.rs` | 192 | `create_task_handler` | `runs.get(&parent_run_id)` | same | yes | **SIG** |
| `handlers/tasks.rs` | 234 | `create_task_handler` | `runs.get(&parent_run_id)` | same | yes | **SIG** |
| `handlers/tasks.rs` | 550,552 | `complete_task_handler` | `runs.get`/`runs.complete(&parent_run_id)` | same | yes | **SIG** |
| `handlers/tools.rs` | 511,539 | tool invocation handlers | `runs.get(&run_id)` | same | yes | **SIG** |
| `handlers/sqeq.rs` | 228 | `ingress_handler` | `runs.get(&parent_run_id)` | same | no (sqeq bearer) | **SIG** |
| `handlers/sqeq.rs` | 252-260 | `ingress_handler` | `runs.start_with_correlation(project, session_id, run_id, parent_run_id, corr)` | — | no | **MISSING**: FabricRunService has no `start_with_correlation`. Needs new method or adapter. |
| `triggers.rs` | 407 | `evaluate_trigger` | `runs.start_command(StartRun)` | `FabricRunService::start(...)` | n/a (internal) | **MISSING** convenience; can reconstruct from StartRun fields |
| `helpers.rs` | 241 | `build_run_record_view` | `runs.get(run_id)` | `FabricRunService::get(project, run_id)` | n/a | **SIG** |
| `bin_seed.rs` | 64,65 | seed | `runs.complete(run_id)` | `FabricRunService::complete(project, run_id)` | n/a | **SIG** (dev-only) |

**Gap summary — RunService**:
- **G1**: `start_with_correlation(project, session_id, run_id, parent_run_id, correlation_id)` missing on `FabricRunService`. Used by sqeq ingress. Must add.
- **G2**: `start_command(StartRun)` convenience missing. Not a Fabric gap — adapter or call-site refactor can unpack `StartRun` into `start(...)` args.
- **G3**: `list_by_session` returns empty vec (by design, delegate to `RunReadModel::list_by_session`). No handler uses this directly through the trait; listing happens via `state.runtime.store.list_runs_filtered` at `handlers/runs.rs:312`. ✅ no action.
- **G4 systemic**: all bare-id calls need project lookup before the Fabric call. Adapter pattern recommended.

---

## 3. TaskService call sites

| File | Line | Handler | Call | Fabric equivalent | Auth? | Gap |
|---|---|---|---|---|---|---|
| `handlers/tasks.rs` | 206 | `create_task_handler` | `tasks.get(&parent_task_id)` | `FabricTaskService::get(project, task_id)` | yes | **SIG** |
| `handlers/tasks.rs` | 220-229 | `create_task_handler` | `tasks.submit(project, task_id, parent_run_id, parent_task_id, priority)` | `FabricTaskService::submit(project, task_id, parent_run_id, parent_task_id, priority, session_id)` | yes | **PARAM DIFF**: Fabric submit adds `session_id: Option<&SessionId>` arg |
| `handlers/tasks.rs` | 275 | `get_task_handler` | `tasks.get(&TaskId)` | same | yes | **SIG** |
| `handlers/tasks.rs` | 286-304 | `add_task_dependency_handler` | `tasks.declare_dependency(...)` | `FabricTaskService::declare_dependency` returns `Err("use FF flow edges")` | yes | **MISSING/Incompat**: Fabric declines; need FF flow-edge wrapper or keep in store projection |
| `handlers/tasks.rs` | 306-327 | `list_task_dependencies_handler` | `tasks.check_dependencies(&task_id)` | `FabricTaskService::check_dependencies` returns empty | yes | **MISSING**: need projection fallback or FF adapter |
| `handlers/tasks.rs` | 312 | same | `tasks.get(&task_id)` | same | yes | **SIG** |
| `handlers/tasks.rs` | 328-343 | `set_task_priority_handler` | `tasks.get(&task_id)` (read-only, no-op write) | — | yes | NO-OP in runtime; Fabric has no priority mutator. Leave as-is. |
| `handlers/tasks.rs` | 371 | `expire_task_leases_handler` | `tasks.list_expired_leases(now, limit)` | `FabricTaskService::list_expired_leases` returns empty | yes | **MISSING** by design (FF lease scanner handles server-side). Delegate to `TaskReadModel` or mark endpoint deprecated under Fabric |
| `handlers/tasks.rs` | 409-423 | `claim_task_handler` | `tasks.claim(task_id, worker_id, lease_ms)` | `FabricTaskService::claim` | yes | **SIG** (needs project) + verify arg compat |
| `handlers/tasks.rs` | 433-442 | `heartbeat_task_handler` | `tasks.heartbeat(task_id, lease_ms)` | `FabricTaskService::heartbeat` | yes | **SIG** |
| `handlers/tasks.rs` | 459,469 | `release_task_lease_handler` | `tasks.get` / `tasks.release_lease` | `FabricTaskService::get` / `release_lease` | yes | **SIG** |
| `handlers/tasks.rs` | 484,494 | `cancel_task_handler` | `tasks.get` / `tasks.cancel` | `FabricTaskService::get` / `cancel` | yes | **SIG** |
| `handlers/tasks.rs` | 522,532,537 | `complete_task_handler` | `tasks.get` / `start` / `complete` | same | yes | **SIG** |
| `handlers/sqeq.rs` | 456 | `resolve_binding_task_handler` | `tasks.get(&binding.task_id)` | same | no | **SIG** |
| `bin_handlers.rs` | 297 | runner API | `tasks.start(&task_id)` | `FabricTaskService::start(project, task_id)` | yes | **SIG** |
| `bin_handlers.rs` | 488 | runner API | `tasks.cancel(&task_id)` | same | yes | **SIG** |
| `bin_seed.rs` | 118,119,128,145,151 | seed | `tasks.start/complete/cancel` | same | n/a | **SIG** (dev-only) |

**Gap summary — TaskService**:
- **T1**: `declare_dependency` returns Err — `add_task_dependency_handler` will break. **Resolution (manager 2026-04-17)**: use the FF flow-edge path (`ff_stage_dependency_edge` + `ff_apply_dependency_to_child`). Do NOT keep it in the cairn-store event log — keeping it in store would fork task dependencies from flow dependencies and break the Session→Flow DAG planned for Phase 3. Fabric must wrap the FF flow-edge fcalls inside `declare_dependency` / `check_dependencies`.
- **T2**: `check_dependencies` returns empty — `list_task_dependencies_handler` will return empty. Delegate to `TaskDependencyReadModel`.
- **T3**: `list_expired_leases` returns empty — `expire_task_leases_handler` becomes a no-op. Decide: kill the endpoint, or delegate to `TaskReadModel`.
- **T4**: `list_by_state` returns empty — used by scheduler/admin; delegate to projection.
- **T5**: `submit` signature adds `session_id` — acceptable; adapter can pass `None` or look up from parent_run_id.

---

## 4. SessionService call sites

| File | Line | Handler | Call | Fabric equivalent | Auth? | Gap |
|---|---|---|---|---|---|---|
| `handlers/sessions.rs` | 170 | `get_session_handler` | `sessions.get(&SessionId)` | `FabricSessionService::get(project, session_id)` | yes | **SIG** |
| `handlers/sessions.rs` | 138-146 | `list_sessions_handler` | `sessions.list(&project, limit, 0)` | `FabricSessionService::list` returns empty | yes | **MISSING** by design — delegate to `SessionReadModel::list_by_project` |
| `handlers/sessions.rs` | 203,301,342,392,420 | session sub-handlers | `sessions.get(&session_id)` | same | yes | **SIG** |
| `handlers/sessions.rs` | 488-500 | `create_session_handler` | `sessions.create(&project, session_id)` | `FabricSessionService::create(project, session_id)` | yes | ✅ direct match |
| `handlers/runs.rs` | 391,1300 | runs handlers | `sessions.get(&session_id)` | same | yes | **SIG** |
| `handlers/github.rs` | 17 | github | `SessionService` import only | — | — | Import cleanup |
| `handlers/sqeq.rs` | 205 | ingress | `sessions.get(&session_id)` | same | no | **SIG** |
| `bin_export.rs` | 286 | export | `SessionService::create` | `FabricSessionService::create` | n/a | ✅ direct match |

**Gap summary — SessionService**:
- **S1**: `list` returns empty — every `list_sessions_handler` call depends on projection. Adapter must delegate to `SessionReadModel`.
- **S2 systemic**: `get(session_id)` needs project. Resolve via `SessionReadModel::get(session_id)` first (cheap).

---

## 5. BudgetService call sites

| File | Line | Handler | Call | Fabric equivalent | Auth? | Gap |
|---|---|---|---|---|---|---|
| `handlers/providers.rs` | 244 | `list_provider_budgets_handler` | `budgets.list_budgets(&TenantId)` | — | yes | **MISSING**: FabricBudgetService has no tenant-list API |
| `handlers/providers.rs` | 266 | `set_provider_budget_handler` | `budgets.set_budget(tenant_id, period, limit_micros, alert_threshold)` | — | yes | **MISSING**: FabricBudgetService has no `set_budget` |

**Gap summary — BudgetService**: **major mismatch**. `FabricBudgetService` models **run-scoped atomic spend reservations** (`create_run_budget`, `record_spend`, `release_budget`). The current trait models **tenant-wide provider spend ceilings** (`set_budget`, `list_budgets`, `check_budget`). These are two different concerns.

**Recommendation**:
- Do NOT replace `BudgetServiceImpl` with `FabricBudgetService` for provider budgets. Keep the existing impl (event-log backed) for `list_budgets`/`set_budget`.
- Add `FabricBudgetService` as a **new** service hanging off `state.runtime.fabric_budgets` for per-run spend reservation, called by the orchestrator before LLM calls.

---

## 6. QuotaService call sites

| File | Line | Handler | Call | Fabric equivalent | Auth? | Gap |
|---|---|---|---|---|---|---|
| `handlers/admin.rs` | 439 | `set_tenant_quota_handler` | `quotas.set_quota(tenant_id, max_runs, max_sessions, max_tasks)` | — | yes (admin) | **MISSING**: FabricQuotaService has `create_tenant_quota` but different shape |

**Gap summary — QuotaService**: Similar to budgets. `FabricQuotaService::check_admission_for_run` provides admission control; the existing `QuotaService::set_quota` writes a `TenantQuota` record. Different models.

**Recommendation**:
- Keep `QuotaServiceImpl` for `/v1/admin/tenants/:id/quota` (CRUD over the projection).
- Wire `FabricQuotaService::check_admission_for_run` in the orchestrator/run-start path only.

---

## 7. Proposed flip plan (post-FF-stable)

**Phase 1 — adapter layer (non-breaking)**:
1. Add `cairn-app/src/fabric_adapter.rs`: impl `RunService`, `TaskService`, `SessionService` traits that wrap `FabricServices` + projection lookup for bare-id calls.
2. Add `start_with_correlation` to `FabricRunService` (G1). Trivial — `start` with `correlation_id` tag on the FF execution.
3. Add `AppState.fabric: Option<Arc<FabricServices>>` gated by `CAIRN_FABRIC_ENABLED` env var. Default OFF.

**Phase 2 — flip per-service**:
4. Flip RunService first (highest value, most call sites). `state.runtime.runs` becomes the adapter when `fabric_enabled`.
5. Flip TaskService. Delegate `declare_dependency`, `list_expired_leases`, `list_by_state` to projection via adapter (T1–T4).
6. Flip SessionService. Delegate `list` to projection (S1).

**Phase 3 — new surfaces**:
7. Add `state.runtime.fabric_budgets` + `fabric_quotas` as NEW services. Wire into orchestrator spend gates. Do not touch existing provider-budget handlers.

**Phase 4 — scheduler/worker**:
8. Wire `FabricSchedulerService::claim_for_worker` + `FabricWorkerService` into `/v1/workers/*` handlers (not covered in this map — separate PR).

---

## 8. Acceptance checklist

- [x] Every Run/Task/Session/Budget/Quota call site in `crates/cairn-app/src/**` appears in a table.
- [x] Each row names: file:line, handler, call, Fabric equivalent, auth, gap.
- [x] Gaps summarised per service.
- [x] `cargo check --workspace` clean (no code changed — pure research doc).
- [x] Plan ordered by risk (adapter first, new surfaces last).

**Call-site totals by service**:

| Service | Call sites | Drop-in | SIG-blocked | Missing-method |
|---|---|---|---|---|
| RunService | 28 | 1 (`start` in create_run) | 24 | 2 (G1 `start_with_correlation`, G2 `start_command`) |
| TaskService | 24 | 1 (`submit` modulo session_id) | 18 | 5 (T1–T5) |
| SessionService | 11 | 2 (`create`) | 8 | 1 (S1 `list`) |
| BudgetService | 2 | 0 | 0 | 2 (missing entire API surface) |
| QuotaService | 1 | 0 | 0 | 1 (missing `set_quota`) |
| **Total** | **66 prod + 4 seed = 70** | **~15** | **~50** | **11** |

*(Counts include duplicates within the same handler; see tables above for per-row detail.)*
