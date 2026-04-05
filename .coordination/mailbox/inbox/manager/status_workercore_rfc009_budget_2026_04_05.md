# Status Update — Worker Core

## Task: provider_budget_tracking (RFC 002/RFC 009)
- **Tests**: 9/9 pass
- **Files created**: crates/cairn-store/tests/provider_budget_tracking.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs
  - Added ProviderBudgetAlertTriggered handler: updates current_spend_micros = current_micros
  - Added ProviderBudgetExceeded handler: sets current_spend_micros = limit + exceeded_by_micros
  - Removed both events from no-op OR groups (were in two places)
- **Notable**: budget_id = "tenant_id:period" composite key matches projection format. ProviderBudgetSet overwrites existing record with current_spend_micros=0 (period reset semantics).
