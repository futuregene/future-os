# 上下文投影与压缩检查点架构方案

状态：**设计提案**（2026-08-24）

本文定义 FutureOS agent 的会话历史、模型上下文投影和上下文压缩之间的长期架构边界。目标不是只修复一次错误的“上下文已压缩”提示，而是消除当前消息过滤、压缩判断、provider 元数据、会话持久化和 UI 事件之间的结构性耦合。

## 1. 背景与已确认问题

当前 run loop 在调用 `transform_context` 前记录原始 `AgentMessage` 数量，随后执行：

```text
AgentMessage
    -> ConvertToLLM
    -> transform_context
    -> ConvertFromLLM
    -> 根据消息数量是否减少推断是否发生压缩
```

`ConvertToLLM` 会过滤只包含 reasoning block 的 assistant message。因此，下面的序列会产生误报：

1. 模型流在 reasoning 阶段被中断；
2. reasoning-only assistant 被保存到会话历史；
3. 下一轮构造上下文时，`ConvertToLLM` 将其过滤；
4. `transform_context` 没有执行任何压缩；
5. 转换后的数量小于转换前数量；
6. run loop 错误发出 `CompactionEnd`；
7. UI 显示“上下文已压缩”。

因此，`result.len() < before_len` 不是可靠的压缩判据。把比较基准改成转换后的消息数量虽然能修复这一例误报，但无法解决下述结构性问题，不应作为最终方案。

## 2. 当前架构的结构性风险

### 2.1 有损投影被写回会话状态

`AgentMessage` 是内部的丰富消息类型，provider adapter 也已经直接消费它。旧 `Message` 类型无法完整表示 `AgentMessage`，两者往返可能丢失：

- `AgentMessage.metadata`，包括 `run_id` 和附件信息；
- OpenAI Responses item ID、encrypted reasoning 等 provider metadata；
- Anthropic thinking signature 和 redacted thinking；
- `ToolResult.is_error`；
- 未来新增但旧 `Message` 无法表示的 `ContentBlock`。

模型上下文投影可以是有损的，但有损结果不能再覆盖会话真相。当前 `ConvertToLLM -> ConvertFromLLM` 的回写违反了这个边界。

### 2.2 压缩结果存在多套真相

当前实现同时依赖：

- 返回消息数量；
- `last_compaction_result`；
- `compaction_occurred`；
- `compaction_failed`。

这些状态由不同路径更新。误报场景已经出现“长度表示发生压缩，但 result、occurred 和 failed 均表示没有压缩”的矛盾。自动压缩和 provider 上下文超限后的强制压缩也没有共享完全相同的状态提交路径。

### 2.3 压缩会重写完整会话日志

压缩后重建 JSONL 会带来不必要的风险：

- 进程异常退出时需要处理整文件替换的一致性；
- 新旧消息按索引继承时间戳，但压缩会删除前缀并插入摘要，索引不再对应；
- 完整历史、模型上下文和 UI 展示被迫共享同一份经过裁剪的数据；
- 后续审计和故障分析无法可靠还原压缩前的模型输入来源。

### 2.4 使用字符串模拟领域事件

当前压缩摘要以伪用户消息 `[Context compaction: ...]` 表示，并在持久化时通过字符串前缀重新识别。这会导致：

- 真实用户消息与内部协议发生命名冲突；
- 结构化信息只能嵌入字符串；
- schema 演进和兼容迁移困难；
- UI、持久化和模型输入共同依赖隐式约定。

### 2.5 UI 事件早于可靠提交语义

当前 `CompactionEnd` 缺少稳定的 checkpoint ID、触发来源和被替换的日志范围。UI 无法可靠去重，也无法区分“内存里生成了摘要”和“检查点已经成功持久化”。

## 3. 设计原则

新架构遵守以下约束：

