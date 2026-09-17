# 旧 A 模型摘要调用与 Prompt

**策略 C 不需要摘要模型，确定性 C 路径不发起任何摘要调用；但运行时默认为 C3（C 加一份模型撰写的交接摘要），因此默认路径每次压缩会发起一次摘要请求。** 本页记录该共享的提示词构造，以及仍保留的显式 legacy A API，不代表默认路径实际发送的内容。 当前机制见 [C 压缩](compaction.zh-CN.md)。

本页说明实际的摘要调用，不把普通回答／历史查询后的答题调用算作压缩调用。其中 `summary_prompt` 与 `call_summary_model_with_messages` **与 C3 默认路径共用**；`summarize_fold`、`serialize_message`、`call_summary_model_bounded` 仅属于保留的 legacy A 路径。权威实现为 [semantic.rs](../agent/src/compaction/semantic.rs) 与 [semantic/evidence.rs](../agent/src/compaction/semantic/evidence.rs)。

## 调用几次？

| 情况 | 摘要模型 API 请求数 |
|---|---:|
| 幂等命中、没有需要处理的内容或未达到自动触发条件 | 0 |
| 一块输入，一次成功 | 1 |
| 输入要拆成 K 块，均一次成功 | K |
| 暂态错误或不完整响应 | 每块首次请求外，最多两次重试 |
| 上下文／长度错误触发严格模式 | 最多再进行一轮更严格的分块折叠，块数可能改变 |

因此没有固定的“每次必然一次”或“总共最多三次”。流式响应里的大量 SSE chunk 不是大量模型请求。现有命令返回 ACK，不返回最终调用计数；不能从 ACK 推断只调用一次。

显式调用旧 semantic API 时使用传入的模型／provider，禁用工具。**C3 默认路径不同：它把活动对话按原角色作为真实消息发送，并复用 agent 自己的 system prompt（已有 checkpoint 时含历史召回指引）与工具定义——这正是它命中 provider 前缀缓存的前提**，见 [C 压缩](compaction.zh-CN.md)。摘要文字预算最多约 4096 estimated tokens；单次生成上限最多 8192，小窗口缩小且不超过模型上限。这不修改正常聊天的输出配置。

## 1. System prompt 原文

该常量是**旧 A 路径**（`call_summary_model_bounded`）的完整 system prompt，也是 C3 路径在调用方未传 prompt 时的**兜底**。生产始终会传：C3 请求携带会话自己的 system prompt（已有 checkpoint 时含历史召回指引）与会话自己的工具定义，下面这段文字只作为**追加在最后的指令**出现在其下。

以下是源码 `SUMMARY_SYSTEM_PROMPT` 的内容：

```text
You are a context summarization agent. Produce a structured handoff summary so another coding agent can continue the work. Do not continue the conversation or answer its questions. Output only the requested structure, using the conversation's primary language.
Evidence completeness: tool results may be partial excerpts. Describe only what the visible excerpt establishes; omitted content remains unknown. Never infer that the full result contains no relevant data, no errors, or only filler because its middle is omitted. Preserve this qualification and the history entry reference. A successful tool execution is not proof that all requested validation passed.
```

这不是正常聊天的完整 system prompt。旧 A 请求不会拷入正常聊天的项目规则、工具定义和历史召回指南；**C3 请求则会带上它们**，因为那正是会话已缓存的前缀（见 [C 压缩](compaction.zh-CN.md)）。

## 2. User message 的组装（旧 A）

旧 A 的每次摘要请求是一个专用 `ModelRequest`：上述 system prompt、一条 user 文本消息、空 tools 列表。各 provider adapter 再映射成自己的 API 格式。（C3 不是这样：它把活动对话按原角色作为真实消息发送、指令追加在最后，前缀才能命中缓存。）

user 文本按下列顺序构造（尖括号内的示例值不是实际用户历史）：

```text
[有旧摘要时：说明旧摘要覆盖更早的历史，保留仍有效的事实，新记录优先解决冲突]
<prior-summary>
<此前 checkpoint 摘要，或上一块折叠的输出>
</prior-summary>

<conversation>
<本次分块中的被覆盖历史，按下面的规则转成文本>
</conversation>

[提供 --instructions 时]
Additional user instructions for this compaction:
<用户的补充指令>

<固定输出模板及 Rules>

<S2 保留策略及本次摘要 token 目标>
```

上一块的摘要成为下一块的 accumulator；不是把多个摘要互不关联地拼起来。第一次没有旧摘要时不带 `prior-summary`。正常的最近保留尾部不作为本次被覆盖内容直接加入；覆盖范围内的保护原文仍可能作为摘要背景输入。

## 3. 历史文本包含什么？

每条有引用的记录前加 `[History entry <entry_id>]`。内容按块序列化：

| 内容块 | 摘要输入 |
|---|---|
| 用户／assistant／system 文本 | 角色标签及正文 |
| 工具调用 | 调用 ID、工具名、参数 JSON |
| 工具结果 | 关联调用 ID、result/error 标记及文字摘录 |
| Reasoning 文本 | `[Assistant reasoning]`，正常最多 2000 字符，严格模式最多 512 字符 |
| 图片 | 文本占位／非 data URL 引用，不发送内嵌图片二进制 |
| provider 隐藏元数据 | 不按原始对象整体序列化进摘要文本 |

**注意区分：S2 保护区不带 thinking，不代表摘要输入完全没有 reasoning 文本。** 当前序列化器会限长地加入历史 reasoning 文本；它不是完整回放隐藏 provider 状态。

工具结果正常保留头尾合计最多 2000 字符，严格模式最多 512 字符，另带省略说明。用户／assistant 正文和工具参数不在此阶段按这个字符数截断，过大的序列化输入由分块预算继续拆分。字符数不是 token 数。

## 4. 固定输出模板原文

```text
Output exactly this Markdown structure and keep every section:

## Objective
- [the user's unresolved objective, or (none)]

## Important Details
- [constraints, decisions and why, important facts, or (none)]

## Work State
### Completed
- [finished and verified work, or (none)]

### Active
- [current or partially completed work, or (none)]

### Blocked
- [blockers, failed commands, and unknowns, or (none)]

## Next Move
1. [immediate concrete action, or wait for the user's next instruction]

## Relevant Files
- [exact path and why it matters, or (none)]

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact paths, symbols, commands, error strings, URLs, and identifiers when known.
- Reflect the current state: completed requests belong in Completed; only unresolved work belongs in Objective, Active, and Next Move; remove resolved blockers.
- Do not mention compaction or the summary process.
```

模板之后还附加以下策略；`N` 由本次预算计算，最多约 4096：

```text
S2 retention: original user directives and selected assistant text are preserved separately. Older assistant outputs may be summarized to fit; carry forward their important facts. Prioritize tool evidence, exact symbols/values, corrections and verification boundaries. Keep canonical headings exactly as shown; write the body in the conversation language. Keep the summary within N estimated tokens.
```

## 维护要求

修改 prompt、摘录规则或预算后，应同步检查幂等策略指纹、结构校验、分块预算和此文档。不要把新 prompt 的同一输入错误地复用成旧摘要。调用次数／费用应按实际 API 请求记录，不按 chunk 数或 checkpoint 数猜测；失败请求和重试也可能收费。
