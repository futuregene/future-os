# Loop 架构：持久看板、可靠控制、基于证据的 Agent

操作指南：[Loop 控制面](../../docs/loop-control-plane.zh-CN.md)。编排驾驶手册：
`skills/builtin/future-loop/SKILL.md`；研究方法：`skills/builtin/future-explore/SKILL.md`。
两份 skill 都由 skills 子模块分发。英文详细契约见 [ARCHITECTURE.md](ARCHITECTURE.md)。

## 边界

模型选择策略、解释科学证据、向人请求判断。内核提供确定性的状态、依赖、租约、证据、
验证器和提示信号，不因为启发式认为“卡住了”就强制重规划。智能的主要增益来自正确投喂
证据，而不是继续增加规则。

账本是权威，session 是缓存。能力统一经 CLI 暴露，仪表盘严格只读。人监督编排者的判断；
无需 LLM 的 watchdog 监督进程活性和通知传输，不监督科学推理、更不自动选模型或重规划。

## 底线、信号、预算

- **底线**：合法状态转移、完成意图、验收缺口、依赖约束、租约、验证式终局。
- **信号**：失败次数、成果连续段、振荡、缺少产物活动、monitor 状态、worker 里程碑。
  信号描述事实，不替 Agent 做策略判断。
- **预算**：外层回合数、验证尝试次数、验证命令独立墙钟上限。`--max-turns` 不是
  token/金额上限，也不是 Agent 内层推理和工具调用的总超时。

自动与手动完成共用非空证据和 acceptance token 检查。模型正常结束回复本身不足以关单：
交接契约未满足时，任务保持未完成并记录明确错误。自动完成还会检查已配置的验证器，并在
写回前复核当前契约。其他可执行任务不会被自动编成后继关系；一个任务切片结束不等于
整个 goal 结束。

手动完成有意不重跑机器验证，但记录其依据为人工审阅或显式覆盖，不伪造机器通过。
`delivered` 不等于 verified；`delivery record` 记录审阅者的判断。token 出现、文件存在、
脚本 exit 0 都不能单独证明探索性结论正确。修改验收标准必须有明确理由。

## 依赖与归属

推进任务的 `--blocks` 指向前置；gate/blocker 的 `--blocks` 指向下游。调度与手动完成
采用相同依赖语义。普通 gate 只阻塞相关任务，`--global-gate` 才全局冻结；无关工作可以
继续执行和完成。决策用 `gate resolve`，不能用 `todo complete` 代替。

`--owner` 是持久指派，租约到期不会取消归属。无 owner 才进入共享池。并行 run 必须使用
不同 agent ID；coordination 任务属于编排者，不进入 worker 前沿。工作区守卫避免写冲突。
依赖决定可执行性，priority 只排序可执行候选。

worker 通过账本 evidence 和产物文件交接，不引入额外协商协议。扇出、汇总、下一轮扇出
由普通 todo 和依赖边表达。

## 可靠 steer

新指令用 `ControlIssued`：UUID、目标 worker/广播、文本、interrupt 标志。
普通指导在下一回合边界注入，`--interrupt` 才额外中断当前会话。latest-wins 仅限同一
目标范围；A 的新指令不覆盖 B 的，广播与定向指导可同时按账本顺序注入。

`ControlAcknowledged` 按接收者落账，且必须晚于完成回合的持久写回。一个 worker 不会替
其他 worker 消费广播。传输失败、中断不会提前消费；崩溃可能导致重投，因此这是
**至少一次指导**，不是外部副作用“恰好一次”。指导必须幂等，不可逆操作另行审批。

中断 watcher 不越过不完整 JSONL 行，也不把失败的中断当送达。旧 `WorkerSteered` /
`SteerConsumed` 账本仍兼容，但新 CLI 不再写单槽指令。模型/思考级改变需要新配置的
会话，不能靠 steer 热切换。

## 异步执行与独立活性监督

生产 `run` 默认 re-exec detached child，检测立即退出的启动失败。`--detach` 是内部
前台子进程标记，`FUTURE_LOOP_NO_DETACH=1` 用于前台嵌入/测试。统一 `future` 二进制
重启自身时保留 `loop` 分组前缀。

