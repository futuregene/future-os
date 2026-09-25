# TUI 渲染一致性（Rust vs golden）

> （[English](tests.md)）Rust TUI 移植 P2 渲染核心的逐字节渲染比对：markdown 渲染器
> （pulldown-cmark 适配器）、ChatArea（含流式 prefix-cache 路径）与终端图片辅助。

## 测试工具

```bash
make test-tui-diff            # 或：tui/tests/golden-diff.sh
tui/tests/golden-diff.sh --verbose   # 显示失败的 diff
tui/tests/golden-diff.sh --keep      # 保留 /tmp/future-tui-diff-* 产物
```

TypeScript TUI 已退役（2026-08，与 CLI 同期）：在 TS 源码被删除**之前**，其渲染
输出已按 corpus 用例记录进 `tui/tests/golden/parity-ts.golden`。测试工具把每个
用例经 Rust 实现（`tui/examples/render_parity.rs`，用 cargo 运行）渲染，并与
golden 逐字节比对：

```
<kind>|<name>|<base64(JSON.stringify(result))>
```

Corpus：`tui/tests/parity-corpus.json`（由 `tui/scripts/gen_parity_corpus.py`
生成——用 `python3 tui/scripts/gen_parity_corpus.py` 重新生成）。覆盖：35 个
markdown 用例（标题、表格含列宽收缩算法、引用、代码块、列表、hr、删除线、链接、
定义、html、空行/空格规则、CJK 换行、padding、思考主题、默认文本样式、kitty 图片
透传、tabs），18 个 ChatArea 用例（user/assistant/system/tool/welcome 消息、
thinking + thinking 隐藏、run 状态、队列位置、viewport 滚动、流式
pending/stopped），5 个 chatStream 用例（流式 prefix-cache 路径）与 39 个终端图片
用例（kitty/iTerm2 编码含 >4096 字节分块、PNG/JPEG/GIF/WebP 尺寸嗅探、id
提取/收集/删除、超链接、图片回退、三种 capability 模式下的 renderImage）。

历史 golden 起源于退役前的 TS 树（`bun render-parity.ts <corpus>`）。它仍是参照，
除了以下经过显式评审的修正（2026-09-11），不是无差别重录：

- `markdown|list-code`：w18 BUG-4 修复了在 24 列宽度下继承来的 60 列边框。修正后
  的四行是带两列缩进的两条 22 列边框，而不是断裂的边框与幽灵行。
  `list_code_borders_fit_content_width_without_spurious_rows` 独立对照这一几何要求
  检查宽度 8/24/40/60/80。
- `chat|thinking-hidden`：基线 `fc81c016` 在有答案内容后本就隐藏占位符
  （`chat_area.rs` 现有的 `collapsed_thinking_placeholder_only_while_streaming`
  测试）。corpus 提供了完整答案，所以过期的 TS `Thinking...` 占位符被修正为基线
  的仅答案渲染；该用例没有因 bughunt 补丁改变任何行为。

其余 95 条记录在编辑这两条之前已逐字节验证一致。将来有意的变更必须指明其用例与
证据。

## 编码一致性

字节一致依赖两个事实，已对退役的 TS 实现经验验证：

1. `serde_json::to_string` 的转义与 JS `JSON.stringify` 完全一致（`\u001b`、
   `\u0007`、`\u001f` 等；`\u007f`/DEL 与非 ASCII 两侧都原样通过）。
2. 带 padding 的标准 base64（`base64::general_purpose::STANDARD`）与
   `Buffer.from(..).toString("base64")` 一致。

corpus 保持每个数字结果为整数，因此 JS 数字序列化（会丢掉尾随 `.0`）不可能与
serde_json 分叉。

## 工具抓到过的 bug（均已修复）

- **h1 样式组合顺序** — TS 应用 `theme.heading(bold(underline(t)))`（fg、bold、
  bold、underline）；移植版写成了 `heading(underline(bold(t)))`。把错误顺序编码
  进去的过期单元测试已修正。
- **`dim()` 内的队列后缀** — TS 把 `dim("queued (#n)")` 的后缀渲染在 dim 区间内；
  移植版在后缀之前重置了样式。