1. **会话日志是唯一事实来源**：已经发生的消息和运行状态只追加，不因模型上下文优化而删除。
2. **模型上下文是派生视图**：过滤、修复和压缩只影响发给模型的 prompt，不反向覆盖日志。
3. **压缩是显式领域操作**：调用方通过强类型结果知道是否发生压缩，不从长度或副作用推断。
4. **自动和强制压缩共用一条路径**：差异只体现在 trigger，不复制状态机和持久化逻辑。
5. **事件代表已提交事实**：永久 UI 标记只由成功落盘的检查点产生。
6. **保留 provider 语义**：prompt 变换不得静默丢失 provider 后续请求所需的元数据。
7. **每个派生结果可追溯**：摘要必须记录它覆盖的稳定日志范围和算法版本。

## 4. 目标架构

```text
不可变 Session Journal
    |
    +-- UI Projection -----------------> 完整会话展示
    |
    +-- Prompt Projection
            |
            +-- replay eligibility
            +-- 最新有效 checkpoint
            +-- checkpoint 后的日志条目
            +-- tool-call 一致性修复
            |
            v
        PromptContext
            |
            v
        Context Manager
            |
            +-- Unchanged
            |
            +-- Compacted(checkpoint)
                    |
                    +-- append journal entry
                    +-- emit committed event
```

### 4.1 不可变 Session Journal

`SessionEntry` 是持久化事实层：

- 每条 entry 具有稳定 `entry_id`、`run_id`、时间戳和类型；
- assistant reasoning-only、中断和失败状态仍然保留；
- 压缩只追加新的 checkpoint entry，不重写旧消息；
- UI、审计和故障恢复都可以读取完整历史；
- journal 写入保持 append-only，单条记录使用现有 JSONL 追加和刷新语义。

reasoning-only 是否可发送给模型，不再通过“是不是空消息”隐式判断，而由 prompt projection 的 replay eligibility 规则决定。建议至少区分：

```rust
enum ReplayEligibility {
    Replayable,
    DisplayOnly { reason: DisplayOnlyReason },
}

enum DisplayOnlyReason {
    InterruptedReasoning,
    IncompleteAssistantTurn,
    UiOnlyEvent,
}
```

### 4.2 Prompt Projection

新增显式的模型上下文投影：

```rust
struct PromptContext {
    messages: Vec<AgentMessage>,
    source_range: EntryRange,
    usage: ContextUsage,
}
```

投影过程负责：

- 找到最新有效压缩检查点；
- 以 checkpoint summary 作为上下文前缀；
- 追加 checkpoint cutoff 之后可重放的日志消息；
- 排除中断 run 的 reasoning-only assistant 等 display-only entry；
- 修复 dangling tool call/tool result；
- 保留 provider metadata、tool error 和附件语义。

投影结果只存在于一次模型请求的准备阶段，绝不用于覆盖 `SessionEntry`。

### 4.3 显式 Context Manager 结果

`transform_context: Fn(Vec<Message>) -> Vec<Message>` 应被强类型接口替代：

```rust
enum ContextPreparation {
    Unchanged {
        prompt: PromptContext,
        usage: ContextUsage,
    },
    Compacted {
        prompt: PromptContext,
        checkpoint: ContextCheckpoint,
    },
}

enum ContextError {
    NoValidBoundary,
    SummaryGenerationFailed,
    InvalidSummary,
    PersistenceFailed,
}
```

调用方只根据 enum variant 执行动作：

```rust
match context_manager.prepare(projected_context, trigger).await? {
    ContextPreparation::Unchanged { prompt, .. } => run_model(prompt),
    ContextPreparation::Compacted { prompt, checkpoint } => {
        journal.append_checkpoint(&checkpoint).await?;
        events.emit(CompactionCommitted::from(&checkpoint));
        run_model(prompt)
    }
}
```

不再比较输入输出长度，也不再使用共享的 `last_compaction_result` 或 occurred/failed 原子变量传递结果。

### 4.4 结构化压缩检查点

建议的数据模型：

```rust
struct ContextCheckpoint {
    id: CheckpointId,
    cutoff_entry_id: EntryId,
    summary: Vec<ContentBlock>,
    tokens_before: u64,
    tokens_after: u64,
    trigger: CompactionTrigger,
    algorithm_version: String,
    model: String,
    context_window: u64,
    created_at: DateTime<Utc>,
}

enum CompactionTrigger {
    Automatic,
    ProviderContextLimit,
    Manual,
}
```

