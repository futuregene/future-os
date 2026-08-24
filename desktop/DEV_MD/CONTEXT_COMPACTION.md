# 上下文压缩架构与语义压缩下一阶段开发计划

状态：**v2 数据底座已落地；语义压缩阶段设计已确认，待开发**（2026-08-24）

基线提交：`8fd6804e Implement durable context compaction checkpoints`

本文是 FutureOS 上下文压缩的当前权威设计。它分为两部分：

1. 已落地且后续不得破坏的 v2 数据与兼容性底座；
2. 下一阶段要实现的本地语义压缩、模型切换检测和压缩请求容错。

本文不再把已经完成的 Journal、Prompt Projection、ContextCheckpoint、RPC/UI marker 和 fork 兼容工作列为未来迁移项。

## 1. 不可放宽的交付约束

任何阶段都必须做到：**没有用户可感知的数据 break**。

升级后，既有会话、消息、reasoning、tool call/result、附件、run 状态、压缩标记、fork/clone 和完整导出必须继续可读。不得因为压缩算法或 schema 演进而：

- 删除、覆盖、重新归属或重排既有消息；
- 隐藏 reasoning、tool error、附件或 provider metadata；
- 把内部摘要伪装成真实用户消息；
- 让旧会话无法继续运行；
- 让旧压缩 marker 消失或重复；
- 让 checkpoint 覆盖的原始条目从 UI 历史或完整导出中消失；
- 因 checkpoint 写入失败而切换到一个无法在重启后恢复的内存上下文。

agent 和 desktop 同版本发布，不要求支持新旧进程在线混用；但新版本必须永久只读兼容磁盘上所有已发布的 session JSONL 和 run-event JSONL。读取旧日志不得触发隐式批量迁移或改写。

## 2. 已确认的产品决策

### 2.1 本阶段不做

以下能力明确排除在下一阶段范围之外：

1. **独立 compaction model/agent**：摘要使用当前会话模型；模型切换场景只允许在旧模型与用户选定的新模型之间重试，不引入第三个专用模型。
2. **TokenBudget 无摘要开新窗口**：不允许在没有语义摘要的情况下清空模型可见历史并开始新窗口。
3. **Provider-native remote compaction**：不实现 `/responses/compact`、`compaction_trigger` 或 provider 私有压缩对象。
4. **Remote → local fallback**：因为没有 remote compaction 路径，所以不设计这条 fallback 链。
5. **把 `replacement_history` 整段写入 checkpoint**：checkpoint 继续只保存摘要、覆盖范围和元数据；模型上下文由 append-only journal 派生，避免重复存储完整历史。

### 2.2 本阶段要做

下一阶段采用“OpenCode 风格的本地结构化摘要 + FutureOS 现有 append-only checkpoint 底座”，并吸收 Codex 中不依赖远端 provider 的生命周期与容错设计：

- previous summary 的显式合并；
- 按完整 user turn 选择 retained tail；
- assistant tool call 与 tool result 的原子边界；
- 摘要输入中的 tool output/media 有界序列化；
- `PreTurn`、`MidTurn`、`Standalone` 三种压缩阶段；
- 模型切换和 context-window downshift 检测；
- 压缩请求自身的 token 预算、context-limit 重试和瞬时错误重试；
- 语义摘要失败后的确定性紧急摘要；
- checkpoint durable commit 成功后才切换 prompt projection；
- 完整的 telemetry、失败诊断和兼容 fixture。

## 3. 已落地的 v2 基线

### 3.1 不可变 Session Journal

`SessionEntry` 是唯一持久化事实层：

- user、assistant、reasoning、tool call/result 和 run marker 保持 append-only；
- 压缩只追加 `type: "compaction"` 的 v2 checkpoint entry；
- 原始消息不会因为模型上下文优化而删除或重写；
- UI、审计、fork 和完整导出仍可访问 checkpoint 覆盖前的所有原始数据。

### 3.2 AgentMessage 原生 Prompt Projection

run loop 不再通过 `ConvertToLLM -> ConvertFromLLM` 的有损往返覆盖会话真相。每次模型请求从完整 journal 和最新有效 checkpoint 派生一次 `PromptContext`：

```rust
struct PromptContext {
    messages: Vec<ProjectedMessage>,
    usage: ContextUsage,
}

struct ProjectedMessage {
    message: AgentMessage,
    source_entry_ids: Vec<String>,
}
```

