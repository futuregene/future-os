# Document fact inventory (historical verification basis)

> This is a faithful paragraph-by-paragraph English translation of the historical snapshot [文档事实清单（历史核验依据）](./fact-inventory.zh-CN.md)（2026-08-06，commit `71f22b0b`）. Conclusions, dates and commit boundaries are preserved verbatim; this translation is not a new verification result.

> **Historical snapshot, not a current fact inventory.** The file list, line numbers, platform scope, defaults and PASS below apply only to
> their original date/commit; "no change needed / skippable" does not apply to later versions. The current 2026-09-07 documents already
> correct approval defaults, Linux sandbox and release platforms, IPC, RPC dual-write and skills/CLI drift. The original record below is preserved for traceability;
> the current entry point is the [document index](../../README.md), and new modifications must be re-checked against source.

> **Internal working document** (not user documentation). This file is an intermediate artifact of the document-verification task: it records, item by item,
> the **key claims and their locations** (file:line / section) in README + all documents under `docs/`,
> as the item-by-item basis for the subsequent "verify against source" step.
> This file only records "what the document says + where it says it", and **does not judge right or wrong**; that judgement is left to the verification step.
> Line numbers are as of the 2026-08-06 worktree.

> **2026-08-26 supplement (worktree `claude/docs-optimization`)**: the correction list from the full
> documentation re-check is in `errors-outdated-missing.md` §I（the old fact snapshots in this file's §1-§14 were not re-recorded item by
> item — old line numbers have drifted and serve only as historical reference）. This round's scope expanded to CLAUDE.md、desktop/CLAUDE.md、
> module READMEs（mobile/packages/rpc/loop/tui），and corrected sandbox three-tier names、the loop CLI command surface（7 groups 40 commands）、
> debug.log gating、crate names etc（see §I for details）.

- Generated: 2026-08-06
- Scope: `README.md`、`README.zh-CN.md`、**all 45 files** under `docs/`（39 .md + 6 .txt；including 6 at the docs root、13 wiki en + 13 wiki zh、5 architecture-audit、6 dist）
- Out of scope: `CLAUDE.md`（agent instructions）、`desktop/CLAUDE.md`、`mobile/README.md`、`skills/README*.md`（component-level READMEs, not README+docs）

---

## 0. Master document list

