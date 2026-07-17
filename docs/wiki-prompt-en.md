# Wiki Generation Prompt

> This is a **generation prompt** for AI use, not user-facing documentation.
> Hand the entire contents of this file to an AI, and it should be able to (re)generate the full set of wiki pages under `docs/wiki/`.
> This file is only responsible for **content generation**; how to publish to GitHub Wiki is out of scope.

---

## 1. Role & Goal

You are a technical writer for FutureOS. Your goal: write a **user-facing** wiki for the **FutureOS desktop app** to help users go from download, installation, and login all the way through daily use.

FutureOS is a **desktop AI Agent workbench**: users don't just "chat for answers" — they **see and verify** the agent's work: what it read, which commands it ran, which files it changed, what it's waiting for approval, and how to continue from there. Built for people pushing real multi-step tasks forward: software development, research, data analysis, writing, report generation, debugging — all in one place.

## 2. Audience & Tone

- **The reader is an ordinary user**, not a developer. Assume they don't know the command line, gRPC, architecture, or other internal details.
- Tone: **clear, concise, friendly, action-oriented**. Use "you" frequently. Avoid passive voice and jargon.
- Write about **how to use**, not **how it's implemented**. Don't expose internal module names, port numbers (unless a specific command genuinely needs one), or code architecture.
- Reinforce FutureOS's core value proposition throughout: **You stay in control** — the agent pauses for approval before risky actions; all work is visible and reviewable.

## 3. Before Writing: Read the Code, Only Document Implemented Features

**Before writing each page, read the relevant code and docs for that page. Ground everything in actual code behavior** — don't describe features from memory or outdated docs. Section 7 lists 3–5 **code entry points** per page. Use them as starting points and explore outward (follow imports, component references, i18n string keys) to confirm:

- Whether the feature actually exists and is visible in the UI;
- Specific button names, menu names, page names, and workflow steps;
- Whether platform differences (macOS / Windows) match the code and packaging config.

**Only write about features that are implemented and actually available to users in the UI.** Any "planned / in-development / hidden" features must not appear. Known **not-yet-launched features to skip**:

- **Research** entry — hidden from navigation.
- **Data** entry — hidden from navigation.
- **Remote / mobile remote** — still under development.

> Judge by the code: e.g., if `gui/src/components/layout/ActivityRail.tsx` has an empty `featureItems` array, those sidebar entries are not shown to users. When writing, if you discover a feature is hidden or unwired in code, don't include it in the wiki.

## 4. Output Requirements: Bilingual (Chinese + English)

Every page must have both a **Chinese** and an **English** version with consistent content. Organized by **language subdirectory**:

```
docs/wiki/
  en/            # All English pages
    Home.md
    Installation.md
    Quick-Start.md
    ...
    _Sidebar.md
    _Footer.md
  zh/            # All Chinese pages (filenames match en/ exactly)
    Home.md
    Installation.md
    Quick-Start.md
    ...
    _Sidebar.md
    _Footer.md
```

- **Identical filenames** in both directories, only the content language differs (`en/Home.md` ↔ `zh/Home.md`).
- **No cross-language links** between Chinese and English docs. No language switcher links. The two sets of pages are independent.
- `_Sidebar.md` and `_Footer.md` exist in each directory.
- **All cross-page links stay within the same language directory** (English pages link to English pages, Chinese to Chinese), using relative paths within that directory.

## 5. Platform Scope

**Only support macOS and Windows. Do not write anything about Linux** (no `.deb` / `.tar.gz`, no apt, no Linux first-launch steps, etc.). Whenever stating supported platforms, always write **macOS and Windows**.

## 6. Page Inventory

Generate the following pages (**do not generate TUI / terminal UI pages**). The table lists filenames — produce one in `en/` and one in `zh/` for each.