checkpoint 之前的消息只从模型 prompt 中被 summary 替代，仍完整存在于 journal、UI 和导出中。cutoff 之后的 recent tail 必须逐字段保留 provider metadata、tool error、附件和未知 content block。

### 3.3 显式 ContextPreparation

压缩是否发生由强类型结果表达，不再通过数组长度、共享原子变量或字符串前缀推断：

```rust
enum ContextPreparation {
    Unchanged { prompt: PromptContext },
    Compacted {
        prompt: PromptContext,
        checkpoint: Box<ContextCheckpoint>,
    },
}
```

自动压缩、provider context-limit 恢复和手动压缩已经进入同一个 `ContextManager`。

### 3.4 v2 ContextCheckpoint

新 writer 复用既有 `SessionEntry` envelope，只在 `content` 内写结构化 v2 checkpoint：

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

loader 已同时接受：

1. 历史 `[Context compaction: ...]` 伪用户消息；
2. 旧 `type: "compaction"`、`content: {summary,tokens_in,tokens_out}`；
3. v2 checkpoint。

损坏、引用缺失或范围非法的新 checkpoint 会回退到此前最近的有效 checkpoint；不存在有效 checkpoint 时，从完整 journal 安全重建。

### 3.5 Durable commit 与 UI marker

每次真正进入压缩流程时生成唯一 `operation_id`，并通过 run-event journal 发出完整生命周期：

1. `compaction_started`：摘要请求开始前发出，UI 显示“正在压缩”；
2. `compaction_committed`：checkpoint durable commit 成功后发出，UI 将同一 marker 原位更新为成功；
3. `compaction_failed`：摘要、边界规划或持久化任一步失败时发出，UI 将同一 marker 原位更新为失败并保留错误详情。

三种事件都携带同一个 `operation_id`；`trigger` 和 `phase` 在 started/failed 中明确携带，committed 从 checkpoint 携带。未达到阈值、无需压缩的 `Unchanged` 路径不发 started，避免产生虚假状态。

checkpoint 与消息 append 共用同一个 FIFO 持久化队列。只有此前 append、checkpoint append、flush 和 `fsync` 全部成功后，才允许：

1. 激活新的内存 checkpoint；
2. 发出 `compaction_committed`；
3. 将 UI marker 更新为成功。

重启、reconnect 和历史加载从 checkpoint 与 run-event journal 恢复状态；live event 只承担低延迟通知。旧 `compaction_end` 继续兼容读取，并与 checkpoint marker 去重。进程在 committed 之前被强制终止时，不激活 checkpoint、不改变 prompt projection；恢复时未闭合的 running marker 必须按中断处理，不能永久显示“正在压缩”。

### 3.6 Fork、分页与存储边界

- fork 会重建 old-to-new entry ID map，并重映射 checkpoint 的覆盖范围；范围不完整的 checkpoint 不复制；
- session entry RPC 支持兼容的 offset/limit 顺序分页；未传分页参数的旧调用仍保留原行为；
- checkpoint 和消息共享同一日志序列，分页不改变历史内容；
- desktop SQLite schema 没有变化，后续阶段也不得为上下文压缩新增 SQLite 表。

## 4. 当前基线的剩余问题

v2 数据底座解决了误报、历史重写和兼容问题，但当前摘要器仍是确定性文件操作摘要：

```text
Previous conversation summarized.
Files read: ...
Modified: ...
```

它不能稳定保存：

- 用户最终目标和明确约束；
- 已做出的决策及原因；
- 已完成、进行中和阻塞的工作；
- 关键命令、错误、验证结果和标识符；
- 非文件类 tool result；
- 上一次 checkpoint 中仍然有效的语义。

因此，当前实现满足“原始数据不丢失”，但不满足“多次压缩后模型仍能可靠继续任务”。下一阶段只改变摘要生成与触发策略，不推翻 v2 Journal/Checkpoint/RPC/UI 底座。

## 5. 下一阶段目标架构

