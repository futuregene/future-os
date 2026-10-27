# 运行事件流的开销与回收

[English](run-journal.md)

每一次 Agent run 都把原始事件流逐条追加进 `<home>/agent/agent.db` 的
`run_events`：一条流式增量、一段工具输出、一次 token 用量各占一行。
`future session journal` 报告这张表的开销，`--reclaim` 把已结束的 run 折进
`run_snapshots`。本文记录它在一个真实库上的开销数字、为什么周边的读取操作并
没有因此变慢，以及带实测耗时的操作步骤。

本文没有任何定时自动行为，也不会有任何命令删掉会话、entries 或
message_blocks。

## 一句话结论

- 在一个 **4.43 GiB** 的库上，`run_events` 连同它的唯一索引占了 **3.56 GiB
  （76%）**，而真正承载对话的 `entries` + `message_blocks` 只有 **378 MB**。
- **体积没有让读取变慢。** `session list`、`session status`（`get_state`）与
  `session transcript` 在 4.43 GiB 和 1.30 GiB 上测得相同的耗时；它们是按会话
  走索引的读取，根本不会碰到膨胀的那些事件页。
- 事件流以前在设计上是无界的：只有 run 完成通知会触发折叠，错过通知的 run 会一直
  留着原始行。这个库里有 **1,166 个已结束 run，其中 135 个仍持有原始事件流**。
  现在有两个入口覆盖它：**有界的启动追赶扫描**（16 个 run／10 秒，一次，且留日志）
  与显式回收。
- `future session journal --reclaim --yes` 折掉了 **85 个 run 的 6,958,103 条事件**，
  用时约 **70 秒**；随后一次 `VACUUM` 用 **5.4 秒**把文件从 **4.43 GiB 降到
  1.30 GiB**。**337 个**会话的转录在前后**逐字节相同**。
- 同一个库后来的副本（**4.77 GiB**）里还有一批从未发过 `agent_end` 的流——被崩溃
  切断的 50 个 `interrupted`/`cancelled` run。判断 run 是否结束的权威是 `runs`，
  所以只要流安静了一分钟它们也能折：回收 138 个 run / 816 万条事件，之后
  **0 个可回收**，文件减少 **77.6%**，363 个会话转录仍逐字节相同。

## 文件由什么构成

数据来自 2026-10-27 11:18 从线上库复制的一份副本，用 `dbstat` 测得（macOS
26.6.2 · Apple M3 Ultra · APFS · `future` v0.0.2）：

| 对象 | 字节 | 占比 |
| --- | --- | --- |
| `run_events`（表） | 2,828 MB | 59% |
| `sqlite_autoindex_run_events_1` | 802 MB | 17% |
| `run_snapshots` | 322 MB | 7% |
| `message_blocks` | 224 MB | 5% |
| `entries` | 155 MB | 3% |
| `history_display` | 50 MB | 1% |
| `history_shapes` | 44 MB | 1% |
| 其余（索引、`sessions`、`runs`） | ≈ 60 MB | 1% |

文件为 **4,753,215,488 B**，`page_size` 4096、1,160,453 页、**空闲页 0**——SQLite
曾经释放过的每一页都已被重新用上，所以这不是上一轮清理留下的碎片。行数：
`run_events` 7,745,220 · `entries` 116,281 · `message_blocks` 154,295 ·
`sessions` 337 行（300 个可列出，37 个已墓碑化）· `run_snapshots` 1,020。

## 是谁写的

按 `json_extract(payload,'$.event_type')` 分组：

| 事件类型 | 行数 | 载荷 | 单行 |
| --- | --- | --- | --- |
| `thinking_delta` | 4,075,791 | 847 MB | 217 B |
| `tool_delta` | 3,485,150 | 840 MB | 252 B |
| `tool_end` | 16,265 | 63 MB | 4,103 B |
| `tool_start` | 32,608 | 20 MB | 663 B |
| `text_chunk` | 99,845 | 15 MB | 159 B |
| `usage` | 15,327 | 5 MB | 351 B |
| 其余全部类型 | ≈ 20,000 | < 2 MB | — |

两种流式增量占了 **97.5% 的行**和 **94% 的载荷**。它并不是"少数超大 payload"
的问题：91% 的行在 128–255 字节之间，全库只有 454 行超过 16 KiB（最大
404,616 B）。所以成本在**行数**，不在单行大小——而且每行比它的载荷更贵，因为
唯一索引（`session_id`、`run_id`、`idx`、`epoch`）是额外付的：表和索引合计
3,630 MB 装 7,745,220 行，即**一条 243 字节的事件实际占约 491 字节**。

