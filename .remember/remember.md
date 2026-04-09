# Handoff

## State
Full dogfood pipeline built and tested end-to-end. Bedrock MiniMax M2.5 orchestrates agents that read GitHub issues via gh_list_issues/gh_get_issue tools. 15 crates, 4275+ tests. Brownfield repo at avifenesh/cairn-dogfood (18 issues). Soak test runs unattended with auto-approval.

## Next
1. **Start the soak test** and let it run for days. Watch /tmp/cairn-soak-log.jsonl.
2. When clean for 3+ days, tag v0.1.0.
3. Future: have agent actually write code + open PRs (not just analyze issues).

## Context
- Bedrock: `minimax.minimax-m2.5` in `us-west-2`. Routes by model_id: contains `.` without `/`.
- GH tools: Deferred tier, discovered via tool_search. gh_list_issues/gh_get_issue/gh_search_code (ReadOnly), gh_create_comment (Sensitive).
- Safe-read list in decide_impl.rs overrides LLM's over-cautious requires_approval for read tools.
- Soak test auto-approves approval gates so it runs unattended.
- Start command: `CAIRN_ADMIN_TOKEN=dev-admin-token cargo run -p cairn-app` then `CAIRN_TOKEN=dev-admin-token INTERVAL=300 ./scripts/soak-test.sh`
- Dogfood repo: avifenesh/cairn-dogfood (Express+React+BullMQ monorepo, 18 issues).