```text
Append-only Session Journal
        |
        v
Prompt Projection
  latest valid checkpoint summary + cutoff 后的 replayable tail
        |
        v
Compaction Trigger Policy
  automatic / provider-limit / manual / model-context-downshift
        |
        v
Turn-aware Tail Selector
  完整 user turn + 原子 tool call/result
        |
        +-------------------------------+
        |                               |
        v                               v
Semantic Summary Input              Recent Tail
  previous summary                   保持 AgentMessage 原样
  covered head
  bounded tool/media serialization
        |
        v
Semantic Summarizer（当前会话模型）
        |
        +-- 请求容错与分块 fold
        |
        +-- 失败 -> Deterministic Emergency Summarizer
        |
        v
ContextCheckpoint v2
        |
        v
append + flush + fsync
        |
        v
activate projection + emit committed event
```

## 6. 压缩生命周期

新增显式阶段概念：

```rust
enum CompactionPhase {
    PreTurn,
    MidTurn,
    Standalone,
}
```

`phase` 是 trigger 之外的正交属性；建议作为 v2 checkpoint `content` 和事件 payload 的可选加法字段。读取旧 checkpoint 时允许缺失，不影响兼容。

| Phase | 发生位置 | 典型 trigger | 继续方式 |
| --- | --- | --- | --- |
| `PreTurn` | 新一轮正常模型请求之前 | `Automatic`、`ModelContextDownshift` | checkpoint 提交后正常开始新 turn |
| `MidTurn` | assistant/tool loop 仍需 follow-up 时 | `ProviderContextLimit`、阈值溢出 | 保留当前 tool 边界，在同一个 run 内直接重试 |
| `Standalone` | 用户主动 compact | `Manual` | 生成并提交 checkpoint，不伪造 continue user message |

不得通过 synthetic `Continue...` 用户消息恢复执行。MidTurn 压缩后由 run loop 直接继续，避免改变 transcript 语义或重复执行工具。

## 7. Trigger Policy

### 7.1 Automatic

保持当前基本判据：

```text
tokens_before = max(provider reported input_tokens, local estimate)
needs_compaction = tokens_before > context_window - reserve_tokens
```

只统计下一次 prompt 的输入占用；completion、reasoning output 和缓存统计不得重复混入 prompt 大小。默认 reserve 继续使用当前模型窗口策略，具体值由模型 registry/config 决定。

### 7.2 ProviderContextLimit

provider 返回 context-length/body-size 错误时，强制进入相同 `ContextManager`，不伪造 token 数值。一次 checkpoint durable commit 后才消耗模型请求 retry；同一投影、同一错误不得形成无限压缩循环。

### 7.3 Manual

用户指令作为摘要 prompt 的附加约束，而不是直接拼接到 summary 文本。例如：

```text
特别保留 JSONL 兼容性、SQLite 边界和未完成测试。
```

手动压缩使用与自动压缩相同的结构化摘要、tail selection、持久化和兼容路径。

Desktop 在已有 Agent session 的对话输入框 `/` 菜单顶部提供「压缩 / Compact」上下文工具。选择后直接调用 standalone `compact` RPC，不生成 `/compact` 用户消息、普通 Agent 回复或新的 Run；摘要请求是该操作唯一的 LLM 通信。`Manual` 明确跳过自动 context-window 阈值，但仍必须找到有效 turn/tool 边界。成功 checkpoint 以 `trigger: "manual"` 追加到 JSONL，Desktop/Mobile 以“你手动压缩了此对话的上下文”分割线显示该用户选择；失败也保留 manual 标识。菜单同时命中上下文工具与 Skill 时工具在上、Skill 在下，并只在混合结果的 Skill 前显示「技能 / Skills」分隔文案；单一类别结果不显示分隔文案。中英文名称和描述都参与搜索。

### 7.4 ModelContextDownshift

建议增加 trigger：

```rust
CompactionTrigger::ModelContextDownshift
```

模型设置变更提交前，使用新模型的 context window、reserve 和输入能力重新评估当前 `PromptContext`：

```text
current_prompt_tokens > new_context_window - new_reserve_tokens
```

仅仅切换模型但新模型能容纳当前 prompt 时，不强制压缩。FutureOS 使用普通文本 summary，不需要引入 Codex 的 provider compaction compatibility hash。

## 8. Turn-aware Tail Selection

### 8.1 Turn 定义

一个 retained turn 从真实 user message 开始，到下一条真实 user message 之前结束。内部 checkpoint summary、UI-only entry 和 run marker 不应被误判为新 user turn。

