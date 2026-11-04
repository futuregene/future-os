# 远端桌面只读命令的数据库读取审计（T2，代码级）

> 本章有订正，订正依据见 05-verification.md section 7；以 05 与最终报告为准。

审计对象：远端（手机）发给桌面宿主机的每一条**只读命令**，它落到 `app.db` / `agent.db`（经 gRPC）/
`tasks.db` / Fusion 库上的读取路径。目标不是提出修复方案，而是给出可核对的判定：**哪条读是 O(1)，
哪条随哪个基数增长，增长多少**。

审计方式分三级，文中每条结论都标注它属于哪一级：

- **R（代码确定）**：直接读 handler 与 store 的 SQL，索引在 schema 里逐条核对过。
- **M（实测）**：在真实数据库的**只读副本**上跑同一条 SQL/同一形状的语句取得的毫秒数（见 §1）。
- **E（推演）**：由 R + M 推出的量级，未经端到端计时。

本次没有对真实库写入任何字节，也没有启动/干扰正在运行的 agent：`~/.future/agent/agent.db` 与
`~/.future/app/app.db` 均以 `mode=ro` + `.backup` 复制到 `/tmp/perfaudit/` 后再操作。

---

## 0. 结论摘要

1. **8 条是严格 O(1)**（主键/唯一索引点查，或内存）：`get_tool_call_args`、`get_event_content_range`、
   `get_read_chunk`、`get_desktop_settings`、`get_settings`、`list_task_deps`、`list_task_revisions`、
   `list_providers`。另有两条"形状健康但有固定成本"：`get_session_entries` 的 `before` 路径
   （keyset，页内固定 150/600 行）与 `get_events_since` 的分页/快照路径。
2. **13 类随某个数据维度增长**（§5 表 A 全列），其中三条是延迟与内存的主要来源：
   - **`get_messages`**：把整个会话物化——实测最大会话 14,531 entries / 20,991 blocks / **28.3 MB JSON**，
     内部按 64 条一页读 **228 次**（实测纯 SQL 52.7 ms），再把**全部**消息序列化进一个 gRPC 回复，桌面侧
     收到后才按 `offset/limit` 在内存里切片。冷启动还要先整会话 hydrate。上限 32 MiB
     （`packages/rpc/src/transport.rs:53`）离这条会话的 27.6 MB 只有 15% 余量。
   - **`list_sessions`**：每次缓存未命中 = 682 行全表扫描 `threads`（含临时 B-tree 排序）
     + 682 个占位符的窗口函数 `runs` 查询 + 703 行 `workspaces`（每行一个相关子查询）+ 两次全量 JSON
     序列化求签名；实测约 6 ms SQL + 3 次序列化，**且全程持有全局 `CATALOG` 互斥锁**。规模线性。
   - **`list_available_skills`**：读路径里有一次**同步 HTTP GET**（连 10 s / 总 60 s 超时，
     `agent/src/skills/manager.rs:130`）。它不是数据库读，但它是这条命令最坏情况延迟的唯一决定因素，
     且完全不受数据规模影响——属于"与数据量无关但可以任意慢"的一类。
3. **两条命令的"分页"是假分页**：`get_session_entries`（offset 路径）先以 250 条一页把**整段历史**
   从 agent 拉完（最大会话需 59 次 gRPC），再在桌面内存里 `offset/limit` 切片；
   `get_messages` 同样先拉全部再切。
4. **`list_tasks` / `list_task_runs` 的耗时随"该任务的运行次数"增长，且 `LIMIT` 不省成本**：
   `task_runs` 上没有 `(task_id, started_at)` 索引，`ORDER BY started_at DESC LIMIT 20` 实测与
   `LIMIT 1` 同价（5000 次运行：0.947 ms vs 0.991 ms），因为排序发生在 LIMIT 之前。
5. 已就位的好设计值得保留并作为对照：canonical 流的 `get_events_since` 用 `UNIQUE(stream_id,last_cursor)`
   做 keyset 分页（实测 19,340 条变更的首页 0.08 ms）；`get_event_content_range` 走
   `content_segments(content_id,byte_offset)` 主键取范围；`get_tool_call_args` 全部是点查。
6. **但按端到端实测（§1.1），今天用户真的会遇到的慢读不是上面那几条 O(E) 的**：
   `list_available_skills` **99 ms**（max 121）、`get_state` **22 ms**、`list_sessions` **20.6 ms**、
   `list_models` **21 ms** 都在用户可触及的路径上；而 619 ms 的 offset 分页与 235 ms 的 `get_messages`
   **目前没有任何已发布客户端在发**（01 §1.2）。详见 §5 表 A-2。

---

## 1. 测量环境与数据基线

副本：`/tmp/perfaudit/agent.db`（2.48 GB → 2.48 GB）、`/tmp/perfaudit/app.db`（54 MB）、
`/tmp/perfaudit/tasks.db`（4.1 MB），生成时间 2026-11-04 08:51。计时为 Python `sqlite3` 同一连接上的
`min` of N 次（暖页缓存），**只含 SQL 执行，不含 Rust 侧反序列化、gRPC、锁与进程开销**——
也就是说下面所有毫秒数都是**下界**。

口径限制（详见 §7.2）：§1.1 的端到端数字来自 T1 从**同一批文件派生的隔离 HOME**
（`target/remote-latency/remote-latency-619xoqsh`），那份副本被**当前构建**打开过，所以它有完整的
当前 schema；而**本机生产库**（本文的 SQL 副本）由**稍旧一点的 agent 进程**持有，里头还缺
`run_snapshot_cache` 表与 `run_events_replay_order` 索引。涉及这两处时，本文以生产库的事实为准，
并把它与当前源码路径分开描述。

| 库 | 表 | 行数 |
|---|---|---|
| agent.db | `sessions` | 709（其中 `session_formats.format=2` 即 canonical 的 317 条，其余 392 条为 legacy format 1） |
| | `entries` | 227,532 |
| | `message_blocks` | 302,962 |
| | `history_display` | 222,792（672 个 session 有投影，`projection_version=2`） |
| | `stream_changes` | 2,559,342 |
| | `content_segments` | 2,186,997 |
| | `content_objects` | 162,492 |
| | `content_references` | 159,614 |
| | `run_events` | 200,693（386 个 session 仍是 legacy 事件日志） |
| | `run_snapshots` | 1,279 |
| | `stream_runs` | 1,300 |
| | `run_snapshot_cache` | **不存在**（见 §3.5） |
| app.db | `threads` | 682（全部活行） |
| | `workspaces` | 703 |
| | `runs` | 1,970 |
| tasks.db | `tasks` | 33（未删 11） |
| | `task_runs` | 554（单任务最大 356） |

最大会话 `20261016-172323-…`：14,531 entries、20,991 blocks、entries JSON 10.5 MB + blocks 文本 17.8 MB
= **28.3 MB**。canonical 最大流 `stream_id=59`：19,340 条变更，`durable_cursor=134,288`。
legacy 最大 run：10,855 条 `run_events`。

### 1.1 端到端基线（同一台机器、同一份数据，引自 01 的取样，**不是本文测的**）

`docs/testing/remote-latency/raw/{cold,warm,snapshot}-20261104-085539.json` 是 01（T1）用 release
构建、对本机 agent 逐条命令取样得到的端到端数据（包含传输、反序列化与 handler 的全部开销）。
本文的代码结论可以直接和它对账，所以我把它摘在这里（p50，单位 ms；`warm` 为 8 次取样的 p50）：

| 命令（取样标签） | cold | warm p50 | 回复字节 | 本文对应节 |
|---|---:|---:|---:|---|
| `get_messages:first_page:largest` | 245.3 | 234.8 | 76,814 | §3.3 |
| `get_messages:deep_page:largest` | 229.8 | 237.0 | 35,340 | §3.3 |
| `get_session_entries:offset_page:largest` | **572.7** | **619.1** | 14,510 | §3.4.2 |
| `get_session_entries:first_paint:largest` | 11.8 | 11.6 | 87,009 | §3.4.1 |
| `get_session_entries:deep_page:largest` | 12.0 | 12.3 | 121,851 | §3.4.1 |
| `get_session_entries:first_paint:median` | 6.4 | 6.2 | 28,596 | §3.4.1 |
| `get_events_since:snapshot:heaviest` | 2.0 | 1.7 | 31,264 | §3.5.3 |
| `get_events_since:first_page:heaviest` | 2.2 | 2.1 | 15,642 | §3.5.1 |
| `list_sessions` | 19.3 | 20.6 | 70,213 | §3.1 |
| `list_workspaces` | 3.7 | 3.4 | 5,459 | §3.2 |
| `get_state:largest` | 22.9 | 22.0 | 1,475 | §3.12 |
| `get_state:median` | 4.5 | 4.2 | 1,488 | §3.12 |
| `list_available_skills` | 86.1 | 99.3（max 121.3） | 47,024 | §3.15 |
| `list_models` / `list_settings_models` / `get_available_models` | 21.4 / 21.1 / 21.1 | 21.4 / 21.3 / 21.4 | ~1.4-2.0 KB | §3.14 |
| `get_tool_call_args:heaviest` | 0.03 | 0.02 | 84 | §3.6 |
| `get_event_content_range:heaviest` | 0.03 | 0.02 | 66 | §3.7 |
| `get_fusion:heaviest` | 0.8 | 0.7 | 86 | §3.8 |
| `list_session_files:workspace_root` | 1.2 | 1.1 | 4,410 | §3.10 |
| `get_desktop_settings` / `get_settings` | 0.05 / 0.04 | 0.04 / 0.03 | 3,013 / 102 | §3.13 |
| `list_tasks` / `get_task:busiest` / `list_task_runs:busiest` | 4.0 / 2.0 / 2.3 | 2.8 / 1.4 / 1.7 | 9.1 / 24.8 / 11.7 KB | §3.17 |
| `list_providers` / `list_skills` | 1.9 / 1.8 | 1.9 / 1.7 | 22.3 / 12.6 KB | §3.15, §3.16 |
| `list_task_deps:busiest` | 0.004 | 0.003 | 48 | §3.17 |

两张表对照出两件本文光看代码看不出来的事：

1. **`offset_page:largest` 的 619 ms 是全场最慢的一条**，比 `get_messages`（235 ms）还慢 —— 因为它把
   §3.4.2 的 59 次串行 gRPC 和「拉全段再丢 99%」叠在一起。而它返回的只有 14.5 KB。
2. **`get_messages` 的两个页耗时几乎一样（235 vs 237 ms），回复字节却差一倍（77 KB vs 35 KB）** ——
   这正是"整段物化与整段过网"的签名：页大小不参与成本。

---

## 2. 总览：命令 → handler → 路由

路由表在 `host/src/remote_host/business/mod.rs:36-125`；派发在 `:132-190`；`get_read_chunk` 在
`host/src/remote_host/mod.rs:35-42` 被提前拦截，不进业务 handler。

