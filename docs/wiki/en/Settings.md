# Settings

Open Settings from the bottom-left of the window: when the left panel is collapsed it is the **gear icon** at the bottom, and when it is expanded it sits in the account menu (click your avatar and email). There's also a **Models** shortcut in the left panel that jumps straight to the Models page.

The settings you'll use day-to-day are **General**, **Providers**, and **Models**. There are also **Phone Control**, **Account**, **Check for updates**, **About**, and **Reset** pages; test builds add an **Environment** switch. The dialog groups them as Desktop, Server and Debug.

---

## General

Desktop-level options for the app:

- **Language** — choose the app's display language.
- **Approval mode** — how much the agent asks before acting:
  - **Manual** — file access follows Allow/Ask/Deny path rules; shell commands ask except for the read-only allowlist.
  - **Sandboxed** — uses macOS Seatbelt, Linux system Bubblewrap, or Windows restricted-token write protection when available; file access still follows path rules. Guarantees differ by platform.
  - **Unrestricted** (default) — no prompts and no sandbox; everything runs.
- **Auto-upgrade skills** — silently upgrade installed skills to their latest version each time the app opens.
- **Skill recommendations** (on by default) — when you send a message, suggest at most one skill you don't have installed yet.
- **Generate a title after the first answer** (on by default) — title a new conversation from its first successful answer; later answers don't retrigger it.
- **Completion bell** — play a sound and attract the window's attention when the agent finishes.

Reasoning starts collapsed in the conversation and can always be expanded by clicking its row. There is no separate setting to enable access.

See [[Approvals and sandboxing|Sandbox]] for defaults, Linux setup, diagnostics and limitations, and [[Using FutureOS|Using-FutureOS]] for approval cards.

---

## Providers

A provider is where your models come from.

### Future (built-in)

**Future** is the built-in provider (your FutureOS account). To use it:

1. Click **Sign in** — this opens the welcome/sign-in screen.
2. Authorize in the browser page that opens. Current builds open it automatically and no longer show a separate verification code.
3. Once connected, you can **Sign out** at any time.

Other built-in providers (such as DeepSeek, OpenAI, Anthropic, Google, and more) are listed too — click **Configure** to set or update the API key for any of them. Use **More providers** to reveal the full list.

### Custom providers

You can add your own provider. Click **+ Add custom provider** and fill in:

- **Name** — a display name (optional).
- **Provider ID** — a unique id (lowercase letters, digits, `-`, `_`).
- **API type** — OpenAI Completions, OpenAI Responses, or Anthropic.
- **Base URL** — the provider's API address (`http`/`https`).
- **API Key**.
- **Models** — one or more model IDs, each with a display name, a required context window and maximum output tokens, and optional thinking support, modality, and per-million-token prices.

The app validates the fields and checks that the provider ID is unique. You can **Edit** or **Remove** a custom provider later.

> A provider's API key is stored separately from any other credentials.

---

## Models

The Models page lists all available models, **grouped by provider**:

- **Search** to filter by model or provider name.
- **Toggle each model's visibility** — hidden models are removed from the model selector, so your list stays focused on the ones you actually use.

The model selector in the input box draws from the same list and shows which provider each model comes from.

---

## Check for updates

Check whether a newer version of FutureOS is available and download the installer for your system. See [[Install FutureOS|Installation]] for how to apply an update.

---

## Account

Shows your signed-in FutureOS account (profile and credit balance) and lets you sign out.

---

## About

Shows the app version, platform, and related release information.

---

## Reset

**Clear local data** wipes FutureOS's local data and restarts the app: chats, background programs and review records are removed, while sign-in and provider settings are kept. On Windows, this page also resets the folder permissions FutureOS added for write protection (no files are deleted).

---

## See also

- [[Quick Start|Quick-Start]] — connect your FutureOS account and send your first message.
- [[Using FutureOS|Using-FutureOS]] — the approval mechanism in detail.
- [[Skills]] — capability packs the agent can use.
