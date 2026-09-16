# fix-agent——第二轮交接与完整发现账本

> 本文是历史快照 [fix-agent — round-2 handoff and complete finding ledger](./fix-agent.md)（2026-09-11，commit `914f27f6`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

会话 `20260911-123030-b1012f`；todo `todo_024bdc7eab9e`；分支 `claude/bughunt-agent`；独占 worktree `/Users/geilige/future-os/.worktrees/bughunt-agent`。

**本地实现/检查已交付供评审。不构成 Windows 原生认证、外部未验证发现的解决，或全局 224 发现目标的完成。** 全部 79 个原始分配 ID 在下方有账。任何已确认的剩余 agent 实现均未被静默省略。显式的外部、潜在、委托与原生验证限制仍与已修复的生产缺陷分开。

来源：只读的 `/Users/geilige/future-os/.future/bughunt/REPORT.md`、`CROSS-MODEL-VERIFICATION.md`、w01–w13、分配的 w25 章节、V1/V2/V4、KIMI、GLM-b1/b2/b3/b6。当前源码、实际测试与可达调用方优先于旧的验证者标签。ROUND2.md 授权额外的 120 分钟窗口，epoch 1789108361–1789115561；它取代第一轮的计时指示。

## commits

按顺序应用本分支的提交（本 worker 未推送或合并任何提交）：

1. `d4781920`——第一轮审批/隔离/有界执行检查点。
2. `1f677403`——第一轮回归修正与诚实的部分证据。
3. `950742c2`——第二轮持久化、流清理、Windows 参数、模型限制、规则诊断与回归。
4. `2ce2fba7`——最终第二轮已验证的背压/边界回归、原生测试定义与对账后的 79 行账本。

该提交后的最终确定性 INDEX.json 对账：**79 行、79 个唯一分配 ID，无缺失或多余 ID**。代码与成功的 16:09:47 完整批次一致。以下仅含文档的提交记录这些不可变 commit ID；验证后未再改代码。

最终报告取代过时的第一轮“compiled/pending”行注。历史部分状态保留在 git 中，未混入当前处置。

## tests——实际最终全 crate 结果

严格从此 worktree 执行：

`python3 /Users/geilige/future-os/.future/bughunt-fix/check-rust.py future-agent`

所提供的脚本序列化整个批次并固定 Rust 1.97.0、CARGO_HOME、RUSTUP_HOME、共享 target、隔离 HOME/USERPROFILE 与 fd 上限。**于 16:09:47 CST 通过**，包括：

- `cargo fmt -p future-agent --check`：通过。
- `cargo clippy -p future-agent --all-targets -- -D warnings`：通过。
- `cargo test -p future-agent`：默认并行度通过。
- 库：**1749 通过、0 失败、1 忽略**（24.50 秒）。
- `cli_smoke`：**23 通过**，包括隔离的 agent 启动/关闭、单例、严格的非法地址处理。
- `compaction_persist`：1 通过；`session_load_test`：1 通过；`sqlite_startup`：3 通过；`sqlite_storage`：14 通过。
- Main/doc 测试目标：零用例，成功。仅 Linux 的 smoke 目标：macOS 上零用例。九个真实 Seatbelt smoke 用例保持显式忽略，不计为已执行。
- 执行成功总数：**1791**，库/Seatbelt 中 10 个用例被忽略。未打扰任何在运行的正式 agent。

更早的一次完整序列化批次也在 15:45:13 通过（1743 个库成功加上相同的集成套件）。更早的开发运行分别为 1738 和 1725 个库成功。两个开发期失败已处理、未隐藏：(1) 在有意收紧 Manual 策略后修正了遗留的旧 `ls` 豁免断言；(2) 一个渠道中断已消耗其信号，使新原因选择错误地将其称为 provider 错误——增加了显式的流中途中断状态并保留原中断预期。最终工具索引守卫同样将 run 标记为 incomplete，而不只是发出错误。

### 并行缓存测试调查

第一轮在 `registry_injects_future_models_from_disk_cache` 中有一个真实的并行失败；串行成功并不能反驳它。源码检查发现其缓存重置发生在**等待 TestHome 的全局 HOME 锁之前**。其他 registry 测试可在等待期间填充共享内存缓存，因此“冷缓存”断言读到了另一个测试的目录。将重置移到 HOME 获取之后，与其他缓存测试的顺序一致。此后默认并行开发运行与两次完整序列化批次均通过。这修复的是已识别的测试隔离窗口；并不声称证明所有可能的全局缓存竞态都不可能。

