# 命令行工具(`future`)

FutureOS 附带一个**可选的**命令行工具,叫 `future`。它随每个下载包一起附带。

> **你多半用不到它。** 桌面应用已能满足大多数日常需求。只有当你想脚本化、自动化,或纯在终端里操作时,才需要用命令行。**如果你不熟悉终端,可以跳过本页。**

---

## 位置

工具跟应用放在一起:

| 系统 | 位置 |
|---|---|
| **macOS**(`.dmg`) | 应用内:`/Applications/FutureOS.app/Contents/MacOS/future` |
| **Windows**(便携 `.zip`) | 解压文件夹里的 `future.exe` |

> 在 Windows 上,命令行工具在**便携包**里。普通安装版只含应用和它的后台服务,不含单独的 `future.exe`。

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

每条命令都要连接 FutureOS 的 agent(后台服务)。如果**桌面应用已打开**,agent 就已经在运行。否则先启动它:

```bash
future agent start
```

---

## 命令组

### `auth` —— 登录与登出

```bash
future auth login     # 通过浏览器登录
future auth status    # 查看是否已登录
future auth logout    # 登出
```

### `agent` —— 管理后台 agent

```bash
future agent start
future agent stop
future agent restart
future agent status
```

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
| `--cwd <dir>` | 设置工作目录。 |
| `--mode json` | 以 JSON 而非文本打印回答。 |
| `--no-session` | 本次不保存为会话。 |

示例:

```bash
future run --model sonnet:high "审查这些改动"
future run @README.md "总结这个文件"
echo "一些文本" | future run "把这段文本整理一下"
```

### `tools` —— 列出与调用工具

```bash
future tools list
future tools call <name> --args '<json>'
future tools call <name> --stdin
future tools call <name> --args '<json>' --output result.png
```

当工具需要文件内容时,文件路径参数会被自动转换。

### `skills` —— 管理能力包

```bash
future skills list
future skills install <name>
future skills uninstall <name>
```

### `channel` —— 聊天渠道桥接(进阶)

把外部聊天平台桥接到 agent(`start` / `stop` / `restart` / `status`)。大多数人用不到。

---

## 小贴士

- **macOS 首次被拦?** 先用右键 →「打开」把 FutureOS 应用打开一次以清除拦截,之后命令行工具也能运行。
- **提示「Connection refused」?** 说明 agent 没运行。执行 `future agent start`,或直接打开桌面应用。

---

## 另见

- [[安装 FutureOS|Installation]] —— 工具随包附带的位置。
- [[技能|Skills]] —— 同样的技能,在应用里管理。
- [[常见问题|FAQ]] —— 常见问题。
