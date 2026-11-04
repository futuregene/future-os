# 远端桌面命令打开延迟：真实数据测量

本章回答一个问题：**每一台手机的每个读命令，在这台机器真实的 2.5 GB agent.db 与 682 条会话之上，打开要多久。**

测量的不是 fixture，也不是合成数据：三份线上数据库用 SQLite 在线备份（含 WAL）复制到一个一次性的 home 里，所有 id 由这份快照的 SQL 现查出来（条目最多的会话、条目中位数会话、事件最多的 run、运行次数最多的任务……）。原始样本在 `raw/`，本文件的表格由 `scripts/perf/remote-commands/summarize-remote-latency.py` 重新生成。

## 1. 结论

**系列位置**：本章是这项审计的第三篇。`01-client-surface.md` 回答「谁在什么时候、用什么参数、发了什么」，`02-code-audit.md` 给出代码级的 O(1)/O(n) 判定与每个基数的来源，本章给出**真机数据上的最终数值**。02 里引用的计时来自同一套探针的早期一轮（采样少、未分冷热），本章的 release 构建 + 10 次采样 + cold/warm 分离是它的定稿；两者在量级上一致（如 14,531 条目会话的 offset 页：02 记 619 ms，本章 p50 698 ms），差异来自采样数与构建。

本轮覆盖了路由表里客户端会发的 **27 个读命令**，按不同数据规模展开成 **41 个组合**。其中 **7 个组合超 100 ms**，落在 **4 个命令**上：

| 命令 | 实测 | 超出原因（已读源码确认） |
| --- | --- | --- |
| `get_session_entries`（`offset` 分页，14,531 条目会话） | **p95 721 ms** | Agent 的 `read_history_page(before=None, offset=n)` 从会话开头走到第 n 条：代价 ∝ offset |
| `get_session_entries`（`offset` 分页，3,145 条目会话） | **p95 162 ms** | 同上，offset=1,572 |
| `get_messages`（14,531 条目会话，首页） | **p95 256 ms**（冷 562 ms） | 先取**整段**历史再在内存里切页：代价 ∝ 会话总条目，与该页在哪无关 |
| `get_messages`（同一会话，offset=7,265 深页） | **p95 239 ms** | 同上：切的是同一份全文，深页并不更快 |
| `get_messages`（3,145 条目会话） | **cold 177 ms**，热 p95 58 ms | 同上；规模减半后热态贴身通过，冷开仍超 |
| `get_state`（14,531 条目会话，进程内首次调用） | **cold 166 ms** | 首次打开该会话的存储（中位数会话 58 ms，小会话 55 ms）；热调用 24 ms |
| `list_available_skills` | **p95 120 ms** | Agent 每次调用都同步 HTTP GET 平台 skill 目录（`agent/src/skills/manager.rs:130`） |

其余组合全部在 25 ms 以内，其中游标式分页（`get_session_entries` 带 `before`）、事件回放（`get_events_since`）、工具参数、任务面板都在 12 ms 以内 —— **已经符合目标的路径不要动，它是本审计要保留的样板**。

## 2. 机器与快照

| 项 | 值 |
| --- | --- |
| 机器 | Apple M3 Ultra，32 核，103 GB 内存，macOS 26.6.2（Darwin 25.6.0，arm64） |
| 构建 | `--release`（bundled SQLite 在 release 下编译；debug 会把 SQLite 语句编译期成本放大到失去意义） |
| Agent | 本 worktree 构建的 `future-agent 0.0.0-5a70ff5c8+local.dirty`，独立 `HOME`，`--grpc-addr 127.0.0.1:<随机空闲端口>` |
| agent.db | 2,507,264,000 B 快照 · 227,939 条目 · 200,693 事件 · 2,362 run · 355 个有内容的会话 · 162,762 个内容对象 |
| app.db | 54,206,464 B 快照 · 682 线程 |
| tasks.db | 4,059,136 B 快照 · 33 个任务 · 554 条运行记录 |
| 采样 | cold = 每个命令在进程内第 1 次调用（独立进程，Agent 刚启动）；warm = 2 次预热 + 10 次测量，取 p50/p95/max |

## 3. 测量口径

