# 回复结束与异常提示

模型调用结束不等于任务失败；没有正文也不等于网络断开。结束语义由协议适配器识别，Agent 决定任务状态，并将异常原因写入 `run_terminal.error` 和 `agent_end`。Desktop 使用同一原因生成分割线，历史与实时结果一致。

## 协议边界

- Chat Completions：模型 `finish_reason` 和 SSE `[DONE]` 分别处理；收到模型结束原因后允许继续读取用量。只有 `[DONE]` 或没有模型终止事件的 EOF 无法确认回复完成。
- Responses：以 response 终态为准，并先合并最终 output 快照；不能仅凭 delta 缺失断言没有输出。
- Anthropic：以 `message_stop` 和 stop reason 为准；`tool_use` 继续工具流程，`refusal` 不视为网络失败。
- `pause_turn` 保留独立分类。当前适配器不支持回放服务端工具的 opaque payload，因此不自动重发；向用户说明模型暂停及软件的恢复限制。
- 未知结束原因保留原始诊断，表示未确认，不擅自归因为模型故障或网络故障。

## 展示

| 情况 | 展示 |
| --- | --- |
| 正常完成、有正文或工具结果 | 正常消息 |
| 正常完成、无正文和工具结果 | 中性分割线：模型已结束生成，未提供最终回复 |
| 用户取消 | 原有手动停止分割线 |
| 上下文压缩 | 原有压缩进度／完成／失败分割线 |
| 单次输出上限（续写预算用尽） | 输出上限分割线；不混同上下文压缩 |
| 单次输出上限但仍在续写 | 无分割线；`output_stream_continuing` 不作说明文字，回复继续增长 |
| 明确连接读取错误 | 模型服务连接中断 |
| 请求超时 | 模型服务响应超时 |
| 缺少或未知模型终态 | 回复意外结束，原因暂未确认 |
| 上游内容限制、取消、暂停 | 分别展示上游声明的原因 |
| 软件处理／保存失败 | 软件处理或保存回复失败 |

已生成的文本、思考和工具记录始终保留。说明不作为模型正文写入。关闭思考显示不影响分割线。工具回合结束不会仅因没有文本报错。正常空回复不修改为失败，不自动重复请求。

## run 内自动续写（输出上限）

撞到单次输出上限时，run **不再直接把控制权交回用户**，而是在同一个 run 里继续写。这样交互会话不必点「继续」，无人值守的 goal worker 也不必靠「重试」去重做。

### 触发与边界

只有 `detected_by == "finish_length"` 会续写。上游明确的其它终止（内容限制、provider 取消、暂停、超时、流错误）都不是「额度用尽」，重发同一请求只会换回同一个拒绝，因此照旧立即以 `incomplete` 结束。用户中断优先级最高，任何情况下都不续写。

预算 `MAX_OUTPUT_LIMIT_CONTINUATIONS = 2`（初始请求之外最多两次续写），与断线重试预算（`MAX_STREAM_RETRIES = 5`）互相独立，避免一条抖动的链路和一次长回答吃掉彼此的额度。它沿用 `max_retries > 0` 这个开关：`set_auto_retry(false)` 与 fusion（显式置 0）因此不受影响，那些调用方要的是「一步一个请求」。

### 两种截断，两种续写指令

- **本轮写出了可见正文**：部分是前缀，续写指令是「从断点接着写，不要重来、不要重复已发出内容、不要重跑已执行过的工具」。
- **本轮正文 0 字（额度全被思考吃掉）**：没有可见前缀可续，而且 reasoning-only 的 assistant 消息在 `convert_to_llm` 里会被丢弃，上游看不到它。此时续写指令改为「停止推理，直接给出完整答案」。同时**降一档思考档位**——重复同一个请求必然再撞同一面墙，这是实测过的形态（三轮 32 768 token 全是 reasoning、正文 0 字）。降档只作用于本次 run 的 provider 私有快照（`snapshot()`），不写会话设置，用户下一条消息仍用他自己选的档位。

