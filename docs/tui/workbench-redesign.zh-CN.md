# FutureOS 工作台 TUI —— 按序改造清单

全屏工作台的工程计划：哪些已经成立、接下来改什么、动哪个文件、什么顺序、每一条
怎么验收。冻结契约（模块划分、类型名、键位、布局常量、demo CLI）在
[workbench-spec.zh-CN.md](workbench-spec.zh-CN.md)；目标与差距的证据在
[dsh-tui-parity.zh-CN.md](dsh-tui-parity.zh-CN.md)；逐条验收记录与截图在
[verification.md](verification.md)。本文与契约冲突时契约优先，本文就是错的 ——
请指出来。

范围：只动 `tui/`。不改 `packages/rpc`、不改 agent（数据来源的完整表见 parity
文档 §7）。

## 0. 现状：已经落地的部分

这份计划原先描述的还是一个不存在的壳。壳现在存在了，所以这一版是对着代码排序的，
不是对着意图排序的。

| 已落地 | 提交 | 定下了什么 |
|---|---|---|
| 工作台作为第二入口 | `7d62e8990` | 合成器、命中图、启动台、主题 |
| `future tui` **就是**工作台 | `2101b3c98` | 一个界面，没有布局开关 |
| 删除内联 UI | `d701765ca` | 旧页面与它的开关是删除，不是废弃 |
| 鼠标全链路可用 | `e72b1213d` | SGR 1006 + 命中图 + 拖拽 + 滚轮；帧不再以换行结尾 |
| 布局整段降级 | `1767726ee` | 不在词中间切、不留空轨、页眉右角整段回退 |
| 面板接真实数据源 | `b55bd66eb` | 目标待办、技能、定时任务、会话、用量 |
| 每一行都绝对定位 | `77c20b9cb` | 帧比终端高也不会再把屏幕向上顶 |
| `/model`、`/theme`、`/sessions` 共用一个选择器 | `8d1afa2c7` | 六套调色板实时预览、模型列表带强度滑杆、会话列表带搜索 |

`ba916d63e` 的状态：`tui/src/workbench` 下 15 个模块 / 9,076 行 / 119 个测试，
`cargo test -p future-tui workbench` 全绿（119 passed），
`cargo test -p future-tui --lib` 1,013 passed，
`workbench_demo --self-test` 输出 ok，`docs/tui/assets/` 里 19 帧（见
[verification.md](verification.md)）。

上面那批提交里有四条正是早先计划里当作「待做」的东西，它们让本清单剩下的部分变成
便宜的收尾，而不是新一轮猜测：

1. **鼠标、SGR 1006、全链路**（`e72b1213d`）：模式随 alt screen 一起写入，并在
   stop/崩溃时写回；报告被解析成 `Press/Release/Drag/Move/Wheel`；绘制与命中共享
   同一张 `HitMap`（`tui/src/workbench/layout.rs`）；点击切面板、滚轮滚聊天或聚焦的
   面板、分隔条可拖。真机验证：`verification.md` §2.2、图 4 与图 17（链路里唯一
   剩下的缺陷是拖拽的符号，P0-1）。
2. **五个面板读真实数据，不是夹具**（`b55bd66eb`）：待办折叠目标的 `events.jsonl`，
   技能合并 `get_state` 与两个技能目录，任务只读打开 `tasks.db`，会话来自
   `list_sessions`，用量来自 `get_state`（`tui/src/workbench/local.rs`、`live.rs:201`）。
   真机验证：`verification.md` §2.3、图 3–图 5。
3. **降级是整段整段的**（`1767726ee`）：低于 88 列撤掉侧栏且不留空轨；低于 24 行
   页眉压成一行、输入框保留边框、activity 隐藏、提示整条丢弃；页眉右角按
   `● 应用 vX` → `● 应用` → `● vX` → `●` 退化。真机验证（120×38 / 90×24 / 60×20）：
   `verification.md` §2.7、图 2、图 14–图 16。