`cutoff_entry_id` 表示 summary 已覆盖到哪个日志条目。下一轮 prompt 由 summary 加该 entry 之后的可重放消息组成，不依赖消息数组索引。

对应 JSONL entry 应是显式结构，而不是伪用户消息：

```json
{
  "type": "compaction",
  "checkpoint_id": "cp_...",
  "cutoff_entry_id": "entry_...",
  "summary": [],
  "tokens_before": 120000,
  "tokens_after": 18000,
  "trigger": "automatic",
  "algorithm_version": "v2",
  "model": "...",
  "context_window": 200000
}
```

### 4.5 统一自动、强制和手动压缩

三种触发方式全部进入同一个 `ContextManager`：

| 触发方式 | trigger | 行为差异 |
| --- | --- | --- |
| token 阈值 | `Automatic` | 正常阈值策略 |
| provider context-limit 错误 | `ProviderContextLimit` | 强制执行一次，不使用伪造 token 值 |
| 用户手动操作 | `Manual` | 明确的用户触发来源 |

不同入口不再自行修改 messages、设置标记或发送事件。重试策略也以 checkpoint 是否成功提交为边界，避免在内存里压缩成功但重启后丢失。

### 4.6 事件提交语义

事件建议区分：

- `compaction_started`：可选的临时进度事件；
- `compaction_committed`：checkpoint 成功追加到 journal 后发出；
- `compaction_failed`：携带结构化错误和 trigger。

`compaction_committed` 至少携带：

```text
checkpoint_id
cutoff_entry_id
trigger
tokens_before
tokens_after
algorithm_version
```

UI 只根据 `compaction_committed` 创建永久标记，并通过 `checkpoint_id` 去重。历史重放和 reconnect 不应重复创建同一标记。

### 4.7 Token 使用模型

上下文策略应使用结构化的 `ContextUsage`，明确区分：

- 当前模型输入 token；
- cached input token；
- completion token；
- reasoning token；
- 模型 context window；
- 估算值和 provider 实测值。

自动压缩的主要判据是下一次模型输入相对于 context window 的占用，不应把上一轮 completion/reasoning token 混入 prompt 大小，也不应在 provider-limit 路径写入 `999999` 一类占位值。

## 5. 不采用的方案

### 5.1 只修正长度比较基准

例如改为比较 `result.len()` 和 `llm_msgs.len()`。它能修复当前误报，但仍然保留有损往返、重复压缩路径、字符串协议、整文件重写和多源状态，不作为正式方案。

### 5.2 仅检查 `last_compaction_result.is_some()`

这仍依赖共享副作用，并要求每个成功和失败分支正确同步多个状态。新增压缩入口后仍容易再次分叉，只适合临时止血。

### 5.3 显式 outcome，但继续把压缩结果写回完整历史

这一方案可以修复误报和状态分叉，但仍会丢失完整会话、引入 JSONL 重写风险，也无法清晰区分 transcript 和 prompt。可以作为迁移阶段，不能作为终态。

## 6. 分阶段迁移计划

### Phase C1：显式结果与统一入口

- 新增 `ContextPreparation`、`ContextError` 和 `CompactionTrigger`；
- 自动压缩和 provider-limit 压缩统一进入 `ContextManager`；
- 删除基于长度的压缩判断；
- 删除 `last_compaction_result`、`compaction_occurred`、`compaction_failed`；
- provider-limit 路径停止使用占位 token；
- 补齐当前误报的跨层回归测试。

此阶段允许暂时维持旧持久化格式，但接口必须按最终模型设计，避免下一阶段再次改调用方。

### Phase C2：建立 AgentMessage 原生 Prompt Projection