| 命令 | 桌面 handler | 最终读取方 |
|---|---|---|
| `list_sessions` | `business/catalog.rs:11` | `remote_host/catalog.rs:223` → `store/{threads,runs,workspaces}.rs` |
| `list_workspaces` | `business/catalog.rs:17` | `remote_host/catalog.rs:317` → `packages/app-workspaces` |
| `get_messages` | `business/history.rs:65` | gRPC `get_messages` → `agent/src/rpc/commands/observability.rs:134` |
| `get_session_entries` | `business/history.rs:94` | gRPC `get_session_entries` → `rpc/commands/session_lifecycle.rs:988` |
| `get_events_since` | `business/history.rs:239` | gRPC `get_events_since` / `get_run_snapshot` → `rpc/broadcaster/replay.rs` |
| `get_tool_call_args` | `business/history.rs:207` | `session/tools_canonical.rs:53` |
| `get_event_content_range` | `business/history.rs:38` | `session/content/transport.rs:142` |
| `get_fusion` | `business/history.rs:193` | `agent/src/fusion.rs:393` + `orchestration/fusion/src/store.rs:492` |
| `get_read_chunk` | `remote_host/read_pages.rs:96` | 纯内存快照 |
| `list_session_files` | `business/transfers.rs:49` | `remote_host/session_files.rs:42-50` + 文件系统 |
| `download_prepare` | `business/transfers.rs:62` | `remote_host/files.rs:554` |
| `get_state` | `business/settings.rs:94` | gRPC `get_state` → `agent/src/rpc/mod.rs:446` |
| `get_desktop_settings` / `get_settings` | `business/settings.rs:20` / `:155` | `packages/app-settings/src/lib.rs:462` |
| `list_models` / `list_settings_models` / `get_available_models` | `business/settings.rs:28,104` | gRPC `list_models` + app settings |
| `list_skills` / `list_available_skills` | `business/settings.rs:32,129` | `agent/src/skills/manager.rs:146,170` |
| `list_providers` | `business/providers.rs:30` | gRPC `list_providers` → `rpc/commands/providers.rs:255`（只读 JSON 文件） |
| `list_tasks` / `get_task` / `list_task_runs` / `list_task_revisions` / `list_task_deps` | `business/tasks.rs:324,337,526,543,542` | `orchestration/tasks/src/store.rs:466,447,591,646` |

---

## 3. 逐命令审计

### 3.1 `list_sessions` — **不是 O(1)**（随 threads、runs 线性）

入口 `host/src/remote_host/business/catalog.rs:11`：

```rust
"list_sessions" => match crate::remote_host::catalog::sessions_catalog_cached("")
    .map(|catalog| (catalog.payload, catalog.signature))
```

`remote_host/catalog.rs:107-118`：缓存命中条件 = 没有"脏"标记 **且** 距上次计算 < 1 s
（`SNAPSHOT_REUSE`，`catalog.rs:86`）。命中时这次读是 O(1)（克隆一份 `SessionsCatalog`，但**克隆本身
是整份 payload 的深拷贝**，见下文）。未命中（超过 1 s、或本进程内任何写操作置了脏位）走
`sessions_catalog`（`catalog.rs:145-172`），它**先取全局锁再读库**：

- `catalog.rs:146` `let mut state = CATALOG.lock().unwrap();`
- `catalog.rs:224` `crate::store::active_run_sessions()`
- `catalog.rs:225` `crate::store::list_threads()`
- `catalog.rs:238-248` `crate::store::list_workspaces()`（整表，仅为把 id → (name,pinned,orderedAt)）
- `catalog.rs:250` `crate::store::latest_run_infos(&thread_ids)`

四条 SQL（逐字）：

```sql
-- host/src/store/threads.rs:48-52
SELECT {THREAD_COLUMNS}
FROM threads
WHERE status != 'deleted'
ORDER BY pinned DESC, COALESCE(last_message_at, updated_at, created_at) DESC

-- host/src/store/runs.rs:157-161
SELECT DISTINCT COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
FROM runs r JOIN threads t ON t.id = r.thread_id
WHERE r.status NOT IN ({TERMINAL_RUN_STATUSES_SQL})

-- host/src/store/runs.rs:305-313（682 个 ? 占位符）
SELECT thread_id, id, status, ended_at FROM (
    SELECT thread_id, id, status, ended_at,
           ROW_NUMBER() OVER (PARTITION BY thread_id ORDER BY created_at DESC, id DESC) AS rn
    FROM runs WHERE thread_id IN (?,?,…)
) WHERE rn = 1

-- packages/app-workspaces/src/lib.rs:160-162（order 在 :151-156）
SELECT {COLUMNS} FROM workspaces WHERE deleted_at IS NULL ORDER BY
  COALESCE((SELECT MAX(COALESCE(t.last_message_at, t.updated_at, t.created_at))
              FROM threads t WHERE t.workspace_id = workspaces.id AND t.status != 'deleted'),
           last_opened_at, updated_at) DESC
```

索引核对（R，实测库内确实存在）：`threads` 有 `idx_threads_recent(status,pinned,last_message_at,updated_at)`
与 `idx_threads_workspace(workspace_id)`；`runs` 有 `idx_runs_thread(thread_id,created_at)`；
`workspaces` 只有主键。EXPLAIN QUERY PLAN（M）：

- `list_threads` → `SCAN threads` + `USE TEMP B-TREE FOR ORDER BY`。
  **`idx_threads_recent` 用不上**，因为排序键是 `COALESCE(last_message_at, updated_at, created_at)`
  ——表达式与索引列不同，SQLite 仍要走全表 + 排序。这正是"看起来有索引、实际全表"的一条。
- `latest_run_infos` → 每个 thread 一次 `SEARCH runs USING INDEX idx_runs_thread (thread_id=?)`
  + `USE TEMP B-TREE FOR LAST 2 TERMS OF ORDER BY` + 两层 `SCAN (subquery)`。
- `workspaces` → `SCAN workspaces` + 每行 `SEARCH t USING INDEX idx_threads_workspace` + 临时 B-tree。

实测（M，真实数据）：`list_threads` 1.52 ms / 682 行；`active_run_sessions` 0.61 ms；
`latest_run_infos` 3.00 ms / 682 占位符；`workspaces` 0.88 ms / 703 行。合计 **约 6 ms SQL**。

规模曲线（M，等价 schema 的合成表，同一 SQL）：

| threads | list_threads | latest_run_infos |
|---:|---:|---:|
| 682 | 0.70 ms | 1.28 ms |
| 2,000 | 1.92 ms | 4.02 ms |
| 5,000 | 5.20 ms | 10.24 ms |
| 20,000 | 21.09 ms | 42.82 ms |

严格线性（≈1.05 µs/行 与 ≈2.1 µs/行），无缓和迹象。

**物化与重复序列化（R）**：`catalog.rs:149` 与 `:181-200` 对同一份 rows 做了**三次**全量 JSON 序列化：
行数组一次求签名、去掉 `lastMessageAt` 后再序列化一次求结构签名、`recency_vector`（`:202-217`）再走一遍
数组。682 行 ≈ 每次 100 KB 量级的 JSON，三次 = 三次分配 + 三次格式化的 CPU，全部发生在
`CATALOG` 锁内。

**并发形状（R + E）**：`sessions_catalog_cached` 的缓存未命中路径与 publisher 的 1 s tick 抢同一个
`CATALOG` 锁，而锁内包含上面 6 ms SQL + 3 次序列化；手机端超时 → 重连 → 再拉 的拉风暴会让后来的
读排队。代码注释（`catalog.rs:59-86`）记录了上一次测量：24 分钟内 153 次 `list_sessions`，
中位 2.6 s、p90 21 s——那已经**不是**数据库读的耗时，而是锁排队 + 冷启策略的耗时。

**N+1 / COUNT / LIKE / GROUP BY**：无 N+1（`latest_run_infos` 是单条窗口函数查询，不是每 thread 一次），
无 `COUNT(*)`、无 `LIKE '%x%'`、无 `GROUP BY` 大表。问题只在"全表 + 表达式排序 + 重复序列化 + 全局锁"。

**复杂度**：O(T + R)（T=threads，R=runs），每次缓存未命中付一次；缓存命中 O(T)（克隆 + 序列化整份 payload
交给 sink）。**不随会话内 entries 数增长**，也不随 `agent.db` 大小增长。

**端到端（§1.1）**：19.3 ms（cold）/ 20.6 ms（warm p50），回复 70,213 B。本文在上面测到的纯 SQL
只有约 6 ms，也就是说这条命令的 **2/3 时间不在 SQL 上**：它在三次 JSON 序列化、一次 `SessionsCatalog`
克隆与 70 KB 的编码/发送上。这就是为什么"库只有 682 行"的这条读能稳定吃掉 20 ms。

---

### 3.2 `list_workspaces` — **不是 O(1)**（随 workspaces 线性，含每行相关子查询）

入口 `business/catalog.rs:17` → `catalog.rs:120-130`（1 s 缓存）→ `catalog.rs:317` `read_workspaces`，
读取 `crate::store::list_workspaces()`（`host/src/store/workspaces.rs:20-23`）→
`packages/app-workspaces/src/lib.rs:143`。

SQL 与索引同 §3.1 的最后一条。计划（M）：`SCAN workspaces` + 每行相关子查询走
`idx_threads_workspace` + 临时 B-tree 排序。实测 703 行 **0.88 ms**（M）。

**复杂度**：O(W · log W)（扫描 + 排序）+ O(W) 次索引探测（子查询按 workspace 取该工作区最新线程时间）。
随工作区数线性增长，实测约 1.2 µs/行；1 万个工作区约 12 ms。**与 thread 的行数无关**（子查询是点查范围）。

**物化**：整表物化后 `read_workspaces` 再按 `kind != "user"` 在内存里过滤（`catalog.rs:317-330`）。
过滤条件没有下推到 SQL（SQL 里没有 `kind` 谓词），所以临时工作区也被读出来再丢掉。

**端到端（§1.1）**：3.7 ms（cold）/ 3.4 ms（warm），3 倍于本文测到的 0.88 ms SQL——差值同样是
序列化与传输（5,459 B），且它与 `list_sessions` 共用一个 1 s 缓存策略。

---

### 3.3 `get_messages` — **不是 O(1)**（随该会话 entries 线性，且整段物化 + 整段过网）

入口 `business/history.rs:65-93`：

```rust
"get_messages" => {
    let offset = cmd.offset.max(0) as usize;
    let limit = if cmd.limit > 0 { cmd.limit as usize } else { DEFAULT_MESSAGE_PAGE_LIMIT };
    let messages = match crate::agent_bridge::get_session_messages(cmd.session_id.clone()).await { … };
    reply(sink, true, paginate_messages(messages, offset, limit), None).await;
}
```

**桌面侧（R）**：`host/src/agent_bridge/queries.rs:314-330` 发**一条** gRPC `get_messages`，
把 agent 返回的 `messages` 数组整体收下；`packages/remote-server/src/wire_limits.rs:35` 的
`paginate_messages` → `paginate_items` 在**已物化的 Vec** 上做 `offset/limit` 切片，再序列化。
即：`LIMIT/OFFSET` 是内存里的，不是 SQL 里的。

**agent 侧（R）**：`agent/src/rpc/commands/observability.rs:134-166`
`sessions.read()` → `messages.read()` → 逐条 `ContextMessage` 映射 → `RpcResponse::ok(... json!({"messages": msgs}))`。
这里**不碰数据库**，但前提是 `session.messages` 已经在内存里。

