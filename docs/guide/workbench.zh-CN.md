# Workbench —— 第二个 TUI

`future-workbench` 是 FutureOS 的全屏终端界面。它是**第二个 TUI**，不是 `future-tui`
的一种模式：inline TUI 把行追加进终端自己的 scrollback 并只对尾部做 diff，而 workbench
占有整块屏幕、重绘绝对行、用「画过什么」来解析鼠标点击。这个程序没有任何一处走 `app.rs`。

目前交付的是**界面本身**：七个分区、侧栏、launchpad、浮层、鼠标与键盘，数据来自确定性的
演示数据。接 agent 的那一层（用 `get_state` / `list_sessions` / 事件流填同一份数据）是下一片；
设计契约见 [`docs/tui/workbench-spec.zh-CN.md`](../tui/workbench-spec.zh-CN.md)。

## 怎么跑

```bash
# 界面本身，演示数据（标题栏会写 demo）
cargo run -p future-tui --bin future-workbench -- --scenario chat

# 任意尺寸打一帧：截图或检查布局
cargo run -p future-tui --example workbench_demo -- --scenario launchpad --width 120 --height 38
cargo run -p future-tui --example workbench_demo -- --scenario chat --plain     # 去掉 ANSI

# 鼠标验收（120x38 与 90x24 两档命中区域）
cargo run -p future-tui --example workbench_demo -- --self-test
```

场景：`launchpad`（还没有会话）、`chat`（一轮对话，过程折叠成一行汇总）、`todo`
（很长的 goal 列表，用来看侧栏的滚动窗口和 `↓ N more`）。

## 一帧长什么样

```
 头部（5 行：像素鲸、宽 ≥ 96 时的字标、会话 · workspace · 模型、后端状态点）── 细分隔线
 ──────────────────────────────────────────┬──────────────────────
 对话（底部对齐，向上有 ↑ 更早）            │ 侧栏：待办 任务 会话
                                            │       用量 技能
 ──────────────────────────────────────────┴──────────────────────
 输入框（❯ 草稿）
 活动行：⠋ 思考 … · 3s · 2 工具                      42 tok/s · 缓存 96%
 状态栏：model · effort · permission   ░░ ctx 6.9k/1.0M 0.7% · ≈¥0.01
 提示行：Esc 回到聊天 · ctrl+b 侧栏 · / 命令 · ? 帮助
```

launchpad 是**正文**，不是盖在对话上的弹窗：没有会话时整块屏幕就是输入界面（logo、字标、
输入框、模型 / 强度 / 模式 / 权限四个可点参数、快捷键、最近会话），底部不再重复画输入带。

两个断点都来自契约：

- **宽 < 88**：侧栏收起，`ctrl+b` 改为把当前面板全屏打开。
- **高 < 24**：头部压成 1 行，输入框只留上边框，活动行隐藏。

每一帧都恰好是 `height` 行、每行恰好 `width` 个显示列（CJK 与 emoji 计 2），1×1 也一样——
这条不变量由帧测试和唯一负责补齐的 `ansi::fit` 共同保证。

## 按键与鼠标

| 按键 | 动作 |
|---|---|
| `tab` / `shift+tab` | launchpad：在输入框与参数行之间移动；对话里：焦点进入 / 离开侧栏 |
| `←` `→` | 移动草稿光标 |
| `↑` `↓` | 滚动对话（或当前面板的列表） |
| `pageUp` / `pageDown` | 对话整页滚动 |
| `ctrl+b` | 侧栏：显示并聚焦 → 取消聚焦 → 收起；窄终端下全屏打开 |
| `/` | 命令面板（草稿为空时）——输入过滤，`enter` 执行 |
| `?` | 帮助浮层（按键 + 命令） |
| `ctrl+e` | 请 app 切换「最终输出 / 完整过程」 |
| `ctrl+c` | 退出 |

| 鼠标 | 动作 |
|---|---|
| 点侧栏页签 | 切换面板 |
| 点会话行 | 打开该会话 |
| 拖分隔条 | 调侧栏宽度（30–48 列），松手持久化 |
| 点输入框任意处 | 聚焦 |
| 对话区滚轮 | 滚动；到顶时请求更早一页 |
| 侧栏滚轮 | 滚动面板 |

鼠标上报是 SGR 1006，只由本程序开启；inline TUI 从不打开它。

## 还没有的部分

- **agent 适配层**：现在数据来自 `workbench::demo`。下一片把 `WorkbenchData` 用 agent 填满
  （会话、用量、模型、技能、待办）并提交 prompt——界面这一侧的契约已经定死：从
  `WorkbenchData` 画，返回 `WbAction`。
- **`future wb`**：统一 `future` 二进制里的子命令，以及「workbench 要不要成为默认终端入口」这个决定。
- 契约里的 P1：时间线轨、diff 双栏卡、图片预览、会话管理器全屏版、全屏草稿编辑器。

## 代码在哪

| 文件 | 职责 |
|---|---|
| `tui/src/bin/workbench.rs` | 入口：raw mode、alternate screen、事件循环、动作应用 |
| `tui/src/workbench/mod.rs` | `Workbench`（视图状态）、`WbEvent` / `WbAction`、键鼠分发 |
| `tui/src/workbench/screen.rs` | 帧合成：七个分区、把局部区域拼进整行、浮层 |
| `tui/src/workbench/layout.rs` | 分区矩形、两个断点、命中区域 |
| `tui/src/workbench/ansi.rs` | 宽度安全文本：`fit`、`pad`、`clip`、按列切片 |
| `tui/src/workbench/widgets.rs` | 分隔线、盒框、页签、进度条、徽标、spinner、logo、字标、输入框 |
| `tui/src/workbench/panels.rs` | 五个侧栏面板（纯渲染函数） |
| `tui/src/workbench/launchpad.rs` | 首屏 |
| `tui/src/workbench/state.rs` | `WorkbenchData` 及其组成部分 |
| `tui/src/workbench/mouse.rs` | SGR 1006 解析 |
| `tui/src/workbench/theme.rs` | `future-dark` / `future-light` |
| `tui/src/workbench/demo.rs` | 确定性演示数据（截图与测试） |
