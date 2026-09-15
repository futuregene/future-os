# 压缩机制开发文档

状态基线：`6b30cce6`，已实现 S2、历史召回和持久化幂等。
本页区分**当前实现**与**待实现设计**；产品规则见 [S2 压缩策略](compaction.zh-CN.md)，查询命令见[历史召回](session-history.zh-CN.md)。

## 1. 核心模型：压缩下一次请求，不改原始历史

压缩不是修改模型正在生成的响应或内部 KV cache，而是让 Agent 为**下一次**模型请求构造较小的消息投影。

```text
原始 journal（持续保存）
        │
        ├── checkpoint 的覆盖范围／保护引用
        │
        ▼
保护的用户／assistant 原文 + 状态摘要 + 最近原始尾部
        │
        ▼
下一次 ModelRequest
```

必须保持以下不变量：

- 原始 user/system/assistant/tool 记录不因压缩被删改。
- 保护的是文本，不把旧工具调用对象、thinking、媒体正文或 provider 隐藏状态重新塞进保护块。
- 最近尾部保持工具调用／结果配对；不能留下孤立工具结果。
- 覆盖边界不能倒退，保护引用必须指向有效的历史条目。
- 摘要不完整、预算不满足或持久化失败时，不启用未经成功提交的 checkpoint。
- 历史中的指令和命令不构成新的执行授权；压缩／召回不重新执行它们。

## 2. 代码导航

| 职责 | 入口 |
|---|---|
| 正常请求前、工具回合后和 provider 超限恢复 | [agent/run_loop.rs](../agent/src/agent/run_loop.rs) |
| 为 run 注入会话配置、持久化和幂等上下文 | [rpc/session_prompt.rs](../agent/src/rpc/session_prompt.rs) |
| 手动压缩执行、用量持久化和返回结果 | [rpc/session.rs](../agent/src/rpc/session.rs) |
| `compact` RPC 的异步 ACK、忙状态与终态通知 | [rpc/commands/settings.rs](../agent/src/rpc/commands/settings.rs) |
| 请求预算 | [compaction/budget.rs](../agent/src/compaction/budget.rs) |
| 规划、分块摘要、重试、最终校验 | [compaction/semantic.rs](../agent/src/compaction/semantic.rs) |
| 持久化幂等入口 `prepare_with_journal` | [compaction/durable.rs](../agent/src/compaction/durable.rs) |
| 指纹、数据库 claim 与原子完成 | [session/compaction_ops.rs](../agent/src/session/compaction_ops.rs) |
| 有序 writer、barrier、`commit_compaction` | [session/persistence.rs](../agent/src/session/persistence.rs) |
| checkpoint 编解码／校验及 fork 重映射 | [checkpoint.rs](../agent/src/session/checkpoint.rs)、[fork.rs](../agent/src/session/fork.rs) |
| 只读历史 search/get | [session/history_query.rs](../agent/src/session/history_query.rs)、[CLI](../cli/src/commands/session_history.rs) |
| 压缩后的模型召回说明 | [agent/history_recall.rs](../agent/src/agent/history_recall.rs) |

## 3. 当前执行流程

### 3.1 请求预算与触发

1. 接受 run，保存新用户消息；每个消息绑定稳定 journal entry ID。
2. 在实际模型步骤前读取当前模型限制，基于原始历史和有效 checkpoint 构造投影。
3. 计入 system、工具定义、消息框架和保守的图片／reasoning 估算；考虑上游报告的用量。
4. 默认经济触发线为 `min(floor(W × 0.8), 256000)`；请求输入上限还要减去配置的最大输出 `O` 和安全余量 `min(2048, W/16)`。
5. 未触发时继续正常请求；需要压缩时进入同一个规划／持久化链路。

经济触发线不是任意截断用户输入的许可。没有旧历史可压、且仍能装进真实请求预算的单条新输入，不应被替换成有损摘要。禁用自动压缩也不取消请求容量保护。

### 3.2 S2 规划与摘要

