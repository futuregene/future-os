# 独立复核：01–04 章与最终报告的数字、判定与归因

本章是这项审计的第五篇，也是唯一一篇**不生产新结论、只检验旧结论**的章节。前四章和
`docs/testing/remote-command-latency-audit-2026-11-04.md` 的每一个数字、每一条复杂度判定、
以及那份报告赖以排序的两条归因，都在这里被独立重算或重测了一遍。

复核不是"再跑一遍汇总脚本"。所有百分位都从 `raw/scaling/*.json` 的 **原始 `samples[]` 数组**
用最近秩重算（不读里面存好的 `p50Ms` 字段）；表格逐格比对；四条越线的交叉点自己拟合；
`get_state` 的归因用**四个构建变体**的对照实验定位。产物与命令在 §9。

## 0 结论摘要

| # | 被复核的判定 | 结论 |
| --- | --- | --- |
| 1 | 04 表 A 的 89 条曲线、88 行数字 | **确认**（85 行逐格相等；3 行属表格列语义问题，数字本身对，见 §7.2 第 11 条） |
| 2 | 03 §5 的 40 行真实数据表（cold / p50 / p95 / max / 字节） | **确认**（40/40 行五列全等） |
| 3 | 四条 100 ms 越线的交叉点：5,600 / 2,200 / 2,200 / 61,600 | **确认**（我复算 5,610 / 2,163 / 2,181 / 61,550） |
| 4 | `get_messages` 25,000 条 `OutOfRange`、44,420,183 B | **确认**（0/10 成功；50,000 条 74,118,580 B） |
| 5 | "约 19,000 条触顶" | **部分确认**：区间是 **18,885–22,636**，19,000 是紧的一端（§2.4） |
| 6 | 02 §3.12 `get_state` 温路径 O(1) | **否证**（04 已发现；本章给出**两条**原因与修复后对照，§3） |
| 7 | 最终报告：`state_metadata` 整段读 legacy 会话 | **确认，但不完整**：它占 68%，另 30% 是第二条 legacy 专属整段读（§3.4） |
| 8 | 04 §4.3 "多出来的时间在命令体之前（verify_session_database / observer / 装配）" | **否证**：代价在**命令体内部**，装配路径不随条目数增长（§3.3） |
| 9 | 04 §2.3 + §6 清单1/3：`list_sessions` "每个线程查一次 latest run" / "应合并成一条聚合" | **否证**：`latest_run_infos` 本来就是**单条窗口函数查询**（`host/src/store/runs.rs:297`），02 §3.1 的"无 N+1"才对（§7.1） |
| 10 | 04 §2.3 "5,000 线程至少 71 ms" | **否证**：重测 43.2 / 46.0 / 49.0 ms，且同一轮内是**双峰**（42–49 与 72–74）；"至少 71"是把缓存未命中那一峰当成了下界（§4.2） |
| 11 | 02 的 cursor 页 O(page)、`get_tool_call_args` O(1)、任务面板 N+1、`list_session_files` 文件系统成本 | **全部确认**（§5） |
| 12 | 01 的客户端调用点 file:line（抽查 5 处） | **确认**（§6） |
| 13 | 04 §4.4 `preferSnapshot` 先整段投影再丢弃 | **确认**；并补上阈值：**8 MiB 预算按每事件 `data.len()+512` 计**，⇒ 任何 ≥约 16,100 事件的 run 永远不可能出快照（实心：8 MiB ÷ (512 + 每事件数据)，即使 payload 为空也只有 16,384）（§5） |

一句话：**数字层面，这份审计是干净的**（03 的 40 行、04 的 85 行、四条越线、两个字节数，
全部可以原样复现）；**判定层面，问题集中在 `get_state` 的归因**——方向对、结论对，
但只找到了一半原因，而"只修一半"达不到最终报告自己定下的判据。

## 1 复核方法

三件事分开做，互不替代：

