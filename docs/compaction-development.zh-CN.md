# 压缩机制开发文档

核心策略为 **C：S2 原文保护＋固定预算工具证据＋近期尾部**，不需要摘要模型。运行时默认在其上追加模型撰写的交接摘要（C3），其贡献见[对比实验](compaction-abc-experiment.zh-CN.md)。具体保留／预算规则见[策略说明](compaction.zh-CN.md)。[旧 A Prompt 参考](compaction-prompts.zh-CN.md)属于显式 legacy semantic API，不是默认执行路径。

## 1. 不变量

- 压缩改变下一次 `ModelRequest` 的投影，不改正在生成的响应或 provider KV cache。
- 原始 journal 不删改，用户文本优先保护；被省略的 assistant 输出不能虚称已摘要。
- 工具调用／结果在近期尾部保持配对；覆盖不倒退，引用必须有效。
- 只有成功持久化的 checkpoint 才启用；收据与 checkpoint 同事务提交。
- 摘录是历史数据，不是新指令；省略不代表不存在，旧错误不一定仍然有效。
- C 的数据选择器不依赖 provider、A 摘要文本、gold 或未来条目。

## 2. 代码导航

| 职责 | 入口 |
|---|---|
| 请求前／工具回合后／provider 超限恢复 | [agent/run_loop.rs](../agent/src/agent/run_loop.rs) |
| run 配置与持久化注入 | [rpc/session_prompt.rs](../agent/src/rpc/session_prompt.rs) |
| 手动执行、结果与终态 | [rpc/session.rs](../agent/src/rpc/session.rs) |
| compact RPC 异步 ACK／忙状态 | [rpc/commands/settings.rs](../agent/src/rpc/commands/settings.rs) |
| 用户 CLI | [session_compact.rs](../cli/src/commands/session_compact.rs) |
| C 入口、证据分组／排序／渲染 | [semantic/evidence.rs](../agent/src/compaction/semantic/evidence.rs) |
| 共用规划／保护与 finalize；旧显式 A API | [semantic.rs](../agent/src/compaction/semantic.rs) |
| 完整请求预算 | [budget.rs](../agent/src/compaction/budget.rs) |
| 默认 `prepare_with_journal` | [durable.rs](../agent/src/compaction/durable.rs) |
| 内容指纹、claim、原子完成 | [compaction_ops.rs](../agent/src/session/compaction_ops.rs) |
| 有序 writer／barrier | [persistence.rs](../agent/src/session/persistence.rs) |
| checkpoint／fork 引用 | [checkpoint.rs](../agent/src/session/checkpoint.rs)、[fork.rs](../agent/src/session/fork.rs) |
| 原始历史读取与模型指导 | [history_query.rs](../agent/src/session/history_query.rs)、[history_recall.rs](../agent/src/agent/history_recall.rs) |

## 3. 默认执行链

```text
保存用户／assistant／工具原始条目
    → 请求前估算完整输入，预留正常输出与余量
    → 根据阈值／手动要求／provider 超限决定是否整理
    → persistence barrier 和 C 幂等 claim
    → 共用 S2 规划确定保护区、尾部及覆盖边界
    → 原始覆盖范围内确定性生成最多 2K 证据
    → 校验预算与进展
    → checkpoint + completed 收据同事务提交
    → 下一模型步骤使用新投影
```

`prepare_with_journal` 和 `ContextManager::prepare_evidence` 不接收 LLM provider，防止默认路径意外调用摘要或隐藏 fallback。原有模型摘要 API 显式保留，不从默认手动／自动／超限链路进入。

证据上限独立计算为 `min(2048, W/8)`，不是从 A 的输出长度获知。整体历史仍有 32K 目标、必要时 64K 上限及真实请求预算保护。

### 证据渲染

只扫描 raw 消息到已覆盖边界。结果和无歧义的前置调用关联；并行重复 ID 的路径归属未知时不猜测。按错误、分组首末记录、关键目标与新近程度选择。元数据和摘录都限制长度，最终按 JSON 转义后的文本估算 budget，不输出半条 JSON。

