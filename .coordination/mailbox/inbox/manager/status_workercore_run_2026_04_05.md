# Status Update — Worker Core

## Task: run_state_machine
- **Tests**: 14/14 pass
- **Files created**: crates/cairn-store/tests/run_state_machine.rs
- **Files changed**: none
- **Issues**: none
- **Coverage beyond spec**: added any_non_terminal, latest_root_run, list_active_by_project, and canceled path — all were reachable given the read model and domain types, and they prove the correctness of the control-plane view the manager depends on.
