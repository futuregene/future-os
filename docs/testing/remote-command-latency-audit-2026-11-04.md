# 远端桌面命令打开延迟：审计与优化报告（2026-11-04）

> **这份报告要回答的问题**：手机 / 桌面客户端连接远端桌面时，真正会发出的每一条命令，用真实数据
> （本机 `agent.db` 2.5 GB / 713 个会话行、其中 355 个有内容，`app.db` 54 MB / 682 线程、`tasks.db` 4 MB）打开要多久；
> 哪些超过 100 ms；这些超过 100 ms 的命令**是不是因为在读数据库时做了非 O(1) 的操作**；
> 它们的延迟**会不会随数据规模一起上涨**。最后给出一份可以直接开工的优化清单，
> 目标是**最差情况下每条命令都在 100 ms 以内打开，并且延迟不随数据量增长**。
>
> **配套章节**（同一目录，报告的数字都能从那里回溯）：
> `01-client-surface.md` 客户端真正发出什么命令、什么参数、什么时序 ·
> `02-code-audit.md` 每条命令的 SQL / 索引 / 复杂度 ·
> `03-measurements.md` 真实数据的冷热实测 ·
> `04-scaling.md` 逐维度放大数据后的斜率 ·
> `05-verification.md` 独立复核（对前四章与本报告的抽查与纠正）
>
> **一句话结论**：当前数据下 **7 个组合超过 100 ms、落在 4 条命令上**，其中只有一条在手机的真实打开
> 路径上（`get_state` 冷开 166 ms）；但把数据放大之后，**会越线的是 7 条命令**，最严重的一条
> （`get_messages`）在约 19,000 条之后不是变慢，而是**直接失败**。两条真正的病根是
> **把"整段历史 / 整个会话"当成一次读取的粒度**（offset 分页 698 ms→2.3 s、`get_messages` 249 ms→失败）
> 和**一个本该 O(1) 的元数据读退化成整会话加载**（`get_state` 每次调用都载入整个 legacy 会话，
> 22 ms 随会话线性到 72 ms）。
>
> **如果只做三件事**：①把 `get_state` 的元数据读改回真正的 O(1)（P0-1，最安全）；
> ②退役 `get_session_entries` 的 offset 路径与 `get_messages`（P0-2/P0-3，它们今天已经超预算，
> 放大后失效）；③给 `list_session_files` 加分页（P1-4，一个 `node_modules` 就能把这条命令推到 229 ms）。
> 三件做完，剩下五条全部落在 100 ms 以内且斜率≈0。

---

## 0 先看结论

### 0.1 超 100 ms 的全部组合（实测，M3 Ultra / release 构建 / 真实快照）

| 命令 | 数据规模 | cold | p50 | p95 | 在谁的打开路径上 | 根因 |
| --- | --- | ---: | ---: | ---: | --- | --- |
| `get_session_entries`（`offset` 分页） | 会话 14,531 条目，offset=7,265 | 589 ms | **698 ms** | **721 ms** | 老客户端 / 桌面接管路径 | Agent 侧 `before=None` 时从头数到第 n 条：代价 ∝ offset |
| `get_session_entries`（`offset` 分页） | 会话 3,145 条目，offset=1,572 | 133 ms | **147 ms** | **162 ms** | 同上 | 同上 |
| `get_messages`（首页） | 会话 14,531 条目 | **562 ms** | **249 ms** | **256 ms** | 仅老客户端（当前手机不发） | 先取回**整段**历史再在内存切片：代价 ∝ 会话总条目 |
| `get_messages`（深页 offset=7,265） | 会话 14,531 条目 | **241 ms** | **235 ms** | **239 ms** | 同上 | 同上（深页不比首页便宜） |
| `get_messages`（首页） | 会话 3,145 条目 | **177 ms** | 57 ms | 58 ms | 同上 | 同上 |
| `get_state` | 会话 14,531 条目（**进程内首次**） | **166 ms** | 24 ms | 25 ms | **手机开会话的第一个命令** | 两个独立成因：cold 是**首次 hydrate**（读全量转写成 `Session`，每会话一次）；温路径是**两处 legacy 专属的全量读**（详见 §3.1） |
| `list_available_skills` | 平台目录 47 KB | 68 ms | **114 ms** | **120 ms** | `/` 菜单、技能页 | Agent 每次调用都**同步 HTTP GET** 平台目录 |

其余 34 个组合全部 ≤ 25 ms；其中**必须保持不动**的平坦样板：游标历史页 11.5 ms、
事件回放 2.0–2.4 ms（含快照与定位页）、工具参数 0.6 ms、任务面板 ≤ 3 ms、设置读取 ≤ 0.1 ms。

### 0.1b 现在没超、但「数据变大就会超」的（这才是本次审计最该看的表）

当前数据只代表这一个用户的历史。把某一个维度放大、其余留在真实值（`04-scaling.md`）：

| 命令 | 随哪个维度 | 当前 | 放大后 | 首次越过 100 ms |
| --- | --- | ---: | ---: | --- |
| `get_session_entries`（offset 页） | 会话条目 | 14,531→698 ms | 50,000→**2,264 ms** | **2,200 条**（实测） |
| `get_messages`（首页/深页） | 会话条目 | 14,531→249 ms | 25,000/50,000→**直接失败**（gRPC 回复 44/74 MB，撞 32 MiB 上限） | **5,600 条**（实测）；**18,885–22,636 条**之间撞上 32 MiB 上限，此后不再有「慢」这个状态，只有「错」 |
| `list_session_files` | 目录条目数 | 10→0.59 ms | 1,000→**45.6 ms**，5,000→**229 ms** | **2,200 个文件**（实测） |
| `get_events_since` + `preferSnapshot` | 单 run 事件数 | 10,855→2.0 ms | 25,000→**41.5 ms**，50,000→**81 ms** | 约 61,600 事件（外推）；超预算后仍先做一次整段投影再丢掉 |
| `get_state` | 会话条目 | 14,531→24 ms | 25,000→36.7，50,000→**72.1 ms**（重跑 64.5） | 约 71,100 条（外推） |
| `list_sessions` | 线程数 | 682→21 ms | 2,000→31 ms，5,000→**43–72 ms（双峰：目录快照命中/未命中）** | 约 7,100–11,000 线程（外推） |
| `list_tasks` | 任务数 | 33→2.6 ms | 500→15.9 ms | 约 3,400 个任务（外推） |

反向对照同样重要：**同一条 `get_session_entries` 的游标路径在 41→50,000 条之间一直是 1.4–15.8 ms**
（平坦），事件首页/深页、`get_tool_call_args`、任务面板在每一档都是平的。
所以「随数据上涨」不是必然，是**分页方式与读取粒度**的选择。

