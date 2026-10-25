# dsh-TUI 0.17 能力盘点与 FutureOS 差距分析

dsh-TUI 0.17 到底是什么、「工作台感」由什么构成、`future-tui` 现在站在哪里。
本文是**证据与差距**文档；冻结的实现契约（类型名、模块划分、布局常量、键位）
在 [workbench-spec.zh-CN.md](workbench-spec.zh-CN.md)；按序改造清单在
[workbench-redesign.zh-CN.md](workbench-redesign.zh-CN.md)；逐条对照、每条都带
截图的验收记录在 [verification.md](verification.md)（英文）。

- **对照对象 A**：`@deepseek-harness-tui/dsh-tui` **0.17.0**，读的是本机安装的
  真实包，加上主管现场抓的终端帧。
- **对照对象 B**：`future-tui`，`dev-huichen` 上的 **130c0277d** —— `future tui`
  成为工作台、内联 UI 被删除的那个提交。
- **撰写时间**：2026-10-25（第一版在 `9d5f02beb`，本版在工作台落地之后）。
- **本文规则**：每条结论都带证据标签；没有标签的句子是设计建议，会明确标注。

## 0. 证据标签

| 标签 | 含义 | 来源 |
|---|---|---|
| `[S:<file>]` | 抓下来的终端帧 | `docs/tui/assets/<file>`（我方）与 `.future/workbench-lab/shots/dsh_*.png` + `raw_*.txt`（dsh） |
| `[R]` | 包内 README | 0.17.0 包的 `README.md`（约 72 KB） |
| `[C:<path>]` | 编译后的包源码 | 安装包的 `lib/types/**`（931 个 `.js` 模块） |
| `[F:<path>:<line>]` | FutureOS 源码 | `130c0277d` 上的 `tui/src/**`（分支 `docs/workbench-parity`） |
| `[P]` | RPC 线协议 | `packages/rpc/proto/future.proto` |
| `[V]` | 本轮真机实测 | 在 tmux 里对隔离 agent 跑的真实会话，命令见 `verification.md` §7 |
| `[D]` | 设计建议 | 未实现、未测量 |

## 1. 方法与实际检查过的东西

dsh-TUI 0.17.0 在本机作为 dsh profile 依赖安装，所以读的是**真实包**，不是宣传页：

- `~/.dsh/profiles/dsh-tui/package.json` 钉住
  `@deepseek-harness-tui/dsh-tui: 0.17.0`；包目录在
  `~/.dsh/profiles/dsh-tui/node_modules/@deepseek-harness-tui/dsh-tui`。
- `.future/workbench-lab/shots/` 里的 dsh 抓帧取自 2026-10-25 16:18–16:23 的一次
  真实会话（工作区 `~/.future/workspaces/chat/thread-20261025-160805-bca419/`）：
  七张 PNG 加七张同一屏幕的 ANSI 原始帧，另外一张 `future_base.png` 是
  **工作台之前**的 FutureOS TUI，用于左右对比。从原始帧量出来，dsh 那次会话跑在
  **120 列 × 38 行** —— 正好是契约选定的主尺寸。

FutureOS 这一侧不是凭记忆，是量出来的。本轮在 tmux 里用 120×38、90×24、60×20
三种尺寸驱动真实工作台，背后是隔离 agent：

- `FUTURE_HOME=/tmp/dvhome future agent --grpc-addr 127.0.0.1:5511` —— 全新的数据
  根，只有 `settings.json`、`models.json`、`skills/`、`tasks/tasks.db` 的副本，
  **没有凭据**（`auth.json` 刻意没有复制）。
- 把一个真实会话（`20261021-005514-…`，2,360 条 entries、2,799 条 message blocks、
  16 个 run）用 `ATTACH` + `INSERT` 搬进这个根：只搬 sessions / entries /
  message_blocks / runs。这就是为什么截图里有真实 transcript、真实
  `ctx 128.9k/200.0k 64.4%`、真实 `≈¥18.84`；而启动台那张（第二个实例、没有会话）
  显示 `‹no model›`：没有 `auth.json` 时 agent 有模型目录但没有密钥，这正是首次
  使用的启动台长相。
- 侧栏面板读的是本机状态，所以待办面板显示的是本仓库真实的目标树
  （`✔/◉/○/✖` + 优先级 + 截断文本），任务/技能面板读 `tasks.db` 与已安装技能目录。

`docs/tui/assets/` 里每一张都是真实合成器的输出，用 `tmux capture-pane -p -e` 抓
下来再渲染成 PNG，没有一张是 mock。`[V]`

## 2. 一段话说明 dsh-TUI 是什么

dsh-TUI 是一个 **React/Ink 终端应用**（自带 reconciler、Yoga 布局 `[R]`），包住
一个 agent 后端（DSH，或经 adapter kernel 的 Claude/Codex），呈现一个连续界面：
带真实输入框的落地页、工具卡片可点的 transcript、常驻的实时侧栏，以及一条携带
模型、思考强度、缓存命中率、会话成本与上下文占用的状态栏 `[S:dsh_chat]`。它是
**带设计系统的客户端**，不是「带提示符的聊天记录」：像素鲸鱼、方块字标、面板符号
和九个设置分组属于同一套 kit `[C:lib/types/components/design-system]`，而且中英
双语贯穿全篇 `[S:dsh_setup1]` 与 `[S:dsh_setup3]`。它自己的 README 明确说后端零
改动：只消费 DSH 的公开导出 `[R]`。

