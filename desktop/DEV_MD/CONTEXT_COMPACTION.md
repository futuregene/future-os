# 上下文投影与压缩检查点架构方案

状态：**设计提案**（2026-08-24）

本文定义 FutureOS agent 的会话历史、模型上下文投影和上下文压缩之间的长期架构边界。目标不是只修复一次错误的“上下文已压缩”提示，而是消除当前消息过滤、压缩判断、provider 元数据、会话持久化和 UI 事件之间的结构性耦合。

本方案有一条不可放宽的交付约束：**迁移不得造成任何用户可感知的数据 break**。升级后，既有会话、消息、reasoning、tool call/result、附件、run 状态、压缩标记、fork/clone 和导出结果必须继续可读；不得因为 schema 更新丢消息、重复消息、改变消息归属、隐藏既有压缩标记，或让原本可继续的会话无法继续。agent 和 desktop 同版本发布，因此不要求支持新旧进程在线混用，但必须兼容磁盘上由历史版本写入的 session JSONL 和 run-event JSONL。

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
8. **持久化 schema 只做加法演进**：复用现有 `SessionEntry` envelope，新代码双读旧格式和新格式，不就地破坏或批量改写旧记录。
9. **兼容以用户结果为准**：内部类型、RPC 事件名可以演进，但升级前后用户看到的完整历史、消息顺序、运行状态和可继续性必须一致。

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

- 每条 entry 具有稳定 `id`、时间戳和类型；新写入的消息 entry 应携带 `run_id`，历史 entry 的 `run_id` 允许缺失并由相邻 `run_started` / `run_terminal` 及日志顺序派生；
- assistant reasoning-only、中断和失败状态仍然保留；
- 压缩只追加新的 checkpoint entry，不重写旧消息；
- UI、审计和故障恢复都可以读取完整历史；
- journal 写入保持 append-only，沿用现有单行 JSONL envelope；checkpoint 的“已提交”以追加、flush 和 `fsync` 全部成功为边界。

当前 `SessionEntry.id` 已经是必填且稳定字段，C3 不对正常旧日志重新编号。对极早期或外部导入的缺失 ID 记录，只允许在加载投影中生成确定性兼容 ID；除非用户明确执行修复操作，否则不得为了补 ID 重写原文件。

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
    messages: Vec<ProjectedMessage>,
    source_range: EntryRange,
    usage: ContextUsage,
}