### 0.2 用户问的两个问题

**问题一：读数据库的时候是不是 O(1)？**

不是。按主键/唯一索引点查的命令确实是 O(1)（`get_tool_call_args`、`get_event_content_range`、
`get_read_chunk`、`get_desktop_settings`、`list_task_deps`…），但**有 5 条读路径不是**，而且它们
分属两种不同的错误：

| 类型 | 具体位置 | 复杂度 | 是否在关键路径 |
| --- | --- | --- | --- |
| **整段加载冒充元数据读**（两处，都只在 legacy 会话上触发） | ①`agent/src/session/manager/loading.rs:13` `state_metadata()` → `:68` `load_metadata()`：`SELECT payload FROM history_shapes WHERE session_id=? ORDER BY position` **全部行**读出并逐行 `serde_json::from_str`，只为数出「用户消息几条」和取一个 `parent_session_id`（占温路径超额 **68%**）。②`agent/src/session/manager/mod.rs:321` `unterminated_run_id()` → `transcript.rs:203` `read_run_marker_headers()` 的 legacy 分支走 `run_marker_rows(db, session, "entry_records")`，而 **`entry_records` 是一个视图**——SQLite 必须先把该会话的行摊出来再过滤（占 **30%**） | O(会话条目) | **是**（`get_state` 每次调用都走两条，且①的结果在已加载会话上根本没被用到） |
| **假分页（先全取再切）** | `host/src/remote_host/business/history.rs:65` `get_messages`：`get_session_messages()` 取**整段**再 `paginate_messages()` 内存切片；`:96-160` `get_session_entries` 的 offset 路径：`fetch_all_session_entries_with_client()` 以 250 条/页**串行拉完整段**（最大会话 59 次 gRPC）再丢掉 99% | O(会话条目) + O(条目/250) 次往返 | 否（当前手机走游标路径），但同一条命令发得出去 |
| **offset 分页** | `agent/src/session/history/paging.rs:215-219`：`before=None` 时 `start = offset`，SQL 仍然是 `ordinal BETWEEN start AND end`，要靠索引走到第 n 条 | O(offset) | 否（同上） |
| **全表扫描 + 窗口函数** | `host/src/store/threads.rs:49`（682 行 threads，表达式排序）、`host/src/store/runs.rs:305`（`ROW_NUMBER() OVER (PARTITION BY thread_id)`）、`packages/app-workspaces`（每行一个相关子查询） | O(threads + runs)，实测已随规模放大（见 02 表 B：20,000 线程 21 ms + 43 ms） | 是（桌面端每 60 s/台、手机 10 min） |
| **`LIMIT` 不省成本** | `tasks` 的 `task_runs` 上无 `(task_id, started_at)` 索引，`ORDER BY started_at DESC LIMIT 20` 先排序再截断（`LIMIT 1` 与 `LIMIT 20` 同价） | O(该任务运行次数) | 否（任务面板） |

一句话：**"库有多大"不影响任何命令，"这个会话/这个 run/这个任务有多长"决定了一批命令的延迟**。
大表读取基本都靠 `(session_id, …)` 前缀索引定位；出问题的不是索引缺失，而是**读取的粒度选错了**。

**问题二：延迟会不会随数据规模同步上涨？**

会，而且是**线性上涨、没有拐点**，已在三个维度上测出斜率：

| 维度 | 受影响命令 | 实测（真实数据） | 斜率 |
| --- | --- | --- | --- |
| 单会话条目数 | `get_messages` | 41→1.3 ms，236→6.5，3,145→57.1，14,531→**249** | ≈0.017 ms/条目，**深页与首页同价** |
| 单会话条目数 | `get_session_entries`（offset 路径） | 118→5.7 ms，1,572→**147**，7,265→**698** | ≈0.096 ms/offset 位 |
| 单会话条目数 | `get_state`（热） | 236→4.5 ms，14,531→24 ms | ≈1.4 µs/条目 |
| 线程数 | `list_sessions` | 682→21 ms；SQL 部分 682→0.70 ms，2,000→1.92，5,000→5.20，20,000→21.1 | ≈1.05 µs/线程（+ `latest_run_infos` ≈2.1 µs/线程） |
| 单任务运行数 | `list_task_runs` / `get_task` | 20→0.022 ms，356→0.095，5,000→0.99，20,000→3.29 | ≈0.16 µs/运行，`LIMIT` 无效 |

对照：**用游标（`before`）分页的同一条命令完全不涨**——14,531 条目会话首屏 11.5 ms，与 236 条目
会话的 6.1 ms 同量级。这说明"随规模上涨"不是必然的，**是分页方式的选择问题**，同一个 handler
里两种写法差 60 倍。

> 逐维度的放大扫描（线程 5,000、条目 50,000、run_events 50,000、task_runs 2,000 等）见
> `04-scaling.md`；本报告 §4 用的是它的结论。

---

### 0.3 这份报告被独立复核过

`05-verification.md` 由一个独立工作流重算了 03 的 40 行、04 的 85 行、四条越线交叉点与两个字节数
（全部原样复现），并用四个构建变体定位了 `get_state` 的归因（§4.3）。它同时**否证**了三处判断，
本报告已按复核结论订正：① `get_state` 的成因是**两处**整段读而不是一处（68%/30%），
只改一处不达标；② 多出来的时间在**命令体内部**，不在 `verify_session_database`/observer 的装配上；
③ `list_sessions` 的 `latest_run_infos` **本来就是单条窗口函数查询**，不是 N+1，
按"每线程一次查询"去改会改到已经正确的实现上；以及 5,000 线程的耗时是 **43–72 ms 双峰**
而不是"至少 71 ms"。凡本报告与前面各章冲突处，以本报告（已订正）与 `05-verification.md` 为准。

---

## 1 审计范围与方法

### 1.1 "所有接口"是哪些

三层，必须合起来读（详见 `01-client-surface.md` §1）：

| 层 | 数量 | 位置 | 权威性 |
| --- | ---: | --- | --- |
| 业务路由表 `route()` | **68 条**（其中 26 条是 `get_`/`list_` 读命令） | `host/src/remote_host/business/mod.rs:43-105` | **权威**：生产路径唯一分发入口，返回 `None` 即回 `Unsupported command` |
| 桥层命令（不进路由表） | **11 条**：`secure_open`/`secure_finish`、`secure_ready`、`client_presence`、`client_disconnect`、`get_presence`、`takeover`、`unpair`、`pair_handshake`、`pair_handshake_confirm`、`get_read_chunk` | `packages/remote-server/src/commands.rs`、`host/src/remote_host/read_pages.rs` | **权威**，且必须与路由表合并才是完整清单 |
| `PHONE_COMMANDS` 测试夹具 | 60 条 | `business/mod.rs:254-315` | **不是清单**，是测试镜像，已滞后于真实客户端 6 条 |
| `command.schema.json` | — | `packages/remote-protocol/` | **只管字段形状**，`type` 是自由字符串、无 enum，当命令清单用会漏掉全部 79 条 |

