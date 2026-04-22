# Harness tools adapter (PR #C)

**Status**: shipped (PR C).
**Scope**: integrate the `@agent-sh/harness-*` Rust crates (0.1.0 on crates.io)
as the implementation backing 10 cairn built-in tools.

## Crates integrated

| Cairn tool name | Upstream crate / entrypoint |
|---|---|
| `bash`         | `harness_bash::bash`         |
| `bash_output`  | `harness_bash::bash_output`  |
| `bash_kill`    | `harness_bash::bash_kill`    |
| `read`         | `harness_read::read`         |
| `grep`         | `harness_grep::grep`         |
| `glob`         | `harness_glob::glob`         |
| `write`        | `harness_write::write`       |
| `edit`         | `harness_write::edit`        |
| `multiedit`    | `harness_write::multi_edit`  |
| `webfetch`     | `harness_webfetch::webfetch` |

## Architecture

```
cairn_tools::ToolHandler  ←  cairn_harness_tools::HarnessBuiltin<H>
                                       │
                                       │ H: HarnessTool
                                       ▼
                              HarnessBash / HarnessRead / ...
                                       │
                                       │ calls
                                       ▼
                               harness-{bash,read,...}
```

`HarnessTool` is an associated-type trait (`type Session; type Result;`) so
each adapter impl stays strongly typed against its upstream session config
and result union. `HarnessBuiltin<H>` is a zero-sized `PhantomData`-only
wrapper that adapts any `HarnessTool` onto cairn's `ToolHandler` surface.

## Key decisions

- **Permission hook (`v1`)**: allow-all. Cairn's executor already gates tool
  calls by role / tenant / approval before dispatching — if a harness tool
  is invoked, cairn has already approved it. A future PR (#228) will add
  per-domain allowlists for `webfetch`.
- **Ledger**: process-global `InMemoryLedger` shared by `write` / `edit` /
  `multiedit`. The `read` adapter bridges to the ledger (harness-read
  itself does not touch it) so a cairn session's `read` → `edit` flow
  passes the upstream `NOT_READ_THIS_SESSION` gate.
- **Error mapping**: one new cairn variant `ToolError::HarnessError { code,
  message, meta }` — pass-through for the 37-variant `ToolErrorCode`.
  No string-parsing at the cairn side.
- **Sensitive patterns**: baseline deny list at the adapter layer
  (`.env`, `.pem`, `.key`, `secrets/**`, `.ssh/**`, credentials, private
  keys). Individual cairn profiles can override via session config.
- **Tool name collisions**: harness names win (upstream battle-tested).
  `grep_search` → `grep`, `file_read` → `read`, etc. The observation-safe
  tool-name list in `cairn_orchestrator::decide_impl` accepts both old
  and new names during the transition.

## Not in scope (deferred to #228)

- `harness-lsp` — needs session-scoped `ServerHandle` caching.
- `harness-skill` — needs `SkillRegistry` wiring and per-skill trust
  levels.

## Files

- `crates/cairn-harness-tools/src/adapter.rs` — `HarnessTool` trait + wrapper.
- `crates/cairn-harness-tools/src/hook.rs` — allow-all permission hook.
- `crates/cairn-harness-tools/src/sensitive.rs` — default deny patterns.
- `crates/cairn-harness-tools/src/error.rs` — `harness_core::ToolError` →
  `cairn_tools::ToolError::HarnessError` mapping.
- `crates/cairn-harness-tools/src/tools/{bash,read,grep,glob,write,webfetch}.rs`
  — 10 concrete `HarnessTool` impls.
- `crates/cairn-harness-tools/tests/happy_path.rs` — 8 integration tests.
