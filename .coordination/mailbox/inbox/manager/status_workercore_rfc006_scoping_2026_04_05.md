# Status Update — Worker Core

## Task: prompt_asset_scoping (RFC 006)
- **Tests**: 7/7 pass
- **Files created**: crates/cairn-store/tests/prompt_asset_scoping.rs
- **Files changed**: none
- **Issues**: none
- **Notable**: list_by_project filters by full ProjectKey equality (not just project_id string); sorted by created_at ascending. list_by_asset is asset-scoped (not project-scoped) — all versions for a given asset ID are returned regardless of project.