1. **离线重算**（不看作者脚本）：`target/remote-latency/verify/verify_numbers2.py`（04 表 A）、
   `verify_03.py`（03 §5）。百分位用最近秩（`k = ceil(q·n)`，1-based）从 `samples[]` 重算，
   与文件里存的 `p50Ms/p95Ms/maxMs` 对比；表格行按**数值**匹配（p50 2 位小数 + n 容差 1.5%），
   不按行序，所以错行、重复行、改过的行都会露出来。
2. **端到端重跑**：用仓库里同一支 `run_scale_sweep.py` 在同一批 home 上重测 3 个档位（§4），
   看数字是否落回噪声带。
3. **归因实验**（本章新增，机器上唯一的"新东西"）：把 `get_state` 的两条候选读**分别**和
   **同时**摘掉，各编译一个 release `future-agent`（26 s/次），在**同一个 home、同一支探针、
   同一份数据**上测 25 个会话（§3）。补丁只在本地编译，**没有提交**（`git status` 见 §9）。

复核过程中机器上有另一个会话在跑任务，这影响到了两次测量（§4.2 有记录），所以凡是
结论依赖单次测量的地方，都补了第二轮/第三轮。

## 2 数字复算：03 与 04 的表

### 2.1 04 表 A（逐维度曲线）

- `raw/scaling/` 里 14 个 warm 文件共 **89 条曲线**；表 A 共 **88 行**。
- 89 条曲线的 p50/p95/max 与我用原始样本重算的最近秩值**逐条相等**（容差 0.011 ms，0 处不符）。
- 88 行里 **85 行**按 (n, p50) 唯一匹配，六个数值列（规模 / cold / p50 / p95 / max / 字节）
  **逐格相等**（cold 按 1 位小数、其余按 2 位小数舍入比较）。
- 剩 3 行是 §7.2 第 11 条的列语义问题（表(d) 的三行 `list_tasks`），**不是数字错**。

### 2.2 03 §5（真实数据表）

- 40 个"有数字"的行全部匹配到 warm 文件的对应 label；**cold（取自 cold 相位文件）、
  p50、p95、max、回复字节五列 40/40 全等**。
- warm 文件共 41 个 label；未被任何表格行使用的只有
  `get_event_content_range:biggest_body`——它的 cold 调用 `success=false`，03 把它写成
  "未测出"，与原始样本一致。
- 03 §1 的"27 个命令 → 41 个组合""7 个组合超 100 ms"计数正确（我按 p50 或 cold 任一 >100 数出 7 个）。

### 2.3 四条越线的交叉点（我自己拟合）

| 曲线 | 我的斜率 | 100 ms 交叉点 | 04 章写的 |
| --- | ---: | ---: | ---: |
| `get_session_entries` offset 页 | 45.95 µs/条目（7 点 OLS，r²=0.999） | **2,163** | 2,200 |
| `list_session_files` | 45.88 µs/项（4 点） | **2,181** | 2,200 |
| `get_messages` 首页 | 17.24 µs/条目（只用成功的 5 点） | **5,610** | 5,600 |
| `get_events_since` + `preferSnapshot` | 1.69 µs/事件（5 点，r²=0.967） | **61,550** | 61,600 |

一致。一处**方法学提醒**：`get_messages` 的 5,600 只能由"成功点"拟合得到。若把两条失败点
（25,000 / 50,000，210 / 362 ms）也算进去，OLS 斜率降到 7.15 µs/条、交叉点变成 **9,090**——
因为失败路径连编码都没做完就返回，单位条目成本反而更低。04 没写它用了哪些点；
结论对"哪种会话先超预算"没影响，但读者不该以为这个 5,600 是全体拟合的结果。

### 2.4 32 MiB 硬边界

- 25,000 条：warm **0/10 成功**，`status: OutOfRange, encoded message length too large: found 44420183 bytes, limit 33554432`；
  50,000 条：**0/10 成功**，`74118580 bytes`。两条 `get_messages`（首页、深页）都是。