**冷路径（R）**：`get_messages` 不在 `agent/src/rpc/commands/mod.rs:431-460` 的"免 hydrate"白名单里，
所以每次调用前都会执行 `ServerSession::ensure_history_loaded_unlocked`（`agent/src/rpc/session.rs:1867`）：

```rust
// agent/src/rpc/session.rs:1890
let loaded = manager.load(&id)?;
```

`Manager::load`（`agent/src/session/manager/loading.rs:52`）→ `load_unrepaired_entries`（`:44`）→
`SqliteStore::typed_entries`（`agent/src/session/sqlite_store/transcript.rs:29`）→
**整会话分页读取**：

```sql
-- agent/src/session/content/reader.rs:264（每页 64 条的定位查询）
SELECT position,entry_id FROM entries WHERE session_id=?1 AND position>?2 AND position<=?3 ORDER BY position LIMIT 64
-- agent/src/session/content/reader.rs:319（每页一次：entries 主体）
SELECT e.position,e.entry_id,e.entry_type,e.role,e.run_id,e.metadata_json,e.content_json,
       coalesce(v.metadata_generation,0),s.current_metadata_json
  FROM entries e JOIN sessions s ON s.id=e.session_id
  LEFT JOIN content_history_versions v ON v.session_id=e.session_id
 WHERE e.session_id=?1 AND e.position>=?2 AND e.position<=?3 ORDER BY e.position
-- agent/src/session/content/reader.rs:354（每页一次：blocks）
SELECT entry_position,ordinal,kind,text,tool_call_id,tool_name,arguments_json,is_error,metadata_json
  FROM message_blocks WHERE session_id=?1 AND entry_position>=?2 AND entry_position<=?3
 ORDER BY entry_position,ordinal
-- agent/src/session/content/reader.rs:423（每页一次：内容引用）
SELECT e.position,r.field,r.content_id,r.byte_offset,r.byte_length,r.encoding
  FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id
 WHERE r.session_id=?1 AND e.position>=?2 AND e.position<=?3 AND r.owner_kind='entry'
 ORDER BY e.position,r.field
```

索引核对：三条范围查询都落在 `entries(session_id,position)` 主键、
`message_blocks(session_id,entry_position,ordinal)` 主键、`content_references(session_id,owner_kind,owner_id,field)`
主键上；正文按引用用 `content_segments(content_id,byte_offset)` 主键分段取（`reader.rs:508-525`，
每段仍在同一事务内，不再多一次往返）。**单页是 O(64)，总量是 O(E_session)**。

实测（M，最大会话）：14,531 entries / 20,991 blocks，按 64 条一页 = **228 页**，
纯 SQL **52.7 ms**，读出 **27.6 MB JSON**（entry JSON 10,502,042 B + block 文本 17,796,761 B）。
换算：约 0.23 ms/页、约 0.5 ms/千条 entry 的纯 SQL 成本——**加上 serde 反序列化 27.6 MB、
`entries_to_agent_messages` 重建 LLM 消息、再整体序列化进回复，这一段没有在这份副本上测到**（§7）。

**放大与重复（R）**：
- 一条命令 = 1 次 gRPC + 内部 228 次数据库作业（读池只有 2 个 worker，
  `agent/src/session/database/readers.rs:12`），串行成 228 次手递手；
- agent 侧编排（`future_rpc::message::ContextMessage`）对每条消息构造一次，
  `MessageBlock::from_model(&serde_json::to_value(block))`（`observability.rs:150-157`）
  **对每个 block 多做一次序列化 + 反序列化**，属于同一份数据的第二次编解码；
- 桌面侧再做一次 `messages_vec` 克隆（`wire_limits.rs:22-27`）+ 切片 + 序列化。
  整条路径上同一份历史被序列化/反序列化 **至少 4 次**。
- `agent/src/grpc/read_admission.rs:40-44` 把 `get_messages` 放进 `HEAVY` 道（只有 **2** 个并发许可）。
  所以单条 `get_messages` 慢，还会**横向拖慢**同一条 `HEAVY` 道上的 `list_sessions`。

**上限风险（R + M）**：`get_messages` 的回复没有字节预算，只有 gRPC 帧上限
`MAX_GRPC_MESSAGE_SIZE = 32 * 1024 * 1024`（`packages/rpc/src/transport.rs:53`）。
本机最大会话实测 27.6 MB，余量 15%；再多约 4.5 MB 的工具输出，这条命令就会以 `ResourceExhausted`
失败，而不是返回一个被截断的页。

**复杂度**：冷路径 O(E_session)（库）+ O(E_session)（内存 + 编解码 + 过网）；热路径仍是
O(E_session)（序列化全部消息）。与 `threads` / `workspaces` / `tasks` 无关。
**是全库唯一一条"回复体积随会话长度无上界增长"的只读命令。**

**端到端（§1.1）**：235 ms（cold）/ 235-237 ms（warm），与上面的 52.7 ms 纯 SQL 相比，**约 180 ms
花在库里之外**（反序列化 27.6 MB、重建 LLM 消息、序列化、过网）。关键证据：`first_page`（回复 76,814 B）
与 `deep_page`（35,340 B）的耗时几乎相同，即**页大小不参与成本**，成本由会话总长决定。

**可达性（引自 01 §1.2）**：`get_messages` **不在任何已发布客户端的命令集里**（手机与桌面都不发；
它只出现在桌面 chunked-read 的名字白名单里）。所以本条是"已实现但未使用"的**潜在**风险：
一旦有客户端按旧文档调用它，它会以 235 ms 起步并随会话线性增长，而不是今天立刻变慢的原因。

---

### 3.4 `get_session_entries` — 分两条路径：`before` 路径 **O(page)**；`offset` 路径 **O(E_session)**

入口 `business/history.rs:94-165`：

- **有 `before`**（手机的主路径）：`agent_bridge/queries.rs:403` `get_session_entries_before`
  → agent `rpc/commands/session_lifecycle.rs:995` `read_history_page`，**真正走游标分页**。
- **无 `before`**（只有 offset，或都没有）：`agent_bridge/queries.rs:466` `fetch_all_session_entries_with_client`，
  以 `PAGE_SIZE = 250` **循环拉完整个会话**（`:470-486`），再交给
  `packages/remote-server/src/wire_limits.rs:164` 在内存里切片。

#### 3.4.1 游标路径（`before`）— O(page)，是本次审计里最健康的读

`agent/src/session/history/paging.rs:501` `read_history_page` 的数据库作业：

```sql
-- paging.rs:538（会话 revision，用于页缓存键）
SELECT revision FROM sessions WHERE id=?1 AND revision>=0
-- paging.rs:581（迁移被跳过则拒绝读）
SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')
-- paging.rs:205（总行数，用于把游标夹在范围内）
SELECT COALESCE(MAX(ordinal)+1,0) FROM history_display WHERE session_id=?1
-- paging.rs:212（仅当 before 给出：把"第 N 个用户回合"换成 ordinal 起点）
SELECT COALESCE((SELECT ordinal FROM history_display
                  WHERE session_id=?1 AND is_user=1 AND ordinal<?2
                  ORDER BY ordinal DESC LIMIT 1 OFFSET ?3),0)
-- paging.rs:226-236（页主体）
SELECT d.ordinal,
  CASE WHEN r.run_id IS NOT NULL THEN
    CASE WHEN header.entry_type='assistant' AND header.position=(SELECT max(a.position) FROM entries a
            WHERE a.session_id=header.session_id AND a.run_id=header.run_id AND a.entry_type='assistant')
      THEN json_set(d.payload,'$.run',{outcome},'$.usage',json_object(...))
      ELSE json_set(d.payload,'$.run',{outcome}) END
  ELSE d.payload END, NULL, header.position, header.entry_id
FROM history_display d
LEFT JOIN entries header ON header.session_id=d.session_id AND header.position=d.source_position
LEFT JOIN runs r ON r.session_id=header.session_id AND r.run_id=header.run_id
  AND EXISTS(SELECT 1 FROM entries terminal
              WHERE terminal.session_id=header.session_id AND terminal.run_id=header.run_id
                AND terminal.entry_type='run_terminal')
WHERE d.session_id=?1 AND d.ordinal>=?2 AND d.ordinal<?3 ORDER BY d.ordinal {ASC|DESC}
```

索引核对与实测计划（M）：

- `MAX(ordinal)` → `SEARCH history_display USING COVERING INDEX sqlite_autoindex_history_display_1 (session_id=?)`，
  O(log n) 取索引末端，与行数无关。
- 页主体 → `SEARCH d USING INDEX … (session_id=? AND ordinal>? AND ordinal<?)`，
  两个相关子查询都走 `entries_run(session_id,run_id)`（`EXPLAIN` 输出 `CORRELATED SCALAR SUBQUERY 1/2`）。
- `paging.rs:212` 那一条是唯一带 `OFFSET ?3` 的：`OFFSET` = 用户回合数-1（手机传 3），
  在 `history_users(session_id,is_user,ordinal)` 上跳几行，实际是常数，**不是**深 `OFFSET`。

实测（M，最大会话，真实数据）：

| 页行数 | 返回行 | 耗时 |
|---:|---:|---:|
| 150（首屏上限 `HISTORY_TAIL_ROWS`，`paging.rs:34`） | 150 | 2.4 ms |
| 300 | 300 | 3.5 ms |
| 600（翻页上限 `HISTORY_PAGE_ROWS`，`paging.rs:43`） | 600 | 5.9 ms |
| 1（另一条只有 1 行的会话，整段历史） | 1 | 0.0 ms |

**线性于页行数（≈10 µs/行），与 14,145 行的会话总长无关**。这就是 keyset 分页应有的形状。
页内正文由 `paging.rs:346` `materialize_display_page` 用**一次批量读**（`reader.rs:291` →
`page_impl` 的四条语句）取回，不是每条 entry 一次查询；代码注释记录了批量化前后的对比：
"one entry costs ~40 round trips… a page of 109 entries cost 1,426 of them — 26 ms"
（`reader.rs:285-290`）——即**批量后 4 条语句/页**。

**端到端（§1.1）**：首屏最大会话 11.8 ms（cold）/ 11.6 ms（warm）、上滑页 12.3 ms、中位会话首屏 6.2 ms。
与本文 2.4 ms 的纯 SQL 相比，约 9 ms 在编排、两段 gRPC（`verify_session_database` 一次 +
`get_session_entries` 一次）、反序列化与 87-122 KB 的编码上。**这是全库唯一一条"页内固定成本、
与总量无关"的读路径**，也是整个远端协议应该对齐的形状。

仍然存在的常数成本（R）：

- **两个相关子查询按行执行**（`paging.rs:228, 235`）：每行一次 `entries_run` 探测。
  它们不是 O(1)/行，而是 O(该行的 run 的 entries 数)/行（两个子查询的 `run_id` 是常量，
  但 `max(a.position)` 与 `EXISTS(... entry_type='run_terminal')` 都要在 `run_id` 范围内多读几行）。
  实测 600 行 5.9 ms 说明这个常数很小（≈10 µs/行），但在"单 run 上千条 entry"的会话上会变大。
- **页缓存**（`paging.rs:111-160`，8 页 / 16 MB）：同一 revision 下同一页只物化一次；
  任何写入都会推进 revision 使缓存失效，所以"边跑边看"时每页都要重算。
