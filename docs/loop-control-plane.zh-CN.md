# Loop 控制面（`future-loop`）

> 面向长期 AI agent 工作的本地控制面——在 agent 执行有界回合期间，保持目标、门禁、todos、证据、额度和交接的稳定。

FutureOS 在 `orchestration/loop` 内置了原生 loop 控制面，提供 `future-loop` CLI 与 `/future-loop` agent 技能。它把一段对话变成一个持久、可复盘、可长期运行的目标：目标、todos、人工门禁、监控、证据与完成状态都持久化在聊天之外，由确定性内核决定下一步该做什么——一次一个回合。

> `future-loop` 是基于 [loopx](https://github.com/huangruiteng/loopx) 的 Rust 改写版，针对 FutureOS 做了适配与扩展（项目本地状态、gRPC 执行桥、quota 内核、扩展与多 agent）。

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

`future-loop run` 让一个纯函数、可注入时钟的内核决定：是否运行、为什么、运行哪个 todo——identity 范围边界、人工门禁优先级、修复预算、成果底线、后继 replan 义务、接受度缺口，以及一个把每次决策归入九种 disposition 之一的调度仲裁层（terminal / monitor-wait / active work / consistency repair / human gate / quiet wait / …）。执行是 fail-closed 的：已取消的目标永不运行；状态歧义时停止而不是继续消耗。

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
- Identity 范围的多 agent：agent 范围边界、lane 推荐、supervisor 提案/回执事件、任务租约、带交付契约的交接文档、todo 依赖图、注意力队列 / operator inbox。

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

技能加载后：创建持久化目标 → 把工作拆成 todos（含依赖链与最终交付复制 todo）→ 用 `future-loop run --max-turns 1` 逐回合推进，每步汇报状态与成本。

也可以直接在终端驱动：

```bash
future-loop goal init --objective "..." --cwd /path/to/project
future-loop todo add --goal <id> --text "collect data" --priority P0
future-loop todo add --goal <id> --text "write report" --priority P0 \
  --blocks <collect-todo-id> --verify "test -f report.md"
future-loop status --goal <id>
future-loop run --goal <id> --model future/deepseek-v4-flash --max-turns 1
```

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
bash scripts/install-future-loop.sh        # CLI → ~/.local/bin/future-loop，技能 → ~/.future/agent/skills/
# 或在 workspace 中构建：
cargo build -p future-loop
```

环境要求与完整产品构建/安装步骤见 [构建与安装](build-and-install.zh-CN.md)。