| # | File | Lines | Type | Language |
|---|---|---|---|---|
| 1 | README.md | 143 | user document (root) | en |
| 2 | README.zh-CN.md | 138 | user document (root) | zh |
| 3 | docs/build-and-install.md | 206 | user document (build/install) | en |
| 4 | docs/build-and-install.zh-CN.md | 190 | user document (build/install) | zh |
| 5 | docs/loop-control-plane.md | 179 | user document (feature guide) | en |
| 6 | docs/loop-control-plane.zh-CN.md | 128 | user document (feature guide) | zh |
| 7 | docs/wiki-prompt.md | 229 | **generation prompt** (for AI, not user docs) | zh |
| 8 | docs/wiki-prompt-en.md | 229 | **generation prompt** (for AI, not user docs) | en |
| 9-21 | docs/wiki/en/*.md（13 files） | see §7 | user documents (wiki) | en |
| 22-34 | docs/wiki/zh/*.md（13 files） | see §8 | user documents (wiki) | zh |
| 35 | docs/architecture-audit/README.md | 18 | internal audit report | zh |
| 36 | docs/architecture-audit/01-agent-guirust-boundary.md | 187 | internal audit report | zh |
| 37 | docs/architecture-audit/02-guirust-guireact-boundary.md | 154 | internal audit report | zh |
| 38 | docs/architecture-audit/03-large-modules-split.md | 278 | internal audit report | zh |
| 39 | docs/architecture-audit/04-react-rendering-performance.md | 149 | internal audit report | zh |
| 40-45 | docs/dist/readme-{macos,windows,linux}[-en].txt（6 files） | 20-34 | release-package readmes | zh/en |

**wiki page inventory**（en/zh filenames one-to-one, 13 pairs）: Home、Installation、Quick-Start、
Using-FutureOS、Settings、Skills、CLI、FAQ、Feishu、DingTalk、Models、_Sidebar、_Footer。
> Note: the wiki-prompt §6 page inventory does **not** have Feishu/DingTalk, but the actual wiki has these two pages
> and the _Sidebar has an「Integrations」group — a deviation between the prompt and the actual pages（see §12-X6）.

---

## 1. README.md（en）

| # | Claim | Location |
|---|---|---|
| R1 | positioning: local-first AI agent workspace, TUI/GUI/CLI/Feishu/DingTalk multi-end, macOS/Linux/Windows | L6-9 (intro under title) |
| R2 | **「1000+ built-in models across 100+ providers」** | L26 (feature table Model Flexibility) |
| R3 | three model-configuration paths: A) `future auth login` device-code login auto-configures; B) `~/.future/agent/auth.json` indexed by provider name (`{"openai":{"type":"api_key","key":"sk-..."}}`), Azure-like with `baseUrl`; C) `~/.future/agent/models.json` custom providers (`providers[].apiKey/baseUrl/models[].{id,name,contextWindow}`) | L46-79 |
| R4 | **agent must be running first, listening on `127.0.0.1:50051`**; `future-agent` starts it, `future-tui` starts the TUI | L85-94 |
| R5 | connection/gRPC errors ⇒ almost always the agent is not running | L96 |
| R6 | 12 TUI slash commands: `/help` `/model` `/new` `/sessions` `/compact` `/scoped-models` `/clone` `/fork` `/tree` `/name [n]` `/status` `/stop` | L105-118 (table header L105) |
| R7 | TUI shortcuts: `ctrl+p` cycle models、`ctrl+t` cycle thinking levels、`ctrl+r` browse sessions、`ctrl+c` interrupt/quit、`tab` complete、`enter` submit、`escape` close overlay、`↑↓` scroll | L122-131 (table header L122) |
| R8 | troubleshooting: connection errors → `lsof -i :50051` port check；auth/"no model" → `future auth login` or add a provider to models.json | L133-140 |
| R9 | remaining feature-table claims: streaming + chain of thought (off ↔ xhigh)；tools read/write/edit/shell + approval + **sandbox tiers (off / manual / macOS Seatbelt)**；JSONL sessions + fork/clone/tree；YAML skills multi-directory discovery；auto compaction + exponential-backoff retry；loop control plane (loopx Rust rewrite)；Rust core | L22-33 |
| R10 | License: MIT | L143 |
| R11 | external links: wiki (github.com/futuregene/future-os/wiki)、future-skills repo | L3, L5 |

---

## 2. README.zh-CN.md（zh）

Mirrors en; key differences:

| # | Claim | Location |
|---|---|---|
| RZ1 | 「内置 1000+ 模型，覆盖 100+ Provider」 | L23 |
| RZ2 | three model-configuration paths（same as R3，links point to docs/wiki/zh/ and docs/loop-control-plane.zh-CN.md、docs/build-and-install.zh-CN.md） | L43-77 |
| RZ3 | agent listens on `127.0.0.1:50051`；`future-agent`/`future-tui` | L80-89 |
| RZ4 | slash-command table（same as R6，12 commands） | L100-113（table header L100） |
| RZ5 | shortcut table（same as R7） | L117-126（table header L117） |
| RZ6 | troubleshooting table（same as R8） | L128-133 |
| RZ7 | feature table（same as R9；sandbox「关闭 / 手动 / macOS Seatbelt」） | L19-30 |
| RZ8 | MIT | L138 |

---

## 3. docs/build-and-install.md（en）

| # | Claim | Location |
|---|---|---|
| B1 | prerequisites: **Rust 1.97+**（pinned by `rust-toolchain.toml`）、**Node.js 24+**（`.nvmrc`）、**Bun required**（TUI build and CLI/GUI packaging use `bun build`）、optional Python 3（only `make generate-models`）、optional protoc（only `make generate-proto`，generated code is committed） | L12-18 |
| B2 | clone: `git clone https://github.com/futuregene/future-os.git` | L22-25 |
| B3 | macOS deps: `xcode-select --install`、rustup、`brew install node oven-sh/bun/bun`、optional `brew install protobuf` | L30-37 |
| B4 | macOS build: `make install` → **/opt/homebrew/bin**；`make install-desktop`；`make package-desktop` → **.app + .dmg**（desktop/src-tauri/target/release/bundle/）；`scripts/build-desktop-macos.sh`（auto-signs with the unique Developer ID Application → `*-sign.dmg`，`--help` for certificate selection/output dir/**Apple notarization** options） | L40-51 |
| B5 | Linux deps: `build-essential mold libssl-dev libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf` + rustup + bun + nvm(Node 24) + optional protobuf-compiler | L58-64 |
| B6 | **mold required on x86_64**（`.cargo/config.toml` passes `-fuse-ld=mold`）；not needed on ARM Linux | L67 |
| B7 | Linux build: `make install` → /usr/local/bin (sudo)；`make package-desktop` → **.deb** | L71-75 |
| B8 | Windows toolchain: VS Build Tools（Desktop C++ workload）、`winget install Rustlang.Rustup`（host triple x86_64-pc-windows-msvc）、Node 24+、Bun、WebView2（bundled with Win10/11） | L79-86 |
| B9 | Windows terminal stack（equivalent of install-cli, no desktop）: cargo build agent/channels → `npm install; npm run gen-version; npm run build; bun build --compile ... --outfile dist/future-tui.exe`（tui）and `...--outfile dist/future.exe --external chromium-bidi`（cli）→ copy to **%USERPROFILE%\.future\bin**（future-agent.exe、future-channel.exe、future-tui.exe、future.exe）→ `& "$bin\future.exe" skills install`（no symlinks on Windows） | L88-108 |
| B10 | Windows desktop app: sidecar named by host triple（future-agent-$triple.exe / future-$triple.exe）→ `npx tauri build --no-bundle` → copy futureos.exe → **future-desktop.exe** | L110-122 |
| B11 | Windows installer: `node scripts\version.mjs --set-bundle` + `npm run tauri:build` → **NSIS .exe**（bundle\nsis\） | L124-128 |
| B12 | scripts: `scripts/start-desktop-windows.bat` dev mode；`build-desktop-macos.sh`/`build-desktop-windows-portable.ps1`/`build-desktop-windows-installer.ps1` mirror the CI pipelines（DMG/portable zip/NSIS），need protoc；artifacts include GUI+agent+CLI，**no TUI** | L130-133 |
| B13 | loop control plane: `orchestration/loop`，`cargo build -p future-loop`（debug→target/debug，release→target/release）；`bash scripts/install-future-loop.sh` → **~/.local/bin/future-loop** + **~/.future/agent/skills/**；verify with `future-loop status` | L135-155 |
| B14 | skills installation: `make install-skills`（symlinks the builtin skills/ submodule）；`future skills install`（**about 13**）；`future init`（installs skills + links local commands on macOS/Linux）；symlinks into `~/.future/agent/skills/`；`future skills list`/`future skills update` | L162-178（`future skills install` L171，`future skills update` L177） |
| B15 | verification: `make test`（cargo test agent + loop）、`make lint`（agent+channels+TUI+CLI+GUI） | L168-171 |
| B16 | development: `make build/lint/fmt/test/clean` | L174-183 |
| B17 | proto: canonical API is `proto/future.proto`；generated code committed；`make generate-proto`（agent + channels + TUI） | L186-206 |

---

## 4. docs/build-and-install.zh-CN.md（zh）

Mirrors en（B1-B17）. Key line numbers: prerequisites L10-16、macOS L28-47、Linux L54-70、
Windows L73-127、loop L129-146、skills L148-166（`future skills update` upgrade → L163）、
proto L180-190. **No content differences**（except links pointing to .zh-CN versions）.

---

## 5. docs/loop-control-plane.md（en）

| # | Claim | Location |
|---|---|---|
| L1 | positioning: local control plane at `orchestration/loop`，`future-loop` CLI + `/future-loop` agent skill | L5-7 |
| L2 | **`future-loop` is the Rust rewrite of loopx (github.com/huangruiteng/loopx)**，adapted for FutureOS（project-local state、gRPC execution bridge、quota kernel、extensions and multi-agent） | L8-11 |
| L3 | goals: `goal init / cancel / delete`，state in `<cwd>/.future/loop/`，event ledger + replay | L40-41 |
| L4 | todos: `todo add / claim / complete / supersede / update / archive`；advancement/user-gate/monitor/blocker categories；`--blocks` dependency chains；claim+lease；completion contract（every completed todo declares a successor or explicit no-follow-up） | L42-45 |
| L5 | human gates: `gate resolve` | L47 |
| L6 | monitors: `--class monitor --cadence ...`，backoff on no change | L49 |
| L7 | decision kernel: `future-loop run`，pure function with injectable clock，**nine dispositions**（terminal/monitor-wait/active work/consistency repair/human gate/quiet wait/…），fail-closed | L54-60 |
| L8 | quota: run/agent/heartbeat three slot-accounting sources、24h/7d summaries、stall repair | L62-65 |
| L9 | scheduling: cadence normalization（`once / hourly / daily / weekly` or `15m / 1h / 2d`）、atomic persistence、host-failure tracking | L67-68 |
| L10 | event sourcing: content-addressed event ids、idempotent append、fail-closed conflict detection；`QuotaSpent`/`EvidenceAttached` events；markdown backfill | L72-76 |
| L11 | migration bridge（verify / migrate / bridge）；privacy-tier projections（public-safe/local-private/private-pointer）；run lifecycle（history/compaction/index/retention/stale detection） | L77-80 |
| L12 | independent verification: `todo add --verify "cargo test" --max-validation-attempts 5`，completes only on exit code 0，replan on budget exhaustion | L84-86 |
| L13 | extensions and multi-agent: capability framework（declared→installed→enabled→ready）、capability gates（run/ask-owner/repair-bridge/skip）、extension manifest + install/enable/disable/rollback + readiness doctor（v1 declarative, executes no code）、identity-scoped multi-agent、supervisor proposals/receipts、task leases、handoff-document delivery contracts、todo dependency graphs、attention queue/operator inbox | L88-108 |
| L14 | diagnostics: benchmark（protocol/run/ledger）、replay（record/run、corpus）、canary（`core-control-plane`/`extension-runtime`/`release-gate`）；`version`/`doctor`/`history`/`turn`/`todo-event`/`evidence-log`、`backup`/restore | L110-113 |
| L15 | **CLI overview**（command surface, the item-by-item verification target）: goal / todo / agent / capability / extension / ops / work-items / handoff / benchmark / replay / canary / cli（subcommands in the original L115-127） | L115-127 |
| L16 | skill-mode quick start: `/future-loop <goal>`；`future-loop run --max-turns 1` | L132-144 |
| L17 | direct examples: `future-loop goal init --objective ... --cwd ...`；`todo add --goal <id> --text ... --priority P0 [--blocks ...] [--verify "test -f report.md"]`；`status --goal <id>`；`run --goal <id> --model future/deepseek-v4-flash --max-turns 1` | L148-155 |
| L18 | state layout: `registry.json`（source of truth）、`goals/<id>/events.jsonl`、`goals/<id>/ACTIVE_GOAL_STATE.md`（reference-compat projection）、`runs/`；runtime state never written outside the project；`.future/loop/` added to .gitignore | L159-167 |
| L19 | installation: `bash scripts/install-future-loop.sh` or `cargo build -p future-loop` | L171-173 |

## 6. docs/loop-control-plane.zh-CN.md（zh）

Mirrors en（L1-L19）; line numbers: L7 positioning、L11-14 loopx rewrite、L40-49 goals/todos/gates/monitors、
L54-60 kernel、L62-80 quota/scheduling/event sourcing、L84-86 independent verification、L88-108 extensions multi-agent、
L110-113 diagnostics、L115-127 CLI overview、L132-144 quick start、L148-155 direct examples、
L159-167 state layout、L171-173 installation. **No content differences**.

---

## 7. docs/wiki-prompt.md（zh，generation prompt）

| # | Claim | Location |
|---|---|---|
| W1 | this file is a **generation prompt** for AI, not user documentation；the whole docs/wiki/ page set can be (re)generated from it | L1-6 |
| W2 | readers are ordinary users；**do not expose gRPC/ports/module names**；repeatedly emphasize「you stay in control」 | L10-14 |
| W3 | only write implemented features；**currently do not write: Research entry, Data entry, Remote/phone remote**（ActivityRail.tsx featureItems empty array hides them） | L18-29 |
| W4 | bilingual dirs en/ and zh/，filenames one-to-one，**no cross-linking**、no language-switch links | L31-48 |
| W5 | **platform scope: only macOS and Windows，no Linux** | L50 |
| W6 | 10-page inventory（Home/Installation/Quick-Start/Using-FutureOS/Settings/Skills/CLI/FAQ/_Sidebar/_Footer）——**no Feishu/DingTalk** | L52-92 |
| W7 | sidebar structure（no Integrations group） | L94-109 |
| W8 | Installation reference: dmg / nsis / zip artifacts；**the `future` CLI ships with every download package**（installed next to the app）；「officially released macOS/Windows installers are signed，macOS also Apple-notarized」；WebView2；.future data location；Settings→Check for updates | L123-134 |
| W9 | Quick-Start reference: FutureGene Connect flow；New Chat / Workspace；**at most 4 images per turn（25 MiB each），non-image files unlimited** | L135-145 |
| W10 | Settings reference: General（Language / Approval mode 手动·沙盒[仅 macOS]·无限制 / Show thinking）；Providers（FutureGene Connect + custom provider: id/name/API type/Base URL/API key/model list，id-uniqueness validation）；Models（grouped by provider、visibility、searchable） | L159-169 |
| W11 | Skills reference: 11 builtin skills table（Account/Web/Paper/Deep research/Document/Image/Browser/Hand-drawn posters/Hand-drawn slides/Subagent/Skill creator） | L150-158（skills table） |
| W12 | CLI reference: **「the command is uniformly named `future`……always use `future` throughout，never write `future-cli`」**（explicit ban）；macOS location `/Applications/FutureOS.app/Contents/MacOS/future`；command groups auth(login/status/logout)、agent(start/stop/restart/status)、run(--model supports model:thinking、--thinking、--continue/-c、--cwd、--mode json、--no-session、@<path>)、tools(list/call --args/--output/--stdin)、skills(list/install/uninstall；**no update**)、channel | L170-186 |
| W13 | FAQ reference: macOS won't open（right-click open / `xattr -dr com.apple.quarantine`）；SmartScreen；WebView2；portable same folder；not signed in；switch model；approval mechanism（no timeout）；.future location；updates；uninstall；**platform = macOS and Windows** | L188-201 |
| W14 | self-checks: link completeness、leak scan（no Linux/.deb/.tar.gz/apt、TUI、gRPC/ports like 50051、Research/Data/Remote）、zh/en alignment、**CLI name `future`** | L209-219 |
| W15 | prohibitions: no TUI page；no hidden features；no Linux；no release/CI maintenance flows；no internal implementation details | L221-229 |

## 7b. docs/wiki-prompt-en.md（en，generation prompt）

Largely mirrors zh, but **has directional conflicts with zh**:

| # | Claim | Location |
|---|---|---|
| WE1 | page inventory lists the CLI.md title as **CLI (`future-cli`)**；sidebar `CLI (future-cli) → CLI` | L81, L104 |
| WE2 | 「The CLI tool **`future-cli`** ships with every download」 | L127 |
| WE3 | **「The release binary is named `future-cli`… dev-time `future` installed via npm link is only a development alias — user-facing wiki must always use `future-cli`, never `future`」**（exactly opposite of zh W12） | L170-171 |
| WE4 | macOS location `.../MacOS/future-cli`；Windows `future-cli.exe` | L173-174 |
| WE5 | self-check item 4:「use `future-cli` throughout, never bare `future`」 | L219 |
| WE6 | the rest（page inventory、skills table、4 images/25MiB、command groups、FAQ、platform macOS+Windows、no Feishu/DingTalk pages）matches zh | L76-229 |

---

## 8. docs/wiki/en/*.md（13 pages，key claims）

### Home.md（40 lines）
| # | Claim | Location |
|---|---|---|
| EH1 | positioning: desktop AI agent workbench，watch and verify agent work | L1-4 |
| EH2 | three steps to start: Installation / Quick-Start / Using-FutureOS（wiki internal links `[[...]]`） | L14-19 |
| EH3 | what you can do: streaming thinking/tool calls；Chat or a bound-folder Workspace；pause for approval before risky operations；right side Files/Runs/Review（✅ 2026-08-06 changed from Artifacts）；Skills used automatically | L23-29 |
| EH4 | footer: **runs on macOS and Windows** | L40 |

### Installation.md（77 lines）
| # | Claim | Location |
|---|---|---|
| EI1 | platforms: macOS and Windows | L3 |
| EI2 | download: macOS `.dmg`；Windows installer `.exe` or portable `.zip`；Releases link | L10-17 |
| EI3 | **the `future` CLI ships with every download package**（installer and portable both，installed next to the app） | L19-21 |
| EI4 | **「Formal macOS and Windows installers are signed, and the macOS build is also notarized by Apple」**（signed + notarized） | L24 |
| EI5 | macOS first launch: drag into Applications → double-click | L27-30 |
| EI6 | Windows: installer runs the .exe；**portable keeps FutureOS.exe and future-agent.exe in the same folder**；SmartScreen prompts — check the publisher；**needs WebView2 Runtime**（recent Win10/Win11 usually bundled，else install Evergreen）；zip「from the Internet」mark → unblock via file properties / `Get-ChildItem -Recurse | Unblock-File` | L32-42 |
| EI7 | data location: macOS `~/.future`，Windows `C:\Users\<you>\.future` | L49-54 |
| EI8 | updates: Settings → Check for updates（signature verified）；portable replaces the folder；.future kept | L56-59 |
| EI9 | uninstall: macOS delete FutureOS.app；Windows Settings uninstall or delete the portable folder | L61-65 |

### Quick-Start.md（71 lines）
| # | Claim | Location |
|---|---|---|
| EQ1 | login flow: gear icon → Providers → Built-in → FutureGene → **Sign in**（✅ was「Connect」；the button's actual copy is Sign in）→ browser authorization（if not auto-opened use the verification code + copyable link） | L9-19 |
| EQ2 | New Chat vs Workspace comparison table | L24-35 |
| EQ3 | sending: streaming replies、tool-activity display、pause for approval before risky operations；**at most 4 images per message（25 MiB each），other files unlimited**；paperclip/drag/paste | L38-48 |
| EQ4 | model selector inside the input box，with the thinking-level control beside it；Settings → Models to manage | L50-56 |
| EQ5 | right panel: each session has Files + Runs；Workspace additionally has Review（✅ 2026-08-06 rewritten，Artifacts disabled） | L58-66 |

### Using-FutureOS.md（91 lines）
| # | Claim | Location |
|---|---|---|
| EU1 | three-column layout: left nav（New Chat、Models shortcut、Skills、Workspaces、Chats、Settings，collapsible）；middle conversation area（streaming replies/plans/tool activity/command previews/errors/approval cards，input box pinned at the bottom）；right context panel（Files/Runs/Review，✅ was Runs/Review/Artifacts，collapsible） | L6-18 |
| EU2 | Chat vs Workspace table: Chat right column shows Files+Runs（✅ was Runs+Artifacts）；Workspace shows Files+Runs+Review；each session independent | L22-34 |
| EU3 | rename/pin/delete sessions（left-column menu） | L36-37 |
| EU4 | conversation: Enter sends、Shift+Enter newline；per-session model switching；attachment limits same as EQ3；send key becomes stop while streaming | L39-46 |
| EU5 | **approval mechanism**: read/write files/run shell/delete/write outside the workspace → stop + approval card + **no timeout**；Allow once / Deny / Allow in this workspace or chat（editable path rules）；**keyboard Cmd/Ctrl+Enter approves、Esc rejects** | L48-64 |
| EU6 | **approval modes**: Manual（asks before file reads/writes，read-only commands run automatically）/ **Sandboxed（macOS only）** / Unrestricted；settable in Settings → General or the composer shield control | L66-72 |
| EU7 | Runs: cards show real commands/status/counts；Inspect/Terminate/Clear finished | L76-81 |
| EU8 | Review: file list、type（added/modified/deleted/renamed）、per-file diff；under version control can switch「Last run changes」 | L83-85 |
| EU9 | Files: every session shows workspace files（Workspace=project folder，Chat=temp session folder）；preview/system-open/attach-from-directory-tree to a conversation（✅ 2026-08-06 replaced the original Artifacts subsection） | L87-89 |

### Settings.md（82 lines）
| # | Claim | Location |
|---|---|---|
| ES1 | entry: bottom-left gear；left-column Models shortcut | L3 |
| ES2 | settings pages: General / Providers / Models + **Check for updates / Reset** | L5 |
| ES3 | General: Language、Approval mode（Manual/Sandboxed[macOS only]/Unrestricted）、Show thinking process、**Auto-upgrade skills**（✅ 2026-08-06 added，silently upgrades installed skills on app open） | L9-18 |
| ES4 | Providers: FutureGene builtin **Sign in**（✅ was Connect；code+link；logged-in state only has Sign out，no Sign in again）；**other builtin providers（DeepSeek/OpenAI/Anthropic/Google etc）click Configure**（✅ was Set key/Update key，dialog title Set <provider> key）；More providers expands the full list | L22-32 |
| ES5 | custom provider fields: Name（optional）、Provider ID（lowercase alphanumeric/-/_）、**API type = OpenAI Completions / OpenAI Responses / Anthropic**、Base URL、API Key、Models（optional display names）；validation + id uniqueness；Edit/Remove | L34-45 |
| ES6 | Models: grouped by provider、search、visibility switch；the composer selector shares the same source and shows the provider | L48-54 |
| ES7 | Check for updates: checks and downloads the OS-appropriate installer | L57-59 |
| ES8 | Reset: Clear local data clears local data and restarts | L61-64 |

### Skills.md（56 lines）
| # | Claim | Location |
|---|---|---|
| ESK1 | Skills = capability packages，used automatically when installed and relevant；left-column Skills entry | L3-6 |
| ESK2 | two tabs: Installed / All（All needs network）；category dropdown + search；Install/Uninstall | L9-14 |
| ESK3 | **14 builtin skills table**（✅ 2026-08-06 corrected，original 11 items included 3 nonexistent Hand-drawn posters/Hand-drawn slides/Subagent）：Account、Browser、Database lookup、Deep research、Document、Experimental design、Image、Paper、Peer review、Scientific writing、Skill creator、Slides、Software install、Web | L18-30 |
| ESK4 | usage: no manual invocation needed — describe the task | L38-44 |

### CLI.md（127 lines）
| # | Claim | Location |
|---|---|---|
| EC1 | tool name **`future`**，ships with every download package；「you probably won't need it」 | L1-5 |
| EC2 | location: macOS（.dmg）`/Applications/FutureOS.app/Contents/MacOS/future`；Windows（portable .zip）`future.exe`；**the Windows CLI is portable-only，the installer has no future.exe** | L9-18 |
| EC3 | running: `future --help`；can add to PATH / alias（macOS alias example） | L21-27 |
| EC4 | **agent must be running**: if the desktop app is open it is，else `future agent start` | L29-35 |
| EC5 | command groups: auth（login/status/logout）、agent（start/stop/restart/status）、run（--model supports `model:thinking` like `sonnet:high`、--thinking off/minimal/low/medium/high/xhigh、@<path>、--continue/-c、--cwd、--mode json、--no-session；example includes piped input）、tools（list / call --args/--stdin/--output，file-path arguments auto-converted）、skills（list/install/uninstall）、channel（start/stop/restart/status，advanced） | L37-93 |
| EC6 | tips: macOS first-launch blocked → right-click open once；Connection refused → `future agent start` or open the desktop app | L95-99 |

### FAQ.md（65 lines）
| # | Claim | Location |
|---|---|---|
| EF1 | **「The current build isn't notarized, so this is expected」**（current build not notarized —— conflicts with Installation L24「notarized」） | L9 |
| EF2 | macOS won't open: right-click open twice；「damaged」 → `xattr -dr com.apple.quarantine /Applications/FutureOS.app` | L10-15 |
| EF3 | SmartScreen: More info → Run anyway | L18-19 |
| EF4 | Windows unresponsive: install WebView2 Evergreen；portable same folder；unblock the zip | L21-28 |
| EF5 | model unusable / not signed in: Settings → Providers → FutureGene → **Sign in**（✅ was Connect） | L30-32 |
| EF6 | switch model: composer selector / Settings → Models | L34-35 |
| EF7 | agent stops to ask = approval mechanism（no timeout）: Allow once/Deny/allow this project | L37-39 |
| EF8 | data location: ~/.future / C:\Users\<you>\.future | L41-45 |
| EF9 | updates: download-over-install / replace folder；.future kept；Settings → Check for updates | L47-49 |
| EF10 | uninstall/clear data: delete the app + delete .future；Settings → Reset also works | L51-53 |
| EF11 | platforms: **macOS and Windows** | L55-57 |

### Feishu.md（204 lines）
| # | Claim | Location |
|---|---|---|
| EFE1 | architecture diagram: Bridge (WebSocket) ↔ Agent (gRPC 127.0.0.1:50051) | L8-14 |
| EFE2 | prerequisites: Feishu developer account（open.feishu.cn / open.larksuite.com）、bot-capability app、**agent running（`make run-agent` or `future agent start`）** | L17-22 |
| EFE3 | app creation steps: enterprise self-built app → enable Bot → note App ID/App Secret | L24-31 |
| EFE4 | permissions: im:message、im:message.p2p_msg:read、im:message.group_msg:read、im:message:send_as_bot、im:resource、contact:user.base:read | L33-42 |
| EFE5 | event subscription: im.message.receive_v1；Request URL any HTTPS（WebSocket doesn't actually call back but the field is required） | L44-49 |
| EFE6 | config.json: agent{grpc_addr(default http://127.0.0.1:50051)、cwd、model(future/deepseek-v4-pro)、thinking_level(off..xhigh)、permission_level(all/workspace/none)}；feishu{enabled、app_id、app_secret、domain("feishu"/"lark")} | L51-87 |
| EFE7 | policies: dm_policy(open/allowlist[default]/disabled)、dm_allowlist、group_policy(disabled[default])、group_allowlist、require_mention(default true) | L89-116 |
| EFE8 | behavior: streaming(true)、resolve_sender_names(true)、max_image_mb(10)、typing_indicator(false) | L118-126 |
| EFE9 | startup: **`make build-channels-release`** + `./target/release/future-channels`；service management `future channel start/status/stop/restart`（macOS launchctl / Linux systemd）；no config.json → creates a template and exits | L128-143 |
| EFE10 | 9 slash commands: /new /status /model /models /effort /stop /compact /cwd /help；handled locally without the agent；unrecognized ones forwarded as ordinary messages | L145-156 |
| EFE11 | reply effects: streaming (default) CardKit live card + collapsible quote blocks；non-streaming single markdown message | L158-163 |
| EFE12 | troubleshooting: bot not replying（bridge status/enabled/policies/logs）；**reconnects every 6 minutes（30s keepalive ping）**；images（im:resource + max_image_mb） | L165-182 |
| EFE13 | trailing See also link text is **`[[CLI (future-cli)|CLI]]`**（inconsistent with the CLI.md page title「future」） | L204 |

### DingTalk.md（170 lines）
| # | Claim | Location |
|---|---|---|
| EDT1 | architecture diagram: Bridge (Stream Mode) ↔ Agent (gRPC 127.0.0.1:50051)；api.dingtalk.com | L8-14 |
| EDT2 | prerequisites: DingTalk developer account（open.dingtalk.com）、Stream Mode app、agent running（`make run-agent`/`future agent start`） | L17-22 |
| EDT3 | app creation: open-dev.dingtalk.com → bot → message receive mode = Stream Mode → Client ID(AppKey)/Client Secret(AppSecret) | L24-30 |
| EDT4 | permissions: im.message.receive、im.message.send、qyapi_robot_webhook_message_send | L32-39 |
| EDT5 | config.json: dingtalk{enabled、client_id、client_secret、domain(default api.dingtalk.com)}（agent block same as EFE6） | L41-70 |
| EDT6 | startup: `make build-channels-release` + `./target/release/future-channels`；`future channel start/status/stop/restart` | L72-89 |
| EDT7 | 9 slash commands（same as EFE10；**the DingTalk version claims「all slash commands are handled locally by the Bridge」**，different wording from the Feishu version） | L91-103 |
| EDT8 | reply effects: markdown via session webhook；**every reply is a new message（webhook does not support in-place editing）**；thinking `> 💭` quote blocks | L105-111 |
| EDT9 | Feishu-vs-DingTalk difference table: connection（pbbp2 protobuf vs Stream Mode JSON）、streaming（CardKit vs new messages）、thinking chain、emoji reactions（✅ vs ❌ API not public）、multimodal（images/files vs text-only markdown） | L113-120 |
| EDT10 | troubleshooting: bot not replying；**frequent reconnects（keepalive every 20 seconds）**；markdown double newlines | L122-135 |
| EDT11 | trailing See also link text `[[CLI (future-cli)|CLI]]`（same EFE13 inconsistency） | L170 |

### _Sidebar.md（21 lines）
| # | Claim | Location |
|---|---|---|
| ESB1 | groups: Getting started（Install/Quick Start）、Using the app（Using FutureOS/Settings/Skills）、Command line（**CLI (future)**）、**Integrations（Feishu/DingTalk）**、Help（FAQ） | L4-20 |
| ESB2 | link text「CLI (future)」→ CLI（uses `future`，conflicts with prompt-en's future-cli） | L14 |

### _Footer.md（3 lines）
| # | Claim | Location |
|---|---|---|
| ESF1 | macOS and Windows；Download / Report an issue external links | L3 |

### Models.md（4983 lines each en/zh）—— **generated file**
| # | Claim | Location |
|---|---|---|
| EM1 | header declares **「3826 models across 143 providers」**/「3826 个模型，覆盖 143 个 Provider」 | L3 |
| EM2 | generation mechanism: `scripts/generate_models.py`（`make generate-models`），data sources models.dev / openrouter / vercel；the script's `generate_wiki_docs()` writes docs/wiki/{en,zh}/Models.md directly | scripts/generate_models.py L231-331 |
| EM3 | structure: Provider Summary table + one section per provider（Base URL + Model ID/Name/Context/Max Output/Image/Reasoning tables） | from L5 |
| EM4 | **README's「1000+ models / 100+ providers」is inconsistent with the 3826/143 here**（README numbers appear stale，needs verification） | README L26 vs Models L3 |

