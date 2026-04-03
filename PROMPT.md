# cairn-rs: Implement RFCs

You are implementing the cairn-rs RFC set. Each iteration does ONE concrete unit of work.

## How to orient yourself

1. Read `RALPH-PROGRESS.md` to see which RFC is current and what phase you are in.
2. Read the current RFC file from `docs/design/rfcs/`.
3. Read the relevant crate source code.
4. Identify what the RFC requires vs what already exists.

## Phases per RFC

For each RFC, work through these phases in order:

### Phase 1: Gap analysis
- Read the RFC thoroughly.
- Read the corresponding crate(s).
- Write a gap list to `RALPH-PROGRESS.md` under a new section for the RFC.
- List concrete missing types, traits, modules, endpoints, tests.
- Commit: `git add -A && git commit -m "chore(rfcNNN): gap analysis"`

### Phase 2: Types and traits
- Add missing domain types, enums, structs, traits the RFC defines.
- Wire them into the crate's lib.rs.
- Make it compile: `cargo check --workspace`
- Commit: `git add -A && git commit -m "feat(rfcNNN): add types — <summary>"`

### Phase 3: Implementation
- Implement the core logic the RFC requires (one function/module per iteration).
- Make it compile and pass existing tests.
- Commit: `git add -A && git commit -m "feat(rfcNNN): implement <what>"`

### Phase 4: Tests
- Add tests for the new code.
- Run `cargo test --workspace --quiet` — must pass with 0 failures.
- Commit: `git add -A && git commit -m "test(rfcNNN): add tests for <what>"`

### Phase 5: Mark complete and advance
- Update `RALPH-PROGRESS.md`: mark RFC as done, set next RFC as current.
- Commit: `git add -A && git commit -m "chore(rfcNNN): mark complete"`

## Rules

1. **One commit per iteration.** Do one phase step, commit, exit.
2. **Always compile-check** before committing: `cargo check --workspace`
3. **Never break tests.** Run `cargo test --workspace --quiet` after changes.
4. **Read before writing.** Always read existing code before adding new code.
5. **Minimal changes per iteration.** Small, correct steps.
6. **Update RALPH-PROGRESS.md** at the end of every iteration with what you did and what remains.
7. **Follow the RFC.** Implement what the RFC says, not more.

## RFC to crate mapping

| RFC | Primary crate(s) |
|-----|-----------------|
| 002 | cairn-domain, cairn-runtime, cairn-store |
| 003 | cairn-memory, cairn-store |
| 004 | cairn-graph, cairn-evals |
| 005 | cairn-runtime, cairn-domain |
| 006 | cairn-domain, cairn-evals |
| 007 | cairn-plugin-proto, cairn-tools |
| 008 | cairn-domain, cairn-store |
| 009 | cairn-domain, cairn-runtime |
| 010 | cairn-api |
| 011 | cairn-app, cairn-store |
| 012 | cairn-api, cairn-app |
| 013 | cairn-domain, cairn-store |
| 014 | cairn-domain, cairn-api |

## Completion

When ALL RFCs in RALPH-PROGRESS.md are marked done and `cargo test --workspace --quiet` passes, output the completion signal: the word COMPLETE wrapped in XML promise tags (open tag: less-than promise greater-than, close tag: less-than slash promise greater-than).

Otherwise, describe what you did this iteration and exit.
