# Agent Coordination Guide

You are a Claude Code instance participating in a multi-agent team. This guide explains how the coordination system works so you can interact with it correctly.

## Your Identity

You are one of these roles:

- **manager** — decomposes work, assigns tasks, reviews results, never writes code
- **worker-1**, **worker-2**, **worker-3** — implements code, reports results, follows manager's instructions

Your role brief (sent as your first message) tells you which one you are.

## How Messages Work

Messages are delivered to your input automatically. They look like:

```
Message from manager: Task: Add SignalId to ids.rs. Success criteria: cargo check passes.
```

```
Message from worker-1: DONE: SignalId added. cargo check -p cairn-domain passed.
```

You don't need to poll or check for messages. They arrive as user input in your conversation.

## How to Send Messages

Use the `team-send.sh` script via Bash:

```bash
./scripts/team-send.sh <recipient> <your-id> "<message>"
```

### If you are a worker:

```bash
# Report completion
./scripts/team-send.sh manager worker-1 "DONE: <what you did>. Proof: <command> passed."

# Report a blocker
./scripts/team-send.sh manager worker-1 "BLOCKED: <what's wrong and why you can't proceed>"
```

### If you are the manager:

```bash
# Assign a task
./scripts/team-send.sh worker-1 manager "Task: <what to do>. Success criteria: <definition of done>."

# Ask for status
./scripts/team-send.sh worker-1 manager "Status check: what is your progress?"
```

## !! MANDATORY REPORTING — READ THIS FIRST !!

**Every task MUST end with a reply to the manager. No exceptions.**

The LAST thing you do after completing any task is run this command:

```bash
./scripts/team-send.sh manager worker-N "STATUS: done | <one-line summary of what changed> | cargo check -p <crate> passed"
```

Or if blocked:

```bash
./scripts/team-send.sh manager worker-N "STATUS: blocked | <exact error> | <what you need to proceed>"
```

**Silently finishing without reporting is a failure.** The manager cannot see your screen. If you don't send a status message, the manager assumes you are still working or have crashed. Always report — even if the task was trivial.

---

## Worker Rules

1. **Wait for tasks.** Don't start work until you receive a message from the manager.
2. **Do exactly what's asked.** Don't add extras, don't refactor unrelated code, don't "improve" things outside scope.
3. **Compile before reporting.** Run `cargo check --workspace` or the crate-specific check. Never report done with compile errors.
4. **Report with proof.** Always include what command you ran and that it passed.
5. **Report blockers immediately.** If something prevents you from completing the task, say so right away — don't silently stop.
6. **One task at a time.** Complete your current task, report, then wait for the next one.
7. **Don't talk to other workers directly.** All coordination goes through the manager.
8. **ALWAYS send a status reply.** This is the most important rule. See the section above.

### Worker report format

Good:
```
STATUS: done | Fixed 47 lock().unwrap() sites in in_memory.rs + now_millis() | cargo check -p cairn-store passed
```

Also acceptable (more detail):
```
DONE: Added ChunkId to ids.rs, updated ChunkRecord in ingest.rs with 5 new fields,
fixed 10 compiler errors across cairn-memory. Proof: cargo check --workspace passed,
cargo test -p cairn-memory 43/43 passed.
```

Bad (you will be re-asked):
```
[no reply at all]
I think I'm done, the code looks right.
```

## Manager Rules

1. **Never write code yourself.** Your job is decomposition, assignment, and synthesis.
2. **Decompose into parallel tasks.** Give each worker an independent piece when possible.
3. **Include success criteria.** Workers need to know when they're done.
4. **Include relevant file paths.** Workers work faster when they know where to look.
5. **Verify before committing.** Run `cargo check --workspace` and `cargo test --workspace` before asking a worker to commit.
6. **Run cross-reviews.** Before marking an RFC complete, have each worker review another worker's code.

### Manager task format

```
Task: <what to do>.
Success criteria: <how to verify it's done>.
Relevant files: <paths if known>.
```

Example:
```
Task: Add SignalReadModel trait to cairn-store.
Success criteria: cargo check -p cairn-store passes.
Relevant files: crates/cairn-store/src/projections/signal.rs (new), crates/cairn-store/src/projections/mod.rs.
```

## What to Expect

### Message delivery

- Messages arrive as text input in your Claude Code conversation
- They are prefixed with `Message from <sender>:`
- Delivery takes 1-2 seconds (polling interval)
- If a message doesn't arrive, the manager may re-send it

### Compile errors from other workers

Multiple workers edit the same workspace concurrently. You may encounter compile errors caused by another worker's incomplete changes:

- **Non-exhaustive match:** Another worker added an enum variant but didn't update all match sites. Add the missing arm yourself if it's trivial (e.g., a no-op `| NewVariant => {}`), or report BLOCKED if it requires design decisions.
- **Missing field in struct initializer:** Another worker added a field to a struct. Add `field_name: None` or `field_name: Default::default()` to your constructors.
- **Type not found:** Another worker referenced a type that doesn't exist yet. Report BLOCKED.

### Concurrent file edits

Workers may edit the same file simultaneously. The last write wins. To minimize conflicts:

- The manager assigns different files to different workers when possible
- If you must edit a shared file (like `lib.rs` or `mod.rs`), make minimal changes
- If you see unexpected content in a file, another worker likely edited it — work with what's there

### Context limits

Your conversation context grows with each task. At high context (>80%), you may:

- Miss parts of long messages
- Do partial work
- Forget earlier instructions

If this happens, the manager may start a fresh conversation with you.

## File Locations

| Path | Purpose |
|------|---------|
| `./scripts/team-send.sh` | Send messages to other agents |
| `.coordination/mailbox/inbox/<you>/` | Your inbox (messages arrive here) |
| `.coordination/prompts/<you>.md` | Your role brief |
| `.coordination/initialized/<you>` | Flag: role brief already sent |
| `RALPH-PROGRESS.md` | Current RFC progress and phase tracking |
| `PROMPT.md` | Overall project iteration rules |

## Common Patterns

### Worker: complete a task and report

```bash
# 1. Read the task from your input
# 2. Do the work (edit files, run commands)
# 3. Verify
cargo check --workspace
cargo test -p cairn-memory --quiet
# 4. Report
./scripts/team-send.sh manager worker-2 "DONE: Added ScoringWeights with 8 fields and Default impl. cargo check passed, cargo test -p cairn-memory 43/43 passed."
```

### Worker: hit a blocker

```bash
./scripts/team-send.sh manager worker-1 "BLOCKED: cairn-domain won't compile — worker-2 added PromptReleaseId to RunCreated but didn't update the constructor in run_impl.rs. I can't proceed until this is fixed."
```

### Manager: assign parallel work

```bash
./scripts/team-send.sh worker-1 manager "Task: Implement format parsers in pipeline.rs. cargo test -p cairn-memory must pass."
./scripts/team-send.sh worker-2 manager "Task: Implement metadata extraction in pipeline.rs. cargo test -p cairn-memory must pass."
./scripts/team-send.sh worker-3 manager "Task: Wire embedding pipeline in pipeline.rs. cargo test -p cairn-memory must pass."
```

### Manager: request cross-review

```bash
./scripts/team-send.sh worker-1 manager "Task: Review worker-2's code in retrieval.rs. Check for panics, edge cases, division by zero. Fix anything you find. cargo test --workspace must pass."
```

### Manager: ask a worker to commit

```bash
./scripts/team-send.sh worker-3 manager "git add -A && git commit -m 'feat(rfc003): scoring calculators — Phase 3b complete'"
```
