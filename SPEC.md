# comms — spec (v1 draft)

A command-line tool that lets AI coding agents (Claude Code, Codex, …) working on the
same project talk to each other: quick one-off questions to get a second opinion, and an
ongoing chatroom where they coordinate parallel work and trade ideas.

Status: **design draft, nothing built yet.** Decisions below came out of a design
interview; anything marked *Open* is still undecided.

---

## 1. Goals and non-goals

**Goals**
- Reduce single-model bias: any agent can cheaply ask another model for its take.
- Let two (or more) live agent sessions share context and work in parallel on one
  checkout without stepping on each other.
- Feel like a background task to the agent: it fires off a message, keeps working, and is
  woken when a reply arrives — no polling discipline required.
- Everything inspectable by the human: plain files, a live view, and the ability to step in.

**Non-goals (v1)**
- Multiple machines or multiple checkouts. Same machine, same project directory only.
- Windows (relies on `flock` and inotify/FSEvents).
- Agents other than Claude Code and Codex (but adapters are pluggable, see §8).
- Hard sandboxing of live sessions — `comms` coordinates; it doesn't enforce permissions
  on sessions it didn't launch headlessly.

---

## 2. Concepts

| Concept | Meaning |
|---|---|
| **Project** | The directory tree `comms` operates on. Root = nearest ancestor containing `.comms/`, else the git root. |
| **Room** | A named chatroom inside a project (`main` by default). Separate rooms for separate topics. |
| **Participant** | A named member of a room: `claude`, `codex`, `claude-reviewer`, or the human (`you`). |
| **Message** | One line of chat in a room. Everyone in the room can read it; only some wake people (§5). |
| **Claim** | A soft, expiring lock on a set of paths, so parallel editors don't collide (§6). |
| **Ask** | A one-shot question answered by a freshly started, headless agent (§7). |

Two modes, one core:
- **Mode A: `ask`.** Quick question → a headless agent is spun up → answer printed → exit.
- **Mode B: rooms.** Long-lived agent sessions chatting in a room, each keeping its own context.

An `ask` made from inside a room is also recorded in that room, so both modes share one history.

---

## 3. Storage

Everything lives in `.comms/` at the project root (added to `.gitignore` by `comms init`).

```
.comms/
  config.toml                 # project settings (§10)
  lock                        # global flock for writes
  claims.json                 # active claims, project-wide (not per room)
  rooms/
    main/
      log.jsonl               # append-only message log
      cursors/<participant>   # last message seq this participant has been delivered
      listeners/<participant> # pid file of the running `comms listen`, if any
      budget.json             # turn budget state (§5.4)
  asks/
    <ask-id>.json             # full record of each ask: prompt, answer, agent session id
```

**Message record** (one JSON object per line in `log.jsonl`):

```json
{
  "seq": 42,
  "ts": "2026-10-04T14:05:12Z",
  "from": "codex",
  "kind": "msg",
  "text": "@claude session store reads req.user before your middleware sets it. Race?",
  "mentions": ["claude"],
  "reply_to": 40
}
```

`kind` ∈ `msg` (chat), `system` (joins, claims, budget events), `ask` / `answer` (mode A
records). `seq` is assigned under the lock and is strictly increasing per room.
`mentions` is parsed from `@name` tokens at write time; `@all` expands to every participant.

All writes take an exclusive `flock` on `.comms/lock`. Reads don't lock (append-only log;
readers ignore a trailing partial line).

---

## 4. Identity

Each agent must know its own name. Because agents share a working directory, identity is
per-process, never stored in a project file. Resolution order:

1. `--as <name>` flag
2. `COMMS_AS` environment variable (set automatically by `comms run`, §9)
3. Auto-detect: `CLAUDECODE=1` → `claude`; Codex equivalent → `codex` (*Open: confirm the
   env marker Codex sets in its shell tool*)
4. Otherwise **fail** with an explanation. Never guess.

The human posts as the configured human name (default `you`) via `comms watch` or
`comms say --human`.

Duplicate agents get distinct names: `comms run claude --name claude-reviewer`.

---

## 5. Delivery: the background-task model

The core idea: **any command that waits for something blocks until it arrives, prints it,
and exits.** The agent runs it as a background task; its harness wakes it with the output.

### 5.1 `comms listen`
- Blocks until there is at least one *waking* message for me (§5.2) after my cursor, then
  prints it and exits 0. Uses fsnotify on `log.jsonl`, no busy polling.
- Also prints a digest of non-waking messages since my cursor ("3 earlier messages: …"),
  so I see the whole conversation without being woken for each line.
- Advances my cursor only after printing (at-least-once delivery).
- One listener per (room, participant): holds `listeners/<name>` pid file; a second
  `listen` exits with an error. This lets hooks check "is a listener running?".
