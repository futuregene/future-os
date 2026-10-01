# 自我认知（Self-inspection）

能观察自身的 Agent，才不必让用户反复解释它已经记录过的事情。FutureOS 本就把需要
知道的东西都记了下来——Agent 的配置、它能做什么、以及它与用户的每一次对话——
并通过同一个 `future` 命令行暴露出来。这里没有单独的「自省 API」，也没有新增模型
工具：Agent 用自己普通的 `shell` 工具读取自身，所以同一批命令对人也可审计、可复现。

内置的 `future-self` 技能是这些命令之上的 Agent 侧入口：它说明哪些读取是安全的、
每一次读取的边界在哪里、以及哪些改动必须先取得用户同意。本页记录命令本身。

## 可以读到什么

| 命令 | 报告什么 |
|---|---|
| `future config get [<key>] [--json]` | 生效中的全局设置（含默认值） |
| `future doctor` | 一次检查登录、Agent 连接、沙箱、provider、会话与技能 |
| `future models --json` | 本 Agent 可用的模型 |
| `future auth status` | 是否已配置平台登录（登录到哪个平台） |
| `future account profile` / `balance` | 用户账户与剩余额度 |
| `future skills list` | 已安装与目录中的技能 |
| `future tools list` / `describe <name>` | 工具面，含参数与示例 |
| `future session list --json` | 全部已记录会话（新的在前） |
| `future session info <id>` | 单个会话的模型、cwd、消息/工具计数、token 与成本 |
| `future loop status` | 当前项目目录下的长程目标 |

`future config get` 用与 Agent 相同的类型化加载器读取
`~/.future/agent/settings.json`，因此它报告的是 Agent 真正会生效的值，而不是文件里
字面写了什么：文件里省略的键依然显示其文档化默认值。带 key 时只打印该值，便于在
脚本中使用。

凭据是刻意的例外。`auth.json` 从不属于这个面：`future config get` 不含任何密钥
材料，设置文档里也没有存放密钥的字段。

## 阅读实现本身

上面的命令回答的是「配置成什么」，源码回答的是「为什么这样表现」。当行为出乎意料、
某个设置的效果不清楚、或准备断言某个边界时，答案通常就在实现里——而仓库自带指路
信息：

| 位置 | 提供什么 |
|---|---|
| `CLAUDE.md` | 工作区结构：哪个 crate 负责什么，以及它对应 `~/.future/` 的哪一部分 |
| `docs/README.md` | 文档索引——指南、架构、内部文档 |
| `docs/guide/`、`docs/architecture/` | 已发布行为，以及子系统背后的设计 |
| `FUTURE.md` + `.future/memory/` | 此前会话记录下来的经验与坑 |
| `packages/rpc/proto/future.proto` | RPC 线上契约（唯一事实来源） |

常见问题基本对应到单个文件：CLI 命令面在 `cli/src/commands/` 与
`cli/src/help.rs`，命令分发在 `agent/src/rpc/commands/mod.rs`，设置语义与默认值在
`agent/src/config/mod.rs`（再用 `rg` 找该字段的消费方），系统提示词组装在
`agent/src/prompt/mod.rs`，技能在 `agent/src/skills/`，会话与历史存储在
`agent/src/session/`。

有两个注意事项比这张表更重要。运行中的 Agent 是一个已构建的二进制，所以源码结论是
针对这份检出的代码，而不是正在回应你的那个进程——把行为归因于刚读到的代码前，先看
`future --version`。另外，检出目录里可能有其他会话尚未提交的改动：可以读，但不要在
别人的工作区里提交或 reset。

阅读源码是为了理解，不是为了让行为改变。有开关的行为走 `future config set`、
`future session set` 或技能；其余的是应当上报的 bug——本地改动会让这套安装与用户
实际运行的发布版分叉，并在下次更新时消失。

## 读取用户与 Agent 的记录

`future session history` 是只读的回忆面（分页细节见
[会话历史回忆](session-history.zh-CN.md)）：

```sh
future session history search --session <id> --query "text" [--limit 5] [--json]
future session history search --all --query "text" [--limit 5] [--sessions 50] [--json]
future session history get --session <id> --entry <entry-id> [--offset N] [--limit N] [--json]
```

`--all` 是「这段对话说了什么」与「我们以前是否处理过这件事」之间的差别。它跨会话
检索，扫描最近更新的 `--sessions` 个会话（1..500，默认 50），并给每条命中标注
`sessionId`。由于扫描有界，响应会说明它覆盖了多少历史：

- `scannedSessions`——实际扫描了多少个会话；
- `truncated`——是否还有更早的会话未被检索；
- `hasMore`——在 `--limit` 之外是否还有更多命中。

忽略这些字段的调用方，会从不完整的检索中自信地得出「我们从未讨论过这件事」。
技能要求相反：只有两个标志都为 false 时才做否定判断。

检索是字面子串匹配（ASCII 大小写不敏感），覆盖用户与助手的文本、工具参数与工具
结果——不是语义检索，也不包含思考内容。检索无结果时，通常问题出在查询词本身：
应该细化查询，而不是断定记录为空。

## 修改设置

```sh
future config get                                   # 全部设置（含默认值）
future config get defaultPermissionLevel            # 单个值
future config set defaultPermissionLevel workspace  # 改一个键
future config set compaction.reserve_tokens 8192
```

`future config get --help` 会列出所有可设置的键及其取值。写入前先校验取值：非法取值
或未知键会让 `settings.json` 保持原样。`set` 就地在 JSON 文档上改一处，因此新版
写入的键——或手工添加的键——不会被旧版的改动抹掉。

写入不需要 Agent 在运行，运行中的 Agent 也不必被通知：设置在用到时才从磁盘读取。
所以延迟在「何时用到」：

| 键 | 何时生效 |
|---|---|
| `defaultModel`、`defaultPermissionLevel` | 下一个新会话 |
| `compaction.*`、`retry.*`、`maxTurns` | 下次 Agent 启动 |

会话级设置仍用 `future session set <id>`（`--model`、`--thinking`、`--cwd`、
`--title`），它对运行中的会话立即生效。能力面用
`future skills install|uninstall|update` 调整。

改设置会改变此后每一个会话的行为，所以技能把它当作用户的决定：说明旧值与新值，
在用户同意后再改。

## 值得写明的边界

- **字面匹配，不是语义检索。** 没有向量、同义词或词干处理；两个词就是一个子串。
- **有界。** 单次检索最多 20 条命中，跨会话扫描最多 500 个会话，单次条目读取
  4..32768 字节。
- **只有原始记录。** 思考内容按设计被排除，媒体主体与 provider 元数据被省略，
  压缩摘要不对外暴露——所以读到的对话与模型当时看到的并不逐字节相同。
- **本地、单机。** 另一个 `FUTURE_HOME` 下、或另一台设备上的会话不可见。
- **不含凭据**：读取、摘要，以及代用户写出的任何内容里都没有。

## 另见

- [会话历史回忆](session-history.zh-CN.md)——检索/读取语义与按字节分页。
- [目录结构](directory-layout.zh-CN.md)——`~/.future/` 下各部分的归属。