## coverage——全部 79 个分配原始发现

下方所有路径均相对于 worktree。`Fixed` 表示实现与所列回归在最终 macOS 批次中通过，除非显式标注 native-only。`Native pending` 不是原生通过。`Latent`/`not established` 不计为已修复的生产 bug。本分区未分配 SF/NF ID。

| ID | 最终处置 | 证据、实现、回归与剩余限制 |
|---|---|---|
| w01-BUG-1 | 已修复；原生缺口 | `rpc/session.rs` 检查截止时间直到进程与读取器完成；有界捕获输出；取消代际终止直接 shell RPC。`commands/settings.rs` 在执行前释放会话锁。通过 `execute_shell_bounds_inherited_pipe_lifetime`、`execute_shell_snapshot_can_be_cancelled_without_session_lock`、现有组杀测试与实际的 `shell_rpc_releases_session_lock_and_abort_stops_process`。逃逸的后代进程可保留分离的读取线程直到 EOF；不再有无界 join。 |
| w01-BUG-2 | 已修复 | 会话保留注入的 `GlobalQueueBudget`；恢复时使用它而非无限队列。`hydrated_scheduler_keeps_injected_global_budget` 加载真实保存的元数据，证明第二个排队请求命中注入的全局容量 1。 |
| w01-BUG-3 | 已修复 | `get_session_stats` 使用真实计数器/成本/SQLite 路径。`session_stats_empty` 现在同时检查非零输入/输出总量 168、成本 0.75、实际数据库路径。缓存输入不会二次计入总量。 |
| w01-BUG-4 | 反驳为当前生产 panic | 在当前可达调用方中，setter 与 loop 锁持有者由外层会话锁串行化。未发现该锁之外存在竞争的持有者。未改为在 try-lock 失败时静默丢弃设置。潜在的锁顺序脆弱性不是生产 panic。 |
| w01-BUG-5 | 已修复 | 折叠投影携带最新 event_id/timestamp/run_sequence 及 idx。`projection_preserves_semantic_order_while_coalescing_deltas` 对照折叠游标检查身份后缀以及文本/顺序。 |
| w01-BUG-6 | 修复且无数据丢失 | 有界事件队列现在应用阻塞背压，而不是把瞬时 Full 标记为永久持久化失败。写入线程不获取广播器/会话锁；断开的写入器/存储错误仍保持 fail-closed。`transient_full_event_queue_backpressures_without_failing_health` 在队列满时执行实际追加，验证事件有序且状态健康。现有日志失败/持久批处理测试通过。慢存储仍可能阻塞生产者，与现有持久化边界一致；未引入增量丢弃。 |
| w02-BUG-1 | 已修复 | 移除程序基名豁免。env/wrapper 在 Manual 下总是询问。通过 shell 命令表与门禁测试；未执行任何命令来演示绕过。 |
| w02-BUG-2 | 已修复 | 任意 shell 文件读取均询问；没有猜测 shell token 的秘密解析器。秘密/绝对/父目录/变量/PowerShell 读取由命令表覆盖。路径感知的读取工具仍然可用。未读取真实秘密。 |
| w02-BUG-3 | 已修复 | Manual git 命令询问；分支/标签/远程/reflog 变更用例已覆盖。避免不完整的逐选项黑名单与 git 辅助程序执行假设。 |
| w02-BUG-4 | 已修复 | sort/uniq/find/date/hostname/yes/seq 不再豁免。外部 `ls` 也询问：其基名可被 PATH 遮蔽。仅精确的内置 `pwd` 保持自动；配置的 shell 启动/环境仍受信任。显式完整权限与实际 OS 包裹行为不变。 |
| w02-BUG-5 | 已修复 | 门禁忽略显式 null 可选权限，匹配处理器的 Option 语义。现有空写入测试现在通过 `ApprovalGate::request` 显式测试 null。非法非 null 载荷仍失败。 |
| w02-BUG-6 | 已修复 | `shorten_home` 使用组件感知的 strip_prefix。现有 home 测试加上兄弟前缀断言确保 `<home>-sibling/file` 不会被缩写成误导性的持久化建议。 |
| w02-BUG-7 | 已修复 | HTML 元数据在插值前转义。`generate_session_html_escapes_content` 现在提供恶意 title/model/cwd 以及消息内容，并禁止原始 script/img 标记。 |
| w03-BUG-1 | 已修复 | 只要 scheduler.active 存续，入队即等待，越过控制/任务槽清理。确定性 `enqueue_prompt_defers_until_scheduler_completion_delivery` 通过，保留新请求而非在 ActiveRunExists 时取消它。 |
| w03-BUG-2 | 已修复 | 系统提示采用已接受的 model/thinking，而非实时会话字段。`queued_run_prompt_uses_accepted_model_and_thinking` 在入队后更改实时设置并检查实际出站 ModelRequest。 |
| w03-BUG-3 | 已修复；消费者集成注记 | `LLMProvider::snapshot` 是增量式的，带有不可变 provider 默认实现。生产 Client 克隆可变代际锁，同时保留实时模型注册表权威。Loop 快照消费它。私有代际变更测试与实际 HTTP `provider_snapshot_sends_frozen_thinking_on_the_real_http_path` 均通过；排队提示测试覆盖准入路径。自定义可变 provider 必须实现 snapshot。直接消费者检查属于 supervisor 集成。 |
| w03-BUG-4 | 已修复 | 词法规范化保留未解析的相对父级，且绝不越过绝对根。回归覆盖 ../x、../../x 与 a/../../x，经实际辅助函数。 |
| w04-BUG-1 | 已修复 | Glob 编译器按字符而非 UTF-8 字节迭代。实际 RuleSet 测试使用 CJK 工作区/规则名并验证秘密 Read/Write Ask、用户 Deny 与单字符通配语义。 |
| w04-BUG-2 | 已修复；原生待验证 | 修正 V1 根因：原生 Path 组件防止重复的绝对余量/前缀；Windows 正则模式/目标分隔符与大小写处理一致。共享 RuleSet 回归在 macOS 通过；仍需原生 Windows 执行。 |
| w04-BUG-3 | 已修复 | 引号剥离要求至少两个字节。纯实现在所有主机上编译以供测试；`malformed_powershell_wrapper_quotes_never_panic` 覆盖孤立引号与 wrapper 形式。建立切片安全性无需 PowerShell 进程。 |
| w04-BUG-4 | 已修复 | 规则解析规范化大小写/边缘空白，拒绝未知字段而不扩大访问。逐规则诊断到达 RuleSet resolution_errors，同时有效规则存活。实际文件 `invalid_rule_fields_are_diagnosed_without_losing_valid_rules` 验证 READ/deny-space 仅适用于读取且两个未知规则均被诊断。 |
| w04-BUG-5 | 外部/原生行为不明；无补丁 | 未观察到实际本地化 Windows 拒绝文本/原因。仅非零状态不能授权无沙箱重试，且 access-denied 可能是普通 ACL 失败。不注入虚假沙箱标记，也不把每个错误标记为沙箱违规。在定义分类器行为前需要受控的原生拒绝证据。 |
| w04-BUG-6 | 已修复 | Tilde 拼接裁剪前导平台分隔符，使 ~//x 保持在 home 下。实际辅助函数回归通过。 |
| w04-BUG-7 | 已修复；原生待验证 | 平台分隔符谓词在 Windows 上处理 ~\\x，在 Unix 上保留字面反斜杠。Windows 断言存在于跨平台辅助函数测试中，但未在 macOS 上执行。 |
| w04-BUG-8 | 已修复 | Seatbelt 使用选定的 `unix_shell()`；工具描述遵循宿主 shell 提示，无虚假 bash 声明。`seatbelt_uses_the_same_interpreter_as_plain_shell` 验证真实准备的 argv。 |
| w05-BUG-1 | (a) 已修复；(b) 未确立 | Windows 计划省略完全被更早字面写入规则覆盖的更低字面子树，因此被遮蔽的 deny 不会变成无条件 ACE。精确/子节点挖除回归在平台中立计划测试中通过。原始 (b) 假设列出回退可写根会覆盖更高的用户 deny；规则引擎并未承诺这一点。物理继承 ACL 行为仍需原生验证；未应用任何全盘移除 deny。 |
| w05-BUG-2 | 已修复 | 共享 MAX_BWRAP_ARGS=9000；MAX_MOUNTS 对每个不透明目录挂载使用最坏情况四个参数加 32 个固定参数余量，而非报告中错误的三参数通用算术。`oversized_literal_mount_plan_fails_before_helper_execution` 构建实际文件系统/规则输入并在 helper 启动前得到 MountLimit。 |
| w05-BUG-3 | 已修复 | 失败的 Linux 探测只缓存 5 秒，以 PATH/工作区/cwd 为键；上下文变化绕过缓存，过期重试。假主机回归驱动实际缓存/探测实现，证明缓存期间无子进程调用，随后成功重试。失败不会被永久缓存。 |
| w05-BUG-4 | 潜在，无生产补丁 | 当前没有生产者为同一 capability/object 产生 GRANT + FILE_GENERIC_WRITE + INHERIT_ONLY。现有仅子 ACE 授予 DELETE 而非 WRITE。仅凭 mask 观察不能确立当前可达的拒绝 bug。 |
| w05-BUG-5 | 不安全契约加固已修复；原生待验证 | TOKEN_USER 使用机器字对齐存储；OwnedSid 在 clone/everyone/派生路径中使用 u32 对齐字，保留精确 SID 字节。更新的 Windows 字节布局/对齐断言存在。未声称实际分配器引发的崩溃；原生 Windows 编译/测试未完成。 |
| w06-BUG-1 | 潜在公共 API 用例；无生产补丁 | SQL NULL 时间戳机制为空的低层保存存在，但当前 RPC 持久化提供带日期的会话/run 条目，且空的旧导入被拒绝。未确立该状态的生产创建者。与畸形数据韧性改进分开保留。 |
| w06-BUG-2 | 已修复 | 重新验证将 source_changed/source_unreadable 分类为逐会话跳过导入，而非中止无关启动。绝不导入过期字节；目标事务错误仍传播。实际文件消失/变更回归与完整 SQLite 导入/启动套件通过。 |
| w06-BUG-3 | 在可达 RPC 边界修复 | cmd_fork 在创建子会话前检查所请求条目存在于已加载父会话。RPC 回归尝试不存在的点、检查失败/无新会话，然后合法 fork 成功。旧公共 fork 辅助函数保留其文档化回退；没有可达 RPC 静默使用该回退。 |
| w06-BUG-4 | 已修复 | 恢复是 FIFO worker 命令，带确认；只有成功的有序恢复才清除先前错误。持锁 worker 回归验证接受的追加先于终态；现有 degraded/close/commit 测试通过。 |
| w06-BUG-5 | 潜在低层诊断用例 | 缺失 ID 的 assistant/tool 可通过畸形公共原始值产生 SQL 错误；生产序列化与导入验证提供 ID。不是已观察到的生产泄漏；无推测性重写。 |
| w07-BUG-1 | 已修复 | 修复将迟到的真实工具结果迁移到调用它的 assistant 的响应窗口内，保留条目身份；不只是移除相邻占位符并留下非法顺序。停在稍后声明同一 call ID 之处并尊重已知 run 归属。实际 Manager 保存/加载回归验证 assistant 之后恰好一个真实结果；共享对账测试通过。 |
| w07-BUG-2 | 重复 / 潜在 | 与 w06-BUG-1 相同的 NULL 时间戳机制；不是第二个已修复的生产缺陷。 |
| w07-BUG-3 | 已修复 | 使用现有 unicode-width 0.2.2 标量宽度表；回归覆盖 ✅、🚀 与半角片假名。近似的标量预览已文档化，不声称是完整字素布局引擎。 |
| w07-BUG-4 | 潜在 / 上游输入未确立 | 跨 run call-ID 复用未对当前 provider/调用路径确立。不基于假设的畸形转录做全局去重语义重写。 |
| w08-BUG-1 | 已修复 | 解码器边界包含未完成事件字节（含行开销），逐事件重置。实际多行回归通过；许多空数据行无法绕过内存记账。 |
| w08-BUG-2 | 狭义修复 | 重复 finish 仍交付新提供的用量；无重复 Finish，不越过权威 [DONE] 读取。幂等/用量回归通过。DONE 之后尾随的非标准成本不足以保持已完成的传输打开。 |
| w08-BUG-3 | 已修复 | Responses 接受整数/浮点/字符串 token 字段（包括 detail 计数），保留 credit_cost，推导缺失的总量。实际适配器回归通过。USD cost/estimated_cost 有意不作为平台信用别名处理。 |
| w08-BUG-4 | 已修复 | 消费者在部分历史定稿前清除 pending（非已完成）工具调用上 provider 拥有的 id/item_id。保留无关元数据与可见内容。实际中断流回归验证持久化历史中没有未完成的 provider 身份。拒绝仅生产者的 drop-after-finish 安慰剂。 |
| w08-BUG-5 | 修复阻塞运行时问题 | 图像准备在 spawn_blocking 中运行，位于异步运行时 worker 之外；现有真实图像/HTTP 测试通过。重复路径读取有意保留，因为附件是实时引用；未引入无界或过期图像缓存。单次解码仍受现有输入/分配边界约束，未声称可中途取消。 |
| w08-BUG-6 | 已修复 | Anthropic EOF 在 Incomplete 之前按确定性索引顺序排干开放块。不完整的思考不会从部分签名获得重放权威。显式开放 text/reasoning/tool 回归与现有传输测试通过。 |
| w09-BUG-1 | 已修复 | 每个独立 loop 获得自己的检查点单元；仅显式 run/会话共享保留。隔离指针测试与会话/run 压缩测试通过。 |
| w09-BUG-2 | 已修复 | 引擎构造函数将配置的 max_turns 应用于 Loop。配置回归断言 7，现有运行时回合限制测试通过。 |
| w09-BUG-3 | 已修复 | 所有消费者工具事件索引 >=256 在 UI 转发/调整尺寸前被拒绝；错误也将 run 以 invalid_tool_index 标记为 incomplete。实际脚本化的 usize::MAX/256 回归验证无工具历史与正确终态分类。 |
| w09-BUG-4 | 外部语义不明；无计费补丁 | 没有权威的多用量 token fixture 来确立累计 vs 增量值。成本语义不能证明 token 语义。不基于推断静默更改计费/记账。 |
| w09-BUG-5 | 已修复 | 断连重试耗尽产生 connection-interruption 原因与最终 tool-end，而非过期的“retrying”或用户中断文本。六断连实际 loop 回归与渠道/标志中断用例通过。 |
| w10-BUG-1 | Agent 已修复；调用方工作已委托 | 限定查找保持权威，随后裸精确 ID 可含斜杠。回归区分共享 family/model 的两个 provider 与唯一裸斜杠回退。歧义调用方引用必须完全限定；fix-apps 拥有桌面/移动端修正并报告交付。集成必须合并两个分支。 |
| w10-BUG-2 | 已修复 | 规范 agent/auth.json 先于旧 agent-app；规范空对象不会复活密钥。隔离 HOME 回归通过。两份 directory-layout 语言文档已更新。不可加载的规范文件保留文档化的旧回退。 |
| w10-BUG-3 | 已修复 | 验证所写内容本身；拒绝前导/尾随空白 ID，而非验证修剪后内容、写入未修剪内容。Provider 验证回归通过。未对用户既有配置文件做未经授权的编辑。 |
| w10-BUG-4 | 已修复 | 加载器区分省略（0 哨兵）与显式 128000；补全只填缺失的限制，并为未知模型提供常规回退。显式 128k 回归与加载器/目录测试通过。 |
| w10-BUG-5 | 已修复 | 仅当本次调用修改了 models 时，auth 写入失败才恢复 models。专用 Unix 失败注入测试证明未更改的 models inode 在失败的 auth 删除后存活；现有回滚测试通过。 |
| w10-BUG-6 | 委托给 fix-apps | ROUND2 将生成器、内置输出 schema/转换与默认选择过滤分配给 fix-apps。阅读其更新的交付账本；精确重叠为 `agent/src/models/mod.rs` 内置转换/默认选择/测试加上 `models/builtin/mod.rs`。本分支不计为实现该补丁。Supervisor 必须集成/复检。 |
| w10-BUG-7 | 已修复 | 畸形 auth 条目警告时不含解码器/凭据值。日志捕获回归验证警告/provider 名、保留的有效条目、无秘密哨兵泄漏。 |
| w10-BUG-8 | 已修复 | 检查正 i32 转换；实际转换回归覆盖负数、零、30 亿与有效 64000。 |
| w11-BUG-1 | 已修复；原生待验证 | PowerShell 安全的 ASCII base64 字面量按 UTF-8 解码并管道给间距正确的 CLI --stdin；后缀保留；支持双单引号；复合程序不重写。实际重写/解码回归保留 Unicode/撇号/逗号与标志。存在原生 PowerShell 执行测试但未在 macOS 运行。Windows 命令行长度限制仍是平台约束。 |
| w11-BUG-2 | 已修复；原生待验证 | Windows shell 在原始超时内等待真实进程退出；返回真实状态；超时的部分输出是信号失败而非退出 0。新增原生 exit-7/timeout/后代测试；未在本机执行。 |
| w11-BUG-3 | 已修复 | Edit 容忍 read 对 CRLF 的 LF 视图并保留未触碰字节/CRLF 插入。实际 read+edit 处理器回归通过。混合行尾插入使用现有 CRLF 存在性；无全文件规范化。 |
| w11-BUG-4 | 已修复 | 递归 rm 根匹配前，绝对 home 比较是平台感知的，包括 Windows 驱动器路径。纯守卫测试使用解析后的 home 而不运行 rm。现有允许的项目目标测试通过。 |
| w11-BUG-5 | 已修复 | 顶层 frontmatter 提取忽略嵌套键。显式嵌套 name/version 回归与技能套件通过。 |
| w11-BUG-6 | 已修复 | MIME 来自解码器实际识别的格式，而非猜测后缀/默认 PNG。真实 BMP 字节以 attachment.bin 存储，在回归中产生 image/bmp。 |
| w11-BUG-7 | 已修复 | 单次与批量模式共享首次出现替换。重复旧文本/CRLF 批量回归验证首个匹配且后续出现完好。 |
| w11-BUG-8 | 通过消除临时文件修复 | 参数重写现在在内存中携带数据；没有创建路径或清理时机可在成功/错误/取消时泄漏 JSON 文件。重写回归断言不存在临时文件/重定向形式。 |
| w12-BUG-1 | 已修复 | 看门狗保留已调度的 run_sequence。暂停时钟测试使用 begin_scheduled(7)，按预期到达 CancellationStuck。 |
| w12-BUG-2 | 已修复 | 共享的逐字符四分之一 token 成本同时驱动上下文估算器与摘要分块拆分。CJK/西里尔/emoji/混合文本回归验证相同单位与有界、无损分块。 |
| w12-BUG-3 | 提议的不变量被反驳；边界已文档化 | 对 window<=1 不存在同时满足 reserve>0 与 reserve<window 的整数。当前结果不虚构容量；在保留有效窗口测试的同时新增显式 [-1,0,1,2] 边界回归/文档。语义摘要预算准入拒绝不可用的小容量；未演示重复付费压缩循环。 |
| w12-BUG-4 | 潜在构造函数不一致 | 当前生产图像调用方在 new_user 前过滤空 URL。未确立可达的畸形 provider 请求；无推测性数据形态变更。 |
| w13-BUG-1 | 修复 shell/审批关闭路径 | 可取消的无锁 shell RPC 与关闭审批取消消除了已演示的锁死锁。实际 RPC 并发/中止回归与隔离 CLI SIGINT smoke 通过。这些是组合证据，不是新的端到端长 shell 期间 SIGINT 捕获。无关的阻塞文件系统操作仍可能延迟运行时关闭。 |
| w13-BUG-2 | 已修复 | 可失败的 IP/SocketAddr 验证；支持方括号 IPv6，主机名以可操作的错误拒绝而非 panic。单元加上实际 CLI 非法地址测试通过。 |
| w13-BUG-3 | 已修复 | 非法/溢出/缺失端口不能回退到 50051。用精确退出 1 + 非法地址诊断 + 五种非法形式无监听日志取代接受退出 0 或 1 的弱 smoke 测试。 |
| w13-BUG-4 | 已修复 | --verbose 启用 debug 默认过滤器；显式 RUST_LOG 保持权威。实际 tracing 启用事件回归验证 debug 在默认模式下关闭/开启。 |
| w13-BUG-5 | 已修复 | 提供的非法沙箱层级在策略转换前返回 tonic InvalidArgument。线上边界回归覆盖 strict、混合大小写与空值。旧内部解析器未改动。 |
| w13-BUG-6 | 委托协议契约 | ROUND2 将 Base64/data-URI 注释/编码契约分配给 fix-cli，调用方评审分配给 fix-apps。本分支没有猜测 image/* MIME 或做无关 RPC schema 变更。Supervisor 必须对账委托结果。 |
| w25-BUG-1 | 已修复；原生待验证 | Job 仅在正常完成的等待后解除武装；超时/读取失败保留 close 即杀。原生测试检查非零页脚且仅终止其自身衍生的后代。不是 macOS 原生通过。 |
| w25-BUG-3 | 已修复 | abort_retry 在中止后取消审批发送方。现有 RPC 测试现在有真实的待处理审批接收方，验证 Cancelled 送达与空待处理列表。 |
| w25-BUG-6 | 机制已修复；原生待验证 | Mutex 串行化 get/probe/set，保留瞬时失败上的重试而非永久缓存。未执行原生并发主机探测；源码锁/OnceLock 身份已评审。 |

## gaps and integration contract

1. **外部证据仍真正缺失：** w04-5 Windows 拒绝归因/本地化与 w09-4 渐进 token 语义。这些未被声称是已修复的生产缺陷。w05-1(b)、w06-1/5、w07-4 与 w12-4 有上述较窄的可达性/不变量处置，而非笼统“已确认”。
2. **原生 Windows 验证未完成：** 运行 agent 原生测试，尤其是 `powershell_executes_rewritten_stdin_with_unicode`、`windows_shell_reports_exit_status_and_timeout_kills_descendants`、SID 布局/对齐、glob/tilde 与主机探测路径。macOS clippy 无法对 cfg(windows) 体做类型检查。未安装重型工具或使用真实凭据来伪造原生证据。
3. **跨负责人集成：** fix-apps 拥有完全限定的桌面/移动端模型引用与 w10-6 生成器/内置读取器/默认过滤器。本分支在同一 models.rs 文件中拥有解析器、缺失窗口哨兵与缓存测试顺序；合并时需谨慎。fix-cli 拥有 w13-6 与 packages/rpc。此处未更改 proto 字段编号。
4. **直接消费者：** 增量式 LLMProvider::snapshot 默认实现避免强制不可变 mock 变更，但可变自定义 provider 必须实现它。Supervisor 应在集成后测试 CLI/桌面消费者。Cargo.lock 变更仅是 agent 对已锁定 unicode-width 0.2.2 的依赖边。
5. **保留的范围限制：** 标量宽度预览不是完整字素布局；单次图像解码不能中途取消；直接 shell 逃逸后代可保留分离读取线程；有界日志背压可能在慢存储上等待。其中任何一项都未被隐瞒为对所有可想象机制的已验证治愈。

## New evidence vs round 1 / rejected approaches

- 第一轮停在缺少已确认实现与集成检查的状态。第二轮提供真实的全 crate 并行通过、针对性回归、有源码支撑的较窄处置，以及其余已确认范围内缺陷的工作实现。
- 未静默丢弃日志增量、猜测 USD/信用别名或 token 累积语义、从任意失败推断沙箱拒绝、在消费者消失后调用生产者清理，或在留下非法消息顺序的情况下于插入占位符后做全局去重。
- 未单独修复 Windows 字符串间距却留下不受支持的 PowerShell `<` 重定向或永久临时文件泄漏。
- 未把串行缓存测试成功归类为基线 flake 的证明；修复了其已识别的重置/HOME 锁顺序并验证了默认并行运行。
- 无 push、PR、分支合并/重置、额外 worker、其他 worktree 编辑或用户 agent 干扰。只有 supervisor 拥有独立验收、集成、最终 PR/auto-merge/CI 与清理。

Next useful check: 独立评审本分支与委托的 models/proto 集成，随后是原生 Windows 执行与直接消费者测试。完整本地 Rust 批次可用上述精确命令复现；外部语义需要证据，而非更多盲目的实现。