选择过程从最新 turn 向前累计，直到达到 `keep_recent_tokens`。优先保留完整 turn；只有单个 turn 本身超过预算时，才允许在 turn 内寻找安全边界。

### 8.2 Tool 原子边界

以下组合不可被 checkpoint cutoff 拆开：

- assistant tool call 与对应 tool result；
- 同一 assistant message 内的并行 tool calls 与其已完成 results；
- provider 要求成对重放的 reasoning/tool metadata。

如果预算边界落在 tool result 上，向前回退到发起该 call 的 assistant message。合成的 dangling-tool 修复项没有独立 journal ID，不能成为 checkpoint cutoff。

### 8.3 Recent tail 保真

recent tail 不执行文本摘要或旧类型往返，继续保留完整 `AgentMessage`：

- provider item ID、encrypted reasoning；
- Anthropic thinking signature/redacted thinking；
- `ToolResult.is_error`；
- 附件、未知 content block 和 message metadata。

摘要输入可以有损序列化，但 recent tail 不可以。

## 9. Semantic Summary

### 9.1 固定输出结构

摘要使用当前会话模型生成，并要求输出固定 Markdown：

```markdown
## Objective
- 用户要完成什么

## Important Details
- 用户约束、偏好、关键事实、设计决定及原因

## Work State
### Completed
- 已完成和已验证内容

### Active
- 正在进行和部分完成内容

### Blocked
- 阻塞、失败命令和未知项

## Next Move
1. 下一步具体动作

## Relevant Files
- 精确路径、符号及其作用
```

摘要必须与会话主要语言一致，保留精确路径、命令、错误字符串、URL、ID 和用户明确措辞，不回答旧 conversation 中的问题，也不继续执行任务。

### 9.2 Previous summary 合并

第二次及后续压缩必须显式向模型提供：

```text
<prior-summary>...</prior-summary>
<conversation>checkpoint 后新产生、且本次将被覆盖的 head...</conversation>
```

prompt 必须说明：

- 新 summary 会完全替代 prior summary；未带入的信息将从模型可见上下文丢失；
- prior summary 中仍有效的目标、约束、决定和并行工作必须保留；
- conversation 比 prior summary 更新，冲突时以 conversation 为准；
- 已解决 blocker 和已完成 active work 要移动到正确状态；
- `Objective` 和 `Next Move` 必须反映最新状态。

previous summary 本身不得被普通 token trimming 静默丢弃。

### 9.3 摘要输入序列化

送给摘要模型的是独立派生视图，不是原始 JSONL 重写。首轮序列化规则：

- user text 原样保留；附件转换为 mime、文件名和稳定引用描述，不嵌入原始二进制；
- assistant text 保留；reasoning 只保留有助于解释决定的有界文本；
- tool call 保留工具名和规范化参数；
- tool result 默认最多保留 2,000 字符，并标出截断；
- tool error 保留错误类型和有界错误文本；
- 已被 previous checkpoint 覆盖的原始消息不重复序列化，只使用 prior summary。

所有截断只作用于摘要请求输入和模型 prompt，不写回 journal，不影响 UI 或完整导出。

### 9.4 模型选择

不新增独立 compaction model/agent：

- 正常 Automatic、ProviderContextLimit 和 Manual 使用当前会话模型；
- ModelContextDownshift 在模型切换真正生效前，优先使用旧模型摘要旧历史；
- 旧模型不可用或发生允许重试的模型相关错误时，允许用用户已选择的新模型重试；
- 不搜索或调用第三个模型，不把摘要工作路由到隐藏 agent。

## 10. 压缩请求容错

压缩请求本身必须在发送前建立独立预算：

```text
summary_input_budget
  = summarizer_context_window
  - system_and_summary_prompt_tokens
  - summary_output_reserve
  - safety_margin
```

不得假设“正常会话能进入压缩”就意味着“完整旧历史 + 摘要 prompt”一定能被同一模型接受。

### 10.1 发送前有界化

按以下顺序缩减摘要输入，直到进入预算：

1. tool output 截断到 2,000 字符；
2. 进一步只保留 tool 名、参数摘要、成功/错误状态和关键尾部；
3. reasoning 缩减为有界文本；
4. 大附件只保留描述；
5. 按完整 turn 将 covered head 划分成多个 chunk。

