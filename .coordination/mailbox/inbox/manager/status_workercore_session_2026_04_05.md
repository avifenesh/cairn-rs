# Status Update — Worker Core

## Task: session_state_machine
- **Tests**: 12/12 pass
- **Files created**: crates/cairn-store/tests/session_state_machine.rs
- **Files changed**: none
- **Issues**: none
- **Adaptation**: Manager specified "Transition to Paused" but SessionState only has Open/Completed/Failed/Archived (Paused belongs to RunState). Tests use Open→Failed as the non-happy intermediate transition and Open→Archived for explicit archival. This is documented in the file header. For count_by_state: no such method on SessionReadModel trait (adding it would break Postgres and SQLite adapters); derived counts from list_by_project + filter instead, which proves the same correctness property.
