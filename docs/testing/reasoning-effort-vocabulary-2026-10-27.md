# reasoning_effort 词表成为数据（2026-10-27）

一次线上 400 的修复记录。结论：**“这个模型接受哪些 effort 值”是配置数据，不是约定**。
模型用 `compat.reasoningEffort` 声明它的访问路径实际接受的值（拼写词表 + 可选整数区间 +
参数位置），我们的 `off/minimal/low/medium/high/xhigh/max` 再按单调规则映射到这份词表上。
没有声明的模型行为完全不变（档位原样上线，见 `thinking-levels-2026-10-26.md`）。

## 1. 故障与复现

会话 `20261027-113424`（`westlakehpc/deepseek-flash`，`thinking_level = medium`）在
`agent.db` 的 `run_terminal` 里留下：

```text
LLM API request failed (HTTP 400): DeepSeek V4.1 reasoning_effort must be
low, high, xhigh, max, or an integer within [1, 100] in chat_template_kwargs
```

用最小请求直接打该 provider（`base_url = https://hpc-api.westlake.edu.cn/v1`，
`model = deepseek-flash`，`max_tokens = 16`），请求体只差 `reasoning_effort` 一个字段：

| 请求体里的写法 | HTTP | 响应 |
|---|---|---|
| `"reasoning_effort": "medium"` | **400** | 与上面同一句原文 |
| `"reasoning_effort": "minimal"` | **400** | 同上 |
| `"reasoning_effort": "low"` / `"high"` / `"xhigh"` / `"max"` | 200 | 正常 |
| `"reasoning_effort": 50`（顶层整数） | **500** | `json: cannot unmarshal number into Go struct field GeneralOpenAIRequest.reasoning_effort of type string` |
| `"chat_template_kwargs": {"reasoning_effort": 50}` | 200 | 正常 |
| `"chat_template_kwargs": {"reasoning_effort": "high"}` | 200 | 正常 |
| `"chat_template_kwargs": {"reasoning_effort": "medium"}` | **400** | 同一句原文 |
| `"chat_template_kwargs": {"reasoning_effort": 0}` / `101` | **400** | 同一句原文（整数区间确为闭区间 1..=100） |
| `"thinking": {"type": "disabled"}` | 200 | 正常（`off` 的关闭拼写） |

即：**枚举 `low|high|xhigh|max` 与整数 `1..=100` 都被接受，但整数只能放在
`chat_template_kwargs` 里；`medium`/`minimal` 两个拼写根本不存在**。所以真正发出去的
`reasoning_effort` 是 `"medium"`，与 provider 的 400 一一对应，和 `chat_template_kwargs`
本身无关（同一位置放 `"high"` 就是 200）。

修复后同一会话路径（隔离 home、真实 provider、真 agent）在 `medium` 上跑通，
实际发出的请求体（对本地 mock 上游抓包，`--thinking` 逐档）：

| 档位 | 发出的字段 |
|---|---|
| `low` | `thinking={"type":"enabled"}`, `chat_template_kwargs.reasoning_effort="low"` |
| `medium` | `thinking={"type":"enabled"}`, `chat_template_kwargs.reasoning_effort=50` |
| `high` | `thinking={"type":"enabled"}`, `chat_template_kwargs.reasoning_effort="high"` |
| `xhigh` | `thinking={"type":"enabled"}`, `chat_template_kwargs.reasoning_effort="xhigh"` |
| `max` | `thinking={"type":"enabled"}`, `chat_template_kwargs.reasoning_effort="max"` |
| `off` | `thinking={"type":"disabled"}`，不发 effort |
| `default` | 什么都不发 |

## 2. 声明形状

`compat.reasoningEffort`（模型级，或 provider 级——与 `thinkingFormat` 一样按 key 合并）：

```json
"compat": {
  "thinkingFormat": "deepseek",
  "reasoningEffort": {
    "values": ["low", "high", "xhigh", "max"],
    "integer": {"min": 1, "max": 100},
    "param": "chat_template_kwargs.reasoning_effort"
  }
}
```

- `values`：接受的拼写，**原样发送**。声明了某个档位就直接用它，所以“把每一个档位都列全”
  等价于旧的无声明行为。
- `integer`：接受的闭区间整数（DeepSeek V4 的 `1..=100`，也可能是 token 预算）。
- `param`：值写到哪里，点号路径，默认 `reasoning_effort`。它存在的原因是网关对
  **数字**只认 `chat_template_kwargs` 里的那一份（见上表 500 的那行）。
  Responses API 的 effort 位置固定为 `reasoning.effort`，忽略该键。
- `values` / `integer` 至少给一个；**未知键直接报错**（`"value"` 这种拼错不会被静默忽略，
  否则词表被悄悄收窄，400 又回到线上）。

`thinkingLevels` 的含义因此更精确了：它是**模型提供的档位**（选择器显示、会话校验都用它），
只有在模型声明了 `compat.reasoningEffort` 时，档位才会被映射成词表里的值；否则含义与
2026-10-26 的契约完全一致（档位就是 wire 值）。

## 3. 映射规则（单调，不丢语义）

产品档位固定为 `off < minimal < low < medium < high < xhigh < max`。

