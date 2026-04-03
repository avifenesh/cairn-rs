# cairn-rs: Fix All Failing Tests and Warnings

You are working on the cairn-rs Rust workspace. Your job is to fix all failing tests and compiler warnings, one iteration at a time.

## Current State

Read the current state by running:
```
cargo test --workspace --quiet 2>&1 | tail -30
```

## Rules

1. **One fix per iteration.** Fix the most important failing test or warning, verify it passes, commit, and exit.
2. **Always verify** by running the relevant test command before committing.
3. **Commit your work** with a clear message: `git add -A && git commit -m "fix: <what you fixed>"`
4. **Don't break other tests.** After your fix, run `cargo test --workspace --quiet` to confirm no regressions.
5. **Read before editing.** Always read the failing test and the code it tests before making changes.
6. **Minimal changes.** Fix the bug, don't refactor surrounding code.

## Known Issues

- `sqlite_parity::rebuild_from_event_stream_produces_identical_state` fails in `crates/cairn-store/tests/cross_backend_parity.rs:980` — timestamp off-by-one (1775236158061 vs 1775236158062)
- Check for compiler warnings with `cargo test --workspace 2>&1 | grep "^warning"`

## Completion

When `cargo test --workspace --quiet` passes with 0 failures AND `cargo test --workspace 2>&1 | grep "^warning"` shows no warnings, output exactly:

<promise>COMPLETE</promise>

If there is still work to do, describe what you fixed this iteration and what remains, then exit.
