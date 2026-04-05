# STATUS: rfc_compliance_summary

**Task:** RFC compliance proof — capstone test  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/rfc_compliance_summary.rs`

Tests (one per RFC, 5-10 lines each):
- `rfc002_causation_id_idempotency` — append event, causation_id resolves to exact position, exactly 1 event after re-delivery
- `rfc005_approval_blocks_run_progression` — run in WaitingApproval, 1 pending approval in operator queue
- `rfc006_prompt_release_draft_to_active` — release starts 'draft', PromptReleaseTransitioned → 'active'
- `rfc008_cross_tenant_isolation` — tenant_a's run not returned for tenant_b; tenant_b query returns None
- `rfc009_route_decision_persisted_with_fallback_flag` — fallback_used=true durable in RouteDecisionReadModel
- `rfc014_unknown_feature_denied_fail_closed` — unknown feature → Denied; known GA feature → Allowed
