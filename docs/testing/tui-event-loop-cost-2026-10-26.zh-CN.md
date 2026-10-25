# `future tui` 事件循环的每帧成本（2026-10-26）

一次性测量记录，不是设计文档。它测量的是 `tui/src/workbench/run.rs` 的驱动循环在这
次修复前后的开销——修的是"每个 agent 事件都重新排版整段对话"和"每 120ms 重排一帧没有
变化的画面"两件事。下面每个数字都是本机实测；方法与夹具都写出来了，可以逐行复核。

## 0. 量到了什么

同一个循环里有两个独立病灶：

| # | 病灶 | 原位置 | 代价 |
|---|---|---|---|
| 1 | 事件路径为了读一个字段，向 adapter 要一整帧**宽度 1** 的数据（`live.data(1).activity.kind`）：每个输入事件两次、每个 agent 事件一次；真正的帧随后在对话区宽度上另做一次 | `run.rs` 的事件分支、`Live::data` | `ChatArea` 的渲染行是**按宽度**缓存的，宽度 1 的探针把缓存打掉，于是每次都要重新折行 + 重新解析全部消息。每个事件两次整段重排 |
| 2 | ticker 分支是空的，循环无法区分"没有变化"和"没有事情发生"：`draw()` 在每轮迭代开头无条件执行 | `run.rs` 的 `event_loop` | 每秒 8 次整帧合成，永不停止，其中包含把每一行已渲染的对话行克隆进 `WorkbenchData` |

修法四件：一轮迭代只算一次帧度量（`layout::Frame`，自带对话区文本宽度）；循环需要的
两个字段改走 `Live::activity_kind()` / `Live::activity_animating()`，不再绕帧；
对已排队的 agent 事件做合并 `drain()`；用一个 `TickClock` 回答"这次 120ms 的 tick 该不该
出一帧"。

## 1. 环境与夹具

| | |
|---|---|
| 机器 | macOS 26.6.2 (25G83)，Apple silicon，32 核 |
| 基线提交 | `origin/dev-huichen` 的 `1e72d58fe`，分支 `perf/tui-frame-cost` |
| 工具链 | 两份二进制、测试与探针都用 pin 住的 `1.97.0`（`rust-toolchain.toml`） |
| `before` 二进制 | 基线提交上的临时 worktree 里 `cargo build -p future-cli` |
| `after` 二进制 | 本分支 worktree 里同一条命令 |
| 构建标记 | 两份都用 `FUTURE_VERSION=0.0.2-framecost`，免得标题栏里的版本号变成差异 |
| 帧 | 120×38，侧栏 36 列 → 对话区宽度 **79** 列，每帧 9031 ANSI 字节 |
| 进程内探针 | `cargo test -p future-tui --release --test perf_loop_probe -- --nocapture` |
| 真终端夹具 | 隔离 `FUTURE_HOME=/tmp/tfc/home`，agent 走同一棵树的 IPC socket（不加 `--grpc-addr`，全程没碰 `~/.future` 上的真 agent）；一份 legacy `perfbig.jsonl` 会话：**600 轮 / 1200 条**，含 markdown、代码块、表格、引用，约 700 KB |
| 真终端测量 | `script -q /dev/null <二进制> tui --session perfbig --offline` 起在 pty 上，再用 `ps -o time=` 在两个时刻做差、按 10s 窗口折算；`ps -o %cpu`（生命周期均值）并排列出 |

## 2. 每个事件的成本，修复前后（进程内）

release 构建。`ms` 是每个事件毫秒数，`rb` 是每个事件的整段重排次数（用
`ChatArea::full_renders` 差分）。每行 20 次重复。

一个流式 delta——应用这个 token，再加上它引起的帧：

| 轮数 | 旧 ms | 新 ms | 加速 | 旧 rb | 新 rb |
|---:|---:|---:|---:|---:|---:|
| 5 | 2.757 | 0.461 | 6.0× | 2.00 | 0.00 |
| 20 | 3.933 | 0.358 | 11.0× | 2.00 | 0.00 |
| 50 | 8.872 | 0.374 | 23.7× | 2.00 | 0.00 |
| 100 | 17.152 | 0.392 | 43.8× | 2.00 | 0.00 |
| 200 | 33.529 | 0.447 | 75.1× | 2.00 | 0.00 |

一次按键（同一条事件路径，没有 delta）：

| 轮数 | 旧 ms | 新 ms | 加速 |
|---:|---:|---:|---:|
| 5 | 1.171 | 0.346 | 3.4× |
| 20 | 3.639 | 0.357 | 10.2× |
| 50 | 8.609 | 0.366 | 23.5× |
| 100 | 16.884 | 0.390 | 43.3× |
| 200 | 33.448 | 0.435 | 77.0× |

一次空闲 tick——循环自身在无变化时的那次重排（旧列是**一帧没变化的画面**；新列是
**根本不出帧**，所以加速比不是一个数）：

