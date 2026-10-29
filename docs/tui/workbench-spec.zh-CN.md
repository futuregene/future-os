# FutureOS Workbench TUI — 设计与实现规格 v1（历史设计记录）

> **历史设计记录。** 本页保留构建全屏工作台时使用的 v1 契约。实现后来已多次修订
> （侧栏宽度、头部分带、图片协议、面板结构等），所以这里的常量与 P1 状态不是当前验收
> 标准。已交付行为以 [TUI 使用指南](../guide/tui.zh-CN.md) 为准；源码事实以
> `tui/src/workbench/` 及其测试为准。

> 目标：在现有 `future-tui`（Rust、自定义 ANSI 渲染、gRPC 客户端）之上，新增一套
> **全屏工作台（workbench）** 界面，体验对齐 dsh-TUI 0.17 的核心能力，并针对
> FutureOS（goal / 多会话 / 技能 / 远程）做得更好。**不修改 agent 的 gRPC 接口**。

本文件是 worker 之间的 v1 历史契约。它记录当时的分工与决策；当前常量、键位和
验收命令以使用指南和测试为准。

---

## 1. 范围

### P0（本次必须落地，可截图验收）

1. 全屏布局：Header / Chat / 侧栏 / 输入框 / 活动行 / 状态栏 / 提示行。
2. 鼠标：点击（切面板、切页签、点行、聚焦输入、点按钮）、滚轮（聊天/侧栏）、
   拖拽（分栏分隔条）、悬停高亮（可选，能力允许时）。
3. Launchpad 首屏：大字 logo、大输入框、参数行（工作区/模型/强度/审批，可点击）、
   快捷入口、最近会话。
4. 侧栏 2 个真实面板：最近（会话名、所在工作区、刷新时间）、工作区（同一批会话按桌面端的方式归档：置顶 / 工作区 / 对话会话）。
   **文件** 不在这两个页签里：它是当前会话工作区的目录树，单独一块浮层
   （`/files` / header 的「文件」按钮；这套 ctrl-only 键位没给它留键），一次一层、目录**打开时才读**，
   `↑`/`↓` 走行，`enter` 在目录上展开、在文件上预览，`⌫` 收起目录（否则跳到它所在的目录）。
   和 `/git` 一样只作用于当前会话，换会话后重读。它的行同样注册进 `HitMap`，
   所以点哪行就开哪行。两个页签只由 `shift+←`/`shift+→` 切：`←`/`→` 始终是草稿光标的，
   侧栏有焦点时也一样（输入框就在屏幕上）。
