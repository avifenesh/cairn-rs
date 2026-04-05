# STATUS: ingest_job_errors

**Task:** RFC 002 ingest job error handling hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/ingest_job_errors.rs`

**Store fix:** Linter had removed ProviderBudgetAlertTriggered and ProviderBudgetExceeded from the no-op arm of apply_projection (creating a second copy of ProviderBudgetSet without fixing the gap). Added both as no-op arms in the no-op section.

Tests:
- `failed_ingest_job_state_persisted`
- `partial_completion_preserves_document_count`
- `list_by_project_returns_failed_and_succeeded_jobs`
- `error_message_round_trips_without_loss`
- `ingest_jobs_are_project_scoped`
- `entity_scoped_read_returns_ingest_job_events`