**结论**：仓库里**没有一个地方收拢了完整列表**。本次以 `route()` + 桥层命令为骨架，再用客户端源码
（`mobile/src/remote/`、`packages/remote-client/src/`、`desktop/src/features/remote-peer/`）逐条核对
实际发出的命令与实填参数，得到 26 条读命令 × 41 个规模组合的实测面。顺带发现两处**规格漂移**（不在
本报告的优化范围内，但会让后续工作踩空）：`PHONE_COMMANDS` 漏登记 6 条手机正在发的命令
（`get_queue`、`cancel_queued_run`、`release_queued_run`、`get_fusion`、`get_event_content_range`、
`rename_workspace`），其中 `get_queue` 是 2 s 轮询。

### 1.2 "打开一条会话"到底发几条命令

这是"打开延迟"的真正口径——用户等的是这一串，不是单条命令：

- **手机**（并发）：`get_state(sessionId)` ‖ `get_session_entries(before=MAX, limit=3, chunkedRead)`
  → 每个 >192 KB 的回复再发 `get_read_chunk`（并发 4）→ 尾页被裁剪时串行补 1–3 页
  → 有活跃 run 时 `get_events_since(sinceIdx=-1, limit=1000)` 逐页回放。
  **最少 3 个往返**。
- **桌面作为客户端**（串行）：`get_session_entries` → `get_state` →（有活跃 run）`get_events_since`。
  串行是桌面侧独有的缺陷，手机已经并发。

所以"每条命令 <100 ms"是必要条件而不是充分条件；§6 的验收判据按**打开序列的总和**给出。

### 1.3 测量方法（为什么这些数字可信）

- **真实数据**：三份线上库用 SQLite 在线备份（含 WAL）复制到一次性 HOME，源库以 `mode=ro` 打开；
  所有 id 由快照 SQL 现查（条目最多的会话、中位会话、事件最多的 run、运行最多的任务）。
  **不写线上库、不起/不杀用户的 agent、隔离 agent 绑随机空闲回环端口。**
- **真实 dispatcher**：探针（`host/src/remote_host/latency_probe.rs`，`#[ignore]` + `#[cfg(test)]`，
  生产路径零改动）把命令喂给 `business::execute` 本身，装在
  `content_capability::in_request(true)` + `lean::in_request(true)` 两个作用域里——与一台当前版本
  手机建连时完全一致，否则 `get_tool_call_args` 根本到不了 handler。
- **计时口径**：`handler_ms` = `execute()` 一次调用的墙钟：app.db 读 + 到 Agent 的 gRPC 往返
  （agent.db 真正被读的地方）+ 页裁剪 + 回复 JSON 构造。**不含**回复发布（传输）与手机渲染。
  回复字节数用生产编码器 `encode_reply_payload_with_gzip` 现算。
- **构建**：release（debug 下 bundled SQLite 的语句编译成本会被放大到失去意义）。
- **冷热分离**：cold = 新进程 + 刚启动 Agent 的第一次调用；warm = 2 次预热 + 10 次采样取 p50/p95。
  cold **未清页缓存**（需要 `sudo purge`），因此它隔离的是"进程内/Agent 内首次状态"，真实冷盘只会更慢。

---

## 2 完整实测表（41 个组合）

`ok` = 不超过 100 ms 预算；**粗体** = 超出。数据规模锚点见 `03-measurements.md` §4。

| 命令 | 规模锚 | cold | p50 | p95 | 回复字节 | 判定 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| `list_sessions` | 682 线程 | 22.4 | 21.2 | 24.1 | 70,213 | ok |
| `list_workspaces` | 682 线程 | 4.8 | 3.6 | 4.7 | 5,459 | ok |
| `get_state` | largest | **166.1** | 24.4 | 24.6 | 1,475 | **超预算（冷）** |
| `get_state` | median | 58.6 | 4.5 | 4.6 | 1,488 | ok |
| `get_state` | small | 55.6 | 4.3 | 4.7 | 1,475 | ok |
| `get_messages` 首页 | largest | **561.6** | **249.0** | **255.5** | 76,814 | **超预算** |
| `get_messages` 深页 | largest @7,265 | **240.6** | **234.6** | **238.9** | 35,340 | **超预算** |
| `get_messages` 首页 | mid | **177.0** | 57.1 | 57.6 | 58,597 | **超预算（冷）** |
| `get_messages` 首页 | median | 17.0 | 6.5 | 6.7 | 38,527 | ok |
| `get_messages` 首页 | small | 5.9 | 1.3 | 1.4 | 23,856 | ok |
| `get_session_entries` 首屏（游标） | largest | 38.8 | 11.5 | 11.6 | 87,009 | ok |
| `get_session_entries` 深页（游标） | largest @7,265 | 26.6 | 11.5 | 11.7 | 121,851 | ok |
| `get_session_entries` 首屏（游标） | median | 14.3 | 6.1 | 6.2 | 28,605 | ok |
| `get_session_entries` **offset 页** | largest @7,265 | **589.3** | **698.4** | **720.8** | 14,510 | **超预算** |
| `get_session_entries` **offset 页** | mid @1,572 | **132.9** | **147.4** | **161.7** | 13,349 | **超预算** |
| `get_session_entries` offset 页 | median @118 | 14.0 | 5.7 | 5.8 | 21,722 | ok |
| `get_session_entries` 深页（游标） | median @118 | 8.1 | 3.6 | 3.7 | 10,303 | ok |
| `get_events_since` 快照 | heaviest run | 45.1 | 2.0 | 2.2 | 31,264 | ok |
| `get_events_since` 首页 | heaviest run | 2.5 | 2.3 | 2.4 | 15,642 | ok |
| `get_events_since` 定位页 | heaviest @9,047 | 2.8 | 2.3 | 2.3 | 12,933 | ok |
| `get_tool_call_args` | heaviest run，2,909 B | 1.0 | 0.6 | 0.7 | 3,027 | ok |
| `get_event_content_range` | — | — | — | — | — | **未测出**（见 §7） |
| `get_fusion` | heaviest run | 1.4 | 1.0 | 1.2 | 53 | ok（仅空结果分支） |
| `list_session_files` | largest 工作区根 | 1.4 | 1.1 | 1.2 | 4,410 | ok |
| `get_queue` | largest | 22.7 | 21.6 | 22.2 | 93 | ok（但是两次往返，见 §6） |
| `get_prompt_receipt` | 最新 prompt | 0.1 | 0.0 | 0.0 | 179 | ok |
| `get_fusion_settings` | — | 1.8 | 1.4 | 1.5 | 400 | ok |
| `get_desktop_settings` | — | 0.1 | 0.0 | 0.0 | 3,013 | ok |
| `get_settings` | — | 0.0 | 0.0 | 0.0 | 102 | ok |
| `list_settings_models` | — | 22.1 | 21.0 | 21.4 | 2,032 | ok |
| `list_models` | — | 21.3 | 21.1 | 21.2 | 1,425 | ok |
| `get_available_models` | — | 21.4 | 21.1 | 27.6 | 1,425 | ok |
| `list_skills` | — | 2.2 | 1.6 | 1.8 | 12,633 | ok |
| `list_available_skills` | 平台目录 | 67.5 | **113.7** | **119.5** | 47,024 | **超预算** |
| `skill_reco_today` | — | 0.2 | 0.0 | 0.0 | 91 | ok |
| `list_providers` | — | 2.1 | 1.5 | 1.9 | 22,341 | ok |
| `list_tasks` | 33 任务 | 5.6 | 2.6 | 3.0 | 9,123 | ok |
| `get_task` | 356 次运行 | 2.1 | 1.4 | 1.5 | 24,813 | ok |
| `list_task_runs` | 356 次运行 | 2.4 | 1.7 | 1.7 | 11,708 | ok |
| `list_task_revisions` | 36 版本 | 2.5 | 1.6 | 1.6 | 329,914 | ok |
| `list_task_deps` | — | 0.0 | 0.0 | 0.0 | 48 | ok |