1. **列在 `values` 里** → 原样发送。
2. **没列但声明了 `integer`** → 发整数。提供方的区间被当作一条连续的深度轴，我们的七个档位
   在轴上等距取点：`off = min`、`max = max`，于是 `1..=100` 时
   `minimal = 17`、`low = 34`、`medium = 50`、`high = 67`、`xhigh = 83`。
   `medium` 恰在中点，所以是 **50** —— 这是“中间档”在对方唯一能表达的形式。
   这是唯一一处我们会替 provider “发明”一个它没写出来的值，所以假设写明：
   `values` 里的拼写应当落在它自己的刻度位置上，这也是“映射后的值仍然单调”的依据。
3. **没列也没有 `integer`** → 吸附到刻度上最近的拼写，**平手取更浅的**（与
   `clamp_thinking_level` 同一条原则：请求无法满足时绝不比用户选的更费）。
   不在刻度上的名字（`turbo`）没有位置，不发 effort（用 provider 默认），并在模型解析时
   记一条 warn。

兜底不静默：声明本身写错（未知键、区间倒置、既无 `values` 又无 `integer`）会让模型解析
失败——`set_model` 阶段就报错并带上模型名；档位无法被词表表达时，
`report_unexpressible_levels` 在模型解析处（切换/重载/首个请求）按
`provider/model/level` 去重记一条 warn，说明它实际会发什么、词表接受什么。

## 4. 代码落点

- `agent/src/llm/schema.rs`：`ReasoningEffortVocabulary`（解析 + `mapping`/`resolve` +
  `describe`）、`effort_wire_value`、`apply_effort_param`（按点号路径写入并合并
  `chat_template_kwargs`）、`report_unexpressible_levels`。
- `agent/src/llm/adapters/openai_chat.rs`：每个 `ChatReasoningFormat` 分支都把 effort 交给
  `apply_effort_param`，格式自己的开关（`thinking` / `enable_thinking` / `chat_template_kwargs`）
  不变；`reasoning-split` 仍然只发开关。
- `agent/src/llm/adapters/openai_responses.rs`：`reasoning.effort` 走同一个解析。
- Anthropic 不受影响：手动模式仍是 budget 映射，自适应模式发 `output_config.effort` 原值。

## 5. 测试

| 测试 | 抓什么 |
|---|---|
| `a_declared_vocabulary_replaces_a_spelling_the_provider_refuses` | 本次故障的形状：`medium` 仍可选（不被删档），但发出去的是 `50`；`off` 仍然只发开关 |
| `a_vocabulary_without_an_integer_range_snaps_to_a_spelling` | 没有整数区间时向最近拼写收敛 |
| `a_vocabulary_works_for_every_chat_format_and_for_responses` | `openai`/`qwen`/`qwen-chat-template`/`deepseek`/`zai` 与 Responses 都把 `medium` 映射掉 |
| `every_registered_model_at_every_level_sends_only_declared_values` | **内置目录全部模型 + 手写形状（含本次故障形状）× 它提供的每个档位**：发出去的 effort 必须是该模型声明的值；没有词表的模型必须原样发送；声明了词表却“什么都没发”也算失败 |
| `vocabulary_mapping_is_monotone_across_the_whole_scale` | 四种词表形状下，七档映射的位置单调不减 |
| `vocabulary_parse_rejects_a_declaration_it_cannot_honor` | 未知键 / 区间倒置 / 两空 / 非法 param 全部报错 |

把 `effort_wire_value` 临时改回“忽略词表”时，前三条与全量遍历测试一起变红，说明它们咬得住
这次故障，而不是给已有行为盖章。

全量回归：`cargo +1.99.1 test -p future-agent --lib` → 2526 passed（本机需
`ulimit -n 4096`，默认 256 会让文件描述符密集的用例假红）；
`cargo clippy -p future-agent --all-targets -- -D warnings`、`cargo fmt -p future-agent`
干净。

## 6. 顺带核查

- 内置目录（`reasoning=true` 的条目声明 `minimal/low/medium/high`）没有任何词表声明，因此
  行为不变；全量遍历确认它们每一档都原样发送。
- `zai` 与 `reasoning-split` 只有开关：档位表达能力受协议限制这一点没有变化，本次没有为它们
  编造深度。
- `future/*` 平台模型的档位来自平台目录（`thinking_levels`），词表也应在平台侧
  （`models.yaml`）声明；本仓无法替它决定，见下。

## 7. 未解决

1. **平台目录的模型仍然只能“整组声明”**：`future/*` 的 `thinking_levels` 来自服务端，若某个
   平台模型的词表与我们七个档位不同，需要在 `/v1/models` 里给出词表字段，本仓这份改动只解决
   `openai-completions` / `openai-responses` 侧。
2. **Provider 编辑器不写 `compat.reasoningEffort`**：桌面/移动/TUI 的模型编辑器只选档位，
   词表仍需要手写 `models.json`（编辑器会保留该键，不会被下一次保存覆盖）。把词表做成表单项
   是下一步的产品决定。
3. 本次修的是 westlakehpc 的 `deepseek-flash`；同一网关的 `Qwen`、`glm-flash` 词表未知，
   没有替它们猜。给它们声明词表前同样要探测一次。