- **历史投影重建**（`paging.rs:558-580` → `recovery.rs:106` `rebuild_history_index`）：
  当 `history_display` 的 `projection_version` 不匹配时，第一次读会 `DELETE` 该会话的全部
  `history_display` 行并按 64 条一批重建（`recovery.rs:65-92`）。本机 672 个会话的
  `projection_version` 全是 2（即当前版本），**当前不触发**；但只要 `PROJECTION_VERSION`
  （`recovery.rs:16`）被提升一次，**每个会话的第一次历史页读**都会付一次 O(E_session) 的重建
  ——最大会话是 14,545 行。这是一条"升级瞬间全体变慢"的隐藏全表写，不是稳态成本。

#### 3.4.2 offset 路径（无 `before`）— **O(E_session)，且 N 次 gRPC**

`agent_bridge/queries.rs:466-487`：

```rust
const PAGE_SIZE: i64 = 250;
let mut offset = 0_i64;
loop {
    let page = read_history_page_with_client(client, get_session_entries_page_command(session_id, offset, PAGE_SIZE)).await?;
    entries.extend(page.entries);          // 全部页都留在一个 Vec 里
    if !page.has_more { return Ok(entries); }
    …
}
```

- 单页是 O(250) 且有索引（同 §3.4.1 的 `offset` 分支 `paging.rs:216-221`，`LIMIT 64` 的 forward 分支在
  `paging.rs:252`），但**循环把全部页累加**：最大会话 14,531/250 ≈ **59 次 gRPC 往返**，
  每次往返都要在 agent 侧重新取锁、重新 `session_key`/`format` 探测。
- 结果回到桌面后，`history.rs:174` `paginate_items_with_cap(entries, offset, limit, "entries", …)`
  在内存里切片并序列化——**又一次整段物化 + 整段过网**。
- 若客户端连 `offset` 也不给，canonical 会话会被 agent 直接拒绝：
  `session_lifecycle.rs:1020-1023` 返回 `history_paging_required`（因为 `before`/`offset` 全缺时
  只能走 `manager.load` 全量投影，而 canonical 会话要求显式游标）。旧手机就是这样拿到错误的。

**复杂度**：O(E_session) 数据库 + O(E_session/250) 次 RPC + O(E_session) 内存。这是"假分页"，与
`get_messages` 同量级，但额外多出几十次跨进程往返。

**端到端（§1.1）**：**572.7 ms（cold）/ 619.1 ms（warm p50）**，而回复只有 14,510 B。
这是整份审计里最长的一条只读命令，也正是"59 次串行 gRPC + 整段物化 + 最后丢掉 99%"的直接后果。
**可达性**：手机与发布的桌面客户端都用 `before`（01 §2.1），走 §3.4.1 的健康路径，所以这个 619 ms
今天不会出现在用户眼前；它是"offset 分页一旦被调用就是灾难"的实测上限。

---

### 3.5 `get_events_since` / `get_run_snapshot` — 分页时 **O(page)**；不分页时 **O(run 事件数)**

入口 `business/history.rs:239-410`。三条分支：

1. `prefer_snapshot && chunked && since_idx == -1 && offset == 0` → `get_run_snapshot`（`:240-266`）。
2. `chunked && offset == 0` → `get_events_since_page_until`（限页）。
3. 其余（**非 chunked 的旧客户端**）→ `get_events_since`（`:327`），**agent 侧全量排空**。

#### 3.5.1 分页路径（canonical 会话）

`agent/src/session/content/journal.rs:892` `segmented_replay`，每页 `remaining.min(128)`（`:944`）：

```sql
-- 每页先确认存储身份不变
-- agent/src/session/content/journal.rs:944 → transport::storage_identity
-- agent/src/session/content/stream.rs:318（流定位）
SELECT id,durable_cursor FROM stream_runs
 WHERE session_id=?1 AND run_id=?2 AND epoch=?3
   AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=stream_runs.id)
-- agent/src/session/content/stream.rs:326（keyset 取页）
SELECT c.id,length(CAST(c.metadata_json AS BLOB))+length(CAST(x.descriptor_json AS BLOB))+x.symbol_bytes
  FROM stream_changes c
  LEFT JOIN stream_contexts x ON x.id=c.context_id AND x.stream_id=c.stream_id AND x.event_kind=c.event_kind
 WHERE c.stream_id=?1 AND c.last_cursor>?2 ORDER BY c.last_cursor LIMIT ?3
```

索引：`UNIQUE(stream_id,last_cursor)`（`agent/src/session/content/schema.sql:170`）→
`stream_changes` 上的 `WHERE stream_id=? AND last_cursor>? ORDER BY last_cursor` 是纯索引范围扫描。
实测（M，19,340 条变更的最大流）：首页 129 行元数据 **0.08 ms**。正文由
`materialize_page`（`journal.rs:705`）**批量**取（`read_inline_bodies`），不是每条一次查询。
⇒ **O(page)，与流的长度无关**。

#### 3.5.2 非分页路径（旧客户端 / 无 limit）

`agent_bridge/queries.rs:193` 的 `get_events_since` 会**排空 agent 返回的所有页**
（`:130-190` 的合并循环：`total.events.append(&mut page.events)` 直到 `has_more` 为假）。
legacy 会话落到 `agent/src/session/journal_compaction.rs:365` `replay_page`：

```sql
-- journal_compaction.rs:387（snapshot 覆盖则直接返回）
SELECT cursor,CASE WHEN cursor>?3 THEN payload END FROM run_snapshots WHERE session_id=?1 AND run_id=?2 AND version=1
-- journal_compaction.rs:391
SELECT EXISTS(SELECT 1 FROM run_events WHERE session_id=?1 AND run_id=?2)
-- sqlite_store/read_replay_event_rows → run_events 按 idx 范围（UNIQUE(session_id,run_id,idx,epoch)）
```

实测（M）：10,855 条事件的完整重放 SQL **4.9 ms**（读全部 payload），`max(idx)` / `min(idx)`
各 0.002-0.003 ms（走覆盖索引）。⇒ **O(run 事件数)，绝对量不大但完全线性**；
再乘上桌面把整段 JSON 放进一次回复的开销。

**这个形状的临时排序已经被绕开（R + M）**：`ORDER BY epoch,idx` 在 SQLite 里会走
`USE TEMP B-TREE FOR ORDER BY`（M，`EXPLAIN` 实测），但生产代码早就绕开了它——
`read_replay_event_rows`（`agent/src/session/sqlite_store/events.rs:483-496`）在非 session_scope 时直接改调
`read_replay_event_rows_by_epoch`（`:516`），逐个 epoch 做 `idx >` 范围读；注释里记了同批 1,001 行的
对比：**0.98 ms vs 20.4 ms**，顺序不变。它依赖的 `run_events_replay_order(session_id,run_id,epoch,idx)`
由当前构建在开库时建（`schema.rs:52-60`，实测：T1 派生的隔离 HOME 里有它，本机生产库没有）。
即使没有这个索引，per-epoch 的 `idx >` 仍能走 `UNIQUE(session_id,run_id,idx,epoch)` 的索引范围，
不会退化成临时 B-tree —— 所以上面 4.9 ms 的量级已经是这条路径的真实形状。

#### 3.5.3 `get_run_snapshot`（快照折叠）

`agent/src/rpc/broadcaster/replay.rs:371`：

- **命中缓存 → O(1)**：`:403` `store.run_epoch(...)`（`stream_runs` 点查）+ `:404`
  `store.cached_run_snapshot(...)`（`run_snapshot_cache` 主键 `(session_id,run_id)` 点查）。
- **未命中 → O(run 事件数)**：`:415` `read_journal_page(run_id, -1, false, None)`（无 limit → 全量），
  再 `fold_events`（`:432`）折出投影。
- **本机实测：`run_snapshot_cache` 表不存在**（M，`sqlite_master` 里没有这张表）。
  `agent/src/session/database/schema.rs:25` 在**每次打开库**时都执行 `schema.sql`（`IF NOT EXISTS`），
  该表也由 `agent/src/session/database/schema.sql:123` 定义，所以"表不存在"只有一种解释：
  **写出这份数据的 agent 进程比当前源码旧**，它还没有这段 DDL。
  后果是：`:404` 的 `store.cached_run_snapshot(...)?` 直接返回 `no such table` 错误，
  `handle_get_run_snapshot`（`observability.rs:281-294`）的错误分类列表里没有这一项，
  于是落到 `:295` 的兜底 `build_fail`；桌面的 `agent_bridge/queries.rs:218-226` 只在
  `unknown command` / 两个字节预算码时才 `Ok(None)` 退回原始重放，其余一律 `Err`（`:227`）。
  ⇒ **在这台机器当前运行的 agent 上，`get_run_snapshot` 不是慢，而是必定失败**；
  而一旦换到当前源码编译的进程，命中缓存后它才是真正的 O(1)。
  这是本次审计里唯一一条"生产环境与源码不一致"的结论，已在 §7 标注。
- **在 T1 的隔离环境里它已经生效**（M，§1.1）：`get_events_since:snapshot:heaviest` 端到端 **2.0 ms
  （cold）/ 1.7 ms（warm）**、回复 31,264 B，且隔离 HOME 的 `run_snapshot_cache` 里确实有 1 行折叠。
  这条路径**一旦命中就是整个协议里最快的整段重放**，与“未命中就排空整个 run”形成两个量级。
- 快照有字节预算：`observability.rs:317-325`，超过 `EVENTS_PAGE_BYTE_BUDGET` 返回
  `run_snapshot_too_large`，客户端退回分页重放。

#### 3.5.4 桌面侧对每个事件做的额外工作（R）

`business/history.rs:379-390`：遍历 `data["events"]`，对每个事件 `data` 字符串做
`serde_json::from_str::<Value>(raw)` 只为检查 `content_key`；随后
`remote_host/lean.rs`（→ `packages/remote-server/src/lean.rs:427` `lean_replay_page`）
在 lean 客户端下**再**对每个事件的 `data` 字符串解析一次并重写。也就是说：一页事件在桌面侧被
**二到三次** JSON 解析，全部发生在命令的关键路径上（不碰数据库，但随页内事件数线性）。

**复杂度**：分页 O(page)；`get_run_snapshot` 命中 O(1) / 未命中 O(run 事件数)；不分页 O(run 事件数)。

---

### 3.6 `get_tool_call_args` — **O(1)**（6-7 次点查）

入口 `business/history.rs:207-230`（需要 lean 能力声明）→ `agent_bridge/queries.rs:381` →
agent `session/tools_canonical.rs:53` `arguments`：

