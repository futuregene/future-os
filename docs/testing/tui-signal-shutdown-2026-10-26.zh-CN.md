# 终端消失之后 `future tui` 不再处理 `SIGTERM`（2026-10-26）

一次性记录，不是设计文档：复现方式、根因（带行号）、修法，以及测试覆盖了什么、没覆盖
什么。下面每个数字都是本机实测；命令都写出来了，可以逐行复核。

这条缺陷是修事件循环帧成本的那个会话（`2a856ab72`）交接出来的，它当时明确没动：
*"`SIGTERM` in the no-tty state is still ignored, identically before and after: the
terminal reader exits at EOF, so nothing polls the signal self-pipe — pre-existing
terminal-lifecycle bug."*

## 0. 结论

| | |
|---|---|
| 状态 | 一个终端已经消失的 `future tui` 仍在运行：终止信号的处理器还装着，而它们写入的管道没人读了 |
| 后果 | `SIGTERM` / `SIGINT` / `SIGHUP` / `SIGQUIT` 被**捕获后静默丢弃**；`kill` 返回 0，进程还在；只有 `SIGKILL` 能去掉它 |
| 根因 | `tui/src/terminal.rs:372`（`if n == 0 { break; }`，基线 `ec891652f`）——reader 线程是 self-pipe 唯一的读者，而它在 EOF 时退出了 |
| 修法 | stdin 消失后 reader 只保留 self-pipe 继续读；终止信号另外武装一个退出上限，让"清理卡住"无法变回"永不退出" |
| 覆盖 | `tui/src/terminal.rs` 一条单元测试 + `tui/tests/signal_shutdown.rs`（六条真进程测试） |
| 未覆盖 | reader 阻塞在"没人排空的终端写入"里；`future agent` 自己的信号路径（只做了代码确认，它没有同一个洞，见 §6） |

## 1. 复现

必须把终端从**正在运行的** TUI 手里拿走，而不是一开始就不给——这是两个不同的状态，
只有一个是坏的。

| 状态 | 怎么产生 | 行为 |
|---|---|---|
| **启动时就没有 tty** | `future tui </dev/null` | `Terminal::start` 直接拒绝（`tui/src/terminal.rs:328`，`NotConnected`），进程立刻以退出码 1 结束，打印 `terminal: stdin is not a TTY — the interactive TUI needs a terminal`。**不是缺陷**：没有东西滞留，而且处理器从未安装过，信号的默认动作仍然生效 |
| **tty 后来消失** | 终端模拟器 / `tmux` 面板 / SSH 会话 / 测试夹具在 TUI 运行时消失（杀掉 `script -q /dev/null …` 里的 `script` 就是一种；交接报告里那 6 个孤儿就是这么来的） | `start` 已经成功、处理器已经装上；stdin 现在读回 EOF/EIO；信号被吞掉。**这才是缺陷** |

测量用的父进程在一条新 PTY 上启动二进制、关掉 master（macOS 上子进程的 stdin 变 EOF，
Linux 上是 `EIO`）、等一会儿，然后每个进程只发一个信号并读取退出状态。单用
`</dev/null>` 复现不出来，动手前值得先知道这一点。

```bash
# 修复前 / 修复后两个二进制，构建参数一致（FUTURE_VERSION=0.0.2-sigtest）
for s in TERM INT HUP QUIT; do python3 sig-one.py /tmp/future-prefix "$s"; done
for s in TERM INT HUP QUIT; do python3 sig-one.py /tmp/future-postfix "$s"; done
```

修复前的二进制是基线提交、只把本次修复的 EOF 分支改回原来的 `break`（让新测试变红的
就是这处改动，见 §5）。

### 1.1 修复前后实测对比（死 PTY，每个信号一个进程）

| 信号 | 修复前（`ec891652f`） | 修复后（本分支） |
|---|---|---|
| `SIGTERM` | **被忽略**——12 秒后仍在，只能 SIGKILL 收尾 | 0.1 秒内退出，退出码 **0** |
| `SIGINT` | **被忽略**——同上 | 0.1 秒内退出，退出码 **0** |
| `SIGHUP` | **被忽略**——同上 | 0.1 秒内退出，退出码 **0** |
| `SIGQUIT` | **被忽略**——同上 | 0.1 秒内退出，退出码 **0** |

退出码是 0（不是"被信号 15 杀掉"）是刻意的：信号被转成应用自己的 ctrl+c 路径，于是
终端被恢复、这次运行按普通退出结束。这对**活终端**原本就是这样（实测：活 PTY 上
`SIGTERM` 在修复前后都是退出码 0）。

任务里点到的另外两个状态在两个构建里都正常：

| 状态 | 修复前 | 修复后 |
|---|---|---|
| 活 PTY、agent 已连接 | `SIGTERM` → 退出，码 0 | 同上 |
| 活 PTY、**agent 连不上**（`FUTURE_HOME` 指向没有 agent 的目录） | `SIGTERM` → 退出，码 0 | 同上 |

