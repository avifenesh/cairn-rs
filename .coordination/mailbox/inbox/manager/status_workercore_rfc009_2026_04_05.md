# Status Update — Worker Core

## Task: route_decision_persistence (RFC 009)
- **Tests**: 11/11 pass
- **Files created**: crates/cairn-store/tests/route_decision_persistence.rs
- **Files changed**: none
- **Issues**: none
- **Notable**:
  - list_by_project filters by project_id (not full ProjectKey) — verified with cross-project isolation test
  - list_by_project sorts by route_decision_id.to_string() lexicographically (not by decided_at)
  - NoViableRoute decisions store with selected_provider_binding_id=None — verified
  - All 4 RouteDecisionStatus variants tested
  - All 3 OperationKind variants (Generate/Embed/Rerank) tested

## Updated Grand Total: 1,121 passing tests (+11)
