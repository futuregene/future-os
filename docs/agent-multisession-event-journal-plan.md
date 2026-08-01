# Agent 多会话、Queued Run 与事件真源实施方案

状态：方案草案（已按当前 `main` 实现和新架构逐项复核）

日期：2026-08-01

范围：Future Agent、Desktop GUI/Tauri、Remote/NATS、TUI、CLI、Channel 的运行模型、事件协议、持久化和删除回收

## 1. 最终结论

采用以下核心模型：

```text
Session：一段完整对话
  ├── Run A：一次用户提交及其完整 Agent 执行（active）
  ├── Run B：生成期间提交的 Follow-up（queued）
  └── Run C：另一个 Follow-up（queued）

Run
  ├── 一个 user message
  ├── 0..N thinking/text/tool/approval event
  └── 一个 terminal 结果
```

锁定决策：

1. **一个用户提交创建一个 run。** 一个 run 不再包含多个用户 turn。
2. **Follow-up 是 queued run。** 它不是当前 run 内的消息，也不修改当前 run；当前 run terminal 后由 session scheduler 串行启动下一个 queued run。
3. **删除 Steer。** 保留 Abort。仓库内没有 GUI/TUI/CLI/Remote/Channel 对 Steer 的真实调用，Steer 却引入中断、重排和 transcript 重写语义。
4. **Turn 不再是运行协议的独立身份。** 新事件路由、排序、审批和恢复只使用 `session_id + run_id + epoch + idx`。历史 transcript 的 `turn_id` 兼容读取，不能再作为调度依据。
5. **Agent 是 transcript 和 run event 的共同真源。** 最终对话写入 session transcript JSONL；高频事件写入 Agent 管理的 per-run event JSONL，不把 token delta 混入 transcript。
6. **GUI SQLite 只保存派生数据。** GUI observer、React 当前页面和 NATS 都消费 Agent canonical event，不拥有主流程。
7. **Session 内一次只运行一个 active run，不同 Session 可并行。** Observer 默认上限保持 128；queued run 使用独立的可配置上限，不能与 observer 配额混为一谈。
8. **删除 Session 必须回收全部 queued/active runtime、transcript、event journal、writer、lock/temp 文件。** GUI 离线删除使用 tombstone/outbox。
9. **不改变默认 sidecar 策略。** 外部管理 Agent 的部署形式继续提供 GUI 退出后 Agent 自主运行能力；本方案不要求 bundled sidecar 跨 GUI 生命周期常驻。

## 2. 术语和身份模型

### 2.1 Session

`session_id` 表示完整对话，负责：

- transcript 历史；
- 单 active run 调度；
- queued run FIFO；
- session 设置和 session-scoped event；
- observer attach、删除和恢复边界。

不同 session 的 executor、队列、writer、broadcaster 和 observer 相互隔离。

### 2.2 Run

`run_id` 在 Agent 接受一次用户提交时生成，代表：

- 一个用户输入；
- 该输入触发的完整 LLM/tool/approval 循环；
- 独立的 queued/running/terminal 状态；
- 独立的事件序列、模型设置快照、用量、错误和重试信息。

一个 run 可以产生多个 UI 展示块，但只有一个 user message。文本、思考、工具和审批卡片都是这个 run 的组成部分，不各自成为 run。

### 2.3 Turn

新架构中，产品语义上的“一轮对话”与 run 是 1:1：

```text
一次用户提交 + 对应 Agent 执行 = 一个 run = 一个产品 turn
```

因此：

- runtime、RPC 和 event envelope 不新增 `turn_index`，也不依赖 `turn_id`；
- 新 transcript entry 以 `run_id` 作为用户、assistant、tool 的关联键；
- 当前已有 `meta.turn_id` 暂时兼容读写，建议新数据令其成为 `run_id` 的确定性别名，或在完成 consumer 迁移后停止写入；
- GUI 对旧 transcript 仍可用 `turn_id` 展示历史上“一个 run 多 turn”的数据，但新数据按 `run_id` 分组；
- 不允许使用“当前 turn”这类可变隐式状态给实时事件归属。

这不是删除 UI 上的“对话轮次”概念，而是删除与 run 重复的第二套运行身份。

### 2.4 Epoch、Idx 和 Event ID

- `epoch`：同一 session runtime 的执行 generation，用于 fencing stale task。它不是业务轮次，也不代替 `run_id`。
- `idx`：单个 run 内所有 canonical event 从 0 开始单调递增，只能由 Agent 的单一 stamping point 分配。
- `session_idx`：model/name/cwd/config 等 session-scoped event 的单调序号，不借用已结束 run。
- `event_id`：跨 gRPC/NATS/SQLite 投影去重身份；run event 可由 `(session_id, run_id, epoch, idx)` 确定性生成。
- 跨 run 顺序使用 session scheduler 持久化的 `run_sequence`，不能比较两个 run 各自的 `idx`。

### 2.5 Canonical event envelope

Run-scoped event：

```json
{
  "event_id": "evt_session_run_138",
  "session_id": "session_abc",
  "run_id": "run_xyz",
  "run_sequence": 42,
  "epoch": 4,
  "idx": 138,
  "type": "text_chunk",
  "data": { "text": "..." },
  "created_at": "2026-08-01T12:00:00.000Z"
}
```

Session-scoped event：

```json
{
  "event_id": "evt_session_abc_27",
  "session_id": "session_abc",
  "session_idx": 27,
  "type": "model_changed",
  "data": { "model": "..." },
  "created_at": "2026-08-01T12:00:00.000Z"
}
```

约束：

1. 所有 run event 必须带 `session_id/run_id/run_sequence/epoch/idx`。
2. Session event 不伪造 `run_id`。
3. 每个 run 只有一个事件序列生成器。
4. Projection 只可合并同一 run 内语义允许合并的相邻 delta。
5. GUI、TUI、NATS 和 mobile 不得从当前页面、最近气泡或 active run 猜测 event 归属。

## 3. 已复核的当前实现与 Gap

