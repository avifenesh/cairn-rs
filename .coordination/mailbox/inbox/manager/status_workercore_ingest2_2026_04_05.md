# Status Update — Worker Core

## Task: ingest_job_lifecycle
- **Tests**: 9/9 pass
- **Files created**: crates/cairn-store/tests/ingest_job_lifecycle.rs
- **Files changed**: none
- **Issues**: none
- **Notable**: IngestJobStarted maps to state=Processing (not Pending). The Pending state exists in the domain enum but is never emitted by the event pipeline. Tests assert Processing explicitly to catch future drift.
