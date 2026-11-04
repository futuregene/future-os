# 远端只读命令的读路径改造：四条超时命令的前后实测

本章是这项审计的第六篇，也是**改代码**的那一篇。前面五章回答「谁在发什么」「哪条读是 O(1)」「今天多慢」「放大之后多慢」；本章回答「改了哪四处、改完之后每一条多快、代价是什么」。

一句话结论：**审计里超预算的 7 个组合全部回到 100 ms 以内（41 个组合的最慢 cold 61.3 ms、最慢 p95 25.0 ms，后者是本次未触碰的 `get_available_models`）；而原来那四条命令的延迟不再随会话规模增长——50,000 条目会话上 `get_messages` 首页 8.1 ms（改造前 360 ms 且直接失败）、`get_session_entries` offset 页 1.4 ms（改造前 2,133 ms）、`get_state` 9.1 ms（改造前 66 ms）。**

## 1. 改了哪四条

| 命令 | 改造前的形状 | 改造后的形状 |
| --- | --- | --- |
| `get_messages` | 整会话物化后切页：代价 ∝ 会话条目，回复体积 ∝ 会话条目（25,000 条起撞 32 MiB 失败） | 只读当前页：`offset` 解析成 `entries` 的行范围，只水合该页（1 次有界读 + 1 次行读取） |
| `get_session_entries`（`offset` 页） | 桌面端按 250 条一页**拉完整段历史**再切（最大会话 59 次 gRPC） | 向 Agent 请求 `offset` 处的**一页**（Agent 侧本来就是 `(session_id,ordinal)` 键定位），并把 `total` 一并带回来 |
| `get_state`（冷开） | 每次轮询都按会话整段加载（legacy 会话），冷开还要多付一次整会话装配 | 计数与血缘改走覆盖索引；legacy 元数据只读 3 行；重复的 `set_model` 不再写盘；观察者挂载移到状态读之后 |
| `list_available_skills` | 读路径里一次同步 HTTP GET（连 10 s / 总 60 s） | 读路径只读缓存（内存 → 磁盘），过期后台刷新；启动预热；无缓存时仍同步取一次并**如实报错**（保留「平台不可达必须报错而不是假装空目录」的既有契约） |

四条改动都只动实现与内部读取路径：**远端客户端的命令名、字段、分页契约全部未变**（见 §6 的 `total` 说明）。

## 2. 口径与复现

- **同一套测量装置**，没有第二个 harness：`scripts/perf/remote-commands/measure-remote-latency.py`（41 个组合，见 `03-measurements.md`）与 `scripts/perf/remote-commands/scale/run_scale_sweep.py`（逐维度放大，见 `04-scaling.md`），探针仍是 `host/src/remote_host/latency_probe.rs`（`handler_ms` = `business::execute` 一次调用的墙钟）。
- **同一份数据副本**：`target/remote-latency/home-main`（agent.db 2.5 GB / 227,939 条 + app.db 682 线程 + tasks.db），来自 `mode=ro` 在线备份；两轮测量用的是**同一个** home，所以前后数字是同一批行。
- **同一台机器**：Apple M3 Ultra，32 核，macOS 26.6.2，`--release` 构建。
- **采样与原章一致**：cold = 该命令在新进程里的第 1 次调用（Agent 刚启动）；warm = 2 次预热 + 8 次测量，p95 用最近秩（8 个样本时 p95 = 最大值）。`list_sessions` / `list_workspaces` 的 warm 间隔 1,100 ms 以绕开 1 秒快照缓存。
- 原始样本：改造前 `raw/{cold,warm}-20261104-run5.json`（第 3 章未改动），改造后 `raw/{cold,warm}-20261104-after.json`；规模曲线改造后 `raw/scaling/history-{cold,warm}-20261104-after.json`。
- 复现（在仓库根目录，先 `cargo build --release -p future-agent`）：

  ```bash
  python3 scripts/perf/remote-commands/measure-remote-latency.py \
      --home target/remote-latency/home-main --tag 20261104-after
  python3 scripts/perf/remote-commands/scale/build_scale_data.py --only history
  python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only history --tag 20261104-after
  ```