---

## 3 问题一：这些读是不是 O(1)？（按证据逐条）

### 3.1 最隐蔽的一条：`get_state` 每次调用都载入整个会话

`get_state` 是**手机每打开一个会话的第一个命令**，也是余额面板、compaction 等待（5 s 轮询）会重复
发的命令。它的 handler 主体（`agent/src/rpc/mod.rs:452-560`）看起来只读内存，但有一条前置调用：

```rust
// agent/src/rpc/mod.rs
let stored_metadata = sess.session_manager.state_metadata(&session_id).ok();   // ← 无条件
let query_count = if sess.history_loaded { /* 用内存里的计数 */ } else { /* 用 stored_metadata */ };
```

`state_metadata()`（`agent/src/session/manager/loading.rs:13`）先按 canonical 格式取两条便宜的查询，
**格式不是 2 就 fall through 到 `load_metadata()`（`:68`）**，后者执行：

```sql
SELECT payload FROM history_shapes WHERE session_id=?1 ORDER BY position   -- 全部行，不只是 header
```

再对**每一行**做 `serde_json::from_str`，拼出一个完整 `Session`，只为返回 `(user 消息数, parent_session_id)`。
本机最大会话（14,531 条目）实测：**该 SQL 6.0 ms + 4.5 MB JSON 解码**；canonical 路径的同名函数
**0.011 ms**（实测，同一连接 50 次取均值）。这解释了 `get_state` 热态 24 ms vs 4.5 ms 的 5 倍差，
也解释了冷态 166 ms 里超出固定 55 ms 的那一截。

更糟的是两个放大器：

- **结果在已加载会话上被直接丢弃**（`history_loaded` 为真时用的是内存计数），但查询照做不误；
- 本机 **713 个会话行里 392 个是 legacy 格式**（只有 321 个是 canonical，`session_formats.format=2`），
  也就是**过半会话每次都付这个成本**。

这一条是"读数据库时不是 O(1)"最典型的例子：**粒度错了**（为了一个计数读整个会话），
**时机错了**（无条件执行），**位置错了**（在最热的命令上）。

### 3.2 假分页：`get_messages` 与 `get_session_entries` 的 offset 路径

| 位置 | 写法 | 后果 |
| --- | --- | --- |
| `host/src/remote_host/business/history.rs:65` `get_messages` | `get_session_messages(session_id)` 取整段 → `paginate_messages(messages, offset, limit)` 内存切片 | 代价 ∝ 会话总条目；**深页不比首页便宜**（235 vs 249 ms，回复字节差一倍，耗时不变——这是"整段物化"的签名） |
| `history.rs:96-160` + `host/src/agent_bridge/queries.rs:466` `get_session_entries` offset 路径 | `fetch_all_session_entries_with_client()` 以 `PAGE_SIZE=250` **循环拉完整段**，再 `paginate_items_with_cap()` 切 100 条 | 最大会话 **59 次串行 gRPC**，只为返回 14.5 KB |
| `agent/src/session/history/paging.rs:215-219` | `before=None` 时 `start = offset.clamp(...)`，SQL 依然 `ordinal >= start AND ordinal < end` | 要走索引数到第 n 条，代价 ∝ offset |

对照同一 handler 的游标路径：`before=Some(MAX)` → `read_history_page` 一次定位、一次有界读取
（`paging.rs:209-213` 用 `history_users(session_id,is_user,ordinal)` 索引 seek 三行）→ 14,531 条目会话
首屏 **11.5 ms / 87 KB**。**同一份数据、同一个位置，两种分页差 60 倍**，这是本次审计最有说服力的对照。

另外 `prepare_backward_entries_page_with_cap`（`packages/remote-server/src/wire_limits.rs:48-110`）
在 while 循环里反复 `serde_json::to_vec(&entries)` 只为判断是否超 512 KiB，每丢一个 exchange 就重新
序列化整个页——CPU 随页行数线性，且与"丢多少"的平方成正比。

### 3.3 目录类读取：全表扫描 + 窗口函数 + 全局锁

`list_sessions`（桌面端**每台主机每 60 s**、手机 10 min，且每次缓存未命中都真读）一次要做：

1. `store::active_run_sessions()` — 扫 `runs` 回答"哪些在跑"；
2. `store::list_threads()` — 682 行 threads 全表 + 表达式排序（`host/src/store/threads.rs:49`）；
3. `store::list_workspaces()` — 703 行，每行一个相关子查询；
4. `store::latest_run_infos(&thread_ids)` — 682 个占位符 + `ROW_NUMBER() OVER (PARTITION BY thread_id)`，
   只为取每个线程**最后一行**（`host/src/store/runs.rs:305`）；
5. 两次全量 JSON 序列化求结构签名。