这条约束和 FutureOS 相同（`packages/rpc` 是唯一线协议，本工作不得触碰），所以它
是有用的参照，而不是竞争对手。

## 3. 能力盘点（0.17）

### 3.1 启动与首次运行

| 能力 | 证据 |
|---|---|
| 普通启动落在**启动台**：两个 5 行方块字标（`DSH`、`TUI`）、带真实光标的大输入框（`❯ 说点什么，或输入 / 看命令…`）、一行参数 `deepseek-flash · Max · standard · 工作区读写`，然后是 cwd + `dsh-tui v0.17.0`，再是后端行（`▸ DSH · dsh-core v0.2.1-rc.2`、`Claude · 未安装 · 按 Enter 安装`） | `[S:dsh_launchpad]` |
| 启动台输入框是**真输入**：输入、`/` 补全、`Tab` 填充、`Enter` 执行/清空、`@` 附件、图片粘贴都可用；`Shift+Enter` 带着草稿进入聊天 | `[R]` |
| 四步首次向导：① API key + 连通性 ② 语言 + 主题 ③ 模型 + 强度 + 工作区 ④ 快捷键与命令；面包屑显示进度；`←/→` 换步、`Enter` 执行、`Esc` 跳过 | `[S:dsh_setup1]`、`[S:dsh_setup3]` |
| 向导可用 `/setup` 随时重跑；语言可实时切换中/英 | `[R]`、`[S:dsh_setup1]` 对 `[S:dsh_setup3]` |
| 启动台默认开启，可关闭（`/settings` → `launchpad` 关掉后直接进聊天） | `[R]` |
| 三种开场动画；鲸鱼点击会醒，但首个任务之后冻结（装饰有状态，不假装进度） | `[R]` |

### 3.2 外壳与页眉

| 能力 | 证据 |
|---|---|
| 五行页眉：像素鲸鱼、三行方块字标、`deepseek-flash · Max effort`、会话 cwd，再一行轮转提示 | `[S:dsh_chat]` |
| 页眉是三层「活着」信号的第一层（页眉、activity 行、状态栏），见 §4.1 | `[S:dsh_chat]`、`[R]` |
| 全部内容在一个 alt screen 内；另有内联模式：transcript 留在终端自己的回滚缓冲里，鼠标选择归终端 | `[R]` |

### 3.3 Transcript

| 能力 | 证据 |
|---|---|
| 角色字形与统一节奏：`❯` 用户、`⏺` 助手、`• Bash(uname -srm; …) · 0s` 表示工具调用并用 `⎿` 接输出、`⚓ 思考 （ctrl+o 展开）` 表示折叠的推理块 | `[S:dsh_chat]` |
| 代码块带标题框（`┌─ code ─…`）与独立背景；中文行按每字两格精确折行，列不歪 | `[S:dsh_chat]` |
| 流式 Markdown、工具卡、`/` 与 `@` 补全、`#L12-14` 行号区间、历史搜索 | `[R]` |
| Diff 卡片：默认左右两栏，**单侧**文件占满正文宽度而不是留一个空栏；折叠/流式展开按文件 | `[R]` |
| 可点击的**时间轴轨道**：每个回合一个刻度；timeline / scrollbar / 隐藏三种模式 | `[R]` |
| Mermaid 画成 Unicode 图；LaTeX 画成 Unicode（叠分数/上下限），图形终端上可出图 | `[R]` |
| 图片：Kitty 与 Sixel 缩略图、居中预览可缩放平移、粘贴时按尺寸适配、文本回退 | `[R]` |
| 全屏拖选按面板复制（聊天、侧栏、diff 任一栏），软折行合并但不凭空插入换行，代码缩进保留；双击/三击选词/选行 | `[R]` |
| 只有一行状态标记的纯文本回复也保留可复制；实时状态行不污染 transcript | `[R]` |

### 3.4 输入区

| 能力 | 证据 |
|---|---|
| 带边框的输入框，`❯` 开头，左侧会话管理入口 `⌸`，右侧全屏编辑器入口 `⛶` | `[S:dsh_chat]` |
| 上下文占用紧贴状态栏上方、右对齐（`6.9k/1.0M 0.7%`）：「我还剩多少」和「要花多少」在同一视线内 | `[S:dsh_chat]` |
| 模型工作期间：`Enter` 转向（steer）、`Tab` 排队、`Ctrl+Enter` 中断并发送；写着命令的文本仍然执行命令而不是变成 steer | `[R]` |
| `Alt+↑` 召回上一条；`Ctrl+U` 删到行首并保留右侧文本与整块图片 chip；`Alt/⌥+←/→` 按词移动 | `[R]` |
| 图片 chip 是原子（方向键跨过、在边界按退格整块删除、`Ctrl+W` 不劈开） | `[R]` |
| 草稿、光标与已插入图片在启动、切会话、`/btw`、全屏编辑器之间都保留；没有任何东西会自动发出 | `[R]` |
| `Ctrl+Shift+E` 全屏草稿编辑器；外部编辑器；vim 模式 | `[R]` |

### 3.5 侧栏

终端够宽时 `Ctrl+B` 把聊天与面板栏分开；分隔条可拖；窄终端与内联模式回退成全屏
面板 `[R]`。八个内置面板，默认全开 `[R]`、
`[C:lib/types/components/sidePanel/builtinPanels.js]`：