```sql
-- tools_canonical.rs:22-27（empty_reservation：7 个 EXISTS，常数）
SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.id=?1 AND s.revision=-1
  AND NOT EXISTS(SELECT 1 FROM entries e WHERE e.session_id=s.id)
  AND NOT EXISTS(SELECT 1 FROM content_operations o WHERE o.session_id=s.id)
  AND NOT EXISTS(SELECT 1 FROM fork_operations f WHERE f.child_session_id=s.id)
  AND NOT EXISTS(SELECT 1 FROM session_tombstones d WHERE d.session_id=s.id)
  AND NOT EXISTS(SELECT 1 FROM legacy_imports l WHERE l.session_id=s.id))
-- reader.rs:147（capture_from：会话内最大 position）
SELECT coalesce(max(position),-1) FROM entries WHERE session_id=?1
-- tools_canonical.rs:47（定位那一条 tool_call）
SELECT e.position,e.entry_id,b.ordinal,b.tool_name,b.is_error,e.timestamp_ms
  FROM entries e JOIN message_blocks b ON b.session_id=e.session_id AND b.entry_position=e.position
 WHERE e.session_id=?1 AND e.run_id=?2 AND b.tool_call_id=?3 AND b.kind=?4
 ORDER BY e.position DESC,b.ordinal DESC LIMIT 1
-- reader.rs:683（取 arguments 引用）
SELECT r.content_id,r.byte_offset,r.byte_length,r.encoding
  FROM content_references r JOIN entries e ON e.session_id=r.session_id AND e.entry_id=r.owner_id
 WHERE e.session_id=?1 AND e.position=?2 AND e.entry_id=?3 AND r.owner_kind='entry' AND r.field=?4
-- 再按 content_segments(content_id,byte_offset) 主键分段取正文
```

索引核对（R + 实测索引存在）：`message_blocks_tool(session_id,tool_call_id,kind)` 覆盖定位条件；
`entries(session_id,position)` 主键覆盖 join；`content_references` 主键
`(session_id,owner_kind,owner_id,field)` 覆盖引用；`content_segments` 主键覆盖正文。
`max(position)` 走 `entries` 主键索引末端（`SEARCH entries USING COVERING INDEX sqlite_autoindex_entries_1`，M）。
**全部是点查/范围点查，与 entries 总数、会话长度无关。**

代价是**固定 4 次读作业 + 1 次 verify**（`reader.rs:193-203` 的最终授权检查会**再读一次当前数据**），
即"常数 5-6 次数据库作业"。绝对量级远小于 1 ms，但它是"每展开一个工具行一次"的命令
（手机 Lean 页面的按需展开），值得知道它的地板是 5-6 次往返而不是 1 次。

---

### 3.7 `get_event_content_range` — **O(取回字节)**，与数据规模无关

入口 `business/history.rs:38-60` → agent `rpc/commands/mod.rs:194-220`
（先 `flush_journal` 篱笆，再 `event_content_range`）→ `session/content/transport.rs:142`：

```sql
-- transport.rs:303（按 (session,run,epoch,cursor,field) 定位不可变范围）
SELECT b.content_id,b.byte_offset,b.byte_length,b.encoding
  FROM stream_runs r
  JOIN stream_changes c ON c.stream_id=r.id
  JOIN stream_content_ranges b ON b.change_id=c.id
 WHERE r.session_id=?1 AND r.run_id=?2 AND r.epoch=?3 AND c.last_cursor=?4
   AND c.last_cursor<=r.durable_cursor AND b.field=?5
   AND NOT EXISTS(SELECT 1 FROM retired_streams d WHERE d.stream_id=r.id)
```

索引：`stream_runs` 的 `UNIQUE(session_id,run_id,epoch)`、`stream_changes` 的
`UNIQUE(stream_id,last_cursor)`、`stream_content_ranges` 是从 `stream_changes`/`stream_contexts`
派生的视图（`schema.sql:196-204`）→ 全部点查。正文按 `content_segments(content_id,byte_offset)`
主键分段取（`transport.rs:183-200`），`max_bytes` 被夹在 `1..=READ_BYTES`（`:152-155`）。
⇒ **O(返回字节)**，不随 entries/事件量增长。唯一的规模项是 `NOT EXISTS retired_streams` 与
`min(c.first_cursor)` 那些点查，常数。

---

### 3.8 `get_fusion` — O(≤256 事件) +（带 step 时）**O(该 step 会话的 entries)**

入口 `business/history.rs:193-206` → `host/src/fusion.rs:1055` → agent `fusion.rs:393` `query_discussion`：

```sql
-- orchestration/fusion/src/store.rs:494（事件，硬上限 256）
SELECT seq,kind,data,timestamp FROM fusion_events WHERE run_id=?1 ORDER BY seq LIMIT 256
```

索引：`fusion_event_run(run_id,seq)` 覆盖；`fusion_steps(run_id)`、`fusion_runs` 主键。
⇒ 主路径 **O(min(事件数, 256))**，即常数上限。

**带 `step_id` 时（`fusion.rs:427-455`）**：`state.session_manager.load(&step.session_id)`
——把**该 step 自己的子会话整段载入**（同 §3.3 的 `Manager::load` 路径），
再 `serde_json::to_string` 整段 transcript，最后按 `cmd.offset` 做**字节切片**（`:449-453`）。
即：按字节窗口返回，但**先物化全部**，O(E_step_session)。子会话就是一次融合参与方的完整对话，
量级与一次 agent run 相当。

另外（R）：`Store::existing` + 每次 `connect()`（`orchestration/fusion/src/store.rs:492` 系列）
在一条命令里**重复开连接**（`discussion` → `get` / `steps` / `events` 各自 connect、各自 prepare）。
每条语句都要重新编译一次，没有语句缓存。

**复杂度**：无 step：O(1)（≤256 行）；有 step：O(E_step_session)（物化 + 全量序列化）。

---

### 3.9 `get_read_chunk` — **O(chunk)**，但每次调用 O(快照数)

入口 `host/src/remote_host/mod.rs:35-42` 直接调 `read_pages::read`（`read_pages.rs:126-160`），
不进业务层。快照是内存里的 `Vec<u8>`（`read_pages.rs:26-33`），`chunk()`（`:96-118`）
按 `offset` 切 `CHUNK_BYTES = 192 KiB`（`:15`）。**不碰数据库。**

唯一随规模变化的是 `Cache::prune()`（`:34-38`，`retain` 遍历全部快照）与 `insert`（`:39-77`）
里 `self.snapshots.values().map(len).sum()`——O(快照数)；快照总数受 `MAX_CACHE_BYTES = 32 MiB`
限制，所以是"有界线性"，量级为几十个条目。**不是数据规模问题。**

---

### 3.10 `list_session_files` — 数据库 O(1)（3 次点查，其中 2 次重复）；文件系统 O(目录项)

入口 `business/transfers.rs:49` → `remote_host/session_files.rs:42-50`：

```rust
let thread = crate::store::find_thread_by_agent_session(session_id)?   // session_files.rs:46
let root = crate::agent_bridge::workspace_path_for_thread(&thread.id)?; // :50
```

- `find_thread_by_agent_session`（`host/src/store/threads.rs:77-89`）：
  `SELECT … FROM threads WHERE agent_session_id = ?1 AND status != 'deleted' LIMIT 1`
  → 走 `idx_threads_agent_session_unique`（部分唯一索引，实测存在），**1 次点查**。
- `workspace_path_for_thread`（`host/src/agent_bridge/session.rs:263-269`）又
  `store::get_thread(thread_id)`（**同一行再读一次**）+ `store::get_workspace(...)`。
  ⇒ 一条命令 **3 次点查，其中 1 次是重复读同一行**。

随后（`session_files.rs:52-108`）：`root.canonicalize()`、`path.canonicalize()`、
`read_dir` 遍历、**每个条目**再 `entry.path().canonicalize()`（一次 syscall + 一次 stat 链）
+ `target.metadata()` + `ensure_path_allowed`，最后在内存里排序。

**复杂度**：数据库常数；文件系统 O(条目数 × 路径深度)（canonicalize 逐段解析）。
一个含数千文件的会话工作目录会让这条命令从"1 ms"变成几十毫秒——**与账户数据量无关，
与那个目录有关**，属于本报告要区分出来的另一维度，不是本次的数据库问题（列出以免被误判）。

---

### 3.11 `download_prepare` — 给了 `name` 时 O(1)；**没给时 O(E_session)**

入口 `business/transfers.rs:62` → `remote_host/files.rs:554` `prepare_download_variant`：

```rust
let (source, display_name) = match requested_name.map(str::trim).filter(|n| !n.is_empty()) {
    Some(name) => (download_source(session_id, requested_path)?, name.to_string()),   // files.rs:565
    None => {
        let entries = crate::agent_bridge::get_session_entries(session_id.to_string()).await?; // files.rs:570
        match session_attachment_name(&entries, requested_path) { … }
```

`files.rs:545-553` 的注释本身就是上一轮测量的记录："fetching *every entry of the session*
(250 per page, megabytes of JSON for a long conversation), on every file open, and once per inline image
in a Markdown preview"。⇒ 老客户端（只发 path）会为"一张内联图片"付一次整会话拉取
（同 §3.4.2 的 59 次 RPC，最大会话 59 × 250 条）。新客户端发 `name` 时只走
`resolve_local_link`（`files.rs:515-523`）= 2 次点查 + `canonicalize`。

每次调用另有 `prune_expired()`（`files.rs:560`）：遍历传输表，O(在途传输数)。

**复杂度**：O(1)（带 name）/ O(E_session)（不带 name，且是"每张图一次"）。
**可达性（引自 01 §2.2）**：当前手机与桌面都**必带** `name`（`mobile/src/remote/transfer.ts:290-293`
写明 name 缺失就要重读整段历史），所以今天的实际路径是 O(1)；O(E_session) 分支是一个兼容旧客户端的退路，
而不是当前成本。

---

### 3.12 `get_state` — 温路径 **O(会话条目)**（两条 legacy 专属全量读）；冷路径 O(条目) 含首次 hydrate

入口 `business/settings.rs:94` → `agent_bridge/queries.rs:524` → agent `rpc/commands/observability.rs:96`
→ `agent/src/rpc/mod.rs:446` `get_state_internal`。

- 命令主体（`rpc/mod.rs:452-560`）**没有显式的表读**：模型注册表（`state.model_registry`，进程内缓存）、
  `AtomicI64` 计数器、`cumulative_cost` 都在内存里。**"无 SQL"只对常量部分成立**：空闲会话（没有活动 run，
  也就是用户点开会话时最常见的状态）在命令体内部还有**两处按条目数工作的读**，两处都按
  `session_formats.format != 2` 分流，只对 legacy 会话生效：
  1. `rpc/mod.rs:540` `sess.session_manager.state_metadata(&session_id)` →
     `agent/src/session/manager/loading.rs:13` `state_metadata`：格式不是 2 就落进 `:26`
     `self.load_metadata(id)` → `:68`；legacy 分支在 `:100` 执行
     `SELECT payload FROM history_shapes WHERE session_id=?1 ORDER BY position`，**取全部行并逐行
     `serde_json::from_str`**，只为数出"user 条目几条"和取一个 parent id。会话已 hydrate 时这个结果
     基本不被使用（`query_count` 只在 `!history_loaded` 时用它，`parentSessionId` 在内存字段为空时才用它）。
  2. `rpc/mod.rs:635` `.unterminated_run_id(&session_id)` → `agent/src/session/manager/mod.rs:321` →
     `agent/src/session/sqlite_store/transcript.rs:203` `read_run_marker_headers`：格式不是 2 就 `:206`
     `run_marker_rows(db, session, "entry_records")`。**`entry_records` 是一个视图**（2,324 B SQL、五个 CTE），
     语句是 `SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type IN (...)`——视图不可索引，
     SQLite 必须先把该会话的行摊出来再按 `entry_type` 过滤。