不得为了让摘要请求通过而裁剪 recent tail；recent tail 根本不参与摘要模型的 conversation 输入。

### 10.2 分块 fold

当 covered head 无法在一次请求内处理时，按旧到新顺序分块：

```text
summary_0 = previous_summary 或空
summary_1 = summarize(summary_0 + chunk_1)
summary_2 = summarize(summary_1 + chunk_2)
...
final_summary = summarize(summary_n + 最后 chunk)
```

每个 chunk 必须优先落在完整 turn/tool 边界。每一步都使用同一结构化模板和“旧 summary 会被替代”的合并规则。只有 final summary 会写入 checkpoint；中间 summary 不写 journal、不发 UI marker。

### 10.3 Context-limit retry

即使本地估算认为请求可容纳，provider 仍可能返回 context limit。发生后允许执行一次更严格的重新规划：

- 降低单 chunk 预算和 tool/reasoning 上限；
- 重新按完整 turn 生成 chunk；
- 重新发送当前 fold step；
- 不重复已经产生外部副作用的普通 agent tool call，因为摘要请求不暴露 tools。

同一个 fold step 最多执行约定次数的 context-limit retry，超过后进入确定性紧急摘要，不能无限删除最旧消息并循环。

### 10.4 瞬时错误 retry

对 timeout、连接中断、server overload、明确 retryable 的 5xx 和 rate-limit 使用有上限的指数退避；认证失败、非法请求、取消和非 retryable 错误不重试。用户中断必须立即终止摘要请求，不得在后台继续提交 checkpoint。

### 10.5 确定性紧急摘要

语义摘要在重试耗尽后，允许使用本地确定性摘要保证 provider-limit 恢复和手动操作有明确结果，但该摘要必须比当前文件列表实现更完整：

- 原样带入 previous summary；
- 保留最近用户目标和明确指令；
- 提取 assistant 最近完成内容；
- 提取 tool 名、参数摘要、成功/失败和关键错误；
- 提取 read/modified files；
- 记录当前 run/turn 是否仍需 tool follow-up；
- 带入手动 compaction instructions；
- 给出 retained tail 起点和下一步未知项。

用 `algorithm_version` 区分质量：

```text
semantic-v1
deterministic-emergency-v1
```

紧急摘要同样必须通过 checkpoint 范围校验和 durable commit。不得写一个空 summary，也不得把语义摘要失败误报成 `semantic-v1` 成功。

## 11. Checkpoint 提交流程

所有策略最终仍生成现有 `ContextCheckpoint`，不写 `replacement_history`：

```text
1. 从 ProjectedMessage.source_entry_ids 计算连续 covered range
2. 验证 covered_from、cutoff 和 tool 边界
3. 生成 operation_id 并发出 compaction_started
4. 生成 final summary
5. 构建 summary + recent tail 的候选 PromptContext
6. 计算 tokens_before/tokens_after
7. append checkpoint 到 session JSONL
8. flush + fsync
9. 激活 active_checkpoint
10. 发出携带同一 operation_id 的 compaction_committed
11. 用新 PromptContext 调用或重试模型
```

步骤 4 至 8 任一步失败时：

- 不修改 active checkpoint；
- 不发送 committed event；
- 发送携带同一 operation_id、trigger、phase 和错误详情的 `compaction_failed`；
- UI 将 running marker 原位更新为失败；
- 不使用只存在内存中的 summary 调用普通模型；
- 返回结构化 persistence error。

## 12. JSONL、RPC 与 SQLite 兼容

### 12.1 JSONL

下一阶段继续写 v2 `SessionEntry` envelope，不引入新的顶层行类型。允许在 v2 `content` 中加法增加可选字段，例如：

```json
{
  "phase": "pre_turn"
}
```

旧 v2 checkpoint 缺少新字段时必须有稳定默认值。`trigger` 新增 `model_context_downshift` 时，desktop/mobile/RPC 必须与 agent 同提交更新；历史 reader 继续接受既有三个 trigger。

摘要算法升级不得改写任何旧 checkpoint。新 writer 通过 `algorithm_version` 表达算法，不通过批量迁移替换历史摘要。

### 12.2 Run-event 与 RPC