- **`handler_ms`** 是 `future_host::remote_host::business::execute` 一次调用的墙钟时间。这一段就是"桌面端为打开这个会话做的事"：app.db 读取、到 Agent 的 gRPC 往返（agent.db 真正被读的地方）、页面裁剪、回复 JSON 构造。**不含**回复的发布/刷出（那是传输），也不含手机端渲染。
- 每条命令在测量时都装在 `content_capability::in_request(true)` + `lean::in_request(true)` 两个作用域里，和一台当前版本手机建立连接时完全一致 —— 否则 `get_tool_call_args` 根本到不了 handler，历史页也拿不到精简版。
- 回复字节数用生产编码器 `reply::encode_reply_payload_with_gzip(reply, true)` 现算，所以表里的字节就是手机真要收的字节（含 gzip），不是内存里的 JSON。
- `list_sessions` / `list_workspaces` 之间有 1 秒快照缓存，所以这两条的 warm 迭代间隔 1100 ms：否则测到的是缓存，不是读取。
- **cold** 的定义要说清：新进程 + 刚启动的 Agent，但**操作系统页缓存没有清理**（macOS 上清页缓存需要 `sudo purge`，探针不申请提权）。因此 cold−warm 的差值隔离的是"进程内/Agent 内的首次状态"（会话打开、索引重建、连接建立、观察者挂载），不是磁盘冷读；真正的磁盘冷读只会比这里的 cold 更慢，不会更快。

## 4. 数据规模锚点

这些 id 都是快照里现查出来的，表格里的"规模锚"就指它们。

| 锚 | 值 |
| --- | --- |
| largest（条目最多） | `20261016-172323-78e08a2913da44819765aeed0d924a56` · 14,531 条目 · 深页游标 7,265 |
| mid | `20261017-235455-e72faf18270d4dd58f8f2043a4898299` · 3,145 条目 |
| median | `20261022-140446-a9be8f40298545c688ea3e4529eea504` · 236 条目 |
| small | `20261020-162512-95336e77e4b749bab18e3dc9e6bd4f67` · 41 条目 |
| heaviest run（事件最多） | `20261019-094227-dbf629a2dd3346da90e09b8cd16683b8` / `run_26ce295be0a248a39e14f806375479f8` · 10,855 事件 · 最大工具参数 `call_00_6V9yT7mAakbPpHiU1OKO4170`（`edit`，2,909 B） |
| 最忙任务 | `tsk_ca72453d023540d49b4c407b530c72d6` · 356 次运行 · 36 个版本 |
| 最新 prompt receipt | `prompt_80520806d0d91ce0c5ce47e4a1c24109` |

## 5. 全量结果

cold 是冷相位那一次；p50/p95/max 是热相位 10 次采样。

