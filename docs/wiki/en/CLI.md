# Command line tool (`future`)

FutureOS ships with an **optional** command-line tool called `future`. It comes with every download.

> **You probably don't need this.** The desktop app already handles everyday use. Reach for the CLI only when you want to script things, automate a task, or work purely in a terminal. **If you're not comfortable with a terminal, you can skip this page.**

---

## Where it is

The tool travels with the app:

| System | Location |
|---|---|
| **macOS** (`.dmg`) | Inside the app: `/Applications/FutureOS.app/Contents/MacOS/future` |
| **Windows** (installer or portable `.zip`) | `future.exe` in the app folder |

The CLI ships in **every** download — both the installer and the portable package include it, sitting next to the app.

---

## Running it

Open a terminal in the folder that contains the binary, then run it with `--help` to see everything:

```bash
future --help
```

To make it easier to run from anywhere, add its folder to your `PATH`, or set up an alias. For example, on macOS:

```bash
alias future="/Applications/FutureOS.app/Contents/MacOS/future"
```

### The agent must be running

Most commands connect to the FutureOS agent (the background service). If the **desktop app is open**, the agent is already running. Otherwise, start the agent with `future agent` (or the `future-agent` binary directly — both are the same code), or open the desktop app, which starts it automatically.

`future auth login` is the exception: it also works with the agent stopped, writing the key to `~/.future/agent/auth.json` for the agent to pick up on its next start.

---

## Command groups

### `init` — first-time setup

```bash
future init
```

Installs all built-in skills. On macOS and Linux, also links `future` (and `future-agent` when available) into `~/.future/bin/` and prints a PATH setup hint.

### `auth` — sign in and out

```bash
future auth login       # sign in via your browser (device-code flow)
future auth status      # show whether you're signed in
future auth credential  # print the API key + endpoint for shell scripts
future auth logout      # sign out
```

### `account` — your account

```bash
future account profile  # email, user ID, verification status, creation date
future account balance  # credit balance (--json for machine output)
```

### `run` — send a one-off prompt and print the answer

```bash
future run "Explain this project"
```

Useful options and forms:

| Form | What it does |
|---|---|
| `--model <model>` | Choose the model. Supports `model:thinking`, e.g. `sonnet:high`. |
| `--thinking <level>` | Thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`. |
| `@<path>` | Include a file's contents in the prompt. |
| `--continue`, `-c` | Continue the most recent session. |
| `--session <id>` | Connect to an existing session by ID. |
| `--fork <entry-id>` | Fork a new session from a specific entry in the current session. |
| `--permission <level>` | File access: `all`, `workspace` (workspace + temp only), or `none` (read-only outside workspace). |
| `--cwd <dir>` | Set the working directory. |
| `--mode json` | Print the answer as JSON instead of text. |
| `--no-session` | Don't save this exchange as a session. |

Examples:

```bash
future run --model sonnet:high "Review the changes"
future run @README.md "Summarize this file"
echo "some text" | future run "Clean up this text"
```

### `skills` — manage capability packs

```bash
future skills list             # list catalog skills (installed + available)
future skills install <name>   # install a specific skill
future skills install-builtin  # install all built-in future-* skills
future skills uninstall <name> # remove an installed skill
future skills update           # upgrade all installed skills
```

### `tools` — list and call tools

```bash
future tools list
future tools describe <name>
future tools call <name> --args '<json>'
future tools call <name> --stdin
future tools call <name> --args '<json>' --output result.png
```

File-path arguments are converted automatically where a tool expects file content.

### `models` — list available models

```bash
future models            # list models from the running agent
future models --json     # machine-readable output
```

### `agent` — run the agent server

```bash
future agent              # start the agent gRPC server
future agent --help       # agent options: gRPC address, logging, profiling
```

`future agent <args>` runs the agent backend directly with the same arguments
as the standalone `future-agent` binary.

### `tui` / `channel` / `loop` — run the other components

`future` is the unified entry point for every Rust component — each runs the
same code as its standalone binary (the `future-*` binaries still exist as
build targets but no longer ship by default):

```bash
future tui                # terminal UI
future channel            # IM channel bridge: Feishu / DingTalk
future loop status        # loop control plane: goals/todos/gates
```

### `session` — manage sessions

```bash
future session list
future session info <id>
future session rename <id> <name>
future session delete <id>
```

Session data lives in `~/.future/agent/sessions/`.

### `doctor` — environment diagnostics

```bash
future doctor
```

Checks login status, component installation, agent connectivity, configuration, providers/models, sessions, and skills in one pass.

---

## Tips

- **macOS blocked it the first time?** Open the FutureOS app once via right-click → **Open** to clear the block, then the CLI runs too.
- **"Connection refused"?** The agent isn't running. Open the desktop app, or run `future agent` directly.

---

## See also

- [[Install FutureOS|Installation]] — where the tool ships.
- [[Skills]] — the same skills, managed from the app.
- [[FAQ]] — common issues.