新增 `compaction_started`；`compaction_committed` 与 `compaction_failed` 加法携带 `operation_id`/`phase`，不删除或改名任何既有字段。三态事件进入现有 run-event journal 和 RPC `StreamEvent`，不新增 session JSONL 顶层行类型。成功后的 prompt projection 仍只以 checkpoint journal entry 为事实来源；started/failed 只描述操作状态，不得改变历史上下文。旧 `compaction_end`、缺少 `operation_id` 的旧 committed 事件和旧 marker 去重规则保持兼容。

### 12.3 SQLite

下一阶段不修改 desktop SQLite schema，不新增 message、run-event、summary 或 checkpoint 表。模型切换状态、摘要中间结果和 retry 状态均属于单次运行内存；只有 final checkpoint 进入 agent JSONL。

## 13. 下一阶段开发计划

### Phase S1：语义摘要核心

目标：用结构化语义摘要替换当前文件列表摘要，但暂不改变 trigger。

- 定义固定 summary template 和 previous-summary merge prompt；
- 建立 summary-only 的 `AgentMessage` 序列化器；
- 实现 turn-aware head/tail selector；
- 实现 tool call/result 原子边界；
- 当前会话模型执行无 tools 的摘要请求；
- final checkpoint 写 `algorithm_version = "semantic-v1"`；
- Manual instructions 进入 summary prompt；
- 保持现有 v2 checkpoint、RPC 和 UI schema 兼容。

建议主要改动区域：

- `agent/src/compaction/`：选择、序列化、prompt、summary orchestration；
- `agent/src/agent/run_loop.rs`：异步 `prepare` 接入；
- `agent/src/rpc/session.rs`：手动压缩异步化与结果映射；
- `agent/src/session/checkpoint.rs`：只补充兼容的算法/可选字段读取测试。

### Phase S2：压缩请求预算与容错

目标：保证摘要请求本身在长会话和 provider 误差下有界、可恢复。

- 计算独立 summary input/output budget；
- 实现 2K tool output 首轮截断和严格模式；
- 实现按 turn/tool 边界的 chunked summary fold；
- 实现一次 context-limit 重新规划；
- 接入有上限的 retryable transport backoff；
- 用户取消贯穿所有 fold step；
- 实现 `deterministic-emergency-v1`；
- 中间 summary 不持久化、不发事件。

### Phase S3：生命周期与模型切换

目标：统一 PreTurn/MidTurn/Standalone，并在模型窗口缩小时提前压缩。

- 增加 `CompactionPhase`；
- 增加 `ModelContextDownshift` trigger；
- 模型设置提交前评估新 context window；
- downshift 时优先旧模型摘要，允许用户选定的新模型重试；
- MidTurn 压缩后继续同一个 run，不生成 synthetic user message；
- provider-limit 每次 retry 绑定唯一 checkpoint，阻止循环压缩；
- phase/trigger 加法贯通 agent、RPC、desktop、mobile 和 thread projection。
- started/committed/failed 使用 operation_id 关联，Desktop/Mobile 原位展示 running/completed/failed 分割线；
- agent/process 中断时不得遗留永久 running 状态，且 committed 前不得改变 active checkpoint。

### Phase S4：兼容、可观察性与发布收口

目标：证明语义升级不会造成数据或行为 break。

- 增加 summary input/output token、chunk 数、retry 原因、压缩率和算法版本指标；
- 记录结构化失败阶段，不记录敏感 summary 正文；
- 使用已发布旧 session/run-event fixture 做升级测试；
- 验证 semantic 和 emergency checkpoint 混合链；
- 验证 fork、分页、reconnect、导出和 marker 去重；
- 验证模型切换前后当前 run、model 字段和 context window 一致；
- 完成全量 agent、RPC、desktop、mobile 和 projection 测试。

S1–S4 可以按独立提交交付，但只有 S1 与 S2 同时完成后才允许默认启用语义压缩；否则超长摘要请求可能让自动压缩从“低质量但可用”退化成“直接失败”。

## 14. 验证矩阵

### 14.1 摘要正确性

- 首次压缩生成完整固定 Markdown 结构；
- 第二次压缩保留 prior summary 中仍有效的目标、约束和决定；
- 新 conversation 与 prior summary 冲突时使用新事实；
- completed/active/blocked 随进度正确迁移；
- 精确路径、命令、错误字符串、URL 和 ID 不被无理由改写；
- summary 为空、只有模板或缺少必要结构时拒绝 semantic checkpoint。

