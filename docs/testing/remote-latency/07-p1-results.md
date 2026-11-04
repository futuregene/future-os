# P1 验收：`list_session_files` 分页化 与 `preferSnapshot` 预判（07）

- 日期：2026-11-04
- 分支 / 工作树：`perf/remote-p1`（`.worktrees/perf-p1`），HEAD `32f0704e8`，三处改动均**未提交**：
  `host/src/remote_host/session_files.rs`、`agent/src/rpc/commands/observability.rs`、
  `agent/src/session/sqlite_store/events.rs`
- 机器：Apple M3 Ultra / 32 核 / 96 GiB，macOS 26.6.2（Darwin 25.6.0），release 编译
- 数据源：本机 `~/.future` 的**只读在线备份**（`mode=ro` + `sqlite3.backup`），
  `237,852` 条目 / `2,504` 个 run / `709` 个线程 / `355` 个有内容会话
- 原始样本：`docs/testing/remote-latency/raw/p1/`（13 个文件）
- 范围：P1-3、P1-4。**P1-5（`get_queue`）按计划延后，不测、不评判。**

---

## 0 结论摘要

| 项 | 判据 | 实测 | 判定 |
| --- | --- | ---: | --- |
| P1-3 `list_session_files`，5,000 项目录，**p50 ≤ 20 ms** | §5 P1-3 | **11.82 ms**（p95 13.16） | **达标**（原 229.39 ms，19.4×） |
| P1-3 附带：回复仍带全部 5,000 项 | 老客户端不受影响 | 是（回复 955,336 B ≈ 191 B/项；默认页 200 项只有 ≈40 KB） | **达标** |
| P1-3 §5 原始判据后半：10 项与 5,000 项耗时比 ≤ 3 | §5 P1-3 | 0.149 ms vs 11.82 ms = **79×** | **不达标**（排序仍要 `readdir` 整个目录，模块文档已声明） |
| P1-4 `preferSnapshot`，**p95 ≤ 5 ms** | §5 P1-4 / §6.1#3 | 被门控的那 1 个 run：**11.47 ms**（p50 9.58；另一次 12.64 / 12.39） | **不达标**（350 ms → 11.5 ms，但没到 5 ms） |
| P1-4 在审计自己的 50,000 事件档 | §6.1#3 | **92.58 ms**（BEFORE 81.02 / 89.83） | **未改善**：该档是旧格式 run，门控按设计不判 |
| (c) 不许回退：游标历史页 / `get_events_since` 无快照 / `get_tool_call_args` / 任务面板 / 设置读取 | §6.1#5 | 全部持平，绝对增量 ≤ 1.44 ms，形状未变 | **达标** |
| 问题一：门控会不会把"以前给快照"的 run 判成 `too_large` | — | 快照里 1,496 个新格式 run，只有 1 个被门控拒绝，且它的 **行数一项** 就已越过旧预算 | **没有误判** |
| 问题二：`list_session_files` 在 50,000 项目录的代价 | — | **144.91 ms**（p95 149.32，回复 9.65 MB） | 仍越过 100 ms |

两句话：**P1-3 达标。P1-4 只修好了 >8 MiB 会计量那一档（快照里只有 1 个 run）；13k–26k 行那一档——真实快照 24 个最大 run 里有 13 个——仍旧每次打开白折 198–573 ms 才被旧逻辑拒掉。**

---

## 1 方法（以及两处必须说明的偏差）

完全走审计自己的测量栈，没有另写测量代码：

1. `scripts/perf/remote-commands/scale/build_scale_data.py` 用审计第 03 章的 `build_home`（`mode=ro` 源 + SQLite 在线备份）建一个共享快照 `base`，再派生单维度 home：
   - `dirs-10` / `dirs-5000` / `dirs-50000`（只放 `app.db`、`tasks.db`，不复制 2.5 GB 的 `agent.db`；workspace 指向该 home 内的 `workspace-roots/files-N`）；
   - `history`（真实 `agent.db` 副本 + 3 个克隆会话 + 5 个放大 run：100 / 1,000 / 10,000 / 25,000 / 50,000 事件）。
2. `scripts/perf/remote-commands/scale/run_scale_sweep.py` 与 `measure-remote-latency.py` 驱动
   `host/src/remote_host/latency_probe.rs`：每个 home 一个隔离 agent，`HOME` 指向带
   `MEASUREMENT_ISOLATED_HOME` 标记的一次性目录，`--grpc-addr 127.0.0.1:<随机空闲端口>`，
   **不碰 50051、不杀正在跑的 agent、不写线上库**。
3. 编译本工作树自己的二进制：`cargo build -p future-agent --release`（14:34）与
   `cargo test -p future-host --release --no-run`（14:35，探针在 host 测试目标里）。

### 1.1 偏差一：探针初始化 app 库失败，需要对**副本**补一列

`host/src/store/schema.rs` 的 `ADDED_COLUMNS` 只在**版本迁移**时执行，而线上
`~/.future/app/app.db` 的 `user_version` 已经是 `APP_DB_SCHEMA_VERSION=4`，于是
`apply_schema` 跳过所有 `ALTER`，`validate_core` 随后报
`Desktop database is incomplete: required workspaces.archived_at is missing`。