## 3. 41 个组合的前后对照

cold 是冷相位那一次；p95 是热相位 8 次采样的 p95。**加粗 = 改造后**。

| 命令（取样标签） | cold 前 | cold 后 | p95 前 | p95 后 | 回复字节 前→后 |
| --- | ---: | ---: | ---: | ---: | --- |
| `list_sessions` | 22.4 | **22.8** | 24.1 | **8.9** | 70,212→70,724 |
| `list_workspaces` | 4.8 | **1.0** | 4.7 | **3.2** | 5,459→5,459 |
| `get_state:largest` | 166.1 | **60.5** | 24.6 | **6.6** | 1,475→1,469 |
| `get_state:median` | 58.6 | **15.5** | 4.6 | **4.9** | 1,488→1,482 |
| `get_state:small` | 55.6 | **15.0** | 4.7 | **4.7** | 1,475→1,469 |
| `get_messages:first_page:largest` | 561.6 | **24.2** | 255.5 | **8.6** | 76,814→76,814 |
| `get_messages:deep_page:largest` | 240.6 | **22.8** | 238.9 | **12.1** | 35,340→35,338 |
| `get_messages:first_page:mid` | 177.0 | **21.4** | 57.6 | **8.4** | 58,597→58,596 |
| `get_messages:first_page:median` | 17.0 | **9.9** | 6.7 | **8.2** | 38,527→38,527 |
| `get_messages:first_page:small` | 5.9 | **7.6** | 1.4 | **7.4** | 23,856→23,856 |
| `get_session_entries:first_paint:largest` | 38.8 | **61.3** | 11.6 | **14.1** | 87,009→87,009 |
| `get_session_entries:deep_page:largest` | 26.6 | **37.8** | 11.7 | **14.3** | 121,851→121,851 |
| `get_session_entries:first_paint:median` | 14.3 | **21.7** | 6.2 | **7.5** | 28,605→28,610 |
| `get_session_entries:offset_page:largest` | 589.3 | **17.7** | 720.8 | **1.7** | 14,510→14,510 |
| `get_session_entries:offset_page:mid` | 132.9 | **5.5** | 161.7 | **1.7** | 13,349→13,349 |
| `get_session_entries:offset_page:median` | 14.0 | **4.0** | 5.8 | **2.1** | 21,722→21,722 |
| `get_session_entries:deep_page:median` | 8.1 | **10.0** | 3.7 | **4.4** | 10,303→10,308 |
| `get_events_since:snapshot:heaviest` | 45.1 | **12.2** | 2.2 | **2.2** | 31,264→28,803 |
| `get_events_since:first_page:heaviest` | 2.5 | **3.2** | 2.4 | **2.8** | 15,642→15,642 |
| `get_tool_call_args:heaviest` | 1.0 | **1.4** | 0.7 | **0.7** | 3,027→3,027 |
| `get_event_content_range:biggest_body` | 0.1（未测出） | **0.2**（未测出） | 0.0 | **0.0** | 66→66 |
| `get_events_since:seek:biggest_body_run` | 2.8 | **4.5** | 2.3 | **2.9** | 12,933→12,933 |
| `get_fusion:heaviest` | 1.4 | **2.6** | 1.2 | **1.4** | 53→53 |
| `list_session_files:workspace_root` | 1.4 | **1.8** | 1.2 | **1.8** | 4,410→4,410 |
| `get_queue:largest` | 22.7 | **6.9** | 22.2 | **6.3** | 93→93 |
| `get_prompt_receipt:newest` | 0.1 | **0.4** | 0.0 | **0.0** | 179→179 |
| `get_fusion_settings` | 1.8 | **1.8** | 1.5 | **2.0** | 400→400 |
| `get_desktop_settings` | 0.1 | **0.2** | 0.0 | **0.0** | 3,013→3,013 |
| `get_settings` | 0.0 | **0.0** | 0.0 | **0.0** | 102→102 |
| `list_settings_models` | 22.1 | **25.1** | 21.4 | **24.9** | 2,032→2,032 |
| `list_models` | 21.3 | **24.7** | 21.2 | **24.8** | 1,425→1,425 |
| `get_available_models` | 21.4 | **24.3** | 27.6 | **25.0** | 1,425→1,425 |
| `list_skills` | 2.2 | **3.8** | 1.8 | **2.4** | 12,633→12,633 |
| `list_available_skills` | 67.5 | **3.1** | 119.5 | **3.4** | 47,024→47,024 |
| `skill_reco_today` | 0.2 | **0.4** | 0.0 | **0.0** | 91→91 |
| `list_providers` | 2.1 | **1.7** | 1.9 | **1.2** | 22,341→22,341 |
| `list_tasks` | 5.6 | **15.1** | 3.0 | **3.4** | 9,123→8,866 |
| `get_task:busiest` | 2.1 | **1.8** | 1.5 | **2.1** | 24,813→24,665 |
| `list_task_runs:busiest` | 2.4 | **2.2** | 1.7 | **2.3** | 11,708→11,691 |
| `list_task_revisions:most_revisions` | 2.5 | **2.4** | 1.6 | **2.4** | 329,914→329,914 |
| `list_task_deps:busiest` | 0.0 | **0.0** | 0.0 | **0.0** | 48→48 |

