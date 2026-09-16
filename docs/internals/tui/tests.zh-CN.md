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
屏幕比对。这是 P4 门禁：欢迎横幅、footer（含 token/cache truthiness）、状态
overlay、帮助卡、模型选择器、会话 overlay、提示回复与 Ctrl+C 退出。

## 测试工具

```bash
make test-tui-tmux            # 或：tui/tests/tmux-diff.sh
tui/tests/tmux-diff.sh --record   # 从 Rust pane 重写 golden
tui/tests/tmux-diff.sh --verbose  # 显示失败的 diff
tui/tests/tmux-diff.sh --keep     # 保留 /tmp/future-tui-tmux-* 产物
```

测试工具启动一个 mock agent 实例（`tui/examples/mock_agent`，一个确定性的
`FutureAgent` gRPC 服务），开一个 tmux 窗口（80x36 pane）跑 Rust TUI
（`future-tui`），用按键驱动它；每个场景步骤用 `capture-pane -p -e` 抓取 pane 并与
golden 逐字节比对：

1. `welcome` — 连接后的空闲屏
2. `typed` — 带文本的输入行
3. `reply` — 已提交提示 + 流式 markdown 回复
4. `status` — `/status` overlay（get_state + list_models）
5. `help-overlay` / `help-closed` — `/help` 卡片 + Escape
6. `model-overlay` / `model-closed` — `/model` 选择器 + Escape
7. `sessions-overlay` / `sessions-closed` — `/sessions` + Escape
8. `ctrl-c` — TUI 必须以状态 0 退出

**Golden 测试**：`tui/tests/golden/<scenario>.txt` 记录参照屏幕——在 TS 源码被删除
**之前**从退役 TypeScript TUI 的 pane 抓取，随后与 Rust pane 逐字节重新验证。
Verify 模式把 Rust pane 与 golden 对比，因此移植中的分叉或屏幕的有意变更（必须与
重录的 golden 一起提交）都会被抓住。有意变更后用 `--record` 重新生成（从 Rust pane
记录）。

## 确定性说明

- Mock agent 响应是固定的（会话状态、模型列表、会话列表、流式回复），所以 TUI
  每次运行渲染相同内容。
- 测试工具在抓取前等待横幅（`future-tui v`），每个步骤睡 1 秒等 33 ms 的渲染
  调度器。
- 斜杠命令用 `submit_cmd` 辅助：输入 → 等 20 ms 自动补全防抖 → Enter（应用弹窗
  选择）→ Enter（提交）。单次快速 Enter 会被自动补全弹窗吃掉。

## 工具抓到过的 bug（均已修复）

- **Footer token 统计的 JS truthiness** — TS 只在值 truthy 时渲染 `↑/↓/R/W` token
  统计（`if (this.data.tokensCacheR)`），所以 0 值被跳过。移植版用了
  `if let Some(n)`，把 `Some(0)` 渲染成 `R0 W0`；P1 footer 一致性测试只用过非零值。
- **状态 overlay 的会话/模型回退** — TS 渲染 `**Session:** ${s.sessionId ||
  "(none)"}`（二选一）；移植版总是追加 ` or (none)`。`**Model:**` 同理，为
  ` or (unknown)`。