4. **每一行都绝对定位，三个列表动词有自己的菜单**（`77c20b9cb`、`8d1afa2c7`）：
   帧再也不会把终端顶上去；`/model`、`/theme`、`/sessions` 打开同一张浮层，带搜索行、
   分组标题、当前值上的 `✓`、强度滑杆和点行即应用。真机验证：`verification.md`
   §2.5、图 8–图 13。

剩下的不是「把壳造出来」，而是**壳里面的四个缺陷**，然后是旧 UI 里壳没有继承的
部分，最后才是对齐打磨。

## 1. 改变了原计划的事实

**F1 —— 开关这件事已经结束：没有 `--layout`，也没有 `--legacy`。**
早先的计划写的是「`--layout workbench|inline`，默认 workbench」，契约写的是「内联
被删除，它的 `--legacy` 一起删」。契约赢了：`future tui` 只启动工作台；`--layout`、
`--legacy`、`--classic`、`--resume` 都不存在，帮助文本一旦写出这些名字，
`help_describes_this_build` 就会失败（`tui/src/help.rs:96`）。**对本清单的约束：
永远不要把渲染器开关加回来。** 两个渲染器共享一条输出流正是被否决的设计，而「临时
开关」就是这样回来的。

**F2 —— 工作台是合成器，不是组件。** 组件拥有的是行
（`Component::render(width) -> Vec<String>`）；工作台拥有的是矩形，返回恰好 `h` 行、
每行恰好 `w` 列（`tui/src/workbench/screen.rs:29`）。所以 transcript 是**适配**而不是
重写：`live.rs` 把消息渲染成行，合成器把它们画进聊天矩形。

**F3 —— 每一行都绝对定位；剩下的是「首帧用的是哪个尺寸」。** `frame_bytes` 用
`CSI row;1H` 逐行定位（`tui/src/workbench/run.rs:368`），从不让光标前进，所以比终端
高的帧只会被裁掉，不会把屏幕顶上去 —— 这一点在 `77c20b9cb` 已经落地。**没有**被钉住的
是「首帧按哪个高度合成」：本次工作中量到，一个 120×38 的实例首帧高了一行（细线在第 5
行、页签在第 6 行、没有空行），下一次重绘自愈；而同尺寸的另一个新实例在 3/6/9 秒都是对
的。两者都发生在「绝对定位」的代码上，所以怀疑对象不是写入器，而是过期尺寸进入了第一次
合成。结论：写入器不动，P0-2 先要求「合成行数 == 写入器拿到的 `h`」的测试，若尺寸可能
过期，就在首次合成前重读。

**F4 —— 鼠标是真的，但有一个轴是反的。** 整条链路接好了：
`Terminal::set_mouse_mode(true)` 在 `start` 之前（`tui/src/workbench/run.rs:92`）、
SGR 1006 解析（`tui/src/workbench/mouse.rs:56`）、与渲染共享的命中图、
`?1000h?1002h?1006h`。点击页签能切面板（实测），滚轮能滚，分隔条拖拽会改
`sidebar_w` —— **方向反了**：`width = start_width + (x - start_x)`
（`tui/src/workbench/mod.rs:568`），而分隔条在 `W - width - 1`，所以把边往右拖会让
侧栏**变宽**、边往左走。两次实测（右拖 36 → 42，左拖 42 → 36）。一个符号，P0-1。

**F5 —— 五条面板命令没有处理函数；另外三条是菜单。** `COMMANDS` 有 12 条
（`tui/src/workbench/mod.rs:904`）；`run.rs:249` 把 `/model`、`/theme`、`/sessions`
截走并打开选择器（面板和手打两条路都是），`command()` 只认
`quit/new/reply-detail/refresh/help` 加启动台 chip
（`tui/src/workbench/live.rs:694`）。于是 `/workspace`、`/usage`、`/skills`、
`/export`、`/transcript` 仍然回 `Unknown command` —— 已实测其中两条
`[V]`（实测；这几条落地后缺陷帧已删除，改法与证据见 `verification.zh-CN.md` §6.1）。P0-3。