这是本分支（#1488 把 `workspaces.archived_at` 加进 `ADDED_COLUMNS`）与线上 app 库之间的**降级/升级错位**，与本次两条改动无关，也不在本任务的写集里（不修，只报告，见 §7.1）。为了让探针能跑，我用脚本把 `ADDED_COLUMNS` 里缺的列补到**一次性 home 的 `app.db` 副本**上（实际只差 `workspaces.archived_at`，正是下一次迁移会做的事）。线上库始终 `mode=ro`。

### 1.2 偏差二：新格式 run 的样本要自己组命令单

审计第 04 章的 home 只能造**旧格式** run（`session_formats` 无行 ⇒ 默认 format=1 ⇒ 回放读 `run_events`；克隆会话也不复制 `stream_changes`）。而 P1-4 的门控**按设计只判新格式（分段）run**。所以：

- 审计 home 上的 before/after 直接沿用既有 label；
- 真实快照里最大的新格式 run，用同一个探针、同一份 JSON 命令单契约，另组两份命令单
  （`p1-canonical.json`、`p1-verdict.json`）驱动。命令单是**数据**，探针代码一行未改。

### 1.3 口径

- `cold`：新进程里该命令的第一次调用；`warm`：2 次预热 + N 次采样（本报告 N=8~12）。
- `p50/p95` 是最近秩：10 个样本的 p95 就是最大值，不要当分位数读。
- `handler_ms` 是 `business::execute` 的墙钟（app 库读 + Agent 往返 + 页面裁剪 + JSON 构造），
  不含传输发送与客户端渲染；`jsonBytes` 是回复序列化后的大小，`replyBytes` 是 gzip 后。

---

## 2 BEFORE / AFTER

**BEFORE 全部来自仓库里已记录的 raw 样本**（不是我这次跑的）：`raw/scaling/dirs-5000-*-20261104-t4.json`
（P1-3 基线，08:3x~09:23）与 `raw/scaling/history-*-20261104-t4.json` + `history-*-20261104-after.json`
（P1-4 基线，09:2x / 11:15，后者是 P0 落地后、P1-4 落地前）。**AFTER 全部是我这一轮自己跑的**，样本在 `raw/p1/`。

### 2.1 `list_session_files`（dirs home，warm）

| 目录规模 | BEFORE p50 / p95 / cold | AFTER p50 / p95 / cold | 回复（json / gzip） |
| ---: | ---: | ---: | ---: |
| 10 项 | 无对照样本 | 0.15 / 0.24 / 0.36 ms | 2,198 B / 1.1 KB |
| 5,000 项 | **229.39 / 269.07 / 230.45 ms** | **11.82 / 13.16 / 15.07 ms** | 955,336 B / 49,034 B（BEFORE 1,005,322 B / 49,035 B） |
| 50,000 项 | 无对照样本（第 04 章只扫到 5,000） | **144.91 / 149.32 / 157.39 ms** | 9,650,340 B / 474,531 B |

- 5,000 项的回复体积从 1,005,322 B 变成 955,336 B，差 49,986 B：两个 home 的 workspace 根路径差 10 个
  字符（`perf-remote-scale` → `perf-p1`），5,000 条 `path` 就是 50,000 B，余下 14 B 是 `rootPath`/`path`
  两个字段的同名差异。**这说明项数没变、仍是全量 5,000 项**（若被截成默认页 200 项，回复只有 ≈40 KB）。
  命令面仍走 `Span::All` 这一行由
  `session_files.rs::a_directory_above_the_page_size_is_not_truncated_through_list` 钉住。
- 冷态 230.45 → 15.07 ms，说明原来的 0.046 ms/项里大部分是每项一次 `canonicalize`。

### 2.2 `preferSnapshot`（审计的 history home，**旧格式** run）

| run 事件数 | BEFORE(t4) p50 | BEFORE(after) p50 | AFTER p50 / p95 | 结果 |
| ---: | ---: | ---: | ---: | --- |
| 100 | 0.43 | 0.55 | 0.69 / 0.83 | 给快照（10,248 B） |
| 1,000 | 0.69 | 0.86 | 1.13 / 1.26 | 给快照（11,878 B） |
| 10,000 | 1.29 | 1.53 | 1.73 / 2.00 | 给快照（26,969 B） |
| 25,000 | 41.45 | 47.22 | **50.14 / 51.09** | 拒绝 → 回落分页（14,702 B） |
| 50,000 | 81.02 | 89.83 | **92.58 / 94.34** | 拒绝 → 回落分页（14,702 B） |

**这一档没有任何改善，因为门控不判旧格式 session**：`run_replay_volume` 先查
`content::format`，非 `FORMAT_VERSION` 直接返回 `None`（旧格式回放没有字节预算，体积判不了）。
旧格式的 50,000 事件折完再拒的 92.58 ms 依然全额付。

### 2.3 `preferSnapshot`（真实快照里的**新格式** run，按体积排序前 24 个）

