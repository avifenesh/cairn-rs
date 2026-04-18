## Summary

<!-- One or two sentences: what does this change do, and why? -->

## Changes

<!-- Bullet list of the concrete changes. Reference files/crates when useful. -->

-
-

## Test plan

<!-- How did you verify this works? Include commands and their results. -->

- [ ] `cargo check --workspace` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean (both feature arms if runtime-adjacent)
- [ ] `cargo fmt --all --check` clean
- [ ] Unit tests green
- [ ] Integration tests green
- [ ] Manual verification (describe below if applicable)

## Related

<!-- Link issues this closes, RFCs it implements, or prior PRs it builds on. -->

Closes #
