# 远端桌面命令打开延迟：审计与优化报告（2026-11-04）

> **范围：只看新客户端。** 本文只统计**当前版本**的手机（`mobile/src/remote/`）与桌面客户端
> （`desktop/src/features/remote-peer/` + `packages/remote-client/`）**真正会发出**的命令与参数。
> 只有老客户端才会发的两条（`get_messages`、`get_session_entries` 的 `offset` 分页）**不在优化范围**，
> 其数字完整保留在**附录 A**——因为它们的处置是"退役"，不是"提速"，混进排序会把预算花在
> 新客户端根本走不到的代码上。
>
> **数据**：本机 `agent.db` 2.5 GB / 713 个会话行（其中 355 个有内容）、`app.db` 54 MB / 682 线程、
> `tasks.db` 4 MB；release 构建、冷热分离、10 次采样。方法见 §1.3 与 `03-measurements.md`。
>
> **配套章节**（同一目录，每个数字都能回溯）：
> `01-client-surface.md` 客户端真正发出什么命令、什么参数、什么时序 ·
> `02-code-audit.md` 每条命令的 SQL / 索引 / 复杂度 ·
> `03-measurements.md` 真实数据实测 · `04-scaling.md` 逐维度放大后的斜率 ·
> `05-verification.md` 独立复核 ·
> `06-read-path-fixes.md` **改造与复测**：四条超时命令改了什么、改完每条多快
> （同一套 harness 的前后对照）
>
> **一句话结论**：新客户端打开一条会话会并发发出 `get_state` + 一页游标历史，因此等待 ≈ 较慢的那一条。
> **其中只有 `get_state` 超过 100 ms**（最大会话首次触达 166 ms、中位会话 59 ms），原因是它每次调用都
> 对 legacy 会话做**两处整段读**（§3.1）。此外只有 `list_available_skills`（114/120 ms，同步 HTTP，
> 无上界）单独超预算，但它在技能页/`/` 菜单，不在打开路径上。
> 老客户端那两条（offset 页 698 ms、`get_messages` 249 ms→2.3 s 并撞 32 MiB 失败）数字更大，
> 但新客户端一条都不发——见附录 A。
>
> **如果只做三件事**：①`get_state` 的**两处**整段读改回 O(1)（P0-1，唯一在打开路径上超预算的命令，
> 且随会话线性涨到 50,000 条的 72 ms）；②`list_available_skills` 离开网络路径（P0-2，唯一没有
> 上界保证的命令）；③`list_session_files` 加分页（P1-3，一个 `node_modules` 就能把它推到 229 ms）。
> 三件做完，新客户端在新数据与 50,000 条的放大档位上都在 100 ms 以内且斜率≈0。

---

## 0 先看结论

### 0.1 新客户端打开一条会话：一次等待由两条并发读决定

手机打开会话（`packages/remote-client/src/syncEngine.ts:596-615`）：`get_state` 与第一页历史
**同时在途**——源码原话是"这是读者全部的等待"。所以等待 ≈ `max(两条)` + 分块补读，不是两者之和。

| 阶段 | 命令（实填参数） | 最大会话（14,531 条） | 中位会话（236 条） | 判定 |
| --- | --- | ---: | ---: | --- |
| 状态 | `get_state(sessionId)`，进程内首次 | **166.1 ms** | **58.6 ms** | **超预算** |
| 状态 | 同上，温 | 24.4 ms | 4.5 ms | ok |
| 首屏历史 | `get_session_entries(before=尾游标, limit=3, chunkedRead)` | 38.8 cold / 11.5 warm | 14.3 cold / 6.1 warm | ok |
| 分块补读 | `get_read_chunk(offset=0,192K,…)`，回复 >192 KB 时 1–2 次、并发 4 | 未测（见 §7） | 通常不发 | 未知 |
| 活跃 run | `get_events_since(runId, sinceIdx=-1, limit=1000, preferSnapshot)` | 2.0–2.4 ms | 2.0–2.4 ms | ok |

⇒ 新客户端打开一条会话：**最大会话首开 166 ms（超 66 ms），中位会话 59 ms**。
两个数字都是"进程内首次触达该会话"，它同时包含该会话的首次 hydrate（每会话一次）。
冷热两条路要分开优化，见 §3.1 与 §6.1 第 6 条。

### 0.2 新客户端会发、且超 100 ms 的命令

| 命令 | 触发 | p50 | p95 | 在打开路径上 | 根因 |
| --- | --- | ---: | ---: | --- | --- |
| `get_state` | 打开会话（第一个命令）、余额面板、compaction 等待中 5 s 轮询 | **166（首开）/ 24（温）** | 25 | **是** | §3.1：对 legacy 会话每次调用做两处整段读 |
| `list_available_skills` | `/` 菜单、技能页、每连接一次 | **114** | **120** | 否 | Agent 每次调用都同步 `GET` 平台技能目录（超时 10 s / 60 s） |