- 每次调用的 p50 仍有 210.1 / 361.8 ms——**失败不是"立即返回"**，它把整条消息造出来再被拒。
- 单位字节：25,000 条 **1,776.8 B/条目**；50,000 条 **1,482.4 B/条目**（拼接会话尾部分布不同，
  04 §2.3 已声明字节列不一定单调）。按这两个点分别外推，32 MiB 在 **18,885** 与 **22,636** 条处触顶。
  04 的"约 19,000"取了紧的一端——**方向正确（保守），但应当写成区间**。

## 3 归因复核：`get_state` 的 1.19 µs/条目到底是什么

这是本次复核投入最大的一项，因为最终报告把它列为 **P0-1（最安全的一条）**，而 04 §4.3
明确写着"归因未完成"。

### 3.1 代码事实（先读，再测）

`get_state` 的命令体 `agent/src/rpc/mod.rs:446` `get_state_internal` 在**没有活动 run** 时
（空闲会话，也就是用户点开会话时最常见的状态）会做两次与条目数有关的工作，两处都按
`session_formats.format != 2` 分流：

1. `:540` `let stored_metadata = sess.session_manager.state_metadata(&session_id).ok();`
   → `agent/src/session/manager/loading.rs:13` `state_metadata`：格式不是 2 就落进
   `:26` `self.load_metadata(id)` → `:68`，legacy 分支在 `:100` 执行
   `SELECT payload FROM history_shapes WHERE session_id=?1 ORDER BY position`，
   **取全部行并逐行 `serde_json::from_str`**，只为数出"user 条目几条"和取一个 parent id。
   结果在"会话已 hydrate"时**基本不被使用**（`query_count` 只在 `!history_loaded` 时用它，
   `parentSessionId` 只在内存字段为空时用它，而空的时候它取到的也是空）。
2. `:635` `.unterminated_run_id(&session_id)` → `agent/src/session/manager/mod.rs:321` →
   `agent/src/session/sqlite_store/transcript.rs:203` `read_run_marker_headers`：格式不是 2 就
   `:206` `run_marker_rows(db, session, "entry_records")`，而 **`entry_records` 是一个视图**
   （`sqlite_master.type='view'`，2,324 B SQL、五个 CTE），语句是
   `SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type IN (...)`——
   视图不可索引，SQLite 必须先把该会话的行摊出来再过滤。

两条都只对 **legacy（format≠2）** 会话生效；canonical 会话走 `entry_header_records`/`entries`
的定点读（`transcript.rs:210`）和 `entries` 计数（`loading.rs:19`）。

### 3.2 实验设计

同一个 home（`history`，含真实 14,531 条会话、两个合成 25,000/50,000 条会话、以及 321 个
canonical 会话）、同一支探针（`latency_probe`）、同一份 25 条命令清单、10 次采样 + 2 次预热
（冷值是每个命令在进程内的第 1 次调用）。四个构建：

| 变体 | 改动（本地，不提交） |
| --- | --- |
| baseline | 无 |
| **A** | `loading.rs:18` 的格式门去掉（cheap SQL 对 legacy 行同样有效：legacy 的 `entries.entry_type` 也是 `'user'`，实测该会话 195 行） |
| **B** | `transcript.rs:206` 的 legacy 分支改为返回空（只摘掉那次视图扫描） |
| **A+B** | 两者同时 |

每次运行都用同一次里 **canonical 会话的中位 p50 当"地板"** 归一化，再报 Δ = p50 − 地板，
这样机器快慢（本轮 3.85–4.86 ms 之间漂了 26%）不会冒充效应。

### 3.3 结果

Δ（ms，已扣掉本地板；每格是该变体所有轮次的中位）：

| 变体 | 地板 | Δ@41 | Δ@3,145 | Δ@14,531 | Δ@25,000 | Δ@50,000 | 斜率 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 3.85 | 0.24 | 4.42 | **17.61** | **29.76** | **59.80** | **1.19 µs/条目** |
| A（摘 `state_metadata`） | 4.23 | −0.02 | 1.33 | 5.96 | 8.91 | **19.35** | 0.39 µs/条目 |
| B（摘 run-marker 视图扫描） | 3.85 | 0.00 | 2.62 | 11.88 | 20.88 | **41.92** | 0.84 µs/条目 |
| **A+B** | 4.86 | −0.22 | **−0.14** | **−0.12** | **0.06** | **0.05** | **0.01 µs/条目** |

