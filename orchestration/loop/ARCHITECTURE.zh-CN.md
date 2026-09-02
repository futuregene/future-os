# Loop 架构：一个 kanban 工具，不是一个规则引擎

本文档阐述 loop 控制平面的设计原则。运维模型（概念、命令、多 agent 编排）见
`docs/loop-control-plane.zh-CN.md`；面向 agent 的驾驶手册见 `future-loop`
SKILL.md。

## 设计原则

1. **kanban，不是规则引擎。** 内核提供确定性工具——todo 状态、verify 门、
   acceptance 契约、evidence、lease——从不替 agent 判断「你卡住了，停下来重新
   规划」。它计算信号（outcome floor、振荡、失败次数、monitor 停滞、无进展回合），
   并以 **advisory（提示）** 的形式放进 turn envelope（回合信封）；如何处理信号是决策，
   而决策不留在内核里。

2. **agent 是编排者——要有可观测性和控制杆。** 决策者是大模型，不是内核。
   内核只守住**正确性底线**——保证 goal 状态合法的硬约束（verify 门、
   acceptance 契约、终局判定、lease）——从不判断一个*探索性*
   结果*对不对*。但「agent 决策」只有当 agent 能*看见*、能*动手*时才有意义。
   所以 loop 交给编排者的不只是状态，还有控制杆：

   - **观测** — 四个面，不只是 `tail`：
     - **行为流（过程）** — `worker tail` 实时流式查看 worker 的回合日志
       （压缩的工具/用量视图）：看 worker 的*手*；
     - **产物（结果）** — 读 evidence 指向的文件（报告、数据）：这是编排者
       判断*探索性*结果的方式——不是盯过程，而是读*活*；
     - **账本（状态与历史）** — `status` / `diagnose` 按需暴露 todo 状态、
       信号、gate、lease：权威看板；
     - **推送（事件）** — supervisor 通知送达状态转换点（见「run 生命周期与
       编排者通知」）；
   - **steer** — `supervisor steer` 打断/纠正正在运行的 worker，
     `todo update` 在看板上中途调整；
   - **停止** — `worker stop` 由编排者判断后停下 worker；
   - **关闭** — 手动 `todo complete` **有意不**重跑机器 `--verify` 门：
     对探索性交付物，编排者读产物就是判定，内核不做二次猜测。

   因此可观测性和可 steer 性是架构的一部分，不是便利功能：它们让「编排者」
   真正成立。

   **worker 只上浮，从不判断要不要找人。** worker 遇到自己解决不了的事，
   不会去开一个「user gate」、也不判断「这事该人来做」——它**上浮给编排者**
   （一个信号/消息，不是冻结），自己那条道照常。*这件事到底要不要人*，是
   编排者的判断：它可以直接答、调整 todo、换策略，或者——只有到这一步——
   才提给那个人。编排者怎么找到人（以及那要不要冻结什么）**loop 不约束**——
   那是编排层行为，在内核之外。所以内核没有 `blocked_by: human` 概念、也没有
   worker 开启的 gate：只有依赖边（`--blocks`，工作必须等待）和一条可靠的
   worker→编排者上浮通道。人位于监督栈的顶端（见「监督的层级」），*经由*
   编排者触达，worker 从不直接找人。

3. **用户通过 skill 使用 loop。** 用户不直接编排内核；他们说出目标
   （`/future-loop <任务>`），由 agent——在 `future-loop` SKILL.md 的指导下——
   拆解目标、驱动 run、读取信号、上浮决策点。SKILL.md 负责「什么时候做什么、
   怎么拆解、怎么 steer」（编排层）；CLI/内核是底层机制（状态 + 硬检查）。决策
   指引放在 SKILL.md，正因为那才是决策者（大模型）会读的地方。

4. **一切能力经 CLI 暴露。** `future loop <cmd>` 是控制平面唯一的机器接口：
   每一次状态变更——goal/todo 变更、gate 裁决、lease、steer、停止、完成——都是
   一次 CLI 调用，确定性、可在事件账本中审计。skill 代表 agent 驱动 CLI；人类
   operator 敲同样的命令；dashboard（`ui`）**有意只读**（变更留在 CLI）。一个
   接口，没有旁门：loop 能做的任何事，CLI 都能表达。