其余新客户端会发的命令（`get_session_entries` 游标页、`get_read_chunk`、`get_events_since`、
`get_tool_call_args`、`get_event_content_range`、`get_fusion`、`list_session_files`、`get_queue`、
`list_sessions`、`list_workspaces`、`list_models`、`list_skills`、`get_settings`/`get_desktop_settings`/
`get_fusion_settings`、`list_providers`、`list_tasks`/`get_task`/`list_task_runs`、`get_prompt_receipt`、
`skill_reco_today`）在当前数据上**全部 ≤ 25 ms**，逐条见 §2——表里除标〔老〕/〔无〕的行之外，
其余都是新客户端会发的。

### 0.3 新客户端会发、现在没超但「数据变大就会严重越线」的

| 命令 | 随哪个维度 | 当前 | 放大后 | 首次越过 100 ms | 在新客户端哪里 |
| --- | --- | ---: | ---: | --- | --- |
| `list_session_files` | 目录条目数 | 10→0.59 ms | 1,000→**45.6**，5,000→**229 ms** | **2,200 个文件**（实测） | 文件面板进目录（含 `node_modules`） |
| `get_state`（温） | 会话条目 | 14,531→24 ms | 25,000→36.7，50,000→**72.1 ms** | 约 71,100 条（外推） | 打开会话 / 余额面板 / 5 s 轮询 |
| `get_events_since` + `preferSnapshot` | 单 run 事件数 | 10,855→2.0 ms | 25,000→**41.5**，50,000→**81 ms** | 约 61,600 事件（外推） | 打开一个还有活跃 run 的会话 |
| `list_sessions` | 线程数 | 682→21 ms | 2,000→31，5,000→**43–72 ms**（双峰） | 约 7,100–11,000 线程（外推） | 桌面端**每台主机每 60 s** 轮询；手机 10 min 自愈 |
| `list_tasks` | 任务数 | 33→2.6 ms | 500→**15.9 ms** | 约 3,400 个任务（外推） | 任务页 |

反向对照同样重要：**同一类读取里，走游标的一条都不涨**——`get_session_entries` 游标页从 41 条到
50,000 条一直是 1.4–15.8 ms，`get_events_since` 首页/深页 2.06–2.15 ms，`get_tool_call_args` 0.55–0.71 ms。
所以"随数据上涨"不是必然，是**分页方式与读取粒度**的选择；这也说明新客户端选对了分页方式，
它剩下的问题只有 `get_state` 一个。

### 0.4 用户问的两个问题（按新客户端口径）

**问题一：读数据库的时候是不是 O(1)？**

不是。新客户端会走到的路径上有 **4 处**不是 O(1)：

| 类型 | 位置 | 复杂度 | 在新客户端哪里 |
| --- | --- | --- | --- |
| **整段加载冒充元数据读**（两处，只对 legacy 会话） | ① `agent/src/session/manager/loading.rs:13` `state_metadata()` → `:68` `load_metadata()`：`SELECT payload FROM history_shapes …` 全部行 + 逐行解码，只为数出用户消息数与 parent id（占超额 **68%**，会话已 hydrate 时结果被丢弃）<br>② `agent/src/session/manager/mod.rs:321` `unterminated_run_id()` → `transcript.rs:203` 的 legacy 分支扫 **`entry_records` 视图**（不可索引，占 **30%**） | O(会话条目) | **`get_state`，每次调用** |
| **全表扫描 + 窗口函数**（一次读里做两遍） | `host/src/store/threads.rs:49` 682 行 threads + 表达式排序、`host/src/store/runs.rs:305` 窗口函数（**单条**查询，不是 N+1）、`packages/app-workspaces` 每行一个相关子查询 | O(threads + runs) | `list_sessions`（桌面 60 s/台） |
| **`LIMIT` 不省成本** | `task_runs` 上无 `(task_id, started_at)` 索引，`ORDER BY … LIMIT 20` 先排序后截断（`LIMIT 1` 与 `LIMIT 20` 同价） | O(该任务运行次数) | `list_task_runs`、`get_task`、`list_tasks` |
| **逐项系统调用** | `session_files.rs:56-105`：每个目录项都 `canonicalize()` + 权限检查 + `metadata()` | O(目录项 × 路径深度) | `list_session_files` |

（老客户端那条"先全取再内存切片"的假分页不在上表——见附录 A。）

一句话：**"库有多大"不影响任何命令，"这个会话/这个 run/这个目录有多长"决定了一批命令的延迟**。
大表读取基本都靠 `(session_id, …)` 前缀索引定位；出问题的不是索引缺失，而是**读取的粒度选错了**。

**问题二：延迟会不会随数据规模同步上涨？**