- 50,000 条：59.80 → A 剩 19.35（**摘掉 68%**）、B 剩 41.92（**摘掉 30%**）、A+B 剩 **0.05 ms**。
  两者可加：40.45 + 17.88 = 58.33 ≈ 59.80（重叠为 0，说明是两段互不重叠的读）。
- **A+B 之后 `get_state` 与会话长度无关**：41 条 4.18/4.64 ms、50,000 条 4.52/4.64 ms
  （两轮），与 canonical 地板 4.26–4.79 ms 同值。
- **canonical 会话本身就是平的**（同一份数据里 321 个真实 format=2 会话）：
  1 条 3.58 ms → 2,043 条 3.94 ms，跨 2,043 条只动 **0.36 ms**，且不单调。
  同规模对照：legacy 2,159 条 **6.74 ms** vs canonical 2,043 条 **3.94 ms**（+2.80 ms，
  与 baseline 斜率 × 2,159 ≈ 2.6 ms 吻合）。

### 3.4 结论

- **监督者的假设成立**：`loading.rs:13` 的 fallback 确实在每个 legacy 会话的每次 `get_state`
  上跑一遍全量 `history_shapes` 解码，且结果在已加载会话上基本被丢弃；canonical 会话不受影响、
  保持平坦——这正是"同一份数据、只差 format"的对照能证明的因果关系。
- **但它只解释 68%**。剩下 30% 是**同一类错误的第二处**：`unterminated_run_id` 对 legacy 会话
  扫 `entry_records` 视图。最终报告把根因写成"一个本该 O(1) 的元数据读退化成整会话加载"，
  应当改成**两条**；只做 P0-1 的话，50,000 条会是 **19 ms**（不是 4.5 ms），
  距 04 §4.3 给的判据（"最好是 < 15 ms 且与会话长度无关"）**不达标**。
- 修复方向（判据明确，不需要再猜）：让 `state_metadata` 与 `read_run_marker_headers`
  对 legacy 行也走同一套定点 SQL——前者用 `SELECT count(*) ... entry_type='user'` +
  `sessions.current_metadata_json`（实测 0.006–0.021 ms），后者从 `entry_header_records`
  或带索引的 marker 行读（`entry_records` 视图不可索引，是这条成本的来源）。

### 3.5 cold 没有被这两处修好（诚实的边界）

同一批实验里 cold（进程内首次调用）：

| 变体 | canonical@1 | legacy@14,531 | legacy@50,000 |
| --- | ---: | ---: | ---: |
| baseline | 19.9 | 95.2 | 173.7 |
| A+B | 24.2 | 90.9 | **127.8** |

即 A+B 只拿掉约 25%。剩下的 cold 是**首次 hydrate 该会话**（`typed_entries` 读全量转写成
`Session`），它在语义上就是 O(条目)、每个会话只付一次。所以：**温路径可以做到与会话长度无关，
冷路径不能**——要压 cold 只能压 hydrate 本身（分片 hydrate、或把首页所需的最小状态单独读出来）。
最终报告把 `get_state` 的 166 ms cold 与 24→72 ms 温路径并列成同一根因，需要拆成两条。

## 4 端到端重跑（3 个档位）

### 4.1 主结论复现

用原脚本在**同一批 home** 上重测 warm 相位（10 迭代），与 t4 原始样本逐条对比（69 条共有）：

| 档位 / label | t4 p50 | t5 p50 | 差 |
| --- | ---: | ---: | ---: |
| `history` `get_session_entries:offset:scaled50000` | 2,264.17 | 2,282.01 | +0.8% |
| `history` `get_messages:first:scaled25000`（失败） | 210.11 | 217.60 | +3.6% |
| `history` `get_events_since:snapshot:50000` | 81.02 | 80.52 | −0.6% |
| `history` `get_session_entries:before:scaled50000` | 15.79 | 15.57 | −1.3% |
| `history` `get_state:scaled50000` | 72.14 | 64.46 | −10.6% |
| `dirs-1000` `list_session_files` | 45.57 | 47.78 | +4.9% |

