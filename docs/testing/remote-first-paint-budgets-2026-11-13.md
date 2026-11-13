# 首屏：历史页与 run 回放的字节边界（2026-11-13）

两份改动，两份实测。数据都来自本机 `~/.future/agent/agent.db`（只读打开）与运行中桌面进程的日志；
所有会话/run id 是用会话名现查的，不是手抄的常量。

## 1 历史页：一次打开不再搬运一整轮（`perf/first-paint-budget`，已合入）

**原因**：`packages/rpc/src/history.rs` 的 `ExchangeStart::Complete` 会把一页往回补到它的提问行，
这个循环没有字节上限；`wire_limits` 的 512 KiB 只能整轮丢弃，单轮超预算时丢不动。

**会话** `8ac7afd8-1d5c-4b6f-b707-330b62a0ea82`（最后一条提问后跟了 1,113 条记录）：

| | 读页次数 | 首屏行数 | 原始 | lean 后 |
| --- | ---: | ---: | ---: | ---: |
| 改前（实测） | 6（222/179/217/193/260/33 行） | 1,114 | 10.35 MB | 5.22 MB |
| 改后（推算） | 1 | 33 | 0.52 MB | 0.29 MB |

- **改前**来源：agent 日志 `history_page_read_started/completed`（2026-11-13 14:03:08.911–14:03:09.639），
  六次串行读页耗时 163/156/144/109/68/9 ms。
- **改后**来源：同一份 store 上按新规则重算（每页 ≤512 KiB，取最新若干行）。
- **复现步骤**（PowerShell，改 `$S` 即可）：

```powershell
$py = @'
import sqlite3, os, uuid
S = uuid.UUID("8ac7afd8-1d5c-4b6f-b707-330b62a0ea82").bytes
con = sqlite3.connect(f"file:{os.path.expanduser(r'~\.future\agent\agent.db')}?mode=ro", uri=True)
cur = con.cursor()
print("rows", cur.execute("select count(*) from history_display where session_id=?", (S,)).fetchone())
print("user rows", cur.execute("select count(*) from history_display where session_id=? and is_user=1", (S,)).fetchone())
print("content bytes", cur.execute("""select sum(co.byte_length) from content_references cr
  join content_objects co on co.id=cr.content_id where cr.session_id=?""", (S,)).fetchone())
'@
$py | Out-File -Encoding utf8 $env:TEMP\count.py; python $env:TEMP\count.py
```

逐轮的字节（哪一轮是 10 MB 那一轮）用同样的连接按 `history_display` 的 `is_user` 边界分组统计：

```sql
select sum(co.byte_length)
from history_display hd
join entries e on e.session_id = hd.session_id and e.position = hd.source_position
join content_references cr on cr.session_id = e.session_id and cr.owner_kind='entry' and cr.owner_id = e.entry_id
join content_objects co on co.id = cr.content_id
where hd.session_id = ? and hd.ordinal >= <该轮提问的 ordinal>;
```

## 2 run 回放：首屏从 run 的尾部起读（`perf/replay-first-paint`）

**原因**：桌面打开一条**正在跑**的会话时，`readRunLog(..., full = true)` 传 `sinceIdx = -1`，
host 就把 run 从头按 1,000 事件一页交给客户端；客户端一页一个往返。run 越长，打开越慢。

**边界怎么定**：`activeRun.lastEventIdx`（Agent 在 `get_state` 里已经给了）减去一个客户端页
（1,000 事件）；读出来之后再按 **512 KiB 字节预算**裁掉头部（一条工具事件可以顶一千条 token 增量，
所以事件数只是选起点的估计，字节才是保证）。既有 `hasMore`/`nextSinceIdx` 游标语义不变。

- 为什么用**事件序号**：它是两端唯一共享的坐标，也是客户端分页的计数单位；按时间取不到游标，
  按字节选起点必须先读过那一段。
- 跳过的头部不是丢失：同一个 run 的历史行仍在上翻路径里（断言
  `the_head_a_first_paint_skips_is_still_reachable_through_history`）。

**同一 run 的对照**（`4d4f73da-cba2-4f07-8cae-f955aac48882`，会话 `8ac7afd8…`）：

| | 往返数 | 事件数 | 字节 |
| --- | ---: | ---: | ---: |
| 改前：从 run 开头 | 10（当时）/ 12（今天的数据） | ~10,000 | 5.11 MB（全 run） |
| 改后：从尾部起读 | 1 | ≤1,000 | 0.42 MB（最后 1,000 条） |

- **改前耗时**（实测，桌面日志 2026-11-13 14:04:20.5→14:04:27.6）：`get_run_snapshot` 1 ms +
  10 次 `get_events_since`（62/64/65/68/69/70/71/71/72/170 ms 服务端），墙钟约 6.6 s。
