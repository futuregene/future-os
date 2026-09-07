# Using FutureOS

This is a tour of the app: the three-panel layout, Chat vs. Workspace, talking to the agent, the approval mechanism, and how to check the agent's work.

---

## The three-panel layout

FutureOS is organized into three columns:

- **Left — navigation.** From top to bottom you'll find: **New Chat**, **Models** (a shortcut into settings), **Skills**, your **Workspaces** (each expands to its conversations), your **Chats**, and **Settings** at the bottom. You can collapse the left panel to give the conversation more room.
- **Center — the conversation.** Your messages, the streaming reply, plans, tool activity, command previews, errors, and approval cards. The input box is fixed at the bottom.
- **Right — the context panel.** See what the agent is doing (Files / Runs / Review). It's collapsible — open it when you want to check the work, hide it when you don't.

---

## Chat vs. Workspace

Each conversation is its own independent agent session — they don't interfere with each other.

| | **Chat** | **Workspace** |
|---|---|---|
| How you create it | **New Chat** — just start typing | **Workspace** — open a folder on your computer |
| Best for | Quick questions, one-off tasks | Real projects tied to a folder |
| Right panel shows | **Files** and **Runs** | **Files**, **Runs** and **Review** |

- A **Chat** is the fastest way to ask something. Its work area is temporary.
- A **Workspace** binds a real folder, so the agent reads and changes files there — and the **Review** view shows exactly what changed.

You can rename, pin, or delete conversations from the menu next to each one in the left panel.

---

## Talking to the agent

- Type in the input box and press **Enter** to send. Use **Shift+Enter** for a new line.
- The **model selector** and **thinking level** control sit inside the input box — you can switch models per conversation.
- Attach local files with the paperclip button, by pasting images, or by dragging files onto the box. Each message supports up to **4 images** (25 MiB each); other file types are not count-limited.
- While a reply is streaming, the send button becomes a **stop** button so you can interrupt.

---

## The approval mechanism — you're in control

When the selected mode and rules require approval — for example for a sensitive file, an external write or a shell command — the agent **stops** and shows an **approval card** above the input box, with **no timeout**. This is not a prompt for every operation: ordinary reads and workspace/temp writes can be allowed, and the desktop defaults to **Unrestricted**, which does not ask. Select Manual or Sandboxed to enable those protections; see [[Approvals and sandboxing|Sandbox]].

The card tells you exactly what's being requested — the command to run, the files to write (with a preview), or the paths involved. Then you choose:

- **Allow once** — let this one action through and continue.
- **Deny** — cancel the action. The agent is told, so it can adjust and try a different approach.
- **Allow in this workspace / this chat** (when offered) — save a rule so similar actions in this project don't ask again. You can edit the path pattern before saving.

> Keyboard: **Cmd/Ctrl+Enter** approves, **Esc** denies (or closes the rule editor first).

### Approval modes

You can set how much the agent asks in **Settings → General**, or from the shield control in the input box:

- **Manual** — file access follows Allow/Ask/Deny path rules; shell commands ask except for the read-only allowlist.
- **Sandboxed** — commands use macOS Seatbelt, Linux system Bubblewrap, or Windows restricted-token write protection when available. File tools still follow path rules; not all calls prompt. Platform guarantees differ.
- **Unrestricted** (default) — no prompts and no sandbox; everything runs. Use with care.

See [[Sandbox]] for Linux installation, unavailable-mode fallback, escalation scope and credential/network limitations. For phone access to desktop sessions, see [[Remote]].

---

## Checking the work (the right panel)

Open the context panel on the right and pick a view from the dropdown at its top.

### Runs

Every background program the agent runs shows up as a card with the **real command**, its status, and a running/finished count. You can:

- **Inspect** a run to see its details.
- **Terminate** a running program.
- **Clear finished** to tidy up.

### Review (Workspace)

For a Workspace, the **Review** view shows the file changes made in the project: the list of changed files, change types (added / modified / deleted / renamed), and per-file diffs. When the folder is under version control, you can also switch to a **"Last run changes"** view to see just what the most recent run changed.

### Files

Every conversation has a **Files** view with the files in its work area: for a **Workspace**, the project folder itself; for a **Chat**, the temporary per-conversation folder the agent works in. Browse the tree, preview files (or open them in your system apps), and attach a file to the conversation from the tree.

---

## See also

- [[Quick Start|Quick-Start]] — the fast path to your first answer.
- [[Settings]] — providers, models, and approval mode.
- [[Skills]] — capability packs the agent uses automatically.