- 保护用户文本，尽量保留 assistant 原文；尾部目标约 8K，小窗口缩小。
- 历史目标约 32K，不含固定 system／工具定义开销；必要时扩到最多 64K，并服从实际余量。
- 输出放不下时保留能容纳的较新 assistant 文本，其余输出摘要化并加保留策略说明；用户文本仍放不下则明确失败。
- 使用当前模型生成结构化摘要，禁用工具；文本预算最多约 4096，请求局部输出上限最多 8192，不能污染正常请求输出配置。
- 分块预算计算模板、旧摘要、用户补充指令、包装文字、输出和余量。工具内容采用带 entry ID 的头尾摘录。
- 被省略的中段是未知，不得从片段推断全文不存在信息／错误，或声称所有验证已通过。
- 连接和流等待均可取消；重试有界。只有 provider 超限恢复允许确定性应急摘要，仍需校验预算与进展。

### 3.3 提交与下次请求

1. `SessionPersistence.barrier()` 保证已接受的消息追加先落盘。
2. 持久化幂等 claim 在外部模型调用前完成，不跨模型请求长期持有 SQLite 事务。
3. 准备成功后，writer 在一个事务中提交 checkpoint 与 completed 收据。
4. 当前 run 启用成功结果，后续请求使用保护原文、摘要和尾部。
5. 仅在有效 checkpoint、持久化会话、shell 启用且允许使用时，附加一份召回说明；它不写入聊天正文、不累积、不加给摘要模型。

手动 RPC 在接受时返回 ACK，**ACK 不代表压缩完成**。调用方应按 operation ID 处理终态事件，不能只看进程退出码或 ACK。

## 4. 数据与幂等协议

### 4.1 Checkpoint

S2 使用 schema 3，保存覆盖范围、摘要、`protected_entry_ids`、模型／算法信息及用量估计。引用原文而非复制正文。恢复、fork 和历史导入必须验证／重映射引用；旧 schema 2 继续可读，已真正删除的旧历史无法恢复。

### 4.2 收据

`compaction_operations` 主键是 `(session_id, input_key)`。原始输入指纹按顺序扫描 canonical journal 中的 user/system/assistant/tool 条目，包含其身份和内容，而不只是“最后一条 ID”。对象键排序规范化，数组／消息顺序保留。

策略键还包含模型／协议／思考档位／cwd／工具配置、预算、触发模式／阶段、规范化指令及摘要策略。checkpoint、run 标记、用量和 session-info 更新不改变原始历史指纹。

```text
无收据 ──原子 claim──> started ──checkpoint + 收据同事务──> completed
                         │                                  │
                         └──可记录的已知失败──> failed       └──同键复用

started 没有持久化结果：返回 compaction_indeterminate，不自动重发模型请求。
```

`indeterminate` 是返回诊断，不是表中的第四种状态。它可能表示仍在执行，也可能表示崩溃中断，不能据此断言已扣费或未扣费。

- 同键成功复用不追加 checkpoint、不重复模型调用或记账；即使原来还保留最近尾部也如此。
- 新消息、原文内容变化或相关参数变化形成新键；不同触发模式不互相冒充同一个操作。
- 复用旧结果不撤销后来的不同参数操作。
- 缓存 checkpoint 缺失、被改动或范围无效时拒绝复用，不静默重生成。
- 已知失败复用错误；不确定状态需要明确的新操作或人工处理，不设置自动过期重跑。
- 删除会话清理收据；fork 使用新会话范围；完全内存／ephemeral 调用没有跨重启保证。

RPC `operationId` 标识请求尝试；复用结果中的 `sourceOperationId` 指向原始压缩操作。内存 request-ID 去重是补充，不能代替持久化内容键。调整压缩语义、参数组成或序列化规则时，应审查并更新幂等策略版本，避免错误复用旧结果。

## 5. 是否提供模型可调用的压缩 CLI？

**建议分层提供，不建议把现有手动 RPC 直接暴露成模型同步自调用。以下全部为待实现设计。**

### 5.1 用户／自动化管理入口：值得增加

建议接口示意（当前不存在）：

```text
future session compact --session SESSION_ID --json
```

