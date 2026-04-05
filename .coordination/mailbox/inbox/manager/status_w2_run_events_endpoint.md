# STATUS: GET /v1/runs/:id/events

**Task:** Wire run event stream endpoint  
**Tests passed:** 4 new (76 total, 0 regressions)

## Changes

### Handler added (`main.rs`)
`list_run_events_handler` — mirrors `list_session_events_handler` exactly:
- reads via `store.read_by_entity(&EntityRef::Run(run_id), after, limit)`
- returns `Vec<StoredEventSummary>` (position, stored_at, event_type)
- supports `?after=<pos>&limit=<n>` cursor pagination (max 500)

### Route wired
`.route("/v1/runs/:id/events", get(list_run_events_handler))`
inserted after `/v1/runs/:id`

### Pre-existing linter bugs fixed
- `lifecycle::StateTransition` → `events::StateTransition`
- `SessionState::Closed` → `SessionState::Archived`

### Test module: `run_events_tests`
- `run_events_unknown_run_returns_empty`
- `run_events_returns_events_for_the_run` — full write→project→read cycle
- `run_events_cursor_pagination_works` — after=first_pos skips first event
- `run_events_are_run_scoped` — run_sc_a events exclude run_sc_b