主结论复现。三处需要单独交代：

1. **`get_session_entries:before:scaled25000` = 45.71 ms（+192%）**——第一次看像"游标页并不平"。
   单独重测该 label 两次：**15.25 / 15.79 ms**，回到 t4 的 15.67。所以那次是**一次性异常**
   （同一时段机器上还有两个别的会话在跑任务），平坦性结论成立，不改。
2. **`threads-5000 list_sessions` = 46.03（t4 70.81，−35%）**：见 §4.2，这条命令本身是双峰的。
3. **`list_workspaces`（5,000 线程）3.88 → 1.42**：这条是 O(1) 读（回复恒定 5,459 B），
   两轮重测 1.4 左右；t4 那次偏高。它的结论（平坦）不受影响，但说明这个机器上 ±1.5 ms 的
   偏差对小命令是常态。

### 4.2 `list_sessions` 是双峰的，04 的"至少 71 ms"读错了峰

同一轮 10 次采样里，`list_sessions`（5,000 线程）的样本分成两簇：

```
rerun1: 72.4, 73.2, 42.5, 43.1, 43.2, ...      encode 稳定在 4.0–4.5 ms
rerun2: 49.4, 48.3, 42.9, 74.0, 59.4, ...      reply 恒为 152,214 B
```

两簇差约 **29 ms**，而 `encodeMs` 稳定 ⇒ 不是编码。命中 `SESSIONS_SNAPSHOT`（`catalog.rs:86`
`SNAPSHOT_REUSE = 1s`，命中路径只 clone）时约 43 ms，未命中走完整读时约 72 ms。
t4 的 8 次采样恰好都落在慢簇（p50 70.8、p95 72.4），于是 04 §2.3 写下"5,000 线程至少 71 ms"、
§1.2 写"约 0.012 ms/会话"。实测应为：**43–72 ms 双峰，斜率 0.009–0.014 ms/线程，
外推交叉点 7,100–11,000**。"至少 71 ms"作为下界不成立；作为"未命中缓存时的值"成立。

顺带把这条命令的 SQL 拆开量了（同一个 app.db，外部 sqlite）：`threads` 查询 7.03 ms、
`latest_run_infos` **8.56 ms（单条查询，4,828 行）**、`workspaces` 0.31 ms ⇒ SQL 合计 15.9 ms，
其余 27–56 ms 在装配/回复构造。所以它**不是 N+1**，§7.1 的纠正成立。

## 5 代码判定复核（02/03/04 互相依赖的四条 + 两条附加）

| 判定 | 代码证据 | 结论 |
| --- | --- | --- |
| 游标页 O(page) | `agent/src/session/history/paging.rs:191-220`：`total` 用 `MAX(ordinal)+1` 取（索引）；`before` 路径用 `ORDER BY ordinal DESC LIMIT 1 OFFSET ?` 求起点（O(页)），随后有界区间读 | **确认**；同函数里 `offset` 路径 `start = offset` 直接进 SQL（O(offset)），即 02/04 的"假分页"判定 |
| `get_tool_call_args` O(1) | `agent/src/session/tools_canonical.rs:46` 单条 `entries JOIN message_blocks ... WHERE session_id/run_id/tool_call_id` 点查 + `Snapshot::capture_from`；实测 41→50,000 条 0.55–0.71 ms | **确认** |
| 任务面板 N+1 | `business/tasks.rs:324-331` `list_tasks` 后**每个 task** 调 `task_list_view` → `store.latest_run_for_task` | **确认**（N+1 是真的；500 任务 15.9 ms） |
| `list_session_files` 成本在文件系统 | `session_files.rs:53-73`：根与每一条目 `canonicalize()`、每条目 `metadata()`、`ensure_path_allowed` | **确认**（数据库侧 3 次点查，实测目录维度 45.88 µs/项） |
| `preferSnapshot` 先整段投影 | `agent/src/rpc/commands/observability.rs:262` `run_snapshot()` 先折叠；超预算时 `business/history.rs:265-270` `Ok(None) => {}` 继续走分页读 | **确认**（含"白做一次再丢弃"） |
| 快照预算的准确阈值 | `broadcaster/projection.rs:120-128`：`bytes += data.len() + 512`，超 `limit` 置 `over_budget`；`journal.rs:150` / `broadcaster/mod.rs:53` 设 `8 * 1024 * 1024` | **补充**：每事件固定计 512 B ⇒ **≥约 16,100 事件必然超预算**（8,388,608 ÷ (512 + 本 run 的 ~9 B/事件) ≈ 16,100；payload 为空时上限也只有 16,384），与实测 10,000 正常 / 25,000 失败吻合。这条把"10,000 之后跳变"从现象变成了可计算的判据 |