- **段落尾随空白** — pulldown-cmark 会把 `Text` 事件的尾随空格裁掉；marked 保留
  它们（一个尾随 `text` token）。段落适配器现在恢复源切片的尾随空白。

## 已接受的分歧（有意为之）

- Overlay 过滤器接受一个 Unicode 标量，含 emoji 等 astral 字符。退役 JS 的
  `key.length === 1` 意外拒绝了代理对；复刻这个 UTF-16 限制不是正确性要求。
- 不受信任的 markdown 控制符（包括解码成 ESC/BEL 的数字实体）在终端样式化之前
  移除；保留终端注入不是一致性要求。可信的内部图片协议行保持独立。

- **块引用/列表项内的链接引用定义**只在顶层保留周围的空行间距（pulldown-cmark
  在容器内对定义的消费方式不同）。记录在 `tui/src/components/markdown.rs`；corpus
  不覆盖。
- 依赖时间戳/随机 id（allocateImageId、newId）的实时渲染排除在 corpus 外——id 显式
  提供。

---

# tmux 屏幕一致性（Rust vs golden，真实应用）

**完整交互式 Rust TUI** 在真实 tmux pane 里跑、对接确定性 mock gRPC agent 的端到端
屏幕比对。这是 P4 门禁，覆盖移植版渲染的每一个屏幕：聊天本身（欢迎横幅、输入、带工具
调用的流式回复、`ctrl+g` 工具体的折叠/展开）、footer/状态读数，以及移植版新增的每个
面板——帮助卡（80x36 下截断、80x72 下全卡、80x36 下滚动）、模型选择器、会话列表、
model scope 与工具菜单、providers 列表（两个 Tab 与编辑表单）、技能浏览器（目录、
搜索过滤、用 escape 清除过滤）、sandbox/permission 面板（overlay、已应用的 tier、
已应用的权限级别）、主题选择器（应用 light、恢复 dark）、`/usage`、`/transcript`
（pager、搜索编辑器与 PageDown/PageUp 键）、`/status`（其卡片承载已退役 `/stats`
面板的会话计数）、`/agent`、`/metrics`、`/snapshot`、`/tool-output`（列表与 diff 正文）、
`/history`，以及文本/图片粘贴、`ctrl+d` 紧凑视图与经 `/sessions` 的分页历史。

## 测试工具

```bash
make test-tui-tmux            # 或：tui/tests/tmux-diff.sh
tui/tests/tmux-diff.sh --record   # 从 Rust pane 重写 golden
tui/tests/tmux-diff.sh --verbose  # 显示失败的 diff
tui/tests/tmux-diff.sh --keep     # 保留 /tmp/future-tui-tmux-* 产物
```

前置条件：`tmux`（它提供 pane 的 PTY）与固定工具链（`rust-toolchain.toml`）——
测试工具自己构建 `future-tui` 与 mock agent。没有 tmux 时会打印
`SKIP: tmux not found` 并以 0 退出，这正是 CI 看到的情况（见下方"本门禁只在本地跑"一节）。

测试工具启动一个 mock agent 实例（`tui/examples/mock_agent`，一个确定性的
`FutureAgent` gRPC 服务），开一个 tmux 窗口（80x36 pane）跑 Rust TUI
（`future-tui`），用按键驱动它。**共 56 项检查 = 55 个 golden 屏幕 + Ctrl+C 退出**，
对应同一个固定会话的每个步骤（每个屏幕都叠在前面步骤产生的聊天之上，所以顺序是
golden 的一部分）。按功能分组：

1. 聊天 —— `welcome`、`typed`、`reply`、`tool-expanded` / `tool-collapsed`
   （`ctrl+g`）、`status`
2. 帮助 —— 80x36 的 `help-overlay` / `help-closed`、80x72 的 `help-full`
   （必须在 resize **之后**才打开卡片）、`help-scrolled`（80x36 下按 PageDown）
3. 选择器 —— `model-overlay` / `model-closed`、`sessions-*`、`models-*`、
   `tools-*`
4. providers —— `providers-builtin`、`providers-custom`（Tab）、
   `providers-form`（Enter）、`providers-closed`
