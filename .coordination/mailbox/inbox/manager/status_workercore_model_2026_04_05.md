# Status Update — Worker Core

## Task: model_catalog
- **Tests**: 25/25 pass
- **Files created**: crates/cairn-domain/tests/model_catalog.rs (new tests/ dir)
- **Files changed**: none
- **Issues**: none
- **Notable**: All 5 builtin models have context_len >= 100k, so all get HighContextWindow. Llama (openrouter) is text-only input and Free cost type — both verified explicitly. estimate_cost_micros for Sonnet at 1M/1M tokens = 18_000_000 µUSD verified arithmetically.