| 项目 | 当前实现 | 新架构结论 |
| --- | --- | --- |
| Session 隔离 | 每个 session 有独立 runtime、队列、broadcaster 和 persistence | 保留 |
| Run 并发 | 每个 session 一次只有一个 active run，不同 session 可并行 | 保留，增加 queued run scheduler |
| Follow-up | TUI 生成中真实调用 `follow_up`，Agent 把字符串塞入当前 run 的 follow-up queue | 改为接受时创建新的 queued run |
| Steer | Agent/RPC/types/tests 已实现，仓库内一方客户端没有真实调用 | 删除 |
| Turn | Agent 为同一 run 内后续用户消息生成多个 `turn_id`；GUI 可按 turn 投影 | 新 run 不再多 turn；旧数据只兼容展示 |
| TUI 状态 | 一个 `activeRunId`；先画用户消息，follow-up 失败被吞掉 | 改为 run registry/queue；ACK 绑定 run；失败可见 |
| CLI | `future run` 为单次普通 prompt | 保持；如遇 busy 返回明确错误，除非显式选择 enqueue |
| GUI/Remote | streaming 时禁止二次提交，无 Follow-up 入口 | GUI 将来可开放 queued submit；Remote 策略可独立开放 |
| 飞书/钉钉 Channel | 新消息会 abort 当前 run、等待 idle 后再 prompt，并用 generation 丢弃旧 stream | 先保留 supersede 产品语义，但改为 Agent 原子操作和独立新 run |
| 请求幂等 | active + 最近 64 个 request ID 的进程内窗口，同 key 返回拒绝而非原 ACK | 改为 session-scoped durable request→RunAck 映射 |
| Run fencing | `run_id + epoch` 防旧 task finalize 新 run | 保留 |
| Run 内排序 | broadcaster 在内存锁内生成 `idx`，ring 约 2,000 条 | stamping 移到 durable Agent event writer |
| Event 真源 | Agent 只有内存 ring；GUI 写 per-run JSONL | 新 run 的 per-run JSONL 移到 Agent；旧 GUI 文件只读兼容并按 retention 回收 |
| NATS event | 已发布形状为 `{type,data,runId,idx}`，没有 event schema version | 用 versioned additive adapter，不能直接替换 payload |
| 历史 GUI event | 正常气泡来自 transcript；import/fork 已能从 transcript 合成基本 tool event | 默认只读兼容/淘汰，不建设全量 import RPC |
| Transcript | session JSONL 已有 ordered writer 和 terminal barrier | 保留；新 run 只追加一组 user/assistant/tool entry |
| 删除 | Agent 主要删除 session JSONL/lock；GUI 对 Agent best-effort | 增加 deleting fence、完整目录回收和 GUI outbox |
| Observer | Tauri/Rust 每 session observer；React 只渲染 | 保留，修正 LRU touch 和 active lease race |
| Sidecar | 默认 GUI 生命周期会影响 bundled Agent | 不改默认部署；能力由外部 Agent 形态满足 |

特别确认：此前“上层都未使用 Follow-up”的判断错误。TUI 当前确实在生成期间调用 `follow_up`；迁移必须同时修改 TUI，不能只删 Agent RPC。

## 4. Session 级 Queued Run

### 4.1 接受模型

统一普通提交和 Follow-up 的 Agent 接口：

```text
enqueue_prompt(
  session_id,
  message,
  attachments,
  client_request_id,
  busy_policy,
  after_run_id?
) -> RunAck
```

建议 ACK：

```json
{
  "session_id": "session_abc",
  "run_id": "run_new",
  "run_sequence": 43,
  "state": "queued",
  "queue_position": 1,
  "client_request_id": "client_req_123"
}
```

`queue_position` 只是 ACK 时刻的提示值，前序 run 取消后允许变化，不能作为排序或状态依据。TUI/GUI 的权威状态来自后续 session state/queue lifecycle event；首版可以只显示“已排队”而不显示精确数字。

接受过程必须在同一个 session scheduler 临界区内完成：

1. 校验 session 不是 deleting/shutting down。
2. 以 `(session_id, client_request_id)` 做幂等检查；相同 request digest 的重试返回同一个 `run_id`，相同 key 但 payload 不同则返回 `duplicate_request_conflict`。该映射与 RunRequest 一起 durable，Agent 重启后仍然有效，并至少保留到 session 删除；压缩 journal 时必须把映射带入 checkpoint/manifest。
3. 生成 `run_id + run_sequence`。
4. 快照本次 run 的 model、thinking、approval、cwd 和相关执行设置；快照必须带 `settings_schema_version`。
5. 把完整 `RunRequest`、`run_accepted` 和 queued/running 状态持久化到 queue/control journal。
6. 空闲则占用 active lease 并启动；忙碌则放入 durable FIFO。只有 run 真正开始时，才按对话顺序把 user entry 追加到 transcript。
7. 持久化成功后才返回 ACK。

`after_run_id` 只作为调用方看到的前驱校验和诊断信息，不让新 run 归属于旧 run。旧 run 已结束但 session 未删除时，提交仍可成为下一个 run；是否接受由 busy policy 明确决定，不能静默丢弃。

Queued run 的输入不能在接受时直接插入 transcript。否则 A 尚在生成时接受 B 会形成 `user A → user B → assistant A` 的错误顺序，并重新引入 terminal rewrite。Canonical 顺序应为：

```text
queue journal: accept B
transcript: user A → assistant A → user B → assistant B
```

### 4.2 Run 状态机

```text
Accepted
  ├── Queued
  │     ├── Starting → Running ↔ WaitingApproval → Finalizing → Terminal
  │     └── Cancelled
  └── Starting → Running ↔ WaitingApproval → Finalizing → Terminal
```

Terminal 状态至少包括：

- `completed`
- `failed`
- `cancelled`
- `interrupted`（Agent 进程退出等不可续跑情况）

`persistence_degraded` 不是 terminal：磁盘尚未承诺时不存在可靠终态。它是 run/session health 上的非终态阻塞状态；writer 恢复并提交 buffered outcome 后，run 才能进入上述真实 terminal。若 Agent 在 degraded 期间崩溃，恢复写能力后按已有 journal 证据收敛为 completed/failed 或 interrupted，不能凭内存中曾经“执行结束”推断 completed。

状态约束：

1. 每个 accepted run 必须最终 terminal，不能留下幽灵 queued run。
2. 一个 session 最多一个 active lease；queued run 严格按 `run_sequence` FIFO。
3. 当前 run terminal 持久化完成后，scheduler 才能启动下一个 run。
4. queued run 只能产生 accepted/queued/cancelled 等 lifecycle event，不接收 text/tool/approval execution event。
5. Approval 只暂停当前 active run，不阻塞其他 session；本 session 后续 run 保持 queued。
6. 设置在接受 run 时快照；接受后的模型/思考/审批设置变化只影响再下一次提交，不回写已 queued run。

### 4.3 Queue policy 与上限

