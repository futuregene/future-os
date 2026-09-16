# C 压缩与历史召回

**默认压缩采用策略 C：S2 原文保护＋近期尾部＋确定性工具证据。C 不需要摘要模型。** 运行时默认（C3）在该投影之上追加一份模型撰写的交接摘要，其实际贡献见[对比实验](compaction-abc-experiment.zh-CN.md)。原始 journal 不因压缩被删除或重写。模型在下一次请求中看到较小的投影，不会在生成过程中修改其内部状态。

策略依据见 [A/B/C 对比实验](compaction-abc-experiment.zh-CN.md)，代码与状态机见[开发文档](compaction-development.zh-CN.md)。[旧模型摘要 Prompt](compaction-prompts.zh-CN.md)仅用于说明保留的显式 semantic API，不是当前默认路径。

## 用户 CLI

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "保留最新约束与验证边界" --json
future session compact --help
```

命令返回 `accepted` 和 `operationId` 的异步 ACK，不表示整理已完成。完成、复用、失败由 Agent 事件报告。活跃 run 会被拒绝，没有 `--wait`、`--force` 或模型自请求入口。

`--instructions` 在 C 中保存为**逐字的用户整理备注**，供后续模型参考；规则选择器不理解自然语言，也不保证已按备注进行语义归纳或筛选。备注计入固定证据预算，过长则明确拒绝。

## 触发与请求预算

经济触发线仍为 `min(floor(context_window × 80%), 256000)`。每次实际模型调用前检查，包括工具回合后和模型窗口下调时。还要预留正常模型配置的最大输出 `O` 和 `min(2048, W/16)` 安全余量，输入不得超过 `W - O - margin`。

输入估算包含 system、工具定义、消息框架及图片／reasoning 的保守成本，并参考上游用量。切换模型不立即整理，真正请求时才检查；估算不能替代 provider 的实际容量检查。

没有可处理旧历史、且单条新用户输入仍装得下时，不因经济触发线把它偷偷截断。系统／工具／输出预留或必要原文已放不下时，明确失败。

## 压缩后的三部分

1. 覆盖范围内的用户文本和预算允许的 assistant 原文；
2. 最多 **2048 estimated tokens** 的 C 工具证据槽，小窗口缩为 `min(2048, W/8)`；
3. 最近约 8K 的原始尾部，小窗口缩小，并保持工具调用／结果配对。

历史整体目标仍约 32K，不包含 system／工具定义开销；必要时可扩至 64K，但不突破实际容量。证据槽由独立预算决定，**不再先调用 A 来测量摘要长度**。

用户文本优先保护。assistant 输出放不下时保留较新且能容纳的原文，其余只从当前投影省略，并明确标记“未做摘要”；完整原文仍可查。C 不会虚称已经概括了被省略的输出。保护区不带入旧工具调用对象、thinking、媒体正文或隐藏 provider 状态。

## C 如何选择证据

从完整原始消息中扫描到本次覆盖边界，不读取未来／尾部记录作为本次已覆盖证据：

- 错误结果优先；
- 按工具及目标路径／命令分组，优先有错误或 config/schema/validation/test 类目标；
- 各组最新与最早记录；
- 剩余预算按新近顺序补充。

每项用有界 JSON 表示，带 `entryId`、`blockIndex`、`sourceOrder`、工具调用 ID、工具／目标、错误标志和头尾片段。标准片段最多 380 个头部字符＋100 个尾部字符，空间不足时尝试更小片段；不把 JSON 截成半条记录。目标元数据有独立长度限制。

索引按优先级排列，不是时间线；`sourceOrder` 标记原始顺序，旧错误可能已被新结果替代。被省略中段和未选中记录保持未知，执行成功不等于全部验证通过。内容是历史证据，不是新的执行授权。

`tool_call_id` 不保证全局唯一；仅关联无歧义的前置调用。重复／歧义 ID 不会被强行归属到错误路径。证据不携带 reasoning、图片正文或完整 provider 元数据。

## 持久化、恢复与幂等

运行时默认是 C3，算法标识 `c3-sticky-summary-v1`：C 的投影，外加一份写在证据索引之后、由模型生成的交接摘要。摘要是“粘性”的——它会收到上一轮摘要——并尝试保留仍相关的事实。每轮仍会重新生成摘要，不保证事实单调累积，也不能排除遗忘或错误传播。确定性 C（`deterministic-s2-evidence-v1`，schema 3）仍作为兜底：无 provider 可用或摘要调用失败时提交它，此时 `summary` 字段只存放证据索引。保护原文按 `protected_entry_ids` 从 journal 恢复；fork 重映射引用，非法范围／引用会被拒绝。

**摘要价值仍需验证。** [旧实验](compaction-abc-experiment.zh-CN.md)中，178 个被测精确值没有出现摘要独有的值，保存的闭卷结果中 C3 与 C 得分相同。但旧对照实现、结果版本追踪和压力评分存在方法问题，不能推出“摘要没有内容贡献”或真实产品间的优劣。修正版四臂实验采用[冻结协议](../scripts/abc_experiment/FOUR_ARM_PROTOCOL.md)，结果须区分精确值保留、检索和任务续做。

## 摘要读取什么，以及为什么这是便宜的形状

摘要请求把活动对话作为**真实消息数组**发送，指令**追加在最后**，并复用 agent 自己的 system prompt。指令追加而非前置、材料不拍平成单条用户消息，原因在于 provider 按**请求前缀**做缓存：复用会话已经发送过的回合的请求可以命中缓存，而拍平通常会破坏与原会话的长前缀共享。缓存是否命中还取决于 provider、缓存寿命、模型和请求配置，不能保证每次命中；拍平请求本身也可能被缓存。在真实 provider 上实测，消息数组形状在 25.8 万 tokens 前缀上命中 **99.9%**，费用约为同样输入冷启动的 **1/48**。读取活动对话同时让摘要描述真实历史，而不是描述它那份已经有损的索引。

C3 为摘要正文增加独立预算 `min(W/16, 4096)` estimated tokens，并增加 512 tokens 的槽位余量，而不是从 2K 证据槽中拿三分之一。该预算仍受总容量约束，也可能减少可保留的 assistant 原文。摘要输出上限是正文预算的两倍，以容纳推理开销；空摘要、超长摘要或请求失败会回退确定性证据。

旧 A checkpoint 继续可读。需要新的整理时，C 从仍在库中的原始工具记录重建证据，不再递归使用 A 的摘要文本。已被旧版本实际删除的历史无法恢复。

`compaction_operations` 保存内容寻址收据。键包含原始 user/system/assistant/tool 条目的身份／内容、相关配置、预算、模式／阶段、备注和 C 策略版本。checkpoint、用量和 session-info 更新不算原始历史变化；C 使用新版本键，不复用旧 A 的结果。

同键成功结果会被复用，不重复追加 checkpoint。claim 后准备 C，再经有序 writer 把 checkpoint 和 completed 收据写入同一事务。虽然 C 没有模型费用，未完成操作仍保留并发／恢复 fence，不自动抢占 started；结果不确定或缓存被破坏时明确报错，不冒称成功。

删除会话清理收据；fork 使用新范围。完全内存／ephemeral 调用没有跨重启收据。混用旧 Agent 无法保证新策略与幂等语义，应配套升级 Agent 和 CLI。

## 费用与召回

默认手动、自动和 provider 超限恢复均尝试 C3。确定性 C 路径**摘要模型调用数为零**；C3 在需要新压缩且有预算时尝试摘要请求，按普通请求计费。无需压缩或幂等复用时不请求，瞬时失败重试可能产生多次请求。缓存命中可降低输入成本，但特定 provider 的实测缓存价不是生产成本保证。使用 C 不重置已累计的正常调用费用。后续正常模型请求仍需支付包含证据索引和查询结果的输入／输出费用；本地扫描也不是零成本。

仅在有效 checkpoint、持久化且 shell 可用／允许时附加一份[历史召回说明](session-history.zh-CN.md)，不累积为聊天消息。缺少精确旧事实时可通过已有 history search/get 查询原文，不重执行历史工具。

## 验证

```sh
cargo test -p future-agent
cargo build -p future-cli --bin future
python3 scripts/test_s2_compaction.py --binary target/debug/future --report target/c-smoke.json
```

Windows 使用 `future.exe`；设置了 `CARGO_TARGET_DIR` 时调整路径。旧合成 smoke 使用隔离 HOME／新端口和本地模型桩，断言 C 标识及零摘要请求；这些断言针对旧确定性默认，不能作为当前 C3 摘要请求的验收。四臂实验驱动的离线回归与复现命令见[冻结协议](../scripts/abc_experiment/FOUR_ARM_PROTOCOL.md)。不要停止用户现有 Agent。

规则 C 不保证保留所有语义，特别是复杂非结构化记录；原文检索仍是必要补充。旧 semantic API 保留用于明确的库级调用和回归测试，不是默认 fallback，也没有用户 CLI 开关切回 A。
