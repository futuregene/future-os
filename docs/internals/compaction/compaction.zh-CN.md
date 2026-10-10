# 运行时压缩

压缩把会话较早的历史——**仅对下一次请求**——替换成一份有界投影。原始 journal 既不被删除也不被改写，召回直接读它。

**共两个策略，运行时默认是第二个：**

| `algorithm_version` | 投影内容 |
|---|---|
| `deterministic-evidence-v1` | 受保护原文 + 近期尾部 + 确定性工具证据索引。**不调用模型。** |
| `summarized-evidence-v1` | 同一投影，外加一份由模型撰写的交接摘要 |

只写入这两个值，也只读取这两个值。`deterministic-evidence-v1` 同时是兜底策略：无 provider 可用或摘要调用失败时提交它。

[对比实验](compaction-closed-book-experiment.zh-CN.md)测量两者各自保留了什么、以多大体积和多少成本保留；[开发文档](compaction-development.zh-CN.md)给出代码与持久化状态机的地图。

## 触发与准入

经济触发线为 `floor(W × 0.8)`——**声明窗口的 80%，没有绝对上限**。`effective_trigger` 会把它夹到 `W − O − margin`，其中 `O` 是本次请求真正发送、并由准入预留的输出预算，`margin = min(2048, W/16)`。

`O` 不是模型声明的上限本身，而是它经 `models::effective_max_tokens` 收窄后的值：`min(声明值, 65536, W/4)`（窗口未知时只剩绝对上限）。供应商声明的上限常常等于窗口——Kimi 的 `/v1/models` 返回 `context_length == max_tokens`，因为输入与输出共享同一个窗口——照单全收会让 `W − O − margin` 归零，连会话的第一个请求都发不出去，且一次模型调用都不会发生。同一个函数既决定请求里的 `max_tokens`，也决定准入预留，两者不会各说各话。

因此 1M 窗口、声明 384000 输出上限的模型预留 65 536、触发于 **800000**（窗口的 80%，声明 16 384 时同样是 800000）；Kimi k3（1 048 576 窗口、声明同值）触发于 **838860**；262 144 窗口声明同值的模型由比例界决定，触发于 **194560**（窗口的 74%）。

每次模型调用前都检查，包括工具回合后和模型窗口下调时，并预留输出预算 `O` 与余量——输入必须装进 `W − O − margin`。预留本身也受同一条界约束：即使调用方传来未经收窄的声明值，输入仍保有窗口的四分之三。估算覆盖 system 文本、工具定义、消息框架与图片／reasoning 的保守成本，并参考上游报告的用量。切换模型本身不触发整理：那次真实请求才检查自己的限制。没有可处理旧历史时，装得下的输入不会仅因越过触发线而被截断；必要内容装不下则明确失败，而不是被静默截断。

## 投影的组成部分

1. 覆盖范围内的用户文本与选中的 assistant 原文；
2. 最多 **2048 estimated tokens** 的证据槽，小窗口缩为 `min(2048, W/8)`；
3. 最近约 8K 的成对工具／对话历史，小窗口缩小。

历史整体目标约 32K，不含 system／工具定义开销；必要时可扩至 **128K**，且不突破实际容量：`min(128000, (limit − fixed) × 3/4)`，128K 窗口得 82K、262K 窗口得 96K。

用户文本优先。assistant 原文放不下时保留较新且能容纳的，被省略的明确标记为**未做摘要**，其原文仍可查。保护区不带入工具调用对象、thinking、媒体正文或隐藏 provider 状态。

## 证据选择

只扫描到本次准入的覆盖边界，顺序为：错误结果优先；再按工具与目标路径／命令分组，优先有错误或 config/schema/validation/test 类目标；然后取每组最新与最早记录；剩余空间按新近顺序补足。

每条选中记录渲染为一行有界 JSON，带 `entryId`、`blockIndex`、`sourceOrder`、调用 ID、工具／目标、错误标志与头尾片段（最多 380 + 100 字符，空间不足时缩小）。不把 JSON 行截成半条来填预算，元数据另有长度限制。

索引按优先级排列而非时间线：`sourceOrder` 是原始顺序，旧错误可能已被替代。被省略的中段与未选中记录保持未知，执行成功不等于验证通过。片段是历史证据而非新的执行授权，歧义或复用的调用 ID 不会被归属到任意路径。证据不携带 reasoning、图片正文或完整 provider 元数据。

## 摘要请求