首版只保留易解释的策略：

- `reject_if_busy`：CLI、automation 等需要严格同步语义的调用方可使用。
- `enqueue_if_busy`：TUI 和未来 GUI Follow-up 使用。
- `supersede_session`：Channel 保持“最新消息优先”时使用；在一个 durable scheduler transaction 中取消 active 和全部既有 queued run，再接受新 run 作为唯一 successor。

`supersede_session` 是普通 abort 不级联规则的显式例外，不是插队操作。被替代的 queued run 都必须得到 `cancelled(reason=superseded)` terminal；新 run 使用更大的 `run_sequence`，因此仍保持 session 顺序单调。该事务必须先 durable 记录新 RunRequest、被取消 run 列表和 supersede intent，再发 active interrupt；任一步无法持久化时整个操作返回失败，不能只取消一半。

删除原有 `steering_mode`、`follow_up_mode`、`all/one-at-a-time` 和 `streaming_behavior`。它们属于旧的 run 内控制模型。

queued run 上限必须独立可配置；首版可采用每 session 128 作为建议值，但它与 observer 的默认 128 没有语义关联。达到上限返回结构化 `queue_full`，不覆盖、不合并、不静默丢弃。全局再设置独立的 queued run 内存/磁盘配额，防止单客户端耗尽资源。

### 4.4 Abort 和取消

- `abort(run_id)` 只取消指定的 active run；不能因为 active run 被取消而默认删除后续 queued run。
- `cancel_queued_run(run_id)` 取消一条尚未启动的 queued run。
- `abort_session(include_queued=true)` 才取消 active 和全部 queued run。
- 客户端用 queued `run_id` 调 `abort` 时返回结构化 `run_not_active`，并在错误详情中给出可用的 `cancel_queued_run` 动作；不能猜测后自动改命令。
- GUI/TUI 的停止按钮默认只停止当前 active run，并明确显示队列是否仍会继续。
- Session 删除固定使用 abort active + cancel all queued + persistence barrier。

### 4.5 Persistence health 与调度熔断

`persistence_degraded` 不应设计成当前 run 的终态，而是 session 级健康状态；如果底层目录/磁盘故障具有全局性，EventJournal manager 同时提升为全局 unhealthy。

```text
Healthy
  → PersistenceDegraded（暂停 dequeue，不启动新 run）
  → Probing/Recovering
  → Healthy（恢复 FIFO）或 RequiresOperatorAction
```

规则：

1. 当前 active run 发生不可满足的 append/fsync 错误时，不得宣称 clean completion；保留待提交 outcome，并进入非终态 degraded 状态。
2. Scheduler 在释放 active lease/dequeue 前先检查 persistence health；不健康时 queued run 保持 queued，不能逐个 dequeue 后失败。
3. unhealthy 期间默认拒绝新的提交并返回 `persistence_unavailable`；如果能够可靠写入独立的健康 WAL，才可选择继续接受但保持 queued。
4. 恢复必须先完成 writer probe、journal 尾部校验和 manifest 修复，再释放调度熔断。
5. GUI/TUI 显示“存储故障，队列已暂停”，不能把它表现成普通 idle。
6. 删除命令仍可执行，但必须走可验证的 close/fence 路径，不能因 unhealthy 永久无法回收。

执行侧采用**有界 backpressure + fail-closed interrupt**，不允许磁盘故障期间继续无限执行并把事件堆在内存：

- EventJournal channel 有固定容量；producer 等待 writer 的时间有上限。append/fsync 失败或 backpressure 超时后，立即设置 run persistence interrupt，停止发起新的 LLM continuation 和新工具调用。
- 已在执行的外部工具按其既有取消能力请求停止；无法撤销的副作用必须被记录为“outcome 未确认”，恢复后优先 reconciliation，不能为了补日志自动重跑工具。
- 内存只保留一个有大小上限的 pending commit batch 和 terminal outcome；超过上限直接保持 degraded/需人工处理，禁止继续增长。
- `abort(active_run)` 属于安全控制，即使 degraded 也要触发内存 interrupt；其 durable terminal 等 writer 恢复后提交。
- `cancel_queued_run` 必须 durable 才能生效。degraded 期间无法写 hard barrier 时返回 `persistence_unavailable`，不得只改内存队列；完整 session 删除是例外，因为它通过 deleting fence 后回收整个 canonical store，不要求为每条 queued run 补写取消事件。

### 4.6 Durable 附件

Queued RunRequest 不能依赖 GUI/TUI/Channel 的临时文件路径，也不应把无上限 base64 直接塞进 `queue.jsonl`。

- Agent 接受前校验单文件、单请求和 session 总配额。
- 附件先写入 Agent 管理的 content-addressed blob store，记录 `blob_id/hash/media_type/size`，fsync 成功后 RunRequest 只保存引用。
- Blob 引用采用两阶段提交：先创建并 fsync upload lease/pending marker，再写 blob；RunRequest hard commit 后标记 referenced 并清除 pending。Orphan GC 必须忽略有效 pending lease，并且不回收创建时间小于安全窗口（建议 10 分钟）的未引用 blob。
- ACK 返回前，RunRequest 和 blob 引用必须同时 durable；失败则整个提交不成立。
- 启动 run 时再次校验 hash。缺失或损坏时该 run 以结构化 `attachment_unavailable` 失败，不能把残缺 prompt 发送给模型。
- queued run 取消、session 删除和 retention 都更新引用并由 orphan GC 回收 blob。进程 crash 留下的过期 pending marker 只能在安全窗口后、并再次扫描全部 queue/checkpoint/transcript 引用后清理。
- 已存在的合法 Agent-owned base64 transcript 仍兼容读取，不要求回写迁移。

### 4.7 RPC 和结构化错误契约

Phase 0 先集中冻结以下 RPC；名称可在 proto 评审时调整，但语义不能散落在调用方：

| RPC | 变更 | 关键返回/约束 |
| --- | --- | --- |
| `enqueue_prompt` | 新增/替代 busy prompt | `RunAck`；原子 `busy_policy` 支持 `reject_if_busy/enqueue_if_busy/supersede_session` |
| `follow_up` | 临时兼容 | 转为新 queued run 并返回 `RunAck`，之后删除 |
| `steer` | 删除/弃用 | 兼容窗口内返回 `unsupported`，不再执行 |
| `abort` | 收紧 | 只接受 active `run_id` |
| `cancel_queued_run` | 新增 | 只取消 queued run，幂等返回终态 |
| `abort_session` | 新增/明确 | 可选择是否包含 queued run |
| `get_session_state` | 扩展 | active、queued 摘要、persistence health、cursor |
| `attach/get_events_since` | 扩展 | canonical cursor replay |
| `delete_session` | 扩展 | deleting fence、幂等和删除策略 |
| `prune_run_events` | 新增 | 只裁剪 terminal run 的详细 event |