这一档没有 BEFORE 样本（第 04 章造不出新格式 run），所以我同时测了同 run 的
**不限量分页回放**作为门控前代价的下界代理（`replay_proxy`）；另给每个 run 配一条
"普通首页"控制，用**回复字节是否相同**判定它是"真给了快照"还是"回落成首页"。

| run（行数 / 会计量） | 门控 | p50 / p95 | 回复 | 判定 |
| --- | --- | ---: | ---: | --- |
| 32,195 / **11.22 MiB** | **拒绝** | **12.39 / 12.64** | 41,762 B（= 首页） | 拒绝→回落（门控拦下） |
| 19,340 / 6.64 MiB | 放行 | 393.11 / 393.13 | 42,374 B（= 首页） | 拒绝→回落（折完才拒） |
| 18,160 / 6.50 MiB | 放行 | 229.92 / 234.44 | 58,170 B（= 首页） | 拒绝→回落 |
| 17,910 / 6.25 MiB | 放行 | 572.60 / 575.81 | 54,238 B（= 首页） | 拒绝→回落 |
| 17,291 / 6.10 MiB | 放行 | 374.25 / 381.50 | 28,591 B（= 首页） | 拒绝→回落 |
| 16,446 / 5.76 MiB | 放行 | 197.82 / 198.13 | 59,090 B（= 首页） | 拒绝→回落 |
| 15,380 / 5.31 MiB | 放行 | 477.83 / 481.46 | 48,342 B（= 首页） | 拒绝→回落 |
| 15,279 / 5.23 MiB | 放行 | 456.49 / 475.48 | 50,756 B（= 首页） | 拒绝→回落 |
| 14,443 / 5.11 MiB | 放行 | 321.25 / 328.04 | 63,968 B（= 首页） | 拒绝→回落 |
| 14,343 / 5.09 MiB | 放行 | 250.93 / 257.94 | 44,537 B（= 首页） | 拒绝→回落 |
| 14,224 / 4.89 MiB | 放行 | 405.41 / 407.45 | 36,771 B（= 首页） | 拒绝→回落 |
| 14,079 / 4.88 MiB | 放行 | 204.56 / 205.03 | 2,490 B（= 首页） | 拒绝→回落 |
| 13,565 / 5.11 MiB | 放行 | 232.20 / 249.26 | 72,823 B（= 首页） | 拒绝→回落 |
| 12,624 / 4.63 MiB | 放行 | 272.26 / 274.70 | 56,319 B（= 首页） | 拒绝→回落 |
| 12,796 / 4.45 MiB | 放行 | 339.87 / 344.02 | 1,244,299 B | **给快照** |
| 12,702 / 4.32 MiB | 放行 | 400.99 / 407.96 | 1,031,363 B | **给快照** |
| 12,393 / 4.14 MiB | 放行 | 752.64 / 769.94 | 639,060 B | **给快照** |
| 12,367 / 4.18 MiB | 放行 | 579.33 / 587.60 | 1,302,804 B | **给快照** |
| 12,295 / 4.38 MiB | 放行 | 248.44 / 250.10 | 1,961,221 B | **给快照** |
| 12,143 / 4.43 MiB | 放行 | 249.39 / 251.85 | 2,041,380 B | **给快照** |
| 12,087 / 4.24 MiB | 放行 | 460.82 / 469.01 | 1,116,280 B | **给快照** |
| 12,075 / 4.37 MiB | 放行 | 347.42 / 353.60 | 1,758,125 B | **给快照** |
| 11,987 / 4.37 MiB | 放行 | 373.53 / 379.51 | 1,731,347 B | **给快照** |
| 11,424 / 4.11 MiB | 放行 | 340.58 / 360.63 | 1,814,866 B | **给快照** |

同 run 的对照（`raw/p1/p1-canonical.json`）：

| 命令 | p50 / p95 | 说明 |
| --- | ---: | --- |
| `preferSnapshot`（32,195 行） | **9.58 / 11.47** | 12 次采样；与上表 12.39 / 12.64 是同一条命令的两次独立测量 |
| 普通首页（同 run，limit 200） | 5.45 / 5.49 | 门控拒绝后客户端实际拿到的东西 |
| 不限量分页回放（同 run，limit 1e6） | 349.79 / 356.07 | 门控**之前**每次打开都要付的回放代价 |

由此：**门控把这一档从 ≈350 ms 降到 ≈11.5 ms（约 30×），但 5 ms 判据没达到**，残余是
门控自身 4~6.5 ms（一次 32,195 行的 `count(*) + sum(byte_length)` 聚合，O(行) 但每行只要
≈0.15 µs）+ 回落首页 5.4~5.9 ms。

### 2.4 (c) 不许回退：新客户端"唯一做对"的那几条（真实快照，warm）

BEFORE 用 `raw/warm-20261104-after.json`（同一台机器、同一锚点：最大会话 14,531 条、
最重 run 10,855 事件、同一个 thread 都一致；只是总量从 229,631 条目 / 687 线程涨到
237,852 / 709）。

