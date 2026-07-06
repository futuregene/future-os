# Command line tool (`future-cli`)

FutureOS ships with an **optional** command-line tool called `future-cli`. It comes with every download.

> **You probably don't need this.** The desktop app already handles everyday use. Reach for the CLI only when you want to script things, automate a task, or work purely in a terminal. **If you're not comfortable with a terminal, you can skip this page.**

---

## Where it is

The tool travels with the app:

| System | Location |
|---|---|
| **macOS** (`.dmg`) | Inside the app: `/Applications/FutureOS.app/Contents/MacOS/future-cli` |
| **Windows** (portable `.zip`) | `future-cli.exe` in the unzipped folder |

> On Windows, the command-line tool is in the **portable** package. The regular installer version contains the app and its background service, not a separate `future-cli.exe`.

---

## Running it

Open a terminal in the folder that contains the binary, then run it with `--help` to see everything:

```bash
future-cli --help
```

To make it easier to run from anywhere, add its folder to your `PATH`, or set up an alias. For example, on macOS:

```bash
alias future-cli="/Applications/FutureOS.app/Contents/MacOS/future-cli"
```

### The agent must be running

Every command connects to the FutureOS agent (the background service). If the **desktop app is open**, the agent is already running. Otherwise, start it first:

```bash
future-cli agent start
```

---

## Command groups

### `auth` — sign in and out

```bash
future-cli auth login     # sign in via your browser
future-cli auth status    # show whether you're signed in
future-cli auth logout    # sign out
```

### `agent` — manage the background agent

```bash
future-cli agent start
future-cli agent stop
future-cli agent restart
future-cli agent status
```

### `run` — send a one-off prompt and print the answer

```bash
future-cli run "Explain this project"
```

Useful options and forms:

| Form | What it does |
|---|---|
| `--model <model>` | Choose the model. Supports `model:thinking`, e.g. `sonnet:high`. |
| `--thinking <level>` | Thinking level: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`. |
| `@<path>` | Include a file's contents in the prompt. |
| `--continue`, `-c` | Continue the most recent session. |
| `--cwd <dir>` | Set the working directory. |
| `--mode json` | Print the answer as JSON instead of text. |
| `--no-session` | Don't save this exchange as a session. |

Examples:

```bash
future-cli run --model sonnet:high "Review the changes"
future-cli run @README.md "Summarize this file"
echo "some text" | future-cli run "Clean up this text"
```

### `tools` — list and call tools

```bash
future-cli tools list
future-cli tools call <name> --args '<json>'
future-cli tools call <name> --stdin
future-cli tools call <name> --args '<json>' --output result.png
```

File-path arguments are converted automatically where a tool expects file content.

### `skills` — manage capability packs

```bash
future-cli skills list
future-cli skills install <name>
future-cli skills uninstall <name>
```

### `channel` — chat platform bridge (advanced)

Bridges external chat platforms to the agent (`start` / `stop` / `restart` / `status`). Most people won't need this.

---

## Tips

- **macOS blocked it the first time?** Open the FutureOS app once via right-click → **Open** to clear the block, then the CLI runs too.
- **"Connection refused"?** The agent isn't running. Run `future-cli agent start`, or just open the desktop app.

---

## See also

- [[Install FutureOS|Installation]] — where the tool ships.
- [[Skills]] — the same skills, managed from the app.
- [[FAQ]] — common issues.