---

## 9. docs/wiki/zh/*.md（zh，key lines mirroring en）

zh pages match en content，line numbers roughly mirror（zh has 1 more line at Feishu L205、DingTalk L170、_Sidebar uses「命令行工具(future)」）.
Two places worth recording:

| # | Claim | Location |
|---|---|---|
| ZI1 | Installation.md：「正式发布的 macOS 与 Windows 安装包均经过签名，**macOS 版本同时经过 Apple 公证**」 | zh/Installation.md L24 |
| ZF1 | FAQ.md：「**当前版本未公证**,这属于正常现象。」（conflicts with Installation L24） | zh/FAQ.md L9 |
| ZC1 | CLI.md: tool name `future`；macOS `.../MacOS/future`；Windows portable `future.exe` | zh/CLI.md L1, L15-16 |
| ZC2 | Feishu.md / DingTalk.md trailing See also link text「命令行工具(**future-cli**)」 | zh/Feishu.md L205, zh/DingTalk.md L170 |
| ZQ1 | Quick-Start.md / Using-FutureOS.md：「每条消息最多附 **4 张图片**（每张最大 25 MiB），其他文件类型不限制数量」 | zh/Quick-Start.md L45, zh/Using-FutureOS.md L38 |
| ZM1 | Models.md「3826 个模型，覆盖 143 个 Provider」 | zh/Models.md L3 |

