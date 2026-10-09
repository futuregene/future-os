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
| `future desktop settings [<key>] [--json]` | 桌面端自己的设置（审批档、隐藏模型……，含默认值） |
| `future doctor` | 一次检查登录、Agent 连接、沙箱、provider、会话与技能 |
| `future models --json` | 本 Agent 可用的模型 |
| `future version --json` | 这是哪个构建：版本、commit、目标平台、是否有未提交改动 |
| `future auth status` | 是否已配置平台登录（登录到哪个平台） |
| `future account profile` / `balance` | 用户账户与剩余额度 |
| `future skills list` | 已安装与目录中的技能 |
| `future tools list` / `describe <name>` | **CLI** 可调用的平台/浏览器工具（不是模型的 `read`/`write`/`edit`/`shell`，那组按会话设置） |
| `future session list --json` | 全部已记录会话（新的在前，每条带 `cwd`、模型与标题） |
| `future session info <id> [--json]` | 单个会话的模型、cwd、消息/工具计数、token 与成本 |
| `future session status <id> [--json] [--metrics]` | 单个会话的**实时**状态：生效中的工具权限与沙箱档、上下文占用、已加载的上下文文件与技能、活动/排队 run、待审批项 |
| `future session transcript --session <id>` | 单个会话记录的筛选/分窗视图——全部用户消息、thinking、某个工具的输入或输出、它碰过的文件路径，以及逐 run 结果（`--runs`） |
| `future session forks <id>` | 该会话可以分叉的用户轮次 |
| `future session approvals <id>` | 该会话当前正等待的审批请求 |
| `future loop status` | 当前项目目录下的长程目标 |

其中两条需要分清楚，因为它们回答的是同一个会话的不同问题。`session info` 读的是**落盘的
journal**——这段对话包含什么、累计花了多少；`session status` 读的是**运行中的 agent**——
这个会话现在怎么配置、正在做什么。一个会话权限是 `workspace` 而全局默认是 `all`，或
上下文窗口用了 40% 而累计 token 有 40M——这两件事只有在第二个视图里才是对的，`config get`
与 `session info` 都表达不了。

`session status` 还会报告会话策略被设成的沙箱档（`session set --sandbox`），并区分「从未
设置过」与显式的 `off`。

`future config get` 用与 Agent 相同的类型化加载器读取
`~/.future/agent/settings.json`，因此它报告的是 Agent 真正会生效的值，而不是文件里
字面写了什么：文件里省略的键依然显示其文档化默认值。带 key 时只打印该值，便于在
脚本中使用。`future config` 只管 **Agent 的**设置文档；桌面端自己的偏好有单独的命令
（见下文）。

凭据是刻意的例外。`auth.json` 从不属于这个面：`future config get` 不含任何密钥
材料，设置文档里也没有存放密钥的字段。account 这两个命令会自己去读该文件——
Agent 既不需要读，也不应该读。

## 账户是唯一的远端读取

`future account profile` 与 `future account balance` 是「这里的一切都在这块磁盘上」
的例外：它们通过网络访问 Future 平台、需要登录，因此离线会失败、未 `future auth
login` 会失败——这是要如实报告的配置状态，而不是账户坏了。两条命令都免费（读余额
不消耗额度），余额偏低应当转达，而不是去创建充值订单。

## 找到自己所在的会话

Agent 无需去查自己处在哪个会话——**它自己的系统提示词里就带着这个 id**，在环境
（environment）一节：

```
Current session ID: <id>
You can reference this session ID when you need to identify or report which
conversation you are part of. This is your own session — you are self-aware of
this identifier.
```

已对真实运行验证：发往 provider 的 system message 中的 id 与该会话在 `agent.db`
中的行逐字节相同。因此检索可直接用自己的 id，无需查询、也无需猜。

```sh
future session list --json     # 用于「其他」会话；isStreaming 标记正在运行的那一个
```

`session list` 服务于你不在其中的会话（要恢复的旧对话）与交叉核对：有活跃 run 的
会话报 `isStreaming: true`，因此一轮进行中恰好只有一行为 `true`。其摘要刻意不含
用量——token 与成本在 `session info <id>`——行序为 `updatedAtMs` 新的在前。