**F6 —— 删内联 UI 删掉的是「屏幕」，不只是像素。** `/sandbox`、`/worktree`、
`/keymap`、`/copy`、外部编辑器、全屏会话管理器，在工作台里都没有落点。其中两项
（parity 文档 §6 第 34、36 行）是 FutureOS 原本**领先**的地方。P1-3。

## 2. P0 —— 壳里面的缺陷（按序）

**状态：四条都已实现并验收。** 下面是当初按缺陷写下的改造清单；每条最终落地的改法、
测试与帧，见 [verification.zh-CN.md](verification.zh-CN.md) §6.1。

每一条：症状与证据、落点、钉住它的测试、验收命令。

### P0-1 分隔条拖拽与指针反向

- **现在**：在 120×38 的 tmux 里，把分隔条往右拖，侧栏从 36 变到 42；往左拖变小。
  指针下的那条边朝背离指针的方向走。
- **修法**：`tui/src/workbench/mod.rs:568` 改成 `width = start_width - delta`
  （`delta = event.x - drag.start_x`），保留 `SIDEBAR_MIN..SIDEBAR_MAX` 夹紧。
  `start_width` 与 `start_x` 不动：抓取点是分隔条**所在**的位置，偏心抓取不该让
  侧栏跳一下。
- **测试**：`a_drag_towards_the_edge_narrows_the_rail` —— 造 120×38 帧，在分隔条上
  按下、向右拖 6，断言 `ResizeSidebar(30)`；从默认值向左拖 6，断言
  `ResizeSidebar(42)`；两端都夹紧。
- **验收**：`cargo test -p future-tui workbench`，再做
  `verification.md` §2.2/M5 的现场检查。

### P0-2 首帧可能按过期的高度合成

- **现在**：`frame_bytes` 每行都绝对定位、从不让光标前进
  （`tui/src/workbench/run.rs:368`），所以比终端高的帧只会被裁掉，不会顶屏 ——
  `77c20b9cb` 去掉了旧的失败模式。**没有**被钉住的是首帧按哪个高度合成。两个测量：
  - 一个 120×38 的实例首帧**高了一行**（细线在第 5 行、页签在第 6 行、没有空行），
    下一次重绘自愈；
  - 同一份构建、同一尺寸的另一个新实例，在启动后 3 秒、6 秒、9 秒都是对的。

  两次都发生在「绝对定位」的代码上，所以写入器不是嫌疑人：过期尺寸进入第一次合成能
  同时解释这两个现象，别的东西解释不了。
- **先诊断**：首帧的 `h` 从哪来？携带真实尺寸的 resize 事件是不是在第一次 `render`
  之后才到？用测试定，而不是靠重写定。
- **修法，按这个顺序**：(1) 先加「合成行数等于写入器拿到的 `h`」的测试（渲染侧的
  `debug_assert!` 之外，写入侧补一条）；(2) 如果启动时尺寸可能过期，就在合成前立刻
  重读，并在读到变化时丢弃这一帧；(3) 只有到这一步之后，才考虑首次 resize 后再画
  一遍 —— 而这正是按 `SIGWINCH` 重绘的终端本来就白送的。
- **测试**：`a_frame_has_exactly_the_rows_it_was_asked_for`，加上已有的
  `a_frame_never_ends_in_a_line_feed`。
- **验收**：`cargo test -p future-tui workbench`；手工：120×38 启动后比对首帧与稳态帧
  （`verification.md` §2.1/L7），再把窗口从 120 缩到 90 时点侧栏页签 —— 必须点中
  页签而不是上面一行。

### P0-3 五条命令写在面板里，按下去什么也不做

- **现在**：`/workspace`、`/usage`、`/skills`、`/export`、`/transcript` →
  `Unknown command`（已实测 `/usage` 与 `/skills` `[V]`；缺陷帧已删除，
  改法见 `verification.zh-CN.md` §6.1）。`/model`、`/theme`、`/sessions` **不在**这个
  名单里：`run.rs:249` 在 `command()` 看到它们之前就把它们变成了选择器。