| 命令 | BEFORE p50 / p95 | AFTER p50 / p95 | Δp50 |
| --- | ---: | ---: | ---: |
| **游标历史页** | | | |
| `get_session_entries:first_paint:largest` | 13.97 / 14.14 | 14.84 / 14.94 | +0.87 ms |
| `get_session_entries:deep_page:largest` | 13.82 / 14.28 | 14.80 / 17.04 | +0.97 ms |
| `get_session_entries:first_paint:median` | 7.18 / 7.49 | 8.21 / 9.00 | +1.03 ms |
| `get_session_entries:deep_page:median` | 4.21 / 4.40 | 4.83 / 5.00 | +0.62 ms |
| **`get_events_since`（不带 preferSnapshot）** | | | |
| `get_events_since:first_page:heaviest` | 2.54 / 2.79 | 2.83 / 2.96 | +0.29 ms |
| `get_events_since:seek:biggest_body_run` | 2.63 / 2.87 | 2.83 / 3.12 | +0.20 ms |
| **`get_tool_call_args`** | | | |
| `get_tool_call_args:heaviest` | 0.63 / 0.72 | 0.82 / 1.06 | +0.19 ms |
| **任务面板** | | | |
| `list_tasks` | 3.06 / 3.41 | 3.10 / 3.50 | +0.04 ms |
| `get_task:busiest` | 1.80 / 2.06 | 2.06 / 4.28 | +0.26 ms |
| `list_task_runs:busiest` | 2.12 / 2.30 | 2.17 / 2.53 | +0.04 ms |
| `list_task_revisions:most_revisions` | 2.13 / 2.44 | 2.43 / 2.71 | +0.30 ms |
| `list_task_deps:busiest` | 0.00 / 0.00 | 0.00 / 0.00 | +0.00 ms |
| **设置读取** | | | |
| `get_settings` | 0.04 / 0.04 | 0.04 / 0.04 | +0.00 ms |
| `get_desktop_settings` | 0.04 / 0.05 | 0.04 / 0.10 | +0.00 ms |
| `list_settings_models` | 24.63 / 24.91 | 25.96 / 26.43 | +1.33 ms |
| `list_models` | 24.60 / 24.80 | 26.04 / 26.32 | +1.44 ms |
| `get_available_models` | 24.67 / 25.04 | 26.02 / 26.35 | +1.36 ms |
| `list_skills` | 2.11 / 2.39 | 2.52 / 2.82 | +0.41 ms |
| `list_available_skills` | 3.01 / 3.40 | 3.51 / 3.76 | +0.50 ms |
| `get_fusion_settings` | 1.65 / 1.96 | 2.45 / 2.82 | +0.81 ms |
| `skill_reco_today` | 0.01 / 0.01 | 0.01 / 0.01 | +0.00 ms |
| `get_prompt_receipt:newest` | 0.03 / 0.04 | 0.03 / 0.04 | +0.00 ms |
| `list_providers` | 1.17 / 1.23 | 1.41 / 1.57 | +0.23 ms |
| `list_workspaces` | 0.94 / 3.23 | 0.95 / 1.07 | +0.01 ms |

24 行的相对增量中位数 **+7.1%**（最小 0.0%，最大 +49%）；绝对增量全部 ≤ 1.44 ms。
**没有任何一条改变形状**（不随规模变化），而且完全没被这两处改动碰到的命令
（`get_settings`、`skill_reco_today`、`list_task_deps`）增量为 0，说明这点整体上浮来自
环境/快照漂移（同一台机器上我这一轮本身在跑满载的编译与备份），不是这两处改动引入的回退。

规模平坦性（history home，AFTER；括号内为 BEFORE(t4)）：

| 路径 | 41 条 | 1,000 条 | 3,145 条 | 14,531 条 | 25,000 条 | 50,000 条 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 游标历史页 p50 | 1.79 (1.39) | 15.88 (14.74) | 19.84 (16.88) | 13.91 (12.25) | 18.00 (15.67) | 17.96 (15.79) |
| 事件首页 p50（事件数 100→50,000） | 1.58 (1.34) | 2.63 (2.18) | 2.51 (2.06) | — | 2.48 (2.05) | 2.64 (2.07) |
| 事件深页 p50 | 1.63 (1.28) | 2.83 (2.37) | 2.73 (2.18) | — | 2.64 (2.14) | 2.65 (2.15) |
| `get_tool_call_args` p50 | 0.68 (0.55) | 0.78 (0.64) | 0.81 (0.67) | 0.72 (0.63) | 0.76 (0.63) | 0.87 (0.71) |

---

## 3 判据逐条

### (a) `list_session_files`，5,000 项，经 RPC：**达标**

- p50 **11.82 ms**（判据 ≤ 20），p95 13.16，冷开 15.07。原 229.39 / 269.07 / 230.45。
- 回显仍是全量 5,000 项（§2.1 的体积论证 + 单元测试钉住 `command_span() == Span::All`），
  老客户端拿到的 `entries` 不变；回复多了 `hasMore`/`nextCursor`（`Span::All` 下恒为 `false`/`null`）。