统一错误码至少包括：`busy`、`queue_full`、`deleting`、`shutting_down`、`duplicate_request_conflict`、`run_not_active`、`run_not_queued`、`stale_run`、`stale_epoch`、`persistence_unavailable`、`attachment_unavailable`、`not_found`。错误码进入 proto/JSON 的稳定字段，不要求客户端解析英文错误字符串。

## 5. Turn 迁移方案

### 5.1 新数据

新 run 的 user、assistant、tool transcript entry 均带相同 `run_id`。运行协议不需要 `turn_id`：

```text
run_id = 唯一执行/轮次身份
entry_id = 单条 transcript entry 身份
idx = 单条实时 event 在 run 内的位置
```

为降低一次迁移风险，`meta.turn_id` 分两步退役：

1. 过渡期：新 entry 仍可写 `turn_id = run_id`，consumer 优先使用 `run_id`。
2. 全部 consumer 和历史测试迁移后：停止写新 `turn_id`，类型改为 legacy optional。

禁止再生成与 `run_id` 无关的当前 turn ID，也删除 `current_turn_id` 这类可变共享状态。

### 5.2 历史数据

旧 transcript 可能有一个 `run_id` 对应多个 `turn_id`，必须原样可读：

- 不重写旧 session JSONL；
- GUI legacy projection 检测到多 turn 时继续按 `turn_id` 分组；
- Runs inspector 将其标记为 `legacy_multi_turn`；
- replay/migration 不为缺失 turn 的旧 event 人工创造新业务身份；
- 新提交永远创建新 run，不延续旧 run 的最后一个 turn。

### 5.3 删除的实现复杂度

迁移后删除：

- `PendingMessageQueue<String>` 的 steering/follow-up 双队列；
- run loop 内 drain follow-up 并继续同一 LLM continuation 的分支；
- steering interrupt/reorder 分支；
- 因 in-run 用户消息重排而触发的 transcript terminal rewrite；
- `steer` RPC、`steerCmd`、`set_steering_mode` 和相关状态字段；
- `follow_up_mode`、`streaming_behavior` 及其死类型；
- realtime event 的 turn stamping 计划。

## 6. TUI 与 Channel 必须同步调整

### 6.1 当前问题

当前 TUI：

1. 在 `state.streaming` 时调用 `client.followUp(value)`，目标是 `activeRunId`。
2. RPC 前先插入用户气泡。
3. `followUp` 失败被吞掉，可能留下没有被 Agent 接受的幽灵气泡。
4. `agent_end` 会清空单一 `activeRunId`；在 queued run 模型下，A 结束与 B 启动的边界不能被解释为整个 session 空闲。

### 6.2 目标交互

TUI 所有用户提交都调用同一个 `enqueuePrompt`：

```text
空闲提交   → ACK(state=running, run_id=A)
生成中提交 → ACK(state=queued,  run_id=B)
再次提交   → ACK(state=queued,  run_id=C)
```

必须调整：

- 删除 `followUp()` 和 `steerCmd()` 客户端 API。
- `prompt()` 返回并保留完整 `RunAck`，不能只更新一个 `activeRunId`。
- TUI state 增加按 `run_id` 索引的 run registry 和 session queue order。
- 用户气泡可先以 `client_request_id + submitting` 乐观展示；ACK 后绑定 canonical `run_id`，失败则显示失败/可重试，不能吞错。
- queued user bubble 显示排队状态和位置；开始时转 running，terminal 后 settled。
- `streaming` 从“是否有一个 activeRunId”改为 session 是否有 active run；另有 `queuedCount`。
- 收到 A 的 `agent_end` 只终结 A；随后根据 B 的 `run_started` 更新 active，不制造整段对话已结束的闪烁。
- Abort 精确发送当前 active `run_id`；取消排队项使用独立命令。
- TUI 断线重连时从 Agent session state/journal 重建 active + queued runs，不能只清空本地 ID。

TUI 展示仍可以把一个 user bubble 和该 run 的 assistant/tool 输出视为一轮；不需要展示 `turn_id`。

### 6.3 同版本迁移

TUI 是当前 Follow-up 的真实调用方，必须与 Agent RPC 在同一发布中切换：

1. 先让 Agent `follow_up` 临时适配为 `enqueue_prompt(enqueue_if_busy)`，返回新 `run_id`。
2. 更新 TUI 使用统一提交 API 和 RunAck。
3. 删除仓库内最后一个 `follow_up` 调用后，再删除兼容 RPC。
4. 外部 gRPC 调用无法从仓库静态分析排除；如果协议有公开兼容承诺，保留一个明确标记 deprecated 的版本窗口，否则在协议版本升级说明中列为 breaking change。

### 6.4 飞书/钉钉 Channel

当前 Channel 并不是 follow-up：飞书收到同一 chat 的新消息时会在 per-chat lock 内先 `abort` 当前 run、等待 idle，再发普通 prompt；DingTalk 也采用 generation/supersede 语义。Phase 1 不能把它们当作“未受影响”的普通 prompt caller。

首版迁移以**保持现有外部行为**为原则：

1. Channel 新消息默认调用 `enqueue_prompt(busy_policy=supersede_session)`：取消当前 active 以及全部既有 queued run，再把新消息作为唯一 successor。
2. `supersede_session` 不是 Steer：旧 active run 保留 cancelled/partial 终态，旧 queued run 以 `superseded` 取消，新消息拥有新的 `run_id`，不重排旧 run transcript。
3. 该操作必须由 Agent scheduler 原子执行，不能由 Channel 自行组合 `abort → wait → enqueue`，也不能只依赖 Channel 进程内 mutex。混合客户端共享同一 session 时，TUI/GUI 已排队的 run 也会被明确取消；这是“最新消息覆盖整个 session”策略的可见代价，UI observer 必须收到对应 terminal。
4. 每条已启动的 Channel 消息对应一个独立流式卡片；queued 期间不创建 streaming CardKit/AI Card，避免空卡片长期占位。
5. 如果排队等待超过短阈值，Channel 可回复轻量“已接收/等待处理中”；精确位置仅作提示，真正开始时再创建或更新该 run 的卡片。
6. generation counter、card id、approval action 都绑定 `run_id`；旧 run 的 terminal 不能结束新 run 的卡片。
7. 图片/文件必须先进入 Agent durable blob store，Channel 下载临时路径不能成为 queued RunRequest 的长期引用。