会，线性、无拐点，新客户端会走到的有 5 条：`list_session_files`（目录 5,000 → 229 ms）、
`get_state`（会话 50,000 条 → 72 ms）、`get_events_since`+`preferSnapshot`（50,000 事件 → 81 ms）、
`list_sessions`（5,000 线程 → 43–72 ms）、`list_tasks`（500 任务 → 15.9 ms）。
斜率与实测点见 §4，逐维度扫描见 `04-scaling.md`。

### 0.5 这份报告被独立复核过

`05-verification.md` 由一个独立工作流重算了 03 的 40 行、04 的 85 行、四条越线交叉点与两个字节数
（全部原样复现），并用四个构建变体定位了 `get_state` 的归因。它同时**否证**了三处判断，本报告已订正：
① `get_state` 的成因是**两处**整段读而不是一处（68%/30%），只改一处不达标；② 多出来的时间在
**命令体内部**，不在 `verify_session_database`/observer 的装配上；③ `list_sessions` 的
`latest_run_infos` **本来就是单条窗口函数查询**，不是 N+1，按"每线程一次查询"去改会改到已经正确的
实现上；以及 5,000 线程是 **43–72 ms 双峰**而不是"至少 71 ms"。

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
实际发出的命令与实填参数，得到 26 条读命令 × 41 个规模组合的实测面。

**这 26 条里有 2 条没有新客户端在发**：`get_messages`（只在 `desktop/src/features/remote-peer/remotePeerClient.ts:1381`
的"走原生分块读的名字白名单"里出现，没有任何调用点构造它）与 `get_session_entries` 的 `offset` 分页分支
（新客户端手机/桌面一律传 `before` 游标：`mobile/src/remote/timeline/historyPaging.ts:26`、
`useOlderHistory.ts:86`、`desktop/src/features/remote-peer/useRemoteTimeline.ts:447`）。
另有 `get_available_models`、`apply_task_revision`、`list_task_revisions` 与三条 `*_task_dep*` 过渡期命令
无新客户端发送。本报告把它们移到附录 A 与 §2 的〔老〕标记下，不参与排序。

顺带发现两处**规格漂移**（不在本报告的优化范围内，但会让后续工作踩空）：`PHONE_COMMANDS` 漏登记 6 条
手机正在发的命令（`get_queue`、`cancel_queued_run`、`release_queued_run`、`get_fusion`、
`get_event_content_range`、`rename_workspace`），其中 `get_queue` 是 2 s 轮询。

### 1.2 "打开一条会话"到底发几条命令

这是"打开延迟"的真正口径——用户等的是这一串，不是单条命令：

- **手机**（并发）：`get_state(sessionId)` ‖ `get_session_entries(before=MAX, limit=3, chunkedRead)`
  → 每个 >192 KB 的回复再发 `get_read_chunk`（并发 4）→ 尾页被裁剪时串行补 1–3 页
  → 有活跃 run 时 `get_events_since(sinceIdx=-1, limit=1000)` 逐页回放。
  **最少 3 个往返**。
- **桌面作为客户端**（串行）：`get_session_entries` → `get_state` →（有活跃 run）`get_events_since`。
  串行是桌面侧独有的缺陷（手机已经并发），所以桌面打开一条会话的等待是**两者之和**而不是较大者；
  在最大会话上这意味着 11.5 + 166 ≈ 178 ms 而不是 166 ms。这是一条客户端侧就能改的 P2（§5 P2-10）。

所以"每条命令 <100 ms"是必要条件而不是充分条件；§6 的验收判据按**打开序列的实际等待**给出
（手机取并发口径 max，桌面取串行口径 sum）。

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
〔老〕= 只有老客户端会发（数字保留，见附录 A，不参与排序）；〔无〕= 没有任何新客户端发送。
**未标记者都是新客户端会发的行**，也就是本报告的优化范围。