- 附带：真实工作区根（`/Users/geilige/future-os`，几项）1.38 → **0.26 ms**。
- **审计 §5 P1-3 判据的后半（10 项 : 5,000 项 ≤ 3）不达标，实测 79×。** 排序要求看见目录里每个名字，
  一次 `readdir` 是 O(目录)；模块文档已把这个残余写明（≈1.6–2.4 ms/5,000 项的枚举 + 每页 N 次
  `lstat`）。要让这一条也成立只能改排序语义（不排序就无法给出稳定游标）。

### (b) `preferSnapshot`：**不达标，且它修的那一档不是审计量到的那一档**

- 判据原文：50,000 事件的 run 上该命令 p95 ≤ 5 ms。
- 被门控的真实 run（32,195 行 / 11.22 MiB）：**p50 9.58~12.39、p95 11.47~12.64**。相对门控前
  （同 run 不限量回放 349.79 ms，回放+折+建 payload 只会更多）**改善约 30×**，但没进 5 ms。
  残余由两段构成：门控自身 4~6.5 ms（O(行) 的 `count+sum`）+ 回落首页 5.4~5.9 ms。
  门控的成本随行数线性（≈0.13–0.2 µs/行），50,000 行时门控本身就要 6.5–10 ms，**这条判据在这个设计下
  基本不可达**。
- **审计的 50,000 事件档（`history` home）完全没动：92.58 ms（BEFORE 81.02 / 89.83）**，因为它
  是旧格式 run，`run_replay_volume` 按设计对非 `FORMAT_VERSION` 返回 `None`。真实数据里最大的旧格式
  run 只有 10,855 事件（`run_events` 全表 19,286 行），所以这一档是**审计合成数据的形状**，
  不是用户会遇到的形状——但"判据在审计自己的 harness 上不达标"这件事必须写明。
- **真正还留着的浪费在中间那一档**：12,624–19,340 行的新格式 run 门控放行、折完再拒，每次打开
  198–573 ms（§2.3 表里 14 个被拒的 run 中 13 个是这么被拒的，门控没参与）。这一档比被门控的
  那 1 个 run 常见得多，也是"折一整个 run 再丢掉"真正的存量。另外 10 个"能装下"的 run
  （11.4k–12.8k 行）同样是每次打开 248–753 ms 重新折一遍——它们至少把结果用上了。
- "能装下的 run 仍然给快照"：**是**。判别方式——同一 run 再发一条普通首页，两条回复字节相同即
  "被拒后回落"，不同即"真给了投影"：
  - 给快照：新格式 11,424 / 11,987 / 12,075 / 12,087 / 12,143 / 12,295 / 12,367 / 12,393 /
    12,702 / 12,796 行（回复 0.64–2.04 **MB**，普通首页只有 8–73 KB，量级差 10~40×）；
    小 run 200 / 201 / 209 行（7.3 / 22.9 / 8.9 KB 投影）；
  - 旧格式：10,000 事件 26,969 B、真实最重 run 28,803 B；
  - 被拒：上述 14 个，回复与普通首页**逐字节相同**。

### (c) 不许回退：**达标**（见 §2.4）

24 条判据内路径全部持平（+0.0~+1.44 ms），规模平坦性保持；`list_session_files` 本身还变快了 5×。

---

## 4 问题一：门控会不会把"以前给快照"的 run 判成 `too_large`

**不会——至少在这份快照上不会，而且原因是结构性的。**

全量统计（`target/remote-latency/scale/homes/history` 的 `agent.db`，与线上快照同源）：

| 项 | 数量 |
| --- | ---: |
| 新格式（分段）run 总数 | 1,496 |
| 门控拒绝（`320*rows + Σbyte_length > 8 MiB`） | **1** |
| 其中"仅靠行数就已越界"（`320*rows > 8 MiB`） | **1**（32,195 行 = 9.83 MiB） |
| "只因为正文字节才越界"（门控里依赖 `byte_length ≤ data.len()` 的那一支） | **0** |
| 体积中位数 | 206.6 KiB |
| 体积前六 | 5.80 / 6.10 / 6.25 / 6.50 / 6.64 / **11.22** MiB |

两个结论：

1. **唯一被门控拒绝的 run，用旧路径也会被拒。** 旧代码在折之前先做整段回放，回放自己按
   `event.data.len() + 320` 记账（`REPLAY_PAGE_BYTES` 同一常量），这个 run 仅 320/行 就 9.83 MiB > 8 MiB，
   所以旧代码在回放途中就会答 `run_snapshot_too_large`。门控只是把这个答案提前、变便宜
   （350 ms → 12 ms），答案本身一字不差。
2. **依赖字节估计的那一支在这份数据上一次都没用到**（0 个 run），所以即使
   `byte_length ≤ data.len()` 这个下界假设哪天不成立，也还没有任何 run 因此被误判。

把 24 个最大 run 逐个实测对照，也只印证同一件事：被拒的 14 个里 13 个是**折完之后**被旧逻辑
（"折出来的 payload > 8 MiB"）拒的，门控完全没参与；剩下 1 个是门控拒的（=旧逻辑同样会拒的那个）。
另有 10 个照旧返回了 0.6–2.0 MB 的投影，判据与改动前一致。

