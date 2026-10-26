# FutureOS 终端界面（TUI）

`future tui` 是全屏终端界面：它占有整块屏幕、重绘绝对行、用「画过什么」来解析鼠标点击。
它取代了之前的行内 TUI（把行追加进终端自身的 scrollback、只对尾部做 diff）——那套界面、
它的代码和它的页面都已删除，不是标记弃用。

设计契约见 [`docs/tui/workbench-spec.zh-CN.md`](../tui/workbench-spec.zh-CN.md)。

## 怎么跑

```bash
future tui                          # 终端界面，接上你的 agent
future tui --session <id>           # 打开指定会话
future tui --continue               # 最近活跃的那个
future tui "看看这个挂掉的测试"      # 启动时直接发第一条消息
future tui -p "这个报错是什么意思"   # 只问一句，不开界面

# 只跑界面，确定性演示数据（不连 agent，标题栏写 demo）
cargo run -p future-tui --example workbench_demo -- --scenario chat

# 任意尺寸打一帧：截图或检查布局
cargo run -p future-tui --example workbench_demo -- --scenario launchpad --width 120 --height 38
cargo run -p future-tui --example workbench_demo -- --scenario chat --plain     # 去掉 ANSI

# 鼠标验收（120x38 与 90x24 两档命中区域）
cargo run -p future-tui --example workbench_demo -- --self-test

# 逐页用真实按键与鼠标事件驱动（命中区、帧尺寸、可达状态）
cargo run -p future-tui --example workbench_interaction -- --size 120x38
cargo run -p future-tui --example workbench_interaction -- --size 40x12

# 以及行为层：指针下面的那一行，打开的真的是那一行吗
cargo run -p future-tui --example workbench_semantics
```

场景：`launchpad`（还没有会话）、`chat`（一轮对话，过程折叠成一行汇总）、`workspaces`
（十二个工作区和它们的会话，用来看侧栏的滚动窗口和 `↓ N more`）。

## 一帧长什么样

```
 专注头部（2 行：会话 · workspace · 后端状态，随后细分隔线）
 舒适头部（可选 6 行：像素鲸、字标、会话 · workspace · 模型、后端状态）
 ──────────────────────────────────────────┬──────────────────────
 对话（底部对齐，向上有 ↑ 更早）            │ 侧栏：最近 工作区
                                            │ ▸ 会话标题
                                            │   ~/future-os · 2m
 ──────────────────────────────────────────┴──────────────────────
 输入框（❯ 多行草稿，最多增高到 8 个可视行）
 活动行：⠋ 思考 … · 3s · 2 工具                      42 tok/s · 缓存 96%
 状态栏：model · effort · permission   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 提示行：Esc 回到聊天 · ctrl+b 侧栏 · tab 焦点 · ctrl+r 会话 · ctrl+p 命令
```

侧栏是一个**任务面板**，分两个页签。**最近**（默认）列出全部会话——名字一行，它所在的工作区
和刷新时间一行——最新的在前。行里的工作区写的是**名字**（`future-os`），不是路径：侧栏只有
30 多列，被截断的路径比这个名字说得更少。**工作区**按桌面端侧栏的方式归档同一批会话：先
**置顶**（用 `p` 置顶的工作区），再 **工作区**（每个工作区一个标题，会话在其下），最后
**对话会话**（不属于任何工作区的会话，即 app 自己的临时目录）。用量数字在状态栏，面板不再
重复；技能属于命令面板。侧栏拿到键盘焦点时，`←`/`→` 在两个页签之间切换。

launchpad 是**正文**，不是盖在对话上的弹窗：没有会话时整块屏幕就是输入界面（logo、字标、
输入框、工作区 / 模型 / 强度 / 模式 / 权限五个可点参数、快捷键），底部不再重复画输入带，
会话列表也只由侧栏负责、这里不再重复。`tab` 在输入框和参数之间移动，`enter` 循环当前参数，
打字或 `escape` 把键盘交回输入框。

两个断点都来自契约：

- **宽 < 88**：侧栏收起，`ctrl+b` 改为把当前面板全屏打开。
- **高 < 24**：头部压成 1 行，输入框只留上边框，活动行隐藏。

每一帧都恰好是 `height` 行、每行恰好 `width` 个显示列（CJK 与 emoji 计 2），1×1 也一样——
这条不变量由帧测试和唯一负责补齐的 `ansi::fit` 共同保证。