struct ProjectedMessage {
    message: AgentMessage,
    // 一个模型消息可能来自一个或多个 journal entry；合成修复项可以为空。
    source_entry_ids: Vec<EntryId>,
    replay_eligibility: ReplayEligibility,
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

`source_range` 只描述本次投影读取的整体范围，不能单独用于生成 checkpoint。`ContextManager` 选择压缩边界时，必须通过每个 `ProjectedMessage.source_entry_ids` 把模型消息边界映射回 journal。合成的 dangling-tool 修复项没有独立 entry ID，不得成为 cutoff；压缩器必须向前或向后移动到最近的完整、可重放且 tool-call 一致的真实 entry 边界。最终 checkpoint 应携带已经验证存在的连续 `covered_entry_range`，其中 `cutoff_entry_id` 是该范围的闭区间终点。

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
    covered_from_entry_id: EntryId,
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

`covered_from_entry_id..=cutoff_entry_id` 表示 summary 覆盖的连续 journal 范围。下一轮 prompt 由 summary 加 cutoff 之后的可重放消息组成，不依赖消息数组索引。提交前必须验证两个边界均存在、顺序合法、checkpoint 自身位于 cutoff 之后，并且边界没有切开 tool call/result 组合。

对应 JSONL entry 必须复用当前 `SessionEntry` envelope。checkpoint 字段放入 `content`，不能使用缺少 `id` 的新顶层结构，也不能依赖 serde 会忽略的未知顶层字段：

```json
{
  "id": "entry_...",
  "type": "compaction",
  "role": "system",
  "content": {
    "schema_version": 2,
    "checkpoint_id": "cp_...",
    "covered_from_entry_id": "entry_...",
    "cutoff_entry_id": "entry_...",
    "summary": [],
    "tokens_before": 120000,
    "tokens_after": 18000,
    "trigger": "automatic",
    "algorithm_version": "v2",
    "model": "...",
    "context_window": 200000
  },
  "timestamp": "2026-08-24T12:00:00+08:00"
}
```

读取端必须同时接受三种历史表示：

1. `[Context compaction: ...]` 伪用户消息；
2. 当前已有的 `type: "compaction"`、`content: {summary,tokens_in,tokens_out}`；
3. 本方案的 `schema_version: 2` checkpoint。

旧格式只在内存中规范化成兼容 checkpoint/projection，不因读取而重写原 JSONL。新代码只写 v2 envelope。遇到损坏、引用不存在或边界非法的 v2 checkpoint 时，记录结构化诊断并回退到它之前最近的有效 checkpoint；如果不存在，则从完整 journal 安全重建，不丢弃原始 entry。

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

UI 永久标记只对应 durable checkpoint，并通过 `checkpoint_id` 去重。live `compaction_committed` 用于即时插入；历史加载、重放和 reconnect 从同一个 checkpoint projection 恢复，不应重复创建标记。

checkpoint journal entry 是永久事实来源，live event 只是低延迟通知。若进程在 checkpoint `fsync` 成功后、event 发出前崩溃，重启/reconnect 必须从 session journal 投影出等价的 `compaction_committed` 或直接投影 UI marker。`get_session_entries` 应扩展结构化 checkpoint payload，或新增 checkpoint projection RPC；不能继续把所有 compaction entry 过滤掉后只依赖瞬时事件。

run-event JSONL 保持现有事件 envelope，只新增 `compaction_started`、`compaction_committed`、`compaction_failed` 类型和 payload。读取和 UI projection 必须继续识别历史 `compaction_end`。新旧事件映射到同一个 checkpoint marker 模型，旧事件使用确定性兼容 ID 去重；不得因为 event vocabulary 更新让历史 marker 消失或重复。

兼容 marker key 的优先级固定为：v2 `checkpoint_id`；旧 compaction entry 的 `entry.id`；仅有旧 event 时使用 `session_id + run_id + event_sequence`。同一 run 同一序位同时存在旧 compaction entry 和 `compaction_end` 时，projection 必须先关联二者再生成一个 marker，不能把两种来源各显示一次。

### 4.7 Token 使用模型

上下文策略应使用结构化的 `ContextUsage`，明确区分：

- 当前模型输入 token；
- cached input token；
- completion token；
- reasoning token；
- 模型 context window；
- 估算值和 provider 实测值。

自动压缩的主要判据是下一次模型输入相对于 context window 的占用，不应把上一轮 completion/reasoning token 混入 prompt 大小，也不应在 provider-limit 路径写入 `999999` 一类占位值。

### 4.8 Fork、clone、导入与导出

checkpoint 引用了 session 内的 entry ID，因此复制会话时必须显式维护引用完整性：

- 如果 fork/clone 保留原 entry ID，则 checkpoint 引用保持不变；
- 如果 fork/clone 为 entry 重新编号，则必须先建立完整 old-to-new ID map，再原子重写 `covered_from_entry_id` 和 `cutoff_entry_id`；
- fork 点早于某 checkpoint 的 cutoff 时，该 checkpoint 不得复制；
- fork 点晚于 checkpoint 时，复制后的 checkpoint 必须经过范围和 tool 边界校验；
- 导入旧日志使用与正常加载相同的双读和确定性兼容 ID 规则；
- 完整导出保留原始消息和结构化 checkpoint，面向用户的 transcript 导出不得把 checkpoint summary 冒充成用户消息。

当前 fork 实现会重新生成 entry ID，因此落地 C3 前必须先实现引用重映射或改为 session-scope 内保留 ID。仅在 cutoff 缺失时回退不够，因为长会话可能因此恢复完整 prompt 并在 fork 后第一轮直接超过 context window。

### 4.9 长历史的有界读取

append-only journal 会持续增长，不能继续假设 `get_session_entries`、remote history、fork 和 UI 首屏可以一次加载完整文件。C3/C4 必须同时提供：

- 新客户端显式携带 `offset`/`limit` 时使用 append-only 顺序分页；未携带分页字段的既有调用继续返回完整列表。响应同时返回 `hasMore`/`nextOffset`，checkpoint 与消息共用同一序列；
- remote UI 按页拉取并拼接完整历史；desktop 现有本地调用暂时保留一次性读取语义；
- checkpoint 和消息使用同一顺序游标，避免分页边界丢 marker；
- fork/export 可流式扫描全量 journal，不通过单个无界 RPC payload；
- 保留现有非分页调用的兼容响应，直到所有内置调用方完成迁移。

分页只能改变传输方式，不能改变用户能够查看、搜索、fork 或导出的历史内容。

## 5. 持久化与兼容性契约

### 5.1 JSONL

本方案会扩展 JSONL，但只允许加法兼容：

- session JSONL 沿用当前 `SessionEntry` 单行 envelope，新增 v2 compaction `content` schema；
- run-event JSONL 沿用当前 event envelope，新增结构化 compaction event 类型；
- 旧记录不批量迁移、不因读取改写；
- 新 loader 双读全部已发布格式，新 writer 只写新格式；
- 任意合法旧 session 在升级前后的可见消息、顺序、附件、reasoning、tool 语义、run 状态和继续运行能力一致。

### 5.2 SQLite

本方案不修改 desktop SQLite schema。消息历史和 run event 已由 agent JSONL 持有，checkpoint 也继续属于 agent journal/RPC projection。不得为了实现本方案重新引入 `messages`、`run_events` 或 checkpoint SQLite 表。若未来遥测需要独立存储，必须作为另一个有独立迁移与降级策略的设计处理。

### 5.3 零用户可感知数据 break

以下任一情况均视为阻断发布的 breaking change：

- 旧会话无法加载、无法继续、消息减少或顺序改变；
- reasoning、provider metadata、tool error、附件或 run 归属在未被 summary 覆盖的 recent tail 中丢失；
- v2 checkpoint 覆盖的原始 entry 从 journal、UI 历史或完整导出中消失；
- 旧压缩 marker 消失、重复，或变成用户消息气泡；
- fork/clone 后 checkpoint 引用失效、历史丢失或首轮 prompt 意外溢出；
- checkpoint 已 durable commit，但重启后 UI/Prompt Projection 不承认它；
- 新增长历史分页后，用户无法继续访问此前可访问的完整内容。

这里的 provider 保真边界必须明确：被 checkpoint 覆盖的旧消息，其 provider metadata 继续完整保留在 journal 和完整导出中，但不会全部重新送给模型；cutoff 之后的 recent tail 在 Prompt Projection 中必须逐字段保真。

## 6. 不采用的方案

### 6.1 只修正长度比较基准

例如改为比较 `result.len()` 和 `llm_msgs.len()`。它能修复当前误报，但仍然保留有损往返、重复压缩路径、字符串协议、整文件重写和多源状态，不作为正式方案。

### 6.2 仅检查 `last_compaction_result.is_some()`

这仍依赖共享副作用，并要求每个成功和失败分支正确同步多个状态。新增压缩入口后仍容易再次分叉，只适合临时止血。

### 6.3 显式 outcome，但继续把压缩结果写回完整历史

这一方案可以修复误报和状态分叉，但仍会丢失完整会话、引入 JSONL 重写风险，也无法清晰区分 transcript 和 prompt。可以作为迁移阶段，不能作为终态。

## 7. 分阶段迁移计划

### Phase C1：显式结果与统一入口

- 新增 `ContextPreparation`、`ContextError` 和 `CompactionTrigger`；
- 自动压缩和 provider-limit 压缩统一进入 `ContextManager`；
- 删除基于长度的压缩判断；
- 删除 `last_compaction_result`、`compaction_occurred`、`compaction_failed`；
- provider-limit 路径停止使用占位 token；
- 补齐当前误报的跨层回归测试。

此阶段允许暂时维持旧持久化格式，但接口必须按最终模型设计，避免下一阶段再次改调用方。

C1 仍使用旧持久化时，只能继续发兼容的 legacy `compaction_end`，不得宣称 checkpoint 已 committed。`compaction_committed` 必须等到 C3 存在可 `fsync`、可恢复的结构化 checkpoint 后启用。

### Phase C2：建立 AgentMessage 原生 Prompt Projection

- `ContextManager` 改为只接收带 journal provenance 的 `PromptContext`；
- run loop 删除 `ConvertToLLM -> ConvertFromLLM` 回写；
- `Message` 仅保留在确有需要的旧兼容或 provider 边界；
- 为 replay eligibility 和 `ProjectedMessage.source_entry_ids` 建立明确规则；
- 增加 provider metadata、tool error、附件和新增 content block 的保真测试。

### Phase C3：追加式检查点

- 在现有 `SessionEntry` envelope 的 `content` 中增加 v2 compaction checkpoint schema；
- 正常旧日志保留已有 ID；仅对确实缺失 ID 的极早期/导入记录生成确定性兼容 ID，且不因读取重写文件；
- prompt projection 读取最新有效 checkpoint 加其后的日志；
- 停止因压缩重写整个 JSONL；
- 双读旧伪用户消息、旧 compaction entry 和 v2 checkpoint，只在内存中规范化；
- 为 fork/clone 实现 ID 引用保持或 old-to-new 重映射；
- checkpoint 使用同步追加，`fsync` 成功才返回 committed；
- 禁止新代码继续生成字符串形式的压缩消息。

### Phase C4：UI 和遥测收口

- UI 改用 durable checkpoint projection，并用 live `compaction_committed` 做即时增量；
- marker 使用 checkpoint ID 去重；
- 历史/reconnect 从 durable checkpoint 恢复 marker，live event 仅做增量通知；
- 同时兼容旧 `compaction_end` 和旧字符串 marker；
- 展示真实 trigger 和 before/after token；
- 为长历史增加 entry-ID 游标分页和按需加载；
- 增加 checkpoint 创建、失败、恢复、压缩率和重复触发指标；
- 新代码停止写 `compaction_end`，但 reader 对已发布旧日志的兼容不得设置基于时间或版本的强制删除点。

## 8. 验证矩阵

### 8.1 消息投影

- reasoning-only assistant 被 prompt projection 排除，但保留在完整会话历史；
- 中断 reasoning 不产生任何压缩事件；
- 正常 reasoning + text assistant 保持可重放；
- dangling tool call 修复不被解释为压缩；
- 用户发送以 `[Context compaction:` 开头的真实消息时不会被当成内部事件。

### 8.2 数据保真

- OpenAI Responses item ID 和 encrypted reasoning 在完整 journal 中保留，cutoff 后的 recent tail 在 prompt 投影中完整保留；
- Anthropic thinking signature/redacted data 在完整 journal 中保留，cutoff 后的 recent tail 在 prompt 投影中完整保留；
- `ToolResult.is_error` 不变化；
- `AgentMessage.metadata`、附件和未知 content block 不被静默删除。

### 8.3 压缩状态机

- no-op preparation 只返回 `Unchanged`；
- 真压缩只生成一个 checkpoint 和一个 committed event；
- 自动、provider-limit、手动压缩使用相同 checkpoint schema；
- checkpoint 持久化失败时不发送 committed event；
- checkpoint append 成功但 `fsync` 失败时不发送 committed event；
- provider-limit 重试最多执行既定次数，不形成压缩重试循环。

### 8.4 持久化与恢复

- 压缩前后的完整 journal 内容保持不变，只追加 checkpoint；
- 重启后重建出的 `PromptContext` 与提交 checkpoint 后一致；
- 多个 checkpoint 只应用最新有效 checkpoint；
- cutoff entry 缺失或 checkpoint 损坏时安全回退并报告错误；
- 旧伪用户压缩消息、旧 compaction entry 和 v2 checkpoint 可以混合读取，且不会重复迁移；
- 写入 checkpoint 中途崩溃不会破坏此前 journal；
- checkpoint `fsync` 后、event 发出前崩溃，重启仍恢复相同 PromptContext 和单个 UI marker；
- fork/clone 在保留或重映射 ID 后仍能应用同一有效 checkpoint；
- fork 点位于 cutoff 前、范围内和 checkpoint 后的行为分别有回归测试；
- 超长 journal 通过分页/流式路径仍可完整查看、fork 和导出。

### 8.5 UI

- reconnect、event replay 和任务重开不会重复显示同一 checkpoint；
- 只有 checkpoint、没有对应 live event 时，重开仍显示一个 marker；
- 旧 `compaction_end` 与新 checkpoint 同时存在时只显示一个 marker；
- `compaction_started` 不创建永久 marker；
- failed/aborted compaction 不显示“已压缩”；
- tokens 为零或摘要为空的非法 committed event 被 projection 拒绝并记录诊断。

### 8.6 升级兼容

- 使用发布版本产生的旧 session JSONL fixture，升级后消息、顺序、附件、reasoning、tool 和 run footer 投影逐项相同；
- 使用旧 run-event JSONL fixture，升级后压缩 marker 和 run 内容逐项相同且不重复；
- 混合包含旧、新 checkpoint 的日志可继续运行，且下一次写入只追加 v2 entry；
- 读取、重开、继续旧会话不会改写其既有 JSONL 前缀；
- downgrade 不属于支持范围，但升级失败不得损坏旧文件，用户仍可用备份或旧二进制读取升级前前缀。

## 9. 完成标准

当以下条件全部满足时，架构迁移才算完成：

1. run loop 不再通过数组长度推断压缩；
2. prompt projection 的有损结果不再覆盖 session journal；
3. 自动和强制压缩只有一个实现入口和一个结果类型；
4. 压缩不再重写完整会话日志；
5. 新会话不再写入 `[Context compaction: ...]` 伪用户消息；
6. UI 永久标记只对应已持久化的 checkpoint；
7. provider metadata 和 tool result 语义通过压缩保真测试；
8. reasoning 中断、tool 修复和普通过滤不会触发压缩提示；
9. 旧会话、旧 run event、fork/clone 和完整导出通过零数据 break fixture；
10. checkpoint 的 durable commit、崩溃恢复和 UI 去重形成闭环；
11. 长历史通过分页或流式读取保持完整可访问；
12. desktop SQLite schema 无变化；
13. 已发布旧格式的 reader 兼容不依赖一次性批量迁移，也不设置会让历史数据突然失效的退出时间点。

## 10. 决策

采用“**不可变 Session Journal + AgentMessage 原生 Prompt Projection + 显式 ContextCheckpoint**”作为目标架构。

当前误报必须通过 C1 的显式 outcome 修复，但不接受仅调整长度比较作为最终实现。C1 到 C4 应按顺序落地，每阶段保持兼容和独立测试，最终删除旧 `Message` 往返、多套压缩状态和字符串压缩写入协议；对已经发布的旧 JSONL 和旧 run event 保持永久只读兼容。

任何阶段只要不能证明升级前后用户数据投影等价、原始 journal 不丢失且会话可继续，就不得发布该阶段。