封装现有 `compact` RPC，复用预算、幂等和持久化，不直接操作数据库。初版默认返回异步 ACK。手动入口保留“活跃 run 时拒绝”的规则；若以后增加等待选项，需订阅／恢复正确的终态事件，不能把 ACK 当成功。

### 5.2 模型入口：只请求，不现场执行

模型调用 shell 时，其 run 仍活跃，因此直接调用现有手动入口会得到 `session_busy`。如果简单移除保护并同步等待，会形成：

```text
run 等待 shell 返回
shell 等待 compact 完成
compact 等待 run 的安全边界
```

此外还可能覆盖尚未落盘的工具结果，或与自动压缩并发。正确入口应提交一个**非阻塞的压缩意图**，而不是嵌套执行摘要。

建议接口示意（当前不存在）：

```text
future session compact request --session SESSION_ID --request-id INTENT_ID --json
```

建议执行顺序：

1. 服务端验证目标与当前工具执行的 session/run/epoch 绑定，记录一个待处理意图。
2. CLI 立即返回“已接受请求”，不等待完成；模型也不轮询等待自身 run。
3. 当前工具调用及其结果照常进入 journal。
4. Agent 在下一次正常模型调用前消费意图，与自动／超限压缩共用单个协调入口。
5. 用当时的稳定历史重新检查必要性和预算，再走既有幂等、摘要与提交路径。
6. 无必要／无收益则 no-op，成功则换用新投影；失败按既有安全策略继续或停止。

模型请求只能是建议：不授予绕过容量、预算、未知状态或权限的 `force` 能力，也不替代自动触发兜底。若尚无可信的 run/epoch 调用来源绑定，先只开放用户管理 CLI，不开放模型入口；自由填写 `--session` 不是授权证明。

### 5.3 幂等之外还要防自触发循环

模型发出的请求工具调用和 ACK 本身也会成为新历史。因此“原始历史指纹改变了”不能证明值得再压缩，持久化内容幂等也不能独自阻止无限自压缩。

模型入口需要另外的 run/epoch/intent 去重、待处理请求合并、已消费状态及有界的频率／收益检查；摘要请求仍不能使用工具。不能因为模型换了一个 request ID，就绕过已有 pending 或 indeterminate 状态。

### 5.4 两段提示词不要混用

- **现有历史召回说明**：只在压缩后启用。
- **未来主动请求压缩的能力说明**：仅在能力实现并启用后，作为短说明在首次可用时提供；否则模型第一次压缩前根本不知道入口。

后者应强调自动机制仍负责容量安全、请求只在边界执行、不等待／轮询、不反复请求。当前没有这个 CLI，不应提前把它写成可用能力。

## 6. 开发与测试清单

必测：同键重复／并发、保留尾部后的重复、重启复用、原文同 ID 改内容、参数变化、事务失败、中断 started 不重跑、fork／删除、缓存损坏、摘要后 usage、正常输出配置不被污染。

若实施模型入口，还必须增加：

- 活跃 run 内提交意图立即返回，工具结果落盘后才执行；无等待环。
- 自动触发与模型意图同时出现，只执行一轮协调决策。
- 过期 epoch、取消／结束的 run 和不可信跨会话目标被拒绝。
- 不断换 intent ID、自调用请求造成的历史增长，不能引发无界摘要／计费。
- 无收益、未知收据、持久化失败和 provider 超限都有明确结果。
- 摘要模型无工具；CLI 版本不匹配、shell 不可用和 ephemeral 场景不宣传不可用能力。

使用项目内 worktree；隔离测试 HOME 和新端口，不重启现有 Agent。macOS 全套测试应提高文件描述符上限；涉及 sandbox 断言的 HOME 放在项目内 target/test-homes，而非系统 temp 放行区。不得在测试日志输出凭据或把私人会话送给外部模型。

幂等命中仍有原始历史指纹和缓存校验的本地成本，不是 O(1) 或零开销；原始历史增长下应单独测 I/O、内存和延迟。合成模型桩证明执行链路，不能替代真实模型的摘要／召回质量评估。
