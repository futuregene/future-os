# FutureOS 终端界面（TUI）

TUI 是终端客户端：`future-tui`，默认通过**每用户本地 IPC**连接 Agent。
macOS/Linux 使用 Unix socket，Windows 使用仅当前用户可访问的命名管道。
Unix 优先使用 `FUTURE_AGENT_SOCKET`；Linux 否则使用 `$XDG_RUNTIME_DIR/future/agent.sock`，
未设置时回退到 `~/.future/run/agent.sock`（也是 macOS 默认路径）。设置 `FUTURE_HOME`
时会接入 `<FUTURE_HOME>/run/agent.sock` 的实例——见[多实例运行](directory-layout.zh-CN.md#多实例运行future_home)。
若无可连接 Agent，TUI 会启动 sidecar 并在退出时关闭自己启动的进程，无需手动预先启动。
也可以自行运行：

```bash
future agent      # 终端 1：agent
future tui        # 终端 2：终端界面
```

远程/开发环境可给 Agent 传入 `--grpc-addr <host:port>` 显式启用 TCP，客户端设置
`FUTURE_AGENT_GRPC_ADDR=<host:port>`（先尝试显式 TCP，再回退本地 IPC）。

`future tui <args>` 在进程内运行 TUI；独立二进制 `future-tui` 与之等价，但已
不再默认安装（需要时用 `cargo build -p future-tui` 构建）。`future tui --help`
可查看全部选项（print 模式、`--list-models`、`--session` 等）。只有一处会用到这个
区别：`/skills` 的安装/卸载是 spawn 统一的 `future` 二进制完成的，因此独立安装的
`future-tui` 需要 `future` 在 `PATH` 上（见[技能](#技能)一节）。

- 构建 / 安装：见 [构建与安装](build-and-install.zh-CN.md)。
- 会话持久化、模型配置、工具审批与沙箱策略都由 agent 处理；TUI 只是前端。
  `/sandbox` 与 `/permission` 可以从这里修改 agent 的会话策略（见
  [沙箱与工具权限](#沙箱与工具权限)）；启动时除应用 TUI 自己持久化的
  `defaultPermissionLevel` 外，不会有其它隐式改动。
- 新会话默认权限为 `all`。CLI 的 `future run --permission none` 禁止所有工具
  调用，而非只禁止写入；区别见[沙箱指南](../wiki/zh/Sandbox.md)。

## 项目指令

每次 run 开始时，agent 从会话工作目录按 `AGENTS.md` → `CLAUDE.md` →
`GEMINI.md` 顺序读取第一个可读文件。不合并文件、不向父目录查找；空文件也会停止查找。
`/reload` 使用相同顺序，上下文文件状态返回文件名而非正文。

使用 `future tui --no-context-files`（简写 `-nc`，也支持搭配 `-p`）禁用这些项目指令。
开关作用于当前会话的后续请求；此 TUI 新建或切换会话后会重新应用。
它不会清除已有对话内容，也不影响独立的 `FUTURE.md` 工作区记忆层。
如果 agent 无法应用禁用开关，请求会报错，而不会悄悄忽略该参数。
在 TUI 内可用 `/context on|off` 切换同一开关。

## 斜杠命令

下表每个命令都被 TUI 拦截：或打开面板、或经 agent 修改设置、或执行一次 agent 命令——
都不会作为提示词发给模型。命令名不区分大小写；`arg` 为命令名之后的内容。本表是
权威分发集合——应用内帮助浮层（`/help`）只列出其中一部分；不属于已知命令的输入
（包括缺少必需参数的命令，如 `/cwd`）会作为普通提示发给模型。

| 命令 | 用途 |
|---|---|
| `/help` | 显示帮助浮层（快捷键 + 核心命令） |
| `/model [name]` | 直接设置模型；不带参数时打开可搜索的模型选择器 |
| `/models` | 打开模型启用范围编辑器（与 `/scoped-models` 同一菜单） |
| `/models default` | 选择 agent 侧的新会话默认模型 |
| `/scoped-models` | 配置模型启用/禁用列表 |
| `/providers` | 管理 provider：增/删/改、API key、同步模型 |
| `/provider-key <id>` | 提示输入 provider 的 API key（见下文） |
| `/skills` | 技能浏览器：搜索、预览、插入、安装 / 卸载 / 升级 |
| `/tools [none\|all]` | 多选内置工具；`none` 表示全部禁用 |
| `/permission [all\|workspace\|none]` | 设置工具权限级别（并记住它）；不带参数时打开沙箱面板 |
| `/sandbox` | 沙箱 tier、后端可用性与权限选择器 |
| `/theme [id]` | 打开主题选择器，或按 id 直接设置主题 |
| `/sessions` | 浏览并切换会话 |
| `/new` | 新建会话 |
| `/clone` | 克隆当前会话（在新分支继续） |
| `/fork` | 从选中的消息分叉 |
| `/tree` | 会话树（fork/clone 层级） |
| `/name <name>` | 设置会话名称 |
| `/delete [--yes]` | 删除当前会话（需 `--yes` 确认）并新建一个会话 |
| `/title [zh\|en]` | 让模型生成会话标题并应用为会话名 |
| `/compact` | 压缩对话上下文 |
| `/status` | 会话状态、模型、token 用量、成本（作为消息打印到对话中） |
| `/usage` | token、成本、上下文与配额面板 |
| `/stats` | 本会话的消息 / 工具 / token 计数与成本 |
| `/agent` | agent 版本、实例 id、已发现与已加载的技能 |
| `/metrics` | agent 上报的运行时计数器 |
| `/snapshot` | 当前 run 的投影快照 |
| `/history <query>` | 搜索本会话已持久化的历史 |
| `/tool-output [call-id]` | 列出本 run 已存储的工具调用，或读取某次调用的完整输出 |
| `/transcript` | 全文 transcript（可搜索的分页浮层） |
| `/copy` | 复制最后一条助手消息 |
| `/export` | 让 agent 把本会话导出为 HTML 文件 |
| `/import` | *TUI 中不可用*（占位，回复提示） |
| `/reload` | 重载技能 + 上下文文件 |
| `/cwd <dir>` | 切换工作目录 |
| `/context [on\|off]` | 列出上下文文件，或开/关上下文文件加载 |
| `/system-prompt <text>` | 替换本会话的 system prompt |
| `/append-prompt <text>` | 向本会话的 system prompt 追加内容 |
| `/rule <glob>` | 为本会话的工具调用放行一个路径 glob（读写，仅本次 run，相对会话 cwd） |
| `/ephemeral [on\|off]` | 停止或恢复把对话轮次写入会话文件 |
| `/autocompact [on\|off]` | 开/关自动上下文压缩 |
| `/autoretry [on\|off]` | 开/关失败 run 的自动重试 |
| `/shell <cmd>` | 经 agent 执行一条命令并显示输出 |
| `/stop` | 停止当前生成 |
| `/cancel <run-id>` | 取消排队中的运行 |
| `/approve <request-id>` | 批准待执行工具 |
| `/reject <request-id>` | 拒绝待执行工具 |
| `/quit-agent [--yes]` | 请求 agent 进程关闭（需 `--yes` 确认） |
| `/editor` | 用 `$VISUAL` / `$EDITOR` 编辑当前草稿 |
| `/cancel-input` | 取消正在等待的 `/provider-key` 输入 |

`/editor` 在输入框被清空之前处理，因此它打开的是**当前草稿**，退出编辑器后把
内容放回输入框，而不会直接提交。

`/provider-key <id>` 会进入 key 输入状态：**下一次提交的内容就是 key**，而不是
提示词；输入 `/cancel-input` 可取消该提示。在 `/providers` 列表中按 `k` 走同一流程。

`/autocompact` 与 `/autoretry` 接受 `on`/`off`（也接受 `true`/`enable` 与
`false`/`disable`，不区分大小写）；不带参数时翻转当前状态。两者都是本会话的
agent 侧设置：自动压缩开启时页脚会显示指示；`/autoretry` 则在 TUI 内保留镜像，
因为 agent 不回传该值。
`/permission <level>` 还会把参数持久化为 TUI 设置文件里的 `defaultPermissionLevel`，
下次 `future tui` 启动即从该值开始（在 `/sandbox` 面板里选的级别只应用、不持久化）。

`/shell <cmd>` 取的是命令名之后的**原始行内容**，而不是按空白折叠后的参数，因此
`printf '%s  %s'` 会原样传给 agent。它没有 PTY：这就是 agent 的一次性 `shell` 命令
——在会话 cwd 里按 agent 自己的超时（默认 120 秒）执行——捕获的输出与
`exit code: N` 会在分页浮层中打开。

`/delete` 与 `/quit-agent` 是两个破坏性命令：不带字面量 `--yes` 时它们只会解释
自己要做什么，不会发送任何请求。`/delete --yes` 删除会话文件后新建一个会话；
`/quit-agent --yes` 请求 agent 停止接受新提示词。后者的行为需要说清楚：agent 会回答
`Existing runs continue; new prompts are rejected.`，但本地 agent 进程本身仍要由退出
信号（Ctrl-C 或桌面端停止）结束——TUI 会原样引用 agent 的这句话，并在进程退出前
保持连接。

## 弹窗菜单与分页浮层

菜单类选择器——`/model`、`/models`、`/models default`、`/scoped-models`、
`/theme`、`/tools`、`/sessions`、`/tree`——共用同一套弹窗菜单框架
（增量搜索、选项卡、多选、固定滚动窗口与底部按键提示）：

| 按键 | 动作 |
|---|---|
| 直接输入，或 `/` | 增量搜索——边输入边过滤行 |
| `↑↓` / `k` `j` | 移动高亮 |
| `page up` / `page down`、`home` / `end` | 滚动窗口；跳到首/尾 |
| `tab` / `shift+tab` | 切换选项卡（带选项卡的菜单） |
| `space` | 切换当前行（多选菜单，如 `/tools`） |
| `enter` | 确认；多选菜单应用已标记的行，未标记任何行时回退为当前高亮行 |
| `escape` | 先清空搜索词，再关闭菜单 |

多选菜单无法用这种方式表达“空选择”，所以“禁用全部工具”有独立参数（`/tools none`）。

`/providers`、`/skills` 与 `/sandbox` 是更复杂的面板，各自有独立的按键集合，见下文
[Provider 与模型](#provider-与模型)、[技能](#技能)、[沙箱与工具权限](#沙箱与工具权限)。

会话类列表（`/sessions`、`/tree`、`/fork`）使用更简单的可过滤列表：
直接输入即过滤（`backspace` 删除字符），`↑↓` 移动高亮并在两端循环，
`enter` 选中，`escape` 关闭。

`/transcript`、`/agent`、`/stats`、`/metrics`、`/snapshot`、`/history` 与
`/tool-output` 会把完整结果放进全屏分页浮层：

| 按键 | 动作 |
|---|---|
| `↑↓` / `k` `j`、`ctrl+u` `ctrl+b` / `ctrl+d` `ctrl+f`、`space` / `b` | 按行或按页滚动 |
| `g` / `home`、`G` / `end` | 跳到顶部 / 底部 |
| `/` | 开始增量搜索；`n` / `N` 跳到下一个 / 上一个匹配 |
| `y` | 复制当前行（复制后浮层关闭） |
| `q` / `escape` | 关闭 |

其状态行左侧显示 `[i/n] query`，右侧显示滚动百分比。结果为空时不会打开空浮层，
而是在对话中给出消息（如 `No history matches for 'needle'.`）。

## 键盘快捷键

| 按键 | 动作 |
|---|---|
| `ctrl+p` | 循环切换模型（设置了启用范围时在范围内循环） |
| `ctrl+t` | 循环切换思考级别 |
| `shift+tab` | 循环切换思考 |
| `ctrl+o` | 展开 / 收起思考内容 |
| `ctrl+g` | 展开 / 收起工具输出 |
| `ctrl+x` | 复制最后一条助手消息 |
| `ctrl+r` | 浏览会话 |
| `ctrl+c` | 中断 / 退出 |
| `ctrl+l` | 清屏 / 重绘 |
| `tab` | 自动补全 |
| `enter` | 提交 / 接受 |
| `escape` | 关闭弹窗 |
| `page up` / `page down` | 聊天向上/向下滚动 |
| `ctrl+↑` / `ctrl+↓` | 聊天向上/向下滚动（行） |
| `↑↓` | 滚动 / 导航列表 |

弹窗或分页浮层打开时所有按键都交给该浮层，因此上述全局快捷键只作用于对话输入框。

工具结果会连同内容一起渲染。每个工具调用行带一行摘要（如 `edit src/a.rs +12 -3`、
`write 3 files +40 -2`），正文折叠在其下：默认显示 4 行窗口，并给出
`… N more lines · ctrl+g to expand` 提示行；`ctrl+g` 展开（最多 200 行）。
统一 diff（`---`/`+++` 头、`@@` 块）与 `apply_patch` 包裹（模型厂商的 FREEFORM
工具，其语法定义收录在 `tests/provider-protocol/fixtures/`）会显示行号栏
并对增/删行着色。工具输出中的原始 ANSI 转义会被剥离，避免破坏屏幕——颜色只由
diff 渲染器提供。

## 复制文本

`/copy` 与 `ctrl+x` 复制最近一条助手消息；分页浮层中的 `y`
（`/transcript` 以及其它任何分页浮层）复制当前行（复制后浮层关闭）。交付按以下顺序尝试：

1. 原生剪贴板程序——`pbcopy`（macOS）、`clip.exe`（Windows）、
   `wl-copy` → `xclip` → `xsel`（Linux）；只有这条路径能**确认**交付成功；
2. 在 tmux 内时使用 tmux 透传（`ESC Ptmux; … ESC \`）；
3. 终端支持时发送普通 OSC 52 请求。

原生复制会报告 `Copied N characters.`；OSC 52 只是*请求*终端写入，TUI 会明确
提示（`the terminal has to apply it (unconfirmed)`）。超过 100 KB 的内容会直接
报错拒绝。完全没有可用后端时报告 `Copy failed: …`。

## 技能

`/skills` 打开一个浏览器，数据来自两个来源：agent 为本会话加载的技能
（`get_commands`，面板打开时即可用，离线也能看到）与可安装的技能目录
（`future skills list --json`）。面板先用会话已有的技能同步打开，再由这两次拉取
补全；因此技能目录加载失败时只在状态行报告，不会隐藏已经加载出来的行。

| 按键 | 动作 |
|---|---|
| `/`、直接输入 | 增量搜索：匹配 id、名称、描述与分组（技能提供中文时也匹配中文） |
| `↑↓` / `k` `j` | 移动高亮 |
| `tab` | 切换分组选项卡（`All`、`Installed`、`Available`，以及技能自身的分组） |
| `enter` | 把高亮技能的**规范英文名**插入提示词 |
| `ctrl+o` | 在分页浮层中查看高亮技能详情 |
| `i` | 安装（或升级）高亮技能 |
| `u`、`u` | 卸载高亮技能——按两次确认 |
| `U`、`U` | 升级所有落后于目录的已装技能——按两次确认，提示会列出技能名 |
| `r` | 刷新：让 agent 重扫技能目录并重新读取目录 |
| `s` | 切换范围（`All` / `Installed`） |
| `esc` | 关闭 |

行会把两个来源合并显示：已安装的技能显示已装版本（`v1.2`）；目录版本更新的显示
`v1.2 → v1.3 ⬆`；只有目录里存在的显示 `v1.4 · not installed`。对未安装的行按
`enter` 会说明它还没安装并提示按 `i`（绝不会插入 agent 解析不了的名称）。
操作进行中该行显示 `working…`，且前一个操作未结束前会拒绝第二个操作。

安装、卸载与升级都是真实操作，并且经统一二进制执行：TUI 以子进程方式运行
`future skills install <id> --version <v>` / `future skills uninstall <id>` /
`future skills update`（超时 120 秒）。因此单独安装的 `future-tui` 需要 `future`
在 `PATH` 上；否则每个入口都会直接提示
`The \`future\` executable was not found — installing or removing skills is
unavailable.`，而不是稍后抛出难以理解的系统错误。只有**成功**的操作才会让 agent
重扫（`refresh_skills`）并随后重新读取两份列表——失败、超时或被拒绝的 id 只报告
结果，不会重新读取。

## 沙箱与工具权限

`/sandbox` 打开沙箱面板；不带参数的 `/permission` 打开的是同一个面板——因为 tier
与权限级别本就属于同一处设置。

| 按键 | 动作 |
|---|---|
| `↑↓` / `k` `j` | 在六行之间移动（三个 tier，然后三个权限级别） |
| `enter` / `space` | 应用高亮行 |
| `r` | 重新探测沙箱后端 |
| `esc` | 关闭 |

tier 行就是桌面端的三种审批模式：

| Tier | 含义 |
|---|---|
| `Manual` | 文件访问遵循审批规则；白名单中的只读命令自动执行，其它命令会询问。不使用操作系统沙箱。 |
| `Sandboxed` | 命令在操作系统沙箱中执行，必要时请求审批。 |
| `Unrestricted`（`off`） | 不询问、不沙箱——一切直接执行。 |

操作系统沙箱是 macOS 原生能力（Seatbelt）；在 Linux 上 agent 通过系统 Bubblewrap
实现，Windows 上则没有沙箱 tier（命令改由 FutureOS 写保护约束）。因此在非 macOS
主机上打开面板会看到相应说明，并且**后端不可用时 `Sandboxed` 行是禁用的**——
高亮会跳过它，选中它不会有任何动作。

可用性是三态，面板不会把三者混为一谈：

- **探测中**——探测尚未返回（`Sandboxed (checking)`）；
- **可用**——找到后端，并显示其名称、路径与版本；
- **不可用**——探测返回但无可用后端，显示诊断原因与稳定的诊断码
  （例如 Bubblewrap 缺失、不可信或版本过旧）。探测*请求失败*（传输错误）时保持
  “探测中”并报告错误，而不会声称沙箱不存在。

agent 会如实回落而不是假装成功：在探测不到后端的主机上请求 `sandbox`，返回的是
`manual`，面板会打印回落横幅（`Sandbox requested but unavailable — fell back to
Manual: …`），而不是当作成功。你在本面板应用 tier 之前看到的值是当前平台的默认值，
面板会明确标注这一点；一旦有策略应答或 `sandbox_policy_changed` 事件带来真实 tier，
该标注即消失。

权限行是 agent 对工具调用的三个级别：

| 级别 | 含义 |
|---|---|
| `All` | 所有工具调用直接执行，不询问。 |
| `Workspace` | 工具调用先请求审批；授权范围限于当前工作区。 |
| `None` | 所有工具调用在执行前即被拒绝。 |

`/permission <level>` 直接应用并把它持久化为 `~/.future/tui/settings.json` 里的
`defaultPermissionLevel`；在面板里选的级别只应用、不持久化。下次启动此 TUI 时，
`defaultPermissionLevel` 会重新应用到 agent。

## Provider 与模型

TUI 编辑的是与 `future models`、`~/.future/agent/models.json` 相同的 agent 侧配置；
所有改动都经 agent 落盘，其他客户端同样可见。

### `/providers`

列表分两个选项卡——**Built-in**（内置目录）与 **Custom**，支持对行做增量搜索，
并提供以下操作键（菜单底部也会显示）：

| 按键 | 动作 |
|---|---|
| `enter` | 编辑高亮的自定义 provider；若高亮行是内置 provider，则改为打开 key 输入 |
| `e` | 编辑高亮的自定义 provider |
| `k` | 设置 / 替换高亮 provider 的 API key |
| `d` | 删除高亮的自定义 provider |
| `a` | 新增自定义 provider |
| `s` | 从 `future models` 同步模型列表 |
| `r` | 重新加载 `~/.future/agent/auth.json` |
| `escape` | 关闭 |

内置 provider 除 key 之外都是只读的：在内置行上按 `e` / `d` 只会给出提示，`k` 仍可用。

新增/编辑表单包含 ID、名称、API 类型、Base URL、API key 与模型表：

| 按键 | 动作 |
|---|---|
| `tab` / `shift+tab`、`↑↓` | 在各字段间移动 |
| `←` / `→` | 在 API 行上循环切换 API 类型 |
| `ctrl+n` / `ctrl+d` | 新增 / 删除一行模型 |
| `space` / `ctrl+t` | 切换当前模型的图片 / 思考支持 |
| `enter` | 保存（先校验），`escape` 取消 |

API key 字段留空表示保留已存的 key；agent 从不回传 key，因此表单也无法显示它。

### 选择与限制模型

- `/model [name]` 设置会话模型；不带参数时打开可搜索的模型选择器。
- `/models` 与 `/scoped-models` 打开启用范围编辑器——即 `ctrl+p` 循环所依据的
  模型集合。该选择以 `enabledModelIds` 持久化到 `~/.future/tui/settings.json`。
- `/models default` 选择新会话使用的全局默认模型——持久化在 **agent 的**
  `~/.future/agent/settings.json`，而不是 TUI 的设置文件。
- `/status` 把会话状态（模型、provider、图片支持、上下文窗口、token 合计、成本）
  打印到对话中；`/usage` 把同一份状态渲染成面板：标题行含模型与会话名、上下文
  进度条、按模型分列的 token/成本行、配额与队列告警；`/stats` 另外给出本会话的消息、
  工具与 token 计数，`/agent` 给出 agent 自身的版本与实例信息。三者都在分页浮层中打开。

## 主题

`/theme` 打开可搜索的选择器，`/theme <id>` 直接设置主题。已知 id：
`dark`（默认）、`light`、`one-dark`、`one-light`、`high-contrast`、`dracula`。
id 不区分大小写，`_` 可替代 `-`；未知 id 回退到默认主题。选择以 `themeId`
存入 `~/.future/tui/settings.json`，并作用于整个界面：对话区、页脚、弹窗菜单、
provider 列表、会话列表、分页浮层，以及用量/沙箱面板。默认调色板与扩展前完全一致
——`/theme dark` 渲染出的字节与内置配色完全相同。输入行的移植渲染器本身不输出任何
颜色（提示符就是朴素的 `> `），所以那里没有可被调色板改变的内容。

## 通知与终端标题

当**你自己的** run 结束或失败时，TUI 可以响终端铃和/或发送 `OSC 9` 桌面通知
（iTerm2、Ghostty、WezTerm、kitty、warp）。同一会话中由其他客户端启动的 run
不会触发通知。

通知渠道位于 `~/.future/tui/settings.json` 的 `notify`：

```json
{
  "themeId": "one-dark",
  "notify": { "enabled": true, "bell": true, "osc9": true, "title": false }
}
```

- `enabled`——总开关；`false` 静默所有渠道。
- `bell`——发出 `BEL`。旧版顶层键 `bellOnComplete: false` 仍可静默铃声。
- `osc9`——发送桌面通知。
- `title`——额外用事件内容设置窗口标题。运行状态标题是另一回事：run 流式输出时
  为 `[>] <project> | <model> | <session>`；默认会写入标题，除非存在显式
  `notify` 对象（此时由 `title` 决定，默认为 `false`）。

通知标题与正文都会做净化——路径或会话名中的转义序列、控制字符与 bidi/不可见
字符无法逃逸到终端；标题截断到 100 列。stdout 不是 TTY 时不写入任何内容，
因此 print 模式与管道输出保持干净。

## 设置与本地文件

TUI 把客户端侧设置持久化到 `~/.future/tui/settings.json`：
`defaultModel`、`defaultThinkingLevel`、`defaultPermissionLevel`、
`enabledModelIds`、`themeId`、`bellOnComplete` 以及上面的 `notify` 对象。
其中 `defaultModel`、`defaultThinkingLevel` 与 `defaultPermissionLevel` 会在 TUI
启动时应用到 agent。日志：`PI_DEBUG_REDRAW=1` 时把调试重绘日志写入
`~/.future/tui/debug.log`；`PI_TUI_WRITE_LOG=1` 时原始屏幕写入记录到
`~/.future/tui/write.log`。

键位就是上文的固定表；`~/.future/tui/keybindings.json` 覆盖文件目前**不会被读取**
（键位管理器里有覆盖机制，但没有任何代码把文件读进它）。

## 排障

| 症状 | 解决办法 |
|---|---|
| 启动即连接 / gRPC 错误 | 未找到或未能启动 Agent。检查 sidecar 错误、用户与 IPC 环境，或手动运行 `future agent`；仅显式 TCP 模式检查配置端口是否占用。 |
| auth / 「no model」错误 | 未配置模型。运行 `future auth login`，用 `/providers` 添加 provider，或编辑 `~/.future/agent/models.json`——见仓库 README「配置模型」 |
| `/editor` 提示缺少编辑器 | `$VISUAL` 与 `$EDITOR` 都未设置。带上其中一个启动，例如 `EDITOR=vim future tui`；空的 `$VISUAL` 不会回退到 `$EDITOR`。 |
| `/copy` 提示 unconfirmed 或失败 | 原生剪贴板程序不可用（远程/SSH 会话？），已改为 OSC 52 请求，需终端自行应用。超过 100 KB 的内容会被拒绝，请改复制较小的范围。 |
| `/skills` 无法安装：提示找不到 `future` 可执行文件 | 面板只在本 TUI 启动时解析一次 `future`。请用 `future tui` 启动（或把统一二进制放进 `PATH`）而不是独立的 `future-tui`；安装后需要重启才生效。 |
| `/sandbox` 显示沙箱不可用 | 探测没有找到可用后端（macOS Seatbelt；Linux Bubblewrap）。面板会给出原因与诊断码，按 `r` 可重新探测。在 Linux 上 agent 会回落到 `Manual`，面板会如实说明而不是假装 tier 已生效。 |
| 工具调用未被询问就被拒绝 | 权限级别是 `None`——见 `/sandbox` → Tool permissions，或执行 `/permission workspace`。 |
| 没有铃声 / 没有桌面通知 | 检查 `~/.future/tui/settings.json` 中的 `notify.enabled`（以及 `notify.bell` / `notify.osc9`，还有旧版 `bellOnComplete: false`）。其他客户端启动的 run、print 模式与管道输出都不会通知。 |
| 主题没有重绘某处 | 对话区、页脚、菜单、provider 列表、会话列表、分页浮层与用量/沙箱面板都会跟随 `/theme`。若仍有部件未被重绘，那是一个 bug——唯一有意不参与的是输入行，因为它本身不输出颜色。 |

参见：[目录布局](directory-layout.zh-CN.md)（`~/.future/` 下各目录职责）、
wiki [命令行工具](../wiki/zh/CLI.md) / [设置](../wiki/zh/Settings.md)（桌面应用版）。