其中 1 与 4 是**两次扫 `runs`**，可以一次窗口函数同时回答。实测 SQL 部分 682 线程约 6 ms，
但合成放大后：`list_threads` 20,000 线程 21.1 ms、`latest_run_infos` **42.8 ms**——合计已过预算。
而且整段在**全局 `CATALOG` 互斥锁**内完成，一条慢的 `list_sessions` 会挡住所有并发请求。

### 3.4 看起来像点查、其实不是

- **`get_queue`**：`host/src/agent_bridge/queue.rs:17` 先 `verify_session_database()`（2 次 app.db 点查 +
  **一次额外 gRPC**），再发一个**完整的 `get_state`**，只为取队列字段——21.6 ms 换 93 字节，
  而 `get_state` 一次就把这些字段带回来了。它是 **2 s 轮询**。
- **`list_available_skills`**：`agent/src/skills/manager.rs:130` 每次调用都同步
  `GET {platform_url}/client/v1/skills`（连接超时 10 s、总超时 60 s）。p95 120 ms 全部是网络，
  **没有任何上界保证**，与数据规模无关——这类"与数据量无关、但可以任意慢"的读，100 ms 预算必须
  用缓存兜住。
- **`list_models` / `list_settings_models` / `get_available_models`**：21 ms 一条、回复只有 1.4 KB。
  Agent 侧对**整个内置模型目录**（~3.8k）逐个调用 `is_model_available("provider/id")`
  （`agent/src/rpc/commands/providers.rs:180-186`，文件自己的注释称其"by far the most expensive test here"），
  再排序去重。复杂度 O(目录模型数)，不随账户数据增长，但常数很大。
- **`list_session_files`**：数据库只 3 次点查（其中 1 次是重复的 `get_thread`），但**每个目录项**
  都要 `canonicalize()` + `ensure_path_allowed()` + `metadata()`（`host/src/remote_host/session_files.rs:56-105`）：
  O(目录项 × 路径深度) 次系统调用。本机的会话根只有 4.4 KB，但一个包含 `node_modules` 的工作区
  目录会让这条命令变成几十毫秒到几秒。
- **横切税：每次内容读都多一个往返**。`verify_session_database`（`host/src/agent_bridge/presence.rs:118`）
  被 `get_state`、`get_messages`、`get_session_entries`、`get_events_since`、`get_event_content_range`、
  `get_queue` 等**每条内容读**都要走一遍：2 次 app.db 点查 + 一次 `get_session_presence` gRPC。
  它是一条安全栅栏（防止拿错库的会话被改名/压缩），不该删，但**不该每条命令都重查**——
  每会话一次 + 失效回调就够。同样地，`get_state` 还额外调 `ensure_observer_for_session`
  （又一次 `find_thread_by_agent_session`）。三条串行往返叠在一起，把一条"全内存命令"的底噪抬到
  4–5 ms，Agent 往返底噪本身约 21 ms 一档（见 `list_models`）。

### 3.5 已经做对的（作为样板，别动）

- **游标分页**（`get_session_entries` 的 `before` 路径）：索引 seek 定位 + 有界读取，
  14,531 条目会话 11.5 ms，**与会话长度无关**。
- **事件回放**（`get_events_since`）：canonical 流用 `UNIQUE(stream_id, last_cursor)` 做 keyset，
  19,340 条变更的首页 0.08 ms；实测首页 2.3 ms、定位页 2.3 ms、快照 2.0 ms。
- **内容分段读**（`get_event_content_range`）：`content_segments(content_id, byte_offset)` 主键取范围，
  成本只与取回字节数有关。
- **工具参数**（`get_tool_call_args`）：6–7 次点查，0.6 ms。
- 桌面设置读取：12 次主键点查，0.04 ms。

---

## 4 问题二：延迟是否随数据规模上涨？

**会上涨，而且是线性、没有拐点。** 14 个隔离 home、每个只放大一个维度、每个档位冷热两相
（`04-scaling.md` §2）。逐维度的结论：

### 4.1 八条确定在涨的曲线

| 命令 | 维度 | 实测（p50） | 形状 | 斜率 |
| --- | --- | --- | --- | --- |
| `get_messages` 首页 | 会话条目 | 3,145→62.5 → 14,531→**251 ms**（25,000/50,000 直接失败） | 线性 | ≈+118 ms/10× |
| `get_messages` 深页 | 会话条目 | 14,531→**253 ms** | 线性 | ≈+120 ms/10× |
| `get_session_entries` offset 页 | 会话条目 | 3,145→**136** → 14,531→**682** → 50,000→**2,264 ms** | 线性 | ≈+592 ms/10× |
| `list_session_files` | 目录条目数 | 1,000→**45.6** → 5,000→**229 ms** | 线性（文件系统，非数据库） | ≈+92 ms/10× |
| `get_events_since` + `preferSnapshot` | 单 run 事件数 | 10,000→1.29 → 25,000→**41.5** → 50,000→**81 ms** | 10,000 处跳变后线性 | ≈+34 ms/10× |
| `get_state`（热） | 会话条目 | 3,145→7.8 → 14,531→23.2 → 50,000→**72.1 ms** | 线性 | ≈+17 ms/10× |
| `list_sessions` | 线程数 | 682→18.1 → 2,000→31.1 → 5,000→**70.8 ms** | 线性 | ≈+27 ms/10× |
| `list_tasks` | 任务数 | 33→2.3 → 500→**15.9 ms** | 线性 | ≈+5.9 ms/10× |

### 4.2 反向对照：同样是分页，游标路径一条都不涨

| 路径 | 41 条 | 1,000 | 3,145 | 14,531 | 25,000 | 50,000 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `get_session_entries` **游标页**（`before`） | 1.39 | 14.7 | 12.2 | 12.24 | 15.7 | 15.8 ms |
| `get_events_since` 首页 / 深页 | — | 1.3 | — | 2.06 | — | 2.15 ms |
| `get_tool_call_args` | 0.55 | 0.55 | — | 0.6 | — | 0.71 ms |

（游标页 1,000→3,145 的小幅回落来自"一页的行数与字节"，不是会话长度：它的成本 = 页内内容，
不是历史总长。）

### 4.3 `get_state` 为什么会随会话长度线性上涨（04 留空的那一格，已由复核用对照实验定位）

`04-scaling.md` §4.3 实测出 `get_state` 线性（≈1.4 µs/条目，50,000 条 72.1 ms）并写"归因未完成"；
`02-code-audit.md` §3.12 的代码判定是温路径 O(1)；`04` 自己又猜"多出来的时间在命令体之前
（`verify_session_database` 的额外 gRPC、`ensure_observer_for_session`、会话装配）"。
`05-verification.md` §3 用**四个构建变体**（baseline / A / B / A+B）在同一份数据上把这三方一次性判完：