| id | 图标 | 顺序 | 最小列宽 | 能力 |
|---|---|---|---|---|
| `todo` | `≡` | 10 | — | 目标/待办列表；`Ctrl+Q` 折叠，空闲时点标题隐藏 |
| `info` | `ⓘ` | 15 | 24 | 滚动 |
| `jobs` | `▸` | 20 | 28 | 滚动、发到聊天、全屏 |
| `btw` | `?` | 22 | 28 | 滚动、发到聊天、全屏 |
| `trajectory` | `∿` | 25 | 28 | 滚动、全屏 |
| `agents` | `◆` | 30 | 28 | 滚动、全屏（子代理看板） |
| `workspace` | `⌗` | 35 | 28 | 滚动、全屏 |
| `companion` | `♥` | 40 | — | 宠物 |

面板条把**非活动**面板画成符号、活动面板画成标题，两端 `◀`/`▶`，全屏入口 `⤢`
—— 实测为 `◀  ⌗  ?  ♥  待办  ▸  ◆  ⓘ  ∿  ▶  ⤢` `[S:dsh_panel]`。空态是解释而不是
留白：agents 面板显示 `○ 当前会话暂无子代理 / …`，jobs 面板显示
`0 运行中  0 已完成` `[S:dsh_panel]`、`[S:dsh_model]`。获得焦点的面板接管键盘，
`Esc` 回到聊天 `[S:dsh_panel]`。

### 3.6 实时状态

| 读数 | 证据 |
|---|---|
| Activity 行：动画帧 + 动词 + 用时 + 工具数 + TPS，空闲或失焦时静止 | `[R]`、`[C:lib/types/components/ActivityLine.js]` |
| 上下文条：填充随后端占用变化，颜色估算内容构成（压缩后清掉过期构成） | `[R]` |
| 状态栏：`deepseek-flash · max · 缓存 96.6% · ≈¥0.01 谷 · dshlab`，再 `ctx 0.7% (6.9k/1.0m)`；`≈¥` 含子代理用量；未定价模型只显示 token 并标注无定价 | `[S:dsh_chat]`、`[R]` |
| git 分支与会话元数据跟随「会话移动目录」（worktree、`working_directory` 工具） | `[R]` |
| 活动归属：activity 行说明是哪个子代理在干活 | `[R]`、`[C:lib/types/components/ActivityLine.js]` |

### 3.7 命令面与选择器

- 二十余条内置命令 `[R]`：`/resume` `/home` `/agentview` `/bg` `/model` `/new`
  `/compact` `/export` `/btw` `/tree` `/fork` `/rewind` `/settings` `/setup`
  `/status` `/cost` `/jobs` `/skills` `/mcp` `/provider` `/auth` `/login`
  `/update` `/dsh` `/codex` `/claude` `/kernel` `/trace` `/rename` `/recap` `/clear`。
- `?` 打开**两栏浮层**：左栏快捷键、右栏命令与一行描述；页脚教翻页
  （`↑/↓ 滚动 · PgUp/PgDn 翻页 · Home/End 首尾 · Esc 关闭`）`[S:dsh_help]`。
- `/model` 把**模型与强度**放一页：provider 页签（`最近使用` 在前）、强度行
  （`推理强度 ←/→ 调整`、`Off Low High Max`）与逐档描述；`Enter` 应用、`Esc` 取消，
  鼠标可点页签/行/档位 `[S:dsh_model]`、`[R]`。
- 补全在启动台与聊天间共享：`Tab` 填充、`Enter` 执行并清空 `[R]`。
- 九个设置分组，中英双语标签与逐项提示，其中 `side-panel` 页把每个面板列成勾选框
  `[C:lib/settings.json]`、`[R]`。

### 3.8 会话与工作区

`/resume`、`/home`、`/agentview`、`/bg` 与 `⌸` 背后是同一个会话管理器：工作区轨道、
带标题与状态/时间的会话行、过滤、`★` 固定；它聚焦当前工作区最近用过的会话，后台
预取列表，读到结果前先画上一次成功的列表，并在读取期间保持手动选择不动 `[R]`。
后台会话（`/bg`）在进程内继续跑，TUI 退出时停止 `[R]`。`/fork` 连历史复制会话并
记录父会话；`/rewind` 回到较早的节点；`/tree` 浏览分叉树；`/export` 导出
transcript `[R]`。

### 3.9 鼠标与选择（仅全屏）

`[R]`：拖选并复制、双击/三击选词/选行、点工具卡、点时间轴刻度、点 `[Image #N]`
预览、滚轮滚动、拖动面板分隔条改宽度、点面板条、点模型选择器行；悬停显示面板名
与被截断的目标文本；浮层把背景压暗并遮住其下文字；自动复制后选区保留到 `Esc`。

协议健壮性被当成功能：分片的 SGR 鼠标报告只在鼠标上报开启期间缓存并拼回，完整报告
立即当作事件消费，缓存有上限（1 秒、64 字节），所以字面输入只会延迟不会被丢弃
`[R]`。

### 3.10 渲染附加项

256 色 / 真彩色主题并可热重载，尊重 `NO_COLOR`/`FORCE_COLOR`，Apple Terminal 与
tmux 共用一套回退 `[R]`；BiDi 文本支持 `[C:package.json]`；语义复制（工具卡复制的
是内容而不是带装饰的行）`[R]`。

### 3.11 通知与终端标题