| 命令 | 规模锚 | cold | p50 | p95 | 回复字节 | 判定 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| `list_sessions` | 682 线程 | 22.4 | 21.2 | 24.1 | 70,213 | ok |
| `list_workspaces` | 682 线程 | 4.8 | 3.6 | 4.7 | 5,459 | ok |
| `get_state` | largest | **166.1** | 24.4 | 24.6 | 1,475 | **超预算（冷）** |
| `get_state` | median | 58.6 | 4.5 | 4.6 | 1,488 | ok |
| `get_state` | small | 55.6 | 4.3 | 4.7 | 1,475 | ok |
| 〔老〕`get_messages` 首页 | largest | **561.6** | **249.0** | **255.5** | 76,814 | **超预算** |
| 〔老〕`get_messages` 深页 | largest @7,265 | **240.6** | **234.6** | **238.9** | 35,340 | **超预算** |
| 〔老〕`get_messages` 首页 | mid | **177.0** | 57.1 | 57.6 | 58,597 | **超预算（冷）** |
| 〔老〕`get_messages` 首页 | median | 17.0 | 6.5 | 6.7 | 38,527 | ok |
| 〔老〕`get_messages` 首页 | small | 5.9 | 1.3 | 1.4 | 23,856 | ok |
| `get_session_entries` 首屏（游标） | largest | 38.8 | 11.5 | 11.6 | 87,009 | ok |
| `get_session_entries` 深页（游标） | largest @7,265 | 26.6 | 11.5 | 11.7 | 121,851 | ok |
| `get_session_entries` 首屏（游标） | median | 14.3 | 6.1 | 6.2 | 28,605 | ok |
| 〔老〕`get_session_entries` **offset 页** | largest @7,265 | **589.3** | **698.4** | **720.8** | 14,510 | **超预算** |
| 〔老〕`get_session_entries` **offset 页** | mid @1,572 | **132.9** | **147.4** | **161.7** | 13,349 | **超预算** |
| 〔老〕`get_session_entries` offset 页 | median @118 | 14.0 | 5.7 | 5.8 | 21,722 | ok |
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
| 〔无〕`get_available_models` | — | 21.4 | 21.1 | 27.6 | 1,425 | ok |
| `list_skills` | — | 2.2 | 1.6 | 1.8 | 12,633 | ok |
| `list_available_skills` | 平台目录 | 67.5 | **113.7** | **119.5** | 47,024 | **超预算** |
| `skill_reco_today` | — | 0.2 | 0.0 | 0.0 | 91 | ok |
| `list_providers` | — | 2.1 | 1.5 | 1.9 | 22,341 | ok |
| `list_tasks` | 33 任务 | 5.6 | 2.6 | 3.0 | 9,123 | ok |
| `get_task` | 356 次运行 | 2.1 | 1.4 | 1.5 | 24,813 | ok |
| `list_task_runs` | 356 次运行 | 2.4 | 1.7 | 1.7 | 11,708 | ok |
| 〔无〕`list_task_revisions` | 36 版本 | 2.5 | 1.6 | 1.6 | 329,914 | ok |
| 〔无〕`list_task_deps` | — | 0.0 | 0.0 | 0.0 | 48 | ok |

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

### 3.2 〔老客户端〕假分页：`get_messages` 与 `get_session_entries` 的 offset 路径

> 这一节只影响老客户端。新客户端两条分页都走游标（`before`），因此这里的代价落在预算之外；
> 处置是**退役**而不是提速，见附录 A。之所以留在正文，是因为它示范了同一位点上游标与 offset
> 相差 60 倍，这个对照正是新客户端那条路径『为什么达标』的证据。

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

### 4.1 八条确定在涨的曲线（新客户端会走 5 条，另 3 条只属老客户端）

| 命令 | 维度 | 实测（p50） | 形状 | 斜率 |
| --- | --- | --- | --- | --- |
| 〔老〕`get_messages` 首页 | 会话条目 | 3,145→62.5 → 14,531→**251 ms**（25,000/50,000 直接失败） | 线性 | ≈+118 ms/10× |
| 〔老〕`get_messages` 深页 | 会话条目 | 14,531→**253 ms** | 线性 | ≈+120 ms/10× |
| 〔老〕`get_session_entries` offset 页 | 会话条目 | 3,145→**136** → 14,531→**682** → 50,000→**2,264 ms** | 线性 | ≈+592 ms/10× |
| `list_session_files` | 目录条目数 | 1,000→**45.6** → 5,000→**229 ms** | 线性（文件系统，非数据库） | ≈+92 ms/10× |
| `get_events_since` + `preferSnapshot` | 单 run 事件数 | 10,000→1.29 → 25,000→**41.5** → 50,000→**81 ms** | 10,000 处跳变后线性 | ≈+34 ms/10× |
| `get_state`（热） | 会话条目 | 3,145→7.8 → 14,531→23.2 → 50,000→**72.1 ms** | 线性 | ≈+17 ms/10× |
| `list_sessions` | 线程数 | 682→18.1 → 2,000→31.1 → 5,000→**70.8 ms**（重测 43–72 双峰） | 线性 | ≈+27 ms/10× |
| `list_tasks` | 任务数 | 33→2.3 → 500→**15.9 ms** | 线性 | ≈+5.9 ms/10× |

上表标〔老〕的三行不在本报告的优化范围（见附录 A）；**新客户端真正要处理的是其余 5 行**。

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

## 5 优化清单（只列新客户端会走的路径，按收益排序）

> **进度（2026-11-04 更新）**：P0-1 与 P0-2 都已落地。P0-1 由 `038d9642d perf(agent): answer
> page-sized reads with page-sized reads` 完成并顺带修掉同类的 `get_messages`（249 → 8 ms）；
> P0-2 由同一提交的 `catalogue_read` + 持久副本完成。这两条不再是待办，下面的原判据保留作为
> 验收口径。P0-1 的**语义等价**在真实快照上单独验证过：392 个 legacy 会话、0 处差异
> （`agent/src/session/manager/tests/equivalence.rs`）。本报告 §0–§4 的数字是**落地前**的基线。

