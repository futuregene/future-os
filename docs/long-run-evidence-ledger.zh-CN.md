# 长程运行证据台账

> （[English](long-run-evidence-ledger.md)）长程目标的问责记录——经
> [Loop 控制面](loop-control-plane.zh-CN.md)驱动的多日、多 turn
> 工作。每个关闭的长程目标在此留一条记录。

长程目标会在多个有界 turn 中消耗真实的墙钟时间、token 和费用。本台账
的意义在于：每个这样的工作都留下一份**事实性、可核验**的记录——
**花了多久、花了多少、验证了什么，以及同样重要的——明确没有做什么**
（已接受的残差、豁免、范围排除）。条目在目标关闭时依据一手数据源
填写，而非凭记忆。

## 如何追加条目

数据源（全部本地、全部可核验）：

- **turn / 墙钟 / 花费**——目标运行历史账本
  `.future/loop/goals/<goal-id>/runs.jsonl`（每次运行的
  `terminal_state`、token 增量、`cost`、工具调用数；每回合实时日志在
  `.future/loop/runs/<run-id>.live.jsonl`），
  以及 `future loop runs history --goal <id> --format json` /
  `future loop quota usage --goal <id>` 的紧凑投影。
- **验证**——目标的官方测量（写明工具、命令、commit、日期），以及
  `future loop evidence-log --goal <id>`。
- **PR**——在目标工作窗口内 `git log --grep`。
- **边界**——目标的豁免/签收记录（用户门禁、验收 todo），以及任何
  读者可能误认为是遗漏的事项。

条目结构：

1. **目标**——id、目标内容、关闭日期。
2. **墙钟与 turn**——首跑→末跑时间戳、跨度、运行分类
   （completed / error / incomplete）。
3. **花费**——运行历史台账记录的 token 入/出与费用，含逐 turn 明细。
4. **验证结果**——验收指标、基线→最终、验证方式。
5. **显式边界**——范围排除、已签收的残差、数据本身的归因限制。
6. **教训**——只记耐久可复用的（细节在 FUTURE.md）。

---

## 条目 2026-08-12 —— workspace 测试覆盖率目标

**目标** `goal_4a742a954e3c`——通过逐 crate 补测试把 Rust workspace
推到约 100% 行覆盖（官方指标：`cargo llvm-cov` summary 的 **Lines**
列，分 crate + 总计），然后对账残差并验收。2026-08-12 关闭，豁免清单
经用户签收。

### 墙钟与 turn

- 首跑：2026-08-09 23:05（+08:00）——末跑：2026-08-12 10:21（+08:00）。
- **跨度：约 59.3 小时（2 天 11 小时 16 分）。**
- **10 次有界运行**：7 completed、2 error、1 incomplete。每个
  error/incomplete 的推进都在目标内重试至完成。
- 台账共 14 个 todo（6 个 crate 推进、工具+基线、cli 收尾、验收、
  用户门禁、交付物、onboarding；2 个被 supersede）。

### 花费（按运行历史台账记录）

合计：**输入 1,777,043,598 token / 输出 3,562,578 token /
约 $1,946.84**，5,979 次工具调用。

| # | 开始（+08:00） | 状态 | 内容 | 输入 token | 输出 token | 费用（USD） | 工具 |
|---|---|---|---|---|---|---|---|
| 1 | 08-09 23:05 | completed | `scripts/coverage.sh` 工具 + workspace 基线（PR #138） | 2,076,302 | 31,272 | 4.39 | 94 |
| 2 | 08-10 00:06 | completed | future-rpc → 100%（PR #139） | 20,874,338 | 192,220 | 32.56 | 214 |
| 3 | 08-10 07:21 | error | future-tui 推进，未遂（turn 4 重做） | 837,524,085 | 1,121,310 | 880.04 | 1,824 |
| 4 | 08-10 13:03 | completed | future-tui → 100%（PR #140、#141） | 33,663,407 | 152,044 | 44.66 | 270 |
| 5 | 08-10 14:30 | error | future-cli 推进（双执行器冲突；由并发会话落地，PR #146、#147） | 71,805,042 | 322,458 | 89.42 | 493 |
| 6 | 08-11 03:12 | completed | future-agent → 96.75%（PR #149） | 547,793,935 | 769,548 | 577.46 | 1,450 |
| 7 | 08-11 06:09 | incomplete | future-channels 推进（turn 8 继续） | 167,851,903 | 549,848 | 193.99 | 842 |
| 8 | 08-11 07:34 | completed | future-channels → lcov DA 100%（PR #150） | 81,609,442 | 310,002 | 101.68 | 546 |
| 9 | 08-11 08:27 | completed | 最终验收测量 + 豁免对账 | 13,305,340 | 95,796 | 20.62 | 220 |
| 10 | 08-12 10:21 | completed | 交付物（`coverage/`）+ 门禁方法（FUTURE.md） | 539,804 | 18,080 | 2.03 | 26 |