按会话也一样：最忙的一个会话持有 327,479 条事件（77 MB），135 个已结束 run
合计持有 772 万条事件。当天有 5 个 run 处于 `running`，947 个 `completed`、
149 个 `cancelled`、50 个 `interrupted`、20 个 `failed`。

## 体积到底有没有拖慢什么

CLI 的墙钟时间（含进程启动，3–5 次取中位数，页缓存已热），同一份内容在折叠前后各测一次：

| 命令 | 4.43 GiB 库 | 1.30 GiB 库 |
| --- | --- | --- |
| `future session list` | 0.11 s | 0.11 s |
| `future session info <id>` | 0.09 s | 0.08 s |
| `future session status <id>`（`get_state`） | 0.01 s | 0.01 s |
| `future session transcript --session <id>` | 0.02 s | 0.02 s |
| `future session transcript --session <id> --all` | 0.21 s | 0.20 s |
| `future session journal`（报告自身） | 冷 23.8 s / 热 11.0 s | 0.56 s |

所以对这个量级的库，诚实的答案是：**面向对话的命令并不需要把事件流缩掉**。它们
读的是 `entries`、`message_blocks` 和几个索引，一次一个会话，成本跟着对话走，
不跟着事件流走。唯一跟着文件体积变化的是事件流报告本身，因为它要扫的就是它要
描述的那张表——这也是 `get_journal_stats` 现在走维护超时、而不是 30 秒的存储
超时的原因：这个库上冷启动的一次报告实测 23.8 秒，而更早一次冷跑直接撞上 30 秒
上限并打印 `Timeout expired`，也就是诊断命令在它最该诊断的那个库上失败。

## 回收究竟删了什么

`future session journal --reclaim --yes` 跑的是 Agent 在 run 完成时做的同一套逐
run 折叠，只是显式、且按有界切片进行（每个 RPC 20 秒工作预算，直到 `hasMore`
为否）：

- **是折叠，不是截断。** 对一个可折叠的已结束 run，增量流被折成 `run_snapshots` 的
  一行（`DeltaFolder` 保留可见文本流、计价事件，以及每一条非增量事件的原样内容），
  之后才删除原始行——发布与第一片删除在同一个事务里，所以崩溃不会让读取方失去任何
  一边。
- **永不触碰：** `entries`、`message_blocks`、`history_*`、`sessions`、`runs`，
  以及任何仍处于 `running` 或尚未结束的 run 的事件流。

### 什么样的 run 可以折

run 有没有结束，权威来源是 `runs`；`agent_end` 只说明它的流*完不完整*。这是两个
不同的问题，而这一页的第一版把第二个问题当成了判据，于是拒绝回收所有流被截断的
run——而那恰恰是崩溃留下的东西。

| run 状态 | 事件流 | 折吗 |
| --- | --- | --- |
| `completed` / `failed` / `cancelled` / `interrupted` | 以 `agent_end` 结尾 | 折，立即 |
| 同上，且**已结束超过 60 秒** | 没有 `agent_end`（被截断） | 折——一分钟后不可能还有生产者会补上缺失的尾巴 |
| 同上，但**结束不到 60 秒** | 没有 `agent_end` | 不折：让 `runs` 落终态的 `run_terminal` 条目写在最终事件广播*之前*，尾巴可能还在路上。计入 `not_terminal` |
| 任意已结束状态 | 没有 `agent_end` 且没有 `completed_at_ms`（legacy 导入） | 不折：判定生产者已消失靠的就是时间，而没有时间可读。计入 `not_terminal` |
| 任意已结束状态 | 含无法证明与可见文本重复的 provider 增量 | 不折：丢掉它们会改变客户端渲染的内容。计入 `provider_delta` |
| `running` | 任何 | 不折：流还在被追加。计入 `not_settled` |

折叠一条被截断的流，发布的正是当时存在的那串事件——它不会凭空补一个终态；报告里的
`Truncated` 是持续计数，记录有多少个已发布快照来自这种流。诊断信息不会丢：`error`、
只有 `tool_start` 没有 `tool_end` 的未完成调用、被取消调用的半截工具参数，以及所有
其它非增量事件都原样保留；折叠唯一去掉的事件，是它合进更长片段里的那些。

在 2026-10-27 12:21 取得的副本（4.77 GiB）上实测：148 个已结束 run 仍持有原始流
——62 个 `interrupted`（134 万条事件）、29 个 `cancelled`（128 万条）、56 个
`completed`（563 万条）、1 个 `failed`（7 万条）——回收折掉 **138 个 run /
8,162,910 条事件**，20 个切片（7 分 49 秒），之后报告 **0 个可回收 run**，
`Truncated` = 50：本页第一版列为"永远不可回收"的那 50 个 run 现在被折掉了。