- `ContextManager` 改为接收 `PromptContext` 或 `Vec<AgentMessage>`；
- run loop 删除 `ConvertToLLM -> ConvertFromLLM` 回写；
- `Message` 仅保留在确有需要的旧兼容或 provider 边界；
- 为 replay eligibility 建立明确规则；
- 增加 provider metadata、tool error、附件和新增 content block 的保真测试。

### Phase C3：追加式检查点

- 为 `SessionEntry` 增加正式的 compaction checkpoint schema；
- 为旧日志补充稳定 entry ID，或在加载时生成可稳定迁移的 ID；
- prompt projection 读取最新有效 checkpoint 加其后的日志；
- 停止因压缩重写整个 JSONL；
- 保留旧 `[Context compaction: ...]` 的只读兼容和一次性规范化逻辑；
- 禁止新代码继续生成字符串形式的压缩消息。

### Phase C4：UI 和遥测收口

- UI 改用 `compaction_committed`；
- marker 使用 checkpoint ID 去重；
- 展示真实 trigger 和 before/after token；
- 增加 checkpoint 创建、失败、恢复、压缩率和重复触发指标；
- 旧 `compaction_end` 仅用于旧会话兼容，完成迁移后删除。

## 7. 验证矩阵

### 7.1 消息投影

- reasoning-only assistant 被 prompt projection 排除，但保留在完整会话历史；
- 中断 reasoning 不产生任何压缩事件；
- 正常 reasoning + text assistant 保持可重放；
- dangling tool call 修复不被解释为压缩；
- 用户发送以 `[Context compaction:` 开头的真实消息时不会被当成内部事件。

### 7.2 数据保真

- OpenAI Responses item ID 和 encrypted reasoning 在投影、压缩后完整保留；
- Anthropic thinking signature/redacted data 完整保留；
- `ToolResult.is_error` 不变化；
- `AgentMessage.metadata`、附件和未知 content block 不被静默删除。

### 7.3 压缩状态机

- no-op preparation 只返回 `Unchanged`；
- 真压缩只生成一个 checkpoint 和一个 committed event；
- 自动、provider-limit、手动压缩使用相同 checkpoint schema；
- checkpoint 持久化失败时不发送 committed event；
- provider-limit 重试最多执行既定次数，不形成压缩重试循环。

### 7.4 持久化与恢复

- 压缩前后的完整 journal 内容保持不变，只追加 checkpoint；
- 重启后重建出的 `PromptContext` 与提交 checkpoint 后一致；
- 多个 checkpoint 只应用最新有效 checkpoint；
- cutoff entry 缺失或 checkpoint 损坏时安全回退并报告错误；
- 旧格式会话可以读取，且不会重复迁移；
- 写入 checkpoint 中途崩溃不会破坏此前 journal。

### 7.5 UI

- reconnect、event replay 和任务重开不会重复显示同一 checkpoint；
- `compaction_started` 不创建永久 marker；
- failed/aborted compaction 不显示“已压缩”；
- tokens 为零或摘要为空的非法 committed event 被 projection 拒绝并记录诊断。

## 8. 完成标准

当以下条件全部满足时，架构迁移才算完成：

1. run loop 不再通过数组长度推断压缩；
2. prompt projection 的有损结果不再覆盖 session journal；
3. 自动和强制压缩只有一个实现入口和一个结果类型；
4. 压缩不再重写完整会话日志；
5. 新会话不再写入 `[Context compaction: ...]` 伪用户消息；
6. UI 永久标记只对应已持久化的 checkpoint；
7. provider metadata 和 tool result 语义通过压缩保真测试；
8. reasoning 中断、tool 修复和普通过滤不会触发压缩提示；
9. 旧会话保持可读并有明确的兼容退出计划。

## 9. 决策

采用“**不可变 Session Journal + AgentMessage 原生 Prompt Projection + 显式 ContextCheckpoint**”作为目标架构。

当前误报必须通过 C1 的显式 outcome 修复，但不接受仅调整长度比较作为最终实现。C1 到 C4 应按顺序落地，每阶段保持兼容和独立测试，最终删除旧 `Message` 往返、多套压缩状态和字符串压缩协议。
