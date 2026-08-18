# Loop 控制面（`future loop`）

> 本地控制面，让长程 AI Agent 工作变得持久、可治理、可验收——目标、门禁、任务、证据与配额在聊天之外稳定存在，Agent 每次只执行一个有界回合，一个确定性内核决定下一步做什么。

## 为什么需要

一次对话会丢失上下文；一个"帮我盯一周"的请求不该靠聊天记录撑着。`future loop` 把请求变成一个**持久化目标**：拆成任务图、挂上人工门禁、每步留证据、完成的定义是可验证的——跨会话、跨重启、跨多个并行 worker 都不会丢。

## 一张图看懂

```
目标 objective
   │
   ├─ 任务图 todos（advancement / user-gate / monitor，--blocks 依赖）
   │
   ├─ 需要人拍板？ ──▶ 提出一个具体问题并等待（user gate）
   │
   ├─ 可以安全推进？ ──▶ 内核发出决策包：run 这个 todo / wait / replan / terminal
   │
   ▼
Agent 执行一个有界回合（gRPC）→ 写证据 → 内核据此决定下一回合
```

## 核心概念

| 概念 | 命令 | 说明 |
|---|---|---|
| 目标 goal | `goal init` | 项目本地状态 `<cwd>/.future/loop/`，事件溯源 + 可重放 |
| 任务 todo | `todo add/update/complete/supersede` | 五类：advancement（推进）/ user-gate（人工门禁）/ user-action（不冻结的人待办）/ monitor（监视外部状态）/ blocker（外部阻塞）；`--blocks` 依赖链；`--priority` |
| 证据 evidence | `todo complete --evidence` | **非空强制**：关单必须写明实际落地了什么（路径、attempt id、测量结果），`--force` 是操作者显式覆盖 |
| 验收契约 acceptance | `todo add --acceptance "tok1,tok2"` | 关单证据必须包含全部 token（大小写不敏感）——"done ≠ delivered" 的硬形式 |
| 验证器 verify | `todo add --verify "cmd"` | 每个回合后内核执行命令，exit 0 才算完成；上限 `--max-validation-attempts`。空关单的物理阻断器 |
| 租约 lease | `lease claim/renew/release/status` | 任务被谁租用、多久过期。**租约活性自愈**：记录持有进程 pid，进程死了自动回收——杀掉 worker 后无需手动清理 |
| 门禁 gate | `gate resolve` | 任何打开的用户门禁冻结全部工作直到解决；user-action（不冻结的人待办）会展示给用户但不阻塞 agent |
| 交付闭环 delivery | `delivery status/record` | 完成 = `delivered` 待验证态；操作者用 `verified/failed/rework` 结案；3 回合未验证自动派生跟进任务 |
| 终局 terminal | `frontier show` | 验证式闭环：todos 完成/被取代 + 闭环意图 + 无验收缺口 + 无待决 deferred 工作；`frontier` 给出终局判定与缺口明细 |
| 配额 quota | `quota should-run/usage/spend/decisions` | 确定性 should-run 内核：每个回合的调度、拒绝原因、花费全部可审计 |
| 调度器 scheduler | `scheduler tick/show/liveness` | 监视器节奏、宿主故障记录、活性心跳 |
| 多 agent | `agent contract/recipe/succession/collective` | 一个目标多个 worker：契约（替补关系/交接规则）、命名配方一键上车、离线超时自动替补晋升、唤醒轮值表、集体回合账本 |
| 前端面 frontier | `frontier show` | 成果连续段（outcome segments）、结构化 replan 规则、有界语义历史（N=50）、终局判定 |

## 用技能驱动 loop（推荐入口）

绝大多数情况下，你不需要手敲下面的 CLI——**用 `/future-loop` 技能让 Agent 自己驾驶**：

```
你（用户）说 "/future-loop 帮我盯着 X 一周"
   │
   ▼
Agent 加载 future-loop 技能（v3.x 驾驶手册）
   ├─ 1. 先 `future loop status` 查是否已有同类目标（有则续做，不重复建）
   ├─ 2. 与你确认计划（步骤/模型/thinking level）——除非你的指令已含完整目标与约束
   ├─ 3. `goal init` + 拆 todos（依赖 --blocks、硬校验 --verify/--acceptance 一起挂）
   ├─ 4. 逐回合驱动：`run --max-turns 1 --agent-id <唯一名>`，回合结束立即重启
   ├─ 5. 用 `todo update --text` 中途 steering 纠正跑偏的 worker
   ├─ 6. 遇到不可逆/昂贵/用户专属决策 → 挂 user gate 等你拍板（gate 冻结一切）
   └─ 7. 收尾：验收 todo 拷贝交付物到项目根 → validated closure（terminal）
```