> 未验证的部分（诚实标注）：我**没有**旧二进制的 BEFORE 字节，做不到"同一 run 在旧代码上真的
> 返回了快照/真的拒了"的直接对照。上面第 1 条靠的是同一常量下的算式，第 2 条靠的是 SQL 统计。
> 如果要把这一条也变成实测，需要一份改动前的 release 二进制。

---

## 5 问题二：`list_session_files` 在 50,000 项目录现在要多少

| 规模 | p50 | p95 | cold | 回复（json / gzip） | 每项 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 10 | 0.15 ms | 0.24 ms | 0.36 ms | 2,198 B / 1.1 KB | 0.015 ms |
| 5,000 | 11.82 ms | 13.16 ms | 15.07 ms | 955,336 B / 49.0 KB | 0.0024 ms |
| 50,000 | **144.91 ms** | **149.32 ms** | 157.39 ms | 9,650,340 B / 474.5 KB | 0.0029 ms |

**50,000 项仍然越过 100 ms 预算**（144.9 ms）。原因不是每项的常数变差了（5,000 项时 2.4 µs/项，
50,000 项 2.9 µs/项，基本平坦），而是"全量返回"这个决定本身：命令面没有游标字段，`Span::All`
必须把 5,000/50,000 项全部描述、全部装进一条回复（50,000 项 = **9.65 MB JSON**，gzip 后 474 KB，
也是一次不小的传输）。按本模块自己的成本模型，那 11.8 ms / 144.9 ms 里是：一次 `readdir` 全表
枚举（5,000 项 ≈1.6–2.4 ms）、一次全量排序、**5,000 / 50,000 次 `symlink_metadata`**（`Span::All`
下每一项都要 `describe`，不是只有一页）、以及 5,000→9.65 MB 的 JSON 序列化。

也就是说：**P1-3 的绝对判据在 5,000 项达标，是因为每项成本降了 19×，不是因为成本变成了 O(1)；把上限从线上摘掉（`Span::All`）之后，50,000 项仍然是一条 145 ms / 9.65 MB 的回复。** 要让它真正不涨，需要把游标放到请求上（`list_page` 已经写好、测试齐全，只差 schema + 两个客户端）。

---

## 6 复现命令

```bash
cd /Users/geilige/future-os/.worktrees/perf-p1

# 0) 本工作树自己的 release 二进制（探针在 host 测试目标里）
cargo build -p future-agent --release
cargo test  -p future-host --release --no-run

# 1) 一次性数据：base 快照（mode=ro 在线备份）+ 单维度 home
python3 scripts/perf/remote-commands/scale/build_scale_data.py --only dirs-10
python3 scripts/perf/remote-commands/scale/build_scale_data.py --only dirs-5000
python3 scripts/perf/remote-commands/scale/build_scale_data.py --only dirs-50000
python3 scripts/perf/remote-commands/scale/build_scale_data.py --only history

# 2) 本分支需要的 app 库补列（只补一次性 home 的副本，线上库 mode=ro）
#    缺的列 = host/src/store/schema.rs 的 ADDED_COLUMNS 里副本没有的那些
#    实测只差 workspaces.archived_at：
for home in dirs-10 dirs-5000 dirs-50000 history; do
  sqlite3 "target/remote-latency/scale/homes/$home/.future/app/app.db" \
    "ALTER TABLE workspaces ADD COLUMN archived_at INTEGER;"
done

# 3) P1-3：10 / 5,000 / 50,000 项目录（warm = 2 预热 + 10 采样）
for home in dirs-10 dirs-5000 dirs-50000; do
  python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only $home \
    --tag p1-after --iterations 10 --phase both
done

# 4) P1-4（旧格式档）：审计 history home 的 5 个事件档
python3 scripts/perf/remote-commands/scale/run_scale_sweep.py --only history \
  --tag p1-after --iterations 8 --phase warm

# 5) 回退检查：真实快照上的整条探针（含 list_session_files、任务面板、设置读取、历史页）
#    先按 harness 的方式建 home（在线备份 + 配置 + MEASUREMENT_ISOLATED_HOME 标记），
#    再对副本补同样的 archived_at，然后：
python3 scripts/perf/remote-commands/measure-remote-latency.py \
  --home target/remote-latency/p1-main --tag p1-after --iterations 10 --phase both
```

新格式 run 的那两批（§2.3、§4）用同一个探针、同一份命令单契约，只换命令单：先起隔离 agent，
再把命令单交给探针（与 `run_scale_sweep.py` 内部做的事完全相同）：