降档沿固定刻度走一级（`high → medium → low → off`），并经过模型自己声明的 `thinkingLevels` 过滤——模型没声明的档位不会发上线。`default` 隐藏了真实深度、无法做相对步进，此时先试 `low`，再退到该模型声明的最浅档；已在 `off` 则不再降。

### 事件与恢复语义

续写发出 `output_stream_continuing`（`{type, attempt, maxContinuations, thinkingLevel?}`），与 `stream_retry` 的区别是：没有重连、回答仍在增长。`thinkingLevel` 只在降档时出现。客户端见到它应清除 `reconnecting` 横幅。

被截断轮已经 push 并持久化的 assistant 消息**不重建**：同一个 `stream_entry_id` 再写第二份载荷会被 transcript store 以 `conflicting entry identity` 拒绝。续写路径只补上未执行工具调用的占位结果，然后复用既有的 `retry_text` 机制把已生成正文带进下一次请求。

预算耗尽仍截断时，回落到原有语义：置 `stream_incomplete`、写 `StreamTruncation`、`run_terminal.error = [OUTPUT_LIMIT] …`，随后交由「goal worker 的 incomplete 重试 ≠ 续写」那一节的路径处理。自动续写是最终失败**之前**的一层，不替换 `incomplete`。

回归覆盖（`agent/src/agent/run_loop.rs` 测试）：`spent_output_budget_continues_the_same_run`、`reasoning_only_cut_steps_thinking_down_before_continuing`、`output_budget_continuation_is_bounded_and_stays_incomplete`、`auto_retry_disabled_does_not_continue_output_budget`、`shallower_thinking_level_walks_down_the_scale`。

## goal worker 的 incomplete 重试 ≠ 续写

对话里的「继续」和 goal worker 的「重试」是两件事。前者是字面续写，后者是重发同一个任务；两者共用同一个截断判定，但携带的上下文完全不同。

### 实际会发生什么

1. `run_loop.rs` 判定流被截断（`finish_length` / `eof_no_terminal` / …），本次 run 以 `incomplete` 结束，`run_terminal.error` 写入 `[OUTPUT_LIMIT] finish_length` 这类分类码，并记下 `truncation = {turns_so_far, output_len, tool_calls_so_far, detected_by}`。`output_len` 是**截断那一刻当前 LLM 回合**的正文长度，不是整个 run 的。
2. `console.rs` 看到 `terminal_state == "incomplete"`，用 `incomplete_streak`（从 ledger 重放，重启不重置）计数，并请求 `incomplete_continue_note`。截断不扣 repair budget（分类为 `InfraRecoverable`），只受自己的上限约束。
3. 下一轮 turn 的报文最前面加上一条便条：`CONTINUE: your previous turn for todo <id> ended INCOMPLETE … Pick up exactly where you left off — do not restart work that already completed.`，连续 `DEFAULT_MAX_INCOMPLETE_RETRIES`（3）次都截断则判 `incomplete_budget`，停下并上报 supervisor。

### 为什么它退化成重做

便条是**文本**，不改变请求形态。worker 的每一轮都是一次全新 prompt，携带的是：

- 同一个 todo 的正文（`compose_turn_envelope`）；
- 上一轮 `RunRecord.evidence`——**整个 run 累积正文的尾部 4 000 字节**（`EVIDENCE_TAIL_BYTES`），进入 envelope 时再截到 1 200 字符；
- goal memory / signals（失败次数、振荡、无进展）。

上一轮的 assistant 消息本体（尤其是被截断的那条）**不在**里面。于是：

- 截断轮留下了正文 → 尾部证据里有它，下一轮看得到「写到哪了」，便条有意义。
- 截断轮 `output_len == 0` → 没有正文可留，证据为空；而 reasoning-only 的 assistant 消息在 `convert_to_llm` 里会被丢弃，历史不留痕。下一轮拿到的报文与第一轮几乎相同，模型只能从零重想一遍，然后撞同一面墙。实测一个 goal 的三轮全是 `completion_tokens 32768 / reasoning_tokens 32761–32767`，即输出额度被思考吃光、正文 0 字；三轮 `truncation` 均为 `{"output_len": 0, "tool_calls_so_far": 0}`。这正是「重试 ≠ 续写」的可观测形态。

