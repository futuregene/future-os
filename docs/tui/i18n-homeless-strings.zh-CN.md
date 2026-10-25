# 没进表的界面文案

`tui/src/workbench/i18n.rs` 把 workbench 的界面文案——页签名、选择器、按键提示、空状态、
通知——用两种语言并排放在一张表里，一行一个键，缺一种语言编译不过。这条性质覆盖的是
**经过表**的文案，对直接写进绘制代码的字符串无效：硬编码的字面量照样编译通过，看起来
也像完工了，只是永远只用一种语言对读者说话。

把这条缝堵上的检查是一个测试：`tui/tests/i18n_guard/`。它随 `cargo test -p future-tui`
运行，不新增 CI 作业；凡是没进表的界面文案，它都会以 `文件:行:字面量` 的形式失败。

## 它读什么

两条规则，各自都刻意保守：

1. **`tui/src/` 下任何字符串字面量里的 CJK 字符。** 这棵树里的界面文案是用中文写的，
   所以一个中文字面量就是一段绕过了表的界面文案，不管它在哪个模块里。内容不受影响——
   对话、会话标题、路径是运行时数据，从来不是字面量。
2. **界面绘制的英文句子**——`workbench/`，加上 workbench 从中取显示文本的几个模块
   （`components/chat_area.rs`、`session_rows.rs`、`themes.rs`、`approval_mode.rs`）。
   “句子”指：一个字面量里有至少两个以空白分隔的、三个字母以上的词，且它不是路径、
   标识符、转义序列或 `{point} {app} {version}` 这类纯骨架。

测试模块不扫描：它不绘制画面，里面的字面量是对表的**期望**。`workbench/demo.rs`
整文件豁免——它里面每个字符串都是真实会话会携带的内容。

### 失败时怎么办

在 [`i18n.rs`](../../tui/src/workbench/i18n.rs) 的 `Key` 里加一个变体、在 `pair()` 里加
一行，然后用 `t(lang, Key::…)` 绘制。确实不是界面文案的字面量有两条出路：

- 在 [`allowlist.rs`](../../tui/tests/i18n_guard/allowlist.rs) 里加一行 `(文件, 字面量)`
  并写明理由；
- 在行尾写 `// i18n: <理由>`——豁免跟着代码走，对一个不变量的 panic 消息这是更合适的形式。

两者都计数。`allowlist.rs` 里的 `BUDGET` 是这棵树允许的豁免总数，计数一旦与之不符——
无论涨还是跌——测试就失败：新增豁免必须动那个 reviewer 一眼能看见的数字，关掉一个就
把它减一。

### 它看不见什么

规则调得让误报很少发生——每周都喊狼来了的检查会被关掉，那比漏报更糟。代价是：

- **一个词的英文标签。** `queued`、`Manual`、`truncated` 都不算句子。今天就有几个是
  纯英文的（approval-mode 芯片的 `Manual` / `Sandboxed`，`chat_area` 里 `RunState`
  的若干状态）。
- **由碎片拼出来的文案**，比如一个标签加一个由第二个 `format!` 拼出的提示。
- **`tui/src/` 之外的文本**——`help.rs` 的 `--help` 输出是刻意纯英文，CLI 按约定保持
  纯英文。
- **一个键的两列含义跑偏。** 那是 `i18n.rs` 自己的测试的职责（两种语言必须不同，且
  `{}` 占位数量一致）。

## `dev-huichen` 现在的状态

在本文档落地的那个提交上，检查读了 52 个文件、2299 个测试模块之外的字面量，命中 60 处
界面文案——全部被豁免：共 34 条豁免，其中 31 行 allowlist（27 条既有缺口、4 条本就不是
界面文案）、1 个文件豁免（`workbench/demo.rs`，25 个字面量）、2 处行内标记。

27 条既有缺口就是 `dev-huichen` 上已经只讲英文的界面文案。每一行在 `allowlist.rs` 里
写明了它为什么还是字面量；关掉一条意味着改用 `t(lang, …)` 绘制、删掉那一行、并把
`BUDGET` 减一。

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
| `components/chat_area.rs:2085` | `ctrl+g to expand` | the footer's hint half (the other half is `truncated`, one word, so the rule  does not reach it) |
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
| `workbench/live.rs:1109` | `Could not set the permission: {error}` | notice: the permission chip failed |
| `workbench/live.rs:1128` | `Could not set the approval mode: {error}` | notice: the mode chip failed |
| `workbench/live.rs:1143` | `Could not answer the approval: {error}` | notice: the approval answer failed to send |
| `workbench/live.rs:1150` | `Could not stop the run: {error}` | notice: the stop request failed |
| `workbench/screen.rs:261` | `no workspace yet` | the header's word for a session that has no workspace |

## 对某个分支跑一遍

检查从环境变量取要读的树，所以同一套规则可以指向一个分支而不动它：

```bash
FUTURE_I18N_GUARD_ROOT=<worktree>/tui/src \
  cargo test -p future-tui --test i18n_guard -- --nocapture
```

覆盖了根目录时只有违规规则生效——`allowlist.rs` 里的基线描述的是它自己所在的那棵树，
不是被测量的那棵。

## 三个待合并分支

`feat/workbench-ui`、`feat/wb-mentions`、`fix/workbench-degrade` 都是在
`fb453522d`（引入这张表的提交）**之前**离开 `dev-huichen` 的，所以它们每一棵树都还是
整片硬编码的界面文案：分别有 181、202、134 处命中。把那些数字当移植清单没有意义——
合并会把表一起带过去，冲突的解决发生在 `dev-huichen` 这一侧。