**技能与 CLI 的分工**：技能负责"何时该做什么、如何拆解、如何驾驶"（编排层）；
CLI 是底层机制（状态内核 + 硬校验 + 决策）。技能是 v3.x 持续维护的驾驶手册，
与本页同步更新；完整语义见 [future-skills/builtin/future-loop](https://github.com/futuregene/future-skills/tree/main/builtin/future-loop)。

## 用户工作流（从零到闭环）

```bash
# 1. 创建目标
future loop goal init --objective "..." --cwd DIR

# 2. 拆任务（依赖 + 硬校验一起挂上）
future loop todo add --goal G --text "..." --priority P0 --verify "cargo check -p X"
future loop todo add --goal G --text "..." --blocks T1 --acceptance "attempt,scored"
future loop todo add --goal G --role user --class user_gate --text "发布门禁" --gate-question "是否发布？"

# 3. 驱动回合（一个 worker 一个 --agent-id；回合结束立即重启）
future loop run --goal G --agent-id mac-worker --model M --thinking-level L --max-turns 1

# 4. 人工拍板
future loop gate resolve --goal G --todo-id GATE --decision "approve"

# 5. 观察与闭环
future loop status --goal G
future loop frontier show --goal G        # 终局判定 + 缺口明细
future loop delivery record --goal G ...  # verified / failed / rework
```

## 硬校验优先（约定靠不住，闸门靠得住）

- 空证据关单会被**拒绝**（默认 fail-closed，`--force` 才放行）
- `--verify` 让"写完"不等于"能编译/有产物"——每个交付类任务都应挂一个
- `--acceptance` 把"验收以外部可观测信号为准"变成硬校验
- 租约活性自愈：死进程的租约自动回收，重启 worker 不再需要手动 release
- 工作区守卫：多 agent 写冲突自动降级串行
- 空转回合（15 分钟无写入）会记账；用 `todo update --text` 中途 steering 干预

## loop 状态以 CLI 为准

控制面通过 **`future loop` CLI** 驱动与观察——目标状态是项目本地的
（`<cwd>/.future/loop/`），不属于任何一个客户端。TUI、桌面 GUI、移动端 App
与 IM 机器人目前没有内置的 loop 视图；它们通过 **`/future-loop` 技能**驱动
同一控制面（技能替 agent 编排 `future loop` 命令）。因为状态在项目里、技能
经 agent 服务运行，所以在一个客户端（如 TUI）启动的目标可以在任何其他客户端
（如飞书聊天）继续驾驶。

## CLI 全景（10 组 43 命令）

```bash
future loop registry        # 全部命令
future loop commands        # 按操作者旅程分组视图
```

- **goal 组**：`goal` `status` `models` `diagnose`
- **todo 组**：`todo` `gate` `replan` `frontier` `lease` `task-graph`
- **agent 组**：`agent` `list` `scope` `lane` `supervisor`
- **ops 组**：`version` `doctor` `history` `turn` `todo-event` `evidence-log` `backup` `authority` `profile` `quota` `scheduler` `store` `backfill` `privacy` `runs` `heartbeat-prompt` `worker-bridge` `serve-status` `run`
- **work-items 组**：`attention` `inbox` `delivery`
- **handoff 组**：`handoff`
- **cli 组**：`registry` `commands`
- **benchmark 组**：`benchmark`
- **replay 组**：`replay`
- **canary 组**：`canary`

## 与 FutureOS 其他部件的关系

- **Agent 服务**（`future agent`，gRPC 127.0.0.1:50051）：`run` 通过它执行每个回合
- **任何客户端都可经技能驱动**（TUI、桌面、移动端、飞书 / 钉钉）：loop 目标由 `/future-loop` 技能编排 `future loop` 命令驱动——桥与 loop 之间没有原生集成；门禁以 agent 消息形式提出一个具体问题
- **技能 `/future-loop`**：编排 Agent 使用本控制面的驾驶手册（v3.x 与本文档同步维护）
- **状态位置**：`<cwd>/.future/loop/`（加入项目 `.gitignore`）

## 更多

- 安装与构建：[build-and-install.zh-CN.md](build-and-install.zh-CN.md)
- 证据账本：[long-run-evidence-ledger.zh-CN.md](long-run-evidence-ledger.zh-CN.md)
- TUI 使用：[tui.zh-CN.md](tui.zh-CN.md)
- 技能源码：[future-skills/builtin/future-loop](https://github.com/futuregene/future-skills/tree/main/builtin/future-loop)