5. 状态栏真实指标：模型 · 强度 · cwd · git 分支 · 上下文占用条 · token · 费用/缓存。
6. `/` 命令面板（带说明、Tab 补全）、`?` 帮助覆盖层、`/` 过滤、历史 ↑↓。
7. 主题：内置 256 色调色板——合同里的六套（`dark` / `light` / `one-dark` / `one-light` /
   `high-contrast` / `dracula`）加上桌面端那一族（`ayu`、`catppuccin`、`everforest`、
   `gruvbox`、`kanagawa`、`nightfox`、`nord`、`rose-pine`、`solarized`、`tokyo-night`、
   `vesper`，其中五套另有 `-light` 版本）——由 `/theme` 选择；选择器会把高亮所在的那套直接预览到整个界面。
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
tui/src/workbench/theme.rs      // Palette、内置主题、语义色
tui/src/workbench/widgets.rs    // 边框、页签、进度条、徽标、像素 logo、spinner
tui/src/workbench/layout.rs     // Rect、Panes、命中区域（HitMap）
tui/src/workbench/mouse.rs      // SGR 1006 解析、MouseEvent、滚轮/拖拽状态机
tui/src/workbench/launchpad.rs  // 首屏
tui/src/workbench/screen.rs     // 全屏合成：header/body/sidebar/input/status
tui/src/workbench/panels.rs     // 侧栏 2 面板的渲染（纯函数，输入同一个行模型）
tui/src/workbench/picker.rs     // /model、/thinking、/mode、/workspace、/theme、/sessions、/lang 的选择器覆盖层
tui/src/workbench/files.rs      // 文件面板的目录树，以及一个路径能预览成什么
tui/src/workbench/syntax.rs     // 预览框的代码着色：语言识别、token 与着色区间
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
> `/preferences`（`ctrl+q`）列出这个终端记住的全部界面选择：界面语言与配色主题（这两行打开的是 `/lang`、`/theme`
> 那两个选择器，语言排在最前一行）、侧栏位置 / 宽度 / 显示、鼠标、通知、动态、快捷键；
> 这些选择和显示状态持久化在 `settings.json`。默认快捷键新增 `ctrl+p` 命令面板、`ctrl+r` 会话、`alt+h` 帮助
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
│ row h-1        HINT: Esc 聊天 · ctrl+s 侧栏 · / 命令 · ? 帮助                │
└──────────────────────────────────────────────────────────────────────────────┘
```

- 宽度 < 88：侧栏折叠为 0；`ctrl+s` 打开全屏覆盖面板；分隔列变为 2 列时间线轨。
- 高度 < 24：HEADER 压成 1 行（wordmark 同行），INPUT 压成 2 行，隐藏 ACTIVITY。
- chat 内容从底部向上排列（最新在输入框上方），向上滚动时顶部出现 `↑ 更早` 徽标。
- 所有宽度计算必须用 `ansi::display_width`（CJK/emoji 记 2）。

### 视觉规格（256 色）

| 语义 | dark | light | one-dark | one-light | high-contrast | dracula | 用途 |
|---|---|---|---|---|---|---|---|
| bg | 232 | 254 | 235 | 255 | 16 | 235 | 画布 |
| panel | 234 | 252 | 236 | 231 | 233 | 236 | 侧栏/菜单底色（输入框用画布底色） |
| border | 238 | 246 | 240 | 250 | 250 | 239 | 边框、分隔 |
| fg | 252 | 236 | 249 | 236 | 231 | 255 | 正文 |
| muted | 245 | 240 | 247 | 241 | 246 | 247 | 次要说明 |
| accent | 80 (teal) | 30 | 75 | 25 | 51 | 141 | 品牌、选中、进度 |
| iris | 111 | 62 | 73 | 30 | 123 | 117 | 链接、页签激活 |
| amber | 179 | 130 | 180 | 130 | 226 | 228 | 思考、警告 |
| green | 114 | 28 | 108 | 28 | 46 | 84 | 成功 |
| red | 203 | 124 | 174 | 160 | 196 | 204 | 错误、危险 |
| purple | 141 | 91 | 176 | 91 | 213 | 212 | 子代理/技能 |

上表是合同冻结的六套，这些数字对这几个 id 仍然生效（验收截图就是按它拍的）。后来加的调色板——桌面端那一族以及其中五套的浅色版本——在 `tui/src/workbench/theme.rs` 的 `PALETTES` 里是同样的 `Palette` 形状，每个 `crate::themes::THEME_SPECS` 的 id 对应一项；色相取自 `desktop/src/theme/palettes.json`。

**每套主题自带画布。** 画布（`bg`）由 `workbench::run::canvas_rows` 铺满每一行整宽，不再由读者终端配色决定（Ghostty 默认是 `#282c34` 的中灰蓝，会让所有深色主题发灰、次要文字掉到 4.5:1 以下）；也不再是所有深色主题共用的同一个近黑 `232`——那样二十多套主题看起来只是同一套换了强调色。现在每一项取的是它自己背景在 256 色表里最近的一档（与桌面端那套调色板一致）：深色主题落在 `#121212`–`#303030` 的灰阶，浅色主题各有自己的米色/驼色/灰白，`dark` 保留出厂近黑，`high-contrast` 保留纯黑。`panel` 是画布之上第一档能读出“抬起”的灰（对比 > 1.1:1）。256 色终端无法表达带色相的深色背景——立方体里最深的有色格是 `#00005f`、`#005f5f` 这类高饱和原色——所以这张表能给出的变化是明度与冷暖，不是色相。

另一条规则也仍然成立：每个文字角色在它可能落脚的画布与 `panel` 上都要过 WCAG AA（4.5:1），正文在画布上过 7:1。画布往上抬一档会掉对比，因此掉下去的角色跟着提升（十套的 `muted`、五套的 `red`、四套的 `purple`、`everforest-light` 的 `muted`），色相家族保持不变；数值旁的注释记着抬了多少。