5. **状态持久化，上下文可重放。** goal、todo、evidence、信号都持久化在对话之外
   （event-sourced、可重放）。**账本（含 evidence）是状态与历史的权威**；session
   连续性是有价值的**缓存**，当封存期间世界变了、缓存失效时，以账本为准、经增量
   信封刷新（见下文「worker 会话生命周期」）。是否 resume 一个会话，始终是调用方
   的选择，不是内核的。

## 内核行为的两类划分

内核只做两类事，用一个问题区分：**违反它，goal 的状态是否就非法了？**

- **底线（floors）** —— 强制执行；违反就让 goal 陷入非法状态。内核唯一
  的强制力。
- **仪表（gauges）** —— 算好递给 agent；*拿它怎么办从来不是内核的事*。
  有的供策略参考、有的掐断预算，但都是信息，不是强制。

### A. 底线（强制：kanban 的确定性语义）

这些是状态一致性的硬约束，弱化它们会让 goal 陷入非法状态：

| 底线 | 为什么必须保留 |
|---|---|
| succession closure missing | 完成必须声明 successor / no-follow-up，否则 goal 永远无法关闭 |
| acceptance gap | 硬契约：acceptance token 必须满足 |
| 终局判定 | kanban 确定性状态：所有 todo done + gaps 满足才关闭 |
| blocker | 阻塞器 |
| work leased to others | 并发正确性 |
| verify 门 | 正确性：exit 0 才 complete |
| lease | 并发互斥 |

机器验证有一个有意的互补面：**编排者判断，经交付闭环记录**。完成先落到
`delivered` 待决态；编排者（或 operator）读产物后裁决为
`verified / failed / rework`——内核记录这个判断，不做二次猜测（手动
`todo complete` 不重跑 `--verify`）。

### B. 仪表（信息：永远不强制 replan）

内核算出并浮现的一切，agent 读了自行处置——或忽略。策略提示与花费封顶
都是内核算好递出的量，区别只在 agent 怎么用，不在种类。没有一个决定
「你卡住了 → replan」。

*软仪表（策略提示）* —— 以 advisory 形式浮现在 turn envelope，也可经
`status` / `diagnose` 查询：

| 仪表 | 检测条件 | agent 看到的提示 |
|---|---|---|
| outcome floor | `surface_streak >= threshold` | 连续 N 个回合无实质产出 |
| 振荡（oscillation） | A→V→A→V 交替 | 交付在 accept/reject 之间反复 |
| repair budget | `failed_attempts > MAX` | 失败 todo 仍 runnable（不再过滤），提示「已失败 N 次」 |
| monitor 停滞 | `consecutive_no_change >= 3` | quiet wait + 「考虑让 watch lane 过期」 |
| 无进展（no-progress） | `no_progress_turns >= 2` | 「考虑用全新会话重启」 |

*硬仪表（花费封顶）* —— 封顶消耗；某个触发后 goal 等待，接下来怎么办
由编排者（或用户）决定——同样永远不是内核强制的 replan：

| 仪表 | 封顶什么 |
|---|---|
| validation budget | `--verify` 门一直失败仍限制 run loop 的回合数 |
| quota（配额） | should-run 判定、调度拒绝、花费 |
| turn 超时 | 一次一个有界回合 |

唯一重要的区分：底线说*这个状态非法*；仪表说*这有个数*——而对一个数
的回应永远是决策，决策在 agent，不在内核。

## run 生命周期与编排者通知

run 如何发起、worker 如何触达编排会话，属于架构决策（上述原则的实现
机制），在此固定：

1. **run 是 detached（异步）的。** 编排者从不被 run 阻塞：它把 run
   作为独立进程派发出去，立即拿回控制权，继续盯其他 worker、读信号、
   响应 gate。同步 run 会把编排者在 run 的整个生命周期内降级成「又一个
   worker」。