任务完成只在「成功、无排队后续」时通知；标题 `dsh-TUI · Task complete`；正文是最近
一条稳定回复压成一行（≤200 字符）；OSC 777/9/99 → Windows toast → BEL 阶梯，
`unfocused`/`always`/`off`；回放、取消、失败都不通知 `[R]`。终端标题带流式标记 `[R]`。

### 3.12 扩展与 agent 集成

Cordis 插件可贡献面板、命令、场景与信息流 `[R]`；agent 预设（含「只有一个工具」的
minimal 预设）、技能、MCP 服务、目标与待办、子代理团队、问卷与审批、
browser/computer-use 扩展、Claude/Codex kernel、OAuth 账号 `[R]`。

### 3.13 长会话工程

会话日志是唯一真相；UI 是事件驱动投影，按可见窗口渲染（O(visible window)），带
虚拟化与有界缓存 `[R]`。这一条决定工作台能否撑过五小时会话，它是**渲染架构**的
主张，不是功能。

## 4. 八条体验能力

功能清单不可移植，这八条可以。它们是「功能实现方式不同，但这件事情必须保住」：

1. **三层活信号，静止时真的静止。** 页眉（谁/在做什么/在哪）、activity 行（此刻在
   做什么，带用时与速率）、状态栏（已经发生了什么，代价多少）。空闲就是**不动**：
   空闲或失焦时动画暂停 `[R]`、`[S:dsh_chat]`。
2. **鼠标能覆盖常用的 80%。** 滚聊天、切面板、展开工具卡、点时间轴回合、聚焦输入框、
   拖分隔条、点模型行。每一条都有命中区，每个命中区都有可见的提示 `[R]`、
   `[S:dsh_panel]`、`[S:dsh_model]`。
3. **一个界面，没有模式迷宫。** 启动台→聊天是同一个输入框、同一套补全；浮层可叠加，
   `Esc` 先关最内层，**再**考虑中断正在跑的回合 `[R]`。
4. **只改显示、不改数据的渐进展开。** `ctrl+o` 展开单条、`ctrl+d` 折叠连续调用、
   `ctrl+q` 折叠待办、目标页脚空闲时隐藏并按下一个快照回来；隐藏只是显示 `[R]`。
5. **诚实的度量。** 占用来自后端，成本含子代理，未定价模型明说而不是显示 ¥0，
   缓存命中率与 TPS 是量出来的而不是暗示的 `[R]`、`[S:dsh_chat]`。
6. **降级是阶梯，不是断崖。** 宽：聊天 + 符号面板栏；窄：面板变全屏；矮：先丢模型
   描述，再丢相邻行，再丢强度行。中文永不挪列 `[R]`、`[S:dsh_chat]`。
7. **输入区连续性。** 草稿、光标、附件在启动、切换、浮层与全屏编辑器之间都在；
   没有任何东西会自己发出去 `[R]`。
8. **不用读文档也能发现。** `?` 是两栏答案，每条命令在任何出现处都带描述，补全带
   提示，设置分组可搜索，界面双语 `[R]`、`[S:dsh_help]`、`[S:dsh_setup1]`。

## 5. FutureOS `future-tui` 现状（证据）

`future tui` **就是工作台**：`index::run` 把握手交互交给
`workbench::run::run` `[F:tui/src/index.rs:951]`，这个界面自己持有 raw mode、
alt screen、鼠标模式与帧时钟 `[F:tui/src/workbench/run.rs:92]`；旧内联 UI —— 它的页
与 `--legacy`/`--classic` 开关 —— 已被删除，由 `help_describes_this_build` 钉住
`[F:tui/src/help.rs:96]`。工作台树 15 个模块、9,076 行、119 个 `#[test]` `[V]`。