---

## 10. docs/architecture-audit/（4 audit reports + README，generated 2026-08-05）

> ⚠️ **Point-in-time snapshot，marked historical（2026-08-06，todo_41f779819879）**: audit baseline dev @ 8aa82925（2026-08-05）；documents entered the repo in the same batch as the first fixes（commit `306cf05f`）— report 01 H2/H3/H4、report 04 H1/H2/H4 were fixed in that commit；still-standing high-risk items are report 01 H1/H5、report 04 H3. Each report's relevant items are marked ✅；the README top timeliness note details file:line drift and the fix list.

| # | Claim | Location |
|---|---|---|
| A0 | audit baseline: `dev @ 8aa82925`（2026-08-05）；investigation done in worktree `8164b8e1`，both trees identical（diff empty）；report file:line valid **at audit time**（2026-08-05）——⚠️ drifted since 2026-08-06（see the warning above this table） | audit/README.md L8-9 |
| A0b | four-report topics and one-line conclusions table | audit/README.md L5-7 |
| A1 | report 01 conclusion: the agent ↔ gui_rust boundary **leaks in both directions**: shadow JSON contract + 7 filesystem bypasses + compile-time source include（`#[path]`）；H1-H5 / M1-M7 / L1-L5 details | audit/01 L1-7, L15-119 |
| A1b | report 01 strongest evidence: RpcResponse.data/StreamEvent.data are JSON strings（proto:220/392）；get_state returns ~35-key ad-hoc JSON（agent/src/rpc/mod.rs:339-376）；GUI writes auth.json/models.json（auth_store.rs:84-149、write.rs:92-258）；`#[path="../../../../agent/src/models/builtin/mod.rs"]`（catalog.rs:15-16）；cleanup.rs:173-241 probes `{id}.jsonl` | audit/01 L32-54, L143-152 |
| A2 | report 02 conclusion: gui_rust ↔ gui_react boundary architecturally clean（all 103 #[tauri::command] registered at lib.rs:600-704；8 events；102 invokeCommand sites 0 bare invoke），but contracts fully hand-synced（39+ type pairs，3 already drifted）；S1-S8 | audit/02 L1-9, L15-52 |
| A3 | report 03 conclusion: 18 oversized-module candidates: 3 Tier1（agent_bridge/mod.rs 1343 lines、session/mod.rs 3624 lines、Composer.tsx 704 lines）、9 Tier2、6 cohesive not to split | audit/03 L1-9, L14-33 |
| A4 | report 04 conclusion: 4 HIGH in the React streaming hot path（H1 handleFork depends on messages, breaking the single memo；H2 threadRunStatuses reducer never bails out → AppShell 25Hz full-tree renders；H3 streaming-tail markdown full reparse O(n²)；H4 Composer+MentionEditor re-render per push）；backend 40ms push coalescing（lib.rs:286-330）≈25/sec | audit/04 L1-9, L18-61 |
| A5 | all audits read-only，no files modified；「fixable separately」 | audit/README.md L3 |