对照交互会话：桌面/手机的「继续」发一条**新的用户消息**，被截断的 assistant 文本仍在会话历史里（续写在字面意义上成立），主机侧 `build_continue_prompt` 再从最近 6 条终止事件里摘出摘要注入。worker 侧没有这份历史，只有上面那截证据尾部——这就是同一个便条在两边效果不同的全部原因。

### 已机械检验的边界，与尚未具备的判据

回归覆盖（`orchestration/goal/tests/agent_run_drive.rs`）：

- `incomplete_continue_note_does_not_replay_completed_work`——一次输出截断后，第二轮报文必须带 `CONTINUE:` 便条、必须带上截断轮的部分正文（即「接着写」的信息载体），且该 todo 不得被重新拆成新条目、截断轮的 `failure_kind` 必须是 `infra_recoverable`（截断是 infra，不是科学失败）。
- `incomplete_turn_without_a_body_does_not_surface_its_progress`——钉住当前的缺口：`output_len == 0` 的截断轮，第二轮报文**不含**截断进度（`turns_so_far` / `output_len` / `tool_calls_so_far`），因此模型无从得知自己已经烧掉了多少额度。修好之后这条会红，届时按新契约改写。

**顺带发现（尚未修复）：repair budget 不落盘。** `writeback` 把 `failed_attempts + 1` 写在内存里的 `Todo` 上，但事件账本里没有任何事件携带这个字段——`TodoUpdated` 没有 `failed_attempts`，`RunRecorded` 只带 `RunRecord`，`replay` 从 `TodoAdded` 的初始值 0 起算后无处再更新它。所以**任何一次重放（status / 下一轮 turn / 进程重启）读到的 `failed_attempts` 都是 0**，`decision::stall`、`replan_obligation`、`heartbeat` 的「repair attempt N」以及 `max_validation_attempts` 门禁看到的都是这个 0。现场核实：`goal_d13e9687e665` 的 `events.jsonl` 里 `failed_attempts` 只出现 4 次、全是 `0`；一条人为构造的 `HardError` 轮次写入后重放，读回仍是 `0`。这使 `failed_attempts` 不能作为「续写不扣预算」的断言载体，上面的用例改用账本里真实存在的 `failure_kind`。修复方向是把计数落到一个既能重放又能被 `--verify` 门禁读取的事件上（例如 `TodoUpdated` 增补该字段，或新增 `RepairAttemptConsumed`）。

还缺一条真正的判据：现在没有任何机器可读的信号区分「接着写」与「重做」。建议的形态是**重叠率**——截断点之前最后 N 个 token 与续写首 N 个 token 的重叠比例，N 取 64 与 128 两档，低于阈值即判为「重做」。它只依赖两侧都已持久化的文本，不需要额外埋点，也不受 `run_loop.rs` 截断判定改写的影响。

run 内自动续写（上一节）已经把「同一次 run 里额度用尽」这一层接住：它会自动降档再试，因此 worker 不再需要靠重试去撞同一面墙。只有续写预算也耗尽、run 仍以 `incomplete` 结束之后，才轮到下面这些手段——此时问题已不在额度，而在任务本身超出了单次输出能装下的规模：把大任务按批拆成多个 todo（每批控制在 ~20k 输出以内），并在 todo 文本里显式写出「续跑时从 `<中间结果目录>` 读回已有结果」。

## 诊断与验证

日志记录模型结束原因、输出字节数、思考块数、工具数，以及协议结束／HTTP EOF 和收包计数；不记录正文或思考文本。原始历史不能据 `completed` 反推上游的具体结束标记。

回归覆盖：正常 stop、拒绝、只有思考、空响应、长度限制、内容限制、暂停、上游取消、未知原因、没有终态的结束，以及 Desktop 实时／历史原因映射。模型网关仍可能存在未声明的协议差异；新增兼容规则必须有构造协议帧或脱敏证据。