## 界面语言

界面有中英两套，**界面文案**（页签、选择器、按键提示、空态、提示行）按读者的语言绘制。
**内容**不翻译：对话、会话标题、文件路径、agent 的输出，写成什么就是什么。

默认跟随系统，读者选过之后记在 `~/.future/tui/settings.json`（`language`：`system` |
`zh` | `en`，与桌面端、手机端同一套词汇；缺这个键就是 `system`）。

- `/lang` 打开选择器：**跟随系统**、**中文**、**English**。
- 语言取 `LC_ALL`、`LC_MESSAGES`、`LANG` 中第一个有效的；Windows 上三者都没设时取
  用户的界面语言。缺失或 `C` 都按英文。
- `cargo run -p future-tui --example workbench_demo -- --lang en` 可以打一帧英文界面；
  截图流程把 `zh` 写进它自己的临时 home 固定下来，基线不因谁跑而变。

文案集中在一张表 `tui/src/workbench/i18n.rs`，两种语言写在同一行：只加一种语言的键编译不过。

## 按键与鼠标

| 按键 | 动作 |
|---|---|
| `tab` / `shift+tab` | launchpad：在输入框与参数行之间移动（`enter` 循环光标所在那项，`escape` 或打字回到输入框）；对话里：把键盘焦点交给侧栏（光标落在屏幕上那条会话上）再收回 |
| `←` `→` | 移动草稿光标；侧栏有焦点时在它的两个页签之间切换（最近 ↔ 工作区） |
| `←` `→` | 移动草稿光标 |
| `↑` `↓` | 在草稿行 / 提交历史之间移动；侧栏聚焦时换到下一条 / 上一条会话（跳过标题行），`enter` 打开 |
| `shift+←` / `shift+→` | 按侧栏顺序切到上一条 / 下一条会话，光标在哪都行：对话与侧栏光标一起跟着换 |
| `p` | 侧栏聚焦时：置顶 / 取消置顶选中会话所在的工作区 |
| `ctrl+j` / `ctrl+enter` | 插入换行。`ctrl+j` 任何终端都送来；`ctrl+enter` 需要终端上报修饰键（kitty 键盘协议、xterm `modifyOtherKeys`，或 Windows Terminal 里自己绑一条 `sendInput`），`shift+enter` 同样需要。Windows Terminal 把 `alt+enter` 留给了全屏，且默认把 `ctrl+enter` 当普通 Enter 发，除非读者自己绑，例如 `{"keys": "ctrl+enter", "command": {"action": "sendInput", "input": "\u001b[13;5u"}}` |
| `ctrl+w`、`alt+d`、`ctrl+u`、`ctrl+k`、`ctrl+y` | 删词、删到行首 / 行尾、粘贴刚删除的内容 |
| `ctrl+z` / `ctrl+shift+z`（或 `alt+y`） | 撤销 / 重做草稿编辑 |
| `pageUp` / `pageDown` 或 `ctrl+↑` / `ctrl+↓` | 滚动对话；`esc` 回到最新输出 |
| `ctrl+g` 或 `/queue` | 查看排队消息；`x` 取消，`r` / `enter` 取消并取回草稿（`ctrl+x` 停止当前运行，不是取消队列） |
| `F6` 或 `/copy` | 复制最后一条助手回答的正文和代码，不带界面装饰 |
| `F7` 或 `/mouse` | 关闭 / 恢复并保存鼠标捕获，便于终端原生选择复制 |
| `F8` | 切换终端原生 scrollback 线性阅读模式 |
| `F9` | 独立切换减少动态模式 |
| `ctrl+b` | 侧栏：显示并聚焦 → 取消聚焦 → 收起；窄终端下全屏打开。显示 / 收起是**偏好**，写进 `sidebarVisible`，下次启动从这里开始 |
| `/` 或 `ctrl+p` | 命令面板（`/` 需要草稿为空）——输入过滤，`enter` 执行 |
| `ctrl+r` | 会话选择器（每条会话的标题、工作区与时间；提示行里也写着这个键） |
| `ctrl+o` 或 `/preferences` | 偏好设置：密度、侧栏位置 / 宽度、快捷键 |
| `F1` 或 `?` | 帮助浮层（显示当前实际生效的快捷键 + 命令） |
| `home` / `end`、`ctrl+a` / `ctrl+f` | 草稿光标移到行首 / 行尾 |
| `ctrl+←` / `ctrl+→` | 草稿按词移动；`ctrl+↑` / `ctrl+↓` 滚动对话 |
| `/lang` | 界面语言：跟随系统 / 中文 / English |
| `ctrl+e` | 切换「最终输出 / 完整过程」：已结束的一轮会把它的全部过程——答案最后一段文字之前*和之后*的调用与思考——折成一行汇总 |
| `F5` | 刷新会话与用量；`ctrl+x` 停止当前生成 |
| `ctrl+c` | 退出 |

