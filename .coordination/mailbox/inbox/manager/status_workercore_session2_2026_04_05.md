# Status Update — Worker Core

## Task: session_state_machine
- **Tests**: 12/12 pass
- **Files created**: crates/cairn-store/tests/session_state_machine.rs
- **Files changed**: none
- **Adaptations**:
  - SessionState has no Paused (that is RunState). Used Open→Failed for the non-happy path.
  - No count_by_state on SessionReadModel trait (adding would break Postgres/SQLite adapters). Derived counts from list_by_project + filter, proving the same property.