- **修法**：在 `live.rs::command()` 里逐条二选一：
  - `/usage` → `SetPanel(Usage)`；`/skills` → `SetPanel(Skills)`；`/workspace` →
    打开工作区选择器（或聚焦页眉的 cwd）。这些是**导航**，三个 match 分支就够。
  - `/export` 与 `/transcript` 需要文件路径和 transcript 转储；要么接到 crate 里
    已有的导出器，要么**先从 `COMMANDS` 里摘掉**，直到它能用。
- **测试**：`every_palette_entry_has_a_handler` —— 对 `COMMANDS` 里每一条，要么能
  解析成选择器（`PickerKind::from_command`），要么落到 `command()` 里一个不是
  `Unknown command` 的分支。只有表驱动测试才不会再烂掉，也能拦住「下一个拿到选择器
  却没有处理函数的动词」。
- **验收**：`cargo test -p future-tui workbench`；现场：`/` → `usage` → `Enter`，
  用量面板获得焦点，提示行**不**出现 `Unknown command`（`verification.md` §2.5/C5）。

### P0-4 git 分支从来没有到达界面

- **现在**：`SessionMeta { branch: String::new(), … }`
  （`tui/src/workbench/live.rs:553`）；于是页眉第二行只有 cwd，而 `screen.rs:247`
  里 `cwd · branch` 那段组合早就在等一个值。
- **修法**：在已经在刷新的本地事实里读它 —— `Live::refresh_local()`
  （`tui/src/workbench/live.rs:201`）—— 用 crate 里已有的 `GitCli::branch()`
  （只读、带超时、可注入 runner），按 cwd 缓存，会话 cwd 变化时重读（worktree 会话
  必须显示自己的分支）。不在仓库里就留空，页眉已经会退化成只显示 cwd。
- **测试**：`the_branch_is_read_once_per_cwd` —— 假 runner 记调用次数：同一 cwd 刷
  两次只有一次读，换 cwd 再读一次，读失败留空而不是 panic。
- **验收**：`cargo test -p future-tui workbench`；现场：页眉第二行为
  `/Users/geilige/future-os · dev-huichen`（`verification.md` §2.4/A6）。

P0 的排序理由：P0-1 与 P0-3 是用户看得见的谎，P0-4 是一个缺掉的 dsh 对齐读数，
P0-2 要先诊断才谈得上修。三个修法都是单文件 + 一个测试；P0-2 从一个测试开始。

## 3. P1 —— 旧 UI 能做、而现在这个壳做不到的

按「读者在前十分钟里会先撞上哪个」排序。

### P1-1 悬停：让可点的东西亮起来

`MOUSE_ON` 是 `?1000h?1002h?1006h`，`MouseKind::Move` 解析后被丢掉
（`tui/src/workbench/mod.rs:584`）。把 `?1003h` 加进 `MOUSE_ON`（`MOUSE_OFF` 里保留
`1003l`），在界面里记住悬停的 hit id，用 `iris` 色只把**提示**画亮（页签文字、行背景、
输入框边框），每帧最多一次。这一步不做 tooltip：tooltip 需要定时器，而一个空闲也在重绘
的定时器会破坏体验能力 1（静止）。用命中图的单元测试覆盖，不用截图（悬停要有钟）。

### P1-2 输入框的 `⛶` 与全屏草稿

现在既没有提示也没有编辑器，外部 `$EDITOR` 路径也到不了。先落提示（一个命中区 +
一个显示草稿的全宽浮层，`esc` 关闭），再接编辑器。这是「输入区连续性」（parity §4.7），
也是让长提示词可用的最便宜一步。

### P1-3 给 `/sandbox`、`/worktree`、`/keymap`、`/copy` 重新找落点

这些屏幕的代码在 crate 里，工作台里到不了。其中两项是 FutureOS 原本**领先**的对齐行。
按偏好排序的选项：
1. 浮层选择器（`/sandbox`、`/worktree`），复用现有视图组件产出的行，靠
   `paint_overlay` 像帮助/面板那样画出来；
2. 如果这一轮浮层太重，就给 worktree/sandbox 状态一个侧栏面板（第 6 个页签）。
`/copy` 应该变成聊天矩形上的动作（复制最后一条回复），而不是一个屏幕。

### P1-4 通知