**行不通的做法**：`shell` 工具不导出 `$FUTURE_SESSION_ID`，环境中什么也没有；
标题也不能当身份（多个会话可同名，新会话干脆没有）。

## 这是哪个构建

版本字符串往往不足以定位代码，所以 `future version --json` 会报出它承载不了的
信息：

```sh
future version --json
```

```json
{
  "version": "0.0.2-2a4df8a7+local.dirty",
  "isRelease": false,
  "bundleVersion": "0.0.2",
  "gitCommit": "2a4df8a738716ed63b933ad6bf488b975a4bd50b",
  "gitCommitShort": "2a4df8a7",
  "gitDirty": true,
  "buildTarget": "aarch64-apple-darwin",
  "buildProfile": "debug"
}
```

`gitCommit` 是关键：发布 tag（`1.2.3`）与协同构建（`0.0.2-<run>+test`）的版本串里
**完全没有 commit**，本地开发构建也只有一个缩写 hash。把 `gitCommit` 与
`git rev-parse HEAD` 对比，才能回答「我正在运行的二进制是不是我正在读的那份代码」。
`gitDirty`（构建时是否有未提交改动）与 `buildTarget`/`buildProfile` 是 bug 报告
否则只能靠猜的部分。若构建时没有 git 检出，commit 相关字段为 `null` 而不是占位符，
调用方就不会把「未记录」当成 commit 名。

运行中的 Agent 通过 `get_agent_info` 报告同样的事实（`gitCommit`、`gitCommitShort`、
`gitDirty`、`buildTarget`、`buildProfile`），这才让**进程**可以与一份检出、或与
连接另一端的 CLI 相互比对。

## 阅读实现本身

