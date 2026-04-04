# Team Coordination: Manager + Workers over tmux

This system runs a manager Claude Code instance that coordinates 3 worker Claude Code instances via file-based messaging and tmux pane injection. The manager decomposes tasks, assigns them to workers, and synthesizes results. Workers implement code and report back.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  tmux session: cairn-team                           │
│                                                     │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐ │
│  │ worker-1 │ │ manager  │ │ worker-2 │ │worker-3│ │
│  │  pane %1  │ │  pane %0  │ │  pane %2  │ │ pane %3│ │
│  └────▲─────┘ └────▲─────┘ └────▲─────┘ └───▲────┘ │
│       │             │             │            │      │
│  ┌────┴─────────────┴─────────────┴────────────┴───┐ │
│  │          team-watch.sh (4 watcher processes)     │ │
│  │  polls .coordination/mailbox/inbox/<agent>/      │ │
│  │  injects messages into target pane via tmux      │ │
│  └──────────────────────────────────────────────────┘ │
│                                                     │
│  ┌──────────────────────────────────────────────────┐ │
│  │  .coordination/mailbox/inbox/                    │ │
│  │    manager/    worker-1/    worker-2/   worker-3/│ │
│  │    (JSON message files)                          │ │
│  └──────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘
```

## Directory Structure

```
.coordination/
├── README.md
├── mailbox/
│   └── inbox/
│       ├── manager/        # Messages TO the manager
│       ├── worker-1/       # Messages TO worker-1
│       ├── worker-2/       # Messages TO worker-2
│       └── worker-3/       # Messages TO worker-3
├── prompts/
│   ├── manager.md          # Manager role brief (injected on first message)
│   ├── worker-1.md         # Worker-1 role brief
│   ├── worker-2.md         # Worker-2 role brief
│   ├── worker-3.md         # Worker-3 role brief
│   └── worker-template.md  # Template for adding workers
├── initialized/            # Flags tracking which agents received their brief
│   ├── manager
│   ├── worker-1
│   ├── worker-2
│   └── worker-3
└── MANAGER_CONTEXT.md      # Optional long-form context for the manager
```

## Scripts

### `scripts/team-send.sh` — Send a message

Atomically drops a JSON message into an agent's inbox. The watcher picks it up and injects it into the agent's tmux pane.

```bash
./scripts/team-send.sh <to> <from> "<message>"
```

**Examples:**
```bash
# Manager assigns a task to worker-1
./scripts/team-send.sh worker-1 manager "Task: Add SignalId to ids.rs. Success criteria: cargo check passes."

# Worker-1 reports completion to manager
./scripts/team-send.sh manager worker-1 "DONE: SignalId added. cargo check -p cairn-domain passed."

# Worker reports a blocker
./scripts/team-send.sh manager worker-2 "BLOCKED: cairn-domain won't compile, missing import."
```

**How it works:**
1. Writes a JSON file to `.coordination/mailbox/inbox/<to>/msg-<nanoseconds>.json`
2. Uses atomic rename (write to `.tmp-*`, then `mv`) to prevent partial reads
3. Message format: `{"from":"...", "to":"...", "ts":"...", "body":"..."}`

### `scripts/team-watch.sh` — Inbox watcher

Polls an agent's inbox directory and injects messages into their tmux pane.

```bash
./scripts/team-watch.sh <agent-name> <tmux-pane-id> <repo-root>
```

**Examples:**
```bash
# Watch worker-1's inbox, inject into pane %1
nohup ./scripts/team-watch.sh worker-1 %1 /path/to/repo > /tmp/w1-watch.log 2>&1 &

# Watch manager's inbox, inject into pane %0
nohup ./scripts/team-watch.sh manager %0 /path/to/repo > /tmp/manager-watch.log 2>&1 &
```

**How it works:**
1. On first message, sends the role brief from `.coordination/prompts/<agent>.md`
2. For each subsequent message, types `Message from <sender>: <body>` into the pane
3. Uses `tmux send-keys -l` for literal text, then `tmux send-keys Enter` to submit
4. On Linux with inotify: instant delivery. On WSL/Windows FS: polls every 1 second
5. Messages are deleted after injection (consumed)

## Setup

### 1. Create the tmux session with panes

```bash
# Create session with 4 panes (adjust layout as needed)
tmux new-session -d -s cairn-team -n team
tmux split-window -h -t cairn-team:team
tmux split-window -v -t cairn-team:team.0
tmux split-window -v -t cairn-team:team.1

# Result: 4 panes — assign them to agents
# Check pane IDs:
tmux list-panes -t cairn-team -F '#{pane_id} #{pane_index}'
# Output like: %0 0, %1 1, %2 2, %3 3
```

### 2. Start Claude Code in each pane

```bash
# In each pane, start Claude Code:
tmux send-keys -t %0 "claude" Enter   # manager
tmux send-keys -t %1 "claude" Enter   # worker-1
tmux send-keys -t %2 "claude" Enter   # worker-2
tmux send-keys -t %3 "claude" Enter   # worker-3
```

### 3. Map panes to agents and find your pane IDs

**Critical:** You must know which pane ID corresponds to which agent. Get this wrong and messages go to the wrong agent.

```bash
# From inside the manager's Claude Code session:
tmux display-message -p '#{pane_id}'
# This tells you YOUR pane ID

# List all panes:
tmux list-panes -a -F '#{pane_id} #{pane_title}'
```

### 4. Start watchers

Start one watcher per agent. Each watcher maps an agent name to a tmux pane ID.

```bash
REPO=/path/to/cairn-rs