能移植的是每个分支**自己新增**的部分：拿它的树与它离开 `dev-huichen` 的那个提交相比，
按 `(文件, 字面量)` 做多重集差。每一处新增再分成：

- **表里已有**、拼写完全相同（或 `{}` 骨架相同）——合并时直接调 `t(lang, Key::…)`，
  不需要新增行；
- `workbench/demo.rs` 里的**演示内容**——无需移植；
- **已是既有缺口**——没有新东西；
- **新的界面文案**——需要新增一行，就是下面的清单。

给合并者的两点结构说明：三个分支都还带着 `tui/src/app.rs` 和
`tui/src/components/skills_view.rs`，而这两个文件在 `dev-huichen` 上已经不存在
（workbench 重写时删掉了）；并且每个分支都是**整文件新增** `tui/src/workbench/` 下的
文件，而不是改共享文件。所以这是把一份功能增量移植到当前界面上，不是一次机械合并。

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

新增的界面文案，按文件：

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

被这个分支在不止一处硬编码的字符串（“表里已有”一列就是应当使用、而不是新增的那一行）：

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

这 84 条里有 4 条是不变量消息而不是绘制文本——`row width drift`、`sidebar rendered`（两处）、`rail rendered` 出自 `debug_assert_eq!` 与 `expect`——它们要的是 `// i18n:` 行内标记，不是表里的一行。剩下 80 条是读者真的会看到的文本。

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

新增的界面文案，按文件：

| file | line | literal |
|---|---|---|
| `workbench/mod.rs` | 228 | `the frame must be h rows` |
| `workbench/mod.rs` | 716 | `鼠标点击` |
| `workbench/mod.rs` | 717 | `鼠标拖动` |
| `workbench/screen.rs` | 132 | `a band returned a mis-sized row: {row:?}` |
| `workbench/screen.rs` | 470 | ` ctrl+b 侧栏 · / 命令 · ? 帮助` |
| `workbench/screen.rs` | 472 | ` Esc 回到聊天 · ctrl+b 侧栏 · tab 焦点 · / 命令 · ? 帮助 · ctrl+n 新会话` |

被这个分支在不止一处硬编码的字符串：

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

这 6 条里有 2 条正是 `dev-huichen` 已用 `// i18n:` 就地豁免的不变量消息（`the frame must be h rows`、`a band returned a mis-sized row: {row:?}`），所以真正要移植的是 4 条——而且主要是 `鼠标点击` / `鼠标拖动` 与 `HelpKeyClick` / `HelpKeyDrag` 共用同一个键。

### feat/wb-mentions

这个分支已提交的 tip 是一个 `dev-huichen` 快照——它对 `tui/src` 没有任何新增。提到
（mentions）与权限那部分工作**还没提交**，就在 `.worktrees/wb-input` 里
（`tui/src/workbench/` 下十一个已修改文件，外加两张截图），所以测量方式是拿工作区与已
提交的树相比。谁把它提交上去，谁就要负责移植。

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

新增的界面文案，按文件：

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

被这个分支在不止一处硬编码的字符串：

| literal | places | already in the table |
|---|---|---|
| `文件` | 2 — `workbench/mod.rs:116`, `workbench/screen.rs:570` | — |
| `会话` | 2 — `workbench/mod.rs:117`, `workbench/screen.rs:571` | `TabSessions` |

## 同一段文字出现在好几处

有三种模式值得一次改对，而不是改三次：

- **面板与区域的名字。** `feat/workbench-ui` 在 `workbench/screen.rs` 和
  `workbench/state.rs` 里硬编码了 `聊天`、`侧栏`、`面板`、`折叠`、`输入`、`启动台`、
  `外观`、`目标`——其中 `聊天` 和 `外观` 各两处；而 `侧栏` 和几个页签名还以表里已有的
  拼写再出现一次（`HintSidebar`、`TabTodo` … `TabSkills`）。一个名字一个 `Key`，面板
  绘制、状态栏和命令面板行都用它。
- **一个动作的名字与它的命令面板行。** `压缩上下文` 出现在 `workbench/state.rs:530`
  和 `app.rs:11210`；`目标` 出现在 `workbench/launchpad.rs:37` 和
  `workbench/state.rs:513`；`新建会话` 出现在 `workbench/launchpad.rs:35` 和
  `workbench/state.rs:458`（那一条其实是 `CmdNew`）。命令面板行和它打开的选择器必须
  共用一个键，否则两种语言会对同一个功能各说各话。
- **空状态和它的出路。** `无待办 · /goal 创建`、`无定时任务 · future task list`、
  `无会话 · /new 开始`、`未安装技能 · /skills` 都把命令提示写死在句子里。每个面板一个
  键、命令作为 `{}` 值传进去，提示才可翻译，而不是被冻在句子里。

`feat/wb-mentions` 为它的选择器重复了同一个形状两次：`文件` 在
`workbench/mod.rs:116` 和 `workbench/screen.rs:570`，`会话` 在
`workbench/mod.rs:117` 和 `workbench/screen.rs:571`。

## 怎么复现这些数字

```bash
# 当前这棵树
cargo test -p future-tui --test i18n_guard -- --nocapture

# 另一个检出（只看已提交内容）
git archive <ref> tui/src | tar -x -C /tmp/probe
FUTURE_I18N_GUARD_ROOT=/tmp/probe/tui/src \
  cargo test -p future-tui --test i18n_guard -- --nocapture
```

移植清单就是把这份输出在分支与 `git merge-base <branch> origin/dev-huichen` 之间按
`(文件, 字面量)` 相减得到的；`feat/wb-mentions` 用 `.worktrees/wb-input` 的工作区代替
分支头。