| 鼠标 | 动作 |
|---|---|
| 点侧栏页签 | 切换面板（侧栏有焦点时 `←`/`→` 等效） |
| 点会话行 | 打开该会话（点工作区标题只接管按键） |
| 拖分隔条 | 调侧栏宽度（30–48 列），松手持久化 |
| 点输入框任意处 | 聚焦 |
| 对话区滚轮 | 滚动；到顶时请求更早一页 |
| 侧栏滚轮 | 滚动面板 |
| 点工作流行 | 打开会话、fork 节点或 worktree；选择 Diff 文件 |
| 点命令面板条目 | 执行该命令（点不动的列表等于把 `ctrl+p` 让给键盘） |
| 点排队消息 | 取消并取回草稿，与在那里按 `enter` 一致 |
| 点浮层外的界面 | 关闭浮层；点在浮框本身上则被吸收，不会穿透到背后的控件 |
| 点偏好或快捷键行 | 选中该行、打开快捷键编辑器或开始键位捕获 |
| 点审批操作 | 执行与按键相同的允许一次、规则并允许或拒绝操作 |

鼠标上报使用 SGR 1006，启动时读取保存的偏好（默认开启）；`/mouse` 可以在运行时释放 / 恢复捕获并保存选择，退出时总会还原终端。草稿的文字、光标、折叠粘贴和图片附件按会话保留，经过浮层、launchpad 和会话切换也不丢失（仅在当前 TUI 进程内，不写入磁盘）。

对话默认跟随最新输出。手动向上滚动后保持阅读位置；新输出只显示提醒徽标，不把读者拉回底部。滚回底部或按 `esc` 恢复跟随。

驱动只重绘有变化的帧：空闲时没有 120ms 定时重绘。流式输出与 spinner 更新按 80ms 合并，输入与 resize 立即重绘；转录按宽度和内容版本缓存。减少动态不调度动画重绘；线性阅读绕过全屏重绘，只追加新正文并重绘单行草稿提示。

## 布局与个性化

默认 **专注密度** 把品牌头部压缩为两行，同样的 38 行终端多出四行对话。`ctrl+o` 打开偏好面板，`↑↓` 选行、`←→` 调整：

- **密度**：专注（两行头部）/ 舒适（完整像素鲸和字标）。
- **侧栏位置**：左侧 / 右侧；拖动分隔条调整 30–48 列，`ctrl+b` 显示 / 聚焦 / 收起。
- **鼠标与通知**：保存启动及运行时的鼠标偏好。本 TUI 提交的运行成功结束、无排队后续且终端失焦时通知；审批与错误无论焦点都提醒，并按请求 id / run id 去重。取消、历史回放和其他客户端的运行不通知；不支持焦点上报时按已聚焦处理，不延迟发送前台完成通知。`notify.enabled` 是总开关，`notify.bell` 与 `notify.osc9` 分别控制提示音和终端桌面通知序列；实际投递取决于终端支持，文本会清理控制序列、压成单行并限制长度。
- **减少动态 / 线性阅读**：`F9` 只关闭动画；`F8` 把纯文本写入终端原生 scrollback，不是 alternate screen 的单栏静态替代。进入时输出一次已加载对话，流式正文只追加完整行且不重复 token，单行草稿原地更新；历史由终端 / 屏幕阅读器自身的滚动导航读取。复杂浮层（diff、历史、偏好）仍使用全屏工作流，尚未完成全线性菜单及屏幕阅读器交互审计。线性模式下按 `ctrl+b` 会退出线性模式。
- **主题**：`/theme` 保留六套主题选择器，包括 `high-contrast`，选择保存为 `themeId`。未保存主题时用 `COLORFGBG` 跟随终端浅色 / 深色（未知值回退深色）；存在 `NO_COLOR`（包括空值）即禁用样式。
- **快捷键**：`enter` 进入编辑器，选中动作后按新组合键。冲突会与原占用者交换，不会出现一个按键同时触发两个动作；`r` 恢复默认，`escape` 取消捕获或返回。复制、队列、鼠标、减少动态和线性阅读同样是可重绑的上下文全局动作；浮层保留局部按键，避免全局键误提交审批或误改历史搜索。

