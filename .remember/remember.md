# Handoff

## State
cairn-integrations crate complete (59 tests): Integration trait, registry, 5 built-in plugins (GitHub, Linear, Notion, Obsidian, GenericWebhook). Runtime CRUD API live (`POST/GET/DELETE /v1/integrations`, dynamic `POST /v1/webhooks/{id}`). Core tools (file_read, file_write, shell_exec, git, grep, glob) now in base tool registry via `build_full_tool_registry()` in tool_impls.rs. All prompts rewritten. E2E tested: 18 cairn-dogfood issues scanned, agents dispatched, tools called. Two blocking issues found. All uncommitted on main.

## Next
1. **Switch to native tool calling** — Current approach asks LLM to output raw JSON arrays. Switch to OpenAI-style `tool_calls` / Anthropic `tool_use` blocks in `decide_impl.rs`. The `LlmDecidePhase` needs to send tools as function schemas and parse structured tool_calls from the response, not raw text.
2. **Clone repo before orchestration** — `orchestrate_single_issue` must clone the repo into a cairn-workspace sandbox, set `working_dir` to the clone, so file/shell/git tools work locally. Use `SandboxService` or `RepoCloneCache`.
3. **Commit all changes** — massive uncommitted diff: cairn-integrations crate, prompt rewrites, tool tier fix, CRUD API, JWT fix.

## Context
- Two E2E blockers: (1) models output prose+JSON mixed → need native tool calling, (2) agent tries file_read on remote repo path → need local clone.
- elephant-alpha is free but can't produce strict JSON. minimax-m2.5 via Bedrock works but hits the clone issue.
- JWT fix: `exp = now + 540` (not 600) because `iat` backdated 60s.
- Avi: tool tiers are dynamic per agent registration, not static. Each integration defines its tool set.
- Gitea Docker: `docker start cairn-gitea` (port 3001), user cairn-admin/cairn-pass-2026.
- Obsidian mock: `node /tmp/obsidian-api-server.js` (port 27124) with vault at /tmp/obsidian-vault.
