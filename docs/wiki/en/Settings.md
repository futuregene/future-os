# Settings

Open Settings from the **gear icon** at the bottom-left of the window. There's also a **Models** shortcut in the left panel that jumps straight to the Models page.

The settings you'll use day-to-day are **General**, **Providers**, and **Models**. There are also **Check for updates** and **Reset** pages.

---

## General

Desktop-level options for the app:

- **Language** — choose the app's display language.
- **Approval mode** — how much the agent asks before acting:
  - **Manual** — prompts before file reads and writes; read-only commands run automatically.
  - **Sandboxed** (macOS only) — commands run in the macOS sandbox; file operations still prompt.
  - **Unrestricted** — no prompts and no sandbox; everything runs.
- **Show thinking process** — show or hide the model's reasoning in the conversation.

See [[Using FutureOS|Using-FutureOS]] for how approval works in practice.

---

## Providers

A provider is where your models come from.

### FutureGene (built-in)

FutureGene is the built-in provider. To use it:

1. Click **Connect**.
2. Authorize in your browser. If it doesn't open automatically, use the **verification code** and **copyable link** shown in the app.
3. Once connected, you can **Sign in again** or **Sign out** at any time.

Other built-in providers (such as DeepSeek, OpenAI, Anthropic, Google, and more) are listed too — click **Set key** / **Update key** to add your own API key for any of them. Use **More providers** to reveal the full list.

### Custom providers

You can add your own provider. Click **+ Add custom provider** and fill in:

- **Name** — a display name (optional).
- **Provider ID** — a unique id (lowercase letters, digits, `-`, `_`).
- **API type** — OpenAI Completions, OpenAI Responses, or Anthropic.
- **Base URL** — the provider's API address (`http`/`https`).
- **API Key**.
- **Models** — one or more model IDs (with optional display names).

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

## Reset

**Clear local data** wipes FutureOS's local data and restarts the app. Use this only if you want a clean slate — your conversations and local settings are removed.

---

## See also

- [[Quick Start|Quick-Start]] — connect FutureGene and send your first message.
- [[Using FutureOS|Using-FutureOS]] — the approval mechanism in detail.
- [[Skills]] — capability packs the agent can use.