每条都写明：**现状 → 改法 → 预期 → 验收判据**。所有判据都可以用 §6.2 的脚本机械复测。
排序依据不是"谁的数字最大"，而是"谁在新客户端的真实路径上、被多少人碰到"——老客户端那两条
数字最大的命令（698 ms / 249 ms）不在本表，它们在附录 A 里等退役。

### P0-1 `get_state` 的两处整段读改回真正的 O(1)

> **落地与验证（2026-11-04）**
>
> 落地：`038d9642d` 一次修掉三处同类读——`state_metadata` 的 legacy fallback、legacy 会话的
> 元数据整段读、`read_run_marker_headers` 扫 `entry_records` 视图——并把 `get_messages` 改成按页。
> 该提交自报 `get_state` 冷开 **166 → 60 ms**、`get_messages` **249 → 8 ms**（判据同 §6.1）。
>
> 验证（本分支的贡献，证明它没有改答案）：`agent/src/session/manager/tests/equivalence.rs` 在
> 一份真实快照的只读副本上，把现在的读与"整段读"的答案逐会话对齐——用户消息数的真值取
> `history_shapes` 里 body 的 `role`，run marker 对齐 `entry_records` 视图的全扫结果——
> **392 个 legacy 会话，0 处差异**（355 个有内容，340 个 canonical 走未改动的分支）。
>
> 这条测试顺带抓到一个陷阱，值得写下来：同一个提交把 `load_metadata` 改成只读三行元数据
> （首条、最新 `session_info`、最新 `model_change`），于是任何"拿 `load_metadata` 的结果当基线"的
> 等价性比较都会得到 0，看起来像新读错了。基线必须建立在**数据**上，而不是另一个可能同时被改的
> 函数上。
>
> 仍未处理（与实现无关）：廉价计数是 O(用户消息数)，常数约 20 ns/条——真实密度 0.016 ms、
> 病理密度（5 万条全是 user）1.01 ms。绝对判据远在预算内，但"Δ ≤ 0.5 ms"这条对病理密度不成立。

**唯一一条在打开路径上超过 100 ms 的命令**，也是唯一一条让"打开一条会话"这条链路超预算的原因。

- **现状**：legacy 格式会话（本机 392/713）每次调用都做两处全量读，温路径最大会话 24 ms、
  50,000 条 72 ms，斜率 1.19 µs/条目，**68% / 30%** 各一半（复核 A/B 实验，§4.3）。
  冷态（首次触达该会话）最大会话 166 ms——但**冷态与温态不是同一根因**：复核实测把两处读都摘掉
  之后，50,000 条的冷值只从 173.7 降到 127.8 ms（约 25%），其余是首次 hydrate 该会话本身。
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
- **验收判据**：legacy 会话 50,000 条 p50 ≤ 6 ms，**且** Δ(50k − 41) ≤ 0.5 ms；
  41 条目 legacy 与 2,043 条目 canonical 的 p50 差 ≤ 0.5 ms；
  打开一条会话（并发口径）在最大会话上 p95 ≤ 100 ms。
- **不在本条范围内**：冷态里的首次 hydrate。它每会话付一次、语义上就是 O(条目)，
  目标见 §6.1 第 6 条。

### P0-2 `list_available_skills` 离开网络路径

> **落地（2026-11-04）**：`038d9642d` 的 `catalogue_read()`——内存先查、再查上次成功抓取写下的副本、
> 超 TTL 先返回旧目录再后台刷新、`prewarm_catalogue` 在启动时预热。与本节原文有一处**不同且更好**：
> 完全无缓存时它**失败关闭**（报错而不是回空目录），因为客户端分不清"平台坏了"和"平台没有技能"；
> 本节原文那句"先回空目录"应以此为准。实现的代价不变：新发布的平台技能最长一个 TTL 才可见。

**唯一一条没有上界保证的命令**：它的延迟由平台服务与网络决定，100 ms 预算在这里无法用代码保证。

- **现状**：p50 114 / p95 120 ms，全部花在 `agent/src/skills/manager.rs:130` 的同步
  `GET {platform_url}/client/v1/skills`（连接超时 10 s、总超时 60 s，回复 47 KB）。
  触发点：手机 `/` 菜单、技能页、每连接一次的中文名回退。
- **改法**：Agent 侧加 TTL 缓存（例如 10 min）+ 后台刷新；读路径只返回缓存，
  缓存缺失时**返回上一次成功结果而不是同步等待**（首次无缓存也异步预热、先回空目录）。
- **预期**：p99 ≤ 5 ms，且与网络状况无关。
- **验收判据**：断网状态下该命令仍 ≤ 20 ms 且返回上一份目录；连续 10 次调用的 p95 ≤ 5 ms。

### P1-3 `list_session_files` 改成按页列