## 6 file:line 抽查（12 处，全部命中）

| 出处 | 引用 | 实际 |
| --- | --- | --- |
| 02 | `agent/src/rpc/mod.rs:446` `get_state_internal` | ✅ |
| 02 | `agent/src/session/sqlite_store/transcript.rs:29` `typed_entries` | ✅ |
| 02 | `agent/src/session/history/paging.rs:501` `read_history_page` | ✅ |
| 02 | `host/src/remote_host/business/settings.rs:94` `get_state` 分支 | ✅ |
| 03 | `agent/src/skills/manager.rs:130` `catalogue()` 同步 GET `{platform_url}/client/v1/skills` | ✅ |
| 最终报告 | `agent/src/session/manager/loading.rs:13` `state_metadata` → `:68` `load_metadata` | ✅（全量 `history_shapes` 语句在 `:100`） |
| 04 | `host/src/agent_bridge/queries.rs:466` `fetch_all_session_entries_with_client`，`PAGE_SIZE = 250` | ✅ |
| 04 | `host/src/remote_host/business/history.rs:65` `get_messages` 分支 | ✅ |
| 01 | `packages/remote-client/src/client.ts:777-786` 订阅 4 类 + flush 后才发布 ready | ✅ |
| 01 | `packages/remote-client/src/client.ts:1668` `client_disconnect` 发布 | ✅ |
| 01 | `packages/thread-projection/src/historyPage.ts:29/48` `HISTORY_PAGE_USER_EXCHANGES = 3`、`HISTORY_TAIL_CURSOR` | ✅ |
| 01 | `mobile/src/remote/timeline/historyPaging.ts:26` 发 `{sessionId, before, limit: 3}` | ✅ |

## 7 差异清单（供各章订正）

按"要不要改数字"分两类。**没有一条需要改 03/04 表格里的数字**；要改的是判定、归因、列语义和措辞。

### 7.1 判定 / 归因（必须改）

1. **02 §3.12**：`get_state` 温路径"O(1)" → 改为 **O(条目)，两条 legacy 专属全量读**；
   §3.12 末尾"不把 22 ms 归因到单一语句上"可以删掉——已归因（§3）。
2. **最终报告 §0.2 表第 1 行**："`state_metadata()` → `load_metadata()` 全部行读出并逐行解码"
   应补第二个来源：`unterminated_run_id` → `read_run_marker_headers` → legacy 分支
   扫 `entry_records` **视图**。占比 68% / 30%。
3. **最终报告 P0-1 的期望值**："改回真正的 O(1)"若只改 `state_metadata`，50,000 条是 **19 ms**
   （不是 4.5 ms），不满足"与会话长度无关"。P0-1 的验收判据应写成
   "**两处都改**，50,000 条 p50 ≤ 6 ms 且 Δ(50k−41) ≤ 0.5 ms"，并附 A+B 的实测（4.5 ms）。
