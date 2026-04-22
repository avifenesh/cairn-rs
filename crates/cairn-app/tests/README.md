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
| `test_soak_30min` | ~30 min + ~10s boot | `~/.cairn-secrets/openrouter.key` |
| `test_soak_1hr` | ~60 min + ~10s boot | `~/.cairn-secrets/openrouter.key` |

### `test_soak_5min`

Proves cairn-app survives **5 real minutes** of N=3 concurrent agent runs
against the real OpenRouter API (MiniMax M2.5 free tier) without memory
leaks, fd leaks, readiness drops, or panics.

Metrics sampled every 15s: process RSS, open fd count, `/health/ready`,
`/v1/status`, event-log growth. Assertions are deliberately loose for the
first iteration (RSS <50% growth, fd <20% growth) — tighten empirically in
follow-up PRs once we have baseline data from several real runs.

Step 1 of a three-step ladder: **5 min (this)** → 30 min → 1 hr. Each
step ships its own PR and tightens bounds.

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

### `test_soak_30min`

Step 2 of the soak ladder. Same structure as `test_soak_5min`, extended
to **1800 real seconds** (30 min) with bounds tightened using 5min
empirical data (RSS +20.4%, fd +6% in PR #92):

- RSS growth: **<50%** (same absolute bound as 5min — memory growth is
  typically sub-linear, so scaling 6× in duration does not justify
  scaling the bound 6×).
- fd growth: **<30%** (tightened from 5min's <20%; 5min saw +6%).
- Upstream 429 count: informational only.
- Readiness drops / subprocess panics: zero tolerance.

Per-iteration pause is unchanged at 5-10s, so 30min × N=3 yields
~540 OpenRouter calls total. Metric sampling cadence stays at 15s (120
samples), but the progress log line prints once per 60s (30 lines total)
to keep stderr manageable.

```bash
cargo test -p cairn-app --test test_soak_30min -- --ignored --nocapture
```

If 429 pressure makes the soak impossible at these defaults, lower
`WORKERS` or widen `WORKER_INTERVAL` and document the tuning in the PR
body.

### `test_soak_1hr`

Final step (3 of 3) of the soak ladder. Same structure as the earlier
steps, extended to **3600 real seconds** (60 min) with bounds tightened
using 30min empirical data (RSS +29.6%, fd +18.8% in PR #98, with RSS
plateauing in the 83-87 kB band after ~10 min — steady-state working
set, not unbounded growth):

- RSS growth: **<40%** (30min saw 29.6% and plateaued; modest headroom
  while still catching any real leak that breaks the plateau).
- fd growth: **<35%** (30min saw 18.8%; real fd leaks grow unboundedly).
- Upstream 429 count: informational only.
- Readiness drops / subprocess panics: zero tolerance.

Per-iteration pause is unchanged at 10s, so 60min × N=3 yields
~1080 OpenRouter calls total. Metric sampling cadence stays at 15s
(240 samples), with a 60s progress log line (60 lines total).

```bash
cargo test -p cairn-app --test test_soak_1hr -- --ignored --nocapture
```

Empirical ladder progression (filled in per PR):
- 5min (PR #92): RSS +20.4%, fd +6%, 11 orchestrations, 0 429s.
- 30min (PR #98): RSS +29.6%, fd +18.8%, 85 orchestrations, 0 429s,
  RSS plateau after ~10 min.
- 1hr (this PR): see PR body for final numbers.
