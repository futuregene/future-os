# tui-rust-port——最终验收报告

> 本文是历史快照 [tui-rust-port — Final Acceptance Report](./tui-rust-port-acceptance.md)（2026-08-07，commit `1467a1cd`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

> 历史迁移快照：下方结果与源路径适用于记录的候选/日期，不适用于后续版本。当前传输与启动行为记录在 [TUI 指南](../../guide/tui.md)。重新验证新变更；不要自动沿用此候选的 PASS。

状态：**已验收**——2026-08-07 验证（合并后 `claude/tui-rust-port` 分支上的最终重验，PR #114）。
范围：`tui/` 的 1:1 TypeScript→Rust 移植 → **`tui/`** 处的 Rust crate（bin `future-tui`，lib `future_tui`）——相同的 UI 渲染 / 按键处理 / 交互 / 参数解析 / 帮助文本，带**自实现的终端后端**（无 crossterm；POSIX 上 `libc`，Windows 上 `windows-sys` 控制台 API）、移植的 TS 单元测试、golden 渲染工具与 tmux 屏幕一致性测试。TypeScript TUI 已**退役**（与 CLI 相同，2026-08）：其渲染 + 屏幕输出作为 golden 提交在 `tui/tests/golden/` 下，于 TS 源码删除前录制。该 crate 消费 `future-rpc::proto`（唯一 proto 代码生成所有者，PR #112），而非拥有生成代码。

---

## 1. 验证结果（全部门禁绿，最终重跑 2026-08-07）

| 门禁 | 命令 | 结果 |
|---|---|---|
| tmux 屏幕一致性（Rust vs golden） | `make test-tui-tmux`（`tui/tests/tmux-diff.sh`） | **11 通过 / 0 失败** |
| 渲染奇偶（Rust vs TS 录制 golden） | `make test-tui-diff`（`tui/tests/diff-ts-rust.sh`） | **97 / 97 用例字节一致** |
| tui-rust 单元测试 | `cargo test -p tui-rust` | **411 通过 / 0 失败** |
| Workspace 测试 | `cargo test --workspace`（rustup 1.97.0） | **通过 / 0 失败** |
| Workspace clippy（CI 标志） | `cargo clippy --workspace --all-targets -- -D warnings` | 干净 |
| 格式 | `cargo fmt --check` | 干净 |
| 三平台 Rust 编译检查 | `cargo check -p tui-rust --target x86_64-pc-windows-msvc` | 通过（Windows 后端） |
| Worktree 卫生 | `git status` 干净；`main` 在 PR 合并前未动 | 已验证 |

tmux 工具启动一个确定性 mock agent 实例（`tui/examples/mock_agent`，一个 `FutureAgent` gRPC 服务器），打开一个 tmux 窗口（80×36 pane）运行 Rust TUI（`future-tui`），用按键驱动它，并在每一步将 `capture-pane -p -e` 屏幕与 golden 文件字节比较——golden 在 TS 源码删除前从退役的 TypeScript TUI pane 录制（移植中的分歧或有意屏幕变更——必须与重新录制的 golden 一起提交——都会被捕获）。

### tmux 场景清单（11 个场景）

| 场景 | 覆盖 |
|---|---|
| `welcome` | 连接后空闲屏幕：banner `future-tui v0.0.0-mock`、页脚、提示符 |
| `typed` | 带输入文本的输入行 |
| `reply` | 提交的提示 + 流式 markdown 回复（聊天区） |
| `status` | `/status` 覆盖层（`get_state` + `list_models`） |
| `help-overlay` / `help-closed` | `/help` 卡片 + Escape |
| `model-overlay` / `model-closed` | `/model` 选择器 + Escape |
| `sessions-overlay` / `sessions-closed` | `/sessions` 覆盖层 + Escape |
| `ctrl-c` | TUI 以状态 0 退出（raw-mode `\x03` → `handle_interrupt`） |

### 渲染奇谓语料清单（97 用例）

