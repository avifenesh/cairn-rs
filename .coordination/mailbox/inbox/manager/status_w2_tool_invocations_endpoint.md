# STATUS: GET /v1/runs/:id/tool-invocations

**Task:** Wire tool invocations endpoint  
**Tests passed:** 4 new (83 total, 0 regressions)

## Changes

### Import added
`ToolInvocationReadModel` added to cairn_store projections import

### Handler: `list_run_tool_invocations_handler`
- `GET /v1/runs/:id/tool-invocations?limit=<n>&offset=<n>`
- Reads via `ToolInvocationReadModel::list_by_run(store, &run_id, limit, offset)`
- Returns `Vec<ToolInvocationRecord>` as JSON

### Route wired
`.route("/v1/runs/:id/tool-invocations", get(list_run_tool_invocations_handler))`
after `/v1/runs/:id/events`

### Test module: `tool_invocations_tests`
- `tool_invocations_empty_for_run_with_no_calls` — empty list for fresh run
- `tool_invocations_returns_two_calls_for_run` — both inv_ids in response, all scoped to run
- `tool_invocation_outcome_field_reflects_completion` — null before, "success"/"completed" after record_completed
- `tool_invocations_requires_auth` — 401 without token
