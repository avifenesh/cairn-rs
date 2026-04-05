# Status Update — Worker Core

## Task: ingest_job_lifecycle
- **Tests**: 9/9 pass
- **Files created**: crates/cairn-store/tests/ingest_job_lifecycle.rs
- **Files changed**: none
- **Issues**: none
- **Notable**: IngestJobStarted projects to state=Processing (not Pending — the Pending state exists in the domain but is never emitted by the event pipeline). Tests assert this explicitly to catch any future regression.
