# Settings

Open Settings from the bottom-left of the window: when the left panel is collapsed it is the **gear icon** at the bottom, and when it is expanded it sits in the account menu (click your avatar and email). The left column of the dialog is the navigation, sorted into five groups by what the settings are about:

| Group | Pages |
|---|---|
| **App** | Appearance, Keyboard shortcuts |
| **Conversation** | Permissions, Replies, Storage |
| **Models and skills** | Providers, Models, Skills |
| **Account** | Account, Check for updates, About |
| **Debug** | Reset (test builds add **Fusion** and **Environment**, see the end) |

There is a **search box** at the top of the navigation: type a setting name or a page name and each result names the page it lives on; Enter opens the first one, Esc only clears the box. Typing a page name (say "Storage") gives you that page itself, not every row inside it.

## First, which settings belong to this machine

| | Where it lives | Can the phone see it? |
|---|---|---|
| Appearance (language, light/dark, theme, font size, model output), keyboard shortcuts, completion bell | This client, on this machine | No, and it cannot change it |
| Permissions, Replies, Storage | This machine's own settings | A paired phone reads and changes them |

So the rows that only change what *you* see sit under **App**, and the rows that change how the agent works sit under **Conversation**. The completion bell belongs to the first kind: this machine plays it, and the phone has its own "task finished" notification.

---

## App

### Appearance (a preference of this machine)

- **Language** — follow the system, Chinese or English; it follows the system language until you pick one.
- **Appearance** — light, dark, or follow the system appearance.
- **Color theme** — 14 palettes (FutureOS, Ayu, Catppuccin, Dracula, Everforest, Gruvbox, Kanagawa and more), each with a light and a dark rendition that follow the appearance setting above. The row shows the active palette as a small swatch; **Show all 14 color themes** opens the full catalogue.
- **Font size** — small, medium, large, extra large; it scales text without changing the layout.
- **Model output** — how much of a settled reply the transcript shows:
  - **Default** (the shipped level) — the reply settles to its final answer, with its reasoning, tool calls and intermediate notes folded into a single row above it that you can expand at any time. The work is still shown in full while the answer streams.
  - **Detailed** — the whole process stays in place: reasoning, tool calls and intermediate notes are interleaved with the answer in the order it happened.

  This is how *this client* draws a conversation, not a property of the machine: the phone has the same choice under its own **Appearance** page.

- **Alerts → Completion bell** (on by default) — play a sound and attract the window's attention when the agent finishes. It rings on this machine; the phone has no such row.

### Keyboard shortcuts

Shortcuts are global inside the app. The page offers:

- **Search commands** by name;
- commands listed by group (the most used first), with each group collapsible; the group for the Vibe panel starts collapsed, since those shortcuts only apply while that panel is open;
- click a row to record new keys, **restore the default** for one row or **restore all defaults**;
- the **fixed keys** at the bottom (Enter, Esc and so on) are hard-wired and cannot be changed.

Changes take effect immediately; there is no save button.

---

## Conversation

### Permissions

**Approval mode** decides how much the agent asks before acting:

- **Full access** (default) — no prompts and no sandbox; commands run with your full local permissions.
- **Manual** — file access follows the approval rules; the read-only allowlist runs without asking and everything else needs approval; no OS sandbox.
- **Sandboxed** — commands run inside the macOS sandbox, Linux Bubblewrap, or Windows write protection, and ask when a command needs it; file access still follows the path rules, and the guarantees differ by platform.
- **Automatic review** — the sandbox, plus a model reviewer that decides the requests Sandboxed would have asked you about. It needs the OS sandbox and a signed-in FutureOS account, and the two failures differ: without a usable sandbox, requests use manual review and the setting is kept; signing out or losing credentials downgrades the saved setting to **Sandboxed**, and signing in again does not restore it.

When the sandbox is unavailable, or Linux only offers write protection, this page gains a row explaining why. See [[Approvals and sandboxing|Sandbox]] for defaults, Linux setup, diagnostics and limitations, and [[Using FutureOS|Using-FutureOS]] for approval cards.

### Replies

Both rows on this page are the same on every client, because they describe the reply itself:

- **Choice cards** (off by default) — let a reply close with a card of options you can tick. While off, the card is not drawn at all **and** the agent is never told the format exists, so no reply offers one; a conversation from before you turned it off shows its cards again once it is on. Design: [choice cards](../../design/choice-cards.md).
- **Generate a title after the first answer** (on by default) — title a new conversation from its first successful answer only; later answers do not trigger it, and it does not compact the conversation.

### Storage

Under **Automatic conversation archiving**:

- **Clean run logs when archiving** (on by default) — keeps the chat text, command titles and generated files, and clears command output and run replays; you can keep talking in an unarchived conversation, but a cleaned run log cannot be recovered.
- **Archive idle conversations automatically** (off by default) — archives only unpinned conversations idle past the threshold; running conversations, ones waiting for approval and the one you are reading are skipped, and workspaces themselves are not archived.
- **Idle days** (1–365, default 30) — idle means no new message or run activity since the last exchange; reading or renaming does not restart the clock, and unarchiving starts it again.