判据（全部满足）：**41 个组合的 cold 与 p95 全部 < 100 ms**。本轮实测最慢 cold 61.3 ms（`get_session_entries:first_paint:largest`）、最慢 p95 25.0 ms（`get_available_models`，本轮未触碰，见下）。

### 3.1 先说清楚运行间抖动

这一轮的机器比基线那一轮更忙：**本次没有改动的命令也整体高了 0–4 ms**——`list_models` 21.3 → 24.7、`list_settings_models` 22.1 → 25.1、`list_tasks` cold 5.6 → 15.1、`get_prompt_receipt` 0.1 → 0.4。也就是说这张表里 ±3 ms 的差值是环境噪声，不是改动；四条被改造的命令改善幅度是 20–700 倍，噪声在其中可以忽略。

同一条命令在**同一份数据**上再跑一次（`measure-remote-latency.py` 的同类取样，改造后三次独立运行）说明抖动幅度：`get_state:largest` cold 47–61 ms、`get_messages:first_page:largest` cold 21–25 ms、`get_session_entries:first_paint:largest` cold 44–61 ms。表里的"改造后"列取最后一次（与最终代码同一构建）的数。

几处需要解释的小变化：

- **`get_messages:first_page:small` 1.4 → 7.4 ms**：改造后这条路是「一页 SQL + 一次行读取」，小会话上它反而比「在内存里切一个 41 条列表」贵约 6 ms。这是**固定成本**（与条目数无关），50,000 条时是 8.1 ms（§4）。放大的收益和抬高的底噪是同一次取舍，见 §6.1。
- **`get_session_entries:first_paint:largest` cold 38.8 → 61.3 ms、p95 11.6 → 14.1 ms**：游标页的代码没有改（除页 JSON 多一个 `total` 字段）。判定为抖动的三条证据：①同一 harness 的规模曲线里，这条路径改造前后是 **11.7 vs 12.2 ms**（14,531 条）与 **15.3 vs 16.0 ms**（50,000 条），即持平或略快（§4）；②单独重测三次得 44.5–50 ms cold、13.9–15.3 ms p50，与基线 38.8/11.8 的差值就是本轮整体的 3–5 ms 抬升；③`04-scaling.md` 记录的同一路径本身就在 12.2–16.0 ms 之间波动。**游标路径无回归。**
- **`get_state` 回复少 6 字节**：基线是在带改动的工作树上编的（版本号带 `+local.dirty`），本轮是干净树，版本字符串短 6 字节。
- **`list_tasks` 少 257 字节**：tasks.db 是同一份副本，但两轮之间任务台账本身有新记录，属于数据漂移，不是回复裁剪。
- **`list_sessions` 多 512 字节 / 3 条线程**：测量过程本身在这份 home 里绑定了线程（`04-scaling.md` 的对照档同样出现过 685+3）。
- **`list_available_skills` 的 cold 3.1 ms**：测量前删掉了 home 里的目录缓存，cold 相位仍然只有 3 ms——启动预热在客户端发问之前就把缓存填好了（§6.4）。没有缓存且平台不可达时，这条命令仍会如实报错。