# IMPORTANT: Match pane IDs to the correct agents!
nohup ./scripts/team-watch.sh manager  %0 $REPO > /tmp/manager-watch.log 2>&1 &
nohup ./scripts/team-watch.sh worker-1 %1 $REPO > /tmp/w1-watch.log 2>&1 &
nohup ./scripts/team-watch.sh worker-2 %2 $REPO > /tmp/w2-watch.log 2>&1 &
nohup ./scripts/team-watch.sh worker-3 %3 $REPO > /tmp/w3-watch.log 2>&1 &
```

### 5. Initialize agents with their role brief

The watcher automatically sends the role brief from `.coordination/prompts/<agent>.md` on the first message. Alternatively, you can paste the manager prompt directly into the manager's Claude Code pane as the first user message.

### 6. Clear init flags to re-send briefs (optional)

```bash
rm .coordination/initialized/*
```

## Usage Patterns

### Manager sends tasks to workers

```bash
# Assign parallel tasks
./scripts/team-send.sh worker-1 manager "Task: Add ChunkId type. cargo check must pass."
./scripts/team-send.sh worker-2 manager "Task: Add ScoringPolicy. cargo check must pass."
./scripts/team-send.sh worker-3 manager "Task: Add IngestJobId. cargo check must pass."
```

### Workers report back

Workers automatically run:
```bash
./scripts/team-send.sh manager worker-1 "DONE: ChunkId added. cargo check passed."
```

The manager receives this as a message in their pane.

### Manager reads inbox manually (fallback)

If the watcher isn't delivering, the manager can poll manually:

```bash
for f in .coordination/mailbox/inbox/manager/msg-*.json; do
  [ -f "$f" ] && cat "$f" && echo && rm "$f"
done
```

### Check watcher health

```bash
# Are watchers running?
ps aux | grep team-watch | grep -v grep

# Check watcher logs
cat /tmp/w1-watch.log
cat /tmp/manager-watch.log
```

### Restart watchers

```bash
pkill -f "team-watch.sh"
sleep 1
# Re-run the nohup commands from step 4
```

## Troubleshooting

### Message delivered but not submitted (text appears in input, no Enter)

**Cause:** `tmux send-keys Enter` arrives before the TUI finishes processing the text.

**Fix:** The `sleep 0.5` in `type_text()` handles this. If still failing, increase to `sleep 1`.

**Manual workaround:** In the stuck pane, press `Up` then `Enter` to re-submit the queued message.

### Wrong agent receives messages

**Cause:** Pane ID mismatch — watcher is targeting the wrong pane.

**Fix:** Verify pane mapping:
```bash
# From each Claude Code pane, run:
tmux display-message -p '#{pane_id}'
```
Then restart watchers with correct IDs.

### Worker idle after message sent

**Cause:** The watcher consumed the message file but injection failed silently.

**Fix:** Re-send the message:
```bash
./scripts/team-send.sh worker-1 manager "Your task again..."
```

Or inject manually:
```bash
tmux send-keys -t %1 -l "Your task text here"
sleep 0.5
tmux send-keys -t %1 Enter
```

### WSL: inotifywait not working

On WSL with `/mnt/` paths, inotify doesn't work. The watcher automatically falls back to 1-second polling. This is expected and functional, just slightly slower.

### Workers at high context (>80%)

Workers accumulate context over many tasks. When context gets high:
- They may do partial work or stop mid-task
- They may miss parts of long messages

**Fix:** Start a fresh Claude Code session in the pane, or send shorter, more focused tasks.

## Role Brief Templates

### Manager brief (`.coordination/prompts/manager.md`)

The manager prompt should establish:
- Team composition (worker-1, worker-2, worker-3)
- Rules: decompose, assign via `team-send.sh`, never implement code directly
- Task format: "Task: <what>. Success criteria: <how>. Relevant files: <paths>."
- Synthesis: review worker results, assign next tasks or summarize

### Worker brief (`.coordination/prompts/worker-N.md`)

Each worker prompt should establish:
- Identity (worker-N)
- Rules: read inbox, work on assigned task, report back via `team-send.sh`
- Report format: "DONE: <what you did>. Proof: <command and result>."
- Blocker format: "BLOCKED: <reason>"

## Message Format

Messages are JSON files in `.coordination/mailbox/inbox/<agent>/`:

```json
{
  "from": "manager",
  "to": "worker-1",
  "ts": "2026-04-03T20:45:00+03:00",
  "body": "Task: Add SignalId to ids.rs. Success criteria: cargo check passes."
}
```

Files are named `msg-<nanosecond-timestamp>.json` for uniqueness and ordering.

## Scaling

To add a 4th worker:
1. Create a new tmux pane
2. Copy `.coordination/prompts/worker-template.md` to `worker-4.md`, replace `{{WORKER_ID}}`
3. Create `.coordination/mailbox/inbox/worker-4/`
4. Start a watcher: `nohup ./scripts/team-watch.sh worker-4 %<pane-id> $REPO &`
5. Start Claude Code in the pane
6. Update the manager prompt to include worker-4

## Known Limitations

- **No message ordering guarantee** between agents — messages from different workers may arrive in any order
- **No delivery confirmation** — the sender doesn't know if the message was successfully injected
- **Single-line messages only** — newlines in message bodies get flattened by the watcher
- **No encryption** — messages are plaintext JSON on the filesystem
- **WSL filesystem latency** — polling on `/mnt/` paths has ~1 second delay vs instant inotify on native Linux
- **Context accumulation** — workers accumulate context and may need fresh sessions for long-running work
