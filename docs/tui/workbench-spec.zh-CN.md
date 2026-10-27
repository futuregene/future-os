# FutureOS Workbench TUI — 设计与实现规格 v1（冻结合同）

> 目标：在现有 `future-tui`（Rust、自定义 ANSI 渲染、gRPC 客户端）之上，新增一套
> **全屏工作台（workbench）** 界面，体验对齐 dsh-TUI 0.17 的核心能力，并针对
> FutureOS（goal / 多会话 / 技能 / 远程）做得更好。**不修改 agent 的 gRPC 接口**。

本文件是 worker 之间的冻结契约：类型名、文件名、布局常量、键位、验收命令不得
各行其是。实现可以增补，但不得改语义。

---

## 1. 范围

### P0（本次必须落地，可截图验收）

1. 全屏布局：Header / Chat / 侧栏 / 输入框 / 活动行 / 状态栏 / 提示行。
2. 鼠标：点击（切面板、切页签、点行、聚焦输入、点按钮）、滚轮（聊天/侧栏）、
   拖拽（分栏分隔条）、悬停高亮（可选，能力允许时）。
3. Launchpad 首屏：大字 logo、大输入框、参数行（工作区/模型/强度/审批，可点击）、
   快捷入口、最近会话。
4. 侧栏 2 个真实面板：最近（会话名、所在工作区、刷新时间）、工作区（同一批会话按桌面端的方式归档：置顶 / 工作区 / 对话会话）。
   第三个 **文件** 浏览会话工作区的目录树：一次一层、目录**打开时才读**，`↑`/`↓` 走行，
   `enter` 在目录上展开、在文件上预览，`⌫` 收起目录（否则跳到它所在的目录）。`←`/`→` 三个页签上
   都是切页签。它的行与其它面板一样注册进 `HitMap`，所以点哪行就开哪行。
5. 状态栏真实指标：模型 · 强度 · cwd · git 分支 · 上下文占用条 · token · 费用/缓存。
6. `/` 命令面板（带说明、Tab 补全）、`?` 帮助覆盖层、`/` 过滤、历史 ↑↓。
7. 主题：内置六套 256 色——`dark` / `light` / `one-dark` / `one-light` /
   `high-contrast` / `dracula`——由 `/theme` 选择；选择器会把高亮所在的那套直接预览到整个界面。
   id 就是行内 TUI 的目录（`crate::themes::THEME_SPECS`），因为 `settings.json` → `themeId`
   两块界面共用同一个键。
8. **它就是终端界面，也就是 `future tui`**：workbench 有自己的全屏渲染循环，`future tui`
   启动它。旧的行内界面是**删除**，不是弃用——代码、页面和 `--legacy` 开关都没了。它也不是
   `--layout` 开关：两个渲染器共用一条输出流的做法被否掉了。

### P1（本次只做骨架/文档，不要求完成）

时间线轨（turn dots 点击跳转）、diff 双栏卡、todo 折叠页脚、任务完成通知、
图片预览、会话管理器全屏版、`Ctrl+O` 详细/折叠、全屏草稿编辑器。

### 非目标

- 不改 `packages/rpc` / proto / agent 的 gRPC 方法签名。
- 不引入 ratatui / crossterm / tui-rs；沿用现有自研终端后端。
- 不重写 markdown / diff / 输入组件的渲染逻辑，复用它们产出的 ANSI 行。

---

## 2. 文件布局（写作用域）

```
tui/src/workbench/mod.rs        // 模块导出 + Workbench 状态机 + WbEvent/WbAction
tui/src/workbench/ansi.rs       // 宽度/截断/拼接/填充（CJK 安全）
tui/src/workbench/theme.rs      // Palette、六套主题、语义色
tui/src/workbench/widgets.rs    // 边框、页签、进度条、徽标、像素 logo、spinner
tui/src/workbench/layout.rs     // Rect、Panes、命中区域（HitMap）
tui/src/workbench/mouse.rs      // SGR 1006 解析、MouseEvent、滚轮/拖拽状态机
tui/src/workbench/launchpad.rs  // 首屏
tui/src/workbench/screen.rs     // 全屏合成：header/body/sidebar/input/status
tui/src/workbench/panels.rs     // 侧栏 2 面板的渲染（纯函数，输入同一个行模型）
tui/src/workbench/picker.rs     // /model、/thinking、/mode、/workspace、/theme、/sessions、/lang 的选择器覆盖层
tui/src/workbench/files.rs      // 文件面板的目录树，以及一个路径能预览成什么
tui/src/workbench/image_view.rs // 图片转半格：适配、缩放、平移
tui/src/workbench/state.rs      // WorkbenchData / TodoItem / SessionRow / UsageData
tui/src/workbench/demo.rs       // 演示数据（截图/自测用，不连 agent）
tui/examples/workbench_demo.rs  // 打印任意尺寸的一帧 + 鼠标自测
tui/src/lib.rs                  // 仅加一行 `pub mod workbench;`
```