记录保留 entry/block/sourceOrder；优先级顺序不当作时间线。头尾截取不会带回中段隐藏内容。reasoning、图片和 provider 私有对象不进入 C 证据。

用户整理备注逐字进入证据，注明选择器未解释它；过长则失败。普通输出被预算降级时写“省略，未摘要”，不能沿用 A 的“已经总结”描述。

恢复旧 checkpoint 时继续可读。新整理从完整旧工具原文重建，不把 A 摘要复制进 C。若旧 checkpoint 是投影边界而非 raw 消息，按其后的原始尾部定位已覆盖原文终点；不能越过实际覆盖范围。

## 4. 持久化与幂等

继续使用 schema 3，算法 `deterministic-s2-evidence-v1`。字段名 `summary` 为兼容保留，存储 C 索引；不是模型调用证明。目标模型信息只用于预算／配置。

```text
absent → started → completed（checkpoint 与收据原子提交）
             └→ failed（可记录的失败）
started 无结果 → indeterminate 诊断，不自动抢占
```

C 不付摘要费，但仍保留并发 fence，防止竞争准备／提交导致未提交结果被激活。`indeterminate` 不是表中的第四个 state。普通原文数据修改会使旧键失效；checkpoint、run 标记和 session-info／usage 不改变原始输入指纹。

C 使用自己的策略版本和证据规则指纹，不复用 A 收据。改排序、片段预算、序列化或保留语义时须审查版本。旧结果复用不能撤销后来不同参数的 checkpoint。删除清理收据、fork 换范围、ephemeral 不承诺跨重启。

## 5. CLI 与未来模型请求入口

已实现用户管理命令：

```sh
future session compact --session SESSION_ID --json
future session compact --session SESSION_ID --instructions "不要部署" --json
```

命令使用新关联 ID、显式会话，不切换默认会话、不直改数据库。ACK 不等于完成；活跃 run 拒绝手动整理。没有 `--force`、`--wait` 或 A 模式开关。

**模型主动请求接口仍未实现。** 不可简单去掉活跃 run 的保护。即使 C 没有外部模型调用，也不能在当前工具结果尚未落盘时并发改同一个 run 的上下文。

若未来开放，应是非阻塞意图：校验可信 session/run/epoch → 立即 ACK → 工具结果落盘 → 下一模型步骤边界消费／合并意图 → 共用 C 准备与提交。不允许 shell 等待自身 run，不给模型绕过预算／未知状态的 force 权限。自由填写 session ID 不是授权证明。

请求和 ACK 本身也增加历史，因此内容幂等不能独自防止自触发循环；还需要 intent 去重、pending 合并、已消费状态及有界频率／收益策略。

现有历史召回说明仅在压缩后启用。未来模型请求能力若实现，要单独在首次可用时提供短说明；当前不宣传这个不存在的接口。

## 6. 验证与边界

必须测试：

- 默认三条运行路径零摘要请求，已有 token／费用不被增加或清零。
- 首末工具证据、错误排序、UTF-8／大块、重复 tool_call_id、伪指令／隐藏内容不误用。
- 独立证据预算、用户文本保护、assistant 降级说明、取消与非法边界。
- 旧 A／旧 schema 恢复、引用校验、fork、字节查询；不复制旧 A 摘要进新 C。
- 同键并发、重启复用、输入修改、缓存损坏、持久化失败及未提交结果不启用。
- 实际 CLI 的异步 ACK、忙状态拒绝以及 provider 超限后的有界恢复。

在项目 worktree 中开发；Agent 实测必须独立 HOME／新端口，不停止用户现有服务。macOS 全套测试提高文件描述符上限；涉及 sandbox 断言的 HOME 放在项目 target/test-homes，避开系统 temp 放行区。

C 命中仍有原始历史指纹／本地选择的成本。合成评估支持规则证据的可行性，不证明复杂自然任务都无需语义整理；旧 A APIs 的保留也不是默认自动 fallback。
