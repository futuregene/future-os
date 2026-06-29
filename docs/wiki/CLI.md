# Command-line tool (`future-cli`)

FutureOS includes an optional command-line tool, **`future-cli`**, in your download. The [[desktop app|Using-FutureOS]] does everything most people need — reach for the CLI when you want to script tasks, automate runs, or work entirely from a terminal.

> Comfortable in a terminal? Read on. Otherwise you can safely skip this page.

## Where to find it

`future-cli` ships inside these downloads from the [Releases page](https://github.com/futuregene/future-os/releases):

| System | Package | Location of `future-cli` |
|---|---|---|
| **macOS** | the `.dmg` | inside the app: `/Applications/FutureOS.app/Contents/MacOS/future-cli` |
| **Windows** | the **portable** `.zip` | `future-cli.exe`, in the extracted folder |
| **Linux** | the **portable** `.tar.gz` | `future-cli`, in the extracted folder |

> On Windows and Linux, the command-line tools are in the **portable** package — the plain Windows installer and Linux `.deb` contain only the app and its agent.

## Running it

Open a terminal in the folder that contains the binary, then:

- **Windows:** `.\future-cli.exe --help`
- **Linux:** `./future-cli --help`
- **macOS:** `/Applications/FutureOS.app/Contents/MacOS/future-cli --help`

To type just `future-cli` from anywhere, add its folder to your `PATH` (or make a shell alias). On macOS, for example:

```bash
alias future-cli="/Applications/FutureOS.app/Contents/MacOS/future-cli"
```

## The agent must be running

Every command talks to the FutureOS agent. If you also use the desktop app, the agent is already running and the CLI just connects to it. Otherwise, start it yourself:

```bash
future-cli agent start
```

## Command groups

```
future-cli <group> <command> [options]
future-cli --help
```

| Group | What it does |
|---|---|
| [`auth`](#auth) | Sign in / out |
| [`agent`](#agent) | Start/stop the background agent |
| [`run`](#run) | Send a one-shot prompt and print the answer |
| [`tui`](#tui) | Launch the [[terminal UI|TUI]] |
| [`tools`](#tools) | List and call tools |
| [`skills`](#skills) | Manage skill packs |
| `channel` | Manage chat-channel bridges (advanced) |

### auth

| Command | Description |
|---|---|
| `future-cli auth login` | Sign in (opens your browser to authorize). |
| `future-cli auth status` | Show whether you're signed in. |
| `future-cli auth logout` | Sign out. |

### agent

| Command | Description |
|---|---|
| `future-cli agent start` | Start the background agent. |
| `future-cli agent stop` | Stop it. |
| `future-cli agent restart` | Restart it. |
| `future-cli agent status` | Show its status. |

### run

Send a prompt and print the response — ideal for scripts.

```bash
future-cli run "Explain what this folder contains"
future-cli run --model deepseek-v4-flash "Write release notes"
future-cli run @README.md "Summarize this file"
echo "some text" | future-cli run "Summarize stdin"
```

- `@<path>` includes a file's contents (repeatable).
- Useful options: `--model <id>` (supports `model:thinking`, e.g. `sonnet:high`), `--thinking <level>`, `--continue`/`-c` (continue the last session), `--cwd <dir>`, `--mode json`, `--no-session` (don't save).

Run `future-cli run --help` for the full list.

### tui

```bash
future-cli tui
```

Launches the [[terminal UI|TUI]]. (You can also run the `future-tui` binary directly — see the [[TUI]] page.)

### tools

```bash
future-cli tools list
future-cli tools call web_search --args '{"query": "gRPC streaming"}'
future-cli tools call image_gen --args '{"prompt": "a red fox"}' --output fox.png
```

- `--args '<json>'` passes arguments; `--stdin` reads them from standard input.
- File-path arguments (e.g. `image_path`) are converted automatically — just pass the path.

### skills

```bash
future-cli skills list
future-cli skills install <name>
future-cli skills update <name>
future-cli skills uninstall <name>
```

See [[Skills]] for what each skill does.

---

## Tips

- **macOS blocked the binary?** Open the app once (right-click → Open) to clear the security prompt, then the inner `future-cli` runs fine. See [[Installation]].
- **"Connection refused"?** The agent isn't running — `future-cli agent start`, or open the desktop app.

See also: [[TUI]] · [[Using FutureOS|Using-FutureOS]] · [[FAQ]]