| 子系统 | 现在有什么 | 证据 |
|---|---|---|
| 帧 | 恰好 `h` 行的一整帧：页眉（+细线）、主体、输入带、activity、状态、提示；浮层整张覆盖 | `[F:tui/src/workbench/screen.rs:29]`、`[S:workbench-chat.png]` |
| 终端会话 | alt screen、括号粘贴、Kitty 键盘、鼠标模式随 alt screen 一起写入并在 stop/崩溃时写回 | `[F:tui/src/terminal.rs:233]`、`[F:tui/src/workbench/run.rs:92]`、`[V]`（运行中 `mouse_any_flag=1`，退出后 `0`） |
| 帧字节 | 每行都用绝对定位（`CSI row;1H`），帧从不让光标前进；底行换行会把 alt screen 顶上去一行、所有点击整体错一格 | `[F:tui/src/workbench/run.rs:368]` |
| 鼠标 | SGR 1006 解析（`Press/Release/Drag/Move/WheelUp/WheelDown`），绘制与命中共用一张 hit map；可点输入框/聊天/分隔条/页签/行/选择器行；聊天与聚焦面板可滚轮；分隔条可拖（边缘跟着指针走，`30..48` 钳住）；选择器点行即应用；只开 `?1000h?1002h?1006h`（不开 hover） | `[F:tui/src/workbench/mouse.rs:56]`、`[F:tui/src/workbench/mod.rs:534]`、`[V]`（点 `用量` 切了面板）、`[S:workbench-panel-usage.png]`、`[S:workbench-drag-fixed.png]` |
| 布局 | `NARROW_WIDTH = 88`、`SMALL_HEIGHT = 24`、`SIDEBAR_DEFAULT/MIN/MAX = 36/30/48`、聊天左右各 2 格内边距；绘制与命中用同一份 `Metrics` | `[F:tui/src/workbench/layout.rs:65]` |
| 侧栏 | 五个页签 `待办 任务 会话 用量 技能`，≥88 列常驻，以下 `ctrl+b` 开全屏面板；分隔条拖 30..48 并在松手后持久化 | `[F:tui/src/workbench/panels.rs:18]`、`[S:workbench-panel-full-60x20.png]` |
| 面板数据 | 待办读 `events.jsonl`（✔/◉/○/✖ + 优先级）、技能读 `get_state` + 两个技能目录、任务读只读 `tasks.db`、会话读 `list_sessions`、用量读 `get_state` | `[F:tui/src/workbench/local.rs:92]`、`[F:tui/src/workbench/live.rs:201]`、`[S:workbench-panel-todo.png]`、`[S:workbench-panel-skills.png]` |
| 启动台 | 鲸鱼 + 方块字标 + 主张句、带框输入框、参数 chip（模型/强度/模式/权限）、提示行、最近会话 | `[F:tui/src/workbench/launchpad.rs]`、`[S:workbench-launchpad.png]` |
| 状态 / activity | `模型 · 强度 · 权限 | 进度条 + ctx 用量/总量 + % · ≈¥`；activity 为 `转轮 + 动词 + 用时 + 工具数 + tok/s`，另有 `缓存 N%` | `[F:tui/src/workbench/screen.rs:468]`、`[F:tui/src/workbench/screen.rs:430]`、`[S:workbench-chat.png]` |
| 主题 | 六套 256 色调色板（`dark`、`light`、`one-dark`、`one-light`、`high-contrast`、`dracula`），沿用内联目录的 id，因为 `themeId` 是两个界面共用的一把键；`/theme` 打开选择器，高亮移动即预览，`esc` 恢复，只有 `enter` 持久化 | `[F:tui/src/workbench/theme.rs:54]`、`[F:tui/src/workbench/picker.rs:312]`、`[S:workbench-picker-theme.png]`、`[S:workbench-light-120x38.png]` |
| 选择器 | `/model`、`/theme`、`/sessions` 共用同一张浮层：搜索行、分组标题、当前值上的 `✓`、模型选择器的强度滑杆，点击行会**同时**移动高亮并应用 | `[F:tui/src/workbench/picker.rs]`、`[F:tui/src/workbench/screen.rs:721]`、`[S:workbench-picker-model.png]`、`[S:workbench-picker-sessions.png]` |
| 命令面板 / 帮助 | `/` 过滤 12 条带描述的命令；`/model`、`/theme`、`/sessions` 打开各自的选择器，另有五条仍无处理函数；`?` 键位表；两者都整张覆盖帧 | `[F:tui/src/workbench/mod.rs:904]`、`[F:tui/src/workbench/run.rs:249]`、`[S:workbench-palette.png]`、`[S:workbench-help.png]` |
| 审批 | 挂起的审批接管 `y`/`n`；activity 行写出工具名与摘要 | `[F:tui/src/workbench/run.rs:189]`、`[F:tui/src/workbench/live.rs:586]` |
| Transcript | 复用现有 markdown/工具卡/diff 组件的渲染行；滚到顶部触发分页 `LoadOlder` | `[F:tui/src/workbench/live.rs:266]`、`[S:workbench-chat.png]` |
| 打印模式 | `future tui -p` 不变（非交互） | `[F:tui/src/index.rs:617]` |

**工作台没有继承下来的东西。** 删内联 UI 时，一些屏幕的替代品并不是面板：
`/sandbox`、`/worktree`、`/keymap`、`/copy`、外部编辑器，在工作台里
**根本到不了** `[F:tui/src/workbench/live.rs:694]`（`command()` 只认
`quit/new/sessions/model/thinking/reply-detail/refresh/help/export` 加启动台 chip；
三个列表动词在更早的 `run.rs:249` 被截走，变成选择器；指界面上东西的动词由 surface
自己答）。另有五条面板命令 —— `/workspace`、`/usage`、`/skills`、`/export`、
`/transcript` —— 以前写在面板里、按下去却回 `Unknown command` `[V]`；现在五条都能
落地（`[S:workbench-usage-panel.png]`、`[S:workbench-skills-panel.png]`），
`every_palette_entry_has_a_handler` 挡住下一条走上老路。那曾是相对内联 UI 的
**回退**，也是改造清单里的 P0-3。

## 6. 差距矩阵

判定：**missing**（无对应物）、**partial**（有界面但没有这项能力）、**equivalent**、
**ahead**（FutureOS 做得更多）、**n/a**（本产品没有该数据源，对齐不是正确目标）。
`now` 列是工作台落地后的判定，`was` 是第一版（`9d5f02beb`）的判定。

