# Status Update — Worker Core

## Task: prompt_version_diff (RFC 001)
- **Tests**: 10/10 pass
- **Files created**: crates/cairn-store/tests/prompt_version_diff.rs
- **Files changed**: none
- **Adaptation**: PromptVersionRecord.workspace is always populated as empty string by the projection — cross-workspace isolation is tested via project.workspace_id on the ProjectKey (which IS set correctly by the projection from the event).
- **Notable**:
  - content_hash() helper produces deterministic stub hashes for different content strings
  - version_number tested to be per-asset (not global) — two assets each have independent 1,2 counters
  - list_by_asset sorted by created_at ascending — verified with window comparison
  - 5-version auto-increment tested end-to-end
  - content_hash uniqueness enforced with HashSet size check

## Updated Grand Total: 1,212 passing tests (+10)