2. **账本是权威状态。** worker 每次 writeback 落在事件账本里——可重放、
   可审计、崩溃不丢。即使其他所有通道都失败，账本也永远不会丢「发生过
   什么」。

3. **编排者感知 = push 触发 + 账本拉取。** 因为编排者是 LLM 会话（只有
   轮询、没有中断），状态*转换点*——完成、失败、gate 打开、worker 死亡——
   以及信号在连续 N 个回合未被响应后的升级——
   以消息形式 push 给 supervisor 会话。push 是**易失的触发器，不是记录**：
   幂等（按转换去重，重发即 no-op）、可丢弃（未注册 supervisor 或 agent
   不可达 → 丢弃，账本仍是权威）。编排者的账本读取（`status`、
   `worker tail`、下一个 turn envelope）总会收敛到真相，所以丢消息只损失
   延迟，永不损失正确性。push 有两条路径：worker 自己的转换报告，以及
   scheduler 对死得来不及报告的 worker 的 dead-holder 清扫。

4. **detached run 由 lease + pid 活性监督，而不是父进程。** 同步 run 免费
   获得崩溃监督（被阻塞的调用方会在 run 死掉时立刻感知）。detach 拿掉了
   这层隐式监督，所以要正式接管：scheduler 的 dead-holder 检查
   （`notify_dead_holders`）发现持有进程 pid 已消失的 lease，向 supervisor
   push 重启提示。因此 detach 成为默认的前提是这条活性路径可靠——它是
   主监督，不是兜底。

### 监督的层级：人来监督编排者

监督是一个栈，顶端是人：

- **编排者（supervisor）监督 worker** —— 通过上面的 lease + pid 活性，
  以及 `worker tail` 做实时查看；
- **人监督编排者。** 编排者是自动化监督链的顶端；loop 里没有任何东西
  监督它。当它停滞、走错、或判断某事需要人时，人是上浮的终点。worker
  只能*经由*编排者触达人（它上浮，从不直接找人）；编排者之后如何让人
  参与——以及那要不要冻结任何工作——是编排层行为，内核不约束。人也可以
  随时直接介入（`todo update`、`worker stop`、手动 `todo complete`）：
  下层自动化，顶端是一个人。

在一个多 agent goal 内，**peer worker 互备**：契约的 `backup_for` 边为
每个 worker 声明一个替补，当 primary 的 lease 过期或心跳静默时替补自动
晋升（succession）。那是 *worker* 层的冗余——peer 之间的横向互备；它不
监督或替换编排者。

## 看板的结构：todo、依赖、worker

三种关系定义了多 worker 工作如何在看板上铺开——关键在于，信息如何在不
存在 worker 间直接消息的情况下流动。

**todo↔todo：依赖 DAG 是看板的骨架。** `--blocks` 边是*唯一*的排序机制——
没有全局优先级队列，也没有 worker 层面的先后，只有 todo 之间的边。扇出
是一个 todo 阻塞多个下游 todo；汇总（综合）是一个 todo 被多个上游 todo
阻塞。图只表达*什么必须先于什么*，别无其它。

**todo↔worker：弱绑定、运行时撮合。** todo 不专属于任何 worker——它摆在
看板上谁都能认领（lease）。实际「谁干哪个」在运行时两步撮合：编排者
spawn worker 时的意图（它知道该让哪个模型探哪个方向），以及 worker 认领
时的匹配（专精、lease 空闲）。关系是多对多、动态解析的；**lease** 是它的
「当前占用」快照，提供互斥与活性，而不是指派。内核不把 todo 指派给
worker——编排者塑形看板，worker 从看板认领。

**worker↔worker：没有直接消息——看板即共享状态。** worker 之间从不互相
说话。信息只经由三个载体流动，全部由账本中转：**evidence**（落地什么的
持久声明）、它指向的**产物文件**（报告、数据）、以及 **turn envelope 的
上下文层**（下一回合从账本重算注入）。一个「查看上游结果并总结」的
worker 并不是在收消息——那*就是*它的 todo：它被 `--blocks` 排在上游 todo
之后，它的信封注入上游的 evidence 和产物路径，它去读那些产物。

