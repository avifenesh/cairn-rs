# Status Update — Worker Core

## Task: skills_catalog
- **Tests**: 17/17 pass
- **Files created**: crates/cairn-domain/tests/skills_catalog.rs
- **Files changed**: none
- **Issues**: none
- **Notable**: disable() does NOT reset status to Proposed — it only clears the enabled flag. Status remains Active after disable(). Tests verify this explicitly. list() filter uses AND semantics for multi-tag queries. New skills start as Proposed+disabled via Skill::new() constructor.

## Updated Grand Total (after skills_catalog)
Previous total: 1,072
New tests added: +17
**New grand total: 1,089 passing tests**