- **改后耗时**：按同一批每页服务端耗时加一次 `get_state`（1–2 ms）推算约 0.1–0.2 s。
  **未做真机实测的原因**：本机运行中的 Agent 是 `0.0.2-8f0f8141f`，与本分支构建出的客户端版本
  （`0.0.0-ffc18f013`）握手被拒；隔离实例需要复制 3 GB 的 `agent.db`，本次没有做。
- **复现**：`host/src/agent_bridge/tests.rs` 里的 `measure_first_paint_on_a_real_run`（`#[ignore]`，
  只读、只发 `get_events_since`）。版本一致时可直接跑：

```powershell
$env:FUTURE_REPLAY_SESSION = "<会话 id>"; $env:FUTURE_REPLAY_RUN = "<run id>"
cargo test -p future-host --lib measure_first_paint_on_a_real_run -- --ignored --nocapture
```

事件数与字节来自 store（`stream_changes` 按 `first_cursor` 排序，`stream_change_content` 求和）：

```sql
select count(*), sum(length(cs.body))
from stream_changes sc
join stream_change_content scc on scc.change_id = sc.id
join content_segments cs on cs.content_id = scc.content_id
where sc.stream_id = (select id from stream_runs where run_id = ?);
```

## 4 预算改按“发送字节”计，并加一道行数闸（`perf/first-paint-lean`）

第一节那个 512 KiB 量的是 **Agent 出来的原始条目字节**，可客户端收到的只是 lean 之后的部分：
思考正文、工具输出正文、工具参数、`shell_result` 内文、`provider_metadata` 都不上线。拿同一轮
（`8ac7afd8` 的最后一条提问之后）按字段拆开：

| 内容 | 字节 | 占比 | lean 后 |
| --- | ---: | ---: | --- |
| 工具输出正文 `tool_result.text` | 6.03 MB | 46.5% | 丢 |
| `metadata.shell_result` 内文 | 2.92 MB | 22.5% | 丢（留判定字段） |
| `provider_metadata` | 2.43 MB | 18.8% | 丢 |
| 工具参数 `tool_call.arguments` | 0.36 MB | 2.8% | 只留标签 key |
| 行骨架（shape JSON） | 0.48 MB | 3.7% | 留 |
| `summary` | 0.22 MB | 1.7% | 留 |
| 可见正文（用户/助手） | 0.19 MB | 1.4% | 留 |

整轮 **12.99 MB → 1.75 MB（13.5%，约 7.4 倍）**。所以现在：

- **字节预算（512 KiB）按“实际会发出的字节”计**：`read_page_with` 接受调用方的权重，host 在客户端
  声明了 lean 时用 `lean_entries` 的结果当权重（`page_weight_as_sent`），未声明就是原始字节。
- **另加 300 行上限**（`PageBudget::FIRST_PAINT`）：字节便宜不等于渲染便宜，一千行两百字节的条目照样是
  一千个行模型。两条独立上限，取先到的那个。

同一轮的效果（按同一份 store 的字段级模型）：

| | 首屏行数 | 首屏上线字节 | 读完这一轮的翻页次数 |
| --- | ---: | ---: | ---: |
| 改前（raw 512 KiB） | 33 | 0.29 MB | 约 50 |
| 改后（lean 512 KiB + 300 行闸） | **300**（行闸封顶） | 约 0.35 MB | **约 6** |

行闸是这次收益的封顶：把 `PageBudget::FIRST_PAINT.rows` 调大，行数与首帧成本一起涨。

**副作用**（写进提交信息，`git log -1` 可查）：

1. 首屏行数变多 → 客户端要建更多行模型，首帧比“33 行”那版慢一点，但远少于原来 1,100 行那次；
2. 为了填满 512 KiB 的**发送**字节，host 可能多读 1–2 个 Agent 切片（本机 gRPC，不上线）；
3. 未声明 lean 的老客户端完全不变（权重仍是原始字节，行为与改前一致）。

复现：与第 1 节同一段 SQL，把 `/content/N/text`（kind 为 `tool_result`/`reasoning`）、
`/content/N/provider_metadata`、`/meta/shell_result` 划到“丢”，其余求和，即得上表两列。

## 3 反向可证

| 断言 | 把改动改回去之后 |
| --- | --- |
| `a_whole_run_replay_starts_at_the_runs_tail`（首屏起点/往返数） | 让 `first_paint_start` 恒返回 `None` → `startIdx` 为 null，断言红（实测：`left: Null, right: Number(4001)`） |
| 同上，另一侧 | `forcing_the_old_start_serves_the_run_from_its_beginning` 把旧的 `sinceIdx = -1` 行为钉住，两者互为反面 |
| `a_turn_larger_than_the_budget_comes_back_as_its_tail`（历史页字节上限） | 把“取尾部”改回整段全量 → `a first page is bounded whatever the turn weighs: 1202600 bytes` 红；把预算调成 0 → 同一行红（`40087 bytes`） |

没有一处靠截图：全是字节数、事件数、往返数、请求里实际带的 `sinceIdx`。