后续若产品决定 IM 改为“不打断、按发送顺序回答”，只需把 Channel policy 切为 `enqueue_if_busy`；这是显式产品开关，不与本次 Steer 删除混淆。Channel bridge 必须在 proto 改动的同一版本更新，并在配置/UI 中解释 supersede 会取消其他客户端在同 session 的排队项。

## 7. Agent Event Journal

### 7.1 目录布局

```text
~/.future/agent/
  sessions/
    {session_id}.jsonl
    {session_id}.jsonl.lock
  run-events/
    {session_id}/
      session-events.jsonl
      queue.jsonl
      {run_id}.jsonl
      manifest.json
  blobs/
    {sha256}
```

- session transcript JSONL 保存最终可见对话和控制事实。
- `queue.jsonl` 保存 run accepted/queued/dequeued/cancelled 顺序，使 GUI/TUI 断开或 Agent 受控重启后仍能审计和恢复队列。
- `{run_id}.jsonl` 保存高频 canonical event。
- `manifest.json` 保存 schema、next sequence 和 high-water mark；损坏时以 journal 尾部修复。
- `blobs/{sha256}` 保存 queued/active run 的 Agent-owned 附件；引用关系由 queue/control journal 和 transcript 决定。
- 路径必须经过 safe-slug 校验，禁止使用客户端绝对路径。

`queue.jsonl` 采用 checkpoint + tail 压缩：默认在 session idle 且超过行数/字节阈值时触发；达到 hard size threshold 时由 scheduler actor 建立 mutation barrier 后触发，不能与 enqueue/cancel/dequeue 并发改状态。Checkpoint 至少包含 `schema_version`、`next_run_sequence`、active/queued RunRequest、设置快照和 blob 引用、`(client_request_id, request_digest) → RunAck` 幂等索引、persistence health 以及 journal high-water mark。写入 `{session}.queue.checkpoint.tmp`、fsync、原子 rename 后才裁剪已覆盖 tail；crash 时选择最后一个校验有效的 checkpoint 并重放其后的 journal。

Agent 进程 crash 后无法继续原 Rust future：active run 恢复为 `interrupted`。尚未开始的 queued run 是否自动继续必须是显式策略；本方案默认继续，因为其输入和设置快照已 durable，并在启动时先完成 interrupted active run 的终态恢复。

### 7.2 单一 writer/stamping point

每 session 一个 `EventJournal` actor：

1. 校验 session/run lease 和 epoch。
2. 分配 `idx` 或 `session_idx`。
3. 生成 `event_id` 和 timestamp。
4. append Agent JSONL。
5. 更新 replay ring/projection。
6. 广播 gRPC observer。

只有成功进入 writer 的 event 才能广播。写入分为 hard barrier 和 bounded group commit：

- `run_accepted`
- `run_started`
- `approval_request/decision`
- `run_terminal`
- queued run cancel

以上控制边界使用 hard barrier。`tool_end` 和高频 delta 允许按 10–50ms/字节阈值 group commit，但仍必须遵守 append-and-commit-before-broadcast：同一批事件 durability 确认后才能对外发送。这样避免大量短工具调用产生每秒多次独立 fsync，同时不让 observer 看到磁盘尚未承诺的事件。

这是有意识的吞吐/崩溃窗口取舍：工具副作用可能已发生，而对应 `tool_end` 最多仍有一个 group-commit 窗口尚未 durable。恢复时依靠 transcript/tool reconciliation 标记 outcome unknown，绝不能因缺少 `tool_end` 自动重跑可能有副作用的工具。

Terminal 顺序：

```text
flush run events
→ commit transcript assistant/tool + run_terminal
→ fsync critical boundary
→ broadcast terminal
→ release active lease
→ scheduler start next queued run
```

任一步失败不能对外宣称 clean completion。

### 7.3 Replay

Atomic attach 顺序：

1. 在 EventJournal 内确定 replay 上界并注册 live receiver。
2. cursor 命中热 ring 时读内存。
3. cursor 早于 ring 时读 Agent per-run JSONL。
4. journal 被 retention 裁剪或损坏时才返回 projection snapshot/gap。
5. replay 后无缝衔接 live event。

内存 ring 大小只影响性能，不影响正确性。

### 7.4 Retention

- session 存在时默认在已配置配额内保留 run event；达到配额只能按下面的完整 terminal run retention 规则裁剪，不能任意截断 active journal。
- session 删除时无条件删除其全部 event journal 和 queue journal。
- 清理 finished run 只删详细 event，不删 transcript；需 Agent prune RPC。
- 只能按完整 terminal run 裁剪，不能裁剪 active/queued run。
- Fork 默认复制 transcript，不复制历史 telemetry 或旧 queued 状态。

## 8. GUI Observer、SQLite 与 NATS

迁移后数据流：

```text
Agent canonical event
  ├── GUI observer 更新 SQLite 派生索引
  ├── Tauri 通知 React 渲染
  └── 按原 envelope 镜像到 NATS
```

### 8.1 GUI 数据职责

删除 GUI canonical event 所有权：

- `RUN_EVENT_BUFFER`
- `DISK_WRITER`
- `~/.future/app/run_events/*.jsonl` 的新写入；既有文件在兼容期仅只读
- collector/observer 的本地 event 双写分支

SQLite runs、approval、tool 和 notification 数据继续存在，但必须能从 Agent journal 重建。冲突以 Agent JSONL 为准；GUI 额外数据缺失时按 canonical event/transcript 降级展示。

### 8.2 Observer

- 每 session 独立 observer，不受 React 当前对话、模型、思考等级和审批设置切换影响。
- 切换对话只改变渲染订阅。
- 默认最多 128 个 live observer task；idle observer 可 LRU sleep，active observer 永不逐出。
- discovery 只更新 registry，不 touch 全部 observer。
- 发现 active run 后、attach 前即建立 active lease。
- queued-only session 不需要常驻 streaming observer，但 registry 必须保留 queued 状态并在 scheduler 启动时唤醒。

### 8.3 审批展示

- Approval event 必须带准确 `session_id/run_id/epoch/idx`。
- 当前对话在输入框上方展示审批卡。
- 非当前对话在左侧列表展示 pending 标记。
- 切换模型/思考/审批策略只影响之后接受的新 run，不改变正在等待审批或已 queued run 的设置快照。