- **现状**：本机会话根 1.1 ms，但 5,000 个文件 **229 ms**（2,200 个即越过 100 ms），
  ≈0.046 ms/项，全部在文件系统：每个目录项都 `canonicalize()` + 判权限 + `metadata()`
  （`host/src/remote_host/session_files.rs:56-105`）。带 `node_modules` / 构建产物的目录就是这种规模。
- **改法**：（a）默认页大小（如 200）+ `hasMore` 游标；（b）只在 `symlink_metadata` 报告是符号链接时
  才 `canonicalize()`，普通文件用 `symlink_metadata` 一次拿到类型与大小；
  （c）`ensure_path_allowed` 对同一父目录结果缓存。
- **预期**：首屏与页大小成正比（200 项 ≈ 9 ms），与目录总量无关。
- **验收判据**：5,000 项目录首屏 p95 < 20 ms，且 10 项与 5,000 项的耗时比 ≤ 3。

### P1-4 `preferSnapshot` 不要先整段投影再丢弃

- **现状**：新客户端打开一个**还有活跃 run** 的会话时，首页 `get_events_since` 带 `preferSnapshot`；
  Agent 先把整个 run 折成快照（O(事件数)），**然后**才检查体积，超预算就回 `run_snapshot_too_large`，
  host 当 `None` 回落到分页读。于是每次打开都白做一次整段投影再丢掉：25,000 事件 41.5 ms、
  50,000 事件 81 ms，而真正的分页读只要 2 ms。
- **阈值**：8 MiB 预算按**每事件 `data.len()+512`** 计，因此**任何 ≥ 约 16,100 事件的 run 永远不可能
  产出快照**——客户端侧可以按水位直接不请求。
- **改法**：折叠之前先按事件数/字节水位判规模直接拒绝；或把投影缓存住（run 不可变，天然可缓存）；
  或客户端在超预算时不请求快照。
- **验收判据**：50,000 事件的 run 上该命令 p95 ≤ 5 ms（与首页同级）。

### P1-5 `get_queue` 合成一次往返

- **现状**：21.6 ms / 93 字节 = 一次 `verify_session_database`（2 次 app.db 点查 + **一次额外 gRPC**）
  + 一次完整 `get_state`，只为从状态里取队列字段；队列面板打开且非空时 **2 s 轮询**。
- **改法**：复用桌面对该会话已有的 `get_state` 结果（同一次请求内缓存），或去掉 preflight
  （队列是只读展示，那条安全栅栏在写命令上才必需）。
- **预期**：21.6 ms → 与命中缓存的 `get_state` 同级（≤1 ms）。
- **验收判据**：2 s 轮询下每 10 s 的 Agent RPC 次数从 10 降到 ≤1。

### P1-6 `list_sessions` / `list_workspaces` 省掉"没人看也算一遍"

- **现状**：682 线程 p50 21 ms（回复 70 KB）；5,000 线程 **43–72 ms 双峰**
  （命中 1 s 目录快照 ≈43 / 未命中 ≈72）。SQL 拆开：`threads` 7.0 + `latest_run_infos` 8.6
  + `workspaces` 0.3 ≈ **15.9 ms**，其余 27–56 ms 在装配与回复构造。全程持 `CATALOG` 全局锁。
  触发点：**桌面端每台主机每 60 s**（`useRemotePeers.ts:31,349-365`）、手机 10 min 自愈。
- **改法**：
  - **不要**去"合并 per-thread 查询"——`latest_run_infos` **本来就是一条**窗口函数查询
    （`host/src/store/runs.rs:297-320`）；复核已证否那个判断（05 §7.1 第 5 条）。
  - 真正该做的：① `active_run_sessions()` 与 `latest_run_infos()` 合成一次 `runs` 扫描；
    ② **把序列化与结构签名移到锁外**，锁内只取快照并按 revision 判断是否真要重算；
    ③ 桌面端 60 s/台的轮询改成事件驱动（手机已经是推送驱动，两端不对称是这块成本的主因）。
- **验收判据**：682 线程 p95 ≤ 12 ms、5,000 线程 p95 ≤ 40 ms；同进程连续两次调用耗时不得相差 60%
  （消除双峰）；`list_sessions` 期间其它命令的 p95 不上升（锁外化）。

### P2-7 内容读的 preflight 每会话一次

- **现状**：`verify_session_database`（`host/src/agent_bridge/presence.rs:118`）被 `get_state`、
  `get_session_entries`、`get_events_since`、`get_event_content_range`、`get_queue` 等**每条内容读**
  各调一次：2 次 app.db 点查 + 一次 `get_session_presence` gRPC。`get_state` 还额外调
  `ensure_observer_for_session`（又一次 `find_thread_by_agent_session`）。
- **改法**：按 `(session, database_id, restore_generation)` 缓存校验结果，presence 变化时失效；
  或在打开会话/建立连接时校验一次。
