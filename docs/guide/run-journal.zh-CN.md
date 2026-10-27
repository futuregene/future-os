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
- 但事件流在设计上是无界的：只有 run 完成通知会触发折叠，错过通知的 run 会一直
  留着原始行。这个库里有 **1,166 个已结束 run，其中 135 个仍持有原始事件流**。
- `future session journal --reclaim --yes` 折掉了 **85 个 run 的 6,958,103 条事件**，
  用时约 **70 秒**；随后一次 `VACUUM` 用 **5.4 秒**把文件从 **4.43 GiB 降到
  1.30 GiB**。**337 个**会话的转录在前后**逐字节相同**。

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

- **是折叠，不是截断。** 对已结束、且原始事件流以 `agent_end` 结尾的 run，增量流
  被折成 `run_snapshots` 的一行（`DeltaFolder` 保留可见文本流与计价事件），之后
  才删除原始行——发布与第一片删除在同一个事务里，所以崩溃不会让读取方失去任何
  一边。
- **永不触碰：** `entries`、`message_blocks`、`history_*`、`sessions`、`runs`，
  以及任何仍处于 `running` 或尚未结束的 run 的事件流。
- **拒绝而不是猜：** 最后一条事件不是 `agent_end` 的事件流，或含有无法证明是重复
  内容的 provider 增量，会被保留并计数（报告里的 `Kept journals`）。

折叠为什么更小：在这个库上，`run_e4a62a1c…` 的 45.8 MiB `thinking_delta` 与
`tool_delta` 折成了一个 **952 KiB** 的快照；696 万条原始事件（约 1.7 GiB 载荷）
折成了 85 个快照（约 114 MiB）。

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

真正要守的性质是"删事件不能改变读者看到的内容"，所以用的是 diff，而不是口头保证：

- 对**全部 337 个**会话 id 执行 `future session transcript --session <id>
  --select all --all --max-bytes 0 --json`（235 MB 输出）——折叠前、折叠后、
  `VACUUM` 之后三次比对，完全一致。这个 diff 是可证伪的：给任意一份 dump 追加
  一个字节，它就会报出该文件。
- `entries`（116,281 行）与 `message_blocks`（154,295 行）按主键顺序导出，前后
  MD5 相同（`1be02f36…`、`0e0dcae1…`）。
- 折叠后与 `VACUUM` 后 `PRAGMA quick_check` 均为 `ok`。
- `cargo test -p future-agent journal_reclaim` 在合成库上守住同一组不变量，其中
  包括"一次回收不会改动 `entries` 与 `message_blocks`"。

## 还剩下什么

上面那个库折叠后的实测残留——这些是已知限制，不是本文在绕过的 bug：

| 残留 | 体积 | 原因 |
| --- | --- | --- |
| 50 个已结束 run 的事件流不以 `agent_end` 结尾（49 个 `interrupted`、1 个 `cancelled`） | 761,984 条事件 · 179 MiB | 折叠会拒绝生产者可能仍会追加的事件流。这些 run 在中途被切断（最大的那个最后一条事件是 `tool_start`），所以在未来出现"已结束 run 不看结尾也能折"的决定之前会一直留着。 |
| 5 个 `running` 的 run | — | 活动 run 永不进入候选，这是设计。 |
| 文件内部的空闲页 | `VACUUM` 前 3.08 GiB | 折叠把页交还 SQLite 的空闲表；空闲增量回收以 10 ms 切片归还，只有 `VACUUM` 会重写文件。 |

读这些数字时还有一点值得知道：自动折叠由 run 完成通知触发，没有启动扫描、也不
重试，所以一个在 run 中途被关掉、被升级、或单纯很忙的库会一直留着原始事件流。
这个库就是这么涨到 4.43 GiB 的——显式回收之前 `journal.reclaimed_runs` 只有 62，
回收之后变成 147。

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