4. **04 §4.3**：把原因写在命令体之前（`verify_session_database` 额外 gRPC、`ensure_observer_for_session`、
   `try_get_session` 装配）→ **错**。在同一台机器上摘掉两处读之后，这些装配逻辑仍在、仍每次执行，
   而 Δ 归零（0.05 ms），说明它们与条目数无关。命令体内部的 `rpc/mod.rs:540` 与 `:635` 才是斜率来源。
5. **04 §2.3 括号 + §6 清单 1 第 6 条 + §7 第 3 个 bullet**："`list_sessions` 对每个线程查一次
   latest run / 应合并成一条按 thread_id 分组的聚合" → **已经是单条窗口函数查询**
   （`host/src/store/runs.rs:297-320`，`ROW_NUMBER() OVER (PARTITION BY thread_id ...)`）。
   02 §3.1 的"无 N+1"是对的。按这条去改代码会改到已经正确的实现上。
   （该 bullet 的**方向**仍然成立：合成 home 里每个线程只有 1 条 run，真实线程往往更多，
   窗口函数要排序的行数因而偏少；错的是"每线程一次查询"这个机制。）
6. **04 §5**："与 02 章的判定差异只有一处" → **两处**：`get_state`（02 说 O(1)）、
   `list_sessions`（02 说无 N+1，04 §2.3 说有）。
7. **最终报告 §0.1 表 `get_state` 行**："首次触达会话存储 ~110 ms + 每次调用载入整个 legacy 会话"
   → 两半应拆开：cold 是 hydrate（每个会话一次，O(条目)），温路径是那两处全量读（每次调用）。
   它们是两条独立的优化项，判据不同（见 §3.5）。

### 7.2 数字与措辞（建议改，不影响结论）

8. **04 §2.3**："偏保守的说法是 5,000 线程至少 71 ms" → 重测 43.2 / 46.0 / 49.0 ms，
   同一轮内双峰（42–49 / 72–74），"至少"不成立。建议写"未命中会话快照缓存时约 71 ms，
   命中时约 43 ms"。
9. **04 §1.2 / 清单 3**："约 0.012 ms/会话" → 0.009–0.014（双峰），交叉点 7,100–11,000。
10. **04 §1.1 / §4.2 与最终报告 §0.1b**："约 19,000 条触顶" → **18,885–22,636**；
    25,000 点 1,777 B/条、50,000 点 1,482 B/条，单位字节不是常数。"19,000"是紧端。
11. **04 表(d) 三行 `list_tasks`**：首列"数据规模"写的是该 home 的 `runsOfBusiest`（10/100/2,000），
    而命令实际的自变量是 `tasks = 33`（恒定）。三行数字（2.27/2.30/3.17）都对，
    但它读起来像"`list_tasks` 随运行次数增长"。正确曲线在表(e)：1→1.26、33→2.27、500→15.92 ms。
    建议该三行首列改标"最忙任务的运行次数（任务数固定 33）"。
12. **最终报告开头的规模**："712 会话"——`sessions` 表现有 **713 行**（392 legacy + 321 canonical）；
    03 的"355 个有内容的会话"是另一个口径。建议统一写"713 个会话行，其中 355 个有内容"。
13. **04 §5 的 `get_state` 冷值对照**（03 166.1 vs 本章 43.4）：两轮的解释（"03 的 cold 是该 home
    的第一个命令"）是对的，但 §4 的重跑显示 cold 对该现象也高度敏感于"这个进程此前有没有
    跑过别的命令"：本章 25 条命令的清单里，`get_state` 之前已跑过更小的会话，cold 只有 95 ms。
    建议把 cold 的口径写成"进程内第 1 次调用 + 此前跑过什么"。

## 8 仍然不确定的

1. **`list_sessions` 的 29 ms 双峰**：我把它归给 `SESSIONS_SNAPSHOT`（1 s 复用）命中/未命中，
   依据是两簇差值、`encodeMs` 稳定、以及 t4 全部落在慢簇。**没有插桩确认命中率**；
   如果另有原因（例如 `catalog_dirty_peek` 与发布线程交错），结论要改，数字不变。