上面的命令回答的是「配置成什么」，源码回答的是「为什么这样表现」。当行为出乎意料、
某个设置的效果不清楚、或准备断言某个边界时，答案通常就在实现里——而仓库自带指路
信息。源码在 [github.com/futuregene/future-os](https://github.com/futuregene/future-os)
的一份检出里，而不是在 `~/.future/`（没有检出时应明说，而不是凭记忆回答）；技能
列表在 [futuregene/future-skills](https://github.com/futuregene/future-skills)，
即 `skills/` 子模块。

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

检索用来「找到」某一段；要想「处理」一个会话，用 `future session transcript`。它把同一批
记录按可选视角投影出来——要哪些角色与块类型（`--select`，包含 thinking）、哪个工具
（`--tool`）、工具的参数还是结果（`--input` / `--output`）、只抽其中提到的文件路径
（`--paths`）、按内容字面过滤（`--grep`）——再用 `--cursor` / `--limit` / `--all` 与
`--max-bytes` 分窗。每个输出的条目都带该 run 的结果与 token 用量，`--runs` 是「一个 run
一行」的账目（状态、耗时、token、错误），`--counts` 一次调用回答「里面有什么」。
`--json` 让每种用法都可脚本化。`session info <id> --json` 则是同一思路用于会话身份：顶层
给出 `cwd`、`model`、`thinkingLevel` 与统计值，旁边附原始会话元数据。选项表与两条已说明的
边界（启发式的 `--paths`，以及 `--tool` 会报告而非错标的无归属工具结果）见
[会话历史回忆](session-history.zh-CN.md#带筛选的会话全文)。

逐 run 看发生了什么：

```sh
future session transcript --session <id> --runs --json   # 哪些 run 失败了、为什么
future session approvals <id>                            # 这个会话在等什么
future session approve <id> <request-id> [--allow <glob> --access read|write]
future session abort <id>                                # 停掉活动 run 并清空队列
```

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

会话级设置仍用 `future session set <id>`，它对运行中的会话立即生效，并分两组：

```sh
# 随会话记录落盘
future session set <id> --model <id> --thinking <level> --cwd <dir> --title <name> --parent <id>

# 作用于运行中的会话（到该 agent 停止为止）
future session set <id> --tools read,shell --permission workspace --sandbox manual
future session set <id> --system-prompt "……" --append-system-prompt "……"
future session set <id> --context-files off --auto-compact off --auto-retry on
```

`--tools` / `--no-tools` / `--no-builtin-tools` / `--system-prompt` /
`--append-system-prompt` / `--permission` / `--sandbox` / `--context-files` /
`--auto-compact` / `--auto-retry` 属于第二组。它们都能用
`future session status <id>` 读回来——这也是 `--sandbox` 不再是「只写」的原因。
`--permission` 是审批门，`--sandbox` 是操作系统包装，两者互相独立；平台无法提供
沙箱时 `sandbox` 会被拒绝。

另外还有 `future session rename <id> <name>`（改标题）、`future session compact
--session <id>`（按需压缩，只返回确认，不是完成的摘要，且忙时会拒绝），以及生命周期
命令 `future session new|fork|forks|clone|title|export`。能力面用
`future skills install|uninstall|update` 调整。

一个最好提前知道、而不是踩到的缺口：**从未运行过的会话，改动要等首次运行时才落盘。**
已有记录的会话，其标题或 cwd 会立即写入；从未产生过 entry 的会话则在首次运行时一并存储，
所以新建的会话在第一次运行前不会出现在 `session list` 里。

有两条命令是「花钱」或「动手」而非读取，因此是显式的：`future session title <id>` 会请该
会话的模型生成标题（一次模型调用；只打印建议，加 `--apply` 才改名），
`future session abort|cancel|approve|reject` 作用于正在运行的工作。

还有一条命令同样是「动手」，而它就是「一个会话触达另一个会话」的方式：

```sh
future run --session <id> "<message>"     # 在那个会话里起一次 run
```

`--session` 要求该会话已存在（未知 id 会被 `switch_session` 拒绝，而不是新建一个），
提示词追加到该会话 —— 默认排在正在进行的 run 之后，用 `--steer` 则打断它。
选哪一种由调用方自己判断，而不是固定行为：追加不动已经在跑的工作，打断则抢占它，
只有调用方知道用户真正想要什么。
它就是 `#` 选中的会话引用的「发送」那一半：用户消息里可以带
`[标题](futureos://session/<id>)`，链接里的 id 正是 `--session` 的参数。
这是另一个会话里的一次完整 run，因此会花积分；命令会在那次 run 结束时返回
（受 shell 工具自身的超时约束），长时间的任务需要相应调大超时。
「读」的那一半是上面的 `future session transcript --session <id>` /
`future session history search --session <id>`。

有两件事只有发送方才能提供，因为目标会话收到的只是一条普通的 user turn：
**来源** —— 这条消息来自另一个会话而不是用户，并给出发送方会话 id；
**意图** —— 期望目标是据此行动，还是仅作知悉。缺了这两项，目标无法分辨
「转发过来的通报」与「向它提出的请求」，可能去做没人让它做的工作。

较长或含非 ASCII 的内容应写入文件后用 `@<file>` 传入，由 CLI 自己读取该文件。
不要先把它读进 shell 变量：Windows PowerShell 5.1 下 `Get-Content` 会用 ANSI
码页解码无 BOM 的 UTF-8 文件，最终落到另一个会话里的就是乱码（与 shell 工具
「读已知文件用 read 工具、不要用 `Get-Content`」是同一条坑）。

改设置会改变此后每一个会话的行为，所以技能把它当作用户的决定：说明旧值与新值，
在用户同意后再改。

## 桌面端的设置

桌面端（FutureOS app）自己的偏好——审批档、隐藏模型、完成提示音、生成标题的语言
等——存在 `~/.future/app/app.db` 的 `app_settings` 表里。它们既不是 Agent 的设置
文档（`future config`，`~/.future/agent/settings.json`），也不是桌面端与 Agent 共用
的 models / providers / auth 文件。

```sh
future desktop settings                        # 全部设置（含默认值）
future desktop settings get approvalTier       # 单个值
future desktop settings set approvalTier manual
future desktop settings set hiddenModels "future/glm-5.3, future/kimi-k3"
```

键名用桌面端 API 的 camelCase 写法；`future desktop --help` 会列出所有键及其取值，
桌面设置界面里能改的每一项在这里都能改。列表值可以写 JSON 数组，也可以写逗号分隔
的列表。写入前先校验取值：非法取值或未知键会让数据库保持原样；桌面端从未写过的
数据库，读取时报告默认值且不会创建它。

读写都不需要桌面端在运行，桌面端会在下次读取设置时看到改动；在它自己的设置界面里
做的修改则立即生效。

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