偏好与快捷键列表在矮窗口中跟随当前项滚动并保留底部提示。所有选择立即写入 `~/.future/tui/settings.json`（`density`、`sidebarPosition`、`sidebarWidth`、`sidebarVisible`）；快捷键的增量覆盖在同级 `keybindings.json`，键名为稳定动作 id，例如：

```json
{
  "commandPalette": "ctrl+p",
  "sidebar": "ctrl+j"
}
```

支持 `Ctrl/Alt + 字母或数字`、`F1`–`F24`、`Tab`、`Escape`、`Enter`、`Home`、`End`、`PageUp`、`PageDown`。非法或未知配置会逐项忽略，不会导致启动失败。帮助浮层、底部提示与事件分发读取同一份键位，修改后三处同步。

配置文件也可以直接编辑：

```json
{
  "density": "focused",
  "sidebarPosition": "right",
  "sidebarWidth": 36,
  "sidebarVisible": true
}
```

## 哪些已经接上 agent

适配层是 `tui/src/workbench/live.rs`：从 `WorkbenchData` 画、把 `WbAction` 落地，并且复用
行内界面的叶子而不是重写——对话用 `ChatArea`（markdown、工具行、思考、最终输出折叠）、
历史用 `session_rows::rows_to_messages`、所有调用走 `GrpcClient`。

| 已经可用 | 说明 |
|---|---|
| 接会话、`--session`、`--continue` | `get_state` + `switch_session` |
| 历史与翻页 | 启动加载最新一页，到顶时取更早一页（`get_session_entries`，有界的向后游标） |
| 发消息与流式 | `prompt`（并入策略）；`text_chunk`、`thinking_*`、`tool_*`、`agent_end` 驱动对话 |
| 队列管理 | `/queue` 读取 `get_state.queuedRuns`；取消与取回复用现有 `cancel_queued_run`，不维护第二套客户端队列 |
| 草稿附件 | 复用 `components/input.rs` 的折叠粘贴和图片标记机制；附件随 prompt 发送 |
| 审批 | 请求挂起时打开完整卡片，显示工具、类型、风险、动作/路径、原始请求和 sandbox 边界。`y` / `n` 允许一次或拒绝；`a` 先添加 agent 建议的**本次运行**工作区规则再批准，规则失败则请求仍保持待决。Esc 只收起卡片，不作决定 |
| Diff 审查 | `/diff` 列出会话工作区已跟踪与未跟踪改动，并显示选中文件的真实 git patch。宽终端保留固定文件导航，旁边的补丁独立滚动；窄终端用 `tab` 在文件与补丁间切换。上下键操作当前视图，左右键选择文件。箭头立即跟随光标，`·` 标出补丁已加载完的那个文件（重新读取还在路上时）。`r` 刷新；这是当前未提交工作区，不是按轮次归属，patch 预览有大小上限 |
| 历史搜索与树 | `/history` 筛选当前 `list_sessions` 的标题/首条消息预览、目录与 id；`/tree` 使用 `parentSessionId`。回车打开所选会话 |
| 浮层列表 | 一条记录固定占一行：上下键正好走一行，长标题就地缩短而不是折行（折行会让一次按键跨过两行）。放不下的字段整块丢弃——工作区路径从中间截断会读成另一个目录——所以窄框里依次是标题、标题 · 工作区、再加 id。`pageUp`/`pageDown` 按一屏移动光标（审批卡这类详情视图则滚动；diff 跟随当前有焦点的半边） |
| `/worktree` 列表 | `›` 是选择；`●` 表示会话已经在这个工作区，所以即使当前行是第一行，选择也看得见 |
| Fork | `/fork` 列出 `get_fork_messages` 的真实用户消息节点；回车调用 `fork` 并打开子会话 |
| Worktree | `/worktree` 列出真实 git worktree，回车设为会话目录；`/worktree new <branch>` 在 `.worktrees/` 下创建并移动会话。不提供 remove/prune/reset/clean |
| 已有设置入口 | `/sandbox`、`/permission [all|workspace|none]`、`/compact [说明]`、`/tools a,b` 或 `/tools off`、`/providers`；`/skills` 与 `/model` 保留已有界面 |
| 会话面板 | `list_sessions`，最新的在前：最近页签是平铺列表，工作区页签按目录归档、置顶的排在前面。置顶记在 `settings.json`（`pinnedWorkspaces`） |
| 用量 / 上下文 / 成本 | `get_state` 的 usage，随同样的事件刷新 |
| `ctrl+n` / `/new`、`ctrl+x` 停止、`ctrl+k` 刷新 | |
| launchpad 参数 | 点「工作区 / 模型 / 强度 / 权限 / 模式」循环切换；在 launchpad 上，工作区与模型 / 强度是替「第一条消息将要创建的会话」暂存的（此时 agent 还没有会话可设），权限与模式则直接循环 agent 自己的状态。工作区会在本客户端有会话的目录之间循环，提示行写出选中目录的完整路径 |
| 主题、阅读档位 | 存 `~/.future/tui/settings.json`（`tui/src/settings.rs`） |