detached 派发或 supervisor 注册会确保独立 `supervisor watch --goal G` 进程存在。
每个 goal 的 OS 文件锁保证单实例。它每两秒检查死租约持有者、超过五分钟仍未验证的交付、
待送通知；即使最后一个 worker 死亡，也不依赖下一次付费 run 或 LLM 轮询来发现。
`scheduler tick` 和回合结束检查保留为补充。

goal 删除/取消，或终局且无待送通知时 watcher 退出。它不是开机服务；宿主重启后需重新
运行该 CLI、重新注册 supervisor，或由操作系统服务管理器托管。没有宿主重启机制就不能
承诺跨断电自动恢复。

生命周期命令先停止失效 worker，再删状态；晚到完成不能复活 superseded 任务。
这与指导性中断不同，后者保留任务以继续工作。

## 持久、合批的通知队列

1. **先记录** `SupervisorNote`，不依赖 Agent 可达或 supervisor 已注册；同 episode 去重。
2. **准备不可变批次** `SupervisorBatchPrepared`：会话、note keys、消息、UUID。
   每批最多 32 条，每条有界并指向完整账本；watchdog 自然合并两个 tick 之间的消息。
3. **推送**使用 `enqueue_if_busy`，不打断编排者。失败重试使用相同请求 key 和相同正文。
4. **送达回执** `SupervisorBatchDelivered` 只在远端接收后写入。接收不代表编排者已执行。

OS 锁串行化并发 flusher。掉线保留批次，恢复补送；换 supervisor 可从账本恢复通知。
旧通知只触发当前状态核对，不能直接变成“重启这个 worker”的命令。无 watcher 的前台
嵌入者可立即 flush，但同样遵守持久化和重放语义。

## 有界验证器

验证器异步执行，独立默认 120 秒超时，可用正数 `FUTURE_LOOP_VALIDATOR_TIMEOUT_SECS`
配置。stdout/stderr 持续排空，仅保留有界诊断尾部；命令或继承管道挂住都会超时。
取消/超时清理子进程树（Unix process group；Windows process-tree termination）。

Unix 用 `sh -c`，Windows 用 `cmd.exe /D /S /C`，不是跨平台通用 shell 语言；可移植
任务应调用可移植校验程序。失败附带诊断，启动失败/超时记 inconclusive，不伪装通过。

## 投喂与真实进展

每轮信封包含目标、todo、验收/验证器契约、已决 gate、上游 evidence、上一轮 evidence、
相关失败、少量近期历史、提示信号。近期里程碑报告也会注入，但明确标为待核实声明。

fan-in 为每个已结束前置保留索引，摘要预算公平分配，不让第一个长报告吞掉后面的来源。
索引随前置数量增长，摘要总量仍受限；完整 evidence 用 `status --format json` 读取。
superseded 来源有明确标记。编排者仍须在下游 todo 写出产物路径，不把摘要当完整知识交接。

worker 回复全文以 `text_chunk.text` 写入各 run 的 live 日志；有界摘要保留最近文字，
不再只留开场计划。完成通知使用 JSON `task_delivery` 回执，含真实 worker/session/run ID、
验证结果、证据尾部和全文日志路径。`pending_other_todos` 统计所有 owner/class 的未完成项，
不是共享任务池大小；`awaiting_review` 不代表科学结论通过或整个 goal 已终局。
外部评分、算力就绪等状态仍需应用适配器监控，watchdog 不会自行查询任意外部服务。

区分 **存活、活动、进展**。`write/edit` 执行开始只是产物活动代理；shell 不自动算写入，
provider input/execution 阶段不重复计数。真实进展来自新产物、验证结果、指标改善或假设
被排除。读论文可能有进展，反复写进度文件也可能没有。

## 适度编排与诚实停止

根据不确定性和风险选择轻量、标准、重型流程。一次代码复盘不强制固定引用数量或多 worker。
仅在预期收益超过通信成本时并行不同方法族。确认配置、预算，不擅自改变用户约束。

检查点比较新增证据、剩余差距与下一步成本。允许达标、限定不可行、预算耗尽、低收益停止、
受阻等结果；只有达标才能称成功闭环。保留阶段成果和验收缺口，扩预算先问人。开放研究
不需要“证明所有可能方法都失败”才允许诚实停止。