---

## 11. docs/dist/*.txt（release-package readmes，6 files）

> ✅ **Verified（2026-08-06，todo_41f779819879）**: all statements match current source，**no changes needed**. These files are live documents — build.yml（macOS dmg / Windows portable / Linux portable）and scripts/build-desktop-windows-portable.ps1、build-windows-signed.yml copy them verbatim as each package's `Readme.txt`（`-en.txt` are reference translations；packaging uses only the zh `.txt`）. The binary trio（`futureos`/`FutureOS.exe` + `future-agent`/`future-agent.exe` + `future`/`future.exe`）matches build.yml assembly steps one by one（L239-301）；macOS「not Apple-notarised」consistent with FAQ/B8；WebView2/WebKitGTK runtime requirements match Tauri defaults；`~/.future` / `C:\Users\<用户名>\.future` data directories correct.

| # | Claim | Location |
|---|---|---|
| D1 | macOS: **「This build is not Apple-notarised」**（not notarized）；drag into Applications；right-click open / `xattr -dr com.apple.quarantine`；~/.future；future at FutureOS.app/Contents/MacOS/future | readme-macos-en.txt L6-22；readme-macos.txt corresponding |
| D2 | Windows: portable zip；**FutureOS.exe and future-agent.exe in the same folder**；SmartScreen More info→Run anyway；unblock the zip / `Get-ChildItem -Recurse | Unblock-File`；WebView2 Evergreen；`C:\Users\<username>\.future`；future.exe same directory | readme-windows-en.txt L4-31；readme-windows.txt corresponding |
| D3 | Linux: portable tar.gz（`tar -xzf FutureOS-portable-linux.tar.gz` + `./futureos`）；**futureos、future-agent、future in the same folder**；WebKitGTK（Debian/Ubuntu `libwebkit2gtk-4.1-0`，Fedora `webkit2gtk4.1`）；~/.future | readme-linux-en.txt L4-22；readme-linux.txt corresponding |
| D4 | all three platform readmes call the CLI `future`（Linux/macOS/Windows consistent） | each file's Notes |