- **预期**：所有内容读各减一个往返；`get_state` 底噪从 4.5 ms → 2 ms 级。
- **验收判据**：`get_state` 在 236 条目会话上 p50 ≤ 2 ms，Agent RPC 次数 3 → 1。

### P2-8 任务面板的索引

- `task_runs` 加 `(task_id, started_at DESC)` 索引，`list_task_runs` 的 `LIMIT` 才真的省成本
  （5,000 运行 0.99 ms → 亚毫秒；实测 `LIMIT 1` 与 `LIMIT 20` 同价，说明排序发生在截断之前）。
- **验收判据**：`LIMIT 1` 与 `LIMIT 20` 的差距 > 10×，2,000 次运行 ≤ 0.3 ms。

### P2-9 `list_models` 的 O(目录) 常数

- 21 ms 一条、回复只有 1.4 KB：Agent 侧对**整个内置模型目录**（约 3.8k）逐个
  `is_model_available("provider/id")`（`agent/src/rpc/commands/providers.rs:180-186`，文件自己的注释称其
  "by far the most expensive test here"），再排序去重。
- **改法**：把逐项全目录解析改成一次索引化的可用集合，注册表更新时缓存。
- **验收判据**：21 ms → ≤2 ms。

### P2-10 桌面客户端把状态读与历史页并发

- **现状**：新客户端里只有桌面是串行的（`desktop/src/features/remote-peer/useRemoteTimeline.ts:438-452`
  先读历史页，`:591-635` 再读 `get_state`），手机已经是并发（`syncEngine.ts:596-615`）。
  串行意味着桌面打开一条会话要等**两者之和**：最大会话 11.5 + 166 ≈ 178 ms。
- **改法**：照手机的形态把两条读同时发出（历史页已有自己的取消语义，状态读失败时丢弃历史结果即可）。
- **预期**：桌面打开一条会话的等待从 sum 变成 max，即从 ~178 ms 降到 ~166 ms，
  并且与 P0-1 叠加后两者都在 6 ms / 12 ms 量级，不再有可感知差异。
- **验收判据**：桌面打开一条会话的命令序列在 trace 里重叠（`get_state` 的发出时间早于
  `get_session_entries` 的回复时间），且总等待 ≤ max + 10 ms。

---

## 6 验收：怎样证明"最差情况 < 100 ms 且不随数据规模上涨"

### 6.1 验收标准（新客户端口径，建议写进任务的完成定义）

1. **打开一条会话**（最重要的一条）：手机取并发口径（`get_state` ‖ 一页游标历史 + 分块补读），
   在**最大会话**（14,531 条）与**首次触达**这两个最差条件下，**p95 ≤ 100 ms**；中位会话 p95 ≤ 30 ms。
   今天的数字是 166 / 59 ms，所以这一条现在不达标，P0-1 是它唯一的阻塞项。
2. **单命令（当前数据）**：§2 表里除〔老〕/〔无〕之外的每一行，p95 ≤ 100 ms。今天只有
   `get_state`（冷 166）与 `list_available_skills`（120）不达标。
3. **单命令（放大后）**：新客户端会走的每个命令在 `04-scaling.md` 的每个档位都达标——
   重点是已经越线或逼近的这几档：

   | 命令 | 必须达标的档位 | 当前 | 目标 |
   | --- | --- | ---: | ---: |
   | `list_session_files` | 目录 5,000 项 | 229 ms | ≤ 20 ms |
   | `get_state` | 会话 50,000 条 | 72 ms | ≤ 6 ms |
   | `get_events_since` + `preferSnapshot` | run 50,000 事件 | 81 ms | ≤ 5 ms |
   | `list_sessions` | 线程 5,000 | 43–72 ms | ≤ 25 ms |
   | `list_tasks` | 任务 500 | 15.9 ms | ≤ 20 ms（含回归余量） |

4. **平坦性**：对每个随规模变化的命令，在"条目 10×、线程 10×、运行 10×"三档上复测，
   **耗时比 ≤ 1.5**。今天 `list_session_files` 是 390×、`get_state`（温）在 41→50,000 之间是 15×、
   `preferSnapshot` 在 10,000→50,000 之间是 63×。
5. **不许回退**：游标页（1.4–15.8 ms 跨 41→50,000 条）、事件首页/深页（2.06–2.15 ms）、
   `get_tool_call_args`（0.55–0.71 ms）、任务面板（≤3 ms）、设置读取（≤0.1 ms）在每一档上都必须是平的。
   任何一条开始随规模增长即为回退——**这是新客户端今天唯一做对的地方**。
6. **`get_state` 的冷态单独定标**：冷态主体是"首次 hydrate 一个会话"，语义上 O(条目)、每会话一次
   （复核：50,000 条 173.7 ms，摘掉两处温路径读后仍 127.8 ms）。它的目标不是"与会话长度无关"，
   而是**冷开一条会话整体 ≤ 100 ms**（第 1 条）；若要进一步压低，必须分片 hydrate 或把首屏所需的
   最小状态单独读出来——那是另一项工作，不与 P0-1 混谈。