- 两处都只对 legacy 会话生效；canonical（format=2）会话走 `entries` 计数（`loading.rs:19`）与
  `entry_header_records` 的定点读（`transcript.rs:210`），所以**同一份数据里 canonical 会话是平的**：
  1 条 3.58 ms → 2,043 条 3.94 ms，跨 2,043 条只动 0.36 ms（05 §3.3）。
- 会话装配部分仍在：`get_state` 要求会话已在 `state.sessions` 里，`commands/mod.rs:342`
  `state.try_get_session` 冷会话会走 `rpc/mod.rs:126-217`：`contains()`（`sqlite_store/mod.rs:18-24`，
  2 次点查：`sessions` 主键 + `legacy_imports` 主键）→ `switch_session_metadata`（读 `sessions` 行与
  metadata）→ 必要时 `set_model` 写一条 `session_info`。
- `get_state` 在 `commands/mod.rs:431-460` 的白名单里，**不触发整会话 hydrate**——这是它比
  `get_messages` 好的地方，值得作为对照；但"不 hydrate"不等于"没有整会话读"，见上面的两处。

**复杂度**：温路径 **O(会话条目)**（仅 legacy 会话；canonical 会话 O(1)）；冷路径另加一次首次 hydrate
（O(条目)，每个会话只付一次）+ 约 3-5 次点查与会话装配。

**端到端与归因（§1.1、05 §3.3）**：最大会话 22.9/22.0 ms，中位会话 4.5/4.2 ms。桌面的
`get_session_state`（`queries.rs:524-536`）在发 `get_state` 之前还做了 `verify_session_database`
（`presence.rs:118-133`：2 次点查 + **一次额外的 gRPC**）和 `ensure_observer_for_session`
（`settings.rs:96` → `observer.rs:265-277`：又一次 `find_thread_by_agent_session`），所以这条命令是
**3 次串行往返 + 3-4 次点查**；但**它们不是斜率来源**：在同一台机器上把上面两处读摘掉之后
（05 §3.2 的 A、B、A+B 三个变体），这些装配逻辑仍在、仍每次执行，而各档的 Δ 都归零。
归因结果（Δ = p50 − 同轮 canonical 地板，ms）：

| 变体 | Δ@14,531 | Δ@25,000 | Δ@50,000 | 斜率 |
| --- | ---: | ---: | ---: | ---: |
| baseline | **17.61** | **29.76** | **59.80** | 1.19 µs/条目 |
| A（只摘 `state_metadata`） | 5.96 | 8.91 | **19.35** | 0.39 µs/条目 |
| B（只摘 run-marker 视图扫描） | 11.88 | 20.88 | 41.92 | 0.84 µs/条目 |
| **A+B** | **−0.12** | **0.06** | **0.05** | **0.01 µs/条目** |

两段可加（50,000 条：40.45 + 17.88 ≈ 58.33 ≈ 59.80），说明是两段互不重叠的读；`state_metadata` 占 68%，
`entry_records` 视图扫描占 30%。A+B 之后 41 条 4.18/4.64 ms、50,000 条 4.52/4.64 ms，与会话长度无关。
因此修复判据是**两处都改**：只改 `state_metadata` 时 50,000 条仍有 19.35 ms，不满足"与会话长度无关"。

**cold 是另一条原因**（05 §3.5）：cold 的主体是**首次 hydrate 该会话**（`typed_entries` 读全量转写），
语义上就是 O(条目)、每个会话付一次；A+B 只拿掉约 25%（50,000 条 173.7 → 127.8 ms）。要压 cold 只能压
hydrate 本身（分片 hydrate，或单独读出首页所需的最小状态），它与温路径那两处不是同一个判据，
不要并成一条根因。

---

### 3.13 `get_desktop_settings` / `get_settings` — **O(1)**（12 次主键点查）

`business/settings.rs:20` → `store/app_settings.rs:57` → `packages/app-settings/src/lib.rs:462-513`：
`read_value` 对 **12 个 key** 各发一次
`SELECT value FROM app_settings WHERE key = ?1`（`lib.rs:394-400`，`app_settings` 主键 = key）。
表只有十几行。

⇒ 常数 12 次点查（N 次查询但 N 是常量），**不随任何数据维度增长**；实测 0.05/0.04 ms（§1.1）。
`get_settings`（`settings.rs:155`）另外调 `product_sandbox_status()`（`settings.rs:318-333`）：
**macOS 上它直接返回 `(true, None)`，不探测**，所以本机的 0.03 ms 是纯设置读；
而 Windows/Linux 上它会 `crate::agent_bridge::probe_sandbox()`（**一次额外的 agent RPC**），
同一条命令在这两个平台上会多一个往返——本报告的数字不覆盖它。

---

### 3.14 `list_models` / `list_settings_models` / `get_available_models` — O(1)

`settings.rs:28,104` → `agent_bridge/queries.rs:542` → agent `rpc/commands/providers.rs:160`
`list_models_response`：拿 `&state.model_registry.read()`（**进程内缓存，不重建**，
`commands/mod.rs:90-96`），但随后对**整个内置目录**做一次过滤 + 排序：
`registry.all_models()` → 先按 `output` 过滤 → 再对剩下的每个模型做
`registry.is_model_available(format!("provider/id"))`（`providers.rs:180-186`；该文件自己的注释说
这是"by far the most expensive test here"，因为它要拿单个 `provider/id` 到全目录里解析）→
`sort_by` → `dedup_by`。`list_models` 分支（桌面 `settings.rs:104`）另外调一次
`store::get_app_settings()`（12 次点查，见 §3.13）只为过滤被隐藏的模型（内存 `retain`）。

**复杂度**：**O(目录模型数)**，不是 O(1)：以 ~3.8k 个内置模型计，实测端到端 21.1-21.4 ms
（§1.1，三个别名同价，说明它们走同一段代码）。它不随账户数据（会话/entries/threads）增长，
但它是一个 **21 ms 的常数**，已经占 100 ms 预算的 1/5。

---

### 3.15 `list_skills` / `list_available_skills` — **读路径里有同步网络请求**

- `list_skills` → `agent_bridge/skills.rs:20` → agent `skills/manager.rs:170` `list_installed`：
  - `recover()` + `reconcile_once()`（`manager.rs:599-611`）：先读 `skills_meta.reconciled`（点查），
    再 `skills_fingerprint()`（`manager.rs:622-650`）**遍历两个 skills 根目录下的每个子目录**，
    对每个 `SKILL.md` 做 `is_file()` + mtime stat。指纹变了就整体 reconcile（重扫 + 重写）。
  - `SELECT name,version,scope,source,location FROM skill_installations ORDER BY name, …`（`manager.rs:175`）
    ——**全表扫描 + 排序**（表小）。
  - 然后**每个 skill 读一次 `SKILL.md`**（`manager.rs:196`）——经典 N+1（文件系统版）。
- `list_available_skills` → `manager.rs:146` `catalogue_with_status` → `manager.rs:130` `catalogue()`：

```rust
let url = format!("{}/client/v1/skills", self.platform_url);
Ok(self.client.get(url).send()?.error_for_status()?.json::<Response>()?.skills)
```

  客户端构造时设的是 **connect_timeout 10 s / timeout 60 s**（`manager.rs:92-103`）。
  这条命令因此**可以在数据库完全空闲的情况下耗时 60 秒**，而它的名字像一条本地读。
  之后才是对每个目录项的 `SELECT source,version FROM skill_installations WHERE name=?1`（`manager.rs:152-159`）
  ——**N+1**（每个目录技能一次点查）。

**复杂度**：`list_skills` O(skills)（含 O(skills) 次文件读 + 目录 stat 遍历）；`list_available_skills`
O(网络) + O(目录项)。**两者都不随账户数据规模（会话/entries）增长**，但前者随已安装技能数线性、
后者随网络状况无界。这是本次审计里唯一"最坏情况延迟 > 数据库本身"的一类。

**端到端（§1.1）**：`list_skills` 1.8/1.7 ms（本机技能数小，看不出斜率）；
`list_available_skills` **86.1 ms（cold）/ 99.3 ms（warm p50，max 121.3 ms）**——在 33 条取样里
排第二（仅次于 offset 分页与 get_messages），而它的 47 KB 回复和本地数据库一点关系都没有：
这 86-99 ms 全花在那一次 HTTP GET 上。**它是本次审计发现的最大的"非数据库"延迟来源，
并且是唯一一条不可用数据规模预测、也不随缓存变快的读。**

---

### 3.16 `list_providers` — O(1)（只读 JSON 文件，无数据库）

`business/providers.rs:30` → `agent_bridge/config.rs:23` → agent `rpc/commands/providers.rs:255`
`provider_view`：`crate::config::providers::read_provider_documents()`（读 `models.json` + `auth.json`），
`catalog` 由注册表在内存给出。桌面侧再 `serde_json::from_str` 一次。
⇒ O(provider 数)，与会话/条目规模无关。**无 SQL。**

---

### 3.17 `list_tasks` / `get_task` / `list_task_runs` / `list_task_revisions` / `list_task_deps`

- **`list_task_deps`**：`business/tasks.rs:542` 直接回 `{"deps": []}`。**O(1)**（功能已删除）。
- **`list_task_revisions`**：`tasks.rs:543` → `store.rs:646` `list_revisions`：
  `SELECT … FROM task_prompt_revisions WHERE task_id = ?1 ORDER BY version`
  → `UNIQUE(task_id,version)` 直接给出顺序（实测计划：`SEARCH … USING INDEX sqlite_autoindex…`）。
  **O(版本数)，无排序开销。**
- **`get_task`**：`tasks.rs:337` → `store.rs:447` `SELECT … FROM tasks WHERE id = ?1`（主键，O(1)），
  但 `task_detail_view`/`task_list_view`（`tasks.rs:60-80, 83-…`）**每行再调一次
  `latest_run_for_task`**（`store.rs:599` → `list_runs_for_task`）。
- **`list_tasks`**：`tasks.rs:324` → `store.rs:466`：

```sql
SELECT {TASK_COLUMNS} FROM tasks WHERE deleted_at IS NULL ORDER BY created_at DESC, rowid DESC
```

  实测计划：`SCAN tasks USING INDEX tasks_active_name` + **`USE TEMP B-TREE FOR ORDER BY`**（M）。
  随后 `tasks.rs:327-330` 对**每个 task** 调 `task_list_view` → `latest_run_for_task` ⇒ **N+1**。
- **`list_runs_for_task`**（`store.rs:591`），`SQL_RUNS_FOR_TASK`（`store.rs:79-80`）：

```sql
SELECT %COLUMNS% FROM task_runs WHERE task_id = ?1 ORDER BY started_at DESC LIMIT ?2
```

  `task_runs` 上只有 `UNIQUE(task_id,due_at)`（实测索引列表：
  `sqlite_autoindex_task_runs_1/2`，**没有 `(task_id, started_at)`**）。计划（M）：
  `SEARCH task_runs USING INDEX … (task_id=?)` + **`USE TEMP B-TREE FOR ORDER BY`**。

逐条实测（M）：