| # | 能力 | dsh-TUI 0.17 | `future-tui` 现在 | now | was | 级别 |
|---|---|---|---|---|---|---|
| 1 | 分层全屏帧 | 有 `[S:dsh_chat]` | 页眉/主体/侧栏/输入/activity/状态/提示 `[S:workbench-chat.png]` | equivalent | missing | — |
| 2 | 鼠标点击 / 滚轮 / 拖拽改宽 | SGR 1006，处处命中区 `[R]` | 点击+滚轮+拖拽都通了；**拖拽方向反了**，无悬停 `[V]` | partial | missing | P0 |
| 3 | 悬停提示与 tooltip | 悬停显示面板名与目标文本 `[R]` | `Move` 解析后被丢弃，从不写 `?1003h` | missing | missing | P1 |
| 4 | 带真实输入框的启动台 | 有 `[S:dsh_launchpad]` | `[S:workbench-launchpad.png]` | equivalent | missing | — |
| 5 | 首次向导，可重跑 | 四步，`/setup` | 无 | missing | missing | P1 |
| 6 | 常驻侧栏 + 可拖分隔 | 8 个面板 `[R]` | 5 个面板，拖拽可用但方向反 | partial | missing | P0 |
| 7 | 实时目标/待办面板 | `todo` 面板 + 折叠 | 真实目标待办，带状态字形与优先级 `[S:workbench-panel-todo.png]` | equivalent | missing | — |
| 8 | 状态栏：分支、缓存率、TPS、标题 | 有 `[S:dsh_chat]` | 模型·强度·权限、进度条+ctx、成本、`缓存 N%`；**分支从未读取**（`branch: String::new()`），TPS 仅运行中 | partial | partial | P0 |
| 9 | Activity 行（动词+用时+工具+速率） | ActivityLine `[R]` | `转轮 + 动词 + 用时 + 工具数 + tok/s`，空闲静止 | equivalent | partial | — |
| 10 | 覆盖全部命令的可过滤面板 | `/` 浮层 + `?` 两栏 | `/` 过滤 12 条带描述；`/model`、`/theme`、`/sessions` 打开选择器；另有 5 条仍回 `Unknown command` `[V]` | partial | partial | P0 |
| 11 | 256 色主题套件 | 主题 + 热切换 `[R]` | 六套调色板，`/theme` 选择器实时预览、`esc` 恢复、`enter` 持久化 `[S:workbench-picker-theme.png]`、`[S:workbench-light-120x38.png]` | equivalent | equivalent | — |
| 12 | 带构成配色的上下文条 | 有 `[R]` | 单色强调色填充 | partial | partial | P1 |
| 13 | 可点击时间轴轨道 | 有 `[R]` | 无 | missing | missing | P1 |
| 14 | 左右两栏 diff，单侧占满 | 有 `[R]` | 单栏 unified diff | partial | partial | P1 |
| 15 | 会话管理器（轨道、固定、过滤、预取） | 有 `[R]` | 会话面板 + `ctrl+k` 刷新；`/sessions` 选择器带搜索、点行即切换；无固定、无工作区轨道 | partial | partial | P1 |
| 16 | 分组可搜索的设置页 | 9 组 | 没有设置页；选择器覆盖模型、强度与主题，其余靠零散命令 | partial | partial | P1 |
| 17 | 全屏草稿编辑器 | `Ctrl+Shift+E`、`⛶` | 无入口、无编辑器；外部 `$EDITOR` 也到不了 | missing | partial | P1 |
| 18 | 按项目的历史搜索 | `Ctrl+R` | 只有提示词召回 | partial | partial | P1 |
| 19 | 转向 vs 排队 vs 中断 | `Enter`/`Tab`/`Ctrl+Enter` | 提交即排到运行之后；`ctrl+x` 中断 | partial | partial | P1 |
| 20 | 回到较早回合 | 双击 `Esc` + `/rewind` | `/fork` 也不在面板里 | missing | missing | P2 |
| 21 | Mermaid、LaTeX | Unicode + 图形 | 无 | missing | missing | P2 |
| 22 | 图片预览缩放/平移、Sixel | 有 | 仅 Kitty/iTerm2 直通 | partial | partial | P2 |
| 23 | 中英双语 | 有 `[S:dsh_setup3]` | 只有中文界面 | partial | missing | P2 |
| 24 | 拖选复制、双击/三击 | 有 | 无；鼠标被工作台吃掉，终端选择要按修饰键 | missing | partial | P2 |
| 25 | 后台会话（`/bg`） | 有 | 一次一个会话，另有排队运行 | n/a | n/a | 不做 |
| 26 | 后台 shell 作业面板 | 有 | 线协议没有 shell 作业概念；「任务」面板展示的是**定时任务**（`tasks.db`） | n/a | n/a | 不做 |
| 27 | 子代理/团队面板 | `agents` 面板 | agent 没有子代理，proto 也没有子代理消息 `[P]` | n/a | n/a | 不做 |
| 28 | 插件面板 | Cordis 注册表 | 扩展单位是技能；没有面板 API | n/a | n/a | 不做 |
| 29 | 通知与终端标题 | 有 `[R]` | `notifications.rs` 在，但工作台没有调用 | partial | equivalent | P1 |
| 30 | 流式 Markdown、工具卡、思考折叠 | 有 | 复用现有组件 | equivalent | equivalent | — |
| 31 | `@` 文件补全、图片粘贴 | 有 | 复用现有组件 | equivalent | equivalent | — |
| 32 | 中文安全宽度算术 | 有 | `display_width` / `truncate_to_width`，新代码全部使用 | equivalent | equivalent | — |
| 33 | 目标控制面 | 只有 goal 工具 | `future-goal` crate + `future goal` CLI，并在待办面板露出 | ahead | ahead | — |
| 34 | Worktree 集成 | 无 | `/worktree` 代码在，但**工作台到不了** | partial | ahead | P1 |
| 35 | 技能面 | `/skills` | 面板带描述，读两个根 | ahead | ahead | — |
| 36 | 沙箱/审批策略面 | 权限模式 + 审批卡 | 档位可读可循环（chip）、审批可答；没有 `/sandbox` 页 | partial | equivalent | P1 |
| 37 | 无头 / 打印模式 | 未记载 | `future tui -p` | ahead | ahead | — |
| 38 | 隔离实例（自己的 home 与 socket） | 按后端的存储目录 | `FUTURE_HOME` 隔离含 IPC 端点（本文每张截图都是这么来的） | ahead | ahead | — |

