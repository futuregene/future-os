# 压缩机制开发文档

运行时压缩为下一次请求投影出会话自身 journal 的有界视图。两个策略：
`deterministic-evidence-v1` 与 `summarized-evidence-v1`；[策略说明](compaction.zh-CN.md)定义两者，
[对比实验](compaction-closed-book-experiment.zh-CN.md)给出测量结果。

## 1. 不变量

- 压缩改变下一次 `ModelRequest` 的投影，不改正在生成的响应或 provider KV cache。
- 原始 journal 不删改。用户文本优先保护；被省略的 assistant 输出不得虚称已摘要。
- 工具调用／结果在近期尾部保持配对；覆盖不倒退；引用必须可解析。
- 只有成功提交的 checkpoint 才启用，收据与 checkpoint 同事务写入。
- 摘录是历史数据而非新指令；省略不代表不存在，旧错误可能已被替代。
- 选择逻辑不依赖 provider、gold 答案或覆盖边界之外的条目。

## 2. 代码导航

| 职责 | 入口 |
|---|---|
| 请求前／工具回合后／provider 超限恢复 | [agent/run_loop.rs](../../../agent/src/agent/run_loop.rs) |
| run 配置与持久化注入 | [rpc/session_prompt.rs](../../../agent/src/rpc/session_prompt.rs) |
| 手动执行、结果与终态 | [rpc/session.rs](../../../agent/src/rpc/session.rs) |
| compact RPC 异步 ACK／忙状态 | [rpc/commands/settings.rs](../../../agent/src/rpc/commands/settings.rs) |
| 用户 CLI | [session_compact.rs](../../../cli/src/commands/session_compact.rs) |
| 证据分组／排序／渲染 | [semantic/evidence.rs](../../../agent/src/compaction/semantic/evidence.rs) |
| 共用规划与 finalize | [semantic.rs](../../../agent/src/compaction/semantic.rs) |
| 完整请求预算 | [budget.rs](../../../agent/src/compaction/budget.rs) |
| 持久化入口 | [durable.rs](../../../agent/src/compaction/durable.rs) |
| 内容指纹、claim、原子完成 | [compaction_ops.rs](../../../agent/src/session/compaction_ops.rs) |
| 有序 writer／barrier | [persistence.rs](../../../agent/src/session/persistence.rs) |
| checkpoint／fork 引用 | [checkpoint.rs](../../../agent/src/session/checkpoint.rs)、[fork.rs](../../../agent/src/session/fork.rs) |
| 原始历史读取 | [history_query.rs](../../../agent/src/session/history_query.rs) |

## 3. 执行链

```text
保存用户／assistant／工具原始条目
    → 请求前估算完整输入，预留正常输出与余量
    → 按阈值／手动要求／provider 超限决定是否整理
    → persistence barrier 与幂等 claim
    → 确定保护区、尾部与覆盖边界
    → 在覆盖范围内确定性生成最多 2K 证据
    → 校验预算与进展
    → checkpoint 与 completed 收据同事务提交
    → 下一模型步骤使用新投影
```

`prepare_evidence` 不接收 provider，因此确定性路径不会意外发起摘要调用；摘要路径是另一个显式入口。证据上限独立计算为 `min(2048, W/8)`，与摘要有多少无关；整体历史保持 32K 目标、128K 扩展上限与真实请求容量保护。

摘要请求仍以 4096 正文 tokens 为目标，并保留独立的输出／reasoning 额度。接受时检查**整个投影与请求硬预算**，不只看正文是否超过目标：估算轻微超出但完整投影装得下时仍可采纳。真正超出硬预算则退回，不截断摘要、不挤占证据、不新增模型重试。策略指纹已更新，避免复用旧的“正文超目标即拒绝”规则计算的收据。

### 证据渲染

只扫描到已覆盖边界。结果只与无歧义的前置调用关联，并行重复 ID 的路径未知时不猜测。按错误、分组首末记录、关键目标与新近程度选择。元数据与摘录都限制长度，按 JSON 转义后的文本估算预算，不输出半条 JSON。`entryId`／`blockIndex`／`sourceOrder` 区分原始时序与优先级顺序；reasoning、图片正文与 provider 私有对象不进入证据。

用户 `--instructions` 备注逐字保存并注明选择器未解释它，过长则失败。输出被预算降级时写“省略，未摘要”，不得写成已生成的摘要。