---

## 12. Cross-document conflicts / observations pending verification（described only，not adjudicated）

> The conflicts below are confirmed to exist between documents；which side is correct is decided by the「verify against source」step.

| # | Topic | The two sides | Location |
|---|---|---|---|
| X1 | **CLI binary name** | `future`: wiki-prompt.md W12（L171/L219）、README R3、wiki CLI.md EC1-EC2、_Sidebar ESB1、dist D4；`future-cli`: wiki-prompt-en.md WE3-WE5（L171/L219，explicitly「never bare future」） | see left |
| X2 | **CLI name inconsistent within the wiki itself** | wiki CLI.md/sidebar use `future`，but Feishu.md/DingTalk.md trailing See also use `future-cli`（en L204/L170，zh L205/L170） | §8/§9 |
| X3 | **macOS notarization status** | notarized: wiki Installation en L24 / zh L24、wiki-prompt W8；not notarized: wiki FAQ en L9 / zh L9（「current build isn't notarized」）、dist readme-macos-en L8（「not Apple-notarised」） | §8/§9/§11 |
| X4 | **does skills have an `update` subcommand** | yes: build-and-install.md B14（L165 `future skills update`）、zh-CN L163；no: wiki-prompt W12（L183「**没有 update**」）、WE（L183「no `update`」） | §3/§7 |
| — | **X4 adjudicated（2026-08-06，todo_cbbb063d2fd4）** | cli/src/commands/skills.ts L19/L48-49/L88/L287-328 implements `update`（updateSkills really performs upgrades）。**build-and-install is correct，wiki-prompt W12/WE wrong**——left for the wiki-prompt todo | |
| X5 | **model counts** | README「1000+ models / 100+ providers」（R2 L26）；Models.md「3826 models / 143 providers」（EM1 L3） | §1/§8 |
| X6 | **wiki-prompt page inventory vs actual wiki** | prompt inventory 10 pages no Feishu/DingTalk、sidebar no Integrations（W6/W7）；actual wiki has Feishu.md+DingTalk.md+Integrations group（ESB1） | §7/§8 |
| X7 | **make targets** | wiki Feishu/DingTalk use `make build-channels-release`（EFE9/EDT6）；Makefile has no such target（grep 0 hits）——likely should be `make build-channels` or a release variant，needs verification | §8；Makefile |
| X8 | **sandbox terminology** | README「sandbox tiers (off / manual / macOS Seatbelt)」（R9）；wiki「Manual / Sandboxed (macOS only) / Unrestricted」（EU6/ES3）；wiki-prompt W10「手动 / 沙盒[仅 macOS] / 无限制」 | §1/§8/§7 |
| X9 | **thinking-level set** | README「off ↔ xhigh」；wiki CLI.md `--thinking` lists off/minimal/low/medium/high/xhigh（EC5）；README doesn't list all levels | §1/§8 |
| X10 | **architecture-audit timeliness** | the audit baseline dev @ 8aa82925（2026-08-05）and its relationship to the current worktree need checking；if source changed, file:line may be invalid | §10-A0 |
| — | **X10 adjudicated（2026-08-06，todo_41f779819879）** | audits are **point-in-time snapshots**: documents and first fixes entered the repo in the same batch（commit `306cf05f`）— report 01 H2/H3/H4、report 04 H1/H2/H4 were fixed in that commit（code comments cite audit numbers）；still-standing high-risk items: report 01 H1/H5、report 04 H3. file:line has drifted，README top has a timeliness note with per-item ✅（details in errors-outdated-missing.md §E） | |
| X11 | **loop CLI command surface** | loop-control-plane.md L15 CLI overview vs actual `future-loop --help`（this round saw goal/todo/ops/cli etc groups running，the full surface needs verification） | §5 |
| — | **X11 adjudicated（2026-08-06，todo_63c718c2a3d5）** | compared one by one against `build_cli_registry()`（main.rs L176-471）: goal group missing `models`/`diagnose`、extension missing `upgrade`、`cli registry` missing `--include-experimental`，fixed both en/zh CLI overviews（recorded in B9）。Everything else consistent | |
| X12 | **`future init` behavior** | build-and-install B14: `future init` = install skills + link local commands on macOS/Linux；needs verification against cli source | §3 |