| 命令 | 规模锚 | cold ms | p50 ms | p95 ms | max ms | 回复字节 | 判定 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `list_sessions` | 682 线程 | 22.4 | 21.2 | 24.1 | 24.1 | 70,213 | ok |
| `list_workspaces` | 682 线程 | 4.8 | 3.6 | 4.7 | 4.7 | 5,459 | ok |
| `get_state` | largest | **166.1** | 24.4 | 24.6 | 24.6 | 1,475 | **超预算** |
| `get_state` | median | 58.6 | 4.5 | 4.6 | 4.6 | 1,488 | ok |
| `get_state` | small | 55.6 | 4.3 | 4.7 | 4.7 | 1,475 | ok |
| `get_messages` 首页 | largest | **561.6** | **249.0** | **255.5** | 255.5 | 76,814 | **超预算** |
| `get_messages` 深页 | largest @7,265 | **240.6** | **234.6** | **238.9** | 238.9 | 35,340 | **超预算** |
| `get_messages` 首页 | mid | 177.0 | 57.1 | 57.6 | 57.6 | 58,597 | **超预算（冷）** |
| `get_messages` 首页 | median | 17.0 | 6.5 | 6.7 | 6.7 | 38,527 | ok |
| `get_messages` 首页 | small | 5.9 | 1.3 | 1.4 | 1.4 | 23,856 | ok |
| `get_session_entries` 首屏 | largest | 38.8 | 11.5 | 11.6 | 11.6 | 87,009 | ok |
| `get_session_entries` 深页 | largest @7,265 | 26.6 | 11.5 | 11.7 | 11.7 | 121,851 | ok |
| `get_session_entries` 首屏 | median | 14.3 | 6.1 | 6.2 | 6.2 | 28,605 | ok |
| `get_session_entries` **offset 页** | largest @7,265 | **589.3** | **698.4** | **720.8** | 720.8 | 14,510 | **超预算** |
| `get_session_entries` **offset 页** | mid @1,572 | **132.9** | **147.4** | **161.7** | 161.7 | 13,349 | **超预算** |
| `get_session_entries` offset 页 | median @118 | 14.0 | 5.7 | 5.8 | 5.8 | 21,722 | ok |
| `get_session_entries` 深页 | median @118 | 8.1 | 3.6 | 3.7 | 3.7 | 10,303 | ok |
| `get_events_since` 快照 | heaviest run | 45.1 | 2.0 | 2.2 | 2.2 | 31,264 | ok |
| `get_events_since` 首页 | heaviest run | 2.5 | 2.3 | 2.4 | 2.4 | 15,642 | ok |
| `get_tool_call_args` | heaviest run, 2,909 B 参数 | 1.0 | 0.6 | 0.7 | 0.7 | 3,027 | ok |
| `get_event_content_range` | 见 §9 | — | — | — | — | — | **未测出** |
| `get_events_since` 定位页 | heaviest run @9,047 | 2.8 | 2.3 | 2.3 | 2.3 | 12,933 | ok |
| `get_fusion` | heaviest run | 1.4 | 1.0 | 1.2 | 1.2 | 53 | ok（见 §7） |
| `list_session_files` | largest 的工作区根 | 1.4 | 1.1 | 1.2 | 1.2 | 4,410 | ok |
| `get_queue` | largest | 22.7 | 21.6 | 22.2 | 22.2 | 93 | ok |
| `get_prompt_receipt` | 最新 prompt | 0.1 | 0.0 | 0.0 | 0.0 | 179 | ok |
| `get_fusion_settings` | — | 1.8 | 1.4 | 1.5 | 1.5 | 400 | ok |
| `get_desktop_settings` | — | 0.1 | 0.0 | 0.0 | 0.0 | 3,013 | ok |
| `get_settings` | — | 0.0 | 0.0 | 0.0 | 0.0 | 102 | ok |
| `list_settings_models` | — | 22.1 | 21.0 | 21.4 | 21.4 | 2,032 | ok |
| `list_models` | — | 21.3 | 21.1 | 21.2 | 21.2 | 1,425 | ok |
| `get_available_models` | — | 21.4 | 21.1 | 27.6 | 27.6 | 1,425 | ok |
| `list_skills` | — | 2.2 | 1.6 | 1.8 | 1.8 | 12,633 | ok |
| `list_available_skills` | 平台目录 | 67.5 | **113.7** | **119.5** | 119.5 | 47,024 | **超预算** |
| `skill_reco_today` | — | 0.2 | 0.0 | 0.0 | 0.0 | 91 | ok |
| `list_providers` | — | 2.1 | 1.5 | 1.9 | 1.9 | 22,341 | ok |
| `list_tasks` | 33 任务 | 5.6 | 2.6 | 3.0 | 3.0 | 9,123 | ok |
| `get_task` | 356 次运行 | 2.1 | 1.4 | 1.5 | 1.5 | 24,813 | ok |
| `list_task_runs` | 356 次运行 | 2.4 | 1.7 | 1.7 | 1.7 | 11,708 | ok |
| `list_task_revisions` | 36 个版本 | 2.5 | 1.6 | 1.6 | 1.6 | 329,914 | ok |
| `list_task_deps` | — | 0.0 | 0.0 | 0.0 | 0.0 | 48 | ok |

## 6. 三条随数据增长的曲线

### 6.1 `get_messages`：代价 ∝ 会话总条目，与"要看哪一页"无关

| 会话条目 | 首页 p50 | 深页 p50 |
| --- | ---: | ---: |
| 41 | 1.3 ms | — |
| 236 | 6.5 ms | — |
| 3,145 | 57.1 ms | — |
| 14,531 | **249.0 ms** | **234.6 ms** |

约 0.017 ms/条目，且**深页不比首页便宜**（同一份全文切两次）。原因是 `host/src/remote_host/business/history.rs` 的 `get_messages` 分支：先 `agent_bridge::get_session_messages(session_id)` 取回**整个会话**，再 `paginate_messages(messages, offset, limit)` 在内存里切片 —— `offset`/`limit` 只决定切出来多少，不决定读多少。会话越长，一次"打开"越贵，这条曲线没有上界之外的拐点。