`tui/src/notifications.rs` 在，工作台从不调用。在「运行成功结束且没有排队后续」时发
（dsh 的规则），标题保留流式标记。测试打在通知**决策**（纯函数）上，不是投递。

### P1-5 会话管理器：固定与工作区轨道

会话面板已有标题、模型、时长与流式状态，`/sessions` 选择器（`picker.rs`）已经能搜索
并点行切换，所以 dsh 那个管理器的「过滤」半边已经盖住了。缺：固定（存进客户端设置）
与列表上方的工作区轨道。

### P1-6 时间轴轨道

聊天右缘两列轨道：每个用户回合一个刻度，点击 = `Scroll` 到那一行，轨道上的滚轮忽略。
命中图已经支持单格区域；transcript 适配层要暴露回合偏移。

### P1-7 带构成配色的上下文条

现在是单色强调色填充。dsh 按内容构成给填充上色，压缩后清掉过期构成。需要
`get_state` 里的构成拆分；如果没有，这一条就保持 partial，而不是编一个。

### P1-8 分组可搜索的设置页

选择器已覆盖读者最常改的东西（`/model` 带强度滑杆、`/theme`、`/sessions`），它们提交的
值都会持久化。缺的是一个**列出其余设置**的地方：一个 `/settings` 浮层，4–6 组（外观、
模型、权限、面板、键位）。那是体验能力 8 的可发现性部分，不是新的数据路径。

### P1-9 按项目的历史搜索

提示词召回已有；跨项目提示词的 `Ctrl+R` 搜索没有。要搜的存储就是 agent 已经暴露的
会话日志。

### P1-10 转向 / 排队 / 中断

`Enter` 提交并在运行中排队（`live.rs::submit`），`ctrl+x` 中断。dsh 区分转向
（`Enter`）、排队（`Tab`）、中断并发送（`Ctrl+Enter`）。放在 P1-4 之后做：是排队徽标
让这个区别可见。

### P1-11 transcript 里的图片

工作台**完全没有**图片处理：没有 `is_image_line` 等价物、没有 Kitty/iTerm2 行保留、
没有附件 chip。内联 UI 的那道守卫（`tui/src/app.rs`，随它一起删掉了）必须先回到合成器
里，图片才可能被画进矩形 —— 否则侧栏往图片行上一画，图片会透出来。未在真机测（抓的
会话里没有图片），所以是 P1 而不是 P0。

## 4. P2 —— 对齐打磨，等 P1 量完再说

- `/rewind` 与双击 `esc` 回到较早回合；`/fork`、`/tree` 进面板。
- 左右两栏 diff 卡片，单侧文件占满。
- Mermaid 与 Unicode LaTeX。
- 图片预览可缩放/平移；Sixel。
- 应用内选择：双击/三击、按面板复制、不凭空插入换行。
- 双语界面（第 23 行）：这是产品决定，不是翻译活 —— 见 parity 文档 §10.3。

## 5. 明确不做

- 第二个渲染器、`--layout` 开关或 `--legacy` 逃生口（F1）。
- ratatui / crossterm / tui-rs：手写终端后端保留。
- 为 `list_sessions` 的每会话用量、或为任何子代理面改 proto（parity §7.4）。
- 子代理、shell 作业、插件面板（parity §8；第 26–28 行是 `n/a`）。
- 重写 `/worktree`、`/sandbox`、技能安装、打印模式、`FUTURE_HOME` 隔离 —— 它们都在，
  P1-3 讲的是**够得着**它们。

## 6. 数据来源（谁读什么、怎么失败）

与契约 §9 一致（因为已经实现），在这里重复一遍，因为每一行新面板内容都要遵守。