---

## 13. Key-claim index for verification（by topic，matching the verification todo's check areas）

| Check area | Involved claims | file:line |
|---|---|---|
| TUI slash commands（12） | R6 / RZ4 | README.md L100-114 |
| TUI shortcuts（8） | R7 / RZ5 | README.md L118-130 |
| agent port 127.0.0.1:50051 | R4 / RZ3 / EFE6 / EDT5 | README L89；wiki Feishu L70-87、DingTalk L56-73 |
| config paths ~/.future/agent/{auth,models}.json、~/.future/channels/config.json、~/.future/loop/ | R3 / EFE6 / EDT5 / L18 | README L46-79；wiki Feishu L51-87 |
| toolchain versions: Rust 1.97+、Node 24+、Bun | B1 / B3-B8 | build-and-install L12-18, L30-86 |
| .cargo/config.toml mold claim | B6 | build-and-install L67；.cargo/config.toml |
| Makefile target surface（install/install-desktop/package-desktop/build-*/run-*/generate-*/install-skills/install-loop…） | B4-B17 / X7 | build-and-install throughout；Makefile |
| CLI command surface（auth/agent/run/tools/skills/channel + options） | EC5 / W12 / X1/X2/X4 | wiki CLI.md L37-93；wiki-prompt L175-185 |
| channels config and slash commands | EFE6-EFE10 / EDT5-EDT7 | wiki Feishu/DingTalk |
| future-loop CLI command surface | L15 / X11 | loop-control-plane.md L115-127 |
| proto（future.proto、generated code、generate-proto） | B17 | build-and-install L186-206 |
| GUI feature claims（approval mechanism/settings pages/skills inventory/4 images 25MiB/Artifacts etc） | EU5-EU9 / ES3-ES8 / ESK3 / EQ3 | wiki Using-FutureOS/Settings/Skills/Quick-Start |
| generated files（Models.md） | EM1-EM3 | scripts/generate_models.py L231-331 |