`summarized-evidence-v1` 把活动对话作为**真实消息**发送、指令**追加在最后**，从而不动前缀，使请求有机会由 provider 的前缀缓存提供。前缀依次是 system prompt、工具定义、消息数组，从第 0 个 token 起比对，因此请求携带**会话自己的 system prompt** 与**会话自己的工具定义**。system prompt 原先会在 checkpoint 之后追加召回指引，预算必须预留它、摘要请求必须复现它；该指引已删除，两条路径现在都只发送会话自己的提示词，并有测试把两处钉在同一个字符串上。

改动这三项中任何一项都会使前缀分叉，整段对话重新按全量计费。实测：替换 system prompt、去掉工具定义、或只多一行，在已预热前缀上的命中都是 **0%**，而同形状请求为 93.7%。在隔离 agent 上跑该路径实测：会话增长到 212 911 tokens 后压缩，`cache_read = 212 548`——**99.8%** 由缓存提供，`cache_write = 360`，约 ¥0.003，而冷启动约 ¥0.53。

摘要是**累积**的：指令携带上一份摘要，新摘要写好后旧的自然作废。它的额度是 `min(W/16, 4096)` 加 512 token 槽位余量，仅在需要摘要时授予，并会放宽准入预算，使证据索引保持完整体积、摘要又能落在 `finalize` 强制的目标内。摘要失败或无可用 provider 时提交确定性投影，索引开头会声明这条消息里没有摘要。

## checkpoint 与幂等

只读取当前 schema。由已退役算法写出的 checkpoint——旧 `schema_version`，或已发布的字符串协议标记——**不被识别**：`latest_context_checkpoint` 跳过它，`project_prompt_context` 看不到边界、改为全量投影 journal，下一次压缩即用当前算法重新覆盖这段历史。那条退役记录留在 journal 里，成为没人读取的惰性条目，既不能使 prompt 变短也不能使其变长。代价是**一次输入等于整个 journal 的请求**（通常因该回合刚发过而命中缓存）以及该会话**多一次摘要调用**，仅一次。

`compaction_operations` 的键包含原始 user/system/assistant/tool 条目的身份与内容、相关配置、预算、模式／阶段、备注与策略版本；checkpoint、用量和 session-info 的更新本身不算原始历史变化。同键成功会复用结果、不再追加 checkpoint，有序 writer 把 checkpoint 与 completed 收据写在同一事务。未决的 `started` 操作仍保留并发／恢复 fence——不自动抢占，也不冒称成功。删除会话清理收据，fork 使用新范围，纯内存／ephemeral 调用没有跨重启收据。需要配套升级 Agent 与 CLI：旧 Agent 不实现这些语义。

保护原文按引用重建；fork 会重映射引用，非法范围被拒绝。

## 用户 CLI

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "保留最新约束与验证边界" --json
future session compact --help
```

返回 `accepted` 与 `operationId` 的异步 ACK，不代表完成；完成、复用或失败由 Agent 事件报告，活跃 run 会被拒绝。没有 `--wait`、`--force`，也没有模型自请求入口。

`--instructions` 是**逐字的用户备注**，供后续接续使用。确定性选择器不解释自然语言，也不声称已按其完成语义筛选；备注计入证据预算，过长会明确失败而不是被静默截断。

## 费用与召回

确定性压缩**不发起任何摘要调用**；默认策略每次压缩增加一次摘要请求，按普通请求计费。普通用量与费用计数保持原样，后续请求仍需为证据索引与召回文本的输入付费，本地扫描也消耗时间与内存。

运行时不向模型附加任何[历史召回说明](../../guide/session-history.zh-CN.md)：该说明已删除，因为它没有改变行为（度量见 [compaction-open-book-experiment.zh-CN.md](compaction-open-book-experiment.zh-CN.md)）。保留的检索入口是普通 shell 工具背后的 `future session history search`/`get`，以及日志已在检索/读取结果中给出的 entry id；缺少精确旧事实时读取原文，不得重放历史副作用。

## 验证

```sh
cargo test -p future-agent
cargo build -p future-cli --bin future
python3 scripts/tests/test_s2_compaction.py --binary target/debug/future --report target/c-smoke.json
```

Windows 使用 `future.exe`；设置了 `CARGO_TARGET_DIR` 时相应调整路径。合成 smoke 使用隔离 HOME、新端口与本地模型桩，验证 80% 窗口触发、标识、原文、普通用量、字节分页与重启，并验证被拒的摘要会回退且不产生第二次计费调用。不要停止用户正在运行的 Agent。

规则证据是有损的，尤其对复杂非结构化材料；原文召回仍是必要补充。
