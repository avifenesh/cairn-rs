# Handoff

## State
I shipped native tool calling (commit 82fc485c), ToolConfig per-integration (da020c57), repo clone before orchestration (27a0b6b7), AGENTS.md→CLAUDE.md symlink + hardening docs (32b89764, eda8238a), ../cairn→../cairn-sdk (96648723). All pushed to main. Plugin cleanup done: removed 26 duplicate/stale plugins, cache cleared. Explore agents work now.

## Next
1. **Cleanup** — 98 `eprintln!` in production code (68 in main.rs, 11 in lib.rs) should migrate to `tracing`. 63 `#[allow(dead_code)]` in lib.rs need audit. `.env.bak` tracked in git. Stale worktree at `.claude/worktrees/clone-repo-before-orchestration`.
2. **Fix OTLP test** — `orchestrate_run_exports_otlp_spans_with_genai_attributes` fails with 403 (auth/RBAC regression from stabilization work, lib.rs:24752).
3. **5 real TODOs** — `marketplace_routes.rs:127`, `repo_routes.rs:85` (auth context extraction), `lib.rs:1480`, `main.rs:5320` (integration-migration).

## Context
- `git config core.bare` keeps resetting to `true` on WSL — run `git config core.bare false` before git ops.
- `git checkout -- .` needed before pull due to WSL filemode diffs.
- 3,504 tests pass, 1 fails (OTLP — pre-existing).