现在的计数：**P0 = 3**（行 2、6、8）、**P1 = 12**（3、5、12–19、29、34、36）、
**P2 = 5**（20–24）、**不做 = 4**（25–28）、**equivalent = 8**（1、4、7、9、11、
30、31、32）、**ahead = 5**（33、35、37、38，以及收敛前的 34）。

选择器浮层（`tui/src/workbench/picker.rs`，843 行）是在第一轮之后落地的，它推进了
第 10、11、15、16 行但没有关闭任何一行：命令面现在有三个真菜单（模型带强度滑杆、
主题带实时预览、会话带搜索与点行切换），调色板从两套变六套。五条死命令、会话固定、
工作区轨道和设置页仍然开着。

`was → now` 两列给出的教训是：工作台一轮就补上了**帧**的差距（1、4、7、9 行），
剩下的几乎全是**帧里面的行为** —— 一个拖拽方向、一个没读的分支、五条回
`Unknown command` 的命令、悬停、向导、设置页。这比第一轮便宜得多，也正是改造清单
排序的依据。

## 7. gRPC 影响

约束是绝对的，并且守住了：**不改 `packages/rpc/proto/future.proto`、不新增 agent
方法、不新增事件。** 工作台读的都是内联 UI 早就读的东西。

### 7.1 `get_state`（`SessionState`）已经提供的 `[P]`

| 工作台需要 | 字段 |
|---|---|
| 模型、强度 | `model` (1)、`thinking_level` (2) |
| 忙 / 压缩中（activity、转轮） | `is_streaming` (3)、`is_compacting` (4) |
| 会话 id、标题、显式/自动 | `session_id` (8)、`session_name` (9)、`explicit_session` (10) |
| 自动压缩标记 | `auto_compaction_enabled` (11) |
| 查询数、排队提示 | `query_count` (12)、`pending_message_count` (13)、`queued_runs` (34)、`queued_count` (35) |
| cwd（页眉、状态栏） | `cwd` (15) |
| 技能列表（侧栏） | `skills` (16) |
| 上下文条：已用/窗口/百分比 | `context_tokens` (19)、`context_window` (20)、`context_percent` (21) |
| 权限档位、沙箱档位（chip、状态栏） | `permission_level` (28)、`sandbox_tier` (41) |
| 输入/输出 token 与缓存读写（缓存率） | `usage.input_tokens`、`usage.output_tokens`、`usage.cache_read_tokens`、`usage.cache_write_tokens`（40 → 1–4） |
| 成本、成本拆分、「无定价」判定 | `usage.cost_cny`、`usage.cost_input_cny` … `cost_cache_write_cny`（40 → 5–9） |
| 活动运行、被中断运行 | `active_run` (33)、`interrupted_run` (36) |
| 审批（y/n） | `pending_approvals` (39) |
| 日志健康状况（状态栏告警） | `event_journal_error` (45) |
| agent 版本/实例（页眉状态点） | `version` (14)、`agent_instance_id` (29) |

### 7.2 其他既有命令提供 `[P]`

| 需要 | 命令 |
|---|---|
| 会话行（标题、模型、cwd、更新时间、首条消息、查询数、是否流式） | `list_sessions` → `SessionSummary` `[F:tui/src/workbench/live.rs:220]` |
| transcript 重放 / 上滚取更早 | `get_session_entries`（先取尾页，再 `LoadOlder`）`[F:tui/src/workbench/live.rs:266]` |
| `/model` 循环用的模型目录 | `list_models` |
| 审批决定 | `approval_decision` `[F:tui/src/workbench/live.rs:843]` |

### 7.3 线协议上没有的 —— 以及选定的本地回退

| 需要 | 状态 | 回退（不改 proto） |
|---|---|---|
| 状态栏的 git 分支 | RPC 没有 | crate 里已有 `GitCli::branch()`（`rev-parse --abbrev-ref HEAD`，带超时、只读），但工作台把 `SessionMeta::branch` 留空 `[F:tui/src/workbench/live.rs:553]` —— 见 P0-4 |
| 目标与待办 | 没有（目标控制面是本地状态） | 依次找 `FUTURE_GOAL_ROOT`、最近的祖先 `.future/goal`、git toplevel —— 所以 `.worktrees/<name>` 里的会话读到它被切出来的那份检出 `[F:tui/src/workbench/local.rs:42]` |
| 定时任务行 | 没有 | `tasks.db` 以 `SQLITE_OPEN_READ_ONLY` 打开；库不存在就是「没有定时任务」，绝不新建 `[F:tui/src/workbench/local.rs:403]` |
| 技能描述 | 部分（`get_state` 只给名字） | 从两个技能目录读 `description:` 行 `[F:tui/src/workbench/local.rs:306]` |
| TPS（token/秒） | 没有 | 客户端按短窗口内的 usage 增量推导，且只在运行中给出 `[F:tui/src/workbench/live.rs:457]` |
| 会话列表每行的成本 | 没有（`SessionSummary` 无 usage） | 行里给模型 + 时长 + 查询数；成本只给当前会话 |
| 子代理列表 | 没有，且 agent 根本没有子代理 | 不做（§8） |
| MCP 服务 | 没有（`future mcp` CLI 拥有 `~/.future/mcp.json`） | 不做 |