**扇出 → 汇总 → 扇出，用这些概念说。** 用不同模型沿不同方向 spawn 若干
worker（并行 todo，互无边）；一个汇总 todo `--blocks` 它们全部，于是下游
worker 读它们的产物做综合；第二轮 todo `--blocks` 这个汇总。编组、选模型、
分方向、定轮次，全是**编排层**的决策（编排者塑形 contract 拓扑、todo
文本、spawn 配置）；看板只保证顺序（边）与互斥（lease），并不建模*哪个
产物流向哪个 todo*——这部分接线由编排者写进 todo 文本和 acceptance 契约。

## steer 与重配一个运行中的 worker

编排者可以在 goal 中途改变 worker 的两类东西，它们走不同机制，因为区别
在于会话是否存活：

- **改「做什么」（指令/目标）→ steer。** `supervisor steer` 记录一个
  `WorkerSteered` 事件（latest wins），worker 的 steer 监听中止当前回合，
  让下一回合把该指令排进信封。这是*打断*式——不是悄悄附加：进行中的推理
  被放弃，worker 在新指令下继续。会话（它积累的上下文）存活。

- **改「用什么跑」（模型/思考等级）→ 退役 + 重开。** 模型和思考等级是会话
  的属性，spawn 时固定，不能经 steer 热更新。改它们意味着 worker 的*配置*
  变了，而配置即身份：退役该会话、用新配置 spawn 一个全新会话，上下文从
  账本冷启动（这正是「上下文超限/方向调整」退役本就在做的事）。所以重配
  不是第三条通道——它就是普通的退役-重开转移，应用到配置变更上。

经验法则：**steer 改任务，respawn 改 worker。** 两者都是编排者的决策，
内核只记录事件。

## worker 会话生命周期

worker 会话是一个**一等生命周期对象**，不是 run 的附属物：编排者创建它、
往它身上挂工作、泊车它、恢复它、最终退役它。resume-vs-fresh 只是这个
生命周期里的一个转移，不是全部。

### 状态与转移

```
   spawn ──► ACTIVE（在岗，持 lease 执行 turn）
                │   ▲
        中断    │   │ resume（InfraRecoverable → 回到中断前状态）
                ▼   │
           INTERRUPTED（FailureKind 已记录）
                │
                ├── InfraRecoverable → resume
                ├── ContextCorrupted → RETIRE + spawn fresh
                └── HardError        → RETIRE + spawn fresh

   ACTIVE ──泊车──► PARKED（无匹配工作/成本/配额；上下文封存）
   ACTIVE ◄─resume + 增量── PARKED

   任意状态 ──► RETIRE（退役）：goal 完成 / 方向调整（大面积 supersede）/
              上下文超限 / 显式 fresh。退役 ≠ 删除——账本永存。
```

ACTIVE 或 PARKED 中的会话都可能被中断（parked 会话不会撞 429，但它的
宿主会死）——INTERRUPTED 记录中断，`InfraRecoverable` 的 resume 回到
会话中断前所处的状态。

**何时泊车（PARKED）**：

- 没有 runnable todo 匹配这个 worker 的专精（model、thinking level、已
  积累的 todo 上下文）——不让它空转轮询；
- 成本控制：等 monitor/gate 期间保活一个会话不值得；
- 配额压力：把会话资源让给更高优先级的 goal。

**恢复的会话需要什么** —— 泊车会话的*推理链*（为什么选这条路、试过什么
失败了）是它真正的价值，原样保留；但封存期间*世界*变了，带着陈旧的世界
模型继续干活是 resume 最大的坑。刷新就是 resume 那个回合从账本重算的
普通 turn envelope（见下文「turn envelope」）：因为信封的上下文层永远从
账本实时推导，恢复的会话自动看到此刻的世界——新 todo、新 evidence、当前
仪表、gate 裁决。

**何时必须 fresh（RETIRE + spawn，绝不 resume）**：

- `ContextCorrupted`：verify 门拒绝了输出——推理链已被污染，续它会带着
  错误前提继续；