### 6.2 `get_session_entries`：游标分页 O(页)，offset 分页 O(offset)

同一份数据、同一个位置，两种分页方式差 **60 倍**：

| 会话条目 | 游标页（`before`）p50 | offset 页 p50 | offset 值 |
| --- | ---: | ---: | ---: |
| 236 | 3.6 ms | 5.7 ms | 118 |
| 3,145 | — | 147.4 ms | 1,572 |
| 14,531 | 11.5 ms | **698.4 ms** | 7,265 |

- 游标路径（`before`）落到 Agent 的 `read_history_page`，一次定位、一次有界读取：14,531 条目的会话里首屏 11.5 ms、回 87 KB，完全平。
- offset 路径（`offset`）同样进 `read_history_page`，但 `before=None`：要数到第 n 条才知道页从哪开始，所以 `offset` 翻倍，时间翻倍。14,531 条目会话的中段要 **0.70 s**，回复却只有 14 KB —— 时间全花在"数到那里"。

手机端用的是 `before` 游标（`mobile/src/remote/useTimelineController.ts` 传 `before`），所以这条 offset 曲线目前主要打的是老客户端与桌面侧接管路径；但它仍然是同一张路由表里发得出去、且会超时的命令。

### 6.3 `get_state`：冷开一格会话 55–166 ms，热后 4–24 ms

| 会话条目 | 进程内第 1 次 | 热 p50 |
| --- | ---: | ---: |
| 41 | 55.6 ms | 4.3 ms |
| 236 | 58.6 ms | 4.5 ms |
| 14,531 | **166.1 ms** | 24.4 ms |

`get_state` 是每次"打开一个会话"的第一个命令。冷相位的 55 ms 是与会话规模无关的固定开销（首个会话还要再叠加约 110 ms 的会话存储打开，见 14,531 条目那一行），热后掉到 4.5 ms —— 说明这部分成本是**首次触达的状态建立**，不是查询本身。即使热态，14,531 条目会话的 `get_state`（24.4 ms）仍比中位数会话（4.5 ms）高 5 倍，这一项也随会话增长。

## 7. 其它值得记下的数字

- **`list_available_skills` ≈ 100 ms 是网络，不是数据库。** `agent/src/skills/manager.rs:130` 的 `catalogue()` 每次调用都同步 `GET {platform_url}/client/v1/skills`，把整份目录（本次 47 KB）拉回来。这条命令的延迟由平台服务与网络决定，100 ms 预算在这里没有上界保证。
- **`get_queue` 花了两次 gRPC 往返。** `host/src/agent_bridge/queue.rs:17` 先 `verify_session_database`，再发一个 `get_state`，只为从状态里取出队列字段：21.6 ms 换 93 字节。同样的信息 `get_state` 一次就带回来了。
- **Agent 往返底噪约 21 ms**：`list_models` / `list_settings_models` / `get_available_models` 都是 21 ms 上下、回复 1–2 KB。这三个命令彼此等价（前两个是同一个 handler），都走 `get_available_models`，成本在 Agent 侧组装模型表，不在数据量。
- **`list_task_revisions` 回复 330 KB 却只要 1.6 ms**：`open_store` + 一次 `list_revisions`，说明 tasks.db 的读本身很便宜；真正要留意的是回复体积（330 KB 在弱网上不是 1.6 ms）。
- **`list_sessions` 21 ms / 70 KB**：682 线程全量快照，每次都真读（1100 ms 间隔绕过了 1 秒缓存），这是可接受的。

## 8. 给后续优化工作的清单

按收益排序，每条都对应上面的实测数字：