5. 技能 —— `skills-overlay`、`skills-filtered`、`skills-escape`（第一次 escape
   清掉查询串、面板仍开着）、`skills-closed`
6. sandbox / permission —— `sandbox-overlay`、`sandbox-tier`、
   `permission-overlay`、`permission-applied`
7. 主题 —— `theme-overlay`、`theme-light`、`theme-dark-restored`
8. 读数 —— `usage-overlay`、`transcript-pager`、`transcript-page-down`、
   `transcript-page-up`、`transcript-search`、`agent`、`metrics`、
   `snapshot`、`tool-output-list`、`tool-output-diff`、`history`（独立 `/stats`
   面板已移除，其计数是第 1 步 `/status` 卡片的一节）
9. 粘贴 —— `paste-folded` / `paste-sent`（输入框内一个占位符，线上收到完整粘贴文本），
   以及图片路径 `paste-image-attached` / `paste-image-two` / `paste-image-renumbered` /
   `paste-image-sent`
10. 紧凑视图 —— `burst-expanded` / `burst-folded` / `burst-restored`
    （`ctrl+d` 把连续三次 read 折叠成一行再恢复）
11. 分页历史 —— `history-tail` / `history-older`（切到另一个 mock 会话并用 PageUp 拉取更早一页）
12. `ctrl-c` —— TUI 必须以状态 0 退出

权威清单就是脚本本身：
`grep -nE '^step |^step_when ' tui/tests/tmux-diff.sh`。

**内容轮询，而非固定 sleep。** `step_when <scenario> "<marker>"` 轮询 pane，等到
"证明目标画面已画出"的字面文本（例如 `Usage · mock-model`、`Tool permissions`、
`current: Workspace`，或合并后技能目录的 `not installed`）才截图——这正是 golden
在负载高的机器上依然稳定的原因。以**状态迁移**为证据、而不是以某个屏幕为证据的场景
用 `require_text`，然后靠断言。

**Golden 测试**：`tui/tests/golden/<scenario>.txt` 存 55 个参照屏幕，每个都用
`capture-pane -p -e` 抓取——**含 ANSI**，整体逐字节比对。它们**全部**由 `--record`
录自 **Rust** pane；文件名可追溯到移植提交 `1467a1cd`（2026-08-07），当时最初十个录自
退役的 TypeScript pane，而 TS 源码早已删除。Verify 模式把 Rust pane 与 golden 对比，
因此移植中的分叉或屏幕的有意变更（必须与重录的 golden 一起提交）都会被抓住。有意变更
后用 `--record` 重新生成。

**录制拒绝固化坏屏幕。** 在 `--record` 下，`step`、`step_when` 与断言都走同一个守
卫：标记始终没渲染出来、屏幕自己的断言失败、或 pane 匹配 TUI 的失败措辞（单独成行的
`Failed to …` / `Error: …`、Rust panic、stub 的 `not implemented:`）都是 **FATAL**
而不是 golden——坏面板不可能被录一次然后永远通过。

**golden 说不出的状态迁移**在两种模式下都断言，并报为 `ASSERT-FAIL`：第一次 escape
让带过滤的面板保持打开且清空了查询串、PageDown 移动了 pager 的视口而 PageUp 走了回
去、第二次 escape 关掉了它、已应用的权限级别到达了聊天。golden 只能钉住像素，只有
断言才能说"这个键起了作用"。

## 确定性说明

- Mock agent 响应是固定的（会话状态、模型列表、会话列表、流式回复），所以 TUI
  每次运行渲染相同内容。
- pane 跑的是本运行临时目录里 `future-tui` 的**副本**，旁边放一个 **stub `future`
  二进制**。`/skills` 会调起 `future skills list --json`，而该二进制按
  `<exe_dir>/future` 优先解析：紧邻 `target/debug/future-tui` 的位置，开发者机器上
  通常是 `target/debug/future` 或已安装的 `future`，它会对着隔离的 HOME 运行、一直
  挂到技能操作超时，然后往面板背后的聊天里丢一行**依赖墙钟**的错误（80 列下 76 列宽
  的卡片，左侧两列正好显示那部分聊天）。stub 给出固定目录——且启动前会被预检——所以
  任何机器上目录都是固定文档。
