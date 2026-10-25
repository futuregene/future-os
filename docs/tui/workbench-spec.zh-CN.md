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
3. Launchpad 首屏：大字 logo、大输入框、参数行（模型/强度/模式/权限，可点击）、
   快捷入口、最近会话。
4. 侧栏 4 个真实面板：待办（goal）、会话（sessions）、用量（usage）、技能（skills）。
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
tui/src/workbench/panels.rs     // 侧栏 4 面板的渲染（纯函数，输入 PanelData）
tui/src/workbench/picker.rs     // /model、/theme、/sessions 的列表选择器覆盖层
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

## 3. 布局（单位：终端单元格）

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ row 0..4   HEADER (5)                                                        │
│   x0..9   像素鲸/像素 FUTURE 图标（4 行, 每行 9 cells）                       │
│   x10..    FUTURE OS  (bold, accent)  ·  workbench                           │
│            session title (dim) · 分支/工作区                                  │
│   right    ⌘ future-tui vX · 后端状态点 (●connected)                          │
│ row 4      细分隔线 (dim, ─)                                                  │
├───────────────────────────────────────────────┬──────────────────────────────┤
│ rows 6..h-8   BODY                            │ SIDEBAR_W = 36 (30..48)      │
│   chat 区域宽 = W - SIDEBAR_W - 1             │ row 6 页签条: 待办 任务 会话 │
│   x = SIDEBAR_W+1 .. W-1                      │        用量 技能            │
│   左右各留 2 列内边距                          │ 分隔列 x = SIDEBAR_W (│)     │
├───────────────────────────────────────────────┴──────────────────────────────┤
│ rows h-7..h-4  INPUT (4): 上边框 / 提示行 / 下边框                            │
│   ❯ <draft>                                   右侧 ⛶                       │
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
| panel | 235 | 252 | 237 | 254 | 17 | 237 | 侧栏/输入框底 |
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

Logo：9x4 像素鲸（用 `█ ▀ ▄` 三档方块），运行时按 8fps 轻微摆尾（demo 可静态）。
字标 `FUTURE OS` 用 block 字体 5 行版本，仅在宽度 ≥ 96 时显示；否则单行文字。

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
| Launchpad 参数段点击 | 打开对应选择器（model/effort/preset/permission） |
| Launchpad 快捷入口点击 | 执行对应 `/command` |
| 悬停 | 页签/行/按钮高亮；600ms 后显示 tooltip（窄终端截断时） |

双击 = 两次 <400ms 同点；三击=行选择（P1）。

### 键盘

沿用现有全局键位；新增/覆盖：
`ctrl+b` 侧栏开/聚焦/关；`tab` 面板内焦点；`shift+tab` 反向；
`ctrl+q` 折叠待办；`alt+z` 放大侧栏；`/` 命令面板；`?` 帮助；`esc` 逐层关闭。

### 选择器（`/model`、`/theme`、`/sessions`）

三个菜单共用同一个组件（`picker.rs`）：带标题的盒子 + 增量搜索行（`❯`）+ 分组标题
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

行的绘制与命中区域来自同一份列表和同一份几何（`screen::picker_box` 用 `box_geometry`），
所以点击一定落在指针下面那一行，而不是旁边一行。

---

## 5. 类型契约

```rust
pub struct WorkbenchData {
    pub app_name: String, pub version: String,
    pub connected: bool,
    pub session: SessionMeta,          // id/title/cwd/branch
    pub model: String, pub effort: String,
    pub mode: String, pub permission: String,
    pub usage: UsageData,              // ctx_used/ctx_total/cost/cache/tps
    pub todos: Vec<TodoItem>,          // goal todos
    pub sessions: Vec<SessionRow>,
    pub skills: Vec<SkillRow>,
    pub jobs: Vec<JobRow>,
    pub models: Vec<ModelChoice>,      // id/label/provider/context/thinking
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
    [--picker model|theme|sessions]
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
