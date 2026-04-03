# Worker Slice Health

Status: generated  
Purpose: keep manager-level crate health visible while workers land slices in parallel.

Interpretation:

- this report runs `cargo test -p <crate>` per owned crate instead of only relying on workspace-wide status
- a red workspace with mostly green slice tests usually means one worker has a concentrated integration issue rather than broad drift

## Current Slice Status

| Worker | Crate | Status | Notes |
|---|---|---|---|
| Worker 2 | `cairn-domain` | `pass` | All crate tests passed in isolation. |
| Worker 3 | `cairn-store` | `pass` | All crate tests passed in isolation. |
| Worker 4 | `cairn-runtime` | `pass` | All crate tests passed in isolation. |
| Worker 5 | `cairn-tools` | `pass` | All crate tests passed in isolation. |
| Worker 5 | `cairn-plugin-proto` | `pass` | All crate tests passed in isolation. |
| Worker 6 | `cairn-memory` | `pass` | All crate tests passed in isolation. |
| Worker 6 | `cairn-graph` | `pass` | All crate tests passed in isolation. |
| Worker 7 | `cairn-agent` | `pass` | All crate tests passed in isolation. |
| Worker 7 | `cairn-evals` | `fail` | Selector resolver has `PromptReleaseState` import/type ambiguity; release service also has borrow-check failures. |
| Worker 8 | `cairn-signal` | `pass` | All crate tests passed in isolation. |
| Worker 8 | `cairn-channels` | `pass` | All crate tests passed in isolation. |
| Worker 8 | `cairn-api` | `pass` | All crate tests passed in isolation. |
| Worker 8 | `cairn-app` | `pass` | All crate tests passed in isolation. |

## Manager Read

- if all rows except one pass, treat the red build as a focused blocker and keep unrelated workers moving
- if several adjacent rows fail together, stop and look for shared-contract drift before more code lands