```bash
# 选 run：门控会计量 = 320*rows + Σ byte_length，按它排序
sqlite3 "file:target/remote-latency/scale/homes/history/.future/agent/agent.db?mode=ro" "
  SELECT r.run_id, count(*) rows, coalesce(sum(c.byte_length),0) bytes,
         320*count(*)+coalesce(sum(c.byte_length),0) AS volume
  FROM stream_changes c JOIN stream_runs r ON r.id=c.stream_id
  GROUP BY r.run_id ORDER BY volume DESC LIMIT 24;"
# 注意 session_id 在 stream_runs 里是 16 字节 blob（UUID 原始字节），转文本才是客户端发的 spell；
# 且只取 app.db 里有 thread 绑定的那些（host 的 preflight 需要）

# 一条 spec 的形状（命令单是 JSON，探针不改代码）：
#   {"label":"verdict:rows32195_vol11.22MiB",
#    "scale":{"sessionId":"<uuid>","runId":"<run>"},
#    "iterations":3,"warmup":1,"spacingMs":0,
#    "command":{"type":"get_events_since","sessionId":"<uuid>","runId":"<run>",
#               "sinceIdx":-1,"limit":200,"chunkedRead":true,"preferSnapshot":true}}
#   对照 spec：同样字段，preferSnapshot=false（普通首页），
#   以及 limit=1000000 的分页回放（门控前代价的代理）。

# 跑探针（隔离 HOME + 随机回环端口，探针自带 MEASUREMENT_ISOLATED_HOME 断言）：
HOME=<home> USERPROFILE=<home> FUTURE_AGENT_GRPC_ADDR=http://127.0.0.1:<port> \
REMOTE_LATENCY_PHASE=warm REMOTE_LATENCY_COMMANDS=<commands.json> \
REMOTE_LATENCY_OUT=docs/testing/remote-latency/raw/p1/<tag>.json \
REMOTE_LATENCY_VERBOSE=1 \
target/release/deps/future_host-355eafc7c233d2ed \
  remote_host::latency_probe::measure_real_snapshot --exact --ignored --nocapture --test-threads=1
```

本轮的原始样本（`docs/testing/remote-latency/raw/p1/`）：

| 文件 | 内容 |
| --- | --- |
| `dirs-{10,5000,50000}-{warm,cold}-p1-after.json` | P1-3 三个规模档 |
| `history-warm-p1-after.json` | 审计 home 的 66 条命令（含旧格式事件档与平坦性路径） |
| `warm-p1-after.json` / `cold-p1-after.json` / `snapshot-p1-after.json` | 真实快照整条探针（回退检查） |
| `p1-canonical.json` | 新格式 32,195 行 run + 首页/不限量回放对照 + 4 个近预算 run |
| `p1-verdict.json` | 24 个最大新格式 run 的 `preferSnapshot` 与首页对照（判据/误判检查的数据） |
| `p1-materialize.json` | 3 个 run 的不限量分页回放（见 §7.10 的否证） |

原始文件同时也留在 harness 的固定位置（`raw/warm-p1-after.json`、`raw/cold-p1-after.json`、
`raw/scaling/*-p1-after.json`），本目录是拷贝。

一次性 home 与命令单留在 `target/remote-latency/`（gitignored）：`scale/homes/dirs-*`、
`scale/homes/history/`、`canonical/` 下的命令单。`scale/base/` 与 `p1-main/` 两块 2.5 GB 的临时库
在跑完后删掉了（磁盘当时只剩 4.3 GB），重跑 §6 第 1 步会重建 `base`。

---

## 7 仍未解决 / 未测（不要当成已证明）

1. **`workspaces.archived_at` 的降级错位（环境问题，不是这两条改动）**：本分支的
   `validate_core` 要求该列，而线上 `app.db` 已在 `APP_DB_SCHEMA_VERSION=4`，`ADDED_COLUMNS`
   不会在"版本已是最新"的库上执行，于是本树开不了用户的 app 库。真实桌面应用下次升级到本分支时
   会撞上同一处（如果它的库也停在 4）。**建议单独开一条**：`ADDED_COLUMNS` 要么在
   `apply_schema` 里无条件幂等执行，要么提升 `APP_DB_SCHEMA_VERSION`。
2. **P1-4 的 5 ms 判据在这个设计下不可达**：门控自身是 O(行) 的 `count+sum`（实测 0.13–0.2 µs/行），
   50,000 行就是 6.5–10 ms，加上回落首页至少 5 ms，天花板 ~12–15 ms。若真要 5 ms，得让门控变成 O(1)
   （把体积记进 `stream_runs` 一列，或在 append 时维护计数器）。
3. **12.6k–26k 行这一档没有任何改善**：门控常量是 `8 MiB / 320 = 26,214 行`，而实测"还能给出快照"
   的上界是 ≈12.8k 行（§2.3，越界点从 12,624 行起），中间这一段全是"折完再拒"，每次 198–573 ms。
   审计 §5 P1-4 自己推的是 ≈16,100 事件阈值，落地常量比它宽 63%。
   **但把常量改小并不安全**：16,100 行这一档在今天这份快照里确实"全是必拒的"（13.5k 行以上没有任何
   run 拿到快照），可一旦出现"行很多、正文很碎"的 delta run（例如 2 万条 `text_chunk` 折成一条消息），
   行数越界而 payload 装得下，按行数的门控就会误拒——这正是 worker 否掉计数门控的理由。
   而"payload 装不下"这个答案没有便宜的下界：`trim_projection` 会丢掉 tool 结果的正文、清空
   reasoning 文本，所以 `Σdata` 也只能给出上界方向。要走这条路，正解是把折好的投影**缓存住**
   （audit §5 P1-4 的第二个选项），而不是换一个更小的行数阈值。