`tui/src/workbench/run.rs` 是驱动：它拥有 raw mode / alternate screen / 事件循环。
`index.rs` 只做参数解析（print mode、`--help`、`--list-models`），交互式一律交给驱动；
不存在第二个界面。

live 接入只允许改：`tui/src/workbench/live.rs`、`tui/src/workbench/state.rs` 的填充函数、
以及 `run.rs` 里「需要先问 agent 的键」那一段。**不得改** `tui/src/components/**` 的既有行为
（只复用它们产出的行）。

---

图里的文案是中文那一套；界面中英各有一套，按读者的语言绘制（`i18n.rs`、`/lang`）。
内容——对话、会话标题、路径——不翻译。

## 3. 布局（单位：终端单元格）

> **当前交互修订**：头部固定为两行（上下文 + 分隔线），不再有可选的品牌头部——
> `/preferences`（`ctrl+o`）只保留侧栏左/右位置与宽度等选择；这些选择和显示状态
> 持久化在 `settings.json`。默认快捷键新增 `ctrl+p` 命令面板、`ctrl+r` 会话、`alt+h` 帮助
> （`?` 也可），`keybindings.json` 的动作 id 映射可重映射，帮助和提示行展示当前实际键位。
> 帮助浮层按用途分组，默认里没有功能键。本文件下面画的是早期的六行品牌头部；
> 现在头部只占两行（上下文 + 分隔线），其他分区不变。

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ row 0..4   HEADER (5)                                                        │
│   x0..     FUTURE OS  (bold, accent)  ·  workbench                           │
│            session title (dim) · 分支/工作区                                  │
│   right    ⌘ future-tui vX · 后端状态点 (●connected)                          │
│ row 4      细分隔线 (dim, ─)                                                  │
├───────────────────────────────────────────────┬──────────────────────────────┤
│ rows 6..h-4   BODY                            │ SIDEBAR_W = 36 (30..48)      │
│   chat 区域宽 = W - SIDEBAR_W - 1             │ row 6 页签条: 最近 工作区    │
│   x = SIDEBAR_W+1 .. W-1                      │        用量 技能            │
│   左右各留 2 列内边距                          │ 分隔列 x = SIDEBAR_W (│)     │
│                                               │ …侧栏一直延伸到输入区旁边   │
│ rows h-6..h-4  INPUT (3): 上边框 / 提示行 / 下边框（在侧栏右边）              │
│   ❯ <draft>                                   右侧 ⛶                       │
├───────────────────────────────────────────────┴──────────────────────────────┤
│ row h-3        ACTIVITY: ⠋ 思考 3s · 2 工具 · 42 tok/s · 缓存 96%            │
│ row h-2        STATUS: model · effort · ctx bar 6.9k/1.0M 0.7% · ≈¥0.01      │
│ row h-1        HINT: Esc 聊天 · ctrl+b 侧栏 · / 命令 · ? 帮助                │
└──────────────────────────────────────────────────────────────────────────────┘
```

- 宽度 < 88：侧栏折叠为 0；`ctrl+b` 打开全屏覆盖面板；分隔列变为 2 列时间线轨。
- 高度 < 24：HEADER 压成 1 行（wordmark 同行），INPUT 压成 2 行，隐藏 ACTIVITY。
- chat 内容从底部向上排列（最新在输入框上方），向上滚动时顶部出现 `↑ 更早` 徽标。
- 所有宽度计算必须用 `ansi::display_width`（CJK/emoji 记 2）。

### 视觉规格（256 色）

| 语义 | dark | light | one-dark | one-light | high-contrast | dracula | 用途 |
|---|---|---|---|---|---|---|---|
| bg | 233 | 254 | 236 | 231 | 16 | 236 | 画布 |
| panel | 235 | 252 | 237 | 254 | 17 | 237 | 侧栏/菜单底色（输入框用画布底色） |
| border | 238 | 246 | 240 | 250 | 250 | 239 | 边框、分隔 |
| fg | 252 | 236 | 249 | 236 | 231 | 255 | 正文 |
| muted | 245 | 243 | 243 | 245 | 246 | 243 | 次要说明 |
| accent | 80 (teal) | 30 | 75 | 25 | 51 | 141 | 品牌、选中、进度 |
| iris | 111 | 62 | 73 | 30 | 123 | 117 | 链接、页签激活 |
| amber | 179 | 130 | 180 | 130 | 226 | 228 | 思考、警告 |
| green | 114 | 28 | 108 | 28 | 46 | 84 | 成功 |
| red | 203 | 124 | 168 | 160 | 196 | 203 | 错误、危险 |
| purple | 141 | 91 | 176 | 91 | 213 | 212 | 子代理/技能 |

后四套在行内 `Theme` 有同名语义的位置上与 `crate::themes` 一致（`accent`、`fg`、`border`、
`dim`、`error`、`success`、`md_link`、`md_heading`、`md_code`）；`bg` 与 `panel` 是
workbench 自己的——全屏画布必须自己画一层背景，而行内主题是继承终端背景的。早期
workbench 写下的 id（`future-dark`、`future-light`）仍能解析到 `dark` / `light`。

TUI 不使用 logo 或图标。聊天页眉只有一行文字品牌 `FUTURE OS`，后面接会话标题与
工作区，下面是细分隔线；启动台在对话栏中居中显示两行的宽字距
名称和主张句，让输入框成为视觉焦点。

---

## 4. 交互（鼠标 + 键盘）

### 鼠标（SGR 1006；启用 `?1000h ?1002h ?1006h`，可选 `?1003h` 悬停）

| 目标 | 行为 |
|---|---|
| 聊天区滚轮 | 上/下 3 行；到顶触发 `LoadOlder` |
| 侧栏滚轮 | 当前面板列表滚动 |
| 侧栏页签点击 | 切换面板（`待办/任务/会话/用量/技能`） |
| 侧栏行点击 | 会话→切换；待办→折叠/展开；技能→插入 `/skill` |
| 分隔列拖动 | 调整 `SIDEBAR_W`（30..48），松手持久化到 settings |
| 输入框点击 | 聚焦并把光标放到点击处（按显示宽度换算字节偏移） |
| `⛶` 点击 | 打开全屏草稿编辑器（P1 可 stub） |
| 工具卡标题点击 | 展开/折叠该卡片（复用 `ctrl+o` 的单项切换） |
| Launchpad 参数段点击 | 打开对应选择器（工作区/模型/强度/审批） |
| Launchpad 快捷入口点击 | 执行对应 `/command` |
| 悬停 | 页签/行/按钮高亮；600ms 后显示 tooltip（窄终端截断时） |

双击 = 两次 <400ms 同点；三击=行选择（P1）。

### 键盘

沿用现有全局键位；新增/覆盖：
`ctrl+m` 模型选择器；`ctrl+t` 思考强度选择器；`ctrl+s` 审批模式选择器；`ctrl+l` 工作区选择器；
`ctrl+1` / `ctrl+2` / `ctrl+3` 侧栏三个页签；`ctrl+v` 读剪贴板（图片成为草稿附件，文字粘进草稿）；
`alt+v` 预览草稿里最新那张图片；
`ctrl+b` 侧栏开/聚焦/关（侧栏外沿的 `›` 按钮一键等效，收起后在**同一列**留一个 `‹` 把它叫回）；
`tab` 面板内焦点；`shift+tab` 反向；
`ctrl+q` 折叠待办；`alt+z` 放大侧栏；`/` 命令面板；`?` 帮助；`esc` 逐层关闭。

默认键位里没有功能键：`F1`–`F12` 在笔记本上要按 `Fn`，紧凑键盘上干脆没有，所以这些动作默认
用 `alt` + 字母（`alt+h` 帮助、`alt+r` 刷新、`alt+c` 复制、`alt+m` 鼠标、
`alt+a` 减少动态）。改成功能键依然支持。帮助浮层按 `ACTIONS` **分组**列出（会话 / 选择 / 面板 /
界面），然后是不可改键的行内键，最后是命令。

按住 `shift+←` / `shift+→` 只换一次：按键返回 `WbAction::SessionStep(delta)`，驱动先读完整个输入
突发，再用 `Workbench::step_session` 应用净步数。

### 选择器（`/model`、`/thinking`、`/mode`、`/workspace`、`/theme`、`/sessions`、`/lang`）

七个菜单共用同一个组件（`picker.rs`）：带标题的盒子 + 增量搜索行（`❯`）+ 分组标题
（模型按 provider 分组，顺序跟 agent 一致）+ 生效行上的 `✓` + 最后一行该菜单的键位提示。

| 键 / 手势 | 行为 |
|---|---|
| `↑` `↓` | 移动高亮，跳过分组标题 |
| `pageUp` / `pageDown` | 十行 |
| 其他字符 | 进入搜索；列表收窄，高亮回到第一行 |
| `backspace` | 收回一格搜索 |
| `←` `→` | 模型选择器的推理强度滑块（六档）；其余菜单无动作 |
| `enter` | 应用高亮行并关闭 |
| 滚轮 | 一格一行 |
| 点击行 | 同时移动高亮并应用（鼠标没有第二个 `enter`） |
| `esc` / 点盒子外 | 关闭且不应用 |

打开时高亮就在当前生效的值上，所以刚打开就 `esc` 什么都不会变。`/theme` 随高亮预览——
整个界面按那套调色板重绘——`esc` 恢复到打开时的主题，只有 `enter` 才会落盘
（`settings.json` → `themeId`）。模型选择器把模型**和**滑块档位一起提交
（`SetModel` + `SetThinking`）；单独按在滑块行上则只改强度。

`/new` 回到启动台，而不是立刻建会话：头部自己的状态（连接点、版本）留着，对话清空，
第一条消息才按启动台上显示的模型/强度建会话。

还没有会话时（启动台，或 TUI 起来时 agent 手上没有会话），选择器里的 `enter` **只暂存**，
不发请求：agent 在没有会话时对 `set_model`/`set_thinking_level` 一律回
`session not found — pass a valid session_id`，所以这一对选择变成 `SetPending*`——头部与启动台
参数段显示的就是它，同时写进 `settings.json` 的 `defaultModel`/`defaultThinkingLevel`，
第一条消息用它们建会话；会话建好后 agent 就是唯一来源，暂存值清空。启动台上点参数段同理：
工作区、模型、强度、审批模式都走暂存——`set_cwd` 和 `set_sandbox_policy` 跟另外两个 setter 一样
需要会话，所以暂存的模式在会话建好后立刻补发一次。

`/thinking`、`/mode`、`/workspace` 是三个不用打开命令面板就能到的选择器：`ctrl+t`、`ctrl+s`、
`ctrl+l` 直接打开它们。`/thinking` 列六档强度；`/mode` 列桌面端输入框的那四档审批模式（手动 /
沙箱 / 自动 / 完全访问），每行带一句解释；`/workspace` 列本客户端有会话的全部目录，并在最前面
多一行「用搜索框里的路径新建工作区」——在那一行按 `enter` 就是 `set_cwd`，和选一个已有工作区
同一个动作。审批模式就是桌面端自己的那四个值；没有单独的 TUI「权限级别」芯片，因为别的客户端
都没有这个选择。

行的绘制与命中区域来自同一份列表和同一份几何（`screen::picker_box` 用 `box_geometry`），
所以点击一定落在指针下面那一行，而不是旁边一行。

---

### 文本界面里的图片

图片用**半格**画：每格一个 `▀`，前景是一对像素的上面那个，背景是下面那个。除了颜色不需要终端做
任何事，所以任何 256 色终端都能显示（不需要 kitty graphics、iTerm2 图片或 sixel），终端上报真彩色
时用 24 位色。它是**预览**：几百列宽对照片是缩略图，对截图则足够看清。

解码在适配层（`image`，与 agent、host、桌面端同一个 crate），每个文件只做一次，长边缩到 1024 像素：
缩放重画的是手上已有的像素。文本预览按 `MAX_TEXT_PREVIEW_BYTES` 有界读取，并在字符边界上切。
读不了的文件会在框里说明原因，而不是开一个空框。

---

## 5. 类型契约
```rust
pub struct WorkbenchData {
    pub app_name: String, pub version: String,
    pub connected: bool,
    pub session: SessionMeta,          // id/title/cwd/branch
    pub model: String, pub effort: String,
    pub mode: String,                  // manual|sandbox|auto|off（桌面端那四档）
    pub usage: UsageData,              // ctx_used/ctx_total/cost/cache/tps
    pub todos: Vec<TodoItem>,          // goal todos
    pub sessions: Vec<SessionRow>,
    pub skills: Vec<SkillRow>,
    pub jobs: Vec<JobRow>,
    pub models: Vec<ModelChoice>,      // id/label/provider/context/thinking
    pub tree: FileTree,                // 文件面板：可见行 + 哪些目录开着
    pub preview: Option<PreviewView>,  // 预览框手里的东西（像素 / 文本 / 为什么不行）
    pub messages: Vec<String>,         // 已渲染 ANSI 行（由 live 层提供）
    pub input: InputView,              // text + cursor byte offset + placeholder
    pub activity: Activity,            // idle|thinking|tool|streaming + detail
    pub scroll: usize,                 // 0 = 贴底
    pub sidebar: SidebarState,         // panel + width + focus
    pub overlay: Option<Overlay>,      // palette|help|panel-full|picker(kind)
    // 选择器自身状态（kind/query/高亮/滑块）在 surface 上，不在这里：
    // 那是读者正在做什么，而不是屏幕上有什么
}