| 场景 | 结果 |
|---|---|
| 真实库单任务 356 次运行，`LIMIT 20` | 0.117 ms |
| 真实库单任务 27 次运行，`LIMIT 20` | 0.020 ms |
| 合成 20 / 100 / 356 / 1000 / 5000 / 20000 次运行，`LIMIT 20` | 0.022 / 0.039 / 0.095 / 0.238 / 0.991 / 3.286 ms |
| 合成 5000 次运行，`LIMIT 1` **vs** `LIMIT 20` | **0.947 ms vs 0.991 ms** |
| `list_tasks`（11 个任务） | 0.010 ms |

`LIMIT 1` 与 `LIMIT 20` 同价是最直接的证据：**排序在 LIMIT 之前**，所以"只取最新一条"并不能省成本，
成本是 O(该任务的运行次数 × log)。`list_tasks` 的 N+1 把这份成本乘上任务数。

**复杂度**：`list_tasks` O(Tasks·log Tasks) + Σ_task O(runs_of_task)；
`list_task_runs` O(runs_of_task)；`get_task` O(1) + O(runs_of_task)；`list_task_revisions` O(revisions)；
`list_task_deps` O(1)。**全部与会话/entries/threads 无关**，只跟任务与运行次数有关。

---

## 4. 横切问题（跨命令）

1. **"分页"在桌面侧是内存切片**：`get_messages`（§3.3）、`get_session_entries` 的 offset 路径（§3.4.2）
   都是先把全部数据取到内存，再 `paginate_items*`（`packages/remote-server/src/wire_limits.rs:164-205`）。
   `prepare_backward_entries_page_with_cap`（`:54-116`）更极端：它在 **while 循环里反复
   `serde_json::to_vec(&entries)`** 只为判断是否超 512 KiB，每丢一个 exchange 就重新序列化一次整页。
2. **重复查询**：
   - `list_session_files` / `download_prepare`：`find_thread_by_agent_session` 已取到 thread 行，
     `workspace_path_for_thread`（`agent_bridge/session.rs:263-269`）又 `get_thread` 一次同一行（§3.10）。
   - `list_sessions`：`active_run_sessions` 与 `latest_run_infos` 是**两次扫 `runs`**（一次回答
     "哪些在跑"，一次回答"每个 thread 最新一次"），可以用一次窗口函数查询同时回答（§3.1）。
     但注意 `latest_run_infos` 本身已经是**单条窗口函数查询**（`host/src/store/runs.rs:297-320`，
     `ROW_NUMBER() OVER (PARTITION BY thread_id ORDER BY created_at DESC, id DESC)`），
     **不是"每 thread 一次查询"**；能合并的是那两次扫，不要把它当成 N+1 去"改成一条聚合"。
   - `get_state` 冷路径：`contains()` 查 `sessions` 存在性，随后 `switch_session_metadata` 再读同一行（§3.12）。
3. **视图的重复编译**：`entry_records` / `resolved_message_blocks` 是深层视图（5 个 CTE + 每行
   json 与相关子查询）。`agent/src/session/sqlite_store/transcript.rs:66-70` 的注释记录了测量：
   "compiling it costs ~25 ms — a per-session listing of a few hundred conversations therefore spent ten
   seconds in the SQLite parser alone"。目前用 `prepare_cached` 缓解（`reader.rs:264,319,354,423` 都是
   `prepare_cached`），但 `agent/src/session/tools.rs:39,96` 与 `tools_canonical.rs:122` 用的是
   `db.prepare`（**未缓存**），每次调用都重编译。
4. **历史投影重建**：见 §3.4.1 末段——`PROJECTION_VERSION` 一变，每个会话的第一次历史页读都要付
   O(E_session) 的 `DELETE` + 重建（`recovery.rs:59-92`）。稳态不触发，升级瞬间全体变慢。
5. **32 MiB 回复上限**：`packages/rpc/src/transport.rs:53`。最大会话的 `get_messages` 已到 27.6 MB，
   没有分页预算兜底（对比：`get_events_since` 有 `EVENTS_PAGE_BYTE_BUDGET`、`get_session_entries`
   有 `BACKWARD_HISTORY_PAGE_BYTES`、`get_run_snapshot` 有 `run_snapshot_too_large`）。
6. **读准入的并发度**：`agent/src/grpc/read_admission.rs:32-52`：`HEAVY` 道 2 个许可
   （`list_sessions`、`get_messages` 等），`PAGES` 道 4 个，等待队列 32。一条重命令（§3.3）会独占
   `HEAVY` 的许可，使同一道里的其它命令排队——这是"某条命令慢导致另一条也慢"的机制来源。
7. **lean 后处理**：`packages/remote-server/src/lean.rs:427-457, 506-…` 在桌面侧对每个事件/entry
   的 `data` 字符串再解析一次并重写。它降低过网体积，但把成本从传输搬到了命令的 CPU 上，
   且与页内条目数线性。
8. **Fusion 库重复开连接**：`orchestration/fusion/src/store.rs:492-500` 一条 `get_fusion` 里
   `discussion` → `get`/`steps`/`events` 各自 connect、各自 prepare（无语句缓存）。

---

## 5. 表 A：哪些读**不是** O(1)（按严重度排序）

排序依据：先看是否随"用户会增长的数据"线性（entries / runs / threads），再看绝对量级，再看并发放大。

| # | 命令 | 读的复杂度 | 随哪个基数 | 实测/推演量级 | 证据 |
|---:|---|---|---|---|---|
| 1 | `get_messages` | O(E_session)，整段物化 + 整段过网 + ≥4 次编解码 | 该会话 entries | 14,531 entries → 228 次库作业 / 52.7 ms 纯 SQL / 27.6 MB JSON；上限 32 MiB | `business/history.rs:65,92`；`queries.rs:314`；`observability.rs:134`；`transcript.rs:29-57`；`reader.rs:319,354,423`；`transport.rs:53` |
| 2 | `get_session_entries`（无 `before` 的 offset 路径） | O(E_session) + O(E/250) 次 gRPC + 内存切片 | 该会话 entries | 最大会话 **59 次 RPC**，再本地切 100 条 | `history.rs:94-165`；`queries.rs:466-487`；`wire_limits.rs:164` |
| 3 | `list_sessions` | O(T+R)，全表扫 + 表达式排序；锁内 3 次全量序列化 | threads / runs | 682 线程 ≈ 6 ms SQL；20,000 线程 21 ms + 43 ms（两部分） | `catalog.rs:145-172,181-217`；`threads.rs:48`；`runs.rs:157,305`；`app-workspaces/lib.rs:160` |
| 4 | `list_available_skills` | O(网络)，最坏 60 s；随后 O(目录项) 次点查 | 网络 / 目录技能数 | 不可用数据规模描述；连接超时 10 s、总超时 60 s | `manager.rs:92-103,130-144,146-167` |
| 5 | `list_tasks` | O(Tasks log Tasks) + Σ O(runs_of_task)（N+1） | 任务数 / 每任务运行数 | 11 任务 0.010 ms；单任务 356 次运行每行 0.117 ms（×11） | `tasks.rs:324-336,60-80`；`store.rs:466,591,599` |
| 6 | `list_task_runs` / `get_task` 的 latest-run 部分 | O(runs_of_task)，`LIMIT` 不减成本 | 该任务运行次数 | 20,000 次运行 3.29 ms；`LIMIT 1` == `LIMIT 20` | `store.rs:79-80,591,599` |
| 7 | `download_prepare`（老客户端，无 `name`） | O(E_session) | 该会话 entries | 每张内联图片一次整会话拉取 | `files.rs:545-583,570`；`queries.rs:466` |
| 8 | `get_session_entries`（`before` 路径） | O(page) 但每行 2 个相关子查询 | 页行数（固定 150/600）× 该 run 的 entries | 600 行 5.9 ms；150 行 2.4 ms | `paging.rs:226-236,252,258-330` |
| 9 | `get_events_since`（非 chunked）/ `get_run_snapshot` 未命中缓存 | O(run 事件数) | 该 run 的事件数 | 10,855 事件 4.9 ms 纯 SQL（+ 全量 JSON 过网） | `queries.rs:130-190,193`；`journal_compaction.rs:365-435`；`replay.rs:415-432` |
| 10 | `list_workspaces` | O(W log W) + 每行相关子查询 | 工作区数 | 703 行 0.88 ms（≈1.2 µs/行） | `catalog.rs:317`；`app-workspaces/lib.rs:143-167` |
| 11 | `list_skills` | O(skills) + O(skills) 次文件读 + 目录 stat 遍历 | 已安装技能数 | 未在大样本上测（本机技能数小） | `manager.rs:170-215,599-650` |
| 12 | `get_fusion`（带 `step_id`） | O(该 step 会话 entries) | 参与方会话长度 | 未测（无 step 的融合记录） | `fusion.rs:427-455` |
| 13 | `list_session_files` | 数据库 O(1)；文件系统 O(目录项 × 路径深度) | 目录项数（非账户数据） | 未测 | `session_files.rs:52-108` |

**表 A-2：同一条表按端到端实测（§1.1）重排，并标出今天谁在发它**

复杂度排序回答“以后会变多慢”，这张回答“今天谁真的慢”。两列必须一起读：

| # | 命令 | 端到端 p50 | 今天谁在发（摘自 01） | 结论 |
|---:|---|---:|---|---|
| 1 | `get_session_entries`（offset 路径） | **619 ms** | **无人**（手机/桌面都带 `before`，01 §2.1） | 潜伏的灾难；一旦被调用就破 100 ms 预算 6 倍 |
| 2 | `get_messages` | **235 ms** | **无人**（01 §1.2） | 同上，且回复无字节预算（§4.5） |
| 3 | `list_available_skills` | **99 ms**（max 121） | 手机 `/` 菜单、技能页（01 §2.3） | **用户可触及**，且是纯网络——已接近 100 ms 预算 |
| 4 | `list_models`（三别名同价） | **21 ms** | 重连恢复、设置页、可见性变更（01 §2.3） | 用户可触及；常数偏大（§3.14） |
| 5 | `list_sessions` | **20.6 ms** | 手机 10 min + 版本前进；**桌面每 5 s × 每台主机**（01 §4） | 用户可触及，且是高频重复 |
| 6 | `get_state` | **22 ms**（最大会话）/ 4.2 ms（中位） | 打开会话时与历史页**并发**发出（01 §2.1） | 关键路径；22 ms 已归因：两条 legacy 专属全量读（68% / 30%），与会话长度线性（§3.12） |
| 7 | `get_session_entries`（`before`） | **11.6-12.3 ms** | 手机/桌面的主路径 | 关键路径，形状健康 |
| 8 | `list_workspaces` | 3.4 ms | 同 `list_sessions` | 不急 |
| 9 | `list_tasks` / `get_task` / `list_task_runs` | 2.8 / 1.4 / 1.7 ms | 任务页 | 绝对值小，但增长律是坏的（§3.17） |
| 10 | `get_events_since`（分页 / 快照） | 2.1 / 1.7 ms | 有活跃 run 时（01 §2.1） | 健康 |
| — | `get_tool_call_args` / `get_event_content_range` | 0.02 ms | 点开一行 | 完全健康 |