| 变体 | Δ@41 | Δ@3,145 | Δ@14,531 | Δ@25,000 | Δ@50,000 | 斜率 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 0.24 | 4.42 | **17.61** | **29.76** | **59.80** | **1.19 µs/条目** |
| A：摘掉 `state_metadata` 的整段读 | −0.02 | 1.33 | 5.96 | 8.91 | **19.35** | 0.39 µs/条目 |
| B：摘掉 `read_run_marker_headers` 的视图扫描 | 0.00 | 2.62 | 11.88 | 20.88 | **41.92** | 0.84 µs/条目 |
| **A+B** | −0.22 | **−0.14** | **−0.12** | **0.06** | **0.05** | **0.01 µs/条目** |

（Δ = p50 减去同一次运行里 canonical 会话的地板值，用来消掉机器快慢的漂移。）

三条结论，都要写进优化任务的定义里：

1. **成因是两处，不是一处**，而且都在**命令体内部**（`agent/src/rpc/mod.rs` 的 `get_state_internal`），
   不是命令体之前的装配：
   - ① `state_metadata()` → `load_metadata()`：legacy 会话把 `history_shapes` 的**全部行**读出并逐行
     JSON 解码，只为数出「用户消息几条」和一个 `parent_session_id`（**占 68%**）；
   - ② `unterminated_run_id()` → `read_run_marker_headers()` 的 legacy 分支扫 **`entry_records` 视图**
     （视图不可索引，SQLite 必须把该会话的行先摊出来再过滤，**占 30%**）。
2. 两处都可加性地摘掉（合起来 58.33 ≈ baseline 59.80，无重叠），**A+B 之后 41 条与 50,000 条同值**
   （4.18~4.64 ms），即真正与会话长度无关。**只改 ① 会在 50,000 条留下 19 ms，不达标。**
3. **canonical 会话本身就是平的**（同一份数据里的 321 个真实 `format=2` 会话：1 条 3.58 ms →
   2,043 条 3.94 ms，跨 2,043 条只动 0.36 ms），同规模对照 legacy 2,159 条 **6.74 ms**。
   这就是"同一份数据、只差一个 format 标记"的因果对照。

**cold 与温路径是两条独立的优化项。** 冷态（进程内首次）里面主要是**首次 hydrate 该会话**
（`typed_entries` 读全量转写并构造 `Session`），A+B 只拿掉其中约 25%（50,000 条 173.7 → 127.8 ms）。
hydrate 在语义上就是 O(条目)、每会话只付一次，要压只能压 hydrate 本身（分片 hydrate，
或把首屏所需的最小状态单独读出来）。所以 P0-1 的判据只针对温路径，cold 的目标另列（§6.1）。

---

## 5 优化清单（按收益排序）

每条都写明：**现状 → 改法 → 预期 → 验收判据**。验收判据都是可以机械复测的数字，
复测方法见 §6.2。

### P0-1 `get_state` 的两处整段读改回真正的 O(1)（收益最大且最安全）

- **现状**：legacy 格式会话（本机 392/713）每次调用都做两处全量读，温路径 24 ms（最大会话）、
  50,000 条 72 ms，斜率 1.19 µs/条目。分成 **68% / 30%** 两半（复核 A/B 实验，§4.3）。
- **改法（两处都必须改，只改一处不达标）**：
  1. `state_metadata()` 的 legacy fallback：计数改走已有的 `history_display` 投影
     （`SELECT count(*) ... WHERE session_id=? AND is_user=1` 命中 `history_users(session_id,is_user,ordinal)`）
     或直接 `SELECT count(*) FROM entries WHERE session_id=? AND entry_type='user'`（实测 0.006–0.021 ms）；
     `parent_session_id` 直接读 `sessions.current_metadata_json`；并**把调用改成惰性**——
     只在 `!sess.history_loaded` 时才查，结果缓存在会话对象上。
  2. `read_run_marker_headers()` 的 legacy 分支不要再扫 `entry_records` **视图**
     （视图不可索引是这条成本的来源）：改成从 `entries`/`entry_header_records` 按
     `(session_id, entry_type, position)` 的索引定点读那三条 marker
     （`entries_kind(session_id, entry_type, position)` 已经覆盖）。
- **预期**：两处都改后与 canonical 同值 —— 复核 A+B 实测 **4.18–4.64 ms，41 条与 50,000 条同值**
  （Δ = −0.22 / 0.05 ms）。只改第 1 处会在 50,000 条留下 **19 ms**，不达标。
- **验收判据**（照抄复核给的）：legacy 会话 50,000 条 p50 ≤ 6 ms，**且** Δ(50k − 41) ≤ 0.5 ms；
  41 条目 legacy 与 2,043 条目 canonical 的 p50 差 ≤ 0.5 ms。
- **不在本条范围内**：cold（首次 hydrate）。它每会话付一次、且语义上就是 O(条目)，
  目标见 §6.1 第 7 条。

### P0-2 退役或重写 `get_session_entries` 的 offset 路径

- **现状**：offset=7,265 时 698 ms（p95 721），且要多发 59 次串行 gRPC。
- **改法**：二选一——（a）让 Agent 侧 `read_history_page` 在 `before=None` 时用 `LIMIT/OFFSET` 直接
  取页（SQLite 仍需数过前 n 行，但对 14k 行是毫秒级，可接受），并把桌面侧的
  `fetch_all_session_entries_with_client` 循环彻底删掉，只转发一页；（b）**推荐**：把 `offset`
  从路由表退役，老客户端改用它已有的 `before` 语义（`before=end` 即可表达"从第 n 条往前一页"），
  让线上只留一条分页实现。
- **预期**：698 ms → **与游标页同级（≈12 ms）**。
- **验收判据**：14,531 条目、offset=7,265 时 < 20 ms；条目 50,000 时 < 25 ms；
  且该命令的 gRPC 次数 = 1（可用 `command_diagnostics` 计数）。

### P0-3 `get_messages` 改成按页读，或直接下线

- **现状**：14,531 条目会话 249 ms 热 / 562 ms 冷，回复 77 KB，深页与首页同价；**5,600 条起越过
  100 ms**，**约 19,000 条起直接失败**（25,000 条时一条 gRPC 回复 44.4 MB、50,000 条时 74.1 MB，
  撞 32 MiB 上限，返回 `OutOfRange`）。也就是说会话足够长之后这条命令不再有"慢"这个状态，只有"错"。