7. **退场项（附录 A）**：`get_messages` 与 `get_session_entries` 的 `offset` 从路由表消失，
   老客户端收到明确的升级错误；不要求任何延迟目标，只要求删除后第 5 条不回归。

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

- **新客户端路径上有两条没测出，它们都在打开链路里**：`get_read_chunk`（回复超过 192 KB 时手机
  必须补读，`readPages.ts:85` 并发 4、单页上限 16 MB）与 `get_event_content_range`（回放里带
  `content_refs` 的事件）。前者在桥层是一次内存切片 + 重新编码，代码判定 O(chunk)，但**没有实测数字**；
  后者要求正文超过 16 KiB 才会有引用，本快照最大的单条正文字段只有 14,077 B，测不到真实引用。
  这两条的"便宜"目前只有代码级证据，跟 §2 里那些有实测的行不是一个强度。
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

## 附录 A：只有老客户端才会发、因此**退役**而不是提速的两条

它们曾经是这份审计里数字最大的两条，现在**没有任何新客户端会发**（§1.1 的核对），
所以它们不进 §5 的排序。处置判据只有一条：**删干净，并让老客户端拿到一个明确的升级错误**。

### A-1 `get_messages`：249 ms，且长会话直接失败

| 规模 | cold | p50 | p95 | 回复 |
| --- | ---: | ---: | ---: | ---: |
| 41 条 | 5.9 | 1.3 | 1.4 | 23,856 B |
| 236 条 | 17.0 | 6.5 | 6.7 | 38,527 B |
| 3,145 条 | **177.0** | 57.1 | 57.6 | 58,597 B |
| 14,531 条 | **561.6** | **249.0** | **255.5** | 76,814 B |
| 25,000 / 50,000 条 | — | **失败** | — | 44,420,183 B / 74,118,580 B（撞 32 MiB 上限） |

根因是"先取回整段历史再在内存切片"（`business/history.rs:65`），代价 ∝ 会话总条目，**深页不比首页便宜**。
触顶区间 **18,885–22,636 条**。

**处置**：从 `route()` 退役，返回一个明确的升级错误；`remotePeerClient.ts:1381` 的白名单里那一项一并删掉。

### A-2 `get_session_entries` 的 `offset` 分页：698 ms → 2.3 s

| 会话条目 | offset | cold | p50 | p95 |
| --- | ---: | ---: | ---: | ---: |
| 236 | 118 | 14.0 | 5.7 | 5.8 |
| 3,145 | 1,572 | **132.9** | **147.4** | **161.7** |
| 14,531 | 7,265 | **589.3** | **698.4** | **720.8** |
| 50,000 | — | — | **2,264** | 2,459 |

根因两层：桌面侧 `fetch_all_session_entries_with_client()`（`agent_bridge/queries.rs:466`）以 250 条/页
**串行拉完整段**（最大会话 59 次 gRPC）再丢掉 99%；Agent 侧 `before=None` 时靠索引数到第 n 条
（`paging.rs:215-219`）。

**处置**：退役 `offset`（`before=end` 已经能表达"从第 n 条往前一页"），或把 Agent 侧改为 `LIMIT/OFFSET`
直接取页并删掉那个循环。**判据**：路由表不再接受 `offset`；老客户端收到升级错误；游标页不回归（§6.1 第 5 条）。

### A-3 顺带说明

另有 `get_available_models`、`apply_task_revision`、`list_task_revisions` 与三条 `*_task_dep*` 过渡期命令
同属"无新客户端发送"。它们当前都很快（≤ 22 ms），留着无害，但同样不该出现在"给新客户端做优化"的清单里。

---

## 附录 B：本次审计顺带发现的两处规格漂移（不属于性能，但会让后续工作踩空）

1. **`PHONE_COMMANDS`（`business/mod.rs:254-315`）滞后于真实客户端 6 条命令**，其中 `get_queue`
   是 2 s 轮询。这意味着"手机发的每条命令都被路由"这个守卫测试对它们是盲的。
2. **仓库里没有收拢完整接口清单的地方**：`route()`（68）与桥层命令（11）分处两个文件，
   `command.schema.json` 只描述字段形状、`type` 无 enum，`PHONE_COMMANDS` 是测试夹具。
   建议把三处合成一份生成式清单（schema 里给 `type` 加 enum，从 `route()` + 桥层命令生成），
   让"新增一条命令"同时被编译期与测试发现。

---

*本报告由 4 个并行工作流（客户端命令面 / 服务端读路径代码审计 / 真机测量 / 逐维度放大扫描）
加一次独立复核产出；每条数字都能在同一目录的 `02`/`03`/`04` 与 `raw/` 原始样本里回溯。*