- 方向调整：大面积 supersede / replan 后，旧上下文全是作废路线的残留；
- 上下文超限：在撞到 token 上限*之前*退役，安排一次交接——旧 worker 把
  「学到什么、坑在哪」写进账本（这正是 evidence 强制非空的价值），fresh
  会话从账本冷启动。

### FailureKind：中断的分类

`FailureKind` 对中断分类，决定 INTERRUPTED →（resume | RETIRE）的分支：

- `InfraRecoverable` — 事故在*外面*（429 / 限流 / 连接重置 / agent 崩溃 /
  流间隙），推理状态完好：**可 resume**。
- `ContextCorrupted` — 事故在*推理里*：verify 门拒绝了输出，推理状态已
  污染：**应 fresh**。
- `HardError` — 回合出错且无可恢复的基础设施原因：**应 fresh**。

内核只提供这个分类（观察数据）；resume-vs-fresh 由调用方经
`--session-policy` / `--resume-session` 显式决策，默认 `auto` 只 resume
内核判定「可恢复」（`InfraRecoverable`）的会话。内核依然是纯工具——
提供状态和信号，但**从不替 agent 做决策**。

两条界定让生命周期保持简单：泊车发生在 **turn 边界**（不做 turn 中途的
抢占式挂起或检查点），会话绑定**一个 goal**（不跨 goal 复用——上下文
污染风险大于收益）。

## turn envelope：编排者给 worker 注入什么

turn envelope 是编排者/内核与 worker 之间唯一的信息接口——worker 执行的
每回合 prompt。它携带**两层**，并有意不含第三层：

- **指令层（每回合）** —— TODO 文本和完成契约（「报告你做了什么、观察
  到什么；声明 successor 或 `--no-follow-up`」）。没有这两个，worker 既
  不知道干什么、也不知道什么叫完成。
- **上下文层（每回合从账本重算）** —— goal 与 objective、上一回合的
  evidence、本 todo 的失败史（已分类）、近期语义历史、已裁决的 gate。这
  让 worker 不重复劳动、不再踩已知的坑——「持久产物而非 session 记忆」
  正落在这里。

**不在信封里：内核的调度内部状态。** should-run 判定、mode、arbitration
处置是内核*自己的*决策状态，是给编排者和 operator 看的——不是给 worker
的。把它们放进 worker 的 prompt，等于把调度器的犹豫泄漏进执行者（worker
该关心的是怎么干活，不是内核觉得该不该跑），也模糊了观察/决策的分离。信封
告诉 worker *做什么、以及做好它所需的上下文*；不告诉 worker 内核在想什么。

**在信封里：可观察信号。** 信号（outcome floor、oscillation、失败计数、
无进展）是另一类量：它们是对*工作本身*的观察、从账本重算——不是调度器的
犹豫。原则 1 承诺信号以 advisory 形式进 turn envelope，它们确实在那里：
信封的上下文层携带一个 `signals` 块，由与 delivery reason 的 advisory
相同的内核检测器重算（一套检测器、两个消费者——编排者从 packet reason 读，
worker 从信封读）。对信号如何处置，一如全程，是决策而非内核指令。

**一个信封，没有特例。** 第一回合、resume 的回合、普通回合用同一个信封，
差异完全由账本此刻装着什么自然产生。第一回合的信封自然短（没有失败史、
没有上一回合 evidence）；resume 回合的信封自然读起来像「你泊车以来的
世界」——因为上下文层永远从账本实时算。

## 信任与授权边界

worker 以**用户的完整信任域**运行：它执行任意 shell（一个 `--verify` 门就是
一条命令）、写它的 workspace，steer 消息能向它注入指令。loop 本身没有沙箱层；
隔离靠 **workspace 边界**（worker 留在自己的 workspace，除非 `--force-workspace`
另有指定）。当 worker 碰到处于或超出这条边界的事——不可逆、昂贵、需凭据——
它不判断「这要找人」，而是上浮给编排者，由编排者决定是继续、换路、还是让人
参与。信任域内自主，编排者就是边界处的那道门。