pub enum WbEvent { Key(KeyEvent), Mouse(MouseEvent), Resize(u16,u16), Tick }
pub enum WbAction {
    Submit(String), Command(String), Scroll(i32), LoadOlder,
    SwitchSession(String), SetModel(String), SetThinking(String),
    SetApproval(String), SelectWorkspace(String),
    SetTheme(String), NewSession,
    SetPanel(PanelId), ResizeSidebar(u16), FocusPane(PaneId),
    ToggleToolCall(usize), PersistSettings(SettingsPatch), Quit, Redraw,
}
```

`Workbench::render(&mut self, d:&WorkbenchData, w:u16, h:u16) -> Vec<String>`：
返回**恰好 h 行、每行显示宽度恰好 w** 的 ANSI 字符串（不足补空格）。
`Workbench::handle(&mut self, e:WbEvent, d:&WorkbenchData) -> Vec<WbAction>`。

---

## 6. 演示/验收

`tui/examples/workbench_demo.rs`：

```
cargo run -p future-tui --example workbench_demo -- \
    --scenario launchpad|chat|todo|help --width 120 --height 38 [--ansi|--plain] \
    [--picker /model|/thinking|/mode|/workspace|/theme|/sessions|/lang]
cargo run -p future-tui --example workbench_demo -- --self-test
```

- `--plain`：去 ANSI 的文本帧，便于 grep 断言。
- `--picker`：同一个帧但菜单已经打开——不需要 agent 就能评审（和截图）选择器。
- `--self-test`：构造 120x38 与 90x24 两帧，断言命中区域：
  点 (id=sidebar.todo) 返回 `SetPanel(Todo)`；拖分隔条返回 `ResizeSidebar`；
  滚轮返回 `Scroll`；点击输入框返回 `FocusPane(Input)`；点击选择器行返回 `SwitchSession`。
  全部通过打印 `self-test ok`。
- 验收命令（任何 worker 交付前都要跑）：
  `cargo fmt --check -p future-tui`（若仓库未强制，可只 fmt 新文件）、
  `cargo build -p future-tui`、`cargo test -p future-tui workbench`。

截图：`tui/scripts/shot.sh <scenario> <out.png>`（第二个参数也可以用尺寸形式：
`shot.sh chat 120x38`）。它在指定尺寸的 pty 里跑应用，再把写出的字节交给
`scripts/screenshots/tui_terminal.py` 渲染；没有 `FUTURE_AGENT_GRPC_ADDR` 时用脚本化的
`demo_agent`，所以不需要 agent、不需要模型、也不需要凭据。

---

## 7. gRPC 约束

只读使用现有 RPC。若 UI 需要的字段现有消息没有，**不要改 proto**；在
`docs/tui/dsh-tui-parity.md` 的「gRPC 影响」表里标记 *需要新增* 并给出替代方案
（例如从 `get_state`/session 列表事件里推导）。已确认可用：会话列表、usage/context、
模型、thinking、tool 事件、goal（本地 CLI 状态）、skills（本地目录）。

## 8. 体验目标（验收人看这三条）

1. 一眼能看出「在跑什么」：header 活动态 + 活动行 + 状态栏三层信息，静止时不闪。
2. 手不离鼠标也能完成 80% 操作：切面板、切会话、滚聊天、开命令、点工具卡。
3. 120x38 与 90x24 两种尺寸都不破版；CJK 文案不串列。
