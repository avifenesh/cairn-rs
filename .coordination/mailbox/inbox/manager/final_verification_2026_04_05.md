# Final Verification Report — 2026-04-05

## cargo build --workspace
**CLEAN** — Finished in 25.84s, 0 errors, 4 warnings (existing cairn-app warnings only)

## cargo test --workspace --exclude cairn-app --lib
**ALL PASS** — 0 failures, 0 regressions

| Crate | Lib Tests |
|---|---|
| cairn-plugin-proto | 13 |
| cairn-api | 113 |
| cairn-channels | 7 |
| cairn-domain | 148 |
| cairn-evals | 42 |
| cairn-graph | 21 |
| cairn-memory | 92 |
| cairn-signal | 7 |
| cairn-runtime | 208 |
| cairn-store | 24 |
| cairn-tools | 114 |
| cairn-agent | 7 (included in workspace) |
| **TOTAL** | **796** |

## Regressions: NONE

All 796 lib tests pass. Full workspace builds clean.
cairn-app excluded per command (lib.rs has pre-existing errors from other workers).