每套调色板在行内 `Theme` 有同名语义的位置上都与 `crate::themes` 一致（`accent`、`fg`、`border`、
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
| 对话里的链接 / 图片点击 | 打开它：网址交给浏览器，`futureos://session/…` 切到那条会话，本地文件在预览框打开 |
| Launchpad 参数段点击 | 打开对应选择器（工作区/模型/强度/审批） |
| Launchpad 快捷入口点击 | 执行对应 `/command` |
| 悬停 | 页签/行/按钮高亮；600ms 后显示 tooltip（窄终端截断时） |

双击 = 两次 <400ms 同点；三击=行选择（P1）。

### 键盘

**只有一个前缀：`ctrl`。** 全局默认键要么是 `ctrl` 组合，要么是带 `shift` 的方向键：
`ctrl+o` 模型选择器；`ctrl+t` 思考强度选择器；`ctrl+l` 界面语言；`ctrl+g` git 浏览；
`ctrl+s` 侧栏开/聚焦/关（侧栏外沿的 `›` 按钮一键等效，收起后在**同一列**留一个 `‹` 把它叫回）；
`ctrl+f` 刷新；`ctrl+q` 队列；`ctrl+v` 读剪贴板（图片成为草稿附件，文字粘进草稿）；
`tab`/`shift+tab` 在对话里逐个选中链接与图片（`enter` 打开光标所在那个：浏览器 / 那条会话 /
预览框），在 launchpad 上则在输入框与参数段之间移动；`/` 命令面板；`?` 帮助；`esc` 逐层关闭。

默认键里没有 `alt`——macOS 上 Option 默认是组字而不是 `ESC` + 字母——字母在冲突之后也不够用，
所以剩下的动作**没有键**：审批模式（`/mode`）、工作区（`/workspace`）、复制（`/copy`）、
鼠标捕获（`/mouse`）、偏好（`/preferences`）、草稿里最新的附件（`/preview`）、文件面板（`/files`）、
侧栏两个页签（点页签，或 `shift+←` / `shift+→`），以及减少动态（偏好面板）。帮助浮层在这些行
的键列印 `—`。不够用的原因就是这些冲突：`ctrl+h`/`i`/`j`/`m` 会被终端发成退格、Tab、`\n` 和回车；
`ctrl+a`/`ctrl+b` 是 `screen` 和 `tmux` 的前缀；`ctrl+z` 是挂起；
`ctrl+u`/`k`/`w`/`y` 是输入框自己的编辑键。
`shift+←` / `shift+→` 在任何地方走侧栏两个页签（侧栏显示并拿到按键），`shift+↑` / `shift+↓` 切上一条 /
下一条会话。聚焦的侧栏最后一行只写这两个组合（`shift+←→ 页签 · shift+↑↓ 切换会话`）：`↑↓` 只是移列表
光标、`⏎` 只是打开光标所在行，“怎么换会话”这件事不能留给猜。文件面板是单独的浮层，它自己
在框里写明那一组键（`↑↓ 选择 · ⏎ 打开 · ⌫ 上一层 · r 刷新 · esc 关闭`）。

默认键位里没有功能键：`F1`–`F12` 在笔记本上要按 `Fn`，紧凑键盘上干脆没有。改成功能键或 `alt`
依然支持——只是**出厂默认**一律是 `ctrl`。帮助浮层按 `ACTIONS` **分组**列出（会话 / 选择 / 面板 /
界面），然后是不可改键的行内键，最后是命令。

按住 `shift+↑` / `shift+↓` 只换一次：按键返回 `WbAction::SessionStep(delta)`，驱动先读完整个输入
突发，再用 `Workbench::step_session` 应用净步数。侧栏光标每按一次就跟着走——那是读者眼睛盯着的东西——
会话本身则等按键安静 80 ms（`run::OPEN_SETTLE`）后才打开，所以按住经过的那些不会被加载。

适配器留着最近显示过的 8 条会话（连渲染好的行一起，见 `Live` 的缓存）。切到其中之一时，那些行**先**画上去、
再去问 agent：一条 `get_state` + `get_session_entries`（两者都按 id 寻址，所以并发发出）用来校正。
页没变就保留它对照的那些行，变了才按新页重建。这一对并发请求替换了原来串行的三次往返
（`switch_session`、`get_state`、取页）——因为 `get_state` 本身就会 hydrate 它被问到的会话。

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

