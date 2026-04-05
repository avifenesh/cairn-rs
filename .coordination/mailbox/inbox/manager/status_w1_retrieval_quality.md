# STATUS: retrieval_quality

**Task:** RFC 003 memory retrieval quality hardening  
**Tests passed:** 5/5  
**File:** `crates/cairn-memory/tests/retrieval_quality.rs`

Tests:
- `three_source_types_all_produce_chunks`
- `source_quality_tracks_each_source_independently`
- `retrieval_ranking_by_relevance`
- `retrieval_is_project_scoped`
- `stale_chunks_score_lower_on_freshness`
