# Status Update — Worker Core

## Task: prompt_release_governance (RFC 006)
- **Tests**: 15/15 pass
- **Files created**: crates/cairn-store/tests/prompt_release_governance.rs
- **Files changed**: none
- **Adaptation**: Manager specified "Draft→InReview→Active" but domain has no InReview state. Actual lifecycle is Draft→Proposed→Approved→Active→Archived. Tests document this in the file header and use the correct state names. Proposed = the governance review gate.
- **Notable**:
  - PromptRolloutStarted sets rollout_percent AND forces state="active" (even if release was "approved") — tested explicitly
  - Regulated governance blocks Draft→Approved shortcut (requires Draft→Proposed→Approved)
  - Active→Approved rollback path: release no longer appears in active_for_selector
  - Gradual ramp 10%→50%→100% verified via successive PromptRolloutStarted events

## Updated Grand Total: 1,136 passing tests (+15)