| 轮数 | 旧 ms/帧 | 8 tick/s = 单核百分比 | 新 |
|---:|---:|---:|---|
| 20 | 0.352 | 0.3 % | 0.0 % |
| 50 | 0.365 | 0.3 % | 0.0 % |
| 100 | 0.389 | 0.3 % | 0.0 % |
| 200 | 0.435 | 0.3 % | 0.0 % |

这一列是平的，因为每次 tick 的代价**不是**对话本身：它是"合成一帧 + 写出去"的固定开销，
一秒付 8 次，而那一帧的字节一个都没变。

一批 20 个 delta（≈0.7 秒的 30 tokens/s 流），合成一层帧：

| 轮数 | 旧 ms | 新 ms | 加速 | 旧 rb | 新 rb |
|---:|---:|---:|---:|---:|---:|
| 5 | 24.0 | 0.3 | 69× | 40 | 0 |
| 20 | 73.3 | 0.4 | 208× | 40 | 0 |
| 50 | 172.1 | 0.4 | 467× | 40 | 0 |
| 100 | 336.5 | 0.4 | 843× | 40 | 0 |
| 200 | 666.0 | 0.4 | 1514× | 40 | 0 |

一帧的时间花在哪（100 轮）：对话行缓存命中 **0.044 ms**，合成 **0.339 ms**，序列化
**0.002 ms**。也就是说修完之后一帧的开销由"合成本身"主导——那正是帧该做的事——其中没有
任何 markdown 工作。

**关键不变量是重排次数。** 修复前，一帧的成本与 **token 数**成正比（每个 token 把整段
对话重排两次）；修复后与 **可见帧数**成正比：一个 delta 只把流式那条消息切进去，不重排任何
东西（每事件 `rb = 0.00`），只有宽度变化或整条消息重渲才付一次重排。

把 delta 那一行外推到流上（成本是按事件计的，所以这只是算术，不是新测量）：30 tokens/s
的 200 轮对话，修复前 30 × 33.5 ms = **每秒 1.01 秒 CPU ≈ 单核 100 %**；修复后
30 × 0.45 ms = **每秒 13 ms ≈ 1.3 %**。这就是"一个进程吃满一个核"的形状：回复在流的时候
它一点都不空闲。

## 3. 真终端的空闲 CPU

同一份会话、同一个 120×38 的 pty，等第一帧稳定后用 10 秒窗口测量：

| 状态 | 修复前 | 修复后 |
|---|---|---|
| 空闲，pty 在，600 轮 | 单核 18.6 %（`ps -o %cpu` 22.3） | **0.3 %**（`ps -o %cpu` 0.1） |
| 空闲，pty 被拿走（杀掉 `script`，`ppid=1`、无 tty——也就是那 6 个孤儿被找到时的状态） | 4.2 %（`ps -o %cpu` 3.7） | **0.3 %**（`ps -o %cpu` 0.0） |

这张表有四点要读出：

* **两份都是 debug 构建**——工单要的就是这个，那些孤儿也是 `./target/debug/future tui`。
  release 构建合成一帧大约便宜 30 倍，所以它的空闲数字会低得多；但它仍然会为一张没变过的
  画面一天合成约 69 万帧。
* 开销**不在对话上**：探针的空闲 tick 一行从 20 轮到 200 轮是平的（0.352 → 0.435 ms），
  所以真终端那个数字是"每帧、每秒 8 次"，不是"每段对话"。
* 死掉的 pty 比活着的**更便宜**（4.2 % 对 18.6 %）：没人读 pty 之后，写帧几乎不要钱——
  这本身就是在量"活着时那些时间花在哪"。
* 修复后，空闲的 `future tui` 在两种状态下都是 **单核 0.3 %**，落在"个位数"目标之内。
  空闲 ticker 剩下的唯一动作是看一眼时钟：没有输入、没有 agent 事件，就没有 `render_all`、
  没有 `WorkbenchData`、没有一个帧字节。对一个空闲实例采 3 秒样（`sample <pid> 3`），
  调用图里**找不到任何 `ChatArea::render_all`、`Live::data`、`screen::compose`**——
  正是把旧循环定罪的那份采样的反面。

## 4. 画面没有变

要求是同尺寸逐字节一致，所以这是核对出来的，不是论证出来的：一个临时 dump 夹具（未提交）
在 15 种终端尺寸 × 侧栏开/关下，用同一份 `ChatArea` 对话、同样的对话区宽度合成帧，把每一行
带转义原样打印——一次对着 `origin/dev-huichen`（基线提交上的临时 worktree），一次对着本分支。

* 30 个帧（15 种尺寸 × 侧栏开/关）、638 行：**除两个块之外全部逐字节相同**。尺寸是
  140×44、120×38、100×40、96×30、88×24、80×20、60×16、40×12、24×10、12×6、8×24、
  7×24、6×24、7×4、5×3。