| 组 | 用例数 | 覆盖 |
|---|---|---|
| markdown | 35 | 标题（含 h1 样式组合顺序）、表格（列宽收缩 + 带符号余数分配）、引用、代码围栏、列表、hr、删除线、链接、定义、html、空行/空格规则、CJK 换行、内边距、thinking 主题、默认文本样式、kitty-image 直通、制表符 |
| chat | 18 | user/assistant/system/tool/welcome 消息、thinking（+隐藏）、run 状态、队列位置、视口滚动、流式 pending/stopped |
| chatStream | 5 | 流式前缀缓存路径（`findStreamCut` / `streamCaches`） |
| image | 39 | kitty/iTerm2 编码含 >4096 字节分块、PNG/JPEG/GIF/WebP 嗅探、id 提取/收集/删除、超链接、图像回退、三种能力模式下的 `renderImage` |

语料由 `tui/scripts/gen_parity_corpus.py` 生成 → 共享 `tui/tests/parity-corpus.json`。Rust 侧（`tui/examples/render_parity.rs`）渲染每个用例，工具与 `tui/tests/golden/parity-ts.golden` 字节比较——TS 输出（`tui/render-parity.ts`，bun）在 TS 源码删除前录制。每用例一行：`<kind>|<name>|<base64(JSON.stringify(result))>`。

### 单元测试增长（移植的 TS 测试 + 新增）

| 阶段 | 数量 | 注记 |
|---|---|---|
| P0 脚手架 | 96 | stdin-buffer / keys / theme / utils / version |
| P1 组件 | 239 | input-bugs (26)、footer (17)、help-screen (3)、SelectList (22)、ScopedModelsSelector (11)、display-fixes（AutocompletePopup + Input.setValue）、keybindings (15)、autocomplete providers/manager (17)、tui.rs overlay (8)、rpc types (2) |
| P2 渲染 | 362 | markdown / chat-area / terminal-image 单元 |
| P3 应用层 | 409 | app (21)、index (16)、grpc_client（8 含 2 个进程内 mock-server 集成测试） |
| P4 最终 | **411** | +footer JS-truthiness、+app apply_status 回退 |

---

## 2. 分歧列表（已知 / 已接受）

1. **blockquote/列表项内的链接引用定义**只在顶层保留其周围空行间距——pulldown-cmark 在容器内消费定义的方式与 `marked` 不同。记录在 `tui/src/components/markdown.rs`；**语料未覆盖**。
2. **依赖时间戳 / 随机 id 的实时渲染**（`allocateImageId`、`newId`）排除在渲染语料外——id 显式提供。
3. **Windows 后端只做类型检查，未运行时测试**：`windows-sys` 控制台实现（raw mode / VT input+output / 经 window-size 事件调整尺寸）通过 `cargo check --target x86_64-pc-windows-msvc`（CI 三平台门禁），但未在真实 Windows 控制台上运行（记录在 `tui/src/terminal_windows.rs`）。
4. **TS 事件循环机制映射到应用循环**而非字面移植：20 ms 自动补全防抖 / `AbortSignal` 取消表达为应用的 33 ms 渲染调度器 + 20 ms 自动补全防抖计时器——可观察行为（防抖、取消、排序）相同，由 tmux 场景验证。

## 3. 工具捕获的 bug（全部修复，回归测试）

移植中经字节比较发现并修复的渲染/行为分歧：

- **h1 标题样式组合顺序**——TS 应用 `heading(bold(underline(t)))`（fg、bold、bold、underline）；移植是 `heading(underline(bold(t)))`。一个过期单元测试编码了错误顺序；已修正。
- **`dim()` 内的排队后缀**——TS 以 dim 跨度内后缀渲染 `dim("queued (#n)")`；移植在后缀前重置了样式。
- **段落尾随空白**——pulldown-cmark 从 `Text` 事件裁剪尾随空格；`marked` 保留它们。段落适配器恢复源切片的尾随空白。
- **表格列宽“收缩适配” + 余数分配**——必须使用带符号算术（JS 中 `remaining - allocated` 变负并跳过循环；`usize` 下溢 → debug panic）。
- **页脚 token 统计 JS truthiness**——TS 只在 truthy 时渲染 `↑/↓/R/W` 统计，因此零值被跳过；移植对 `Some(0)` 渲染 `R0 W0`。现在四个统计都用 `n > 0` 守卫。
- **状态覆盖层会话/模型回退**——TS 渲染 `**Session:** ${s.sessionId || "(none)"}`（二选一）；移植总是追加 ` or (none)` / ` or (unknown)`。
- **welcome 后虚假的“Connection to agent lost”闪动**——5 秒连接看门狗必须只武装一次并在首个数据时清除（绝不重新武装；长期死流是 10 秒心跳的职责），且 poke 驱动的重订阅必须静默（TS 取消旧流并忽略其过期 end/error 处理器）。两者均修复并带回归测试（进程内 tonic mock 服务器：`idle_stream_does_not_flap_after_first_data`、`session_change_resubscribes_silently`）。

