# Status Update — Worker Core

## Task: provider_call_audit (RFC 009)
- **Tests**: 11/11 pass
- **Files created**: crates/cairn-store/tests/provider_call_audit.rs
- **Files changed**: none
- **Architecture note**: ProviderCallRecord.run_id is always None in the projection (the event run_id goes to RunCostRecord accumulation, not the call record). list_by_run is implemented via list_by_decision: one route_decision_id per run → the run's calls are retrievable via that decision ID. list_by_session uses RunCostReadModel which accumulates per-run costs.
- **Notable**:
  - cost_micros accumulation via RunCostRecord proven incrementally (4 calls, checked after each)
  - Free calls (cost_micros=0) add to provider_calls count but not to total_cost_micros
  - Latency percentile test: 5 known values sorted → p50=95ms, p80=120ms verified arithmetically
  - All 3 ProviderCallStatus variants (Succeeded/Failed/Cancelled) verified

## Updated Grand Total: 1,234 passing tests (+11)