- **改法**：当前手机与桌面**都已不发 `get_messages`**（`01-client-surface.md` §1.2）。优先
  **从路由表退役并保留一个迁移期错误码**，把流量引到 `get_session_entries`（游标）；若必须保留，
  则让 Agent 按 `(offset, limit)` 返回页，桌面不再取整段，并加字节预算。
- **预期**：退役即消失；若保留，249 ms → **≤15 ms** 且与会话长度无关。
- **验收判据**：退役后路由表无此命令且老客户端收到明确的升级错误；保留则 14,531 与 50,000 条目
  会话耗时相差 ≤ 20%，且 50,000 条时不再失败。

### P1-4 `list_session_files` 改成按页列

- **现状**：5,000 个文件 **229 ms**（2,200 个文件即越过 100 ms），≈0.046 ms/项，全部在文件系统：
  每个目录项都 `canonicalize()` + 判权限 + `metadata()`（`host/src/remote_host/session_files.rs:56-105`）。
  带 `node_modules` 的工作区就是这种规模。
- **改法**：（a）默认页大小（如 200）+ `hasMore` 游标；（b）只在 `symlink_metadata` 报告是符号链接时
  才 `canonicalize()`，普通文件用 `symlink_metadata` 一次拿到类型与大小；
  （c）`ensure_path_allowed` 对同一父目录结果缓存。
- **预期**：首屏与页大小成正比（200 项 ≈ 9 ms），与目录总量无关。
- **验收判据**：5,000 项目录首屏 < 20 ms，且 10 项与 5,000 项的耗时比 ≤ 3。

### P1-5 `preferSnapshot` 不要先整段投影再丢弃

- **现状**：`get_events_since` 带 `preferSnapshot` 时，Agent 先把整个 run 折成快照（O(事件数)），
  然后才检查体积；超预算（>10,000 事件）就返回 `run_snapshot_too_large`，host 当 `None` 回落到分页读。
  **于是每次打开都白做一次整段投影再丢掉**：25,000 事件 41.5 ms、50,000 事件 81 ms，
  而真正的分页读只要 2 ms。
- **改法**：折叠之前先按事件数/字节水位判规模直接拒绝；或把投影缓存住（run 不可变，天然可缓存）；
  或让客户端在超预算时不请求快照。复核补上了阈值的确切形状：8 MiB 预算按**每事件
  `data.len()+512`** 计，因此**任何 ≥ 约 16,100 事件的 run 永远不可能产出快照**——
  也就是说这个『优化项』对长 run 从一开始就是纯浪费，客户端侧可以按水位直接不请求。
- **验收判据**：50,000 事件的 run 上该命令 ≤ 5 ms（与首页同级）。

### P1-6 `list_available_skills` 离开网络路径

- **现状**：p95 120 ms，取决于平台服务与网络，无上界（连接超时 10 s、总超时 60 s）。
- **改法**：Agent 侧加 TTL 缓存（例如 10 min）+ 后台刷新；读路径只返回缓存，缓存缺失时返回
  上一次成功结果而不是同步等待。
- **预期**：p99 ≤ 5 ms。
- **验收判据**：断网状态下该命令仍 ≤ 20 ms 且返回上一份目录。

### P1-7 `get_queue` 合成一次往返

- **现状**：21.6 ms / 93 字节 = 一次 preflight + 一次完整 `get_state`，2 s 轮询。
- **改法**：复用桌面对该会话已有的 `get_state` 结果（同一次请求内缓存），或去掉 preflight
  （队列是只读展示，删掉的那条安全栅栏在写命令上才必需）。
- **预期**：21.6 ms → 与命中缓存的 `get_state` 同级（≤1 ms）。
- **验收判据**：2 s 轮询下每 10 s 的 Agent RPC 次数从 10 降到 ≤1。

### P1-8 `list_sessions` / `list_workspaces` 省掉"每条命令都重算"

- **现状**：682 线程 p50 21 ms（回复 70 KB）；合成到 5,000 线程是**双峰 43–72 ms**
  （命中 1 s 目录快照时 ≈43 ms，未命中走完整读时 ≈72 ms），≈0.009–0.014 ms/线程。
  把这条命令的 SQL 拆开量：`threads` 7.0 ms + `latest_run_infos` 8.6 ms + `workspaces` 0.3 ms ≈ **15.9 ms**，
  **其余 27–56 ms 在装配与回复构造**。全程持 `CATALOG` 全局锁。
- **改法**：
  - **不要**去"合并 per-thread 查询"——`latest_run_infos` **本来就是一条**窗口函数查询
    （`host/src/store/runs.rs:297-320`，`ROW_NUMBER() OVER (PARTITION BY thread_id ...)`）；
    复核已证否"每线程查一次"这个判断（`05-verification.md` §7.1 第 5 条）。改它会改到已经正确的实现上。
  - 真正该做的是：① 把 `active_run_sessions()` 与 `latest_run_infos()` 合成一次 `runs` 扫描
    （现在是两次扫同一张表）；② **把序列化与结构签名移到锁外**，锁内只取快照并按 revision 判断
    是否真要重算；③ 桌面端 60 s/台的轮询改成事件驱动（手机已经是推送驱动）——这条命令的
    p50 里有一半是"没人看也要算一次"。
- **预期**：SQL 部分 15.9 ms 可压到 ≈12 ms；真正的收益在装配与锁（27–56 ms），
  目标是 682 线程 p95 ≤ 12 ms、5,000 线程 p95 ≤ 40 ms。
- **验收判据**：线程数 10 与 5,000 的耗时比 ≤ 4（当前 ≈18–30×）；同一进程内连续两次调用的
  耗时不得相差 60%（消除双峰）；`list_sessions` 期间其它命令的 p95 不上升（锁外化）。

### P2-9 内容读的 preflight 每会话一次

- **现状**：`verify_session_database` 被每条内容读调用（2 次点查 + 1 次 gRPC）。
- **改法**：按 `(session, database_id, restore_generation)` 缓存校验结果，`session_presence`
  变化时失效；或在建立连接/打开会话时校验一次。
- **预期**：所有内容读各减一个往返（≈2–4 ms），`get_state` 底噪从 4.5 ms → 2 ms 级。
- **验收判据**：`get_state` 在 236 条目会话上 p50 ≤ 2 ms，Agent RPC 次数 3 → 1。

### P2-10 任务面板的索引

- `task_runs` 加 `(task_id, started_at DESC)` 索引，`list_task_runs` 的 `LIMIT` 才真的省成本
  （5,000 运行 0.99 ms → 亚毫秒；实测 `LIMIT 1` 与 `LIMIT 20` 同价，说明排序发生在截断之前）。
  判据：`LIMIT 1` 与 `LIMIT 20` 的差距 > 10×，2,000 次运行 ≤ 0.3 ms。