然后在真 pty 上做了端到端核对：§3 那两份二进制（修复前 / 修复后，同一提交、同一工具链、
同一个 `FUTURE_VERSION`）各自打开同一份 600 轮会话、120×38，它们写出的**最后一帧逐字节相同**
——7599 字节，完全一致——只要把 `Terminal::start` 启动时写的 kitty 键盘模式转义序列放在一边。

* 只有 **7×24 与 6×24** 不同——也就是"对话区只有 7 或 6 列、但还留有对话行"的情况
  （7×4 与 5×3 相同，因为四行高的帧根本留不出对话区）。原来驱动按 `band − 4`（最少 1）
  折行，合成器按 `min(2, band/4)` 留边，两者在 8 列以下互相矛盾，折好的文本越过了渲染器
  真正拥有的宽度。现在两边都读同一个定义（`layout::chat_inner`），于是 7 列时折行宽度
  由 3 变成 5、每行显示两个字符而不是一个。这是边角修补，不是重排：任何可用宽度下，分带、
  留白、文案都没有变化。

## 5. 回归

| 命令 | 结果 |
|---|---|
| `cargo test -p future-tui`（pin 住的工具链） | **1055** 个 lib + 6 个 `cli_smoke` + 8 个 `i18n_guard` + 1 个 `perf_loop_probe`，0 失败 |
| `cargo test -p future-cli --lib -- tui` | 过滤器没匹配到任何用例，退出码 0（这条过滤词在 crate 里没有对应用例；覆盖面见下一行） |
| `cargo test -p future-cli --lib`（`ulimit -n 4096`） | 1398 通过、1 失败：`commands::mcp_client::tests::process::the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest`，即 `docs/testing/flaky-cli.md`（§8.2）已记录的那个 flake，成因不在本次写集内。单独跑 3/3 通过 |
| `cargo test -p future-cli --lib`（默认 `ulimit -n 256`） | 224 个失败，全是 browser/`test_server` mock 的 `Os { code: 24, "Too many open files" }`——本机 fd 上限，不是代码 |
| `cargo fmt -p future-tui --check` | 干净 |
| `cargo clippy -p future-tui --all-targets` | 在 pin 住的工具链上干净（本机 Homebrew `1.96.0` 遮蔽它时，唯一告警是 `agent/src/session/compaction_ops.rs:79` 既有的 `nonminimal_bool`） |
| `make check-docs` | 331 份文档，位置/配对/链接/脚本/围栏全部通过 |

锁住契约的新用例：

* `workbench::run::tests::an_idle_tick_with_no_clock_on_screen_paints_nothing`
  ——画面上没有任何读时钟的东西时，500 次 tick（120ms 一次，正好一分钟）一帧都不出。
* `workbench::run::tests::an_idle_tick_paints_the_session_ages_once_a_minute`
  ——会话列表在屏幕上时，唯一的时相关内容是那一列的年龄（按分钟取整）：两分钟里正好两帧。
* `workbench::run::tests::a_run_in_flight_paints_once_per_spinner_frame`
  ——忙碌行按 spinner 的**绘制帧**（4 tick）重绘，不是每 tick 一帧。
* `workbench::run::tests::a_queued_burst_is_applied_in_order_and_bounded`
  ——合并 drain 按顺序应用已排队事件，到 `COALESCE_LIMIT` 停住，剩下的留给下一轮。
* `components::chat_area::tests::a_probe_width_is_a_full_rebuild_and_the_frame_width_is_free`
  与 `...::a_streamed_delta_at_one_width_does_not_rebuild_the_transcript`
  ——宽度缓存的真实行为，就是循环现在假设的那样。
* `workbench::layout::tests::a_frame_carries_the_width_the_conversation_is_drawn_at`
  与 `...::chat_inner_is_the_band_minus_its_padding`
  ——对话区宽度的唯一一处定义。

## 6. 没测到的，以及一处已知缺口

* **真实的模型流。** 真跑一次 run 要花钱，所以每 token 成本是在进程内测的（§2），不是端到端。
  合并本身由上面的单测覆盖，空闲循环的真终端数字是 §3。
* **那 6 个孤儿进程各吃满 ~100 % 的确切状态。** 没有复现出来。不用模型能造出的、最接近
  的状态是 §3 的两行（修复前 10.5 % / 11.0 %）。探针说明"每个进程 100 %"正是**流式 run**
  在大对话上的开销（30 tokens/s、200 轮 = 单核 115 %），所以"一段没人看、还在被应用的流"
  能解释它，但那个具体现场没有重建。
* **无 tty 时 `SIGTERM` 依旧无效，修复前后一致。** 这是另一个既有的缺口：终端读线程在
  EOF / 读失败时直接退出（`tui/src/terminal.rs` 里 `n == 0 => break` / 读错误那一支），
  之后没人再 poll 信号 self-pipe，应用的退出回调永远不会跑。它不是本次改动引入的（两个
  二进制实测一致），这里也不动它——它是终端生命周期的问题，不是帧成本的问题；但它正是
  那些孤儿需要 `SIGKILL` 的原因，而修复之后需要它的理由只剩这条退出路径，不再是 CPU。