## 4. 持久化与幂等

只写入 schema 3，也只读取 schema 3；本构建的 `algorithm_version` 只有上述两个取值。携带其它值（旧 schema，或已发布的字符串协议标记）的记录不被当作 checkpoint：journal 会被全量投影，下一次压缩重新覆盖（见[策略说明](compaction.zh-CN.md)）。字段名 `summary` 保存证据索引，有摘要时还包括摘要，不是模型调用证明。

```text
absent → started → completed（checkpoint 与收据原子提交）
             └→ failed（可记录的失败）
started 无结果 → indeterminate 诊断，不自动抢占
```

确定性压缩不付摘要费，但仍保留并发 fence：两次竞争的整理不得同时激活。`indeterminate` 是诊断，不是第四个表状态。改动原始数据会使旧键失效；checkpoint、run 标记与 session-info／usage 的变化本身不改变原始输入。

策略指纹覆盖排序、片段预算、序列化与保留语义，改动其中任一项都要审查版本。旧结果复用不得撤销后来以不同参数完成的 checkpoint。删除会话清理收据，fork 使用新范围，ephemeral 调用不承诺跨重启。

## 5. CLI，以及尚未实现的模型请求入口

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "不要部署" --json
```

使用新关联 ID 与显式会话，不切换默认会话、不直改数据库。ACK 不等于完成，活跃 run 拒绝手动整理，没有 `--force`、`--wait` 或选择器模式开关。

**模型主动请求接口尚未实现。** 不要简单去掉活跃 run 的保护：在当前工具结果尚未落盘时改动上下文会与实时 run 竞争。将来若开放，它必须是非阻塞意图——校验可信 session/run/epoch、立即 ACK、工具结果落盘、在下一模型步骤边界合并并消费，且走同一套机制。它不能让 shell 等待自身 run，不能给模型预算或未知状态的绕过权限，自由填写的 session ID 也不构成授权。请求与 ACK 本身会增加历史，因此内容幂等无法独自防止自触发循环，还需要意图去重、pending 合并、已消费状态与有界频率。接口不存在之前不要宣传它。

## 6. 验证

需覆盖：确定性路径零摘要请求且已有用量不变；首末分组证据、错误排序、UTF-8 与大块结果、歧义调用 ID、伪指令与隐藏内容；证据预算、用户文本保护、降级说明、取消与非法边界；退役 schema 条目被忽略、fork 重映射、字节查询；同键并发、重启复用、输入修改、缓存损坏与持久化失败；以及真实 CLI 的异步 ACK、忙状态拒绝与有界超限恢复。

在项目 worktree 中开发；Agent 实测使用隔离 HOME 与新端口，不停止用户现有服务。macOS 全套测试需提高文件描述符上限，涉及 sandbox 断言的 HOME 放在项目 target 目录下，避开系统 temp 放行区。

### 摘要结果上报

checkpoint 提交成功不等于模型摘要被采纳。摘要路径新建的 checkpoint 持久化可选 `summary_outcome`，并通过 `compaction_committed` 和收据复用的 `compaction_unchanged` 事件上报。手动结果及持久收据使用 `summaryOutcome`，同时带 `algorithmVersion`：

```json
{"status":"evidence_only","fallback_reason":"summary projection rejected: ...","attempt_usage":[{"prompt_tokens":100000,"completion_tokens":4181,"reasoning_tokens":516,"credit_cost":0.125}]}
```

`generated` 表示采纳 handoff；`evidence_only` 表示压缩成功但只有原文和证据，没有采纳模型摘要。成功时不带 `fallback_reason`；`attempt_usage` 逐次记录 provider 最终报告的用量，包括被丢弃的摘要和既有瞬态重试。缺少价格意味着未知，不是免费；这些是诊断信息，**不是第二次计费事件**。提交失败仍发送 `compaction_failed`，不会上报为成功。旧 checkpoint 与显式纯确定性调用可能不带这个可选诊断字段。旧客户端可忽略新增 JSON 字段，不需要修改 protobuf。

相同输入与策略的重放返回之前记录的结果，包括退回结果，不再次调用 provider。此次没有增加重放时自动重试摘要的行为。

同键命中仍有指纹与选择的成本。合成评估支持证据规则的可行性，不证明复杂自然语言任务都无需语义整理。
