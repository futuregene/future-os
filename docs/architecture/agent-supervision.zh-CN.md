# Agent 启动仲裁与自愈

> [English](agent-supervision.md)
>
> 设计日期：2026-10-21。源码基线：`a91fcf0ec`。
> 状态：**P1 与 D6 已落地**（监督器、`Busy` 让位、有界等待、熔断、三入口接线、Agent 自触发续跑，以及 TUI 进程内托管 Agent）。
> 本文把"已落地的设计"与"必须由用户拍板的产品决定"分开写，见 [§2](#2-判定分区设计-vs-产品决定)；待决项汇总在 [§12](#12-待决问题汇总)，实测记录在 [§13](#13-实测记录)。

现在有三个入口会把 Agent 带起来：`future agent`、`future host`、`future tui`。三者都遵守同一个实例锁，但**谁先启动、谁负责、死了怎么办**三件事各有各的写法，缺的正是这一层。本文补的是这一层，不改变实例锁、存储格式和 RPC 契约。

## 1 现状

### 1.1 三个入口的所有权

| 入口 | 进程内跑 Agent | 拉起外部 Agent | 持有的锁 |
|---|---|---|---|
| `future agent` | 它自己 | — | `~/.future/agent/agent-instance.lock`（服务锁）+ socket 的 `.owner.lock`（端点租约） |
| `future host` | 取到服务锁时 | 否 | `~/.future/app/desktop.lock`（宿主锁）+ 视情况服务锁 |
| `future tui` | 是：二进制为 `future` 时经 `crate::agent_host` 内嵌 | 否 | 内嵌时持服务锁 |

三者的判定都收敛在 [`select_agent`](../../packages/rpc/src/connection.rs)：先握手外部服务，失败且确认本地端点缺席（`NotFound`/`ConnectionRefused`）+ [`check_instance_lock`](../../packages/rpc/src/connection.rs) 通过，才发 `LocalStartPermit`，最终由 [`AgentService::with_instance`](../../agent/src/service/mod.rs) 里的 `try_write()` 真正仲裁。

### 1.2 四个缺口

四个缺口今天均已关闭（1–3 由 P1 关闭，4 由 D6 关闭）；保留这段是为了记住监督器取代了什么。两个 TUI 缺口（1 与 4）是由 [§7](#7-tui-改为进程内启动) 关闭的，不是靠改良 sidecar。

1. **只有启动时拉起一次。** TUI 的 [`ensure_agent_running`](../../tui/src/agent_supervisor.rs) 在 `run` 开头调用一次，之后只剩 [grpc_client 的重连探测器](../../tui/src/rpc/grpc_client.rs)（1s→30s backoff，界面显示 "Connection to agent lost"），**永远不会重新 spawn**。Desktop 同理，只在 setup 里调一次。
2. **headless 看不见外部 Agent 的死。** 主循环每秒调 [`check_running`](../../cli/src/headless/agent.rs)，但它只读**自己 host 的 `Phase::Failed`**；attach 外部 Agent 时 `Phase` 恒为 `Ready`，所以外部 Agent 死了 headless 继续 serving，然后每个 RPC 报错。
3. **输掉锁竞争的一方当作失败处理。** `InstanceFile::acquire` 把冲突变成字符串（[instance.rs](../../agent/src/service/instance.rs)），`AgentHost` 把它记成 `Phase::Failed`（终态：清理不确定就保锁、不允许同进程重启），headless 直接退出、TUI 报 `exited with status 1`。**正确行为是让位等待**。
4. **TUI 的 sidecar 随 TUI 死亡。** `OwnedAgent::drop` 会 kill（[agent_supervisor.rs](../../tui/src/agent_supervisor.rs)），于是 headless 的可用性挂在"某个终端窗口还开着"上。

## 2 判定分区：设计 vs 产品决定

| 编号 | 内容 | 性质 | 状态 |
|---|---|---|---|
| D1 | 仲裁沿用 OS 实例锁；不新增 leader 租约 | 设计 | **已认可** |
| D2 | 输掉竞争的一方让位等待，不当作失败 | 设计 | **已认可** |
| D3 | 客户端统一为"探测 → 确认缺席 → 抢锁 → 启动/让位"的监督器 | 设计 | **已认可** |
| D4 | 等待必须有界，且期间界面必须说明（[§5](#5-等待必须有界)） | 设计要求 | **已认可** |
| D5 | 崩溃循环熔断（[§6](#6-熔断)） | 设计 | **已认可** |
| **P1** | 监督器核心 + `Busy` 分类 + 三个入口接线（[§10](#10-落地清单)） | 实现 | **已落地** |
| **D6** | TUI 改为"外部优先，否则进程内启动"，去掉子进程 sidecar（[§7](#7-tui-改为进程内启动)） | **产品决定** | **已落地**：`tui/src/agent_host.rs` 经 `future_host` 内嵌，`tui/src/agent_supervisor.rs` 把 `start_in_process` 装成驱动；独立的 `future-tui` 二进制仍是 attach-only |
| **P2** | 让 Agent 活过 TUI 退出：detached sidecar + `--ephemeral` 空闲退出（[§8](#8-生命周期与所有权)） | **产品决定** | **待用户决定** |
| **D7** | Agent 被**用户主动**停掉后，其他客户端是否自动重启（[§4.3](#43-四种死法要分开)） | 产品决定 | 建议：不自动；**待确认** |
| **D8** | 重启后的续跑语义：谁跑、什么时候算可续跑（§9.1、§9.2） | 设计 | **部分落地**：续跑判据已实现；`cause` 区分尚未实现（见 §12） |
| **D9** | 触发也放进 Agent（用户 2026-10-21 提出） | 产品决定 | **已落地**：Agent 启动后自触发，开关 `~/.future/agent/settings.json` 的 `resumeInterruptedOnStart`（**默认开**，与桌面原有的 `autoResumeInterrupted` 一致）；Desktop 的一次性 pass 已退役 |
| **D10** | 续跑不发用户消息，在 loop 内以 request-only 指令续上（§9.3，用户 2026-10-21 提出） | 产品决定 → 设计 | **已落地**，分三步：接受路径 + 指令、占位 tool 结果、放开 assistant 尾巴 |

P2 与 D7 **不是已定方案**，本文不为它们设计代码路径，只描述选项、代价与影响面。

标注"已落地"的项有代码与测试；标注"已认可"的仍只是方案：D8 的 cause 区分尚未实现。

## 3 判定依据

只用三样东西，不引入第四种状态：

1. **端点**：用 readiness 探测本地 socket/管道，拿 `agent_instance_id`。
2. **实例锁**：`agent-instance.lock` 是 OS 建议锁；持有者被杀时内核自动释放，**天然没有 stale pid 问题**。这是整套设计能成立的前提。
3. **停止信号**：RPC 返回 `agent_stopping`，或 shutdown 原因。这类"正在退"不能被当成"缺席"。

推论：**锁被占但端点还没就绪 = 别人正在启动**。这时既不能启动（会违反单实例），也不能报错（会误报故障）。

## 4 状态机

```
            ┌──────────── readiness 探测（带 instance_id）───────────┐
            │                                                        │
   Ready(compatible) ───────────────► Attached / Owned（健康）
            │  连续失败 / 流断 / 心跳超时 / agent_stopping
            ▼
          Lost ──┬── Absent（连不上 + 锁 free + 端点租约可回收）
                 │        └─► Candidate ─┬─ acquire 成功 ─► Launching ──ready──► Owned
                 │                       └─ InstanceBusy ─► WaitingForOwner
                 ├── Busy（锁被占，端点未就绪）────────► WaitingForOwner
                 └── Blocked ──────────────────────────► NoLaunch（只报告，永不启动）
```

- `Candidate` 之前的抖动：`min(30s, 1s·2ⁿ) × U(0.8, 1.2)`，与 TUI 现有的重连 backoff 同量级，避免两个客户端同一毫秒一起抢。
- `Blocked` 包含：版本/协议/存储契约不符、`agent_stopping`、显式 `--grpc-addr` 不可达、`FUTURE_DESKTOP_AGENT_MODE=external`。这些**永远不触发启动**（现有语义，保持）。

### 4.1 两个分类

- `agent/src/service/instance.rs`：锁冲突是 typed error 而不是文案匹配——`InstanceFile::acquire` 用 `future_rpc::instance_lock::lock_conflict_message` 把 `WouldBlock` 答复出去，`future-host` 再映射成 `Error::AgentBusy`（[`host/src/agent_host.rs`](../../host/src/agent_host.rs)）。
- `host/src/agent_host.rs`：输掉竞争**不得**落进 `Phase::Failed`。`Failed` 是"清理不确定、保锁、本进程内不许重启"的终态；实现上让位的那次尝试停在 `Stopped`，旁边带一个 `yield_reason`（没有单独的 `Phase::Busy`），这样 `start` 与状态页能报告持锁者自己的拒绝，而不是一次没人要求过的取消。

### 4.2 谁负责发起启动

| 入口 | 可内嵌 | 可 spawn 子进程 | 自愈 |
|---|---|---|---|
| `future host` | 是 | 否（自己能内嵌，多进程无收益） | 是 |
| Desktop GUI | 是 | 否 | 是 |
| `future tui` | 是（仅限合并后的 `future` 二进制） | 否 | 是 |
| channels / goal / `future run` / 手机 | 否 | 否 | 只 attach |

生产部署仍建议用 systemd / launchd / NSSM 包住 `future host`：进程管理器不会"自己也死掉"，客户端自愈只该是兜底。

### 4.3 四种"死法"要分开

| 观测到的 | 判定 | 动作 |
|---|---|---|
| 端点消失 + 锁 free（`kill -9`、崩溃、OOM） | 非正常退出 | 进入 `Candidate`，带抖动重启 |
| `agent_stopping` / shutdown 原因 = 宿主退出 | 正常退出，但服务仍被需要 | 重启 |
| shutdown 原因 = 用户主动停止 / 升级 | 用户有意停的 | **不自动重启**（D7）；提示"已停止，重新运行命令或按重试" |
| 版本不符 / 显式端点不可达 / external 模式 | 不是死 | `NoLaunch`，只报告 |

## 5 等待必须有界

**硬规则：任何客户端都不许"无声地一直等下去"。** 每个入口必须同时具备三件事：一个上限、一句随进度变化的话、一条"等不到时怎么办"的出路。

| 入口 | 探测节奏 | 等待上限 | 期间界面/终端 | 对它自己的 run 的影响 | 上限到达后 |
|---|---|---|---|---|---|
| `future tui` | 1s（`WAIT_PROBE_INTERVAL`） | **180s**（`WAIT_FOR_OWNER`） | 状态行：`等待另一个客户端启动的 Agent（pid 1234 @ host-a）… 12s`；从 30s（`WAIT_NOTICE_AFTER`）起追加"冷数据根可能要几分钟，进度见 `<日志路径>`" | 输入仍可编辑；发送被拒并显示同一句话——**不排队、不假装成功** | 以 exit 1 报错并打印同一句话（TUI 的启动发生在界面之前，没有可退回的可用界面） |
| `future host` | 250ms→1s | **180s** | stderr 每 30s 一行，同上信息 | 不接单（它本来就没起来） | **报错退出**（exit 1），打印持锁者 pid/host + 日志路径，交给进程管理器 |
| Desktop GUI | 250ms→1s | **180s** | Agent 状态页显示"等待另一个客户端启动的 Agent"，带 pid/host | 依赖 Agent 的页面不可用 | 进入 `Failed` 并给"重试"按钮；不启动第二份 |

三条配套约束：

1. **上限只约束"等别人"**；等自己的启动走 `AgentHost` 现有的 180s 超时，两者不要叠加成 360s。
2. **每次新的"缺席"观测重置计时**，所以"对方启动失败、锁已释放"能立刻进入自己的 `Candidate`，而不是把 180s 耗完。
3. **不允许把等待写成静默重试。** 等待中任何状态变化（拿到 pid、拿到 instance_id、对方退出、锁释放）都必须反映到那句状态行里。

`pid` / `host` 不引入新文件：现有的 `agent-instance.json` 已经写了 `pid`、`host`、`executable`、`futureHome`（Windows 还有 `startTimeFiletime`），`lock_conflict_message` 就是读它。

## 6 熔断

只统计**本进程自己发起**、且**未到 Ready 就结束**的启动尝试（别人发起的失败不计入，避免互相背锅）：

| 参数 | 值 |
|---|---|
| 计数窗口 | **300s**（滑动） |
| 触发阈值 | 窗口内 **3 次** |
| 打开后冷却 | **60s**，期间不再自动启动 |
| 半开放行 | 冷却结束后**只允许 1 次**尝试 |
| 半开再失败 | 冷却翻倍：120s → 240s → 480s → 上限 **600s** |
| 半开成功 | 计数清零，恢复常态 |
| 显式用户动作 | 立即放行 1 次（重跑命令、TUI 按 `r`、Desktop 的"重试"），不重置计数 |

打开熔断时**必须**打印一次：尝试次数、每次的退出原因摘要、日志路径、以及"这是保护，不是放弃"。熔断不阻止 attach：如果别的客户端在这期间把 Agent 起来了，端点探测成功就正常进入 `Attached`。

进程重启即清零（计数器在进程内存里）。两个客户端各有各的预算，最坏 5 分钟内 6 次尝试，已被抖动摊开。

**可选、非权威**：`<home>/agent/agent-launch.lock` 让同一时刻只有一个客户端在"启动+等 ready"，其余直接等它。它不是仲裁者（仲裁永远是实例锁），只是减少注定失败的初始化尝试；只有测量证明确实存在这种反复尝试时才加。

## 7 TUI 改为进程内启动

> **已落地（D6）。** TUI 现在就是这么做的：`tui/src/agent_host.rs` 在进程内托管完整 Agent（只有合并后的 `future` 二进制可以），`tui/src/agent_supervisor.rs` 负责 attach → 等待 → 启动。下面的内容是当时被接受的那个方案，保留其代价分析。

### 7.1 方案

TUI 采用与 headless 完全相同的策略：外部可达就 attach；缺席且锁自由就在**本进程内**跑 `AgentService`；锁被占但端点未就绪就等待（§5）。子进程 sidecar 与 `check_sidecar_program` 一并移除。

### 7.2 可行性

- 依赖方向成立：`tui → host → agent`，无环（`cli → tui` 不变）。
- `AgentHost` 是进程级单例 + `configure_driver`，TUI 已经是"每个入口各自配一次 driver"的形状，接入方式与 [headless 的 `drive`](../../cli/src/headless/agent.rs) 相同。
- 判定逻辑零新增：`select_agent` 已经覆盖 external-first。

### 7.3 硬约束

**只有 unified `future` 二进制能内嵌。** Agent 会 `current_exe()` 重新执行自己当 Linux 沙箱 helper（[runner.rs](../../agent/src/sandbox/linux/runner.rs)），而 `--linux-sandbox-helper` 的分派在 [cli/src/main.rs](../../cli/src/main.rs)；shell 工具也依赖当前 exe 所在目录进 PATH。独立 `future-tui` 二进制内嵌会同时打断 Linux 沙箱与 CLI 自调用。

⇒ 独立 `future-tui` 必须保持 **attach-only**，并说明原因（与今天 `check_sidecar_program` 的提示同一形态）。

### 7.4 代价

| 代价 | 说明 | 处理 |
|---|---|---|
| 生命周期耦合 | sidecar 崩了 TUI 还活；内嵌后 `abort`/段错误会带走 TUI（`catch_unwind` 只拦 Rust panic） | 接受；TUI 崩溃钩子本来就要恢复终端 |
| 退出要有界 | 内嵌 = 退出时要等 `stop_until`（10s），失败即终态 | 先离开 raw mode/恢复终端，再有界停机；停机失败就带错误退出，不继续假装正常 |
| **stderr 会花屏** | `environment::report` 是 `eprintln!`（[environment.rs](../../host/src/environment.rs)） | 加 report sink：进 TUI 状态区 + 日志文件，同时开文件日志；这是本方案必须先做的前置 |
| 冷启动要能看到 | TUI 现在 5s 没响应就报错；内嵌走 180s 预算 | 界面先立起来，状态行说明"Agent 正在启动"，30s 后补日志提示 |
| 常驻内存变大 | TUI 原本是最轻的入口 | 提供 `FUTURE_TUI_AGENT=attach-only`（或 `--attach-only`）逃生口 |
| 一次性 `-p` 模式 | 每次多一套冷启动 | 见 §12 待决 |

### 7.5 收益

- 三个入口收敛成一条策略，少一份会与 `select_agent` 分叉的实现。
- TUI 里不再有"子进程的生命周期"这类问题（P2 对 TUI 不再需要）。
- 不再出现"headless 的 Agent 其实住在某个终端窗口里"的所有权倒置。

## 8 生命周期与所有权

> **待用户决定（P2）。** 用户此前对桌面端的态度是"关掉就关掉"，而本项会让 Agent 活过 TUI 退出，因此**不能当作已定方案**。

问题陈述：D6 采纳后，TUI 内嵌的 Agent 仍然随 TUI 退出而死。若此刻 headless 在 attach，它会在下一轮监督里**重启** Agent —— 以"打断运行中的任务"为代价换取服务不中断。

两个选项：

| 选项 | 做法 | 代价 |
|---|---|---|
| A（当前语义，D6 之后自然成立） | Agent 随启动者死；其他客户端按 §4.3 重启 | 运行中的 run 被中断；但语义简单、无残留进程 |
| B（P2） | sidecar 与父进程解耦（Unix `process_group(0)`、Windows 新进程组 + `CREATE_NO_WINDOW`），`Drop` 不再 kill，改为带 `--ephemeral` 的内部参数：**无活跃订阅、无 running/queued run、无待审批**持续 `IDLE_GRACE`（120s）后自行正常退出 | 多一个可能残留的进程；需要给 Agent 加"活跃订阅计数"；用户"关掉就该关掉"的预期被改变 |

无论 A 还是 B，**空闲退出的判断都必须由 Agent 自己基于真实状态做**，不能由客户端"走之前问一句有没有别人"（那个窗口有竞态）。

## 9 数据安全与客户端一致性

- 新 owner 只能经 `with_instance` 取锁后碰数据库；旧进程只要还活着就一定持锁（stopping 期间锁保留到最后一个写入者结束），因此**不可能并发打开 DB**。被 SIGKILL 的走 WAL 恢复，未完成运行收敛成 interrupted。
- 新实例 = 新 `agent_instance_id`：客户端必须丢弃 pinned channel、流游标和内存队列假设，重新握手。RPC 已经很明确——经校验的通道不得透明重连到替代实例（[transport.rs](../../packages/rpc/src/transport.rs)）。
- 停机期间的 `agent_stopping` 必须被当作"等等再看"，不能当成 `Absent` 触发抢占（现有 `Blocked` 分支已如此，监督器沿用）。

### 9.1 重启之后，会话怎么继续

[#1277](../../agent/src/rpc/resume.rs)（当时的实现，现已由 Agent 侧接替）给了正确的基础，但它的边界决定了"重启只是一次抖动"这个结论只在特定条件下成立。本设计必须显式依赖它，否则自愈会把"会话继续"变成"会话停在半路"。

#### 它实际做了什么

`resume_interrupted_runs_at_launch` 不接续原流：

1. 把中断的那一行结算成 `cancelled` + `error_type=interrupted`；
2. 提交一个**新 run**，prompt 为 `继续上一个任务。` + "上一轮运行在关闭时被中断了（工具可能已经执行了一部分）" + 最近 6 条执行摘要；
3. 模型带着完整 transcript 重做这一段。

真正的"流式不中断"是另一条路：**Agent 还活着**时客户端重连，collector 从 `after_idx = last_idx` 续流；Agent 一旦重启，attach 返回 `RunGone`，只能从 journal 恢复。

因此准确表述是：**会话不终止、历史不丢，但当前这一轮会重做一遍**。用户看到的是一次可见的断开 → 重启 → 新 run 起来。

#### 让它成立的四个条件

| 条件 | 现状 | 本设计需要做什么 |
|---|---|---|
| 重启后的会话里有人跑续跑 pass | 历史上只有 Desktop 跑；headless 只跑 `reconcile_interrupted_runs`（结算/重接流，不发 prompt），TUI 完全没有 | **已落地**：由 Agent 自己在 Ready 后触发，不依赖任何宿主在场；Desktop 的 pass 已退役，避免同一会话收到两条续跑（见 §9.2） |
| 中断要留下"可续跑"的状态 | 优雅停机走 `cancel_all` → 写 terminal → `interruptedRun` 为 null → `plan_resume` 判 `Terminal` → **不续跑**；只有异常死亡（`kill -9` / 崩溃 / OOM）留下 `interrupted` | 优雅退出时按 cause 标记：`agent_shutdown` → 可续，`user_cancelled` → 不可续（见下面「前提一」） |
| 会话必须在 `app.db` 里有 thread + run 行 | run 行由宿主提交 prompt 或 import 生成（discovery 每 60s） | 只有 TUI 在跑、没有任何宿主的机器没有候选；自愈首次拉起时补一次 import |
| 其余产品门槛 | 最新 run、非 `waiting_approval`、7 天内、`autoResumeInterrupted`（默认开，等于代用户花钱） | 不改 |

第 2 行是最容易改坏的一处：TUI 过去用 `SIGKILL` 杀自己的 sidecar（`OwnedAgent::drop`），**不走优雅停机**，所以 journal 留着未终止的 run，恰好可续跑。D6 落地后 Agent 在进程内，正常退出会走优雅停机，run 变成 `cancelled`，**反而不可续跑**——这正是必须记录 cause 的原因（§9.2 前提一）。

#### D8 的判据（已修正）

最初草稿按"还有谁在看"（活跃订阅数）来定：有别的客户端订阅就标 interrupted，独居就标 cancelled。**这个判据是错的**，原因两点：

1. 它恰好会在用户要的场景里失效（TUI 先死、Agent 独居时关掉，就永远不会续）；
2. 它把"用户的意图"猜成了"还有谁在看"——一个客户端连着不代表用户想让任务继续。

改用 cause（[§9.2](#92-落点tui宿主还是-agent) 的前提一）：**为什么结束**决定续不续，**不是谁在看**。活跃订阅数只留给 P2 的空闲退出用，两个问题不要共用一把尺子。

`autoResumeInterrupted`（即将改为 `~/.future/agent/settings.json` 里的 key）继续作为"允许代我发 prompt"的同意门槛。

### 9.2 落点：TUI、宿主，还是 Agent

三个位置在技术上都能做。区别不在"能不能"，而在**去重的原子性**和**同意从哪里来**。

#### A 只放在 TUI（可行，且不需要 host 依赖）

现有 RPC 已经够用，无需新增接口：

- `list_sessions` 给出每个会话的 `updatedAtMs` 与 `isStreaming`（[session_lifecycle.rs](../../agent/src/rpc/commands/session_lifecycle.rs)）—— 年龄门槛和"正在跑"的判断不需要 app.db；
- `get_state` 给出 `interruptedRun`（未终止 run）与 `pendingApprovals`；
- `prompt` 提交续跑。

TUI 只需要读同一个同意开关：`future-app-settings` 只依赖 rusqlite + serde（[app-settings](../../packages/app-settings/Cargo.toml)），读的是 `app.db`，**不需要引入 `future-host`**。

代价：TUI 之后出现第三份策略实现，而且它看到的是**会话**而不是"对话"——会覆盖 channels / goal / CLI / 子会话产生的会话，这些会话的续跑未必是用户想要的。

#### B 放在宿主（现状）

优点是策略、行结算、`resume:<run id>` 关联、UI 表现全在 `app.db` 一侧。缺点已在 §9.1 第一行说过：没有宿主在跑的机器（只有 TUI，或只有 `future agent`）永远不续跑。

#### C 放在 Agent（技术上最干净）

Agent 已经具备全部判据，不需要 app.db：

| 判据 | Agent 侧的现成能力 |
|---|---|
| 这个会话有未终止的 run | `unterminated_run_id`（[manager/mod.rs](../../agent/src/session/manager/mod.rs)），只读 run 标记，不加载全文 |
| 它现在没在跑 | `runtime.snapshot()` / `is_streaming` |
| 它卡在审批上 | `approval_gate.pending_for_session` |
| 年龄门槛 | 会话 `updated_at` |
| 执行摘要 | 自己的 journal（比 `store::list_run_events` 更权威） |

而且它已经拥有"新 run 把上一个中断 run 关成 `interrupted_by_restart`"这条逻辑（`new_run_closes_a_prior_interrupted_run`），续跑就是它的自然延伸。

#### 关键区别：去重靠不靠原子性

今天 `get_state` → `prompt` 是**两步**。两个客户端（或客户端 + 宿主）同时看到 `interruptedRun`，都能通过检查，然后一个排队、一个执行，**同一个任务跑两遍**。`new_run_closes_a_prior_interrupted_run` 只能在第一步之后清掉标记，管不住这个窗口。

只有"判定 + 提交"在同一个锁里，才能结构性地去掉重复。Agent 是唯一能提供这把锁的地方。

#### 决定：触发也在 Agent（用户 2026-10-21 提出）

用户提出把触发也放进 Agent 自动完成，场景是：`future agent` 先跑起来 → TUI 连上去跑一个任务 → TUI 挂掉 → 关掉 Agent → 再启动 Agent，中断的会话自己接着跑。

采纳，因为它是三个位置里**唯一能结构性去重**的：单实例持锁 = 单一触发器，"两个客户端同时看到 `interruptedRun` 然后各发一次"的窗口不存在。代价是下面四个必须一并处理的事实。

| 职责 | 归属 | 理由 |
|---|---|---|
| 判定 + 提交续跑 prompt，**原子完成**，且由 Agent 自己在 Ready 后触发 | **Agent** | 只有它有 journal、活跃 run、审批和那把锁；也只有它在裸 `future agent` 时存在 |
| 同意开关 | **`~/.future/agent/settings.json`**（新 key，如 `resumeInterruptedOnStart`） | Agent 不能读 app.db：裸 `future agent` 可能根本没有它，且那是 app 的存储（见下面「前提二」） |
| `resume:<run id>` 类关联与行展示 | 宿主保留，作为**消费者** | 它本来就维护这些行 |
| 旧的一次性 pass（`resume_interrupted_runs_at_launch`） | **退役** | 两处并存就是双重提交（就是上面那个窗口） |

#### 前提一：停机时必须区分"为什么结束"

实测：`request_shutdown` → `drain` → 每个会话 `abort()` → 活跃 run 走正常终态，状态是 `cancelled`（[session_prompt.rs](../../agent/src/rpc/session_prompt.rs)），而 `request_abort` **没有 cause 参数**。所以**"关掉再启动"目前不会续跑**：正常关闭后 `interruptedRun` 为 null，`plan_resume` 判 `Terminal`。只有 `kill -9` / 崩溃 / OOM（journal 留着 `run_started` 没有 `run_terminal`）才是 `interrupted`。

因此 D8 的判据不是**"还有谁在看"（订阅数），而是"为什么结束"（cause）**。订阅数判据恰好会在用户这个场景里失效：第 3 步 TUI 死了，第 4 步 Agent 独居，按订阅数会判成 `cancelled`。

需要给 abort 加一个 cause：

| cause | 终态 | 下次启动续不续 |
|---|---|---|
| `user_cancelled`（UI 停止、取消排队、客户端发 `abort`） | `cancelled` | **不续**。用户叫它停。 |
| `agent_shutdown`（宿主退出、Ctrl+C、升级、关窗口） | `interrupted_by_restart`（或 `cancelled` + 可续标记） | **续** |
| 没有终态（进程被杀/崩溃/OOM） | 已是 `interrupted` | **续** |

这样用户五步里的第 4 步（正常关闭）也成立。风险仍是"用户为了停一个失控任务而关 Agent，下次启动又自动续上"；缓解手段就是下面的护栏 + 续跑出的新 run 可以随时取消。

#### 前提二：开关跟着执行者走（不是 D9 的额外成本）

容易误以为这是 D9 特有的代价；实际上把机制放在 headless 或 TUI 里也会遇到同一个问题。

真正的事实只有一个：**同意必须放在执行者读得到的地方**。

- 触发在客户端 → 执行者是客户端，开关放 `app.db` 完全合理（headless 启动时就会 `initialize_app_store`）；
- 触发在 Agent（D9）→ 执行者是 Agent，而 **`future-agent` 根本不依赖 `future-app-settings`**（[agent/Cargo.toml](../../agent/Cargo.toml)），裸 `future agent` 也可能没有 app.db。

所以这不是选型的扣分项，而是选型的直接后果。而且它已有现成做法：`set_default_model` 就是一个 **sessionless RPC → 读 `settings.json` → 改字段 → 存回**（[providers.rs](../../agent/src/rpc/commands/providers.rs)）。新增一对 `get/set_resume_policy` 与它同形，Desktop 设置页照现在的写法调即可。

要做的只有两件小事：`~/.future/agent/settings.json` 新增一个字段（与 `compaction` / `retry` / `maxTurns` 同级），以及把旧 `app.db` 的值一次性映射过去，避免升级后设置被静默重置。

#### 前提三：护栏（没有它们就是无界花钱）

最坏情形是一个任务稳定把 Agent 弄死（OOM）：重启 → 自动续 → 又OOM → …。所以：

| 护栏 | 值 |
|---|---|
| 单会话续跑次数 | 24h 内最多 **2** 次 |
| 续跑链深度 | 续跑产生的新 run 又中断时，连续 **2** 次就停 |
| 每次启动总预算 | 最多 **3** 个会话 / **2** 并发 |
| 年龄 | 沿用 **7 天** |
| 跳过 | 等待审批的会话、`ephemeral` 会话、子会话（除非根会话也存在且命中） |
| 报告 | 每续一个打一行（会话、原 run、新 run）；放弃时打一行原因 |

触发时机的硬要求：Ready **之后**、监听已起（客户端能看到这个 run）、**不阻塞 readiness**、纳入 `service_work`（停机要等它收尾），并发受限（冷服务器上不要一次拉出十个模型请求）。

#### 前提四：客户端要能认出"这不是我说的话"

如果续跑走"发 prompt"的形态，那条 prompt 是 **user 消息**，会原样进 transcript，三端会渲染成用户自己打的字，必须靠标记区分。

D10（[§9.3](#93-不发用户消息在-loop-里续上)）直接取消这个形态：续跑只追加 request-only 指令，transcript 里根本没有这条消息。但 `run_started` 仍要带 `auto_resume_of`，因为客户端还要把"这段是自动继续的"画出来。

#### 已知取舍

| 取舍 | 说明 |
|---|---|
| 无人在场也会花钱 | 服务器重启后自动继续；这是用户要的行为，但仍需在开关旁说明 |
| 覆盖非交互来源的会话 | channels / goal / CLI / 子会话的会话也会被续；若接受不了，缩成"只续曾经有交互客户端加入过的会话"（需要新增标记） |
| 与 P2 的关系 | 两者都需要 Agent 知道"还有谁在看"，但用途不同：P2 用于空闲退出，续跑用 cause。不要合并成一个判据。 |

#### 抖动上界

一次重启对用户可见的代价 = 发现失踪（探测节奏，≤1s）+ 等待/启动（≤180s，通常数秒）+ 冷启动 + 一次新 run 的模型调用。**会话本身与历史不受影响**；被打断的工具调用由续跑 prompt 的执行摘要兜住，但**不保证工具只执行一次** —— 这一点必须在续跑 prompt 与本文里都说清（现有 prompt 已经这么写）。

### 9.3 不发用户消息：在 loop 里续上

结论：**可以，而且代码里已经有这个形状**；原先有两处硬阻断。**已落地**：接受路径、占位 tool 结果与放开后的守卫都已就位，下面的顺序正是分步落地时逐步验证的顺序。

#### 已有先例：request-only 消息

[run_loop.rs](../../agent/src/agent/run_loop.rs) 在"重连后继续回答"时插入一条只存在于本次请求里的消息，注释写得很直白：

> This is request-only recovery context, not a fabricated user submission in the durable conversation or UI.

这正是"不落用户消息、只在请求里说明一句"的现成机制，只是现在只在流式重试里用。

#### 阻断一：loop 拒绝以 assistant 结尾

`run_streaming_with_messages` 开头就拒绝：

```rust
if last.role == "assistant" {
    return Err(anyhow!("Internal error: conversation ended with an assistant message. \
         This is a bug — please report it."));
}
```

而 run 中途死亡留下的 journal，尾部恰好是 assistant —— assistant / tool 条目在流式过程中就已经落盘。

#### 阻断二：`prompt_internal` 必然造一条 user 消息

它 build → push 进 `self.messages` → 立刻持久化 → 广播 `user_message`。没有"无 prompt 的 run"这个入口。

#### 尾部有三种形态，不能只处理一种

| 尾部 | 请求是否合法 | 续跑要做什么 |
|---|---|---|
| tool 结果 | 合法 | 追加 request-only 指令即可 |
| assistant（纯文本/推理） | 被阻断一拒绝 | 放开守卫 + 追加指令；或把这段 assistant 移出请求、内容并进指令 |
| assistant 带 tool_calls、没有结果 | **非法**（HTTP 400：tool_calls 必须跟 tool 消息） | 补一条占位 tool 结果 + 指令 |

第三种最容易漏：现有取消路径已经知道这一点（run_loop 为未执行的调用补 `[Tool execution cancelled …]`），但**崩溃路径不会补**。而崩溃时"这个工具到底跑没跑"是未知的，占位文本必须写成不确定，不能抄"未执行"。

#### 改动形状（D10）

1. Agent 内部新增"续跑 run"的接受路径：不造 user 消息，但仍走 accept-before-mutate、`run_lease`、`start_run` 广播与 run 行创建。
2. 守卫从"一律拒绝 assistant 结尾"改为"只有 journal 证明它是未终止 run 的尾巴时才允许"。**不能删掉那段 assistant** —— 那是模型已经产出的内容，删掉就是丢历史。
3. 在请求尾部追加 request-only 指令，内容是现有续跑 prompt 的三段：继续、为什么中断、已执行摘要。只是它不再进 transcript。
4. `run_started` 带 `auto_resume_of`，三端把这类 run 渲染成"自动继续"。

#### 收益

- **缓存**：只在尾部追加，前缀不变。放进 system prompt 会改前缀，按工作区记忆那是 48 倍成本量级。
- **语义忠实**：transcript 读起来是"同一个回合被打断后继续"，而不是凭空多一句用户发言。
- **没有假气泡**，客户端不需要"隐藏某条用户消息"的特例。

#### 代价

- 动的是最敏感的接受路径（`run_lease` / journal / broadcaster / run 行），比提交一条普通 prompt 风险高。
- `thread-projection` 与三端 UI 目前都以"run 跟着一条 user 消息"为默认形状，需要能处理无 user 的 run。
- 若某个 provider 既不吃 assistant 尾巴、也不吃追加指令，就得退回"发一条 prompt"的形态；两条路要能共存，不能只有一条。

#### 建议

D10 优先于"发 prompt"的初版：先做"尾部是 tool"和"assistant 带 tool_calls 补占位结果"这两种（改动最小，**不必放松守卫**），再放开纯 assistant 尾巴。

实际落地时，守卫在最后一步之前多了一个中间形态：带未应答 tool_calls 的 assistant 尾巴**本身就是** assistant 结尾，所以第二步只允许这一种——仅在续跑时，且仅当这条尾巴上确实有占位结果能应答的调用。第三步才把允许范围扩到续跑路径上的任意 assistant 尾巴。这个放宽从来不是通用的：`continue_interrupted_run` 会重新读 `unterminated_run_id`，journal 证明不了就拒绝；provider 两种都不吃时，仍然退回旧的"发 prompt"形态。

## 10 落地清单

### P1（已落地）

| 改动 | 位置 |
|---|---|
| `Supervisor` / `Decision` / backoff；`StartupBlocked` 细分 `Busy` 与 `Blocked` | `packages/rpc/src/supervision.rs`（`select_agent` 在 `packages/rpc/src/connection.rs`） |
| 锁冲突的 typed error | `agent/src/service/instance.rs`（`lock_conflict_message`）、`host/src/agent_host.rs`（`Error::AgentBusy`） |
| 让位而非失败：`Stopped` + `yield_reason`、`detach_attached()` | `host/src/agent_host.rs` |
| 一次性启动 → 常驻监督；等待状态行（§5） | `tui/src/agent_supervisor.rs` |
| `check_running` → 监督 tick（探端点，而不只看自己的 phase） | `cli/src/headless/agent.rs` |
| 同一 tick 接入 | `desktop/src-tauri/src/agent_supervisor.rs` |
| 熔断（§6） | 监督器内部，不落新文件 |
| `CancelCause` 贯穿 abort → 终态记录（D8 的前提） | `agent/src/rpc/session.rs`、`agent/src/runtime/`、`agent/src/session/entry.rs` |
| Agent 自触发续跑（§9.2）：Ready 后判定 + 原子提交，含护栏 | `agent/src/service/startup.rs`、`agent/src/rpc/`、`agent/src/config/mod.rs` |
| `~/.future/agent/settings.json` 新增开关 + 旧 `app.db` 值的一次性映射 | `agent/src/config/mod.rs`、`packages/app-settings`、桌面设置页 |
| 续跑 run 的无 user 接受路径 + request-only 指令 + assistant 尾巴守卫（D10） | `agent/src/rpc/session_prompt.rs`、`agent/src/agent/run_loop.rs`、`agent/src/session/entry.rs` |
| 崩溃尾部缺 tool 结果时补"不确定"占位 | `agent/src/agent/run_loop.rs`（现有取消路径的邻位） |
| `run_started` 带 `auto_resume_of`，客户端渲染为"自动继续" | `agent/src/session/entry.rs`、`tui/`、`desktop/`、`mobile/`、`packages/thread-projection/` |
| 旧的一次性 pass 已退役（`resume_interrupted_runs_at_launch` 与 `host/src/agent_bridge/resume.rs` 一并删除） | `desktop/src-tauri/src/lib.rs`；新实现在 `agent/src/rpc/resume.rs` |

### 后续

- **D6**（TUI 内嵌）：已落地——`future-host` 已在 `tui/Cargo.toml`，`tui/src/agent_host.rs` 驱动它，spawn 分支与 `check_sidecar_program` 已删除。
- **P2**（生命周期）：`--ephemeral` + 活跃订阅计数 + 空闲退出。尚未实现：`future agent` 没有 `--ephemeral` 参数。

## 11 验收

全部是进程级测试，形态参照 `agent/tests/service_startup.rs`（实例锁与启动竞争）与 `cli/tests/remote_start_refusal.rs`（入口在任何 Agent 启动之前的拒绝）：

1. **并发冷启动**：同一 fresh home 同时起 headless + tui → 恰好 1 份 Agent、另一方 attach、**没有任何一方报错退出**、instance id 一致。
2. **owner 被 `kill -9`**：剩下的一/两个在预算内拉起 Agent，新 instance id，DB 完整，旧 run 变 interrupted。
3. **双 survivor**：headless + tui 都 attach，杀掉 owner → 只起 1 份，另一方等待/attach。
4. **等待有界**：假 owner 持锁迟迟不就绪 → 三个入口各自打印 §5 的那句话，并在上限后按表行动。
5. **熔断**：假 Agent 秒退 → 第 3 次后停手、打印原因；冷却后半开只试 1 次。
6. **慢启动**：假 Agent 5s 才 ready，同时另一客户端启动 → 不抢锁、不误判缺席。
7. **D6 采纳后**：TUI 退出不带走 headless（若 P2=A 则验证"headless 重启且任务被标记 interrupted"）。
8. **手动 `future agent` Ctrl+C**：客户端不自动重启，打印提示（D7）。
9. **用户提的五步**（§9.2 场景，必须端到端验证）：`future agent` 起来 → TUI 接入跑一个长任务 → `kill` 掉 TUI → **正常**关掉 Agent → 重新 `future agent` → 那个会话必须自动接着跑。
10. **cause 区分**：同一个 home，一次用户主动取消 → 重启**不**续；一次正常关闭 → 重启**续**。
11. **护栏**：模拟“续跑本身又被中断” → 连续 2 次后停止；模拟 3 个候选会话 → 只起 3 个、并发 ≤ 2；每个都有日志行。
12. **标记**：TUI/Desktop 显示续跑的消息时带“自动继续”标识，不是用户的字。
13. Windows：命名管道 + `.owner.lock` 回收 + 新进程组。

## 12 待决问题汇总

需要用户明确回答的（按影响面排序）：

1. ~~D6：TUI 是否改为进程内启动？~~（**已定**：已落地，见 §7）
2. **P2：Agent 该不该活过 TUI 退出？** 选项 A（随启动者死 + 他人重启，会打断当前这一轮，但会话会续上，见 §9.1）vs B（detached + 空闲退出）。
3. **D7：用户主动停止后，别的客户端要不要自动把它拉起来？** 建议不自动（尊重"我说停就停"），但需要确认。
4. ~~续跑的默认值~~（已定）：`resumeInterruptedOnStart` 默认**开**，与桌面原有的 `autoResumeInterrupted` 一致，避免搬动机制时静默丢掉既有行为。
5. **续跑要不要覆盖非交互来源的会话？**（channels / goal / CLI / 子会话）。默认建议：先只续"用户眼前对话"能识别的会话，其余需额外标记。
6. ~~P1 开工许可。~~（**已定**：已落地，见 §10）

不需要用户决定、仅记录为实测可调的参数：等待上限 180s、熔断阈值 3 次/300s、冷却 60s 起步、`IDLE_GRACE` 120s、`future tui -p` 是否也内嵌。

## 13 实测记录

以下每一行都是**真跑出来的**，不是读代码推断。方法：release 二进制、每个用例一个独立 `FUTURE_HOME`、
与开发者真实 `~/.future` 隔离；假模型端点是一个本地 stub（不为验证消耗真实凭据）。

| # | 场景 | 改动前 | 现在 |
|---|---|---|---|
| 1 | `future agent` 拥有 Agent，headless 附着 → `kill -9` agent | 30s 内无人接管，headless 只反复重连 | **3s 内** headless 拉起新 Agent（`the Agent is gone; starting a new one` + 新 instance id） |
| 2 | headless 拥有 Agent，TUI 附着 → 结束 headless | 40s 内无人接管，TUI 停在 `Connection to agent lost` | **1s 内** TUI 拉起新 Agent |
| 3 | 锁被占用、默认端点不存在（Agent 只绑 TCP）→ 启动 TUI | TUI **exit 1**，报 "Cannot start a local Agent" | TUI **不退出**，等待并打印 `waiting for the Agent another process is starting (30s of 180s)` + 持锁者 pid/host |
| 4 | 同上，另一个 headless | exit 1 | 同样等待（`supervise()` 路径） |
| 5 | 开关 `resumeInterruptedOnStart=true`，有中断 run 且有模型 → 启动 `future agent` | 无续跑 | 日志 `1 conversation(s) checked, 1 interrupted, 1 continued`（没有中断 run 的健康启动不打印任何内容）；假端点收到 1 次请求，最后一条 user 消息是续跑 prompt；会话 `interruptedRun` 变为 null、`queryCount` 1→2 |
| 6 | 同上但**开关未开**（默认） | — | **0 次**续跑，会话如实保持 `interrupted_by_restart` |
| 7 | 中断会话没有可用模型 | — | 如实报告 `has no model configured; left interrupted`，不假装成功 |
| 8 | 中断时间超过 7 天 | — | 跳过（计入 `checked`，不计入 `continued`） |

同轮修掉的测试/环境问题：

| # | 问题 | 根因（实测） | 修法 |
|---|---|---|---|
| 9 | `models::tests::registry_injects_future_models_from_disk_cache` 并行偶发（HEAD 10 次 2 次失败） | 多个测试模块在没有 `TestHome` 的情况下构建 `Registry`，其后台刷新线程会在「reset → 断言」窗口内把另一份目录写进进程级内存缓存 | 测试构建里给读/发布加同一把 `CACHE_ACCESS`（可重入），复位、发布、构建互斥；并给刷新线程加代际号，跨过复位的旧线程不再发布 |
| 10 | 长路径 worktree 下 `grpc::tests::the_public_serve_wrappers…` 失败 | `$HOME/.future/run/agent.sock` 超过 unix `sun_path`（macOS 104 字节）：worktree 下 113，主工作树 78 | 端点路径超预算时回退到临时目录下的**每用户短目录**＋路径哈希（仍按实例隔离）；短路径布局不变 |
| 11 | `cli/tests/bin.rs::config_wizard_reports_the_missing_console…` 挂死 | stdin 是管道时，unix 侧仍让 `rpassword` 打开 `/dev/tty`；进程有控制终端时永久阻塞（Windows 侧本来有防护） | 在调用平台读取前就拒绝非终端 stdin，与 Windows 报同一句错误 |

改动前的那三行（1、2、3）是本次修复的起点：它们记录的是**修复前**的行为，用来证明修复真的改变了行为，
而不是"本来就能用"。

已知未覆盖：Windows 命名管道与进程组；有真实模型凭据时的 `cause` 区分（见 §12）。
