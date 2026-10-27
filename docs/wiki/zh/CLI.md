# 命令行工具(`future`)

FutureOS 附带一个**可选的**命令行工具,叫 `future`。它随每个下载包一起附带。

> **你多半用不到它。** 桌面应用已能满足大多数日常需求。只有当你想脚本化、自动化,或纯在终端里操作时,才需要用命令行。**如果你不熟悉终端,可以跳过本页。**

---

## 位置

工具跟应用放在一起:

| 系统 | 位置 |
|---|---|
| **macOS**(`.dmg`) | 应用内:`/Applications/FutureOS.app/Contents/MacOS/future` |
| **Windows**(安装版或便携 `.zip`) | 应用目录里的 `future.exe` |
| **Linux**（`.deb`） | PATH 中的 `future` |
| **Linux**（portable / CLI-only tarball） | 解压目录中的 `future`；加入 PATH 前用 `./future` |

CLI **随每个下载包一起附带** —— 安装版和便携版里都有,就装在应用旁边。

---

## 运行

在含有该二进制的文件夹里打开终端,用 `--help` 查看全部用法:

```bash
future --help
```

想在任意位置都能运行,可把它所在文件夹加入 `PATH`,或设置别名。例如在 macOS 上:

```bash
alias future="/Applications/FutureOS.app/Contents/MacOS/future"
```

### agent 必须在运行

大部分命令都要连接 FutureOS 的 agent(后台服务)。如果**桌面应用已打开**,agent 就已经在运行。否则,用 `future agent` 启动 agent(或直接运行 `future-agent` 二进制,二者是同一套代码;打开桌面应用也会自动拉起 agent)。

`future config`、`future auth login` 和 `future auth logout` 在 Agent 未运行时也能直接使用，配置写入 `~/.future/agent/`。技能目录/安装命令也不要求 Agent 在线。TUI 保留独立 Agent 启动方式；桌面默认在自身进程中运行完整 Agent，或连接兼容外部服务。退出内置宿主也会停止 CLI/TUI 的任务，见 [Desktop Agent 指南](../../guide/desktop-agent.zh-CN.md)。`future doctor` 可诊断连接；默认通过每用户本地 IPC 发现，不是始终开放的 TCP 端口。

---

## 命令组

### `init` —— 首次初始化

```bash
future init
```

安装所有内置技能。在 macOS 和 Linux 上,还会把 `future`(若存在,连同 `future-agent`)链接到 `~/.future/bin/`,并提示 PATH 配置。

### `config` —— 配置模型提供商，以及读写全局设置

```bash
future config
```

交互式选择 **FutureOS** 或**自定义提供商**：

- FutureOS：若已配置 token，会先询问是否重新登录；否则直接进入设备码登录流程。
- 自定义：依次填写提供商 ID、API 协议、Base URL、API key、模型 ID、上下文窗口等信息。API key 输入时不会回显。

配置写入 `~/.future/agent/models.json` 和 `~/.future/agent/auth.json`。如果 agent 正在运行，会通过 RPC 写入并立即刷新模型列表；否则在 agent 下次启动时生效。

同一组命令也用于读写 agent 的全局设置（不带子命令时仍是上面的交互流程）：

```bash
future config get                                   # 生效中的设置（含默认值）
future config get defaultPermissionLevel            # 单个值，便于脚本使用
future config set defaultPermissionLevel workspace  # 改一个键
future config set compaction.reserve_tokens 8192
```

`future config get --help` 列出全部可设置的键与取值。写入前先校验：取值非法或键名未知时
`settings.json` 保持原样。读写都不需要 agent 在运行；改动在下一个新会话
（`defaultModel`、`defaultPermissionLevel`）或下次 agent 启动时
（`compaction.*`、`retry.*`、`maxTurns`）生效。完整说明见[自我认知](../../guide/self-inspection.zh-CN.md)。

### `auth` —— 登录与登出

```bash
future auth login       # 通过浏览器登录(设备码流程)
future auth status      # 查看是否已登录
future auth credential  # 输出 API key 与端点,供脚本使用
future auth logout      # 登出
```

### `account` —— 账户信息