- `--timeout <dur>` → exit code 2 if nothing arrived. Budget exhausted → exit code 3.

**Convention (written into each agent's instructions by `comms install`/`comms run`):**
keep exactly one `comms listen` running in the background; when it fires, handle the
message, reply if needed, and immediately start a new `comms listen`.

### 5.2 What wakes whom
A message wakes participant P if any of:
- it @mentions P, or `@all`
- it's from the human (human messages wake **everyone** in the room)
- it's a claim-conflict notice about P's claim (§6)

Everything else is history: visible in the digest and in `comms read`, but doesn't wake.

### 5.3 Per-agent delivery paths

| | Claude Code | Codex |
|---|---|---|
| While working | Background `comms listen` completes → harness notifies | `PostToolUse` hook runs `comms hook inbox`; injects waking messages via `additionalContext` |
| When it ends its turn | Background `listen` keeps running; completion re-invokes Claude even when idle | `Stop` hook runs `comms listen --timeout <idle_wait>`; if a message arrives, returns `decision: block` with the message → Codex continues |
| Backstop | `Stop` hook: if no listener is running and waking messages are pending, block and deliver them | — (Stop hook is already the main path) |
| Fully idle beyond `idle_wait` | Still woken (background task) | **Not woken**: needs the human. *Open: `comms run codex --tmux` to inject a prompt via tmux, v2* |

Why the asymmetry: Codex has no equivalent of Claude Code's "background command finished →
start a new turn" ([openai/codex#29922](https://github.com/openai/codex/issues/29922) proposes one).
Its `Stop` hook can force continuation, so we hold Codex's turn open for a bounded idle window
instead. Trade-off: during `idle_wait` the Codex TUI looks busy; Esc interrupts it.

All paths share the cursor, so each message is delivered to a participant exactly once
regardless of which path picks it up.

### 5.4 Turn budget
Two agents that can wake each other can loop forever ("good point!" / "thanks!").
- Each room counts agent-authored messages since the last human message.
- Default budget **20**. When exhausted: agents' `comms say` is refused with
  "budget exhausted, waiting for the human"; a `system` message is posted; `listen`
  only wakes on human messages.
- Any human message resets the counter. `comms budget <n>` changes the limit for a room.

---

## 6. Claims

Structured, soft, expiring locks over paths so parallel editors coordinate.

```
comms claim "src/auth/**" --note "refactoring middleware to async" [--ttl 30m]
comms release "src/auth/**"      # or: comms release --all
comms claims                     # who holds what, notes, expiry
comms check src/auth/session.ts  # exit 1 + holder info if claimed by someone else
```

- Globs are relative to project root. Claims are **project-wide**, not per room.
- Default TTL 30 min. Renewed whenever the holder edits a claimed file (via hook) or
  re-runs `comms claim`. Expired claims are dropped lazily.
- Claiming/releasing posts a `system` message to the claimer's current room.
- Overlapping claims by different holders are refused, showing the existing holder.

**Enforcement on conflict: block, then suggest asking.** `comms install` adds a
`PreToolUse` hook to each agent:
- Claude Code: matcher `Edit|Write|MultiEdit|NotebookEdit`; path from `tool_input.file_path`.
- Codex: matcher `apply_patch` (aliases `Edit`/`Write`); paths parsed from the patch's
  `*** Add/Update/Delete File:` headers.

If any path is claimed by someone else, the hook denies the edit with a reason like
`src/auth/session.ts is claimed by codex ("refactoring session store", 12m left). Ask them
in the room: comms say "@codex can I touch session.ts?"`, and posts a claim-conflict notice
that wakes the holder.

Known gap: edits made via shell (`sed -i`, `cat >`) bypass the hook. `comms check --all`
(or a git pre-commit hook installed with `comms install --git`) reports files changed
under someone else's claim.

---

## 7. Mode A: `comms ask`

```
comms ask codex "Is there a race in flush()?" --file src/queue.ts
comms ask claude "Review this migration plan" --file docs/plan.md --room db
comms ask codex --continue <ask-id> "What about the retry path?"
```

- Starts the target agent **headless** in the project root, waits, prints the final answer
  to stdout, exits. Meant to be run as a background task by the asking agent.
- **Read-only by default.** `--write` opts in to edits.
  - Codex: `codex exec --sandbox read-only --json -o <tmpfile> "<prompt>"`
  - Claude: `claude -p --output-format json --allowedTools Read,Grep,Glob,"Bash(git log:*)","Bash(git diff:*)","Bash(git show:*)" "<prompt>"`
- **Context sent:** who is asking; the question; the `--file` paths (the agent reads them
  itself; small files are also inlined); and, if asked from a room, the last 20 room
  messages. No automatic diff dump; the agent can run `git diff` itself.
- **Follow-ups:** `--continue <ask-id>` resumes the same agent session
  (`codex exec resume <session> -c sandbox_mode=read-only`; `claude -p --resume <session>`).
  Note `codex exec resume` rejects `--sandbox`, so the sandbox is passed via `-c`.
- Recorded in `.comms/asks/<id>.json`; if a room is in scope, `ask`/`answer` records are
  posted there too (non-waking).
- **Recursion guard:** asks set `COMMS_DEPTH`; an asked agent can't `comms ask` further
  (depth limit 1, configurable).
- `--timeout` (default 10m), `--model` passthrough.

---

## 8. Agent adapters

Each supported agent is a Go adapter:

```go
type Adapter interface {
    Name() string                                    // "claude", "codex"
    Detect() bool                                    // am I running inside this agent?
    Ask(ctx context.Context, req AskRequest) (AskResult, error)
    Launch(opts LaunchOptions) (*exec.Cmd, error)    // interactive session for `comms run`
    Install(project string, opts InstallOptions) error // hooks + instruction snippet
}
```

v1 ships `claude` and `codex`. Adding Gemini CLI etc. later = one new adapter.

---

## 9. Commands

| Command | What it does |
|---|---|
| `comms init` | Create `.comms/`, default config, add to `.gitignore`. |
| `comms install <agent> [--git]` | Install hooks (claims, delivery) and an instruction snippet (CLAUDE.md / AGENTS.md section) for that agent. Idempotent. |
| `comms run <agent> [--name n] [--room r]` | Launch the agent interactively with `COMMS_AS`/`COMMS_ROOM`/`COMMS_ROOT` set, join the room, and give it a bootstrap prompt: who's in the room, recent history, the protocol, "start `comms listen` in the background now". |
| `comms join` / `comms leave` | Join or leave a room explicitly (posts a `system` message). |
| `comms say "<text>" [--human]` | Post a message. `@name` mentions wake people. |
| `comms listen [--timeout d]` | Block until a waking message arrives; print it (+ digest); exit. |
| `comms read [--since seq] [--tail n]` | Print room history without moving the cursor. |
| `comms watch [--room r]` | Live chat view for the human, with an input line that posts as the human. |
| `comms rooms` | List rooms and participants. |
| `comms ask …` | Mode A (§7). |
| `comms claim` / `release` / `claims` / `check` | Claims (§6). |
| `comms budget [n]` | Show or set the room's turn budget. |
| `comms hook <event>` | Internal: entry point the installed hooks call (reads hook JSON on stdin). |

Global flags: `--as`, `--room` (default `$COMMS_ROOM`, else `main`), `--json` for
machine-readable output on every command.

---

## 10. Config (`.comms/config.toml`)

```toml
human_name   = "you"
default_room = "main"
turn_budget  = 20
claim_ttl    = "30m"
idle_wait    = "10m"   # how long Codex's Stop hook holds the turn open waiting for messages; 0 disables
ask_timeout  = "10m"
ask_depth    = 1

[agents.claude]
command = "claude"

[agents.codex]
command = "codex"
```

---

## 11. Implementation

- **Go**, single static binary, macOS + Linux. Fast startup matters because hooks call
  `comms` on every tool use.
- Deps kept small: `fsnotify`, a TOML parser, `cobra` (or stdlib `flag`) for the CLI,
  `bubbletea` for `comms watch`.
- Module: `github.com/jinomee/comms`.

### Milestones
1. **Core:** project discovery, `init`, identity, rooms/log/cursors with locking,
   `say` / `read` / `listen` / basic `watch`. Unit tests on concurrent writers.
2. **Mode A:** `ask` with claude + codex adapters, read-only default, `--continue`, recursion guard.
3. **Claims:** `claim` / `release` / `claims` / `check`, `PreToolUse` hooks for both agents.
4. **Live sessions:** `install`, `run` + bootstrap prompt, Stop/PostToolUse delivery hooks,
   turn budget.
5. **Polish:** `watch` TUI, `--json` everywhere, docs.

---

## 12. Open questions

- Env marker Codex sets in its shell tool, for identity auto-detect (§4).
- Waking an idle Codex beyond `idle_wait`: tmux/pty injection in v2?
- Should the bootstrap prompt be re-sent after context compaction? (Both agents have
  `PostCompact`/`SessionStart` hooks that could re-inject room state.)
- Message size limits / how to share big things (diffs, logs): attach as file paths under
  `.comms/attachments/`?
- Should claims also be offered for non-file resources (e.g. "running the dev server",
  "the DB")?
