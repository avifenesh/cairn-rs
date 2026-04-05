# Postgres ReadModel Gap Analysis

**Date:** 2026-04-05

## PgAdapter currently implements (7 read models)

| ReadModel | File |
|-----------|------|
| `SessionReadModel` | `pg/adapter.rs:62` |
| `RunReadModel` | `pg/adapter.rs:119` |
| `TaskReadModel` | `pg/adapter.rs:243` |
| `ApprovalReadModel` | `pg/adapter.rs:349` |
| `CheckpointReadModel` | `pg/adapter.rs:394` |
| `MailboxReadModel` | `pg/adapter.rs:451` |
| `ToolInvocationReadModel` | `pg/adapter.rs:522` |

## Missing Postgres implementations (44 read models)

InMemoryStore implements all of these; PgAdapter has none:

### Core operational (high priority — needed for production reads)
- `SignalReadModel`
- `IngestJobReadModel`
- `EvalRunReadModel`
- `PromptAssetReadModel`
- `PromptVersionReadModel`
- `PromptReleaseReadModel`
- `RouteDecisionReadModel`
- `ProviderCallReadModel`
- `ApprovalPolicyReadModel`
- `ExternalWorkerReadModel`

### Organization hierarchy
- `TenantReadModel`
- `WorkspaceReadModel`
- `ProjectReadModel`

### Provider / cost
- `ProviderBindingReadModel`
- `ProviderConnectionReadModel`
- `ProviderHealthReadModel`
- `ProviderHealthScheduleReadModel`
- `ProviderPoolReadModel`
- `ProviderBudgetReadModel`
- `ProviderBindingCostStatsReadModel`
- `RunCostReadModel`
- `SessionCostReadModel`
- `RunCostAlertReadModel`

### Observability / tracing
- `LlmCallTraceReadModel`

### Commercial / config
- `LicenseReadModel`
- `DefaultsReadModel`
- `GuardrailReadModel`
- `ChannelReadModel`
- `NotificationReadModel`
- `CredentialReadModel`
- `CredentialRotationReadModel`
- `QuotaReadModel`

### Operator / identity
- `OperatorProfileReadModel`
- `WorkspaceMembershipReadModel`

### Policy / routing
- `RoutePolicyReadModel`
- `ApprovalPolicyReadModel` _(listed above)_

### Dependency / lease / SLA
- `TaskDependencyReadModel`
- `TaskLeaseExpiredReadModel`
- `RunSlaReadModel`

### Recovery / resilience
- `CheckpointStrategyReadModel`
- `OperatorInterventionReadModel`
- `PauseScheduleReadModel`
- `RecoveryEscalationReadModel`
- `SnapshotReadModel`
- `RetentionPolicyReadModel`

### Sharing
- `ResourceSharingReadModel`
- `SignalSubscriptionReadModel`

### Audit
- `AuditLogReadModel`

## Summary

| | Count |
|-|-------|
| Implemented in PgAdapter | 7 |
| Missing (have in InMemory, not in Pg) | 44 |
| **Total read models in InMemoryStore** | **51** |

## Suggested priority order for worker-2

1. **Tier 1** — needed for any real operator workflow:  
   `PromptAssetReadModel`, `PromptVersionReadModel`, `PromptReleaseReadModel`,  
   `ProviderBindingReadModel`, `RouteDecisionReadModel`, `EvalRunReadModel`,  
   `TenantReadModel`, `WorkspaceReadModel`, `ProjectReadModel`

2. **Tier 2** — cost tracking and observability:  
   `RunCostReadModel`, `SessionCostReadModel`, `ProviderCallReadModel`,  
   `LlmCallTraceReadModel`, `ProviderHealthReadModel`

3. **Tier 3** — remaining operational:  
   `ExternalWorkerReadModel`, `ApprovalPolicyReadModel`, `TaskDependencyReadModel`,  
   `OperatorProfileReadModel`, `WorkspaceMembershipReadModel`

4. **Tier 4** — commercial, config, resilience (can stub initially):  
   All remaining 26