### 验证结果

官方测量：2026-08-12 在 `main@b24d5501` 上单次全量运行
`scripts/coverage.sh`——**regions 98.26% / functions 98.03% /
lines 98.80%**，**3,864 个测试，0 失败**。

| Crate | 基线 @4d3dd2fc（lines） | 最终（summary Lines； missed 数） | PR |
|---|---|---|---|
| future-rpc | 91.77% | 99.79%（summary 7 / per-line 0） | #139 |
| future-tui | 67.80% | 100.00%（0） | #140、#141 |
| future-cli | 42.46% | 99.83%（summary 40 / per-line 15） | #146、#147 |
| future-loop | 77.72% | 98.43%（summary 279 / per-line 191） | #148 |
| future-agent | 84.44% | 96.75%（summary 987 / per-line 700） | #149 |
| future-channels | 31.50% | 99.84%（summary 17 / per-line 0；lcov DA 100%） | #150 |
| **总计** | **70.15%**（54,559/77,775） | **98.80%** | — |

测试数：2,173 → 3,864（+1,691）。合并 PR 共 10 个：#138（工具）、
六个 crate 推进（#139；#140+#141；#146+#147；#148；#149；#150）、
#151（flake 加固）。

残差对账：summary missed 1,330 行 = **真实 906 + 幻影 424**（summary
按函数记录计数；per-line 工具对泛型实例做 max 合并）。906 真实 missed
= 446 行不可执行的归因伪影（365 个闭括号、74 个行首锚点、7 个注释）
+ ~460 行防御性/死 arm 代码，并经 lcov DA 零命中 = HTML uncovered-line
交叉验证。

推进中发现的真实缺陷：**6 个真 bug**——future-loop 4 个（#148：
`doctor --agent-addr` 与 `benchmark run --agent-addr` 嵌套 `block_on`
panic、monitor 无变化轮询追加伪 `TodoCompleted`、`try_claim_todo`
重建漏掉过期 arm）、future-channels 1 个（#150：gRPC client
`entry_id` 遮蔽）、future-cli 1 个（CDP 分发循环订阅竞态）——另有
**2 个 flake 根因**在 #151 修复（spawn_mock 抢端口竞态、
`resolve_future_base_url` 环境泄漏）。

### 显式边界

- **范围仅限 Rust workspace。** `desktop/src-tauri`（基线约 9,753 行
  未覆盖）、desktop 前端、mobile 均不在本目标范围内。
- **门禁指标是 summary Lines**；regions（98.26%）与 functions
  （98.03%）如实报告但不按 100% 门禁。
- **906 行真实残差是"经签收接受"，不是遗忘**：豁免类别 W1–W8
  （平台 `cfg(windows)` / 防御性死 arm / OS 故障注入 / 竞态窗口 /
  归因伪影 / summary 幻影 / 测试 mock 闭包 / 死生产者）已于
  2026-08-11 经用户门禁 todo_01368ac862e2 签收；完整清单见
  `coverage/acceptance-waivers.md`。强行覆盖意味着删除防御性代码、
  在生产代码加 `cfg(test)` 钩子、或使用仅限 nightly 的
  `#[no_coverage]`——已明确否决。
- **424 行幻影 summary 行无法被任何 per-line 工具打印**（已验证测试
  无法移动）；per-line 真相 = lcov DA 零命中 / HTML uncovered-line。
- **交付物仅本地**：`coverage/`（lcov.info、html/、summary.txt、
  missed-lines.txt、acceptance-waivers.md）按 `scripts/coverage.sh`
  的设计被 gitignore。
- **归因限制**：运行历史花费只覆盖 10 次 loop 运行。future-loop 推进
  （PR #148）与 future-cli 收尾（PR #146/#147）部分由共享 worktree
  的并发交互会话执行（双执行器冲突），其花费未计入上表。token 数为
  台账按次记录的累计上下文 token，非去重 token；error/incomplete
  运行的部分花费已计入合计。

### 教训

- 任何官方测量前先净化环境（unset 所有 `CARGO*` 变量、rustup
  工具链 bin 置于 PATH 最前、隔离 `HOME`）——否则 unit-hash 错位
  与 auth 文件泄漏会污染计数。
- summary 的 "missed line" 为真当且仅当该行所有 region 均为零；
  追行之前务必用 per-line 工具交叉核对。
- 一个目标上的两个执行器需要不相交的文件集 + 早提交/勤提交；共享
  worktree 里放一份未跟踪的 `COORDINATION-NOTE.md` 有效。
- 只在全量运行时出现的 flake：在相同净化环境下单独重跑该测试——
  本次两例都是真实缺陷，不是噪声。

---

*下一条目：下一个长程目标关闭时追加在此行之下。*