### 8.4 NATS

- 当前已发布 payload 是 `{type, data, runId, idx}`，不能直接替换成字段命名和形状不同的 canonical envelope。Bridge 必须做显式 wire adapter。
- NATS event 增加 `schemaVersion`；v1 字段 `type/data/runId/idx` 保持原名和原语义，只做 additive 扩展，v2 consumer 才读取 `eventId/sessionId/runSequence/epoch` 等新字段。
- 发布端根据 handshake/capability 选择兼容 payload；在移动端最低兼容版本尚未满足前，不删除 v1 字段，也不把 `data` 从字符串静默改成对象。
- 去重键在 v2 使用 `eventId`，旧客户端继续使用 `(runId, idx)`。
- 单 publisher FIFO 保序；客户端按每 run cursor 检测 gap。
- queue overflow 或断线后从 Agent durable journal 补齐。
- ACK 中的 `run_id/run_sequence` 只能由 Agent 生成。
- 没有 mobile/NATS 时 Agent 和桌面仍自主完成。

发布顺序：先发布能同时理解 v1/v2 的 bridge 和 mobile reader，再开启 v2 additive 字段，最后经过明确兼容窗口才允许移除 v1。NATS schema 需要独立契约测试，不能只靠 gRPC proto 测试覆盖。

## 9. GUI 旧 Event 数据兼容与淘汰

完整导入旧 GUI 高频 event 不是基线要求。事实依据：历史气泡主要由 Agent transcript 的 user/assistant/tool entry 渲染；当前 import/fork 路径已经能从 transcript 合成基本 `tool_start/tool_end`，因此全量 importer 的收益主要限于 aborted partial text、精细 thinking/tool 时间线、usage 和 Runs inspector 诊断，而不是正常历史对话可读性。

基线采用低复杂度策略：

1. Agent 新版本只为新 run 写 canonical event journal。
2. GUI 对新 run 使用 Agent-first；旧 GUI event 文件进入 read-only legacy namespace，不再追加。
3. 历史正常完成 run 使用 transcript + SQLite run summary 渲染；需要工具列表时继续从 transcript 合成最小 projection。
4. 旧 aborted/failed run 若存在 legacy event 文件，可继续恢复 partial output 和详细 Runs inspector；文件不存在时明确降级为 transcript/terminal summary，不伪造高频事件。
5. Session 删除时仍立即回收对应 legacy 文件；其他 legacy 文件只在用户清理或明确 retention 到期后删除，不能用“一个发布周期”作为无条件静默删除依据。
6. 基线不新增 Agent `import_run_events` RPC，也不做 count/max idx/hash 全量搬迁。
7. 如果后续确认有合规、审计或长期 Runs inspector 需求，再单独设计离线 migrator；优先导入 compact projection/summary，而不是 token delta 原样搬运。

退出 legacy reader 的前提是产品明确接受旧 run 只有 transcript 级详情，或所有需保留数据已完成另行迁移；不能仅以新版本发布时长判断。

## 10. Session 删除与回收

### 10.1 Agent 删除状态机

```text
Active/Idle
  → Deleting（拒绝新提交和配置写入）
  → abort active + cancel all queued
  → 等待 matching task 退出
  → transcript/event/queue writer barrier + close
  → canonical 目录 rename-to-trash
  → 从 sessions map 移除
  → Deleted
  → 异步物理清理 trash
```

要求：

1. `delete_session` 带 `client_request_id`，重复调用幂等。
2. 支持 `reject_if_running` 和 `abort_and_delete`；GUI 确认删除使用后者。
3. 删除过程中不能 `try_write` 失败后静默继续。
4. 先关闭 handle，再 rename/delete，兼容 Windows。
5. 晚到 event 受 deleting fence 和 epoch 拦截，不能重建文件。
6. observer 收到 `session_deleted`/NotFound 后终止。
7. transcript、run-events、queue、manifest、lock、temp 都属于同一删除生命周期。

### 10.2 GUI tombstone/outbox

SQLite 增加 `agent_session_deletions`：

```text
session_id PRIMARY KEY
requested_at
mode
attempt_count
last_error
completed_at
```

GUI 删除 thread/workspace 时：

1. 与本地 rows 删除在同一 SQLite transaction 写 tombstone。
2. import/discovery 不得重新创建 tombstoned session。
3. worker 重连 Agent 后发送幂等 delete。
4. Agent 确认后标记 completed，短期审计后 GC。
5. 单删、批删、workspace 删除和 clear-all 共用同一服务。
6. 多 thread 引用同一 session 时只由最后拥有者触发 Agent 删除，除非用户明确强制删除。

Agent 启动时继续清理 `.trash-*`、orphan event 目录和过期 temp。GUI 在迁移期继续清理 legacy run event orphan。

## 11. GUI/TUI 重启和 Agent 故障恢复

GUI/TUI 重启：

1. 从 Agent 查询 session active run、queued runs 和 cursor。
2. active run attach durable replay 后继续显示。
3. queued run 恢复排队气泡和位置。
4. approval 从 Agent journal/get_state 重建。
5. 不先把 SQLite 非终态 run 写成 cancelled；Agent 不可达时显示 `reconciling`。

Agent 重启：

1. 原 active future 无法接续，matching run 记为 interrupted。
2. 完成 interrupted terminal commit。
3. 默认恢复 durable queued runs 并按 FIFO 启动；可通过安全模式暂停队列，但不能丢失。
4. GUI observer 按 journal cursor 收敛，不制造假 completed/cancelled。

GUI 崩溃不影响外部部署 Agent 的 executor、队列或审批状态。默认 sidecar 随 GUI 重启属于部署生命周期，不改变协议能力。

## 12. 分阶段实施

### Phase 0：不变量测试

- 固定 Session/Run/Event 状态机。
- 固定“一次提交一个 run”和 session 单 active lease。
- 测试并发提交、幂等 ACK、FIFO、abort/cancel/delete race。
- 测试 event envelope 和 `idx` 唯一递增。
- 对 scheduler 状态机使用 property-based test；输入随机 submit/abort/cancel/crash/recover 序列，持续验证单 active、无丢失、无重复 terminal。
- 预留 writer failpoint，在每个 append/flush/fsync/rename 边界可确定性 crash 或返回 I/O 错误，Phase 3 复用同一套 harness。

退出条件：重复 request、重复 run sequence、同 session 双 active、跨 run event 均会测试失败。

