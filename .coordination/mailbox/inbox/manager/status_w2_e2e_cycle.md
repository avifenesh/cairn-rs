# STATUS: e2e write→project→read cycle

**Task:** 5 end-to-end cycle tests in main.rs  
**Tests passed:** 5/5 (all 60 main.rs tests pass, 0 regressions)  
**Location:** `crates/cairn-app/src/main.rs` tests module

Tests:
- `e2e_append_session_then_list_sessions_shows_it` — POST SessionCreated → GET /v1/sessions shows session_id
- `e2e_append_run_then_list_runs_shows_it` — POST RunCreated → GET /v1/runs shows run_id
- `e2e_append_approval_then_list_pending_shows_it` — POST ApprovalRequested → GET /v1/approvals/pending shows it with null decision
- `e2e_resolve_approval_removes_from_pending` — resolve(approved) → pending list empty, decision=approved
- `e2e_dashboard_active_runs_reflects_appended_run` — starts at 0, after RunCreated → active_runs=1

Run with: `cargo test -p cairn-app --bin cairn-app -- e2e`
