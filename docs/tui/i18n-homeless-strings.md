# Interface text that never reached the table

`tui/src/workbench/i18n.rs` keeps the workbench's chrome — tab labels, pickers,
key hints, empty states, notices — in both languages, one key per row, and a key
with only one language does not compile. That property covers the text that goes
*through* the table. It says nothing about a string written straight into a
drawing path, and a hardcoded literal compiles, looks finished, and silently
reports only one language to the reader.

The check that closes that hole is a test: `tui/tests/i18n_guard/`. It runs with
`cargo test -p future-tui`, needs no new CI job, and fails with
`file:line: literal` for every piece of chrome that is not in the table.

## What it reads

Two rules, one conservative each:

1. **A CJK character in any string literal under `tui/src/`.** The chrome is
   authored in Chinese in this tree, so a Chinese literal is chrome that skipped
   the table, whichever module it sits in. Content is untouched — a
   conversation, a session title or a path is runtime data, never a literal.
2. **An English sentence drawn by the surface** — `workbench/`, plus the
   modules the workbench pulls display text out of (`components/chat_area.rs`,
   `session_rows.rs`, `themes.rs`, `approval_mode.rs`). "Sentence" means two
   whitespace-separated words of three or more letters, in a literal that is not
   a path, an identifier, an escape sequence or a format skeleton such as
   `{point} {app} {version}`.

Test modules are not scanned: they never draw a frame, and their literals are
expectations *about* the table. `workbench/demo.rs` is exempt as a file — every
string in it is the content a real session would carry.

### When it fails

Add a variant to `Key` and a row to `pair()` in
[`i18n.rs`](../../tui/src/workbench/i18n.rs), then draw the text with
`t(lang, Key::…)`. Two escapes exist for a literal that is not chrome:

- a row in [`allowlist.rs`](../../tui/tests/i18n_guard/allowlist.rs) saying why,
  one row per `(file, literal)` pair;
- `// i18n: <reason>` at the end of the line — the exemption travels with the
  code, which is the right shape for an invariant's panic message.

Both are counted. `BUDGET` in `allowlist.rs` is the number of exemptions the tree
is allowed, and the test fails when the count stops matching it, in either
direction: a new exemption has to move the number a reviewer can see, and a
closed one lowers it.

### What it cannot see

The rules are tuned so that a false alarm stays rare — a check that cries wolf
every week gets switched off, which is worse than a miss. What that leaves out:

- **A one-word English label.** `queued`, `Manual`, `truncated` are not
  sentences. Several of them are English-only today (the approval-mode chip's
  `Manual` / `Sandboxed` rows, the run states in `chat_area`'s `RunState`).
- **Text assembled from fragments** across calls, e.g. a label plus a hint built
  by two `format!`s.
- **Text outside `tui/src/`** — `help.rs`'s `--help` output is deliberately
  English-only, and the CLI is English by policy.
- **A key whose two columns drift** in meaning. That is `i18n.rs`'s own tests'
  job (the two languages must differ, and keep the same number of `{}` holes).

## Where `dev-huichen` stands

At the commit this document landed with, the guard reads 52 files, 2299 string
literals outside test modules, and 60 chrome candidates — all of them exempted,
by 34 exemptions: 31 allowlist rows (27 known gaps, 4 not chrome), one file
exemption (`workbench/demo.rs`, 25 literals) and 2 in-source markers.

The 27 known gaps are the English-only chrome already on `dev-huichen`. Each row
says in `allowlist.rs` why it is still a literal; closing one means drawing it
with `t(lang, …)`, deleting its row and lowering `BUDGET` by one.

| file:line | literal | why it is not in the table yet |
|---|---|---|
| `approval_mode.rs:71` | `Automatic review` | the approval-mode chip's row |
| `approval_mode.rs:72` | `Full access` | the approval-mode chip's row |
| `session_rows.rs:33` | `Context compacted` | the history's compaction marker |
| `session_rows.rs:31` | `Context compacted: {before} → {after} tokens (estimated)` | the same marker, carrying the token counts |
| `components/chat_area.rs:118` | `merged into the previous run` | a run state's description in the conversation |
| `components/chat_area.rs:1813` | ` Thinking... (ctrl+o to expand)` | the live-run placeholder shown while the thinking is folded away |
| `components/chat_area.rs:2010` | `tool call` | the reply summary counts tools in English, pluralised inline |
| `components/chat_area.rs:2010` | `tool calls` | the plural half of the same count |
| `components/chat_area.rs:2080` | `  … {hidden} more line{plural} · {hint}` | the folded-output footer: pluralised inline and cut to width |
| `workbench/live.rs:309` | `Could not load the transcript: {error}` | notice: the transcript failed to load |
| `workbench/live.rs:323` | `Could not load older rows: {error}` | notice: the page above the transcript failed to load |
| `workbench/live.rs:440, 1167` | `Approve tool execution` | notice: the approval card's title |
| `workbench/live.rs:780` | `Failed to send: {error}` | notice: the prompt failed to send |
| `workbench/live.rs:813, 1003` | `Could not set the model: {error}` | notice: the model change failed |
| `workbench/live.rs:853` | `Could not export: {error}` | notice: /export failed |
| `workbench/live.rs:877` | `Could not switch session: {error}` | notice: switching session failed |
| `workbench/live.rs:992` | `Could not start a session: {error}` | notice: a new chat failed to start |
| `workbench/live.rs:1014` | `Could not set thinking: {error}` | notice: the effort change failed |
| `workbench/live.rs:1030` | `Could not cycle the model: {error}` | notice: the model cycle failed |
| `workbench/live.rs:1045` | `Could not cycle thinking: {error}` | notice: cycling the effort failed |
| `workbench/live.rs:1063` | `No models to choose from yet` | notice: the model picker has nothing to list |
| `workbench/live.rs:1128` | `Could not set the approval mode: {error}` | notice: the mode chip failed |
| `workbench/live.rs:1143` | `Could not answer the approval: {error}` | notice: the approval answer failed to send |
| `workbench/live.rs:1150` | `Could not stop the run: {error}` | notice: the stop request failed |
| `workbench/screen.rs:261` | `no workspace yet` | the header's word for a session that has no workspace |

## Running it against a branch

The check takes the tree to read from an environment variable, so the same rules
can be pointed at a branch without touching it:

```bash
FUTURE_I18N_GUARD_ROOT=<worktree>/tui/src \
  cargo test -p future-tui --test i18n_guard -- --nocapture
```

With the root overridden only the violation rule applies — the allowance
baseline in `allowlist.rs` describes the tree the file lives in, not the one
being measured.

## The three branches waiting to merge

`feat/workbench-ui`, `feat/wb-mentions` and `fix/workbench-degrade` all left
`dev-huichen` **before** `fb453522d` (the commit that introduced the table), so
each of their trees is still entirely hardcoded chrome: 181, 202 and 134
candidates respectively. Counting those is not a port list — merging brings the
table with it, and the merge's own conflicts are on the dev-huichen side.

What is portable is what each branch *adds*: its tree measured against the
commit where it left `dev-huichen`, as a multiset difference of
`(file, literal)` pairs. Each added literal is then one of:

- **already in the table**, spelled identically (or with the same `{}` shape) —
  the merge calls `t(lang, Key::…)` and adds no row;
- **demo content** in `workbench/demo.rs` — nothing to port;
- **already a known gap** — nothing new;
- **new chrome** — it needs a row, and that is the list below.

Two structural notes for whoever merges: all three branches still carry
`tui/src/app.rs` and `tui/src/components/skills_view.rs`, which no longer exist
on `dev-huichen` (the workbench rewrite removed them), and each branch adds whole
files under `tui/src/workbench/` rather than editing shared ones. The merge is a
port of a feature delta onto the current surface, not a mechanical merge.

### feat/workbench-ui

| | |
|---|---|
| tip | `d7ccaeb93` |
| where the branch left `dev-huichen` | `1d8a972cb` |
| hardcoded chrome in the tree today | 181 literals |
| …added by this branch | 167 literals |
| …already spelled the same in the table | 46 → `TabSessions`×6, `PickerTitleModel`×5, `TabSkills`×4, `HintHelp`×4, `CmdNew`×2, `ChipEffort`×2, `ChipMode`×2, `HintNewChat`×2, `CacheLabel`×2, `HintSidebar`×2, `CmdQuit`×2, `PickerTitleTheme`, `LaunchpadRecent`, `UsageContext`, `UsageCost`, `HintCommands`, `TabTodo`, `TabJobs`, `TabUsage`, `ChipAccess`, `ActivityThinking`, `ActivityTool`, `UsageOutput`, `CmdSkills` |
| …demo content (`workbench/demo.rs`) | 37 → nothing to port |
| …already a known gap | 0 → nothing new |
| …**new chrome** | **84** |

New chrome, by file:

| file | line | literal |
|---|---|---|
| `workbench/screen.rs` | 202 | `row width drift` |
| `workbench/screen.rs` | 586 | `sidebar rendered` |
| `workbench/screen.rs` | 594 | `rail rendered` |
| `workbench/screen.rs` | 615 | `sidebar rendered` |
| `workbench/screen.rs` | 647 | ` ↑ 更早 {scroll} 条 ` |
| `workbench/screen.rs` | 678 | `还没有消息` |
| `workbench/screen.rs` | 679 | `· 按 / 打开命令，或直接输入任务` |
| `workbench/screen.rs` | 686 | `↑ 更早 {scroll} 条` |
| `workbench/screen.rs` | 872 | `enter 发送` |
| `workbench/screen.rs` | 874 | `shift+enter 换行` |
| `workbench/screen.rs` | 876 | `/ 命令` |
| `workbench/screen.rs` | 917 | `输入任务，/ 命令，? 帮助` |
| `workbench/screen.rs` | 982 | `{} 工具` |
| `workbench/screen.rs` | 989 | `空闲 · 等待输入` |
| `workbench/screen.rs` | 1024 | `已连接` |
| `workbench/screen.rs` | 1024 | `离线` |
| `workbench/screen.rs` | 1099 | `聊天` |
| `workbench/screen.rs` | 1101 | `面板` |
| `workbench/screen.rs` | 1104 | `折叠` |
| `workbench/screen.rs` | 1220 | ` 命令面板 ` |
| `workbench/screen.rs` | 1287 | `↑↓ 选择` |
| `workbench/screen.rs` | 1288 | `tab 补全` |
| `workbench/screen.rs` | 1289 | `enter 执行` |
| `workbench/screen.rs` | 1290 | `esc 关闭` |
| `workbench/screen.rs` | 1309 | `鼠标` |
| `workbench/screen.rs` | 1309 | `点击面板 / 页签 / 行，滚轮滚动，拖分隔条调宽` |
| `workbench/screen.rs` | 1310 | `侧栏 开 / 聚焦 / 关` |
| `workbench/screen.rs` | 1311 | `面板焦点轮转（shift+tab 反向）` |
| `workbench/screen.rs` | 1312 | `侧栏放大到 48 列` |
| `workbench/screen.rs` | 1313 | `折叠 / 展开待办列表` |
| `workbench/screen.rs` | 1314 | `命令面板（过滤、tab 补全、↑↓ 历史）` |
| `workbench/screen.rs` | 1315 | `本帮助（esc 关闭）` |
| `workbench/screen.rs` | 1316 | `聊天滚动 1 行（pgup/pgdn 一屏）` |
| `workbench/screen.rs` | 1317 | `回到最早 / 最新` |
| `workbench/screen.rs` | 1318 | `发送输入` |
| `workbench/screen.rs` | 1319 | `输入换行` |
| `workbench/screen.rs` | 1320 | `逐层关闭：覆盖层 → 面板 → 输入` |
| `workbench/screen.rs` | 1321 | `折叠 / 展开最后一张工具卡` |
| `workbench/screen.rs` | 1322 | `future-dark / future-light 切换` |
| `workbench/screen.rs` | 1323 | `切换侧栏面板` |
| `workbench/screen.rs` | 1339 | ` 帮助 ` |
| `workbench/screen.rs` | 1407 | ` {} 全屏 ` |
| `workbench/state.rs` | 80 | `启动台` |
| `workbench/state.rs` | 81 | `聊天` |
| `workbench/state.rs` | 83 | `输入` |
| `workbench/state.rs` | 336 | `空闲` |
| `workbench/state.rs` | 464 | `会话列表（全屏）` |
| `workbench/state.rs` | 470 | `恢复历史会话` |
| `workbench/state.rs` | 476 | `切换模型` |
| `workbench/state.rs` | 482 | `思考强度 low\|medium\|high` |
| `workbench/state.rs` | 488 | `运行模式 plan\|act\|review` |
| `workbench/state.rs` | 494 | `权限 ask\|auto\|read-only` |
| `workbench/state.rs` | 500 | `切换主题 future-dark\|future-light` |
| `workbench/state.rs` | 501 | `外观` |
| `workbench/state.rs` | 506 | `打开侧栏面板 todo\|jobs\|sessions\|usage\|skills` |
| `workbench/state.rs` | 507 | `外观` |
| `workbench/state.rs` | 512 | `goal 控制面状态` |
| `workbench/state.rs` | 513 | `目标` |
| `workbench/state.rs` | 524 | `工具与 MCP 服务` |
| `workbench/state.rs` | 530 | `压缩上下文` |
| `workbench/state.rs` | 536 | `清空当前对话` |
| `workbench/state.rs` | 542 | `键位与帮助` |
| `workbench/state.rs` | 643 | `无待办` |
| `workbench/state.rs` | 645 | `{open} 待办 · {active} 进行中` |
| `workbench/panels.rs` | 108 | `无待办 · /goal 创建` |
| `workbench/panels.rs` | 125 | `✔ {done}/{total} · {active} 进行中` |
| `workbench/panels.rs` | 135 | `（已折叠 · ctrl+q 展开）` |
| `workbench/panels.rs` | 178 | `↑ 还有 {start} 条` |
| `workbench/panels.rs` | 197 | `无定时任务 · future task list` |
| `workbench/panels.rs` | 235 | `无会话 · /new 开始` |
| `workbench/panels.rs` | 296 | `缓存` |
| `workbench/panels.rs` | 297 | `速度` |
| `workbench/panels.rs` | 318 | `未安装技能 · /skills` |
| `workbench/launchpad.rs` | 36 | `会话列表` |
| `workbench/launchpad.rs` | 37 | `目标` |
| `workbench/launchpad.rs` | 142 | `工作台` |
| `workbench/launchpad.rs` | 154 | `{} 工作台` |
| `workbench/launchpad.rs` | 163 | `输入任务开始，或键入 / 打开命令` |
| `workbench/launchpad.rs` | 191 | `描述任务，@ 引用文件，/ 打开命令…` |
| `workbench/launchpad.rs` | 386 | `enter 发送 · / 命令` |
| `app.rs` | 11210 | `压缩上下文` |
| `app.rs` | 11214 | `{tools} 个工具` |
| `workbench/theme.rs` | 93 | `Future Dark` |
| `workbench/theme.rs` | 114 | `Future Light` |

Strings this branch hardcodes in more than one place (the "already in the table"
column is the row to use instead of a new one):

| literal | places | already in the table |
|---|---|---|
| `会话` | 6 — `workbench/state.rs:35`, `workbench/state.rs:459`, `workbench/state.rs:465`, `workbench/state.rs:471`, `workbench/state.rs:531`, `workbench/state.rs:537` | `TabSessions` |
| `模型` | 5 — `workbench/state.rs:132`, `workbench/state.rs:477`, `workbench/state.rs:483`, `workbench/state.rs:489`, `workbench/state.rs:495` | `PickerTitleModel` |
| `技能` | 4 — `workbench/launchpad.rs:38`, `workbench/state.rs:37`, `workbench/state.rs:519`, `workbench/state.rs:525` | `TabSkills` |
| `帮助` | 4 — `workbench/launchpad.rs:40`, `workbench/screen.rs:1103`, `workbench/state.rs:543`, `workbench/state.rs:549` | `HintHelp` |
| `压缩上下文` | 2 — `app.rs:11210`, `workbench/state.rs:530` | — |
| `重构 goal 控制面` | 2 — `workbench/demo.rs:56`, `workbench/demo.rs:115` | — |
| `新建会话` | 2 — `workbench/launchpad.rs:35`, `workbench/state.rs:458` | `CmdNew` |
| `目标` | 2 — `workbench/launchpad.rs:37`, `workbench/state.rs:513` | — |
| `强度` | 2 — `workbench/panels.rs:299`, `workbench/state.rs:133` | `ChipEffort` |
| `模式` | 2 — `workbench/panels.rs:301`, `workbench/state.rs:134` | `ChipMode` |
| `新会话` | 2 — `workbench/screen.rs:476`, `workbench/state.rs:161` | `HintNewChat` |
| `sidebar rendered` | 2 — `workbench/screen.rs:586`, `workbench/screen.rs:615` | — |
| `缓存 {}%` | 2 — `workbench/screen.rs:998`, `workbench/state.rs:220` | `CacheLabel` |
| `聊天` | 2 — `workbench/screen.rs:1099`, `workbench/state.rs:81` | — |
| `侧栏` | 2 — `workbench/screen.rs:1100`, `workbench/state.rs:82` | `HintSidebar` |
| `退出 workbench` | 2 — `workbench/screen.rs:1324`, `workbench/state.rs:548` | `CmdQuit` |
| `外观` | 2 — `workbench/state.rs:501`, `workbench/state.rs:507` | — |

Four of the 84 are invariant messages rather than drawn text — `row width drift`, `sidebar rendered` (twice) and `rail rendered` come from `debug_assert_eq!` and `expect` — so they want a `// i18n:` marker, not a table row. The remaining 80 are text a reader sees.

### fix/workbench-degrade

| | |
|---|---|
| tip | `54ae4c69a` |
| where the branch left `dev-huichen` | `1d8a972cb` |
| hardcoded chrome in the tree today | 134 literals |
| …added by this branch | 120 literals |
| …already spelled the same in the table | 66 → `HintCommands`×2, `HintHelp`×2, `HintNewChat`×2, `CmdReplyDetail`×2, `PickerTitleModel`, `ChipEffort`, `ChipMode`, `ChipAccess`, `HintSidebar`, `LaunchpadTagline`, `LaunchpadNewChatIn`, `LaunchpadRecent`, `InputHintLaunchpad`, `ApprovalWaiting`, `CmdNew`, `CmdSessions`, `CmdModel`, `CmdWorkspace`, `CmdTheme`, `CmdUsage`, `CmdSkills`, `CmdExport`, `CmdTranscript`, `CmdHelp`, `CmdQuit`, `HelpKeyNew`, `HelpKeySidebar`, `HelpKeyFocus`, `HelpKeyScroll`, `HelpKeyPage`, `HelpKeyPalette`, `HelpKeyHelp`, `HelpKeyReplyDetailSet`, `HelpKeyClick`, `HelpKeyDrag`, `HelpKeyQuit`, `SummaryTodo`, `SummaryJobs`, `SummarySessions`, `SummarySkills`, `UsageContext`, `UsageCost`, `UsageCache`, `UsageOutput`, `InputHintChat`, `ChatOlderHint`, `ActivityThinking`, `ActivityTool`, `ActivityStreaming`, `ActivityTools`, `HelpFooter`, `PalettePanelTitle`, `PaletteFilter`, `PaletteEmpty`, `ChatOlder`, `TabTodo`, `TabJobs`, `TabSessions`, `TabUsage`, `TabSkills`, `CacheLabel`, `PanelEmptyJobs` |
| …demo content (`workbench/demo.rs`) | 30 → nothing to port |
| …already a known gap | 18 → nothing new |
| …**new chrome** | **6** |

New chrome, by file:

| file | line | literal |
|---|---|---|
| `workbench/mod.rs` | 228 | `the frame must be h rows` |
| `workbench/mod.rs` | 716 | `鼠标点击` |
| `workbench/mod.rs` | 717 | `鼠标拖动` |
| `workbench/screen.rs` | 132 | `a band returned a mis-sized row: {row:?}` |
| `workbench/screen.rs` | 470 | ` ctrl+b 侧栏 · / 命令 · ? 帮助` |
| `workbench/screen.rs` | 472 | ` Esc 回到聊天 · ctrl+b 侧栏 · tab 焦点 · / 命令 · ? 帮助 · ctrl+n 新会话` |

Strings this branch hardcodes in more than one place:

| literal | places | already in the table |
|---|---|---|
| `补一个回归测试` | 2 — `workbench/demo.rs:82`, `workbench/demo.rs:178` | — |
| `修复 parser 的 off-by-one` | 2 — `workbench/demo.rs:95`, `workbench/demo.rs:172` | — |
| `描述你要做的事 · @ 文件 · # 会话 · / 命令` | 2 — `workbench/demo.rs:148`, `workbench/live.rs:115` | `InputHintLaunchpad` |
| `待办` | 2 — `workbench/demo.rs:219`, `workbench/state.rs:204` | `TabTodo` |
| `任务` | 2 — `workbench/demo.rs:219`, `workbench/state.rs:205` | `TabJobs` |
| `会话` | 2 — `workbench/demo.rs:219`, `workbench/state.rs:206` | `TabSessions` |
| `用量` | 2 — `workbench/demo.rs:219`, `workbench/state.rs:207` | `TabUsage` |
| `技能` | 2 — `workbench/demo.rs:219`, `workbench/state.rs:208` | `TabSkills` |
| `命令` | 2 — `workbench/launchpad.rs:81`, `workbench/screen.rs:594` | `HintCommands` |
| `帮助` | 2 — `workbench/launchpad.rs:82`, `workbench/screen.rs:556` | `HintHelp` |
| `新会话` | 2 — `workbench/launchpad.rs:83`, `workbench/launchpad.rs:152` | `HintNewChat` |
| `Approve tool execution` | 2 — `workbench/live.rs:301`, `workbench/live.rs:724` | — |
| `最终输出 / 完整过程` | 2 — `workbench/mod.rs:694`, `workbench/mod.rs:714` | `CmdReplyDetail` |

Two of the six are the same invariant messages `dev-huichen` already exempts in place with `// i18n:` (`the frame must be h rows`, `a band returned a mis-sized row: {row:?}`), so the port is four strings — and mostly `鼠标点击` / `鼠标拖动` sharing one key with `HelpKeyClick` / `HelpKeyDrag`.

### feat/wb-mentions

This branch's committed tip is a `dev-huichen` snapshot — it adds nothing to
`tui/src`. The mentions and permission work lives **uncommitted** in
`.worktrees/wb-input` (eleven modified files under `tui/src/workbench/` plus two
screenshots), so it was measured from the working tree against the committed one.
Whoever commits that work is the one who has to port it.

| | |
|---|---|
| tip | `742fed83f` |
| where the branch left `dev-huichen` | `bb7178bb3` |
| hardcoded chrome in the tree today | 202 literals |
| …added by this branch | 47 literals |
| …already spelled the same in the table | 4 → `TabSessions`×2, `PickerEmptyModel`, `PickerUngrouped` |
| …demo content (`workbench/demo.rs`) | 1 → nothing to port |
| …already a known gap | 1 → nothing new |
| …**new chrome** | **41** |

New chrome, by file:

| file | line | literal |
|---|---|---|
| `workbench/mod.rs` | 116 | `文件` |
| `workbench/mod.rs` | 123 | `↑↓ 选择 · ⏎ / tab 插入 · esc 关闭（不带 @ 提交则原样发送）` |
| `workbench/mod.rs` | 124 | `↑↓ 选择 · ⏎ / tab 插入会话链接 · esc 关闭` |
| `workbench/mod.rs` | 1666 | `选择模型与推理强度` |
| `workbench/mod.rs` | 1667 | `圈选 ctrl+p 轮换的模型范围` |
| `workbench/mod.rs` | 1668 | `安全等级：all / workspace / none` |
| `workbench/mod.rs` | 1680 | `与 ctrl+t 相同：推理强度` |
| `workbench/mod.rs` | 1681 | `侧栏待办面板（仓库的目标看板）` |
| `workbench/mod.rs` | 1719 | `会话列表，选中即切换` |
| `workbench/mod.rs` | 1720 | `轮换模型（/models 圈选的范围）` |
| `workbench/mod.rs` | 1721 | `轮换推理强度` |
| `workbench/mod.rs` | 1722 | `轮换安全等级 all/workspace/none` |
| `workbench/mod.rs` | 1723 | `命令与技能，tab 补全` |
| `workbench/mod.rs` | 1724 | `引用文件（插入 @path，提交时带内容）` |
| `workbench/mod.rs` | 1725 | `引用会话（插入会话链接）` |
| `workbench/mod.rs` | 1731 | `切页签、点行、点状态栏的模型/强度/权限` |
| `workbench/screen.rs` | 569 | `命令/技能` |
| `workbench/screen.rs` | 570 | `文件` |
| `workbench/screen.rs` | 572 | `换会话` |
| `workbench/screen.rs` | 573 | `换模型` |
| `workbench/screen.rs` | 741 | `输入以过滤命令与技能 · tab 补全 · ⏎ 运行` |
| `workbench/screen.rs` | 748 | ` 没有匹配的命令或技能` |
| `workbench/screen.rs` | 761 | ` 技能（进草稿后由 agent 展开）` |
| `workbench/screen.rs` | 763 | ` 命令` |
| `workbench/screen.rs` | 848 | `（没有匹配）` |
| `workbench/picker.rs` | 53 | `安全等级` |
| `workbench/picker.rs` | 54 | `模型范围` |
| `workbench/picker.rs` | 99 | `↑↓ 选择 · ⏎ 应用安全等级 · esc 取消` |
| `workbench/picker.rs` | 100 | `↑↓ 移动 · space 勾选 · ⏎ 保存范围（空 = 全部） · esc 取消` |
| `workbench/picker.rs` | 110 | `没有匹配的等级` |
| `workbench/picker.rs` | 396 | `不拦工具调用` |
| `workbench/picker.rs` | 397 | `workspace 外与危险操作需批准` |
| `workbench/picker.rs` | 398 | `只读：禁用所有工具` |
| `workbench/live.rs` | 1163 | `模型范围已清空（ctrl+p 回到全部模型）` |
| `workbench/live.rs` | 1165 | `ctrl+p 在 {} 个模型间轮换` |
| `workbench/live.rs` | 1401 | `[{path}: {} KB, not inlined — read it in pieces with the read tool]` |
| `workbench/live.rs` | 1405 | `[{path}: binary, not inlined]` |
| `workbench/panels.rs` | 116 | `本仓库没有进行中的目标` |
| `workbench/panels.rs` | 120 | `future goal init 创建一个` |
| `workbench/panels.rs` | 130 | `目标` |
| `workbench/panels.rs` | 138 | `这个目标还没有待办` |

Strings this branch hardcodes in more than one place:

| literal | places | already in the table |
|---|---|---|
| `文件` | 2 — `workbench/mod.rs:116`, `workbench/screen.rs:570` | — |
| `会话` | 2 — `workbench/mod.rs:117`, `workbench/screen.rs:571` | `TabSessions` |

## The same text in several places

Three patterns are worth fixing once rather than three times:

- **Pane and panel names.** `feat/workbench-ui` hardcodes `聊天`, `侧栏`, `面板`,
  `折叠`, `输入`, `启动台`, `外观` and `目标` in `workbench/screen.rs` and
  `workbench/state.rs` — `聊天` and `外观` twice each, and `侧栏` plus the tab
  titles a second time as the strings the table already has (`HintSidebar`,
  `TabTodo` … `TabSkills`). One `Key` per name, drawn by the panel painter, the
  status bar and the palette row alike.
- **An action's name and its palette row.** `压缩上下文` appears in
  `workbench/state.rs:530` and `app.rs:11210`; `目标` in
  `workbench/launchpad.rs:37` and `workbench/state.rs:513`; `新建会话` in
  `workbench/launchpad.rs:35` and `workbench/state.rs:458` (that one is
  `CmdNew`). A palette row and the picker it opens must share the key, or the
  two languages will disagree about the same feature.
- **The empty state and its escape hatch.** `无待办 · /goal 创建`,
  `无定时任务 · future task list`, `无会话 · /new 开始`, `未安装技能 · /skills`
  each carry a command hint inside the sentence. One key per panel with the
  command as a `{}` value keeps the hint translatable instead of frozen inside
  the sentence.

`feat/wb-mentions` adds the same shape twice for its pickers: `文件` at
`workbench/mod.rs:116` and `workbench/screen.rs:570`, `会话` at
`workbench/mod.rs:117` and `workbench/screen.rs:571`.

## Reproducing the numbers

```bash
# this tree
cargo test -p future-tui --test i18n_guard -- --nocapture

# another checkout, commits only
git archive <ref> tui/src | tar -x -C /tmp/probe
FUTURE_I18N_GUARD_ROOT=/tmp/probe/tui/src \
  cargo test -p future-tui --test i18n_guard -- --nocapture
```

The port tables were produced by diffing that output between a branch and
`git merge-base <branch> origin/dev-huichen`, per `(file, literal)` pair; for
`feat/wb-mentions` the working tree of `.worktrees/wb-input` stood in for the
branch's head.
