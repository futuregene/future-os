# Loop 控制面（`future-loop`）

> 面向长期 AI agent 工作的本地控制面——在 agent 执行有界回合期间，保持目标、门禁、todos、证据、额度和交接的稳定。

FutureOS 在 `orchestration/loop` 内置了原生 loop 控制面，提供 `future-loop` CLI 与 `/future-loop` agent 技能。它把一段对话变成一个持久、可复盘、可长期运行的目标：目标、todos、人工门禁、监控、证据与完成状态都持久化在聊天之外，由确定性内核决定下一步该做什么——一次一个回合。

> `future-loop` 是基于 [loopx](https://github.com/huangruiteng/loopx) 的 Rust 改写版，针对 FutureOS 做了适配与扩展（项目本地状态、gRPC 执行桥、quota 内核、扩展与多 agent）。

主要调用方式是 `future loop <command>`；独立二进制 `future-loop` 与其等价且仍会安装（安装脚本与 `make run-loop` 使用它）。


```
objective / issue / project
   │
   ▼
loop 状态：目标 + 门禁 + todos + 范围 + 证据 + 额度
   │
   ├─ 需要人工判断？──────────▶ 提出具体问题并等待
   │
   ├─ 有安全回退？────────────▶ 执行一个有界 agent 切片
   │
   ▼
agent 执行一个回合（gRPC）→ 写入证据 + 交接 + 下一个 todo
   │
   ▼
额度决定下一次 tick
```

## 亮点

### 持久化目标与 todo 工作图

- **目标**（`goal init / cancel / delete`）：项目本地状态位于 `<cwd>/.future/loop/`，以事件账本 + 重放持久化。
- **Todos**（`todo add / claim / complete / supersede / update / archive`）：advancement / user-gate / monitor / blocker 类别、优先级、依赖链（`--blocks`）、claim + 租约生命周期，以及与参考实现兼容的完成契约（每个完成的 todo 必须声明后继或显式 no-follow-up）。
- **人工门禁**（`gate resolve`）：todo 阻塞在一个具体问题上，直到人工决策落地——绝不"模糊等待"。
- **监控**（`--class monitor --cadence ...`）：定时观察外部状态（CI、PR、文件），无变化时退避，陈旧目标绝不消耗额度。

### 确定性 should-run 决策内核

`future loop run` 让一个纯函数、可注入时钟的内核决定：是否运行、为什么、运行哪个 todo——identity 范围边界、人工门禁优先级、修复预算、成果底线、后继 replan 义务、接受度缺口，以及一个把每次决策归入九种 disposition 之一的调度仲裁层（terminal / monitor-wait / active work / consistency repair / human gate / quiet wait / …）。执行是 fail-closed 的：已取消的目标永不运行；状态歧义时停止而不是继续消耗。

### 额度与调度

- 跨 `run` / `agent` / `heartbeat` 三来源的 slot 记账、24h/7d 用量汇总、以及检测"仅表面进展"循环的 stall repair。
- 调度状态机：节奏归一化（`once / hourly / daily / weekly` 或 `15m / 1h / 2d`）、原子持久化、host 失败跟踪。
- 监控 poll 以可重放事件（`MonitorPolled`）落账，写回精确。

### 事件溯源与投影

- 内容寻址事件 id + 幂等追加 + fail-closed 冲突检测；`QuotaSpent` / `EvidenceAttached` 事件；markdown 回填进账本。
- 按目标 schema 迁移桥（verify / migrate / bridge）、隐私分级投影（public-safe / local-private / private-pointer）、run 生命周期（history / compaction / index / retention / stale 检测）。

### 独立验证

`todo add --verify "cargo test" --max-validation-attempts 5` 挂载独立验证器：内核在每次回合后在目标 cwd 运行它，仅当退出码为 0 时才完成 todo，重试预算耗尽时触发 replan——闭环是"已验证"而非"自评"。

### 扩展与多 agent

- 能力框架：provider 生命周期（declared → installed → enabled → ready）、可查询 catalog、能力门禁（run / ask-owner / repair-bridge / skip）、按能力注册的命令钩子。
- 扩展：声明式 manifest + install / enable / disable / rollback（保留修订版本）+ readiness doctor——v1 为声明式，绝不执行扩展代码。
- Identity 范围的多 agent：agent 范围边界、lane 推荐、supervisor 提案/回执事件、任务租约、带交付契约的交接文档、todo 依赖图、注意力队列 / operator inbox——由 `agent` 命令组驱动（见[多 agent 工作流](#多-agent-工作流)）。

### 评估与诊断

- benchmark 闭环（protocol / run / ledger，复用同一 gRPC 通道）、decision replay + model-behavior corpus、canary 冒烟套件（`core-control-plane` / `extension-runtime` / `release-gate`）。
- `version` / `doctor` / `history` / `turn` / `todo-event` / `evidence-log` 诊断，以及 `backup` / 恢复。

## CLI 一览

```text
goal          goal 生命周期（init / cancel / delete）· status · models · diagnose
todo          add | claim | complete | supersede | update | archive
              gate resolve · replan ack · lease · task-graph
agent         onboard · scope · lane · supervisor
capability    list | propose | commands · catalog · 按能力钩子
extension     install | upgrade | enable | disable | rollback | status | capabilities
ops           version · doctor · history · turn · todo-event · evidence-log
              backup · authority · profile · quota · scheduler · store
              backfill · privacy · runs · heartbeat-prompt · worker-bridge
              serve-status · run
work-items    attention · inbox
handoff       handoff [--write]
benchmark     protocol | run | ledger
replay        record | run · corpus build | run
canary        smoke [--profile ...]
cli           registry [--json] [--include-experimental]
```

不带参数运行 `future-loop` 查看完整分组帮助。

## 快速开始（技能模式）

在会话中向 agent 输入：

```
/future-loop 把这个长期目标拆成 goal 和 todos，持续推进到完成
```

技能加载后：创建持久化目标 → 把工作拆成 todos（含依赖链与最终交付复制 todo）→ 用 `future loop run --max-turns 1` 逐回合推进，每步汇报状态与成本。

也可以直接在终端驱动：

```bash
future loop goal init --objective "..." --cwd /path/to/project
future loop todo add --goal <id> --text "collect data" --priority P0
future loop todo add --goal <id> --text "write report" --priority P0 \
  --blocks <collect-todo-id> --verify "test -f report.md"
future loop status --goal <id>
future loop run --goal <id> --model future/deepseek-v4-flash --max-turns 1
```

## 多 agent 工作流

`agent` 命令组用于建模由多个 agent 共享的目标。每个 agent 以 `--agent-id`
标识、界定自己的工作范围，并通过交接文档移交——这样 supervisor（或人）
就能判断谁负责什么、下一个 agent 需要知道什么。

> 下面的命令都是**扁平顶层命令**——`future-loop` 把 `agent`、`scope`、
> `lane`、`supervisor`、`handoff`、`task-graph`、`attention`、`inbox` 全部
> 在顶层分发。帮助输出里的 `agent` / `todo` / `work-items` 分组只是展示用
> 分组。

### 1. 注册 agent（登记 + 能力声明）

```bash
# 仅注册（quota --agent-id 的前置条件）
future loop agent --goal <id> --agent-id codex

# 注册并声明能力（能力门禁的输入）
future loop agent onboard --goal <id> --agent-id codex --capability shell,github
```

`onboard` 会记录一条带能力声明的 `AgentOnboarded` 事件。

### 2. 范围与 lane

```bash
# identity 范围边界：该 agent 可见/可认领的 todos，以及属于他人（边界外）的认领
future loop scope --goal <id> --agent-id codex [--exclude docs,build]

# 该 agent 的紧凑 lane 推荐（分类 + 建议动作）
future loop lane --goal <id> --agent-id codex
```

frontier 输出列出 `visible agent todos`、`claimed by self`、`other agent
claims`、`open user gates` 与 `unclaimed advancement` 计数；`lane` 汇总该
agent 的进展范围与建议的下一步动作。

### 3. Supervisor 决策

```bash
# 提案一个决策：observe（默认）或 execute（带能力）
future loop supervisor propose --goal <id> --agent-id super --decision-id d1 \
  --target-agent-id codex --kind execute --capabilities shell --summary "run tests"

# 记录宿主的回执（executed | failed | rejected）
future loop supervisor receipt --goal <id> --decision-id d1 \
  --receipt-id r1 --adapter-id host --outcome executed

# 以 JSON 投影全部 supervisor 事件
future loop supervisor events --goal <id>
```

### 4. 交接

```bash
# 打印交付契约（降级模式 + 摘要）与交接文档
future loop handoff --goal <id>

# 同时写入 .future/loop/goals/<id>/HANDOFF.md
future loop handoff --goal <id> --write
```

交付契约由 run 历史推导（新的在前）；交接文档渲染为 markdown，下一个
agent 无需重读整个账本即可接续上下文。

### 5. 协调

```bash
# todo 依赖图（拓扑序；有环则 fail closed）
future loop task-graph --goal <id>

# 单个目标或全部目标的注意力队列
future loop attention --goal <id>
future loop attention --all

# operator inbox 紧急度投影
future loop inbox --project .
```

## 部署拓扑（推荐）

控制面刻意保持**无守护进程**（daemonless）：每个 `future loop` 命令都是短生命周期进程——加载账本、做一件有界的事、持久化、退出。因此可用性来自**外部调度器**按你选定的节奏调用 `future loop run`，而不是一个需要你维持存活的常驻 loop 进程。

```
cron / systemd timer / CI 定时流水线        （可用性来源）
   │  每次 tick 一次调用
   ▼
future loop run --goal <id> --agent-id <name> --max-turns 1
   │  有界切片：决策 → 执行一个回合 → 写回 → 退出
   ▼
<cwd>/.future/loop/                         （事件账本——唯一状态）
```

为什么可以安全地用外部方式驱动：

- **有界调用**——每次 tick 受 `--max-turns` / `--max-turn-secs` 上限约束；卡住的回合会优雅停止而不是占住目标，下一次 tick 从账本继续。
- **重启安全的状态**——事件账本采用内容寻址、幂等追加；崩溃或重叠的 tick 重放后仍然干净，冲突时 fail-closed，而不是重复消耗额度。
- **租约协调**——`run` 在租约下认领 todo（默认 4 小时，`--lease-secs`），两个调度器不会静默抢占同一个 todo。务必传稳定的 `--agent-id`（首次使用自动注册）；`--anonymous` 放弃协调、可能发生竞争。
- **fail-closed 内核**——已取消的目标永不运行；状态歧义时停止而不是继续消耗。

示例驱动方式：

```cron
# cron——每 15 分钟一个有界回合
*/15 * * * * cd /path/to/project && future loop run --goal <id> --agent-id cron-worker --max-turns 1 >> .future/loop/cron.log 2>&1
```

```ini
# /etc/systemd/system/loop-worker.service
[Service]
Type=oneshot
WorkingDirectory=/path/to/project
ExecStart=/usr/local/bin/future loop run --goal <id> --agent-id systemd-worker --max-turns 1

# /etc/systemd/system/loop-worker.timer
[Timer]
OnCalendar=*:0/15
Persistent=true
```

```yaml
# CI 定时 tick（GitHub Actions）。CI runner 是临时的：跨运行持久化
# .future/loop/（例如 actions/cache），否则每次 tick 都从空账本重新开始。
on:
  schedule: [{ cron: "*/30 * * * *" }]
  workflow_dispatch:
jobs:
  tick:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: future loop run --goal <id> --agent-id ci-worker --max-turns 1
```

调度状态机是对外部驱动器的补充：`future loop scheduler tick|show`
维护重启安全的节奏递进（驱动器需要退避状态时有用），
`future loop scheduler record-host-failure` 记录宿主错过/延迟的 tick，
让存活缺口浮现在状态里而不是无声消失。

### 可选：常驻 runner

守护进程从不是必需的，但有两个常驻便利设施：

- **包装循环**（工作站）：`while true; do future loop run --goal <id>
  --agent-id local-runner --max-turns 1; sleep 300; done`——与 cron 相同
  的有界回合语义，只是不依赖 cron。
- **`future loop serve-status [--port 8791]`**——零依赖、仅 GET 的 HTTP
  仪表盘（`GET /`、`GET /goals.json`）。它是只读投影，永远不是第二真相
  源；可与任何拓扑并行运行，用于可观测性。

完全自定义的 runner 可以用 `future loop worker-bridge`——参考 stdio
契约：bridge 每 tick 向 stdout 输出一行带类型的回合数据包，你的 worker
在自己的运行时里执行有界回合，再写回一行 JSON 结果。每个目标选择
**一个驱动器**（多 agent 场景下每个 `(goal, agent-id)` 一个）——租约
让重叠安全，但单一驱动器让节奏与额度记账可预测。

## 状态布局

```
<cwd>/.future/loop/registry.json                        — 注册表（真相源）
<cwd>/.future/loop/goals/<id>/events.jsonl              — 每目标事件账本
<cwd>/.future/loop/goals/<id>/ACTIVE_GOAL_STATE.md      — 参考兼容投影
<cwd>/.future/loop/runs/                                — run 历史
```

运行时状态绝不写进项目之外；把 `.future/loop/` 加入 `.gitignore`。

## 安装

```bash
make install-skills                      # 首选：链接 /future-loop 技能（无需构建——
                                          # `future loop` 通过统一 CLI 运行）
# 可选：独立二进制（开发用途）
bash scripts/install-future-loop.sh      # CLI → ~/.local/bin/future-loop，技能 → ~/.future/agent/skills/
# 或在 workspace 中构建：
cargo build -p future-loop
```

环境要求与完整产品构建/安装步骤见 [构建与安装](build-and-install.zh-CN.md)。