折叠为什么更小：在上面那个库上，`run_e4a62a1c…` 的 45.8 MiB `thinking_delta` 与
`tool_delta` 折成了一个 **952 KiB** 的快照；696 万条原始事件（约 1.7 GiB 载荷）
折成了 85 个快照（约 114 MiB）。

## 启动时的追赶扫描

完成通知只覆盖"进程还活着、run 正常结束"的情形。其它情况——崩溃、被停、旧版本、
折叠失败——以前只能靠人记得去跑一次回收，否则就一直留着；这个库就是这么涨到
4.43 GiB、还有 135 个已结束 run 攥着原始流的。

所以 compactor 线程在启动时跑一次有界的追赶扫描（`CatchUp`，走的是原来处理完成通知
的同一条队列），并留下日志：

```
run-journal catch-up: folded settled journals, kept the rest
  examined=10 folded=10 events=162544 snapshot_bytes=3143781
  truncated=8 errors=0 refused=none has_more=true elapsed_ms=10050
```

- **上下限：** 最多折 16 个 run（`CATCH_UP_MAX_RUNS`）、最多折 10 秒
  （`CATCH_UP_BUDGET`），先到先停；计时从读到候选计划之后才开始，所以"启动把这个
  任务排在它自己的迁移后面"不会把预算全耗在等待上。单次折叠不会被中途切断，因此
  一个特别大的 run 可以让一次追赶超过时间上限——下一次启动接着做
  （`has_more=true`）。
- **不是定时器，也不在请求路径上：** 它就是一次扫描，由 `initialize` 入队、在
  compactor 线程上执行；没有任何东西安排第二次，也没有任何 RPC 等它。
- **天生可诊断：** `folded: 0` 且 `refused` 非空 = 看过并且合规地保留了；
  `examined: 0` 打到 debug 日志 = 没有已结束的流需要折；**完全没有这一行** = 追赶
  扫描根本没跑。在此之前，"安静的库"和"坏掉的库"在一个不再缩小的文件里长得一样。
- **一次处理不完也不要紧：** 显式回收仍是处理大积压的入口，它是同一套折叠，只是够得
  更远。

在上面那份 4.77 GiB 的副本上，升级后的第一次启动折了 10 个 run（162,544 条事件，
其中 8 个是截断流），用时 10.05 秒并报告 `has_more=true`；显式回收处理掉其余 138 个。

## 操作步骤

数字全部来自上面那个 4.43 GiB 的库；整个流程的实际工作时间不到两分钟（备份之外）。

1. **先停掉占用这个 home 的东西。** 退出桌面应用并停掉 Agent（`future agent`，
   或启动它的 host）。`VACUUM` 需要排他事务，而 Agent 的 `busy_timeout` 是 5 秒：
   在这个库上实测，一次 24 秒的 `VACUUM` 期间来的写入方等了 5.01 秒后以
   `database is locked` 失败。`future data backup` 同样在 home 被占用时拒绝运行，
   这正是它该做的。
2. **备份。** `future data backup <新目录>` 会生成一份逐文件 SHA-256 校验过的副本
   （含 `agent.db` 与 home 的其余部分）。它是唯一受支持的备份路径，见
   [数据备份与恢复](data-backup.zh-CN.md)。
3. **先看报告。** `future session journal`——开销、最大 run、可回收量、被保留的
   部分及原因。
4. **折叠。** `future session journal --reclaim` 看计划，`--reclaim --yes` 执行。
   中途打断是安全的、可续跑，再执行一次就继续。在这个库上：4 个切片、
   **85 个 run / 6,958,103 条事件，约 70 秒**，最后以 `quick_check: ok` 结束。
5. **缩文件。** 折叠会释放页，但不把页还给文件系统：第 4 步之后文件仍是 4.22 GiB，
   其中 752,688 个空闲页（3.08 GiB）。这些页会被新事件复用，所以库不再继续膨胀；
   要让文件本身变小必须做一次 `VACUUM`。在 Agent 仍停着的状态下：
   ```bash
   sqlite3 ~/.future/agent/agent.db "VACUUM;"
   ```
   实测 **5.4 秒**（4.22 GiB → 1.30 GiB 文件）；而在未折叠的 4.43 GiB 文件上要
   **24.0 秒**，且只能降到 4.18 GiB——因为它没有空闲页，没什么可还的。这需要留出
   等于最终文件大小的磁盘空间（SQLite 会在原文件旁边构建新文件），结束后
   `PRAGMA quick_check` 为 `ok`。