2. **`get_state` 的 cold 主体**（A+B 之后仍 90–128 ms @14.5k/50k）：只确认了它随条目线性、
   每个会话付一次，**没有继续拆**（`typed_entries` 的视图/分页、`Session` 构造、repair 三步）。
3. **`get_state` 里剩余的 0.01 µs/条目**：A+B 后 50,000 条 Δ=0.05 ms，在噪声内，判为 0；
   更细的常数项没有继续追。
4. **一次未复现的异常**（游标页 @25,000 的 45.71 ms，重测 15.3/15.8）。当时机器上有两个
   并发会话在跑任务；我没有把并发负载做成受控变量，所以只能说"一次性"。
5. **单机、单构建、无并发控制**：所有数字来自这一台 M3 Ultra、release 构建；`get_state`
   的归因实验有 4 个变体 × 1–3 轮，其余档位多为 1 轮。

## 9 复现

```bash
# 0. 探针与 Agent（与审计同一份源码；本复核未提交任何改动）
cargo test -p future-host --release --no-run
cargo build -p future-agent --release            # target/release/future-agent

# 1. 数字复算（不调用作者的 summarize_scaling.py）
python3 target/remote-latency/verify/verify_numbers2.py   # 04 表 A：89 曲线 / 88 行
python3 target/remote-latency/verify/verify_03.py         # 03 §5：40 行五列

# 2. 端到端重跑（3 个档位；产物随后移到 target/ 下，避免污染 04 的 raw/）
python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only history    --phase warm --iterations 10 --tag t5
python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only dirs-1000  --phase warm --iterations 10 --tag t5
python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only threads-5000 --phase warm --iterations 10 --tag t5
python3 target/remote-latency/verify/compare_e2e.py       # t4 vs t5 逐条对比

# 3. `get_state` 归因：四个变体（补丁只在本地，编译前备份二进制）
#    A: 删 agent/src/session/manager/loading.rs:18 的格式门
#    B: agent/src/session/sqlite_store/transcript.rs:206 的 legacy 分支 return Ok(Vec::new())
cargo build --release -p future-agent && cp target/release/future-agent <变体名>
python3 target/remote-latency/verify/verify_probe.py \
  --home target/remote-latency/scale/homes/history \
  --specs target/remote-latency/verify/specs-getstate.json \
  --out   target/remote-latency/verify/getstate-<变体>.json \
  --agent-binary "$PWD/<变体名>" --iterations 10
python3 target/remote-latency/verify/summarize_attr.py     # §3.3 的表

# 4. 代码/SQL 侧的旁证
python3 target/remote-latency/verify/sql_list_sessions.py threads-5000   # 7.03 / 8.56 / 0.31 ms
sqlite3 -readonly target/remote-latency/scale/homes/history/.future/agent/agent.db \
  "SELECT type FROM sqlite_master WHERE name='entry_records';"            # view
```

产物（全部在 gitignored 的 `target/remote-latency/` 下，`docs/` 只增加本章）：

| 路径 | 内容 |
| --- | --- |
| `verify/verify_numbers2.py`、`verify_03.py` | §2 的离线重算器 |
| `verify/verify_probe.py`、`specs-getstate.json` | 任意命令清单 × 单 home 的驱动（§3） |
| `verify/getstate-{baseline,A,B,AB}*.json` | 四个变体 × 1–3 轮的原始样本 |
| `verify/future-agent-{baseline,patched,markers,both}` | 四个构建（baseline 与源码重建逐字节相同） |
| `verify/summarize_attr.py`、`compare_e2e.py` | §3.3 / §4 的汇总 |
| `verify/raw-t5/` | §4 的重跑原始样本（从 `docs/…/raw/scaling/` 移出，避免 04 的汇总脚本重复计行） |

复核期间对源码做的两处补丁已用 `git checkout --` 还原，`git status` 里只有本任务新增的
未跟踪文件（03/04/05 与两份脚本目录），没有工作区改动。