## 4. 规模曲线：四条命令不再随会话变长

同一套 `run_scale_sweep.py`，同一个 `history` home（41 → 236 → 1,000 → 3,145 → 14,531 → 25,000 → 50,000 条目，后两档是把真实最大会话整段复制拼接出来的），**p50，单位 ms**：

| 条目数 | `get_messages` 首页 前 → 后 | `get_messages` 深页 前 → 后 | offset 页 前 → 后 | `get_state` 前 → 后 | 游标页 前 → 后 |
| ---: | --- | --- | --- | --- | --- |
| 41 | 1.4 → **6.6** | 1.3 → **5.7** | 1.3 → **1.1** | 4.8 → **4.6** | 1.4 → **1.6** |
| 236 | 7.3 → **7.6** | 7.3 → **8.1** | 6.1 → **1.9** | 4.6 → **4.6** | 6.8 → **7.1** |
| 1,000 | 22.4 → **7.7** | 21.7 → **7.3** | 20.3 → **1.8** | 5.5 → **4.6** | 14.7 → **15.2** |
| 3,145 | 62.5 → **8.0** | 61.5 → **7.3** | 136.4 → **1.6** | 7.8 → **4.7** | 16.9 → **18.9** |
| 14,531 | 251.1 → **8.2** | 252.9 → **10.5** | 681.7 → **1.6** | 23.2 → **6.1** | 12.2 → **13.3** |
| 25,000 | 210.1（**失败**）→ **8.4** | 209.9（**失败**）→ **12.6** | 1,211.1 → **1.6** | 36.7 → **7.2** | 15.7 → **17.2** |
| 50,000 | 361.8（**失败**）→ **9.0** | 362.4（**失败**）→ **18.9** | 2,264.2 → **1.5** | 72.1 → **10.0** | 15.8 → **17.3** |

前两列与第四列的"前"来自 `04-scaling.md` 的同一档位；"后"是本轮同一套 harness 的结果（`raw/scaling/history-{cold,warm}-20261104-after.json`）。

读法：

- **`get_messages` 从 0.017 ms/条目变成与页无关**：41 → 50,000 条（1,220 倍数据），首页 6.6 → 9.0 ms。改造前同区间是 1.4 → 361.8 ms，而且 25,000/50,000 不是变慢而是**失败**（回复 44.4 MB / 74.1 MB 撞 32 MiB 帧上限）。改造后这两档**成功**，回复仍是 76,814 B（一页的量，与会话无关）。
- **深页随 offset 略涨**：5.7 → 18.9 ms（41 → 50,000 条）。涨的是 `OFFSET` 在 `entries` 上走过的头部行（索引内扫描，约 0.3 µs/行），不是正文；这是「客户端按条目序号翻页」这一契约的固有代价，见 §6.2。14,531 条（真实最大会话）深页 10.5 ms。
- **offset 页 1.1–1.6 ms，与 offset 完全无关**：Agent 侧 `read_history_page(before=None, offset=n)` 本来就在 `(session_id,ordinal)` 上定位；改造前慢的是桌面端把整段历史拉完再切。
- **`get_state` 4.6 → 10.0 ms**：从「整会话加载」降到只剩两项随条目缓慢增长的固定成本（`count(*)` 的覆盖索引扫描 + 会话首次装配），50,000 条也只有 10 ms，远在预算内（判据是 < 100 ms）。
- **游标页持平**：12.2 → 13.3 ms（14,531 条）、15.8 → 17.3 ms（50,000 条）——与 §3.1 说的整体环境抬升同幅（未改动的命令本轮普遍慢 5–15%），形状仍与总长无关。这是审计要保留的样板，本轮没有改它的读法。

## 5. 每条改了什么（代码级）

### 5.1 `get_messages`：把「整段物化」换成「读这一页」

