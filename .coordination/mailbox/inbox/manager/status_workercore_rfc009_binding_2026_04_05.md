# Status Update — Worker Core

## Task: provider_binding_lifecycle (RFC 009)
- **Tests**: 11/11 pass
- **Files created**: crates/cairn-store/tests/provider_binding_lifecycle.rs
- **Files changed**: none
- **Adaptation**: ProviderBindingRecord has no priority field (priority lives in RouteRule). Effective binding priority is the active flag: list_active() returns only active bindings for a given OperationKind, which is how the router selects candidates. Tests verify this via deactivation-removes-from-routing-candidates pattern.
- **Notable**:
  - Connection is tenant-level (TenantKey), binding is project-level (ProjectKey) — both read models tested
  - list_active() scoped by both project AND OperationKind — tested for all three variants (Generate/Embed/Rerank)
  - Deactivating a binding removes it from list_active (routing candidates) but NOT from list_by_project (audit view)
  - Settings fields (temperature, max_output_tokens, timeout, structured_output_mode, daily_budget) round-trip

## Updated Grand Total: 1,223 passing tests (+11)