这张表把结语拉回实际：**今天最慢的用户可触及读是 `list_available_skills` 99 ms 与 `get_state` 22 ms、
`list_sessions` 20.6 ms**；而两条 200-600 ms 级的读（offset 分页、`get_messages`）当前没有发送者。
所以“最坏情况能否 <100 ms”这个目标，今天主要卡在**一次同步网络请求**和**两个 20 ms 级常数**上，
而不是卡在随数据量增长的那几条读上——后者是下一个数量级（会话更长、任务更多）才会暴露的。

表 A 第 9 行还有一个**环境相关**的事实（§7.2）：本机生产库缺 `run_snapshot_cache` 表，所以在生产
agent 上 `get_run_snapshot` 直接失败而不是变慢；§1.1 里的 1.7 ms 来自 T1 的隔离 HOME（当前构建、
缓存命中）。两者是同一批数据，差别只在“谁打开过这个库”。

**是 O(1) 的（作为对照）**：`get_tool_call_args`（6 次点查）、`get_event_content_range`（点查 + 范围）、
`get_read_chunk`（内存切片）、`get_desktop_settings` / `get_settings`（12 次点查）、`list_task_deps`（常量）、
`list_task_revisions`（索引直出顺序）、`list_providers`（只读 JSON 文件）、
`get_session_entries` 的 `before` 路径（keyset，页内成本固定）。`get_state` 的温路径是 **O(会话条目)**
（仅 legacy；两处全量读，§3.12），`list_models` 属于“不随账户数据长、但随目录模型数线性”（§3.14），
所以两者都不在这份 O(1) 名单里。

---

## 6. 表 B：哪条读随哪个数据维度增长、大致如何增长

"粗略如何"一列给出实测点与拟合斜率（同机、温页缓存、纯 SQL）。

| 数据维度 | 受影响命令 | 增长形状 | 实测点 / 斜率 |
|---|---|---|---|
| **单会话 entries** | `get_messages`（全部路径）、`get_session_entries`（offset 路径）、`download_prepare`（无 name）、`get_fusion`（带 step） | 线性 O(E)；`get_messages` 还要 + O(E/64) 次库作业 + O(E) 编解码 | 14,531 entries → 228 页 / 52.7 ms SQL / 27.6 MB；≈3.6 µs·entry⁻¹（SQL 部分） |
| **页内行数（固定上限）** | `get_session_entries`（`before`）、`get_events_since`（分页） | 线性于页行数，与会话总长无关 | 150 行 2.4 ms；300 行 3.5 ms；600 行 5.9 ms（≈10 µs/行）；事件首页 129 行 0.08 ms |
| **单 run 的事件数** | `get_events_since`（非 chunked）、`get_run_snapshot`（未命中） | 线性 O(events) | 10,855 事件 4.9 ms（≈0.45 µs/事件）；水位点查 0.003 ms |
| **threads 数** | `list_sessions`（含 `list_threads` + `latest_run_infos`） | 线性 O(T)，两条 SQL 各自线性 | 682→0.70 ms；2,000→1.92；5,000→5.20；20,000→21.09（list_threads，≈1.05 µs/行）；latest_run_infos 1.28→42.82（≈2.1 µs/行） |
| **workspaces 数** | `list_workspaces`、`list_sessions`（取 id→name 映射那一遍） | 线性 + 排序 O(W log W) | 703 行 0.88 ms（≈1.2 µs/行） |
| **runs 表行数** | `list_sessions`（`active_run_sessions` 全表过滤；`latest_run_infos` 按 thread 汇总） | `active_run_sessions` 线性于 runs；`latest_run_infos` 是单条窗口函数查询，排序行数随 threads 线性（**不是每 thread 一次查询**） | 1,970 runs → 0.61 ms（active）；682 threads → 3.00 ms（latest） |
| **单任务运行次数** | `list_task_runs`、`get_task`、`list_tasks`（每行一次） | 线性 + 排序 O(R log R)，**LIMIT 无效** | 20→0.022 ms；356→0.095；1,000→0.238；5,000→0.991；20,000→3.286（≈0.16 µs/run）；`LIMIT 1` 与 `LIMIT 20` 同价 |
| **任务数** | `list_tasks` | 线性 + 排序 + 每任务一次 latest-run | 11 任务 0.010 ms SQL（N+1 部分另计） |
| **已安装技能数** | `list_skills`、`list_available_skills` | 线性 + 每技能一次文件读 | 未测（样本太小） |
| **流变更数** | `get_events_since`（分页） | **不增长**（keyset，页内 128 条） | 19,340 条变更的首页 0.08 ms |
| **会话数（709）** | 无命令随它线性增长 | — | `list_sessions` 读的是 threads，不是 sessions；`contains` 是点查 |
| **库文件体积（2.48 GB）** | 无命令随它线性增长 | — | 所有大表读取都是按 `session_id`/`stream_id` 前缀定位的索引范围 |

一句话概括这张表的对立面：**"库多大"不影响任何命令；"这个会话/这个 run/这个任务有多长"决定
`get_messages`、`get_session_entries`(offset)、`get_events_since`(非分页) 的延迟；
"有多少会话/工作区/任务"只决定 `list_sessions`、`list_workspaces`、`list_tasks` 的延迟。**

---

## 7. 未测部分与不确定性（供后续工作接手）

1. **端到端毫秒数不是本测的**：§1.1 的端到端数字全部引自 01 的取样（
   `docs/testing/remote-latency/raw/*.json`），含传输、反序列化与 handler 的全部开销；
   本文其余所有数字都是"同一条 SQL 在只读副本上"的耗时，**只含 SQL 执行**。
   两者对账时注意差异来源：`get_messages` 的 52.7 ms SQL 对应 235 ms 端到端；
   `get_session_entries:first_paint` 的 2.4 ms SQL 对应 11.6 ms 端到端——**每条读都有 9-180 ms
   的"库外税"**，它不在本文的测量口径里，只能引用。
2. **两个环境不同步（最重要的口径限制）**：本文有**两套数据源**，必须分开读——
   - **生产库**（`~/.future/*`，本文所有 SQL 数字与索引存在性的事实来源）：缺 `run_snapshot_cache` 表
     与 `run_events_replay_order` 索引，说明**持有它的 agent 进程**早于这两处改动。在这份库上，
     `cached_run_snapshot` 会因 `no such table` 直接失败（§3.5.3）。
   - **T1 的隔离 HOME**（`target/remote-latency/remote-latency-619xoqsh`，§1.1 端到端数字的来源）：
     由当前构建打开过，两项都在（`run_snapshot_cache` 已有 1 行折叠，`run_events_replay_order` 已建）。
   两者都是同一批数据的副本，所以复杂度和量级可以直接对比，但**任何涉及这两张对象的行为，
   必须说明是在哪个环境里产生的**。要把这条拉齐，需要让生产 agent 用当前构建重启一次
   （`run_events_replay_order` 会在开库时建一次索引，大库上这本身耗时）。
3. **页内两个相关子查询的真实代价**（§3.4.1）只在最大会话的相邻 600 行上量过（5.9 ms）。
   若存在"单 run 上千条 entry"的会话，斜率会变大；本机数据里单 run 最大 578 条 entry，
   不足以定这条曲线。
4. **`PROJECTION_VERSION` 提升后的重建成本**（§3.4.1）没有实测（当前所有会话都是版本 2）。
   重建是写路径（`DELETE` + 分批插入），在只读副本上无法复现。
5. **`list_skills` / `list_available_skills` 的规模曲线**没有测：本机技能数不足以看出斜率，
   而 `list_available_skills` 的主要成本在网络，测它需要一次真实平台请求（本次未发起）。
6. **合成的规模曲线**（§3.1、§3.17）用的是与真实表等价 schema 的临时表（记录在
   `/tmp/perfaudit/scale_catalog.py` 与 `scale_tasks.py`，测完即 DROP）。它们证明的是 SQL 形状的
   线性度，不是真实数据的分布——真实数据只有 682/703/554 这一档。这些脚本是临时工具，没有入库；
   需要重现时按 §8 的思路重写即可。
7. 本文**不提出修复方案**。上文标注的"健康形状"（keyset 分页、点查、批量正文读取、`prepare_cached`）
   仅用于区分"已解决"与"未解决"，不作为改造建议。

---

## 8. 复现方式

```bash
# 只读副本（不触碰正在运行的 agent，不写真实库）
sqlite3 "file:$HOME/.future/agent/agent.db?mode=ro" ".backup /tmp/perfaudit/agent.db"
sqlite3 "file:$HOME/.future/app/app.db?mode=ro"     ".backup /tmp/perfaudit/app.db"
sqlite3 "file:$HOME/.future/tasks/tasks.db?mode=ro" ".backup /tmp/perfaudit/tasks.db"
```

本次用的计时脚本只存在于本次运行的 `/tmp/perfaudit/`（`measure_page.py` / `measure_app.py` /
`measure_events.py` / `measure_streams.py` / `measure_load.py` / `scale_catalog.py` /
`scale_tasks.py` / `check_tables.py`），它们是临时工具、不是仓库产物——换一台机器就没了。
所以把三个头号数字的**最小复现**写在这里，不依赖那些脚本：

```bash
# (1) 整会话读 = O(E)：228 页 × 2-4 条语句；换 S 为任意 session
S=20261016-172323-78e08a2913da44819765aeed0d924a56
sqlite3 agent.db "SELECT count(*) FROM entries WHERE session_id='$S';"                       -- 14531
sqlite3 agent.db "SELECT sum(length(CAST(metadata_json AS BLOB))+coalesce(length(CAST(content_json AS BLOB)),0))
                    FROM entries WHERE session_id='$S';"                                     -- ~10.5 MB
sqlite3 agent.db "SELECT sum(coalesce(length(CAST(text AS BLOB)),0)+coalesce(length(CAST(arguments_json AS BLOB)),0))
                    FROM message_blocks WHERE session_id='$S';"                               -- ~17.8 MB

# (2) 历史页是 keyset（不随会话增长）：把 LIMIT 换成 150/600 两次，耗时只随页行数变
sqlite3 agent.db "EXPLAIN QUERY PLAN SELECT d.ordinal FROM history_display d
                    WHERE d.session_id='$S' AND d.ordinal>=1000 AND d.ordinal<1150 ORDER BY d.ordinal DESC;"

# (3) LIMIT 不省成本：任务运行表没有 (task_id,started_at) 索引
sqlite3 tasks.db "EXPLAIN QUERY PLAN SELECT id FROM task_runs
                    WHERE task_id='tsk_ca72453d023540d49b4c407b530c72d6' ORDER BY started_at DESC LIMIT 20;"
```

每条 SQL 的索引判定用 `EXPLAIN QUERY PLAN <同一语句>` 复核；§3 中凡写"计划为…"的地方都可这样重放。
§3.1 与 §3.17 的规模曲线需要构造行数（本次用等价 schema 的临时表），无法用 `EXPLAIN` 代替——
它们只证明"线性"，不证明真实分布。

§1.1 的端到端取样不在本文重跑：它由 T1 的探针（`host/src/remote_host/latency_probe.rs`、
`scripts/perf/remote-commands/`）在隔离 HOME（`target/remote-latency/remote-latency-619xoqsh`）上生成，
原始 JSON 与 agent 日志同目录。本文只读它们，不重新启动 agent（避免扰动正在运行的服务）。
