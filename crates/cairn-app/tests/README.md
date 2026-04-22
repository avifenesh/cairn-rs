# cairn-app integration tests

Each `.rs` file in this directory is its own integration test crate. Shared
helpers live in `support/`.

Most tests use the `LiveHarness` in `support/live_fabric.rs` — it spawns the
real `cairn-app` binary on an ephemeral port against a shared Valkey test
container, and returns a per-test uuid-scoped tenant/workspace/project triple.

## Running

```bash
# All tests (~3300 across the workspace).
cargo test --workspace

# Just cairn-app HTTP tests.
cargo test -p cairn-app

# Single test.
cargo test -p cairn-app --test test_http_lifecycle

# Echo subprocess stderr to test output (useful when debugging
# server-side behavior triggered by a test).
CAIRN_TEST_ECHO_SERVER_STDERR=1 cargo test -p cairn-app -- --nocapture
```

## Reality-check tests (opt-in)

A small family of `#[ignore]`'d tests exercises the real product end-to-end
with real dependencies. They are not part of default CI — run them on demand.

| Test | Duration | Requires |
|---|---|---|
| `test_soak_5min` | ~5 min + ~10s boot | `~/.cairn-secrets/openrouter.key` |

### `test_soak_5min`

Proves cairn-app survives **5 real minutes** of N=3 concurrent agent runs
against the real OpenRouter API (MiniMax M2.5 free tier) without memory
leaks, fd leaks, readiness drops, or panics.

Metrics sampled every 15s: process RSS, open fd count, `/health/ready`,
`/v1/status`, event-log growth. Assertions are deliberately loose for the
first iteration (RSS <50% growth, fd <20% growth) — tighten empirically in
follow-up PRs once we have baseline data from several real runs.

Step 3 of a three-step ladder: **5 min (this)** → 30 min (#174) → 1 hr
(#175). Each step ships its own PR and tightens bounds.

```bash
# Provision the key once (600-mode, outside the repo):
mkdir -p ~/.cairn-secrets
chmod 700 ~/.cairn-secrets
$EDITOR ~/.cairn-secrets/openrouter.key
chmod 600 ~/.cairn-secrets/openrouter.key

# Run the soak.
cargo test -p cairn-app --test test_soak_5min -- --ignored --nocapture
```

Without the key file the test self-skips with an informational message.
Linux-only (reads `/proc/<pid>/status` + `/proc/<pid>/fd/`).

Upstream OpenRouter 429s are counted but do not fail the test — this is a
soak of cairn, not of the upstream provider.