### 7.4 确实需要改 proto 的部分（记录在案，不请求）

只有两个愿望真的被卡住：**`list_sessions` 里的每会话用量**（可让会话列表按成本排序）
和**任何子代理/作业面**（需要 agent 侧先有这个概念）。两者都是「P2 或永不」，在本
目标下都不允许膨胀成线协议改动。

## 8. FutureOS 不该照抄 dsh 的地方

1. **不要子代理/作业面板。** agent 里没有子代理概念，线上也没有可显示的东西。永远
   空着的面板是对能力撒谎。
2. **不要插件/面板 API。** Cordis 注册表是一整套扩展模型；FutureOS 的扩展单位是技能，
   而技能已经有面板落点。
3. **不要 kernel 切换（`/dsh` `/claude` `/codex`）。** 后端就是这个仓库的 agent。
   有用的映射是 `/providers` + `/models`。
4. **不要第二套会话模型。** `/bg` 后台会话会与 agent 自己的队列和会话生命周期重复；
   工作台应当**露出** `queued_runs`，而不是发明一套平行的「后台」。
5. **别把已经更好的东西丢掉**：`/worktree`、`/sandbox` 档位、技能安装、目标控制面、
   打印模式、`FUTURE_HOME` 隔离。工作台是它们的壳，不能顺手把壳做成坟 —— 而现在
   `/worktree` 与 `/sandbox` 确实**丢了**（见 §5、P0-3/P1）。

## 9. 验证方法与剩余未知

如何重新推导本文：

1. 读钉住的包：`~/.dsh/profiles/dsh-tui/package.json`（版本）与
   `…/node_modules/@deepseek-harness-tui/dsh-tui/README.md`（能力）。
2. 重抓 dsh 屏幕：tmux 里跑 `dsh-tui`，用按键驱动，`tmux capture-pane -e -p` 存成
   `shots/raw_<name>.txt`，再把同一缓冲区渲染成 `shots/dsh_<name>.png`。
3. 重抓 FutureOS 屏幕：`tui/scripts/shot.sh`（完整配方见 `verification.md` §7）——
   隔离 `FUTURE_HOME`、隔离 agent 端口、尺寸写在第二个参数（`120x38`）。它在那个尺寸
   的 pty 里跑工作台并把它写出的字节渲染成图；不用 tmux，也不用 `ansi2png.py`。
4. 用 `[F:…]` 标签核验 FutureOS 的结论；真正要 grep 的是 `?100`（鼠标）、
   `sidebar`、`goal_todos`、`branch`。

刻意没有猜的未知：

- **未测**：鼠标延迟/手感、悬停时序、本终端的通知投递，以及 dsh 的 activity 行在
  *嵌套*工具运行时的行为。
- **未测**：dsh 在 90×24 下怎么降级 —— 每一张 dsh 抓帧都是 120×38。它的降级阶梯
  （§4.6）是 `[R]` 级证据；FutureOS 的阶梯是在 120×38 / 90×24 / 60×20 实测的
  `[S:workbench-90x24.png]`、`[S:workbench-60x20.png]`。
- **未核**：dsh 每个面板在 28 列宽里会不会裁切；§3.5 的 `minColumns` 是
  `[C:…/builtinPanels.js]` 的结构事实，不是渲染测试。
- **新增**：60 列时状态栏左侧标签被 `…` 截断（`future/deepse…`），而所有数字保持完整
  `[V]`。这是有意的（进度条先布局），但一眼看去像是另一个模型名；等模型名有了短别名
  时值得回头处理。

## 10. 开放问题 —— 以及第一版三个问题是怎么被回答的

1. ~~默认开启，还是藏在 `--layout workbench` 后面？~~ **以发布作答**：`future tui`
   就是工作台，没有开关。`--layout`、`--legacy`、`--classic` 都不存在，
   任何一个回来 `help_describes_this_build` 就会失败 `[F:tui/src/help.rs:96]`。
   第一版说的「内联回退」是被**删除**而不是保留：两个渲染器共享一条输出流，正是被
   否决的那个方案。
2. ~~待办面板可以直接读 `<cwd>/.future/goal` 吗？~~ **以使用作答**：直接读、不启进程、
   best-effort，且按**会话** cwd 解析根目录，所以 worktree 里的会话读自己的检出
   `[F:tui/src/workbench/local.rs:42]`。
3. ~~只有英文的启动台是不是坏的第一印象？~~ 仍然开放，而且更尖锐了：整个工作台界面
   只有中文 `[S:workbench-launchpad.png]`，所以第 23 行是产品决定，不是翻译活。

给主管的新问题：

4. 五条死命令是**接线**（`/usage` → 聚焦「用量」面板、`/skills` → 「技能」、
   `/workspace` → cwd）还是**先摘掉**？一条回 `Unknown command` 的命令，比一条不存在
   的命令更伤信任。
5. 分隔条拖拽是否按一行符号位修正（P0-1，让边跟着指针走）？现在这个反向行为在任何
   地方是有意的吗？