### 14.2 Tail 与 provider 保真

- tail 默认从完整 user turn 开始；
- cutoff 不拆分 assistant tool call/result；
- 单个超大 turn 能找到安全内部边界，找不到时返回明确错误或重新规划；
- recent tail 的 Responses item ID、encrypted reasoning、thinking signature、tool error、附件和 metadata 逐字段一致；
- 摘要输入截断不修改 journal/UI/export 中的原 tool output。

### 14.3 请求容错

- 估算可容纳但 provider 返回 context limit 时执行严格模式重试；
- 超长 covered head 通过多个完整 turn chunk fold；
- 每个 fold step 都合并此前 accumulator summary；
- 中间步骤失败时不写半成品 checkpoint；
- retryable 网络错误按上限退避；
- authentication、invalid request 和用户取消不错误重试；
- 重试耗尽后写 `deterministic-emergency-v1`，而不是伪造 semantic 成功；
- emergency summary 也不能为空，且必须带入 prior summary。

### 14.4 生命周期与模型切换

- PreTurn 压缩后正常开始新 turn；
- MidTurn 压缩后不重复执行已完成工具；
- Standalone 手动压缩不生成 synthetic user message；
- 从大窗口模型切到小窗口模型、且 prompt 超阈值时先压缩后切换；
- 新模型可以容纳当前 prompt 时不产生无意义 checkpoint；
- 旧模型失败、用户选择的新模型成功时只提交一个 checkpoint；
- 两个模型都失败时进入确定性紧急摘要或返回明确错误，不留下部分状态。

### 14.5 持久化与零数据 break

- 压缩前后的 journal 前缀逐字节不变，只追加 checkpoint；
- checkpoint commit/fsync 失败时 active projection 不改变；
- 重启后 PromptContext 与 commit 后一致；
- semantic 与 emergency checkpoint 多次交替时只应用最新有效 checkpoint；
- 旧字符串 marker、旧 compaction entry 和旧 v2 checkpoint 可混合读取；
- 旧会话继续运行不会改写既有 JSONL；
- fork 后 checkpoint 范围仍有效；
- UI/reconnect/event replay 不重复 marker；
- started → committed 与 started → failed 都只显示一个原位更新的 marker；
- 强制退出后未 committed 的操作不改变 checkpoint，恢复 UI 不永久停留在 running；
- 完整导出继续包含所有原始消息与 tool output；
- desktop SQLite schema snapshot 完全不变。

## 15. 完成标准

下一阶段只有在以下条件全部满足后才算完成：

1. 默认压缩摘要是结构化 `semantic-v1`，不是文件列表；
2. 多次压缩显式合并 previous summary；
3. retained tail 按 turn 和 tool 原子边界选择；
4. 摘要请求有独立 token budget、分块 fold 和有界 retry；
5. 语义摘要失败有可识别的 `deterministic-emergency-v1`，不写空 checkpoint；
6. PreTurn、MidTurn 和 Standalone 行为有回归测试；
7. context-window downshift 在切换模型前完成必要压缩；
8. 不存在独立 compaction model/agent、provider-native remote compaction 或无摘要新窗口；
9. checkpoint 不保存整段 `replacement_history`；
10. journal 仍 append-only，checkpoint durable commit 后才激活；
11. 已发布旧 JSONL/run-event fixture、fork、分页、UI marker 和完整导出通过零数据 break 测试；
12. desktop SQLite schema 无变化。

## 16. 最终决策

FutureOS 下一阶段采用：

> **本地结构化语义摘要 + previous-summary fold + turn-aware recent tail + 有界请求容错 + append-only ContextCheckpoint**

OpenCode 的摘要协议和 tail selection 是主要算法参考；Codex 只借鉴与 provider-native compaction 无关的生命周期、模型 downshift 检测和请求容错。FutureOS 继续保留自身已经完成的不可变 journal、稳定 entry provenance、durable checkpoint、fork 引用重映射和 UI/RPC 兼容底座。

任何实现如果要求删除原始消息、重写历史 JSONL、写入整段 replacement history、在没有摘要时清空模型上下文，或在 checkpoint 未 durable commit 前切换 prompt，都不符合本设计。
