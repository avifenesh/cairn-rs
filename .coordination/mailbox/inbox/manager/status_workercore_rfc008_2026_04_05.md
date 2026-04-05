# Status Update — Worker Core

## Task: workspace_rbac_enforcement (RFC 008)
- **Tests**: 16/16 pass
- **Files created**: crates/cairn-store/tests/workspace_rbac_enforcement.rs
- **Files changed**: none
- **Issues**: none
- **Notable**:
  - list_workspace_members queries by workspace_id string only (no tenant scope); get_member uses full WorkspaceKey — both paths tested
  - WorkspaceMemberRemoved uses retain() — exact operator+workspace match; tested both partial remove and full clear
  - WorkspaceRole default is Member (has Default derive) — not tested since it was not asked for
  - Cross-workspace: carol has Admin in ws_alpha, Viewer in ws_beta — verified has_at_least works correctly for per-workspace gate checks

## Updated Grand Total: 1,152 passing tests (+16)
