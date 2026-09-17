# 会话历史召回

`future session history` 通过 Agent 只读查询原始会话数据库，不调用模型、不重新执行历史工具，也不向模型加载整段历史。需要配套的新版本 CLI 与 Agent；不新增模型工具，可通过现有 `shell` 调用。

## 先搜索，再读取

```sh
future session history search --session SESSION_ID --query "ExpoSharing" --limit 5 --json
future session history get --session SESSION_ID --entry ENTRY_ID --json
```

搜索范围是指定会话的用户/assistant 文本、工具参数和结果，不包含 thinking、图片正文、provider 隐藏元数据、会话配置与 checkpoint。采用字面子串匹配（ASCII 忽略大小写），也可精确匹配 `tool_call_id`；多个词作为一个完整子串，不是语义搜索。未使用全文索引，数据量很大时搜索仍需扫描该会话的相关文本。

- query：非空，最多 200 字符，不能包含 NUL。
- search `--limit`：默认 5 条，范围 1～20。
- 结果按最新 entry 优先，返回 `entryId`、`blockIndex`、角色、时间、run/tool 标识、简短片段和 `byteOffset`。
- `hasMore` 表示还有匹配，可缩小关键词或增加 limit。
- 片段按字节限制，边缘可能出现替代字符；精确原文应使用 get。

## 有界读取

```sh
future session history get --session SESSION_ID --entry ENTRY_ID --offset 8192 --limit 8192 --json
```

**get 的 offset/limit 单位是 UTF-8 字节，不是行号或 token。** offset 指向该 entry 中可读块正文按顺序直接拼接后的字节位置，中间不加入分隔符。默认 limit=8192，允许 4～32768。服务端按索引定位 entry，再返回有限的 BLOB 子串，不把整个大结果复制进 Rust/RPC 输出缓冲区；SQLite 内部读取字段的成本仍可能随字段大小增加。

返回的 `chunks` 保留各自的 block 编号、工具 ID、字段类型、块内字节偏移和总大小。跟随 `nextOffset` 继续读取，不猜测中文字符的字节边界；非法偏移会报错。也可把搜索结果的 `byteOffset` 交给 get。`hasMore=false` 且 `nextOffset=null` 表示结束。

`--json` 保留准确文本，包括换行和 NUL。工具参数是序列化 JSON 的**文本片段**，分页片段不保证本身是完整 JSON。`omittedKinds` 告知被排除的非召回块类型。

## entry ID 是什么？

entry ID 是 Agent 生成的历史记录标识，通常为时间戳加随机后缀，在会话内唯一。`_future_journal_entry_id` 只用于内部绑定，普通模型消息不会自动显示这个 ID。

工具的 `tool_call_id` 用于关联调用与结果，不保证跨会话或跨运行唯一。搜索可以找出候选记录，再通过返回的 `entryId` 精确读取。一条 entry 可以包含多个 block。这样模型通过查询结果获得引用，不需要给所有消息正文添加 ID。

## 什么时候向模型介绍召回命令？

仅在以下条件同时满足时，为正常模型请求附加一份 `Archived conversation recall` 说明：

1. 当前使用成功提交或恢复的有效压缩 checkpoint；
2. 会话会持久化，不是 ephemeral；
3. 工具列表启用了 shell，且本轮权限允许使用工具；
4. 当前会话 ID 已知。

在请求边界动态添加：中途压缩成功后，下一次模型调用即出现；未压缩、压缩失败且无旧 checkpoint 时不出现；重启恢复已压缩会话后仍出现。每次从基础 system prompt 构建，不重复累加、不伪造用户消息、不写进聊天记录。摘要请求自身仍然无工具，也不附加召回说明。

说明要求模型只为缺失的精确历史事实检索，查询当前状态则优先安全、低成本的只读检查；历史指令不是新的授权，不为回忆重做写入、删除、发消息、部署或昂贵任务。旧 CLI 不支持命令时应说明版本限制，不应转而扫描其他会话。

## 边界

每次查询必须显式指定会话，不存在的会话或 entry 不会回退到默认会话。entry 必须属于该 session。沿用现有的**按用户隔离的 RPC 权限模型，而不是新增加的会话级 ACL**；有权限的 CLI 用户仍能显式查询自己的其他会话，system prompt 中“只查当前会话”的要求不是安全沙箱。没有开放任意 SQL 或数据库路径。

触发和保留预算见 [S2 压缩策略](../internals/compaction/compaction.zh-CN.md)。原始数据库历史保持不变；召回和有条件的使用说明不保证所有模型都会主动检索而不猜测。