### Phase 1a：Queued Run 核心

- 增加 `RunRequest/RunAck`、durable queue 和 session scheduler。
- 冻结 RPC、busy policy 和结构化错误 schema。
- 普通 prompt 支持 `reject_if_busy/enqueue_if_busy`，实现 session-scoped durable idempotency。
- 临时把 `follow_up` 适配为创建新 queued run。
- 删除 run loop 内 steering/follow-up continuation 和 transcript rewrite。
- 删除 steering/follow-up mode 与 streaming behavior。

退出条件：正常存储条件下，每次 accepted text submit 都有独立 run、严格 FIFO、幂等 ACK 和唯一 terminal。生产客户端切换前继续受 feature flag/兼容 adapter 保护。

### Phase 1b：安全边界与 Supersede

- 实现 persistence health、bounded backpressure、执行 interrupt 和恢复熔断。
- 实现 durable blob、两阶段 upload lease、配额和 GC。
- 实现原子 `busy_policy=supersede_session`，取消 active + 全部 queued 后接受唯一 successor。
- Channel 改用原子 supersede，不再自行 `abort → poll idle → prompt`。
- 完成 queue checkpoint/compaction 和 crash failpoint 测试。

退出条件：磁盘故障不继续执行或排空队列；附件不被 GC 竞态删除；supersede 在任意 crash 点不出现半取消/双 active；之后才允许 Phase 2 客户端正式切换。

### Phase 2：TUI/Channel 同步迁移和 Turn 收敛

- TUI 改统一 `enqueuePrompt` 和完整 RunAck。
- 增加 run registry、queued UI、ACK/失败 reconciliation。
- 修复 agent_end 边界和 abort/cancel 目标。
- consumer 优先按 run 分组；保留 legacy multi-turn 投影。
- 仓库内无调用后删除 `follow_up/steer` RPC 和 dead types。
- 飞书/钉钉的 card、generation、approval action 全部绑定新 `run_id`，补充 supersede/queued 集成测试。

退出条件：TUI 连续排队 10 次不出现幽灵气泡、错 run、错误 idle 状态或丢消息。

### Phase 3：Wire envelope 与 Agent EventJournal

- 修改 proto、Agent event builder、TUI/CLI/GUI/Channel types。
- stamping 移到 durable writer。
- 实现 append-before-broadcast、disk replay、atomic attach、retention projection。
- 增加 I/O failure 和 truncated tail recovery。

退出条件：GUI/TUI kill 后可从任意 cursor 恢复；所有 event 身份和顺序正确。

### Phase 4：GUI Agent-first 与旧日志兼容

- GUI read path 切到 Agent。
- observer 只做投影、通知和 NATS mirror。
- 新 run 停止写 GUI event log，移除 GUI writer/buffer。
- 旧 event log 保留只读 fallback；历史正常 run 从 transcript 渲染，缺详细事件时明确降级。
- 实现按 session 删除和显式 retention 清理，不新增默认全量 import RPC。

退出条件：GUI event 目录无新写入，历史数据仍可读。

### Phase 5：删除回收闭环

- Agent deleting fence、队列取消、writer close、完整目录删除。
- GUI tombstone/outbox。
- 单删、批删、workspace、clear finished/all 共用服务。
- Agent/GUI orphan GC。

退出条件：Agent 离线删除重连后完成；被删 session 不回导；无 transcript/event/queue orphan。

### Phase 6：Observer、NATS 和恢复

- 修复 observer LRU touch/active race。
- NATS 通过 versioned adapter 做 additive envelope 升级，并保留 v1 consumer 兼容和 cursor backfill。
- SQLite startup 使用 reconciling。
- active/queued/approval 状态从 Agent 重建。

退出条件：128+ session 无周期抖动；active observer 不逐出；重启无假状态。

### Phase 7：GUI/Remote 开放 Follow-up

- GUI streaming 时允许创建 queued run。
- queued bubble 使用 canonical `run_id` 展示和取消。
- Remote/mobile 可选择开放 `enqueue_if_busy`，或继续明确拒绝。
- sidebar 同时展示 running/queued/approval 状态。

退出条件：多个 session 交叉排队时，气泡、工具、审批、终态和通知均归属正确 run。

## 13. 验收与故障注入矩阵

| 场景 | 必须验证 |
| --- | --- |
| TUI 生成中连续提交 10 次 | 创建 10 个不同 queued run，FIFO，无幽灵气泡 |
| 重复 `client_request_id` | 返回同一 RunAck，不重复入队 |
| 同 session 并发提交 | `run_sequence` 唯一，始终只有一个 active |
| 多 session 同时运行 | event 不串 session/run，各 session 可独立推进 |
| active run 等待审批 | 同 session 队列不越过；其他 session 不受影响 |
| 模型/思考等级切换 | 已 accepted run 快照不变，下一次提交使用新设置 |
| abort active | 只终结目标 run，默认继续下一个 queued run |
| cancel queued | 目标不启动并有 terminal；其余 FIFO 不变 |
| Agent 在 dequeue 边界 crash | 不重复启动或丢失 queued run |
| GUI/TUI 在 text/tool/approval 阶段退出 | 外部 Agent 继续，重启从 durable cursor 恢复 |
| ring 超过 2,000 event | 从 Agent disk journal 完整补齐 |
| event writer 磁盘满 | run 不假完成，进入 persistence degraded |
| journal 最后一行截断 | 保留前序并修复/忽略 truncated tail |
| persistence degraded 且存在 20 个 queued run | scheduler 熔断，20 个 run 保持 queued，不被排空式失败；修复后继续 FIFO |
| active run streaming 时 writer 持续失败 | bounded backpressure 后 interrupt，不启动新 LLM/tool，内存 pending batch 不超上限 |
| degraded 期间 cancel queued | 返回 `persistence_unavailable` 且内存/磁盘队列均不变；恢复后可重试 |
| durable attachment 在 queued 期间重启/删除 | blob 可恢复且 hash 正确；取消/删除后引用与 orphan 被回收 |
| blob write 与 RunRequest commit 间并发 GC/crash | pending lease/安全窗口阻止误删；过期 orphan 最终可回收 |
| queue checkpoint 任一 fsync/rename 点 crash | 使用最后有效 checkpoint + tail 恢复，队列和幂等映射不丢不重 |
| 飞书/钉钉与 TUI 共用 session | `supersede_session` 取消 active 和 TUI queued，创建唯一新 run；所有被替代项有 terminal |
| tool 已产生副作用但 `tool_end` 未 commit 即 crash | 恢复为 outcome unknown，不自动重跑工具 |
| NATS overflow/断线 | 客户端发现 gap 并从 Agent 补齐 |
| 新 bridge + 旧 mobile | v1 字段和语义不变；旧端不因 additive 字段错位 |
| 256 个 session | observer LRU 稳定，active 永不逐出 |
| 删除含 active+queued 的 session | 全部终结/fence，磁盘与内存完整回收 |
| Agent 离线时 GUI 删除 | tombstone 阻止回导，重连完成删除 |
| legacy multi-turn transcript | 原样展示，新提交创建新 run |
| legacy event compatibility | transcript 气泡可读；有旧日志时保留 partial/inspector，无日志时明确降级 |
| Fork | 复制 transcript，不复制旧 telemetry/queued state |