```bash
future account profile  # 邮箱、用户 ID、验证状态、创建日期
future account balance  # 余额(--json 输出机器可读结果)
```

两条命令都免费。它们是 CLI 中唯一会离机的读取：通过网络访问 Future 平台，需要
先 `future auth login`。API key 由 CLI 自己读取，无需手动传入——而余额偏低应当
转达给用户，不是去创建充值订单的理由。

### `version` —— 这是哪个构建

```bash
future version            # 与 `future --version` 输出相同
future version --json     # 版本、commit、脏标记、目标平台、构建配置
```

`--json` 会报出版本串承载不了的信息：`gitCommit`（该二进制构建自哪个完整对象名）、
`gitDirty`、`buildTarget`、`buildProfile`。发布版与协同构建的版本串里**完全没有**
commit，因此这是判断「正在运行的二进制是不是我正在读的那份代码」的唯一方式。

### `run` —— 发一次性 prompt 并打印回答

```bash
future run "介绍一下这个项目"
```

常用选项与写法:

| 写法 | 作用 |
|---|---|
| `--model <model>` | 选择模型。支持 `model:thinking`,例如 `sonnet:high`。 |
| `--thinking <level>` | 思考级别:`off`、`minimal`、`low`、`medium`、`high`、`xhigh`。 |
| `@<path>` | 把某个文件的内容包含进 prompt。 |
| `--message-file <path>` | 把文件内文**当作**消息本身发出，逐字不变：不包 `<file>` 外壳、不截断也不裁空白。与 `@<path>` 和位置消息互斥；供脚本与 agent 之间的派发使用。 |
| `--continue`、`-c` | 继续最近的会话。 |
| `--session <id>` | 连接指定 ID 的已有会话。 |
| `--fork <entry-id>` | 从当前会话的某个条目分叉出新会话。 |
| `--permission <level>` | `all`（新会话默认，完全访问）、`workspace`（审批门控访问）、`none`（拒绝所有工具调用）。这不是 OS 沙箱选择器，见 [[审批与沙箱|Sandbox]]。 |
| `--tools`、`-t <names>` | 用逗号分隔要启用的工具名（如 `read,shell`）；`--no-tools` / `--no-builtin-tools` 可禁用工具。 |
| `--steer` | 中断当前会话正在运行的任务；不传时，新 prompt 排在忙碌任务之后。 |
| `--no-wait` | 提交 prompt 并打印 ack 后立即退出；稍后可用 `future session wait <id>` 取回结果。 |
| `--system-prompt`、`--append-system-prompt` | 替换或追加系统提示词。两者都随会话落盘、每次 run 生效。 |
| `--cwd <dir>` | 设置工作目录。 |
| `--mode json` | 以 JSON 而非文本打印回答。 |
| `--verbose` | 把进度与工具调用写到 stderr。 |
| `--no-title` | 不为本次新建的会话生成标题。默认会在首答之后用一次模型调用生成并保存标题；已有会话（`--session`/`--continue`/`--fork`）从不改名。 |
| `--grpc-addr <addr>` | 显式指定 Agent 的 TCP 地址（默认每用户本地 IPC；也可用 `FUTURE_AGENT_GRPC_ADDR`）。 |
| `--no-session` | 本次不保存为会话。 |

示例:

```bash
future run --model sonnet:high "审查这些改动"
future run @README.md "总结这个文件"
future run --message-file /tmp/prompt.md --session abc123
future run --tools read,shell "读一下 README 并列出文件"
```

`@<path>` 与 `--message-file` 不是同一个东西的两种写法。`@<path>` 会把文件包进
`<file name="…">` 外壳，再把你的消息文字接在后面——这是人在终端里“附带一个文件”的意思。
`--message-file` 则把文件字节当作整轮消息发出，对端看到的只有你的文字；脚本化派发和
agent 之间转达需要的就是这个，否则目标会话（以及从首轮生成的标题）开头是一个文件外壳，
而不是那条指令。

完整选项见 `future run --help`。