## 还没有的部分

- **`@` 文件引用**尚未实现；粘贴图片路径与折叠粘贴已复用 `tui/src/paste.rs`。
- **命令面板**只列适配层实现的命令。提供商与工具管理采用紧凑入口（`/providers`、
  `/tools`），`/skills` 仍列在命令面板里。审批允许规则仅本次运行有效；持久化规则文件编辑仍在桌面端。
- 后续增强：时间线轨、图片预览、会话管理器全屏版、全屏草稿编辑器。

## 代码在哪

| 文件 | 职责 |
|---|---|
| `tui/src/workbench/run.rs` | 驱动：终端、按需合并重绘、复制与鼠标捕获 |
| `tui/src/workbench/editor.rs` | 共享多行编辑器的字节索引适配；按会话保存完整草稿 |
| `tui/src/workbench/scroll.rs` | 跟随最新、未读输出与回到最新语义 |
| `tui/src/clipboard.rs` | 纯助手正文剪贴板输出 |
| `tui/src/workbench/live.rs` | agent 适配层：填 `WorkbenchData`、落地 `WbAction` |
| `tui/src/index.rs` | 参数、`--help`、print mode、`--list-models`；交互式交给驱动 |
| `tui/src/workbench/mod.rs` | `Workbench`（视图状态）、`WbEvent` / `WbAction`、键鼠分发 |
| `tui/src/workbench/screen.rs` | 帧合成：七个分区、把局部区域拼进整行、浮层 |
| `tui/src/workbench/layout.rs` | 分区矩形、两个断点、命中区域 |
| `tui/src/workbench/ansi.rs` | 宽度安全文本：`fit`、`pad`、`clip`、按列切片 |
| `tui/src/workbench/widgets.rs` | 分隔线、盒框、页签、进度条、徽标、spinner、logo、字标、输入框 |
| `tui/src/workbench/panels.rs` | 五个侧栏面板（纯渲染函数） |
| `tui/src/workbench/picker.rs` | `/model`、`/theme`、`/sessions` 的列表选择器 |
| `tui/src/workbench/launchpad.rs` | 首屏 |
| `tui/src/workbench/state.rs` | `WorkbenchData` 及其组成部分 |
| `tui/src/workbench/mouse.rs` | SGR 1006 解析 |
| `tui/src/workbench/theme.rs` | 六套调色板：`dark`、`light`、`one-dark`、`one-light`、`high-contrast`、`dracula` |
| `tui/src/workbench/demo.rs` | 确定性演示数据（截图与测试） |
| `tui/examples/demo_agent.rs` | 截图对话用的脚本化 gRPC agent |

## 截图

终端界面和桌面/手机端一样是真截的：真实二进制在固定尺寸的 pty 里跑一个脚本化 agent，把它写出的字节重放成 PNG。

```bash
cargo build -p future-cli --bin future
cargo build -p future-tui --example demo_agent
python3 scripts/screenshots/capture.py tui tui-chat.png     # → .screenshots/tui-chat.png
```

完整做法、参数和两点限制见 [screenshots.zh-CN.md](screenshots.zh-CN.md)（第 8 条）；`scripts/screenshots/tui_terminal.py --self-test` 不开浏览器就能验证单元模型。
