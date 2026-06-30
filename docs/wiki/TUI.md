# Terminal UI (`future-tui`)

FutureOS includes a **terminal interface**, `future-tui`, in your download. It's an optional, keyboard-driven way to chat with the agent without leaving the terminal — a lighter alternative to the [[desktop app|Using-FutureOS]] for people who live in a shell.

> Prefer a graphical experience? Just use the app — see [[Using FutureOS|Using-FutureOS]].

## Where to find it

`future-tui` ships inside these downloads from the [Releases page](https://github.com/futuregene/future-os/releases):

| System | Package | Location of `future-tui` |
|---|---|---|
| **macOS** | the `.dmg` | inside the app: `/Applications/FutureOS.app/Contents/MacOS/future-tui` |
| **Windows** | the **portable** `.zip` | `future-tui.exe`, in the extracted folder |
| **Linux** | the **portable** `.tar.gz` | `future-tui`, in the extracted folder |

> On Windows and Linux, the command-line tools are in the **portable** package — not the plain installer / `.deb`.

## Launch it

Make sure the agent is running first — if the desktop app is open, it already is; otherwise run `future agent start` (see [[CLI|CLI]]).

Then start the TUI either way:

- run the binary directly — `./future-tui` (macOS/Linux) or `future-tui.exe` (Windows), or
- launch it through the CLI — `future tui`.

It connects to the agent at `127.0.0.1:50051` by default.

## What you can do

The TUI is a full chat client for the agent:

- hold a multi-turn conversation with streaming replies and visible "thinking",
- switch the **model** and **thinking level** on the fly,
- create, switch, and fork **sessions** (your history is saved by the agent),
- watch tool calls run and see their results inline.

Because everything is stored on the agent side, a conversation you start in the TUI is the same one you can continue from `future run --continue` or pick up in the desktop app.

## Getting around

- **Text selection** uses your terminal's native selection (mouse tracking is off), so you can copy normally.
- **Scrolling** is keyboard-driven.
- The input handles multi-line paste and modern key combos in capable terminals.

## Troubleshooting

- **"Connection refused" / nothing happens** → the agent isn't running. Start it with `future agent start` or open the desktop app.
- **macOS blocked it** → open the app once (right-click → Open) to clear the security prompt, then the inner `future-tui` runs fine. See [[Installation]].

See also: [[CLI|CLI]] · [[Using FutureOS|Using-FutureOS]] · [[FAQ]]