### 1.2 对交接报告的一处更正

那份报告还记了 100% CPU。那是它在 00:36 看到的那批孤儿——在帧成本修复合入之前。
现在树里有 `2a856ab72`，同一个死 PTY 进程在修复前和修复后的二进制上都只花
**5 秒 0.01 秒 CPU**（约 0%）。信号这个洞与它相互独立：进程留着几乎不花代价，而这
反而更容易让人忘掉它。

## 2. 根因

信号处理是 self-pipe：`enable_raw` 用 `sigaction` 装处理器
（`tui/src/terminal_posix.rs:190`，`install_signal_handlers` 在 `:153`），处理器只做
一次 async-signal-safe 的 `write(2)`，把信号号写进一条非阻塞管道
（`tui/src/terminal_posix.rs:53`）。真正的活都发生在 **reader 线程**上——它用
`poll(2)` 同时盯 `{stdin, 信号管道}`（`tui/src/terminal_posix.rs:250`，分发在
`tui/src/terminal.rs:400` 与 `:407`）。

而它也是唯一会提前离开循环的东西：

- `tui/src/terminal.rs:363`（`ec891652f`）：读出错 → `break`；
- `tui/src/terminal.rs:372`（`ec891652f`）：**stdin EOF → `break`**——PTY master 关掉
  的 TUI 走的就是这一条。

从它 `break` 的那一刻起，管道就没人 `poll` 了。处理器仍然装着（恢复 `SIG_DFL` 的
`restore_raw` 只从 `stop()` 进来），所以 `SIGTERM` 也不再执行默认动作；它被写进一条
没有读者的管道，然后被忘记。主循环同样不知道 stdin 没了，于是进程就那么待着——这是
`SIGTERM` 既不处理、也不默认的唯一状态，只有 `SIGKILL` 管用。

## 3. 修法

两处，都在终端层；输入、绘制、排版、帧成本一律没动，`terminal_windows.rs` 只为 §3.2
需要的那一个探测多了一个方法。

### 3.1 reader 比 stdin 活得久

`tui/src/terminal.rs:81`（`wait_next`）——stdin 报 EOF 或读错误时循环不再 `break`：
它置上 `stdin_gone`（`:450`、`:476`）继续跑。变的只是 poll 集合，不是循环本身：

- `tui/src/terminal_posix.rs:321` 的 `wait_for_signals` **只**盯信号管道。fd 0 必须
  退出集合——挂断的 PTY 会一直是可读的，留着就是空转——而管道必须留在里面，因为处理器
  还装着。
- 分发分支一行没动，所以 `SIGWINCH`、退出回调、恢复后重新抛出这三条在这个状态下都照
  常工作。正是退出回调让信号变成普通的优雅退出，而不是被杀。

`stop()` 不受影响：它置 `stop_flag` 并 `wake()` 管道，而那正是"只等信号"时的等待对象
（`tui/src/terminal.rs:619`）。

### 3.2 清理不能让"永不退出"重新出现

优雅路径是"把 ctrl+c 交给应用，让它自己退"。它管用时这是对的，卡住时就不是任何
路径了——卡在读 agent、卡在往没人排空的终端写、或者循环已经不在了。所以捕获到终止
信号后，在调用回调**之前**武装一个上限（`tui/src/terminal.rs:120`
`arm_exit_deadline`，调用点在 `:525`）：

- `SIGNAL_EXIT_DEADLINE`（生产 10 秒；测试等不起十秒，用
  `FUTURE_TUI_EXIT_DEADLINE_MS`）之后，线程查 `Backend::raw_active()`
  （`tui/src/terminal_posix.rs:341`、`terminal_windows.rs`）；
- 终端**仍在 raw 模式**，说明清理还没走到恢复那一步：执行既有的兜底
  `restore_terminal_for_exit`（`tui/src/terminal.rs:979`——光标、括号粘贴、鼠标与
  键盘协议、进度指示、termios、stderr），再按信号的默认处置去死，退出状态因此仍然写着
  "SIGTERM"；
- raw 模式已经关掉，说明终端已经交还，后面的退出动作（设置落盘、断开 agent、释放
  单实例锁）继续做完。

上限线程只睡觉和写，永远不会阻塞在优雅路径卡住的那个地方。也没有把 `SIGTERM` 改回
`SIG_DFL` 当捷径：处理器一直装着，先跑的永远是优雅路径。

## 4. 值得知道的行为

- EOF 之后到达的信号仍然进应用的退出回调，也就是和以前同一条优雅路径：恢复终端、落盘
  会话、断开 agent、释放锁（端到端验证过：退出状态是 0，不是被信号杀掉）。
