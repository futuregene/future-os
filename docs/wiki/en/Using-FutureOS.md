# Using FutureOS

This is a tour of the app: the three-panel layout, Chat vs. Workspace, talking to the agent, the approval mechanism, and how to check the agent's work.

---

## The three-panel layout

FutureOS is organized into three columns:

- **Left — navigation.** From top to bottom you'll find: **New Chat**, **Models** (a shortcut into settings), **Skills**, your **Workspaces** (each expands to its conversations), your **Chats**, and **Settings** at the bottom. You can collapse the left panel to give the conversation more room.
- **Center — the conversation.** Your messages, the streaming reply, plans, tool activity, command previews, errors, and approval cards. The input box is fixed at the bottom.
- **Right — the context panel.** See what the agent is doing (Runs / Review / Artifacts). It's collapsible — open it when you want to check the work, hide it when you don't.

---

## Chat vs. Workspace

Each conversation is its own independent agent session — they don't interfere with each other.

| | **Chat** | **Workspace** |
|---|---|---|
| How you create it | **New Chat** — just start typing | **Workspace** — open a folder on your computer |
| Best for | Quick questions, one-off tasks | Real projects tied to a folder |
| Right panel shows | **Runs** and **Artifacts** | **Runs** and **Review** |

- A **Chat** is the fastest way to ask something. Its work area is temporary.
- A **Workspace** binds a real folder, so the agent reads and changes files there — and the **Review** view shows exactly what changed.

You can rename, pin, or delete conversations from the menu next to each one in the left panel.

---

## Talking to the agent

- Type in the input box and press **Enter** to send. Use **Shift+Enter** for a new line.
- The **model selector** and **thinking level** control sit inside the input box — you can switch models per conversation.
- Attach up to **4 images** per message with the paperclip button, by pasting, or by dragging files onto the box.
- While a reply is streaming, the send button becomes a **stop** button so you can interrupt.

---

## The approval mechanism — you're in control

This is the heart of FutureOS. When the agent wants to do something with real-world consequences — **read or write a file, run a shell command, delete files, or write outside the workspace** — it **stops** and shows an **approval card** just above the input box. It waits for you, with **no timeout**.

The card tells you exactly what's being requested — the command to run, the files to write (with a preview), or the paths involved. Then you choose:

- **Allow once** — let this one action through and continue.
- **Deny** — cancel the action. The agent is told, so it can adjust and try a different approach.
- **Allow in this workspace / this chat** (when offered) — save a rule so similar actions in this project don't ask again. You can edit the path pattern before saving.

> Keyboard: **Cmd/Ctrl+Enter** approves, **Esc** denies (or closes the rule editor first).

### Approval modes

You can set how much the agent asks in **Settings → General**, or from the shield control in the input box:

- **Manual** — prompts before file reads and writes; read-only commands run automatically.
- **Sandboxed** (macOS only) — commands run inside the macOS sandbox; file operations still prompt.
- **Unrestricted** — no prompts and no sandbox; everything runs. Use with care.

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

### Artifacts (Chat)

For a Chat, the **Artifacts** view collects the outputs the agent produced — reports, summaries, tables, and files. You can preview them, copy their contents, and export or open the originals. You can also upload a file into Artifacts yourself.

---

## See also

- [[Quick Start|Quick-Start]] — the fast path to your first answer.
- [[Settings]] — providers, models, and approval mode.
- [[Skills]] — capability packs the agent uses automatically.