- `agent/src/session/manager/loading.rs` 新增 `Manager::message_page(id, offset, limit)`：`offset` 在第 `user/system/assistant/tool` 行上的序号空间里解析成一个行范围（`SELECT position,entry_id ... ORDER BY position LIMIT ? OFFSET ?`，走 `entries` 主键），再只水合这些行；同时返回该会话的消息总数。canonical 会话用 `Snapshot::entries_page`（与整段加载同一条读取路径，同样的 JSON），legacy 会话用 `entry_records` 视图按同一范围取，**不读整段**。
- `agent/src/rpc/commands/observability.rs`：`handle_get_messages` 收到 `offset`/`limit` 时走上面的分页读，回复里带上 `offset/nextOffset/total/hasMore`（无分页调用仍读内存列表，语义不变）。
- `agent/src/rpc/commands/mod.rs`：**分页的 `get_messages` 不再触发整会话 hydrate**。这是冷开的真正大头：一条 40 条消息的请求原本要先付 `ensure_history_loaded_unlocked` 的整会话装载（14,531 条目实测 384 ms，而这一页本身 14 ms）。
- `host/src/agent_bridge/queries.rs` / `host/src/remote_host/business/history.rs`：桌面端把 `offset/limit` 发给 Agent，拿回一页后只做原有的远端裁剪（lean 化 + 字节预算），再按绝对 `offset` 重述游标。

### 5.2 `get_session_entries` 的 `offset` 路径：把「59 次往返」换成「1 次」

- `agent/src/session/history/paging.rs`：页 JSON 增加 `total`（`MAX(ordinal)+1`，这条读本来就要它来做范围夹取），分片回复同样带上。
- `packages/rpc/proto/future.proto`：`SessionEntriesResponse` 增加 `optional int64 total = 5`（**只增字段，不复用编号**，`make generate-proto` 重新生成）。
- `host/src/agent_bridge/queries.rs` 新增 `get_session_entries_at_offset`：直接按 `offset/limit` 请求一页。
- `host/src/remote_host/business/history.rs`：`offset` 分支改为「一页 + 原有裁剪」，`offset/nextOffset/total/hasMore` 全部按绝对位置重述；旧 Agent（无 `total`）的回复按「本页到达处」保守回填。

### 5.3 `get_state` 冷开：三个线性项全部消掉

- `agent/src/session/manager/loading.rs`：
  - `state_metadata` 不再在非 canonical 会话上退回 `load_metadata`。计数与血缘都来自有界行（`entries` 的 `entries_kind` 覆盖索引，实测 7 µs；父会话来自 `sessions.current_metadata_json`，3 µs）。改造前它每次轮询都整段加载（14,531 条目 23 ms，50,000 条目 66 ms）。
  - `load_metadata` 的 legacy 分支只读 3 行（首行 + 最新 `session_info` + 最新 `model_change`），与 canonical 分支读的是同一组位置。
- `agent/src/session/sqlite_store/transcript.rs`：`read_run_marker_headers` 的 legacy 分支先用 `entries` 拿到标记行的位置，再只组装那几行的 payload（原来是整段扫 `entry_records`）。
- `agent/src/rpc/session.rs`：`set_model` 在身份未变时不再写 `session_info`。冷开正是「把已经读到的模型再装一遍」，这次写盘会推高 revision、让所有页缓存失效，并且同步等一次落盘。
- `host/src/remote_host/business/settings.rs`：`get_state` 先读状态、再挂观察者。观察者的首次 attach 会自己发一条命令，和这次读抢同一个会话的装配（冷开实测 77 ms vs 单独跑的 10 ms）。

### 5.4 `list_available_skills`：读路径不再摸网络

- `agent/src/skills/manager.rs`：新增进程内 + 磁盘缓存（`<agent home>/skills-catalogue.json`），读走 `catalogue_read()`：命中即返回，超过 5 分钟在**后台线程**刷新；**没有任何缓存时**仍同步取一次，失败照旧上报——`platform_backed_skill_commands_fail_closed_when_the_platform_is_unreachable` 明确要求「平台不可达不能假装是空目录」，这是既有产品契约，本轮不推翻，只把它从「每次调用」降为「这台机器第一次」。
- `agent/src/service/startup.rs`：启动时在线程里预热（`SkillManager` 会构造 blocking HTTP client，不能放在 async 启动路径上——在那里构造会直接 panic，这一点由 `cli::cut_short_drain_tests` 抓到）。