| Page File | Page Title | Purpose |
|---|---|---|
| `Home.md` | FutureOS | Landing page: one sentence explaining what it is and what you can do, navigation to other pages |
| `Installation.md` | Install FutureOS | Download, first launch (macOS/Windows), data location, updates, uninstall |
| `Quick-Start.md` | Quick Start | From fresh install to first answer: login → start conversation → send message → choose model → see the work |
| `Using-FutureOS.md` | Using FutureOS | App tour: three-panel layout, Chat vs Workspace, how to follow up and steer the agent, approval mechanism, right-side panel |
| `Settings.md` | Settings | Settings pages (focus on General / Providers / Models); built-in FutureGene login, custom provider, model visibility |
| `Skills.md` | Skills | Overview of built-in skill packs and how to use them |
| `CLI.md` | CLI (`future-cli`) | Optional advanced command-line tool: location, running, command groups |
| `FAQ.md` | FAQ & Troubleshooting | Quick answers to common problems |
| `_Sidebar.md` | — | Left sidebar navigation |
| `_Footer.md` | — | Footer (download, report issue links) |

> This is the **minimum page set**. If you discover small features in the code that are already released to users but not listed here, fold them into the most relevant existing page (generally don't create new pages), and note the addition in the deviation report (Section 9).

### Sidebar Structure (TUI removed)

```
### FutureOS
- Home

**Getting started**
- Install → Installation
- Quick Start → Quick-Start

**Using the app**
- Using FutureOS → Using-FutureOS
- Settings → Settings
- Skills → Skills

**Command line (advanced)**
- CLI (future-cli) → CLI

**Help**
- FAQ
```

## 7. Content Guidelines Per Page

> **Every bullet below is "reference content — defer to the code."** They provide scope and approximate facts, but button names, page names, commands, options, skill lists, and other **specific details must come from the code you read**. When there's a conflict, follow the code and record the discrepancy in the deviation report (Section 9). The reference content in this file may lag behind the code.
> Write in a user-facing, actionable style. The **"code entry points (read first)"** listed for each page are exploration starting points for you only — **do not** include them in the actual wiki pages.

### Home
**Code entry points (read first):** `gui/src/app/App.tsx`, `gui/src/components/layout/AppShell.tsx`, `gui/src/components/layout/ActivityRail.tsx`, `gui/src/i18n/locales/` (feature naming & copy), `CLAUDE.md` (overall positioning).
- One-line definition: **desktop AI Agent workbench** — not just chat, but seeing and verifying the agent's work.
- "Getting started" in three steps: Install → Quick Start → Using FutureOS.
- "What you can do" highlights: agent conversations with streaming thinking, tool calls, and visible work; quick Chat or folder-bound Workspace; you stay in control (approval before risky actions); reviewable work (background runs, file changes, outputs); use skill packs.
- Note at bottom: runs on **macOS and Windows**.

### Installation
**Code entry points (read first):** `gui/src-tauri/tauri.conf.json` (packaging artifacts: dmg / nsis / zip — confirm actual artifact types), `gui/src-tauri/build.rs` (sidecar binaries bundled), `scripts/build-windows-portable.ps1` (Windows portable package contents), `Makefile` (`package-gui` and other packaging targets), `CLAUDE.md` (`~/.future` data/config location).
- **Download**: go to the Releases page and get the latest version for your system.
  - macOS: `.dmg` disk image
  - Windows: installer (`.exe`), or portable `.zip`
- Mention that the CLI tool `future-cli` **ships with every download** (both installer and portable, placed next to the app) — see the CLI page for details.
- **First launch** (current builds are unsigned/notarized, system warnings are expected):
  - **macOS**: drag FutureOS into Applications; first time right-click → "Open" → click "Open" again; if "damaged", run `xattr -dr com.apple.quarantine /Applications/FutureOS.app` in Terminal then open.
  - **Windows**: installer version — run `.exe`; portable version — extract the entire folder then double-click `FutureOS.exe` (keep `FutureOS.exe` and `future-agent.exe` in the same folder). First SmartScreen prompt: "More info → Run anyway". Requires **Microsoft Edge WebView2 Runtime** (pre-installed on recent Win10 and Win11; if missing, install the Evergreen version from Microsoft's website).
- **Sign in**: first use requires internet and in-app sign-in — see Quick Start.
- **Data location**: `.future` folder in your home directory (macOS `~/.future`, Windows `C:\Users\<you>\.future`).
- **Updating**: download the latest and install over the old one (replace folder for portable). `.future` data is preserved.
- **Uninstalling**: macOS — delete `FutureOS.app`; Windows — uninstall from Settings or delete portable folder. To also remove data, delete `.future` afterward.

### Quick-Start
**Code entry points (read first):** `gui/src/features/settings/FutureLoginDialog.tsx` (device-code login flow), `gui/src/features/settings/ProvidersPage.tsx`, `gui/src/features/agent/NewConversation.tsx`, `gui/src/features/agent/Composer.tsx` (send, model selector, attachments), `gui/src/components/layout/ActivityRail.tsx` (New Chat / Workspace entries).
- **Open and sign in**: Settings (gear icon bottom-left) → Providers → built-in FutureGene → Connect → authorize in browser (if it doesn't open automatically, use the verification code + copyable link shown in the app). Mention you can also bring your own provider (see Settings).
- **Start a conversation**: two ways — **New Chat** (fastest, for questions and one-off tasks), **Workspace** (bind a folder on your computer, for real projects).
- **Send your first message**: use the input box at the bottom; you'll see streaming replies, tool call displays, and **pauses for your approval** before risky actions; local files are supported, with up to 4 images per turn (25 MiB each) and no count limit for other files.
- **Choose a model (optional)**: the model selector is right in the input box; you can also manage models in Settings → Models.
- **See the work**: right-side panel — Runs (background tasks), Review (file changes in Workspace), Artifacts (outputs from Chat).

### Using-FutureOS
**Code entry points (read first):** `gui/src/components/layout/AppShell.tsx` (three-panel layout), `gui/src/components/layout/ActivityRail.tsx` (left navigation — use this to confirm exactly which entries exist), `gui/src/components/layout/ContextPanel.tsx` (right panel), `gui/src/features/agent/ApprovalPrompt.tsx` (approval mechanism), `gui/src/features/runs/RunsPanel.tsx` + `gui/src/features/review/ReviewPanel.tsx` + `gui/src/features/artifacts/ArtifactsPanel.tsx` (three right-side views).
- **Three-panel layout**: left = navigation (use what `ActivityRail.tsx` actually renders: New Chat, your Workspaces & conversations, Chats, Settings); center = conversation (messages, streaming replies, plans, tool activity, command previews, errors, approval cards, input box fixed at bottom); right = context (see what the agent is doing, collapsible).
- **Chat vs Workspace**: compare in a table (how created, suitable scenarios, what the right panel shows). Emphasize each conversation is an independent agent session that doesn't interfere with others.
- **Talking to the agent**: send via input box; model selector can be changed per-session; up to 4 images per turn, while non-image attachments are not count-limited.
- **Approval mechanism — you're in control**: risky actions pause with an approval card above the input box and wait (no timeout); Allow to continue, Reject to cancel and inform the agent so it can adjust.
- **Right panel for reviewing work**: Runs (running/completed, can stop/clear, each card shows real commands), Review (Workspace file changes: file list, stats, diff; under version control there's also a "previous changes" view), Artifacts (Chat outputs).
- (Don't write about Research / Data entries — they are hidden from navigation.)

### Settings
**Code entry points (read first):** `gui/src/features/settings/SettingsDialog.tsx` (page composition), `gui/src/features/settings/GeneralPage.tsx`, `gui/src/features/settings/ProvidersPage.tsx` + `CustomProviderDialog.tsx`, `gui/src/features/settings/ModelsPage.tsx`, `gui/src/features/settings/FutureLoginDialog.tsx`. **Use these to confirm what settings pages actually exist and their real fields.**
- Access via gear icon bottom-left; there's also a Models shortcut under New Chat. **Page count and names must match `SettingsDialog.tsx`** (typically General / Providers / Models plus user-visible pages like "Check for updates" and "Reset"; dev-only pages are skipped). Focus on the three below.
- **General**: desktop-level options. Use real labels from code; typically includes: **Language**, **Approval mode** (Manual / Sandbox [macOS only] / Unrestricted), **Show thinking process**.
- **Providers**:
  - **FutureGene (built-in)**: Connect login flow (browser auth / verification code + link); after connecting you can sign in again or sign out.
  - **Custom provider**: add an OpenAI-compatible or Anthropic-compatible provider; fill in id, name, API type, Base URL, API key, model list; the app validates and checks id uniqueness; can edit/delete.
- **Models**: all available models listed grouped by provider; toggle each model's visibility; searchable; the input box selector draws from the same source and shows which provider each model comes from.

### Skills
**Code entry points (read first):** `gui/src/features/skills/SkillsView.tsx`, `gui/src/integrations/skills/skillsClient.ts`, `cli/src/commands/skills.ts`, `agent/src/skills/mod.rs` (skill discovery). **Use these first to confirm the actual list of currently existing skills and their purposes**, then fill in the table below based on reality.
- Definition: built-in capability packs the agent **automatically uses** when relevant; also a **browseable/installable/uninstallable directory** (Installed / All tabs, catalog comes from the online directory). **The Skills sidebar entry is visible** (unlike Research/Data which are hidden) — confirm via `ActivityRail.tsx`.
- Common built-in skills table (skill name + purpose, **reference only — use the actual All tab list from the app**): Account (profile/quota/recharge), Web (search the web and read full articles), Paper (search PubMed/ArXiv/DOI and fetch full text), Deep research (multi-source cross-checking, cited reports), Document (PDF/Word to structured text), Image (generate/edit/analyze images, including OCR), Browser (drive a browser: open pages, click, type, screenshot), Hand-drawn posters (hand-drawn vertical infographic posters), Hand-drawn slides (hand-drawn sketch slides composited into PDF), Subagent (run multiple tasks in parallel), Skill creator (help make new skills).
- **How to use**: no manual activation needed — just describe your needs; you can also browse and install/uninstall on the Skills page. (Don't write about Research / Data entries — they are hidden.)

### CLI
**Code entry points (read first):** `cli/src/index.ts` (subcommand dispatch — use this to confirm the actual command groups), `cli/src/commands/run.ts`, `cli/src/commands/auth.ts` + `agent.ts`, `cli/src/commands/tools.ts` + `skills.ts`, `cli/src/help.ts`. **Commands, subcommands, and options must match the code exactly.**
- Positioning: optional CLI tool `future-cli`, shipped with every download; the desktop app already covers most needs — use the CLI when you want scripting / automation / pure-terminal workflows. Start with a note that terminal-shy users can skip this page.
  - > ⚠️ The release binary is named **`future-cli`** (see `tauri.conf.json` sidecar, `docs/dist/readme-*.txt`, in-app copy). The dev-time `future` installed via `npm link` is only a **development alias** — user-facing wiki must always use `future-cli`, never `future`.
- **Location**:
  - macOS (`.dmg`): inside the app at `/Applications/FutureOS.app/Contents/MacOS/future-cli`
  - Windows (**portable** `.zip`): `future-cli.exe` inside the extracted folder
  - Note: on Windows, the CLI is only in the **portable** package; the standard installer includes only the app and agent.
- **Running**: open a terminal in the folder containing the binary; use `--help` to explore; can add to PATH or create an alias (give macOS alias example).
- **Agent must be running**: every command needs the FutureOS agent; if the desktop app is open it's already running, otherwise `future-cli agent start`.
- **Command groups** (use `cli/src/index.ts` actual dispatch; remove `tui` group):
  - `auth`: sign in / sign out / status (`login` / `status` / `logout`)
  - `agent`: start/stop background agent (`start` / `stop` / `restart` / `status`)
  - `run`: send a one-shot prompt and print the answer (give examples: direct question, `--model`, `@file`, pipe input; explain `@<path>` includes files, common options `--model` (supports `model:thinking`), `--thinking`, `--continue`/`-c`, `--cwd`, `--mode json`, `--no-session`)
  - `tools`: list and call tools (`tools list`, `tools call <name> --args '<json>'`, `--output`, `--stdin`; file path arguments auto-convert)
  - `skills`: manage skill packs (`list` / `install` / `uninstall`; **no `update`** — subcommands must match code)
  - `channel`: chat channel bridge (advanced, one-line mention)
- **Tips**: macOS first-time blocked → right-click open the app first to clear the block; "Connection refused" → agent not running, use `future-cli agent start` or open the desktop app.

### FAQ
**Code entry points (read first):** `gui/src-tauri/tauri.conf.json` (install/signing related), `gui/src/features/settings/FutureLoginDialog.tsx` + `ProvidersPage.tsx` (login issues), `gui/src/features/agent/ApprovalPrompt.tsx` (approval), `cli/src/commands/agent.ts` ("connection refused" / agent not running), `CLAUDE.md` (data location).
Cover these questions (remove all Linux and TUI items):
- macOS won't open ("unidentified developer" / "damaged"): right-click open; for "damaged" use `xattr -dr com.apple.quarantine /Applications/FutureOS.app`.
- Windows says "Windows protected your PC": SmartScreen, click "More info → Run anyway".
- Windows: nothing happens when launching: install Microsoft Edge WebView2 Runtime; for portable version, confirm `FutureOS.exe` and `future-agent.exe` are in the same folder.
- Can't use any model / not signed in: Settings → Providers → FutureGene → Connect, or add your own provider.
- How to switch models: input box selector, or Settings → Models.
- The agent stops and asks me something: that's the approval mechanism, Allow/Reject, no timeout.
- Where conversations and settings are stored: `.future` in home directory (macOS `~/.future`, Windows `C:\Users\<you>\.future`).
- How to update: download latest and install over the old one, data preserved.
- How to uninstall / clear data: delete the app; to also remove data, delete `.future`.
- Which platforms are supported: **macOS and Windows**.

## 8. Formatting & Cross-linking Conventions

- Page name comes from the filename: `Quick-Start.md` → page **Quick-Start**.
- Cross-page links use **relative paths within the same language directory**, e.g., English version: `[Quick Start](Quick-Start)`, Chinese version: `[快速开始](Quick-Start)` — both point to the same-named file in the same directory.
- **No cross-directory links, no language switcher links** (see Section 4).
- External links: Releases page `https://github.com/futuregene/future-os/releases`; report issues `https://github.com/futuregene/future-os/issues`.
- Include appropriate "See also" cross-links at the bottom of each page.
- Keep Markdown tables, code blocks, and blockquotes clean and readable.

## 9. Post-Generation Self-Check & Deviation Report

After writing all pages, you must do the following two steps.

**A. Self-check (fix until passing):**

1. **Link integrity**: every `[[Display Text|Slug]]` Slug resolves to a **real `.md` file in the same language directory**; no cross-language links, no language switcher links.
2. **Leak scan**: full-text search to confirm **none** of the following appear — Linux / `.deb` / `.tar.gz` / apt, TUI / terminal UI pages or links to them, gRPC / port numbers (e.g. 50051), Research entry / Data entry / Remote mobile remote. (Note: the skill name **Deep research** and descriptive words like "research" / "data analysis" in homepage use cases are normal content and don't count as leaks.)
3. **Chinese-English alignment**: `en/` and `zh/` filenames match 1:1 with the same count; section structure and coverage points are consistent across language versions (only language differs).
4. **CLI name**: use `future-cli` throughout, never bare `future` (check commands, paths, and examples).

**B. Deviation report**: After generation, output a separate "code vs this prompt's reference content" discrepancy list — for every place where you followed the code and it differs from the reference content in Section 7 of this file (e.g. skill list, settings page count & names, CLI commands/subcommands, button names, etc.), list each item. The purpose is to feed these corrections **back into this prompt** to close the loop.

## 10. Prohibited Items

- ❌ Do not generate TUI / terminal UI pages, and don't link to or mention them from other pages (remove `tui` from CLI command groups).
- ❌ Do not write about unlaunched / hidden / in-development features: **Research entry, Data entry, Remote (mobile remote)**, or anything hidden or unwired in code.
- ❌ Do not write anything about Linux.
- ❌ Do not write about wiki publishing / sync / CI / GitHub Actions or other maintenance workflows — this prompt is only for **content**.
- ❌ Do not expose internal implementation details (architecture, module names, gRPC, etc.) unless a specific CLI command genuinely requires it.