## 4. 交付内容（`claude/tui-rust-port` 上的提交，领先 origin/main）

- `cfa22727` P0 脚手架：RESEARCH.md（框架比较 → 自实现）、workspace crate（bin `future-tui`，lib `future_tui`）、镜像 `scripts/version.mjs` 的 build.rs、自实现 POSIX 终端后端（raw mode、TIOCGWINSZ、sigaction+self-pipe、poll(2) 读取线程、kitty-query、进度保活）、stdin-buffer/keys/theme/utils 移植、help.rs 逐字、main.rs 接线。96 个单元测试。
- `70a82c91` + `08ee59f0` P1：组件层——tui.rs 核心、input（UTF-16 游标语义）、autocomplete（SlashCommand/FilePath/Attachment providers + 弹窗）、select_list + scoped_models_selector、footer、help_screen、keybindings。239 个单元测试。
- `9a8c422b` P2：渲染核心——markdown（pulldown-cmark 适配器）、chat_area（延迟重渲染 + 流式前缀缓存）、terminal_image（kitty/iTerm2）、render-parity 工具 97/97。362 个单元测试。
- `6856afae` + `4e705ecc` P3：应用层——grpc_client（tonic、持久 StreamEvents 管理器、截止时间有界 unary）、app.rs（全部 19 个斜杠命令、覆盖层、agent 事件处理、设置持久化、基于 diff 的渲染管线）、index.rs（1:1 parseArgs + print 模式 + list-models + 交互循环）。409 个单元测试。
- `1c26fe83` + `86cc9c3c` P4：tmux 屏幕一致性工具 + golden（11 场景）、mock agent 示例、工具 trap 杀死自己的 mock agent。411 个单元测试。
- `76ba8c58` Windows 终端后端（`tui/src/terminal_windows.rs`，windows-sys 控制台 API）：raw mode / VT input+output / window-size-event 调整尺寸检测；POSIX 路径逐字移动到 `terminal_posix.rs`，位于共享 `ReadWait` 分发之后——修复 CI 三平台检查；重构后重验 tmux 11/11 + parity 97/97。
- `8f592924` 在 TS 参考与 Rust 移植中都去掉 `[Context]` welcome 屏幕块（用户请求）；`contextFiles` 状态保留给 `/status`/reload。
- TS 退役 + 提升（合并进 PR）：TypeScript TUI 源码删除，`tui/rust/*` 提升为 `tui/`，工具转换为 golden 模式（`tui/tests/golden/parity-ts.golden` 删除前录制；tmux golden 对 Rust pane 重验），crate 消费 `future-rpc::proto`（唯一代码生成所有者，PR #112），Makefile/CI/docs 对 TUI 改为纯 cargo。

## 5. 重跑门禁

```bash
make test-tui-rust            # cargo test -p tui-rust (411)
make test-tui-diff            # 渲染奇偶 vs golden，97/97（rustup 1.97.0；无 bun）
make test-tui-tmux            # tmux 屏幕一致性 vs golden，11/11（交互式
                              #   终端；缺 tmux 时优雅 SKIP）
rustup run 1.97.0 cargo clippy --workspace --all-targets -- -D warnings
rustup run 1.97.0 cargo test --workspace
rustup run 1.97.0 cargo fmt --check
rustup run 1.97.0 cargo check -p tui-rust --target x86_64-pc-windows-msvc
```

注：Homebrew cargo 忽略 `rust-toolchain.toml`——总是在 `rustup run 1.97.0` 下运行。Unset 泄漏的 `FUTURE_VERSION`（`env -u FUTURE_VERSION`）使构建（与 Rust `--version` 字符串）确定性。`make test-tui-tmux` 必须在有 tmux + 可用 PTY 的环境运行（本地机器，非 headless CI）。TypeScript TUI 已退役；golden 是参考（仅在有意更改屏幕时用 `tui/tests/diff-ts-rust.sh --record` / `tui/tests/tmux-diff.sh --record` 重录）。
