# Future Loop 架构简化：弱化规则型功能，强化 kanban 工具

> 原则：**决策者是大模型（agent），不是 loop 内核。** loop 内核应该是一个
> 「kanban 工具」——提供 todo 状态、verify 门、acceptance 契约、evidence、
> lease 这些**确定性工具**，而不是一个「规则引擎」——替 agent 判断「你卡住了，
> 停下来重新规划」。对 agent 的决策提示应该放在 `future-loop` SKILL.md 里，
> 由 agent 自己读取、自己决策。

## 一、功能分类

### A. 规则型功能（弱化：从「强制 replan」→「信号 + 继续交付」）

这些是内核「替 agent 判断卡住并强制 replan」的硬编码规则，本质上是把
「你怎么决策」写死在内核里，违背「决策者是大模型」的原则：

| 规则 | 触发条件 | 弱化后 |
|---|---|---|
| outcome floor | `surface_streak >= threshold` | 记录信号，交付时在 reason 里附加提示 |
| oscillation | A→V→A→V 交替 | 记录信号，交付时在 reason 里附加提示 |
| repair budget | `failed_attempts > MAX` | 不再过滤失败 todo，交付时提示「已失败 N 次」 |
| monitor stall | `consecutive_no_change >= 3` | 继续 quiet wait，提示「考虑 watch-lane expiry」 |

### B. 正确性底线（保留：这是 kanban 的确定性语义，不是「替 agent 决策」）

这些是「状态一致性」的硬约束，弱化它们会让 goal 陷入非法状态：

| 底线 | 为什么必须保留 |
|---|---|
| succession closure missing | 完成必须声明 successor/no-follow-up，否则 goal 永远无法关闭 |
| acceptance gap | 硬契约：acceptance token 必须满足 |
| terminal 判定 | kanban 确定性状态：所有 todo done + gaps 满足才关闭 |
| user gate | 用户门，冻结工作 |
| blocker | 阻塞器 |
| work leased to others | 并发正确性 |
| verify 门 | 正确性：exit 0 才 complete |
| lease | 并发互斥 |

### C. 移到 SKILL.md 的提示（agent 决策指引）

内核不再「替 agent 决策」，但要在 SKILL.md 里教 agent **读信号、自主决策**：

- 看到 `surface_streak >= N` → 考虑换策略或 supersede
- 看到振荡信号 → 考虑换验证方法或拆分 todo
- 看到 `failed_attempts > 1` → 考虑 supersede 或找 operator
- 看到 monitor 连续无变化 → 考虑 watch-lane expiry 或写 blocker

## 二、改动清单

1. `decision/mod.rs`：移除规则型 replan 分支（outcome floor / oscillation /
   repair budget / LLM zombie / monitor stall），交付 reason 附加信号提示
2. `decision/stall.rs`：检测函数保留为「观察数据」（信号源），不再用于强制 replan
3. `decision/oscillation.rs`：同上
4. `console.rs`：run loop 删除 repair-budget break（只留 validation-budget break）
5. `state.rs` + `agent_client.rs` + `console.rs`：**session retention** —— 同一
   原则的产物。内核记录「会话为什么中断」（`SessionRetention`）+ 保留会话 id，
   由调用方（`run --session-policy auto|fresh|resume` / `--resume-session ID`）
   决定 resume-vs-fresh。内核不替调用方决定是否恢复会话。
6. SKILL.md：新增「agent 决策指引」+「session retention」章节
7. 信号暴露：`status` / `diagnose` 输出里保留这些信号（agent 可查询）

## 三、信号仍保留（不删除，只改用途）

`outcome_floor_breach` / `oscillation_replan_reason` / `repair_exhausted` /
`is_monitor_stalled` 这些检测函数**保留**——它们从「强制 replan 的触发器」变成
「agent 可读的观察信号」，通过两个渠道暴露：
1. 交付 reason 里附加提示（agent 在 turn envelope 里直接看到）
2. `status` / `diagnose` 输出（agent 主动查询）

## 四、session retention（同一原则的延伸）

`resume-vs-fresh` 也是「决策权在调用方」的体现：内核只提供**观察数据**
（会话为什么中断：`InfraRecoverable` = LLM 状态完好，可 resume；
`HardError`/`ScienceVerifyFailed` = 推理状态已坏，应 fresh），不替调用方决定。
调用方通过 `--session-policy` / `--resume-session` 显式决策，默认 `auto` 只
resume 内核判定「可恢复」的会话。

这样内核依然是「纯工具」——它提供状态和信号，但**不替 agent 做决策**。