1. **`get_messages` 改成按页读**（14,531 条目会话 249 ms → 应与游标页同级，即 ~12 ms）。它今天把整段历史取回内存再切片；要么让 Agent 按 `offset/limit` 直接给页，要么让这条命令只服务老客户端并由 `get_session_entries` 接管当前客户端。改完复测 `get_messages` 四条规模锚应当落在 1–15 ms 且**不随会话条目增长**。
2. **`get_session_entries` 的 `offset` 分页**（698 ms → 应与游标页同级）。两条路：让 Agent 用 SQL `LIMIT/OFFSET` 或直接 seek 到第 n 条，而不是从头遍历；或者把 `offset` 从路由表里退役，只留 `before`。判据：14,531 条目、offset=7,265 必须 < 100 ms。
3. **`get_state` 的冷开**（166 ms → < 100 ms）。这是"打开会话"的必经第一步，也是当前唯一一条**手机真实路径**上超预算的命令。要解决的是首次触达会话存储的那 ~55–165 ms：预热、惰性索引，或把首屏需要的字段与索引建立拆开。
4. **`list_available_skills` 不要在网络路径上**（p95 120 ms，且无上界）。Agent 侧缓存目录（带 TTL），或让这条命令走本地缓存 + 后台刷新。
5. **`get_queue` 合并成一次往返**（21.6 ms → 与 `get_state` 同级或直接复用一次 `get_state` 的结果）。
6. **不要动已经达标的路径**：游标页（11.5 ms）、`get_events_since`（2–2.8 ms，含快照与定位页）、`get_tool_call_args`（0.7 ms）、任务面板（≤3 ms）、设置读取（≤1 ms）。这些是"平坦"的样板，优化工作若让它们变慢即为回退。

## 9. 未测出与不确定

- **`get_event_content_range` 未能测出。** 这条命令要求客户端拿着一个 `content_refs` 引用（Agent 只为**超过 16 KiB** 的内联上限的正文生成引用，见 `agent/src/session/content/transport.rs:15`）。本快照里已发布会话中最大的单条正文字段是 **14,077 B**（`tool_end`），低于该阈值，所以探针从真实页面上取不到任何引用（`raw/warm-20261104-run5.json` 里该条的 `parseError` 记录了这一点）。能测到的替代是定位页读取：`sinceIdx=9047` 的 `get_events_since` 为 2.3 ms。**它的真实成本仍未测量。**
- **`get_fusion` 只测到"没有融合运行"这一支。** 快照里 `fusion_runs` 为 0 条，任何真实 `(session, run, tool_call)` 都答 `found:false`（53 字节，1.2 ms）。有数据时的读取路径（`fusion_events` 分页）没有测到。
- **cold 不是磁盘冷读。** 未清理页缓存（需要 `sudo purge`，探针不提权）。cold−warm 的差值只隔离进程内/Agent 内首次状态；真实冷盘只会更慢。
- **快照是一个时点。** 测量期间用户的 agent 正在写线上库（agent.db 在 09:03 仍在增长），快照是备份那一刻的一致状态（227,939 条目 / 355 个有内容会话）；换一天数字会有几个百分点的漂移，但上面三条曲线的斜率不会。
- **单机、单次。** 一台 M3 Ultra、release 构建、每命令 10 次热采样。没有跨机器、跨构建配置的对照，p95 由 10 个样本的最近秩给出（即最大值），所以 p95 与 max 相同是采样数的结果，不是巧合。
- **未覆盖写命令与配对命令**（`prompt`、`abort`、`approval_decision`、`pair_*`、`upload_*`、`download_*`、`get_read_chunk`）：本轮任务是"打开"延迟，这些命令不会在打开一个会话或一个面板时发出。

## 10. 复现

```bash
# 1. 构建探针与 Agent（release；与本文数字同构建）
cargo test -p future-host --release --no-run
cargo build -p future-agent --release

# 2. 复制三份数据库（在线备份，源库以 mode=ro 打开）、
#    起一个独立端口的 Agent、跑 cold + warm 两个相位，写出 raw/*.json
python3 scripts/perf/remote-commands/measure-remote-latency.py \
    --iterations 10 --tag "$(date +%Y%m%d-%H%M%S)"

# 3. 从原始样本重新生成上面那张表
python3 scripts/perf/remote-commands/summarize-remote-latency.py --tag <tag>
```

脚本永不写线上数据库：源库以 `mode=ro` 打开并做 SQLite 在线备份，探针只在一次性 home 里运行（home 里有 `MEASUREMENT_ISOLATED_HOME` 标记，探针没有这个标记就拒绝启动），Agent 绑在随机空闲回环端口上（不碰 50051，不 `kill` 用户正在跑的 agent）。`--keep-home` 保留那份拷贝以便事后用 SQL 复核某个数字，`target/remote-latency/` 下另有每次运行的 Agent 日志与探针逐次输出。