`future run` 在**它自己新建的会话**首答结束后会生成并保存标题（一次额外的模型调用，可能耗时数十
秒并消耗额度），使用的生成器与桌面端、`future session title` 相同；语言取桌面设置里的
`titleLanguage`。通过 `--session`、`--continue` 或 `--fork` 接入的**已有会话一律不改名**。
生成期间如果你改了名字、又发了一轮，或会话已经开始跑下一次，过期的建议会被丢弃（不会覆盖你的
命名）；生成失败只会在 stderr 记一行，run 本身仍然成功，答案照常输出。不需要这个行为时用
`--no-title` 关掉。

### `skills` —— 管理能力包

```bash
future skills list             # 列出目录中的技能(已装 + 可用)
future skills install <name>   # 安装指定技能
future skills install-builtin  # 按目录分类安装 builtin/ 下的技能
future skills uninstall <name> # 卸载已安装的技能
future skills update           # 升级所有已安装技能
```

### `tools` —— 列出与调用工具

```bash
future tools list
future tools describe <name>
future tools call <name> --args '<json>'
future tools call <name> --stdin
future tools call <name> --args '<json>' --output result.png
```

当工具需要文件内容时,文件路径参数会被自动转换。

### `mcp` —— 配置 MCP 服务器并调用其工具

```bash
future mcp list                          # 已配置的服务器,以及它们来自哪个文件
future mcp status                        # 逐个启动、握手、报告工具数——确认真的可用
future mcp add rea --command rea --arg --mcp
future mcp tools rea --query binary      # 该服务器的工具,一行一个
future mcp describe rea inspect_go_binary  # 一个工具的完整参数 schema
future mcp call rea inspect_go_binary --args '{"path":"/usr/bin/foo"}'
future mcp call rea inspect_go_binary --args '{...}' --out full.json  # 写出全部结果
future mcp serve rea                     # 长驻一个会话,从 stdin 逐行收请求
future mcp remove rea
```

服务器配置在 `~/.future/mcp.json`(与 `agent/` 同级,所以 `FUTURE_HOME` 会把它一起搬走),
以本地进程、stdio 上的 MCP 协议运行。agent 自身没有 MCP 客户端,所以这一组就是 agent
会话触达 MCP 服务器的途径:技能或用户通过普通的 `shell` 工具调用 `future mcp`。

`list` 只回答「配置了什么」,`status` 回答「能不能用」——它逐个启动服务器、完成握手、
报告工具数与耗时然后停掉(不调用任何工具);失败时附上服务器自己的 stderr(通常正是
原因,例如缺引擎或缺凭据),并且有任何一台不可达就以退出码 1 结束;`enabled: false`
只报 `disabled`,不算失败。

每一步都有边界,因为这一组要面对的服务器很大:`tools` 每个工具一行并分页
(`--limit`,默认 50;`--offset` 继续)。`describe` 需要一个精确的工具名并打印完整
JSON Schema。`call` 最多打印 `--max-bytes`(默认 64 KB),并提示用 `--out <file>`
取回其余部分,因此返回数百 MB 的工具不会淹没向它提问的那个终端。服务器启动慢时加
`--timeout <secs>`(默认 60)。

这里有两个不同的预算,各有开关:`--max-bytes` 限制**打印**多少,`--max-frame-bytes`
限制**读入内存**的单帧多大(默认 32 MiB,也可用 `FUTURE_MCP_MAX_FRAME_BYTES`)。
帧装不下时 `--max-bytes` 帮不上忙,所以超限报错会同时点名正在等待的方法、说明这是读预算,
并给出两个旋钮;很多服务器自己也有响应预算(REA 是 `REA_MCP_MAX_RESPONSE_BYTES`)。
工具名写错时,`describe` 与 `call` 给出同样的「Did you mean」提示。
工具自己报告的错误(`isError`)会把内容打到 **stderr** 并
以退出码 1 结束,与 `future tools call` 报告工具失败的方式一致——脚本或 agent 不必从
stdout 里解析标记来判断调用是否成功。

`call` 每次都会重新启动服务器,对快服务器无所谓;但需要「启动时加载并分析目标」的引擎就
很贵——Ghidra 一次冷启动约 4 分钟,五次调用就是五次。`serve` 只付一次:

```bash
printf '%s\n' '{"tool":"open_binary","args":{"path":"/bin/ls"}}' \
              '{"tool":"open_binary","args":{"path":"/bin/cat"}}' \
  | future mcp serve rea
# {"ok":true,"ready":true,"server":"rea","toolCount":207}
# {"ok":true,"isError":false,"result":{...}}
# {"ok":true,"isError":false,"result":{...}}
```

两个方向都是行分隔 JSON:请求是 `{"tool":...,"args":{...}}`,应答是 `{"ok":...,"result"/"error":...}`。
第一行是 ready 应答(握手已完成、工具数已知),所以调用方可以等这一行,而不必猜分析要多久。
请求按序逐条处理:有状态的引擎会话不该并发驱动。单条请求失败(JSON 非法、缺 `tool`、
服务器拒绝、工具报错)只影响它自己,会话继续。stderr 留给服务器自己的日志,stdout 始终是干净的 JSON 流。

### `desktop` / `workspace` —— 桌面应用自己的设置与 workspace

```bash
future desktop settings                       # 桌面应用的全部生效设置
future desktop settings get approvalTier      # 单个值，便于脚本
future desktop settings set approvalTier auto # off | manual | sandbox | auto
future workspace list                         # workspace 列表及其会话
future workspace add ~/projects/demo --name Demo
```

读写的是桌面应用自己的数据库（`~/.future/app/app.db`），不是 agent 的设置文档——
后者由 `future config` 管理。无需桌面应用在运行；运行中的应用下次读取设置时生效。
`future desktop settings --help` 列出全部可设键，包括 `hiddenModels`、
`autoUpgradeSkills`、`autoConnectRemote`、`bellOnComplete`、`titleLanguage`、
`skillRecommend` 与 `autoResumeInterrupted`（最后一项现在已无作用：Agent 不会自己续跑被中断的运行，
下一轮由某个地方发出的 prompt 开始）。这里添加的 workspace 会在应用发布目录后
出现在左栏（以及已配对手机上）；目录必须已存在。

### `remote` —— 已配对设备与远程桌面

```bash
future remote status                       # 远程桌面是否开启、当前谁在连接
future remote enable | disable             # 从终端开关这个开关
future remote devices [list]               # 已配对设备，以及正在操控的那台
future remote devices add                  # 生成邀请（需要桌面在运行）
future remote peers [list]                 # 本机连接的远程桌面
```

配对、主机模式与两侧各自能做什么，见 [[手机远程|Remote]]。

### `task` —— 定时任务

```bash
future task list [--all]
future task add --name "Nightly review" --prompt "Review new issues" \
  --cwd ~/project --model <model> --thinking high --daily --time 09:00
future task runs "Nightly review"          # 运行台账
future task output <run-id>                # 某次运行的完整回答
```

定时任务 = 可复用的 prompt + 触发方式，触发时或手动请求时以完全权限运行。
其他触发方式为 `--every`、`--at`、`--weekly`、`--monthly`、`--yearly`；
`--timezone` 指定墙钟时间按哪个时区读取。运行由桌面（或 host）的 tick 循环执行，
不由本命令执行，所以 `future task run` 只是排队，`--wait` 等待结果。
见 [[定时任务|Scheduled-Tasks]]。

### `data` —— 离线备份与恢复

```bash
future data backup <new-directory>
future data verify <backup-directory>
future data restore <backup-directory> --yes
future data resume | rollback
```

先停止桌面、agent 以及其他写入者。备份包含两个数据库、托管的图片/工作区/审查数据、
兼容导入的旧数据源，以及包含凭据的 agent 配置——请妥善保管。恢复以原来的 FutureOS home
为目标；未完成的恢复会阻止启动，直到 `resume` 或 `rollback` 完成。

### `models` —— 列出可用模型

```bash
future models            # 列出运行中 agent 的模型
future models --json     # 机器可读输出
```

### `agent` —— 启动 agent 服务

```bash
future agent              # 启动 agent gRPC 服务
future agent --help       # 查看 Agent 选项（连接覆盖、日志、profiling）
future agent --probe-sandbox # 一次性 OS 沙箱诊断，不启动常驻服务
FUTURE_HOME=/tmp/futureos-b future agent  # 第二个实例：独立状态与 IPC 端点
```

