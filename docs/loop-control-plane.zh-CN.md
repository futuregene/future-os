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

> **编排者是 AI agent，loop 是它的看板 + 操纵杆。** 设计原则——kanban 而非
> 规则引擎、带可观测/操纵杆的 agent 编排者、用户经 skill 驱动、CLI 单一接口、
> 持久状态优于 session 记忆——统一维护在
> [`orchestration/loop/ARCHITECTURE.zh-CN.md`](../orchestration/loop/ARCHITECTURE.zh-CN.md)，
> 此处不再重复；本页只讲运维模型。

## 核心概念

| 概念 | 命令 | 说明 |
|---|---|---|
| 目标 goal | `goal init` | 项目本地状态 `<cwd>/.future/loop/`，事件溯源 + 可重放 |
| 任务 todo | `todo add/update/complete/supersede` | 五类：advancement（推进）/ user-gate（人工门禁）/ user-action（不冻结的人待办）/ monitor（监视外部状态）/ blocker（外部阻塞）；`--blocks` 依赖链；`--priority` |
| 证据 evidence | `todo complete --evidence` | **非空强制**：关单必须写明实际落地了什么（路径、attempt id、测量结果），`--force` 是操作者显式覆盖 |
| 验收契约 acceptance | `todo add --acceptance "tok1,tok2"` | 关单证据必须包含全部 token（大小写不敏感）——"done ≠ delivered" 的硬形式 |
| 验证器 verify | `todo add --verify "cmd"` | 每个 **run 回合边界**后内核执行命令，exit 0 才算完成；上限 `--max-validation-attempts`。用于机器可判的确定性交付物。**不适用于探索性任务**（检索/报告）——内核判不了其正确性，正确性由编排 agent 读工件判断；手动 `todo complete` 也刻意不重跑 `--verify` |
| 租约 lease | `lease claim/renew/release/status` | 任务被谁租用、多久过期。**租约活性自愈**：记录持有进程 pid，进程死了自动回收——杀掉 worker 后无需手动清理 |
| 门禁 gate | `gate resolve` | 任何打开的用户门禁冻结全部工作直到解决。门禁是决策点不是工作项：对 gate 用 `todo complete` 会**报错并指向 `gate resolve`**（决策被记录，绝不默默标 done）；user-action（不冻结的人待办）展示给用户但不阻塞 agent |
| 交付闭环 delivery | `delivery status/record` | 完成 = `delivered` 待验证态；操作者用 `verified/failed/rework` 结案；3 回合未验证自动派生跟进任务 |
| 终局 terminal | `frontier show` | 验证式闭环：todos 完成/被取代 + 闭环意图 + 无验收缺口 + 无待决 deferred 工作；`frontier` 给出终局判定与缺口明细 |
| 配额 quota | `quota should-run/usage/spend/decisions` | 确定性 should-run 内核：每个回合的调度、拒绝原因、花费全部可审计 |
| 调度器 scheduler | `scheduler tick/show/liveness` | 监视器节奏、宿主故障记录、活性心跳 |
| 多 agent | `agent contract/recipe/succession/collective` | 一个目标多个 worker：契约（替补关系/交接规则）、命名配方一键上车、离线超时自动替补晋升、唤醒轮值表、集体回合账本 |
| worker 可观测 | `worker tail` | 把 worker 的实时回合日志（`.live.jsonl`）渲染成浓缩 tool/用量视图（`--raw` 看原始）——编排者观察 worker 实际在做什么的窗口，据此 steer / stop / 放行 |
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
   ├─ 5. 用 `todo update --text` 纠正跑偏的 worker（下一回合生效）
   ├─ 6. 遇到不可逆/昂贵/用户专属决策 → 挂 user gate 等你拍板（gate 冻结一切）
   └─ 7. 收尾：验收 todo 拷贝交付物到项目根 → validated closure（terminal）
