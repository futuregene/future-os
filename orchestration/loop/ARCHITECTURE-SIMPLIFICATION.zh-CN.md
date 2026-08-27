# Loop 架构简化：一个 kanban 工具，不是一个规则引擎

> 原则：**决策者是大模型（agent），不是 loop 内核。** loop 内核应该是一个
> **kanban 工具**——提供确定性工具（todo 状态、verify 门、acceptance 契约、
> evidence、lease），而不是一个**规则引擎**——替 agent 判断「你卡住了，停下来
> 重新规划」。对 agent 的决策提示放在 `future-loop` SKILL.md 里，由 agent 自己
> 读取、自己决策。

## 一、功能分类

### A. 规则型功能（已弱化：从「强制 replan」→「信号 + 继续交付」）

这些曾是内核「替 agent 判断卡住并强制 replan」的硬编码规则——本质上是把
「你怎么决策」写死在内核里，违背「决策者是大模型」的原则。现在每一项都变成
agent 读取的观察信号，不再是内核强制的 replan：

| 规则 | 原触发条件 | 现在 |
|---|---|---|
| outcome floor | `surface_streak >= threshold` | 记录信号，交付 reason 里作为提示（advisory）浮现 |
| oscillation | A→V→A→V 交替 | 记录信号，交付 reason 里作为提示浮现 |
| repair budget | `failed_attempts > MAX` | 失败 todo 仍 runnable（不再过滤），提示「已失败 N 次」 |
| monitor stall | `consecutive_no_change >= 3` | quiet wait + 提示（「考虑 watch-lane expiry」） |
| LLM zombie | `no_progress_turns >= 2` | 提示（「考虑用全新会话重启」） |

### B. 正确性底线（保留：kanban 的确定性语义，不是「替 agent 决策」）

这些是状态一致性的硬约束，弱化它们会让 goal 陷入非法状态：

| 底线 | 为什么必须保留 |
|---|---|
| succession closure missing | 完成必须声明 successor / no-follow-up，否则 goal 永远无法关闭 |
| acceptance gap | 硬契约：acceptance token 必须满足 |
| terminal 判定 | kanban 确定性状态：所有 todo done + gaps 满足才关闭 |
| user gate | 用户门，冻结工作 |
| blocker | 阻塞器 |
| work leased to others | 并发正确性 |
| verify 门 | 正确性：exit 0 才 complete |
| lease | 并发互斥 |
| validation budget | `--verify` 门一直失败仍限制 run loop——正确性底线，不是策略规则 |

### C. 移到 SKILL.md 的决策提示

内核不再「替 agent 决策」，但 SKILL.md 教 agent **读信号、自主决策**：

- 看到 `surface_streak >= N` → 考虑换策略或 supersede
- 看到振荡信号 → 考虑换验证方法或拆分 todo
- 看到 `failed_attempts > 1` → 考虑 supersede 或找 operator
- 看到 monitor 连续无变化 → 考虑 watch-lane expiry 或写 blocker

## 二、改动清单

1. `decision/mod.rs` — 移除规则型 replan 分支（outcome floor / oscillation /
   repair budget / LLM zombie / monitor stall），交付 reason 以 **advisory**
   形式携带这些信号。
2. `decision/stall.rs` — 检测函数保留为观察数据（信号源），不再用于强制 replan。
3. `decision/oscillation.rs` — 同上。
4. `console.rs` — run loop 不再因 repair-budget 耗尽而 break（只保留
   validation-budget break）。
5. `state.rs` + `console.rs` — **session retention**，同一原则在 resume-vs-fresh
   上的体现：内核记录会话「为什么中断」（`SessionRetention` + `FailureKind`），
   并把会话 id 保留在磁盘；调用方通过 `run --session-policy auto|fresh|resume` /
   `--resume-session ID` 决定 resume-vs-fresh。
6. SKILL.md — 新增「agent 决策指引」+「session retention」章节。
7. 信号暴露 — 信号仍出现在交付 reason（agent 在 turn envelope 里直接看到），
   且可查询。

## 三、信号仍保留（不删除，只改用途）

`outcome_floor_breach` / `oscillation_replan_reason` / `repair_exhausted` /
`is_monitor_stalled` 这些检测函数**保留**——它们从「强制 replan 的触发器」变成
「agent 可读的观察信号」，通过两个渠道暴露：

1. 交付 reason 里附加提示（agent 在 turn envelope 里直接看到）；
2. 可查询的状态（例如 `status` / `diagnose`），agent 按需读取。

## 四、session retention（同一原则的延伸）

resume-vs-fresh 也是「决策权在调用方」的体现：内核只提供**观察数据**（会话
为什么中断），从不替调用方决定。`FailureKind` 对中断分类：

- `InfraRecoverable` — LLM 状态完好（429 / 限流 / 连接重置 / agent 崩溃 /
  流间隙）：**可 resume**。
- `ScienceVerifyFailed` — verify 门拒绝了输出，推理状态已坏：**应 fresh**。
- `HardError` — 回合出错且无可恢复的基础设施原因：**应 fresh**。

调用方通过 `--session-policy` / `--resume-session` 显式决策，默认 `auto` 只
resume 内核判定「可恢复」（`InfraRecoverable`）的会话。内核依然是纯工具——
提供状态和信号，但**从不替 agent 做决策**。