**Organize the database** at the end of the page is a one-off maintenance action, not a reset: it walks every conversation, upgrades old record formats, reclaims process data that can go (command output, run replays and snapshots) and compacts the text, then returns the file space to the system. When it finishes it reports the size before, the size after and the space released; conversations that are running, malformed or failing are skipped and the failures can be expanded. The chat text is kept, and the app does not need a restart.

---

## Models and skills

### Providers

A provider is where your models come from.

The first row under **Built-in** is **Future**, which is your FutureOS account: that row shows only the sign-in status and **Update model list** — signing in and out both live on the **Account** page, and while you are signed out the button there takes you to it. Other built-in providers (such as DeepSeek, OpenAI, Anthropic, Google, and more) are listed too — click **Configure** to set or update the API key for any of them. Use **More providers** to reveal the full list.

**Custom** providers are your own. Click **+ Add custom provider** and fill in:

- **Name** — a display name (optional).
- **Provider ID** — a unique id (lowercase letters, digits, `-`, `_`).
- **API type** — OpenAI Completions, OpenAI Responses, or Anthropic.
- **Base URL** — the provider's API address (`http`/`https`).
- **API Key**.
- **Models** — one or more model IDs, each with a display name, a required context window and maximum output tokens, and optional thinking support, modality, and per-million-token prices.

The app validates the fields and checks that the provider ID is unique. You can **Edit** or **Remove** a custom provider later.

> A provider's API key is stored separately from any other credentials.

### Models

The Models page lists all available models, **grouped by provider**:

- **Search** filters by model or provider name; group headers collapse, and a provider's whole list can be shown or hidden at once.
- **Toggle each model's visibility** — hidden models are removed from the model selector, so your list stays focused on the ones you actually use.

The model selector in the input box draws from the same list and shows which provider each model comes from.

### Skills

- **Auto-upgrade skills** — silently upgrade installed skills to their latest version each time the app opens.
- **Skill recommendations** (off by default) — when you send a message, suggests at most one skill you do not have installed yet.

Installing, upgrading and removing skills themselves happen on the **Skills** page in the left panel. See [[Skills]].

---

## Account

### Account

Shows your signed-in FutureOS account: status, balance and **Recharge**. It is also the only place to **sign in** and **sign out** (with a confirmation). The balance pays for model calls made through the platform, and new tasks cannot run at zero. The community edition uses its own models and keys, is not billed through the platform, and therefore shows no balance.

### Check for updates

Checks whether a newer version of FutureOS is available, then downloads and installs it; it takes effect after a restart. The page shows **when it last checked** (a cached result is never passed off as a fresh check), and offers a manual download when an installer does not apply. The version number itself is on the **About** page and is not repeated here. See [[Install FutureOS|Installation]] for how to apply an update.

### About

- The app name and the **version number** (the only place it is shown); test builds also carry a **Test build** badge;
- the license (MIT), the website and GitHub;
- the WeChat official account card (QR code, copy the name, read an article) — not shown in the community edition;
- the open-source software it is built on.

---

## Debug (mostly test builds)

In a release build this group holds only **Reset**; **Fusion** and **Environment** appear in test builds only.

### Reset

- **Clear data** — wipes FutureOS's local data and temporary workspace and restarts the app: conversations, background programs and review records are removed, while sign-in and provider settings are kept. It cannot be undone, and it asks for confirmation.
- **Write-protection permissions → Reset write protection** (Windows only) — removes the folder permissions FutureOS added for write protection; no files are deleted, and you should wait for a running task to finish before resetting.

Organizing the database is not here: it does not wipe anything, and it has moved to the **Storage** page.

### Environment (test builds only)

Switches sign-in and API addresses between the **production** and **test** environment; switching signs you out and restarts the app, so you sign in again in the new environment. The same page carries the **community edition** switch, which hides commercial features (account balance, the Future provider and so on).

### Fusion (test builds only)

Configures `/fusion` discussions: once enabled, `/fusion` starts a discussion between multiple models; turning it off does not stop a discussion already under way. The host frames the question and writes the summary, and 2–5 participant roles each pick their own model and thinking level; the same model can appear in several roles with different thinking levels.

---

## On the phone

A paired phone has pages of the same name — **Permissions / Replies / Storage / Models / Providers / Skills / Scheduled task settings** — and they show that desktop's settings. The phone's own preferences (appearance, language, WeChat official account) live one level up. The phone's settings also have a search box at the top, with the same rules as the desktop.

---

## See also

- [[Quick Start|Quick-Start]] — connect your FutureOS account and send your first message.
- [[Using FutureOS|Using-FutureOS]] — the approval mechanism in detail.
- [[Skills]] — capability packs the agent can use.
- [[Approvals and sandboxing|Sandbox]] — sandbox differences by platform and diagnostics.