4. **50,000 行的新格式 run 没有实测**：线上快照里最大的新格式 run 是 32,195 行，第 04 章的
   harness 造不出新格式（分段）run（克隆只复制 `run_events`）。所以 (b) 的 50,000 事件那一格
   只有旧格式数字（92.58 ms）与 32,195 行的外推。
5. **我没有旧二进制**：P1-4 的"改动前在同一 run 上真的拒了"只能靠算式（§4）与不限量回放代理
   （349.79 ms）间接证明；P1-3 的 50,000 项目录没有 BEFORE 样本（第 04 章只扫到 5,000）。
6. **投影缓存看起来没生效**：`run_snapshot` 折完会 `spawn_blocking` 往 `run_snapshots` 写缓存
   （表里另有 2,709 行，说明缓存在用），但同一 run 连续 4–5 次调用每次都还是同一量级
   （被拒的 198–573 ms、被服务的 248–753 ms），没有任何一次明显变快。这一点没查下去——
   如果缓存命中能生效，13k–26k 这一档的**重复**打开会降一个量级，值得作为下一条。
7. **冷态不是磁盘冷读**（没有 `sudo purge`），cold 列是下界。
8. **单机单次**：M3 Ultra，10~12 个样本，p95 = 最大值；`get_task` 这类 1~2 ms 的命令相对噪声大
   （+49% 的那一行就是它）。
9. **P1-5 `get_queue` 按计划延后**，不测、不评判；它出现在标准探针的同一份原始样本里
   （`warm-p1-after.json`），仅此而已。
10. **顺手否证的一条**：我原本想用"不限量分页回放"的回复体积去验证 `byte_length ≤ data.len()`
    这个下界（`p1-materialize.json`）。**不成立**：回复被远端服务按条数/字节再裁一次，
    12,624 行的 run 不限量回放只回了 94 KB JSON（≈500 条事件），所以回复体积不能当"物化总量"用。
    这也说明客户端在 `preferSnapshot` 回落时拿到的永远只是一页，与 §2.3 的"逐字节等于首页"一致。

---

## 9 第二轮：P1-4 的门控被重做了（本章 §0 的 P1-4 判定已被本节取代）

本章 §0 的 P1-4 判定是在第一版门控上做的。第一版只比"存储列"预测，结果只拦下 1 个 run，
13 个"折了 198–573 ms 再被拒"的 run 全部漏过。第二轮按本章给出的证据重做，**两个立论被实测推翻**：

1. **拒掉这些 run 的不是折叠后的 payload，而是回放自己的 8 MiB 记账**——快照里 21 次拒绝全部
   发生在读取途中（`run replay exceeds memory budget`），**折叠从来没跑过**。真正返回的快照
   只有 0.70–2.43 MB，离 payload 预算还有 3.4–11 倍。
2. **那个记账无法从存储列推导**：回放后的 `data` 是重建出来的 JSON（entry id、content key、
   偏移都来自共享 context 表），真实 run 上约 **340 B/事件**，而列里只有 40–75 B 正文 + 约 80 B 元数据。
   所以 `rows × 320 + 正文 + 元数据` 只是下界，比真实记账低约 30%——这正是第一版只拦下 1 个的原因。

第二版因此是"下界直接判 + 触发时按一段有界样本外推"：
`REPLAY_ESTIMATE_SAFETY_PERCENT = 115`（15% 余量）。实测结果：

| 项 | 第一版 | 第二版 | 旧路径 |
| --- | ---: | ---: | ---: |
| 13 个残留 run（12.6k–26k 行） | 只拦 1 个 | **拦 7 个**；这一组的门控耗时 **p50 2.52 ms / p95 6.95 ms** | 194–706 ms |
| 由存储下界直接判定的 run | 1 个 | 2 个（含 19,340 行那个，**0.00 ms**） | 501–736 ms |
| 误判（"以前给快照、现在被拒"） | 0 | **0 / 1,131 个 canonical run** | — |

余量与代价（如实记录）：

- 15% 是实测出来的下限：设成 3% 会**误拒两个真实 run**（12,702 行 → 1,167,006 B 快照；
  12,075 行 → 1,996,182 B 快照）。所以不能再压。
- **仍然没到本节要的 p95 ≤ 5 ms**：门控 2.5–7 ms 加 host 侧回落页约 5.5 ms ≈ **8–9 ms**。
  绝对量已经在 100 ms 预算内，但要真正便宜，得把"这个 run 的记账"持久化，而不是每次重算。
- **13 个里的 6 个仍然照旧**（194–706 ms）：它们的估计落进 15% 余量内，需要精确值而不是采样。
  下一棒 `P1-4c`：在 append 时维护每个 run 的回放字节量（或写一条拒绝标记），让门控变成 O(1)。

自测：`cargo fmt -p future-agent --check` ✔ · `cargo clippy -p future-agent --all-targets -- -D warnings` ✔ ·
observability 模块 8/8 ✔ · `rpc::commands` 323/323 ✔ · 整套 `cargo test -p future-agent`
**2,992 lib + 全部集成目标，0 失败** ✔（本机需 `--test-threads=4`，默认并行度会撞 fd 上限，
那是 fd 限制不是代码问题：复现 63 次失败，4 线程全过）。