```

**技能与 CLI 的分工**：技能负责"何时该做什么、如何拆解、如何驾驶"（编排层）；
CLI 是底层机制（状态内核 + 硬校验 + 决策）。技能是 v3.x 持续维护的驾驶手册，
与本页同步更新。其内容源头在 **`skills` git submodule** 的
`skills/builtin/future-loop/SKILL.md`（仓库 [future-skills](https://github.com/futuregene/future-skills)）；
改 `~/.future/agent/skills/` 的安装副本只对本地机器生效——要随仓库分发文档改动，
请改 submodule 源（向 future-skills 提 PR，再回本仓库 bump 指针）。

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
future loop ui                           # 实时 Web 仪表盘（http://127.0.0.1:7717）
future loop status --goal G
future loop worker tail --goal G --agent-id mac-worker   # 观察 worker 实时回合
future loop frontier show --goal G        # 终局判定 + 缺口明细
future loop delivery record --goal G ...  # verified / failed / rework
```

## Web 仪表盘（`future loop ui`）

`future loop ui [--port N] [--root DIR] [--no-open]` 在 `127.0.0.1`（默认端口 7717）
起一个本地、**严格只读**的仪表盘。它每次请求都重放与 CLI 相同的事件账本，并通过
SSE 推送变更，因此页面始终是 `.future/loop/` 的忠实、实时投影——仅此而已：服务端
只读取 loop 状态根，且只存在 GET 端点（其他任何方法都返回 405）。所有变更（gate
resolve、goal cancel 等）仍留在 CLI——页面只显示对应的 `future loop` 命令。

- **总览（Overview）**：目标群（fleet）总数（活跃/终局/已取消目标、未决闸门、24h/7d
  运行/成本/quota 槽位）、关注队列（严重度、等待项、建议动作），以及按 triage 排序
  的目标卡片。
- **目标详情（Goal detail，多标签页）**：Board —— 内核的 should-run 决策（原因 + 代码
  + 等待项）、下一步动作、花费/吞吐（14 天 sparkline、token/成本/槽位分桶、7 天结果
  拆分）、未决闸门，以及 todo 依赖 DAG（分层、可点选检查器，含 verify/acceptance/lease/
  evidence 明细）；Todos —— 完整表格，含每个 todo 的 runs/token/成本汇总与活动窗口；
  Workers —— agent 租约、心跳、活性告警、每个 worker 的成本/token 汇总、交付闭环、
  replan 义务、验收缺口；Runs —— 运行账本（验证回执、失败类型、token、成本、证据）+
  语义历史；Events —— 原始事件账本。
- 所有状态在每次请求时都从 `.future/loop/` 投影出来；仪表盘不持有独立状态，也不写入
  任何内容。

## 硬校验优先（约定靠不住，闸门靠得住）

- 空证据关单会被**拒绝**（默认 fail-closed，`--force` 才放行）
- `--verify` 让"写完"不等于"能编译/有产物"——每个交付类任务都应挂一个
- `--acceptance` 把"验收以外部可观测信号为准"变成硬校验
- 租约活性自愈：死进程的租约自动回收，重启 worker 不再需要手动 release
- 工作区守卫：多 agent 写冲突自动降级串行
- 空转回合（15 分钟无写入）会记账；用 `todo update --text` 纠正（下一回合生效）

## 双向消息（supervisor ↔ worker）

supervisor（运行 `/future-loop` 技能的编排 agent）与其 worker 通过目标账本交换消息——
没有进程内推送通道。两个方向都走同一份事件源状态：

- **注册 supervisor**（每个目标一次）：
  `future loop supervisor register --goal G --session-id <supervisor-agent-session>`
  这把 supervisor 的 agent 会话 id 绑定到目标；worker 在 `replay` 时读到它，并把报告
  定向到它。

- **下行（supervisor → worker，一次中断）：**
  `future loop supervisor steer --goal G [--agent-id A] --instruction "..."`
  一条 `WorkerSteered` 事件落入账本；运行中 worker 的 watch 任务看到它，并中止进行中的
  运行（真正的 `supersede_session` 中断，不是 system-prompt 提示）。下一回合把这条指令
  排入它的 envelope 并执行。

- **上行（worker → supervisor，一次报告）：**
  在回合边界，worker 把报告入队（`enqueue_if_busy`，因此绝不打断 supervisor）到已注册
  会话，且恰好针对三种状态迁移：用户闸门打开（①）、todo 完成（②）、或 todo 因科学/
  硬错误失败（③）。每条报告按迁移幂等键控，因此跨运行的重发会去重。若未注册
  supervisor，持久化用户闸门仍是权威的干预通道。

