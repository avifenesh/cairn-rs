# Status Update — Worker Core

## Task: external_worker_lifecycle (RFC 011)
- **Tests**: 11/11 pass (1 import error fixed at compile time: ExternalWorkerReported is in cairn_domain, not cairn_domain::workers)
- **Files created**: crates/cairn-store/tests/external_worker_lifecycle.rs
- **Files changed**: none
- **Notable**:
  - sentinel_project: worker events are tenant-scoped (no real project) — use ProjectKey(tenant, "_", "_")
  - health.is_alive stays true after ExternalWorkerSuspended (suspension changes status, not health)
  - Heartbeat with outcome=Some clears current_task_id; heartbeat without outcome sets it
  - Double-suspend is idempotent (status remains "suspended")
  - list_by_tenant is unordered (HashMap iteration); pagination tested with limit/offset

## Updated Grand Total: 1,245 passing tests (+11)