### P2-11 `list_models` 的 O(目录) 常数

- 把 `is_model_available(format!("provider/id"))` 的逐项全目录解析改成一次索引化的可用集合，
  并在注册表更新时缓存。判据：21 ms → ≤2 ms。

---

## 6 验收：怎样证明"最差情况 < 100 ms 且不随数据规模上涨"

### 6.1 验收标准（建议写进任务的完成定义）

1. **单命令（当前数据）**：§2 表的 41 个组合在 release 构建下 p95 ≤ 100 ms；cold ≤ 150 ms
   （cold 不含磁盘冷读，真实冷盘只会更慢，所以留出余量）。
2. **单命令（放大后）**：`04-scaling.md` 的每个档位都必须达标，重点是已经越线的四档和逼近的两档：

   | 命令 | 必须达标的档位 | 当前 | 目标 |
   | --- | --- | ---: | ---: |
   | `get_session_entries` offset 页 | 会话 50,000 条 | 2,264 ms | ≤ 25 ms |
   | `get_messages` | 会话 50,000 条 | **失败** | 退役，或 ≤ 20 ms 且不失败 |
   | `list_session_files` | 目录 5,000 项 | 229 ms | ≤ 20 ms |
   | `get_events_since` + `preferSnapshot` | run 50,000 事件 | 81 ms | ≤ 5 ms |
   | `get_state` | 会话 50,000 条 | 72 ms | ≤ 5 ms |
   | `list_sessions` | 线程 5,000 | 71 ms | ≤ 25 ms |

3. **打开一条会话**：手机 3 个往返的总和 p95 ≤ 250 ms（`get_state` + 游标历史页 + `get_read_chunk`）；
   桌面串行路径同样 ≤ 300 ms，或改成与手机一致的并发。
4. **平坦性**：对每个随规模变化的命令，在"条目 10×、线程 10×、运行 10×"三档上复测，
   **耗时比 ≤ 1.5**（当前 `get_messages` 是 190×，offset 页是 1,600×，`list_session_files` 是 390×）。
5. **无上界项**：`list_available_skills` 这类外部依赖必须在断网/超时下仍 ≤ 20 ms（缓存兜底）。
6. **不许回退**：游标页、事件首页/深页、`get_tool_call_args`、任务面板、设置读取在每一档上都必须是平的
   （§4.2 是它们的基线）。任何一条开始随规模增长即为回退。
7. **`get_state` 的 cold 单独定标**：cold 的主体是"首次 hydrate 一个会话"，语义上 O(条目)、
   每会话一次（复核：50,000 条 173.7 ms，摘掉两处温路径读后仍 127.8 ms）。它的目标不是
   "与会话长度无关"，而是**冷开一条会话整体 ≤ 150 ms**；若要进一步压低，必须分片 hydrate
   或把首屏所需的最小状态单独读出来，那是另一项工作，不与 P0-1 混谈。

### 6.2 复测方法（已就位，直接跑）

```bash
cargo test -p future-host --release --no-run
cargo build -p future-agent --release
python3 scripts/perf/remote-commands/measure-remote-latency.py --iterations 10 --tag "$(date +%Y%m%d-%H%M%S)"
python3 scripts/perf/remote-commands/summarize-remote-latency.py --tag <tag>
# 逐维度放大扫描（先建 14 个单维度 home，再在每个 home 上跑同一支探针）
python3 scripts/perf/remote-commands/scale/build_scale_data.py
python3 scripts/perf/remote-commands/scale/run_scale_sweep.py
python3 scripts/perf/remote-commands/scale/summarize_scaling.py --write   # 重新生成 04 的表
```

脚本以 `mode=ro` 打开线上库做在线备份，探针只在带 `MEASUREMENT_ISOLATED_HOME` 标记的一次性 HOME 里
运行，Agent 绑随机空闲回环端口——**不写线上库、不碰 50051、不杀用户正在跑的 agent**。

---

## 7 未测出与不确定（不要当成已证明）

- **`get_event_content_range` 未测出**：它要求客户端拿着 `content_refs` 引用，而 Agent 只为超过
  16 KiB 的正文生成引用；本快照最大的单条正文字段是 14,077 B，低于阈值，探针取不到任何真实引用。
  它的成本目前只有代码级判定（O(取回字节)）。
- **`get_fusion` 只测到"没有融合运行"这一支**（快照里 `fusion_runs` 为 0）。有数据时的
  `fusion_events` 分页没测；带 `step_id` 的那一支会整段载入子会话（`02-code-audit.md` §3.8），
  属于同类问题，但未实测。
- **cold 不是磁盘冷读**（未 `sudo purge`）。真实首次打开只会更慢；`get_state` 冷开的 166 ms 是下界。
- **写命令与配对流未覆盖**（`prompt`、`abort`、`approval_decision`、`upload_*`、`download_*`、
  `get_read_chunk`）：本任务是"打开"延迟。
- **单机单次**：M3 Ultra + release。p95 由 10 个样本的最近秩给出（等于最大值），不要把 p95 当分位数读。
- **跨平台未测**：`get_settings` 在 Windows/Linux 会多一次 `probe_sandbox()` RPC（macOS 上直接返回），
  这三个平台的数字不能混用。
- **测量期间线上库仍在增长**，快照是一个时点（227,939 条目 / 355 个有内容会话）；换一天绝对值会有
  几个百分点漂移，但上面三条曲线的**斜率**不会变。

---

## 8 附录：本次审计顺带发现的两处规格漂移（不属于性能，但会让后续工作踩空）

1. **`PHONE_COMMANDS`（`business/mod.rs:254-315`）滞后于真实客户端 6 条命令**，其中 `get_queue`
   是 2 s 轮询。这意味着"手机发的每条命令都被路由"这个守卫测试对它们是盲的。
2. **仓库里没有收拢完整接口清单的地方**：`route()`（68）与桥层命令（11）分处两个文件，
   `command.schema.json` 只描述字段形状、`type` 无 enum，`PHONE_COMMANDS` 是测试夹具。
   建议把三处合成一份生成式清单（schema 里给 `type` 加 enum，从 `route()` + 桥层命令生成），
   让"新增一条命令"同时被编译期与测试发现。

---

*本报告由 4 个并行工作流（客户端命令面 / 服务端读路径代码审计 / 真机测量 / 逐维度放大扫描）
加一次独立复核产出；每条数字都能在同一目录的 `02`/`03`/`04` 与 `raw/` 原始样本里回溯。*