启动台的默认值分两种来路：`/new`（`ctrl+n`）**继承刚离开的那个会话的工作区**——读者刚在哪个
项目里干活，下一间就开在哪个项目；聊天会话不继承，下一条消息会新建自己的 chat 目录。一次全新的
启动则用 **TUI 进程的启动目录**（`cd 项目 && future tui` 即该项目）；从 app 自己的临时目录启动、
或者显式选了「对话会话」时，默认是**对话会话**（不绑工作区——跑在桌面端同款的
`~/.future/workspaces/chat/<session id>`，也就是侧栏「对话会话」一节归的目录：会话 id 由 TUI
自己生成，目录在建会话时创建）。模型与强度仍是**产品默认值**（都从 `list_models` 读：`isDefault`
与 `defaultThinkingLevel`——和 `new_session` 自己用的是同一套解析），审批是 **完全访问**（`off`）：
把上一个会话的模型、审批带进下一个，会让 `/new` 变成前一间的副本而不是新开一间。默认审批
不只是写在参数上，建会话后会真的下发，所以参数不会写着会话实际不走的值。

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
同一个动作；命令写成 `/workspace <路径>` 时，参数作为搜索行内容预填（`enter` 直接落到那一行，
不再是一个空列表）。那一行下面还有唯一一行不是目录的选项：**对话会话**，也就是「不选工作区」——
在它上面按 `enter`：没有会话时暂存为「不选工作区」，第一条消息以 TUI 生成的会话 id 在
`~/.future/workspaces/chat/<id>` 下建目录；已有会话时把会话挪到它自己同名的 chat 目录
（侧栏「对话会话」一节认的就是这个前缀）。目录由 TUI 自己创建：读者输入的路径是读者的事，打错了
该由 agent 报错，而这个路径没人会替他建。审批模式就是桌面端自己的那四个值；没有单独的 TUI
「权限级别」芯片，因为别的客户端都没有这个选择。

行的绘制与命中区域来自同一份列表和同一份几何（`screen::picker_box` 用 `box_geometry`），
所以点击一定落在指针下面那一行，而不是旁边一行。

---

### 侧栏的折叠记忆

侧栏不画搜索行：两个页签列的是同一批会话，搜会话是历史选择器的事——`ctrl+r`
（`Action::Sessions`）自己带一行筛选，匹配的也是同样三个字段（标题、工作区、id，
不区分大小写）。侧栏里的 `/` 就是命令面板的键，与在输入框里按它一样；侧栏页脚只列
它自己的按键（`shift+←→` 切页签、`shift+↑↓` 换会话）。

折叠（在工作区标题上按 `enter`，或点它）会跨进程保留：路径写进
`~/.future/tui/settings.json` 的 `foldedWorkspaces`，下次启动在按任何键之前就已经
收起。

### 文本界面里的图片

图片用**半格**画：每格一个 `▀`，前景是一对像素的上面那个，背景是下面那个。除了颜色不需要终端做
任何事，所以任何 256 色终端都能显示（不需要 kitty graphics、iTerm2 图片或 sixel），终端上报真彩色
时用 24 位色。它是**预览**：几百列宽对照片是缩略图，对截图则足够看清。

解码在适配层（`image`，与 agent、host、桌面端同一个 crate），每个文件只做一次，长边缩到 1024 像素：
缩放重画的是手上已有的像素。文本预览按 `MAX_TEXT_PREVIEW_BYTES` 有界读取，并在字符边界上切。
读不了的文件会在框里说明原因，而不是开一个空框。

文本预览**按语言着色**。`syntax.rs` 把每一行分成关键字、类型、被调用的名字、字符串、数字、注释、
属性与键；它只回答 token 是什么，从不说颜色，颜色由 `theme::Palette::token` 决定——于是高亮的代码
跟随当前主题，而不是把深色高亮器的配色搬到浅色画布上。分词在文件打开时做一次，每一行带着自己的
着色区间走。

它刻意不是解析器。读者要的是**一行的可见形状**：注释、字面量（包括跨行的 Rust raw 字符串、Python
docstring 与 JavaScript 模板字符串）与声明；代价是每行一次向前扫描，行间只带一点状态。不认识的扩展名
原样绘制，不去猜。制表符展开成四个空格：绘制行里若留着真正的 `\t`，光标会跳到终端自己的 tab 位，
整帧的列就散了。

---

## 5. 类型契约
```rust
pub struct WorkbenchData {
    pub app_name: String, pub version: String,
    pub connected: bool,
    pub session: SessionMeta,          // id/title/cwd/chat/branch
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
