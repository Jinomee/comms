# comms reference

The full reference for comms' messaging engine: launching agents, messaging, subscriptions, the TUI, cross-device relay, and configuration. Install and the comms-specific features (ask, claims, turn budget, `init`) are covered in the [README](../README.md).

This document comes from [hcom](https://github.com/aannoo/hcom) by aannoo (MIT), the project comms was forked from, with names updated.

---

## Quickstart

Terminal 1:

```bash
comms claude
```

Terminal 2:

```bash
comms codex
```

Prompt:

- `ask the other agent their favorite cake`
- `review what claude did and send it fixes`
- `spawn 3x opencode, split work, collect results`
- `fork yourself to investigate the bug and report back`
- `when codex goes idle, send it the next task`

Open the TUI dashboard:

```bash
comms
```

---

## What agents can do

**Message** each other in real time: mid-turn or wake immediately when idle

**Observe** each other: status, transcripts, file edits, live terminal screens, command history.

**Subscribe** and notify on status changes, file edits, collisions, specific events. React automatically.

**Spawn**, **fork**, **resume**, **kill** in any terminal emulator or headless.

---

## How it works

Hooks record activity to a local SQLite database and deliver messages from it.

```text
agent → hooks → db → hooks → other agent
```

Hooks activate only when an agent is launched with `comms` in front. Normal usage is unaffected.

Any other AI tool without hooks can join by running `comms start`. Any process can wake agents with `comms send`.

---

## Terminal

Every agent runs in a real terminal you can see, scroll, and interrupt. Any emulator works for spawning. **kitty**, **wezterm**, **tmux**, **zellij**, **waveterm**, **cmux**, **herdr** also support closing panes from `comms kill`.

To configure a custom terminal open/close setup, tell an agent to run:

```bash
comms config terminal --info
```

---

## Cross-device

Connect agents across machines via MQTT relay.

```bash
comms relay new               # get token
comms relay connect <token>   # on each device
```

```bash
comms relay status            # check connection
comms relay off|on            # toggle
```

<details>
<summary>Relay Security</summary>

### Security

- Relay payloads are end-to-end encrypted. Brokers do not see data.
- Treat the join token like an SSH key or API key.
- If the token may have leaked, run `comms relay off --all` to disconnect all devices.
- Use a private/custom/self-hosted broker with `--broker` and `--password` for better security.

### Security model

`comms relay` is one trust domain for one operator's devices. Membership is all-or-nothing. There are no scoped roles, read-only peers, or per-device permissions.

Relay payloads use a shared PSK with XChaCha20-Poly1305. The encryption binds each payload to the relay, topic, and timestamp. A replay guard drops duplicate envelopes inside a freshness window.

Brokers and network observers cannot read or forge payloads without the PSK. They can still see metadata: topic names, timing, message sizes, and connection patterns.

### What the token means

The join token contains the relay ID, broker URL, and raw PSK. comms does not ask a server to validate it. It has no expiry, no scope, and no revocation list.

On public brokers, a leaked token gives an attacker full control of the relay. They can decrypt captured traffic, publish authenticated relay traffic, send text to listening agents, launch agents on enrolled devices, kill running agents, and use remote relay RPCs. If those agents can run tools, treat that as shell access on every enrolled device in the relay.

On private brokers with `--password`, the token still leaks the PSK, so captured traffic is still exposed. But the token alone is not enough to publish unless the attacker also has the broker password. Use a private broker when broker-side access control matters, or when the metadata shape of your traffic is itself sensitive. `--password` is broker access control, not another layer of message encryption.

### Limits by design

- Forward secrecy. A leaked PSK can decrypt old captured traffic.
- Per-device attribution inside a relay. Sender identity is routing metadata, not authorization. Every enrolled device speaks with full authority.
- Prompt injection from an authenticated peer. Enrollment is total trust — a peer can launch, kill, and drive agents via RPC, not just send messages. Only enroll devices you would give shell access to.
- Local OS compromise. comms trusts the local user account and `~/.comms/config.toml`. It does not defend against another user on the same account or malware with filesystem access.

### Storage

The PSK is stored in `~/.comms/config.toml`. On Unix, comms writes that file with mode `0600`.

comms keeps the PSK out of environment variables. Remote `config_get` and `config_set` refuse `relay_psk`, `relay_token`, `relay_id`, and the broker URL. `comms relay status` shows only a short fingerprint so two devices can verify they share the same key without printing it.

Anyone who can read that file — another user on the same OS account, malware, or a backup written without preserving permissions — has the full PSK.

### Incident response

Run `comms relay off --all`. It asks every reachable trusted peer to disable the relay, then disables it locally, so your agents stop acting on attacker messages. It is best-effort damage control, not containment: the attacker's device ignores the request.

The PSK cannot be revoked. There is no server to notify and no denylist to update. Anyone who has the PSK can keep using the old relay until you stop using it.

To keep using relay after a leak, create a new relay with `comms relay new` and move every trusted device to the new token. Rotation also changes the `relay_id`, so retained state on the old broker topics is orphaned.

</details>

---

## Troubleshoot

```bash
comms status                  # diagnostics
```

```bash
comms reset all               # clear and archive: database + hooks + config
```

---

## Uninstall

Safely remove all comms hooks:

```bash
comms hooks remove
```

Then remove binary:

```bash
cargo uninstall comms
# or: rm "$(which comms)"
```

---

## Reference

<details>
<summary>Tools</summary>

### Supported tools

| Tool | Message delivery | Connect |
|---|---|---|
| Claude Code | automatic | `comms claude` |
| Gemini CLI | automatic | `comms gemini` |
| Codex CLI | automatic | `comms codex` |
| Antigravity CLI | automatic | `comms agy` |
| OpenCode | automatic | `comms opencode` |
| Kilo Code | automatic | `comms kilo` |
| Pi | automatic | `comms pi` |
| Oh My Pi | automatic | `comms omp` |
| Cursor CLI | automatic | `comms cursor-agent` |
| Kimi | automatic | `comms kimi` |
| Copilot CLI | automatic | `comms copilot` |
| Grok Build | automatic | `comms grok` |
| Anything else | manual via `comms listen` | `comms start` (run inside tool) |

```bash
comms r <session_id>           # Resume a session started outside comms
comms f <session_id>           # Fork a session in comms
```

#### Claude Code headless and subagents

Detached background processes in print mode stay alive. Manage through the TUI.

```bash
comms claude -p 'say hi in comms'   # print mode (separate Agent SDK credits)
comms claude --headless            # Run normal claude in background pty (works for any tool)
```

For subagents, run `comms claude`, then prompt:

> run 2x task tool and get them to talk to each other in comms

</details>


<details>
<summary>CLI</summary>

### CLI commands

What you might type from a shell. Agents run their own commands that they learn from the comms CLI primer (~700 tokens) at launch. `comms <command> --help` for full flags.

### Spawn

```bash
comms [N] claude|gemini|codex|agy|opencode|kilo|pi|omp|cursor-agent|kimi|copilot|grok   # launch N agents
comms r <name|session_id>     # resume agent
comms f <name|session_id>     # fork session
comms kill <name|tag:T|all>   # kill + close terminal pane
```

comms launch flags:

| Flag | Purpose |
|---|---|
| `--tag <name>` | Group label — agents can be addressed as `@tag` |
| `--terminal <preset>` | Where windows open: `default` (auto-detect), `kitty`, `wezterm`, `tmux`, `cmux`, `iterm`, etc… |
| `--dir <path>` | Directory where the agent launches |
| `--headless` | Run in background pty with no terminal window |
| `--device <name>` | Spawn on a remote device (via relay) |
| `--comms-prompt <text>` | Initial user prompt |
| `--comms-system-prompt <text>` | Append to system prompt |

Anything else is forwarded to the tool: `--model sonnet`, `--yolo`, etc.

### Other commands

```bash
comms                           # TUI dashboard
comms send -b @luna -- hey      # one-off message to an agent
comms list                      # show all active agents
comms term [name]               # view/inject into an agent's PTY screen
comms events --wait <filters>   # Block until match for scripting
comms update                    # update comms version
```

`comms run docs --cli` for all commands.

</details>

<details>
<summary>Config</summary>

### Configuration

Config lives in `~/.comms/config.toml`. Precedence: defaults < `config.toml` < env vars.

```bash
comms config                           # show all values with sources
comms config <key>                     # get
comms config <key> <value>             # set
comms config <key> --info              # detailed help for a key
comms config -i <name> <key> <value>   # per-agent override at runtime
```

### Keys

| Key | Purpose |
|---|---|
| `tag` | Group label — launched agents become `tag-name` |
| `hints` | Text appended to every message the agent receives |
| `notes` | Text appended to bootstrap (one-time, at launch) |
| `auto_approve` | Auto-approve safe comms commands (send/list/events/…) |
| `auto_subscribe` | Event subscription presets: `collision`, `created`, `stopped`, `blocked` |
| `name_export` | Export instance name to a custom env var |
| `title_mode` | Terminal/tab title behavior: `combined` (default), `label`, or `off` |
| `terminal` | Where new agent windows open (`comms config terminal --info`) |
| `timeout` | Idle timeout for headless Claude (seconds) |
| `subagent_timeout` | Keep-alive for Claude subagents (seconds) |
| `claude_args` / `gemini_args` / `codex_args` / `opencode_args` / `kilo_args` / `pi_args` / `omp_args` / `cursor_args` / `kimi_args` / `copilot_args` / `grok_args` | Default args passed to the tool |

### Scope

```bash
comms config tag mycrew                        # global
comms config -i luna hints "respond in JSON"   # per-agent
COMMS_TAG=dev comms 3 claude                    # per-launch env
```

### Per-project isolation

```bash
export COMMS_DIR="$PWD/.comms"    # isolate comms state (db, logs) to this folder
rm -rf "$COMMS_DIR"              # clean up
```

Run `comms config <key> --info` or `comms run docs --config` for the full per-key reference.

Edit `~/.comms/env` to set external env vars passed to every launched agent.

</details>

<details>
<summary>Workflow Scripts</summary>

### Multi-agent workflows

Bundled and user scripts (`~/.comms/scripts/`) for multi-agent patterns:

```bash
comms run                   # list available scripts
comms run debate "topic"    # run one
comms run docs              # tell agent to run this to create any new workflow
```

### Included scripts

Tell agent to run them:

**`comms run confess`** — An agent (or background clone) writes an honesty self-eval. A spawned calibrator reads the target's transcript independently. A judge compares both reports and sends back a verdict via comms message.

**`comms run debate`** — A judge spawns and sets up a debate with existing agents. It coordinates rounds in a shared thread where all agents see each other's arguments, with shared context of workspace files and transcripts.

**`comms run fatcow`** — headless agent reads every file in a path, subscribes to file edit events to stay current, and answers other agents on demand.

**`comms run onidle`** — waits for an agent to go idle, then types text into another agent (`comms run onidle luna nova 'luna is done, review it'`) or launches a new one with it as the prompt (`comms run onidle luna codex 'review what luna just did'`).

Custom scripts: drop `*.sh` or `*.py` into `~/.comms/scripts/` — auto-discovered, override bundled scripts of the same name. Ask an agent to author one; `comms run docs --scripts` is the authoring guide.

</details>

<details>
<summary>Build</summary>

### Building from source

```bash
# Prerequisites: Rust 1.88+

git clone https://github.com/jinomee/comms.git
cd comms
cargo build
cargo test
```

### Using local build

Two options:

**Symlink** — simple, dev build is global.

```bash
ln -sf $(pwd)/target/debug/comms ~/.cargo/bin/comms
```

**dev_root** — works regardless of how comms was installed (cargo install, a copied binary, etc.); picks the newer of debug/release automatically:

```bash
comms config dev_root $(pwd)
comms config dev_root --unset  # revert
comms status    # run local build
```

For concurrent worktrees, scope each to its own DB:

```bash
COMMS_DIR=$PWD/.comms COMMS_DEV_ROOT=$PWD comms claude
```

</details>

---

## Contributing

Issues and PRs welcome. The codebase is Rust.

```bash
cargo build && cargo test
comms config dev_root $(pwd)
comms status
just ci  # run the CI gate locally
```

---

## License

[MIT](../LICENSE)