`future agent <args>` 直接运行 agent 后端——参数与独立二进制 `future-agent`
完全一致，`FUTURE_HOME=DIR` 把该实例的 FutureOS home（默认 `~/.future`，即
`$FUTURE_HOME`）换成另一个目录，于是多个 Agent 可以并存而无需共享会话、锁或
socket（见[多实例运行](../../guide/directory-layout.zh-CN.md#多实例运行future_home)）。
客户端设置同一个 `FUTURE_HOME` 即接入对应实例。

### `tui` / `channel` / `goal` / `host` —— 运行其他组件

`future` 是所有 Rust 组件的统一入口——每个都运行与独立二进制完全相同的代码；
独立二进制仍是 `cargo build -p <crate>` 的构建目标，发布产物则是 `future`：

```bash
future tui                # 终端界面
future channel            # IM 渠道桥：飞书 / 钉钉
future goal status        # goal 控制面：goal/todo/gate
future host               # 前台主机：把这台机器作为远程桌面
future relay init|serve   # 自建、免账号的中继，用于远程控制
```

`future host` 是不带 GUI、让手机或另一台电脑持续连到本机的方式；见
[主机模式指南](../../guide/desktop-host.zh-CN.md) 与 [[手机远程|Remote]]。
`future relay` 在你不想使用官方中继时自建 NATS 中继。

### `session` —— 管理会话

```bash
future session list
future session set <id> [--parent <id>] [--title <name>] [--cwd <dir>]
                        [--model <id>] [--thinking <level>]
future session info <id> [--json]
future session status <id> [--json]     # 实时状态：权限、沙箱、上下文、run
future session transcript --help # 筛选并分窗查看单个会话的记录
future session history --help   # 搜索并读取原始历史
future session compact --help   # 请求手动压缩上下文
future session reset <id> [选择器] [--yes]  # 原地删除部分历史
future session rename <id> <name>
future session new [--cwd <dir>] [--name <text>]
future session forks <id>                # 该会话可分叉的轮次
future session fork <id> --entry <id>
future session clone <id>
future session title <id> [--apply]      # 生成标题（一次模型调用）
future session export <id> [--out <path>]
future session abort <id>                # 停掉活动 run 并清空队列
future session cancel <id> --run <run-id>
future session wait <id> [--run <run-id>] [--timeout <duration>]  # 等待某个 run 结束并打印结果
future session approvals <id>
future session approve <id> <request-id> [--allow <glob>]
future session reject <id> <request-id>
future session delete <id>
future session prune --older-than 3d [--yes]  # 删除闲置超过该时长的会话（默认只预览）
```

`session info` 读的是落盘 journal（对话包含什么、累计花了多少）；`session status` 读的是
运行中的 agent（这个会话**现在**怎么配置：生效中的工具权限、沙箱档、上下文占用、已加载的
上下文文件与技能、活动与排队 run、待审批项）。只有后者能回答「这个会话的设置与全局默认
不同」这类问题。`session set` 写两组——随会话落盘的（`--model`、`--thinking`、`--cwd`、
`--title`、`--parent`、`--system-prompt`、`--append-system-prompt`）与作用于运行中的
（`--tools`、`--no-tools`、`--no-builtin-tools`、`--permission`、`--sandbox`、
`--context-files`、`--auto-compact`、`--auto-retry`）——两组都可用 `session status` 读回。
`abort`/`cancel`/`approve`/`reject` 是 TUI `/stop`、`/cancel`、`/approve` 的 CLI 对应物，
因此在脚本里发起的 run 也能从脚本里停掉。

`--append-system-prompt` 是上面这个划分里的例外：它随会话记录落盘，因此 agent 重启后仍在、
之后任何客户端发起的 run 也都带上，并在每次 run 中追加在项目上下文与工作区记忆之后。
`future session new --append-system-prompt "……"` 在创建时即可设置，传空字符串即清除。
`session status` 会报出该附加段以及项目上下文是否启用；`session status <id>
--system-prompt` 打印下一次 run 将发送的完整提示词（加 `--json` 则输出为对象）。

`--system-prompt`（整体替换）同样随会话记录落盘：它替换提示词的开头部分（内置身份与
工具规则），项目上下文、工作区记忆与附加段仍接在其后；传空字符串恢复内置提示词。
`future session info` 可从 journal 读回这两个值。

`future session history search --all --query "<关键词>"` 跨会话检索，因此「我们以前是否
处理过这件事」不必先知道是哪个会话。它扫描最近更新的 `--sessions` 个会话（默认 50），
并返回 `scannedSessions` 与 `truncated`，所以只覆盖了一部分历史时是可见的，而不是被
悄悄当成「从未讨论过」。详见[自我认知](../../guide/self-inspection.zh-CN.md)。

`future session transcript --session <id>` 面向「处理」整段对话而非「找到」某一段：
`--select` 选择切片（user、assistant、thinking、tool-call、tool-result、session、compaction），
`--tool` 只看单个工具，`--input` / `--output` 取工具的输入或输出，`--paths` 只抽其中提到的
文件路径，`--cursor` / `--limit` / `--max-bytes` 用于分窗翻页；`--counts` 是了解陌生会话的
低成本第一步。`session info` 与 `transcript` 都支持 `--json`。

`set` 修改**已有**会话的这些设置（只改传了的参数，其余不动，`--parent ""` 表示解除父会话）。
`--parent` 只记录会话谱系，不复制父会话历史（那是 `fork` 的行为），且父会话必须是已存在的会话。

已有记录的会话，其标题与 cwd 会立即写入；从未运行过的新会话则在首次运行时一并落盘。模型与思考
等级属于运行快照，随下一次运行写入记录。`future run` 只在显式传了 `--cwd`、或该会话由这次运行新建时
才使用自己的 cwd（新会话默认落在调用方目录）；用 `--session`/`--continue` 挂到已有会话时不会改动
这里设的 cwd。

`session reset` 是 `session compact` 的破坏性对应物。压缩只能把历史**投影**掉：它必须保留
每一段用户原文，因此当用户原文本身就超过上限时，会话会一直报
`context compaction budget exceeded` 且不会自愈。`future session reset <id>` 则是真正删除
你点名的那部分——整轮（`--all`、`--keep-first N`、`--keep-last N`），以及保留部分里的条目
类型（`--drop thinking`、`tool-calls`、`tool-results`、`attachments`、`compaction`；一个「user
轮」= 一条用户消息与它之后的内容，直到下一条用户消息，与 `session forks` 的口径一致）。
不加 `--yes` 时只做预演；有活动 run 或正在压缩时会拒绝而不是打断；会话记录本身（id、标题、
cwd、模型、提示词、权限）不会被改动；真正执行前会先写一份**完整**历史（含将被删的条目）的
无损 JSON 备份并打印路径（`~/.future/agent/session-backups/<会话 id>-<时间戳>.json`），
所以误操作可以恢复，将来的 restore 也有输入。

会话数据存放在 Agent 的 SQLite 数据库 `~/.future/agent/agent.db`；`~/.future/agent/sessions/` 仅作为旧版
JSONL 数据的导入来源保留。

### `doctor` —— 环境诊断

```bash
future doctor
```

一次检查登录状态、组件安装、Agent 连通性、OS 沙箱可用性、配置、provider/模型、会话与技能。

---

## 小贴士

- **macOS 被拦？** 先核对下载来源和签名；可信未签名测试包有单独首次启动说明，见 [[常见问题|FAQ]]。
- **连接失败？** 运行 `future doctor`，检查 Agent 启动/兼容性/实例锁错误、用户和 IPC 环境是否一致；按需打开桌面或运行 `future agent`。

---

## 另见

- [[安装 FutureOS|Installation]] —— 工具随包附带的位置。
- [[技能|Skills]] —— 同样的技能,在应用里管理。
- [[常见问题|FAQ]] —— 常见问题。
- [主机模式](../../guide/desktop-host.zh-CN.md)、[自建中继](../../guide/self-hosted-relay.zh-CN.md)、[定时任务](../../architecture/tasks.zh-CN.md) 与 [数据与存储](../../architecture/sqlite-migration.zh-CN.md) —— `future host`、`future relay`、`future task`、`future data` 背后的指南。