- **基础设施停止 + 死 worker：** 在到达回合边界写回之前退出的 worker（gRPC 传输丢失、
  不完整重试预算耗尽）也会上报一条 `infra_stopped` 备注。
  直接死掉的 worker（SIGKILL / 崩溃 / 宿主机故障）不执行任何代码，因此周期性的
  `scheduler tick` 会检测到孤儿租约（持有者 pid 已死），并向已注册的 supervisor 推送
  一条 `host_died` 备注，促使编排者重启，而不是只在下次 `status` 轮询时才发现有 worker
  死了。

- **回合中观察 worker：** 上述消息都是回合边界驱动的。要看 worker *此刻*在做什么
  （在调哪些工具、token/成本消耗），用 `future loop worker tail --goal G [--agent-id A]
  [--lines N] [--raw]`——它把 worker 的 `.live.jsonl` 回合流渲染成浓缩 tool/用量视图。
  这是 steer/stop 操纵杆的可观测补充：先观察，再带完整上下文打断或纠正。

## loop 状态以 CLI 为准

控制面通过 **`future loop` CLI** 驱动与观察——目标状态是项目本地的
（`<cwd>/.future/loop/`），不属于任何一个客户端。TUI、桌面 GUI、移动端 App
与 IM 机器人目前没有内置的 loop 视图；它们通过 **`/future-loop` 技能**驱动
同一控制面（技能替 agent 编排 `future loop` 命令）。因为状态在项目里、技能
经 agent 服务运行，所以在一个客户端（如 TUI）启动的目标可以在任何其他客户端
（如飞书聊天）继续驾驶。

## CLI 全景（7 组 41 命令）

```bash
future loop registry        # 全部命令（组/命令）
future loop commands        # 按操作者旅程分组视图
```

多动词命令（`supervisor`、`worker`、`todo`）暴露逐动词用法：`<cmd> <sub> --help`
渲染该动词的精确签名（如 `supervisor steer --help` → `--agent-id` +
`--instruction`），`<cmd> --help` 列出其子命令——编排者无需解析合并后的顶层
usage 即可发现参数。

- **goal 组**（5）：`goal` `status` `ui` `models` `diagnose`
- **todo 组**（6）：`todo` `gate` `replan` `frontier` `lease` `task-graph`
- **agent 组**（5）：`agent` `scope` `lane` `supervisor` `worker`（list / stop / **tail**）
- **ops 组**（18）：`version` `doctor` `history` `turn` `todo-event` `evidence-log` `backup` `authority` `profile` `quota` `scheduler` `store` `backfill` `privacy` `runs` `heartbeat-prompt` `worker-bridge` `run`
- **work-items 组**（3）：`attention` `inbox` `delivery`
- **cli 组**（2）：`registry` `commands`
- **canary 组**（1）：`canary`

## 与 FutureOS 其他部件的关系

- **Agent 服务**（`future agent`，gRPC 127.0.0.1:50051）：`run` 通过它执行每个回合
- **任何客户端都可经技能驱动**（TUI、桌面、移动端、飞书 / 钉钉）：loop 目标由 `/future-loop` 技能编排 `future loop` 命令驱动——桥与 loop 之间没有原生集成；门禁以 agent 消息形式提出一个具体问题
- **技能 `/future-loop`**：编排 Agent 使用本控制面的驾驶手册（v3.x 与本文档同步维护）
- **状态位置**：`<cwd>/.future/loop/`（加入项目 `.gitignore`）

## 更多

- 安装与构建：[build-and-install.zh-CN.md](build-and-install.zh-CN.md)
- 证据账本：[long-run-evidence-ledger.zh-CN.md](long-run-evidence-ledger.zh-CN.md)
- TUI 使用：[tui.zh-CN.md](tui.zh-CN.md)
- 技能源码：[`skills/builtin/future-loop`](../skills/builtin/future-loop)（git submodule → [future-skills](https://github.com/futuregene/future-skills/tree/main/builtin/future-loop)）