---

## 14. Leftover observations from the read-through phase（from the superseded docs-verification/DOC-FACTS.md，commit 3ed92ad7）

> The early read-through draft（346 lines）was later superseded by this file（400 lines，§0-13 restructure）. The items below existed in that
> draft and were not fully absorbed by §A-E；they remain open for follow-up todos（especially todo_9bb2c6dd1c38 filling gaps）.

### 14a. Not-fully-absorbed observations pending verification

| # | Observation | Status |
|---|---|---|
| H11 | Feishu permission scope table: wiki lists 6 rows（including contact:user.base:read），the old draft suspected「5」——defer to the actual wiki page（wiki-page verification belongs to todo_bcf715c7cc0e） | open |
| H14 | does reading a file require approval: Using-FutureOS approval mechanism says「read or write a file」needs approval，while Manual mode says「read-only commands run automatically」——internal consistency belongs to the GUI verification area（see errors-outdated-missing.md §E） | ✅ resolved（todo_cab9a84ced24）：**consistent，no contradiction**——file-read tool access triggers the file_read approval（approval.rs），while Manual mode's「read-only commands」refers to **read-only shell commands** auto-allowed（shell_auto_allow classification）— different subjects |
| H15 | wiki-prompt §7's referenced GUI source files（ActivityRail.tsx featureItems、SettingsDialog.tsx、Composer.tsx）and audit report 02/03 line numbers need cross-confirmation of existence | ✅ resolved（todo_cab9a84ced24）：all three files exist——ActivityRail.tsx featureItems is an empty array（Research removed，PRODUCT.md §4.9）、SettingsDialog.tsx devOnly mechanism（Remote/Environment dev-only）、Composer.tsx shield/model/thinking-level/stop button |

### 14b. 「Missing documents/sections」candidates（for todo_9bb2c6dd1c38）

| # | Candidate gap | Basis | Status（todo_9bb2c6dd1c38） |
|---|---|---|---|
| I-1 | Models.md missing「how to regenerate」（generator script、command、auto-sync?） | scripts/generate_models.py；EM1-EM3 | ✅ resolved（todo_bcf715c7cc0e：script + file-header comments） |
| I-2 | dist readmes have a Linux version but the wiki has no Linux page——platform positioning needs a decision（add a Linux page vs keep macOS+Windows） | docs/dist/readme-linux*.txt | ✅ adjudicated：**keep macOS+Windows**——release.yml publishes only macOS（arm64+x64 dmg/updater）+ Windows（x64 setup），the Linux portable package is tester-only，no wiki page needed |
| I-3 | no complete reference for channels config（Feishu/DingTalk each have their own page，no unified config.json schema reference page） | EFE6/EDT5 | ✅ resolved：created docs/channels-config.md + zh（all schema fields/defaults，field-by-field verified against channels/src/config.rs） |
| I-4 | loop-control-plane guide doesn't cover the `agent` command group（onboard/scope/lane/supervisor）or handoff usage examples | loop-control-plane.md L115-127 | ✅ resolved：en/zh added the「Multi-agent workflow / 多 agent 工作流」section（agent onboard/registration、scope、lane、supervisor propose|receipt|events、handoff [--write]、task-graph、attention/inbox examples）；and notes these are flat top-level commands（the agent/todo/work-items groups in help are display only） |
| I-5 | no index README at the docs/ top level（other than architecture-audit）——docs directory lacks navigation | directory structure | ✅ resolved：created docs/README.md + zh（top-level guide table、wiki inventory、dist notes、internal working documents） |
| I-6 | no TUI usage document（the wiki deliberately excludes TUI，but README has TUI slash commands/shortcuts） | README R6/R7 | ✅ resolved：created docs/tui.md + zh（17 slash commands、8 shortcuts、settings/keybindings/log paths、troubleshooting） |
| I-7 | no complete `.future/` directory-layout document（responsibilities of the agent/channels/tui/app/workspaces subdirectories） | CLAUDE.md；config-path check area | ✅ resolved：created docs/directory-layout.md + zh（agent/models/auth/sessions/skills/logs、channels、tui、app(db/images/review)、workspaces/chat、loop、bin；plus notes on ~/.agents/skills and project-local .future/） |
