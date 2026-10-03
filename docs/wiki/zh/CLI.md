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

`future config`、`future auth login` 和 `future auth logout` 在 Agent 未运行时也能直接使用，配置写入 `~/.future/agent/`。技能目录/安装命令也不要求 Agent 在线。TUI 与桌面一样可以自动启动自己的 Agent sidecar。`future doctor` 可诊断连接；默认通过每用户本地 IPC 发现，不是始终开放的 TCP 端口。

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
| `--continue`、`-c` | 继续最近的会话。 |
| `--session <id>` | 连接指定 ID 的已有会话。 |
| `--fork <entry-id>` | 从当前会话的某个条目分叉出新会话。 |
| `--permission <level>` | `all`（新会话默认，不受限）、`workspace`（审批门控访问）、`none`（拒绝所有工具调用）。这不是 OS 沙箱选择器，见 [[审批与沙箱|Sandbox]]。 |
| `--tools`、`-t <names>` | 用逗号分隔要启用的工具名（如 `read,shell`）；`--no-tools` / `--no-builtin-tools` 可禁用工具。 |
| `--steer` | 中断当前会话正在运行的任务；不传时，新 prompt 排在忙碌任务之后。 |
| `--system-prompt`、`--append-system-prompt` | 替换或追加系统提示词。 |
| `--cwd <dir>` | 设置工作目录。 |
| `--mode json` | 以 JSON 而非文本打印回答。 |
| `--verbose` | 把进度与工具调用写到 stderr。 |
| `--grpc-addr <addr>` | 显式指定 Agent 的 TCP 地址（默认每用户本地 IPC；也可用 `FUTURE_AGENT_GRPC_ADDR`）。 |
| `--no-session` | 本次不保存为会话。 |

示例:

```bash
future run --model sonnet:high "审查这些改动"
future run @README.md "总结这个文件"
future run --tools read,shell "读一下 README 并列出文件"
```

完整选项见 `future run --help`。

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
future agent --home /tmp/futureos-b  # 第二个实例：独立状态与 IPC 端点
```

`future agent <args>` 直接运行 agent 后端——参数与独立二进制 `future-agent`
完全一致，包括 `--home DIR`：它把该实例的 FutureOS home（默认 `~/.future`，即
`$FUTURE_HOME`）换成另一个目录，于是多个 Agent 可以并存而无需共享会话、锁或
socket（见[多实例运行](../../guide/directory-layout.zh-CN.md#多实例运行future_home)）。
客户端设置同一个 `FUTURE_HOME` 即接入对应实例。

### `tui` / `channel` / `loop` —— 运行其他组件

`future` 是所有 Rust 组件的统一入口——每个都运行与独立二进制完全相同的代码，
独立二进制仍是构建目标（`cargo build -p <crate>`），但已不再默认安装：

```bash
future tui                # 终端界面
future channel            # IM 渠道桥：飞书 / 钉钉
future loop status        # loop 控制面：goal/todo/gate
```

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
future session rename <id> <name>
future session new [--cwd <dir>] [--name <text>]
future session forks <id>                # 该会话可分叉的轮次
future session fork <id> --entry <id>
future session clone <id>
future session title <id> [--apply]      # 生成标题（一次模型调用）
future session export <id> [--out <path>]
future session abort <id>                # 停掉活动 run 并清空队列
future session cancel <id> --run <run-id>
future session approvals <id>
future session approve <id> <request-id> [--allow <glob>]
future session reject <id> <request-id>
future session delete <id>
```

`session info` 读的是落盘 journal（对话包含什么、累计花了多少）；`session status` 读的是
运行中的 agent（这个会话**现在**怎么配置：生效中的工具权限、沙箱档、上下文占用、已加载的
上下文文件与技能、活动与排队 run、待审批项）。只有后者能回答「这个会话的设置与全局默认
不同」这类问题。`session set` 写两组——随会话落盘的（`--model`、`--thinking`、`--cwd`、
`--title`、`--parent`）与作用于运行中的（`--tools`、`--no-tools`、`--no-builtin-tools`、
`--system-prompt`、`--append-system-prompt`、`--permission`、`--sandbox`、
`--context-files`、`--auto-compact`、`--auto-retry`）——后者可用 `session status` 读回。
`abort`/`cancel`/`approve`/`reject` 是 TUI `/stop`、`/cancel`、`/approve` 的 CLI 对应物，
因此在脚本里发起的 run 也能从脚本里停掉。

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
等级属于运行快照，随下一次运行写入记录。`future run` 启动时会用自己的 `--cwd`（默认当前目录）覆盖会话 cwd，
所以这里设的 cwd 只保持到下一次运行为止。

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
- **连接失败？** 运行 `future doctor`，检查 sidecar 错误、用户和 IPC 环境是否一致；按需打开桌面或运行 `future agent`。

---

## 另见

- [[安装 FutureOS|Installation]] —— 工具随包附带的位置。
- [[技能|Skills]] —— 同样的技能,在应用里管理。
- [[常见问题|FAQ]] —— 常见问题。