| 面板 | 来源 | 规则 |
|---|---|---|
| 待办 | `<目标根>/goals/<id>/events.jsonl` | `FUTURE_GOAL_ROOT`，否则最近的祖先 `.future/goal`，否则 git toplevel；取最近活跃且未终态的目标；`✔ ◉ ○ ✖` + 优先级 + 文本 |
| 技能 | `get_state.skills` + `~/.future/agent/skills` + `~/.agents/skills` | 名字来自 agent，描述来自目录，任一可用即可 |
| 任务 | `<home>/tasks/tasks.db` | `SQLITE_OPEN_READ_ONLY`；库不存在就是「没有定时任务」，绝不新建文件 |
| 会话 | `list_sessions` | 标题、模型、时长、是否流式；读失败保留上一次的好列表 |
| 用量 | `get_state.usage` | 只给完整值，绝不把数字裁成带 `…` 的 |
| 分支 | `git rev-parse --abbrev-ref HEAD`（P0-4） | 只读、超时、按 cwd 缓存、不在仓库里留空 |

每个读取器都是 best-effort：库缺失、半行、不可读文件都只会让行数变少，不 panic、不弹提示。

## 7. 风险

| 风险 | 缓解 |
|---|---|
| 「临时」渲染器开关回来 | `help_describes_this_build` 钉住这些开关不存在；契约禁止开关（F1） |
| 鼠标回退在 CI 里看不见 | 命中图在 120×38 / 90×24 / 60×20 有单元测试；人工闸门是 `verification.md` §7 的现场配方 |
| 尺寸不匹配时帧会位移 | P0-2 先钉行数，再在合成前重读尺寸；写入器的无结尾换行不变，继续有测试 |
| 面板读取把 TUI 绑到别的 crate 的文件格式上 | 读取是 best-effort 且不带版本；格式变了就退化成更少的行 |
| 长会话 | 界面只渲染可见窗口，历史按 `LoadOlder` 分页；每帧不扫全量 transcript |

## 8. 发布与回滚

没有发布开关：`future tui` 就是工作台。缺陷在任务分支上修再合入 —— 一次糟糕的界面
改动的回滚是 `git revert` 那个提交，不是加开关。`docs/tui/assets/` 里的帧是视觉基线；
改动影响某个分区时用 `tui/scripts/shot.sh` 重拍。

## 9. 本目标的验收

单条改动（worktree，任务分支上）：

```bash
cargo fmt -p future-tui --check
cargo clippy -p future-tui --all-targets -- -D warnings
cargo test -p future-tui workbench
cargo run -q -p future-tui --example workbench_demo -- --self-test   # 输出 `self-test ok`
```

截图（只有分区变化时才重拍；配方见 `verification.md` §7）：

```bash
# 真机会话：先 export FUTURE_AGENT_GRPC_ADDR 与 SESSION_ID（§7 步骤 3），
# 然后任意场景/尺寸（本文的帧）
tui/scripts/shot.sh chat 120x38     # docs/tui/assets/workbench-chat.png
tui/scripts/shot.sh usage 120x38
tui/scripts/shot.sh chat 90x24
tui/scripts/shot.sh chat 60x20
tui/scripts/shot.sh help 120x38
tui/scripts/shot.sh palette 120x38

# 确定性、不需要 agent、两次运行字节相同（仓库自己的路径）
python3 scripts/screenshots/capture.py tui
```

两条都要用：`capture.py tui` 是「diff 意味着界面变了」的闸门，且不需要凭据；真机配方
证明的是面板读的是这台机器的真实数据。

整体验收（`verification.md` 记录的内容）：它 §1 表里每一行都有帧或命令，
`docs/tui/assets/` 至少 8 张 PNG，上面四个 P0 各自带着测试关闭。

## 10. 待决事项

1. **P0-3**：五条导航命令接线，还是从 `COMMANDS` 删掉？建议：接 `/usage` 与
   `/skills`（三个 match 分支），在写出文件之前删掉 `/export` 与 `/transcript`，
   `/workspace` 要么开选择器要么先不放。
2. **P1-3**：worktree/sandbox 用浮层选择器还是第 6 个侧栏页签？浮层更接近被删的屏幕，
   页签更接近 dsh 的面板模型。
3. **P2 双语**：只有中文界面是有意的产品立场吗？如果是，第 23 行就按「决定」关闭，
   而不是按「差距」。
4. 120 列下 36 列的默认侧栏合适吗？它给聊天留 83 格；dsh 的八个面板大致在 28–32 分割。
   改常量之前先量。