- EOF 之后 reader 每秒醒 10 次（循环本来就有 100ms 的 poll 上限）；它不重绘任何东西。
- EOF 本身仍然不是退出。这不属于本次修复：没有键盘的界面还有屏幕和 agent 事件流，把
  "终端没了"当成"进程该没了"是交互变更，不是生命周期修复。

## 5. 测试

| 测试 | 位置 | 钉住什么 |
|---|---|---|
| `reader_loop_keeps_handling_signals_after_stdin_eof` | `tui/src/terminal.rs`（单元，真 PTY） | master 关掉之后，`SIGTERM` 仍能在 2 秒内到达退出回调，且 `stop()` 仍能 join reader 线程 |
| `sigterm_stops_a_tui_whose_terminal_is_gone` | `tui/tests/signal_shutdown.rs` | **真进程**：PTY 上启动，关掉 master → `SIGTERM` → 15 秒内退出，码 0 |
| `sigint_and_sighup_stop_it_too` | 同上 | `SIGINT`、`SIGHUP` 同理，每个信号一个进程 |
| `a_blocked_cleanup_still_exits_on_sigterm` | 同上 | 永不返回的退出回调会被上限强制结束（测试里设成 800ms）；同时断言进程确实走了、且不是在"上限来得及跑"之前走的 |
| `a_tui_with_no_tty_does_not_linger` | 同上 | 没有 tty 的状态自己退出，不留残余 |
| `eof_alone_does_not_stop_the_process` | 同上 | 关掉终端后等 500ms：不退出。这让上面几条保持诚实——如果 EOF 本身就会结束进程，它们不碰信号路径也能过 |

集成测试里的子进程是测试二进制自己重跑（`--exact signal_shutdown_child`），配独立的
`FUTURE_HOME`、stdin 上的 PTY（带 `FD_CLOEXEC`，所以父进程那次 close 真的是最后一个
master）以及临时文件里的输出。测试 panic 时 `Drop` 会 SIGKILL 并回收子进程，所以红的
那次运行不会留下正在被修的那种孤儿。

**是证伪过的，不只是绿的：** 把 EOF 分支改回原来的 `break`
（`ec891652f` 的 `tui/src/terminal.rs:372`），
`reader_loop_keeps_handling_signals_after_stdin_eof`、
`sigterm_stops_a_tui_whose_terminal_is_gone`、`sigint_and_sighup_stop_it_too`、
`a_blocked_cleanup_still_exits_on_sigterm` 会变红（实测；另外两条按预期保持绿），
改回来就重新变绿。

### 测试没覆盖什么

- **reader 阻塞在写里。** 如果 reader 卡在往缓冲区已满的终端 `write(2)`（例如被挂起的
  `tmux` 客户端），信号字节就一直躺在管道里没人读，上限也就不会被武装。那是另一种
  卡死——本次修复的边界是"reader 还在跑"。
- **`future agent` / `future host`。** §6 是代码确认，不是测试。
- **Windows。** 控制台路径没有信号；Ctrl+C 和控制台关闭是以输入事件的形式到 reader
  线程的，所以 `wait_next` 在那里什么也不做。`terminal_windows.rs` 里新增的
  `raw_active()` 只由 CI 编译（本机对 `future-tui` 做交叉 clippy 会停在
  `libsqlite3-sys` 的 C 构建上）。

## 6. `future agent` / `future host` 没有同一个洞

两者都不构造 `Terminal`（没有 raw 模式、没有 self-pipe），所以 reader 线程那套结构根本
不存在：

- headless 入口（`cli/src/headless/mod.rs:780` 的 `ShutdownSignals`）用
  `tokio::signal` 拿 SIGINT/SIGTERM/SIGHUP（`:797`），信号流与服务循环在 `select!` 里
  对撞（`:263` `until_shutdown`），并把是哪个信号结束的运行记下来（`StopCause`，`:213`）；
- `future agent`（`agent/src/cli/shutdown.rs:24`）在专用线程和专用 runtime 上只看
  ctrl+c，5 秒宽限后直接 `std::process::exit(130)`，不等可能已经卡住的收尾。它根本不
  捕获 `SIGTERM`，所以 SIGTERM 走默认动作直接终止进程——与本报告是相反的失效方向
  （没有清理，但确实停下）。

两边都没有改动。

## 7. 环境

| | |
|---|---|
| 机器 | macOS 26.6.2 (25G83)，Apple silicon |
| 基线提交 | `origin/dev-huichen` 的 `ec891652f`；分支 `fix/tui-signal-shutdown` |
| 工具链 | 仓库 pin 的 `1.97.0`（`rust-toolchain.toml`） |
| 二进制 | `cargo build -p future-cli`（debug），修复前与修复后都用 `FUTURE_VERSION=0.0.2-sigtest` |
| 检查 | `cargo test -p future-tui`（1056 条 lib 测试 + 各集成目标）、`cargo fmt --check`、`cargo clippy -p future-tui --all-targets -- -D warnings` |
