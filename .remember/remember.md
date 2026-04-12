# Handoff

## State
GitHub webhook→orchestrator pipeline built. New `cairn-github` crate (JWT auth, HMAC verification, full REST API client). Webhook handler at POST /v1/webhooks/github with HMAC-SHA256 verification, configurable event→action mappings (GET/PUT /v1/webhooks/github/actions). 6 new API-based GitHub tools in cairn-tools (create_branch, read_file, write_file, create_pr, merge_pr, list_contents). Pipeline: webhook → create session + run → trigger orchestration → LLM decides actions → tools execute → approval gate. All 72+9 tests pass, workspace compiles clean.

## Next
1. **Deploy to dolly** — rebuild Docker image, set GITHUB_APP_ID + GITHUB_PRIVATE_KEY_FILE + GITHUB_WEBHOOK_SECRET env vars, configure event→action mappings via API
2. **Configure event actions** — PUT /v1/webhooks/github/actions with rules for cairn-dogfood issues
3. **Wire GitHub API tools into orchestrate handler** — the tools exist but need to be registered in the orchestrate handler's tool registry (currently only gh CLI tools are registered)
4. **Test end-to-end** — trigger a webhook from cairn-dogfood, verify session/run creation, orchestration, PR creation, approval flow

## Context
- Dolly: ssh -i ~/.ssh/dolly.pem ubuntu@ec2-3-239-71-6.compute-1.amazonaws.com
- GitHub App: cairn-agent-dev (ID 3353056), install 123311552, key at /app/github-app.pem
- Webhook secret: cairn-webhook-secret-2026-k9x7m2p4q8
- No workers — work solo