关键指标：

- session active/queued run count
- queue wait time 和 queue full count
- stale epoch drop count
- event append/fsync latency
- event journal bytes（per-session/global）
- queue journal 行数/bytes
- fsync/group-commit failure count
- writer backpressure time/timeout 和 pending batch bytes
- persistence health 状态和熔断持续时间
- replay disk fallback/cursor gap count
- observer live/sleep/overflow count
- deletion outbox pending age
- orphan GC count
- NATS drop/backfill count

## 14. 回滚和兼容

1. Queued run 在 Agent-first 阶段可先关闭 GUI 入口，但 TUI 必须随 Phase 1/2 同步。
2. `follow_up` 兼容适配器只把请求转换成新 run，不再保留旧同-run语义。
3. Steer 若存在公开外部兼容承诺，先返回 deprecated/unsupported 并发布版本说明，再从下一协议版本移除；仓库内调用可直接删除。
4. 旧 transcript 和 GUI event 文件只读不重写；legacy reader 保留到 retention/产品降级条件满足，不以固定一个发布周期强制退出。
5. 降级到不认识 `queue.jsonl` 的旧 Agent 前必须排空或显式导出 pending queue；禁止直接回滚后让未启动输入静默消失。启动检测到未知 queue schema 时 fail closed 并提示升级，不得忽略文件。
6. Legacy event 文件只在 session 删除、用户显式清理或 retention 到期时删除。
7. NATS v1 字段在兼容窗口内保持 additive-compatible，bridge 不可先于 mobile reader 移除旧字段。
8. Deletion tombstone 在回滚版本中也不得丢弃。

## 15. 明确不在本方案内

- 不改变 bundled sidecar 默认启动/退出策略。
- 不实现 Agent crash 后从任意工具栈帧继续原 active future。
- 不把 token delta 混入 transcript JSONL。
- 不复制 fork 来源 session 的完整 telemetry 或 pending queue。
- 不恢复 Steer 的“中途改变当前生成方向”能力。
- 不要求 GUI、mobile 或 NATS 在线，外部 Agent 仍能自主完成 active/queued run。

## 16. Open Questions（不阻塞核心模型）

以下问题不会改变“一次提交一个 run”的核心，但必须在对应 Phase 开始前拍板并写成配置/发布决策：

1. **Channel 产品策略：**基线使用 `supersede_session` 保持“最新消息覆盖”；是否以及何时把 IM 默认切到 `enqueue_if_busy`。
2. **Legacy 详情保留：**旧 aborted partial output 和 Runs inspector 详细时间线需要保留多久；若无合规要求，建议随用户 retention 清理而不是建设全量 importer。
3. **公开 RPC 兼容窗口：**外部 gRPC 是否有稳定性承诺；这决定 `follow_up/steer` deprecated adapter 保留几个版本。
4. **配额：**每 session/global queued run、单请求附件、blob store 和 event journal 的具体默认上限。
5. **NATS 发布门槛：**当前仍需支持的最低 mobile schema/capability 版本，以及 v1 字段最早移除版本。
6. **Persistence health 粒度：**同一根目录通常按全局熔断；如果不同 session 可配置不同 volume，则按 writer/storage domain 隔离，而不是固定一刀切。

## 17. 逐项复核结论

1. **完整对话逻辑在 Agent：成立。** executor、queued scheduler、审批和 event writer 都在 Agent；GUI/TUI 只是观察和提交。外部 Agent 部署下 GUI 崩溃不影响执行。
2. **JSONL 真源：成立。** transcript 与 per-run event 使用不同物理 journal，但都由 Agent 管理；SQLite 冲突时以 Agent 为准。
3. **Observer 独立和 128 LRU：成立。** 每 session 独立；idle 可休眠，active 永不逐出；React 切换不影响 observer。
4. **身份不串位：成立。** 新协议使用 session/run/epoch/idx；一个提交一个 run 后不再需要第二套 turn identity。
5. **顺序不乱：成立。** session 间不定义总序；session 内 run_sequence，run 内 idx，均由 Agent 单点生成。
6. **设置只影响下一轮：成立。** 每个 run 在 accepted 时快照设置；当前 active 和已 queued run 不受后续切换影响。
7. **审批跨对话展示：成立。** pending approval 归属 session/run；当前对话展示卡片，非当前对话展示列表标记。
8. **无 mobile/桌面可自主运行：成立。** NATS 和 GUI observer 都不是执行依赖。
9. **Sidecar：按已确认边界处理。** 默认打包重启 Agent 可以接受；协议能力支持其他常驻部署。
10. **Follow-up：保留产品行为但更换内部模型。** TUI 当前真实使用，迁移为新 queued run；未来 GUI 复用同一入口。
11. **Steer：删除有明确收益。** 可移除中断、重排、双队列、多 turn 和 transcript rewrite 复杂度。
12. **Turn：完成收敛。** 新数据中产品 turn 与 run 1:1；历史 `turn_id` 仅兼容展示，不参与运行协议。
13. **删除回收：闭环。** 删除 active/queued runtime、Agent 全部 journal 和 GUI 派生数据；离线删除由 tombstone/outbox 保证最终完成。
14. **迁移：有兼容路径。** TUI/Channel 同版本切换，旧 multi-turn transcript 可读，旧 GUI event 文件只读兼容并按明确 retention 回收，不默认建设全量 import RPC。
15. **持久化故障：已闭环。** persistence health 会暂停 scheduler，queued run 不会在磁盘故障期间被排空式失败。
16. **协议兼容：已纳入。** NATS 使用 additive versioned adapter；幂等键、附件和 queued 设置快照均跨重启 durable。
