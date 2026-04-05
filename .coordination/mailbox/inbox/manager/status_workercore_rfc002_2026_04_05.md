# Status Update — Worker Core

## Task: event_envelope (RFC 002)
- **Tests**: 31/31 pass
- **Files created**: crates/cairn-domain/tests/event_envelope.rs
- **Files changed**: none
- **Adaptation**: Manager specified EventSource variants User/System/Agent but actual enum has Runtime/System/Operator/Scheduler/ExternalWorker. All five real variants tested and documented in the file header.
- **Notable**:
  - Three independently tagged serde contracts verified: source_type, scope, entity
  - for_runtime_event() auto-derives ownership from payload.project() — tested explicitly
  - All four OwnershipKey From<> conversions tested (Project, Workspace, Tenant, TenantKey)
  - RuntimeEntityRef.kind() tested for all four major entity types

## Updated Grand Total: 1,194 passing tests (+31)