- 测试工具在抓取前等待横幅（`future-tui v`），`step` 睡 1 秒等 33 ms 的渲染调度器；
  `step_when` 则等自己的标记，不靠 sleep。
- 斜杠命令用 `submit_cmd` 辅助：输入 → 等 20 ms 自动补全防抖 → Enter（应用弹窗
  选择）→ Enter（提交）。单次快速 Enter 会被自动补全弹窗吃掉。
- pager 场景不假设页大小：一页是 `视口 − 1` 行并截断在最后一页，所以步长是内容的
  属性。证据是 pager 自己的 `…%` 读数加上屏幕的字节指纹，断言是"读数变大了""字节变
  了""PageUp 走了回去"——写死 `PageUp → 0%` 会报出一个并不存在的缺陷。
- golden 是带 ANSI 的 pane（`capture-pane -p -e`），而 `step_when` / `wait_text`
  的轮询与 `expect_pane_*` 断言读的是**纯文本** pane（`capture-pane -p`）。这个差异
  是有意的——标记应该匹配重绘无法藏进转义序列的文本——但也意味着标记在屏幕上出现时，
  golden 仍可能在样式或布局上不同。
- 清理是刻意限定范围的：`tmux kill-session` 会关掉 pane 并向其进程组发 SIGHUP，但
  终端已消失的 TUI 会忽略 SIGHUP/SIGINT/SIGTERM（它的退出路径需要事件循环，而后者
  已停止推进），所以测试工具先 SIGKILL pane 自己的子进程（精确的 `ppid` 查找），再只
  扫描**本次运行**的 gRPC 地址（`grpc-addr 127.0.0.1:<每次运行的端口>`）。不要把这里
  换成无差别的 `pkill future-tui`——本仓库同时有多个会话在工作，那会误伤它们的实例。

## 本门禁只在本地跑，所以要主动维护

CI 不跑它：runner 上没有 tmux，测试工具会打印 `SKIP: tmux not found …` 并以 0 退出，
而 `Makefile` 也把 `*-diff` / `*-tmux` 目标列为手动的迁移验收门禁。纯本地门禁因此会
**静默腐坏**，而它确实腐坏了——golden 最后写入于 2026-08-07（移植提交 `1467a1cd`），
而欢迎行在 2026-08-09 加入了 `ctrl+o expand/collapse` 提示（`c1c946ef`），于是从那
天起每个场景都是红的，持续 **约 6 周**，直到这次面板重录。保持它可信的两个习惯：

- 只要屏幕有**任何**有意变更，就用 `--record` 重录并逐场景复核漂移，且把 golden 与
  改动一起提交；
- 动到面板时就跑它，而不是只在动渲染器时才跑。

过期还会让场景抓到**错误的**屏幕，而 golden 并不会漂移成肉眼可见的损坏：transcript
pager 的搜索编辑器学会抢占第一次 escape 后，该场景里仅剩的那次 escape 只关掉了搜索
编辑器，于是当时排在那里的 `/stats`（现已移除）及其后各个场景录下的都是 transcript，
而不是它们声称的面板。
上面的状态迁移断言正是用来抓这类漂移的。

## 工具抓到过的 bug（均已修复）

- **Footer token 统计的 JS truthiness** — TS 只在值 truthy 时渲染 `↑/↓/R/W` token
  统计（`if (this.data.tokensCacheR)`），所以 0 值被跳过。移植版用了
  `if let Some(n)`，把 `Some(0)` 渲染成 `R0 W0`；P1 footer 一致性测试只用过非零值。
  （分项 `↑/↓/R/W` 的排布未变；会话**累计**总量现在带 `Σ` 前缀，以免被误读为当前
  上下文——`/usage` 面板把后者标为 `Cumulative tokens (resent each call)`。）
- **状态 overlay 的会话/模型回退** — TS 渲染 `**Session:** ${s.sessionId ||
  "(none)"}`（二选一）；移植版总是追加 ` or (none)`。`**Model:**` 同理，为
  ` or (unknown)`。
