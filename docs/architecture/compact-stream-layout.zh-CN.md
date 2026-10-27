# 格式 2 紧凑变更存储契约

日期：2026-10-27。英文：[Compact stream layout](compact-stream-layout.md)。
关联：[开发计划](stream-storage-development.zh-CN.md)、[空间基线](stream-storage-space-baselines.zh-CN.md)。
本页记录实施契约；验证结果另行登记，不以实现状态代替验收。

## 布局与发布边界

Agent schema 8 直接修订未发布目标，逻辑会话格式仍为 2。`stream_layout` 的单例值为
`compact-owned-stream-v1`。没有旧候选转换、双编码、双写或历史正文压缩。
已发布 schema 2–7 及 Desktop 已发布迁移 SQL/校验和不改动；旧会话继续使用格式 1。
打开已为 schema 8 的库时，先检查标记、权威列、索引定义、外键和回收触发器，再配置 WAL 和执行初始化。
旧开发库不匹配时拒绝并保留文件；测试使用隔离数据根，不能清空含真实历史的个人数据库。

精确 DDL 位于 `agent/src/session/content/schema.sql`，编解码位于同目录 `compact.rs`。

## 业务身份与历史兼容

新格式的自产会话、消息和运行 UUID 用完整 16 字节值存储，接口和只读检查视图输出标准小写 36 字符。
已发布历史 TEXT 原样保留，包括看起来像 UUID 的字符串；提供者 opaque ID 不改拼写。
历史 SQL 列声明仍可为 TEXT，实际存储类别以 `typeof` 为准；不能据声明或字符串长度猜测键的表示。
主外键绑定通过 `session/identity.rs` 统一处理，内部流/上下文/符号引用继续使用整数。

会话身份解析同时检查会话、删除墓碑和历史导入登记；同一公开 ID 的 TEXT/BLOB 歧义明确失败。
新会话创建在同一事务核对身份占用后才选用 UUID BLOB，已有会话继续原格式。
旧 JSONL 恢复遇到已有新格式会话时保留源文件并跳过导入；在提交事务内重查会话与墓碑，
不能创建第二个同名身份、覆盖已提交正文或复活已删除会话。
`sessions_inspect`、`entries_inspect`、`runs_inspect`、`stream_symbols_inspect` 仅计算可读投影，不另存 UUID 字符串副本。

## 不可变上下文

`stream_contexts` 用整数 ID 表示一个运行/epoch 内的稳定描述符：
`[稳定元数据对象, 单字段路径或 null, 单字段编码或 null, 符号引用对象, 事件码]`。
稳定白名单为顶层 `run_sequence`，以及 data 中的 `type`、`entry_id`、`tool_id`、`tool_name`、
`tool_type`、`tc_index`、`content_key`、`block_id`。不存在和显式 null 保持区别。
字段变化创建新上下文；数据库触发器拒绝 UPDATE。描述符最大 16 KiB。

查找使用 `(stream_id, SHA-256)` 索引，复用前比较完整描述符，不能只比较摘要。
碰撞候选有数量上限；发生超限明确失败，不猜测身份。变更的复合外键保证 context、stream 和事件码一致。
不设置跨事务的短 ID 缓存，因此回滚分配的 ID 不会泄露给后续事务；SQLite prepared-statement 缓存只保存语句。

`stream_symbols` 在每条流内保存去重的 entry_id、tool_id、content_key、block_id、tool_name 和 tool_type。
规范 UUID 无损保存为 16 字节，其他串保持原始 TEXT；符号 UPDATE 被拒绝。上下文只保存整数符号引用，复合外键验证每个引用属于同一条流。描述符与还原后符号字节分别受 16 KiB 上限约束。
长标识不再重复进入每个上下文和定位索引；变更索引用短 ID 定位。
`stream_contexts_entry/tool/key` 支持源消息、工具和正文键定位，`stream_changes_context`
支持 context、事件类型和游标联合定位。源定位索引仅覆盖相应事件码，避免为每个增量维护无用的定位项；定位查询先筛上下文，再按变更索引查找。

## 紧凑变更和范围

`stream_changes` 保存 stream/context ID、起止游标、显式事件码、剩余元数据、范围和存在性位。
`metadata_json` 是剩余 JSON，不能当成完整协议事件；使用统一编解码器还原后再消费。
原始时间戳字符串保留在剩余 JSON，不缩减精度。未知事件的码为 0，原类型保存在 `unknown_kind`。
`event_type` 是由码和未知类型计算的虚拟列，不存储重复字符串。

| 码 | 事件 | 码 | 事件 |
|---:|---|---:|---|
| 1 | agent_start | 10 | thinking_end |
| 2 | agent_end | 11 | tool_start |
| 3 | message_start | 12 | tool_delta |
| 4 | message_end | 13 | tool_input_end |
| 5 | text_start | 14 | tool_end |
| 6 | text_chunk | 15 | tool_result |
| 7 | text_end | 16 | usage |
| 8 | thinking_start | 17 | model_changed |
| 9 | thinking_delta | 18 | tool_sandboxed |

单字段正文范围直接存在变更行，字段路径/编码属于上下文。位 1/2/4 分别表示可从范围或游标
还原的 `content_offset` / `content_bytes` / `content_first_idx`。只有原值与还原值完全相等才提取，
否则保留原值。其他字段（包括未知、false、null、空集合和 provider 不透明串）保留。
多字段事实使用 `stream_change_content`。`stream_content_ranges` 只是一层 UNION ALL 视图，
统一直接范围和扩展链接，不复制数据。终态的低频扩展也通过此视图读取。

## 原子发布与生命周期

正文暂存、水位和确认边界沿用分段方案。暂存归属使用 `stream_body_owners`，主键为整数 `(stream_id,key_id)`，复用流符号字典。
`[run,epoch,key]` 编码仅作内部临时适配，不进入归属表或索引。暂存可以提前保留流、符号和正文，但不推进持久游标。
发布事务在已持久化正文上原子提交变更、上下文、范围与水位。未发布 owner 保护暂存正文免于回收；删除 owner 排队回收，退役流分批释放。
重试逐项核验首游标、事件类型、完整还原元数据及所有范围/编码；不同数据明确冲突。
合并连续增量时，除范围派生字段和代表最后片段的顶层时钟外，其余元数据必须相同；变化的扩展事实不被覆盖。

范围解析仍以会话、run、epoch、游标、字段和数据库身份授权，整数 ID 不是权限凭据。
元数据页在同一读取快照捕获身份和水位；跨恢复代次失败，不拼接旧元数据与新正文。
回收与恢复均查询统一范围视图；删除直接范围有专用回收触发器和反向索引。
reset/prune 的退役处理分批删除变更、上下文、符号和 owner，不增加历史扫描或结束时整理。

## 有界分页

游标选择先估算剩余 JSON、描述符和还原符号的字节，沿用 16 KiB 页预算及已有单条例外，最多 256 条。
随后批量读取变更、本页去重上下文、扩展链接及去重符号；查询数不随行数线性增长。
数据库线程只返回原始字符串与范围，流式回放及恢复在调用线程解析每个上下文一次并组装事件。
原始选择器的总内存另有 256 KiB 硬上限，扩展字段和还原符号也计入预算。
终态/源消息等事务边界可以解码单个有界事实，不能读取正文或整轮历史。

正文段 16 KiB、写入 64 KiB、范围读取 256 KiB、25 ms/128 事件/64 KiB 调度、WAL+FULL、
独立 checkpoint 和背压规则保持不变。空间收益必须与迁移、故障、WAL、读取和响应数据一起验收。