6. **验证。** 再跑一次 `future session journal`（行数与体积），然后读一个真实对话：
   `future session transcript --session <id> --all`。验收标准是转录，不是字节数——
   见下一节。

Windows 上同样步骤，路径换成 `%USERPROFILE%\.future\agent\agent.db`，`sqlite3`
调用从一个 `PATH` 里有 `sqlite3.exe` 的 shell 里执行。

## 验证是怎么做的

真正要守的性质是"删事件不能改变读者看到的内容"，所以用的是 diff，而不是口头保证。
测过两遍，两个库的副本；第二遍包含被截断的流。

第一个库（2026-10-27 11:18，4.43 GiB，未折任何截断流）：

- 对**全部 337 个**会话 id 执行 `future session transcript --session <id>
  --select all --all --max-bytes 0 --json`（235 MB 输出）——折叠前、折叠后、
  `VACUUM` 之后三次比对，完全一致。这个 diff 是可证伪的：给任意一份 dump 追加
  一个字节，它就会报出该文件。
- `entries`（116,281 行）与 `message_blocks`（154,295 行）按主键顺序导出，前后
  MD5 相同（`1be02f36…`、`0e0dcae1…`）。

第二个库（2026-10-27 12:21，4.77 GiB，**148** 个已结束 run 持有原始流，**47 个
会话** 含有被崩溃切断的 `interrupted`/`cancelled` run）：

- 同样的转录 dump，**全部 363 个**会话 id（249 MB），在动手前、启动追赶之后、完整
  回收之后、`VACUUM` 之后各取一次——**每次 0 差异**。那 47 个含截断 run 的会话转录
  全部非空，且 47 个全部逐字节相同。
- `entries`（123,258 行）与 `message_blocks`（163,526 行）：前后 MD5 相同
  （`5e634326…`、`0a084e58…`）。
- `run_events` 8,397,370 → 71,916 行；`run_snapshots` 1,057 → 1,205；
  `PRAGMA quick_check` 为 `ok`；随后一次 `VACUUM` 用 5.5 秒把文件从 4.83 GiB 降到
  **1.07 GiB**（相对最初的 4.77 GiB 副本：−77.6%）。
- `cargo test -p future-agent journal` 在合成库上守住同一组不变量，其中包括"一次
  回收不会改动 `entries` 与 `message_blocks`"。

## 还剩下什么

第二个库完整回收后的实测残留——这些是已知限制，不是本文在绕过的 bug：

| 残留 | 体积 | 原因 |
| --- | --- | --- |
| 没有 `completed_at_ms`（legacy 导入）且流被截断的已结束 run | 这个库上没有 | 没有结束时间就没有"生产者已消失"的证据，计入 `not_terminal`，直到能建立时间为止。 |
| 含无法证明与可见文本重复的 provider 增量 | 这个库上没有 | 丢掉会改变客户端渲染的内容，计入 `provider_delta`。 |
| 落在 60 秒宽限期内的 run | — | 尾巴可能还在路上；下一次启动（或下一次回收）处理。 |
| 5 个 `running` 的 run | — | 活动 run 永不进入候选，这是设计。 |
| 文件内部的空闲页 | `VACUUM` 前 3.4 GiB | 折叠把页交还 SQLite 的空闲表；空闲增量回收以 10 ms 切片归还，只有 `VACUUM` 会重写文件。 |

这个库不会再自己漂回 4.4 GiB 的 journal：启动追赶扫描会折掉被停、崩溃留下的那些，
而 `future session journal` 里的 `Truncated` 计数就是它在做事的持续证据（第二个库上
是 50，回收结束后是 `Reclaimable 0 settled runs`）。

## 复现这些测量

```bash
# 复制一份副本，永远不要直接对线上库动手
export FUTURE_HOME=/tmp/j            # 它有自己独立的 agent/，因此有自己的锁与 socket
mkdir -p "$FUTURE_HOME/agent"
cp ~/.future/agent/agent.db* "$FUTURE_HOME/agent/"   # db、-wal、-shm 一起复制
future agent --grpc-addr 127.0.0.1:5099 &
FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 future session journal
FUTURE_AGENT_GRPC_ADDR=127.0.0.1:5099 future session journal --reclaim --yes
sqlite3 "$FUTURE_HOME/agent/agent.db" "PRAGMA quick_check; PRAGMA freelist_count;"
```

对副本起 Agent 必须用独立的 `FUTURE_HOME` 和一个空闲端口：它不会与你正在使用的
Agent 共用端点或锁。