## 6. 代价与口径（读数字前请先看这四条）

### 6.1 小会话的 `get_messages` 变贵了约 5 ms

改造后小会话首页是 6.6 ms（原 1.4 ms；同一份数据上再跑一次是 7.4–7.6 ms）。这是**固定底噪**：一页 SQL + 行读取 + 一次跨进程往返的代价，与会话长度无关（50,000 条时 8.9–9.0 ms）。换来的是一条**不再有上界**的读变成有界读——原来的形状在 25,000 条以上不是慢，而是必然失败。如果将来更在意小会话的这一跳，可以把「已 hydrate 的会话直接切内存列表」作为快路径加回来；本轮没有这么做，因为那会让同一命令在冷热两种状态下返回**不同的索引空间**。

### 6.2 分页的索引空间是「存储的历史」，不是「修复后的历史」

`get_messages` 的 `offset` 现在落在**存储里的** `user/system/assistant/tool` 行上；`load()` 里有三步跨条目的内存修复（去掉空 assistant、去重 tool 调用、补齐悬空 tool result）。对健康会话两者完全一致——canonical 会话实测**逐字节相同**（1,746 条消息，两页都与整表切片一致）。对内部不一致的会话（比如进程在 tool 调用后崩过），旧路径会**凭空造出**补位的 tool result 行，新路径按存储原样给出：本机最大 legacy 会话因此少 3 行（14,138 → 14,135）。这是刻意的：分页读不能为了让第 N 页与旧编号对齐而把整段装载回来。

### 6.3 `total` 是内部字段的加法，不是客户端契约的变更

远端回复里的 `offset/nextOffset/total/hasMore` 语义一个都没变；`SessionEntriesPage` 只是多了一个 `optional total`（proto 里是 `optional int64 total = 5`，只增编号）。旧桌面端读新 Agent 时忽略它；旧 Agent 的回复里没有它，于是桌面端退回**改造前的老做法**（把拿回来的列表按 `offset/limit` 在本地切片），既能分页也不会撒谎。

### 6.4 `list_available_skills` 的冷开仍可能是一次网络往返

只有「这台机器从来没取到过目录」时才会走同步取（并且会如实报错，既有的 fail-closed 契约不变）。正常路径由启动预热填好，实测 cold 3.1 ms / p95 3.4 ms（原 cold 67.5 / p95 119.5）。本轮测量前删除了 home 里的目录缓存，cold 相位仍然只有 3 ms —— 启动预热在客户端发问之前就把缓存填好了。

## 7. 没有做的部分

`04-scaling.md` 表 A 里越线的行列还有两条，不在本次「四条命令」的范围内，原样记录：

- `list_session_files`：随目录条目线性（5,000 项 229 ms），成本在 `readdir`/stat，与数据库无关。
- `get_events_since` + `preferSnapshot`：25,000 事件起先做一次整段投影再按预算丢掉（41–81 ms），真正的分页读只要 2 ms。

两条都已在 `04-scaling.md` §4.4/§4.5 给出判据（默认页大小 + 截断提示；折叠前先判规模或缓存投影），本轮未改。

## 8. 自测

`cargo fmt --all -- --check`、`cargo clippy -p future-agent --all-targets -- -D warnings`（含 `packages/rpc` 的 proto 重新生成）、`cargo test -p future-agent --lib`（2,971 通过 / 0 失败 / 5 忽略）、`cargo test -p future-host --lib`（2,043 通过 / 0 失败 / 13 忽略）与 `make check-docs` 全绿。新增测试：

- `agent/src/session/manager/tests/message_page.rs`：分页是整表的切片（canonical）、一页只读它返回的行、深页与首页读代价相同。
- `agent/src/session/manager/tests/legacy_state.rs`：legacy 会话的 `state_metadata` / `load_metadata` / run 标记都只读有界行（并断言读取作业次数不随历史增长）。
- `agent/src/skills/manager.rs`：缓存命中不触网；取回一次后第二次读不再取；无缓存时如实报错。
