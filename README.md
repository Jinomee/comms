# comms

**Let your coding agents talk to each other.** Get a second opinion from a different model, or run Claude Code and Codex side by side in one project and have them coordinate instead of collide.

`comms` is a fork of [hcom](https://github.com/aannoo/hcom) (MIT) by aannoo. hcom provides the messaging engine: launching agents through a wrapper, @mentions, and delivering messages mid-turn or by waking idle agents. comms adds the pieces for two models sharing one codebase:

| | What it does |
|---|---|
| **`comms ask`** | One-shot, read-only question to a fresh Claude or Codex. Prints the answer and exits, so it works well as a background task. |
| **`--read-only`** | Launch a live agent that can read and message but can't edit files. |
| **Claims** | Soft, expiring file locks. Edits to another agent's claimed files are blocked, and the blocked agent is told who holds them. |
| **Turn budget** | Stops two agents from messaging each other forever without you. |
| **Per-project data** | `comms init` keeps a project's agents and messages in `.comms/`, separate from other projects. |

Supports `claude` and `codex` for the comms features. Everything hcom supports (opencode, gemini, cursor, …) still works for messaging.

---

## Install

From source (Rust 1.88+):

```bash
git clone https://github.com/jinomee/comms.git
cd comms
cargo install --path .
```

This installs two binaries next to each other:
- **`comms`**: the command you use.
- **`hcom`**: the engine. Agents call it internally.

If you already have upstream hcom installed, this `hcom` replaces it. The two are compatible.

---

## Quickstart

```bash
cd your-project
comms init            # creates .comms/ (gitignored) for this project
```

Terminal 1 and terminal 2:

```bash
comms claude
comms codex
```

Then prompt either agent normally:

- `ask codex to review your plan before you start`
- `claim src/auth/** while you refactor it, and tell claude what you're doing`
- `split the work with claude: you take the API, they take the tests`

Run `comms` with no arguments to watch the conversation in a dashboard.

---

## Second opinions: `comms ask`

```bash
comms ask codex "Is there a race in flush()?" --file src/queue.rs
comms ask claude "Review this migration plan" -f docs/plan.md
git diff | comms ask codex -          # question from stdin
```

`comms ask` starts the agent headless in the current directory, waits, and prints its final answer.
- **Read-only by default.** Claude gets read-only tools, and Codex runs in `--sandbox read-only`. Pass `--write` to let it edit.
- **Follow-ups:** `--continue <session>` keeps the same conversation. The session id is printed after each answer.
- **Other flags:** `--model`, `--timeout` (seconds, default 600) and `--json`.
- **No nesting:** an asked agent can't `ask` again.

An agent can run `comms ask` itself, for example Claude asking Codex. In Claude Code, run it as a background task; the answer arrives when the command finishes.

## Read-only agents

```bash
comms codex --read-only     # reviewer that can read and message, not edit
comms claude --read-only
```

What `--read-only` does for each agent:
- **Claude:** its edit tools are disabled. Bash stays available so it can still send messages, which means shell edits are still possible.
- **Codex:** runs in a read-only sandbox.

## Claims

```bash
comms claim "src/auth/**" -n "moving middleware to async"   # default 30m
comms claims                                                 # who holds what
comms claims --check src/auth/session.rs                     # exit 1 if someone else holds it
comms release "src/auth/**"                                  # or: comms release --all
```

How claims behave:
- **Blocking:** if another agent tries to edit a claimed file, its edit hook blocks the change and tells it who holds the file and how to ask. Claude's Write/Edit and Codex's apply_patch are both covered.
- **Notifying:** the holder gets an @mention saying someone is waiting.
- **Renewal and expiry:** claims renew while the holder keeps editing under them. They expire after their TTL (`--ttl 2h`), and they're dropped when the holder exits.
- **Subagents:** a Claude subagent shares its parent's claims.

Agents run these commands themselves (they're auto-approved). You can claim files too: from a plain terminal, you're `bigboss`.

> Edits made through the shell (`sed -i`, `cat >`) don't go through the edit hooks, so claims can't block them.

## Turn budget

Each agent-to-agent message counts toward that pair of agents. After **20** messages with no human input, comms refuses further messages between them. The agents are told to stop and report back to you.

```bash
comms budget          # per-pair counts
comms budget reset    # let them continue
comms budget 50       # change the limit (0 = off), or set HCOM_TURN_BUDGET
```

Counts reset when you send a message, or when you type directly into an agent. Agents can see the budget but can't change it.

## Per-project data

`comms init` creates `<git root>/.comms/hcom` and adds `.comms/` to `.gitignore`. Inside the project, every `comms` or `hcom` call uses that directory, including calls agents make. Outside any initialized project, data lives in `~/.hcom` as in upstream. An explicit `HCOM_DIR` always takes precedence.

---

## Everything else

All of hcom's features still apply: spawning and forking agents, subscriptions, threads, transcripts, cross-device relay, the TUI and the config. See **[docs/HCOM.md](docs/HCOM.md)** (hcom's own README) and `comms --help`.

[SPEC.md](SPEC.md) has the original design for comms and notes on how each part maps onto hcom.

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets
```

Upstream hcom is tracked as the `upstream` remote. Internal names (`hcom`, `HCOM_*`, `~/.hcom`) are left unchanged so upstream fixes still merge cleanly:

```bash
git remote add upstream https://github.com/aannoo/hcom.git   # once
git fetch upstream && git merge upstream/main
```

## License

MIT. Copyright (c) 2025 aannoo (hcom), with comms changes by its contributors. See [LICENSE](LICENSE).
