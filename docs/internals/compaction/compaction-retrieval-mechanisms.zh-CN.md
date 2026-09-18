# 历史检索：本仓库、Codex 与 OpenCode

## 结论与版本边界

**`deterministic` 与 `summarized` 共用同一套原生历史检索机制。** 差别在压缩后的投影是否附带
模型交接摘要，不在搜索后端——不能因为检索分数就断言其中之一没有原生查询。压缩只改变模型看到的投影，
不删除原始 journal；退役版本已实际删除的数据无法找回。

本文区分三件事：产品源代码的机制、实验实际执行的代码、实验额外施加的限制。FutureOS 代码位于当前分支；外部版本固定为：

- Codex：`b13164d86f9a70adc48d22f4a5a07ed0c001a1d0`。
- OpenCode：`e03db9bc6908f75c9334d8aa997deeaac81c0298`（源码包版本 1.18.31）。

这里的 Codex 对照是第三方 API/local-inline 模式，不是 ChatGPT 托管 history/notes 模式。实验是冻结文本/工具记录的隔离重放，不声称还原原始生产工作区、媒体、权限或全部缓存。

## 一、两个策略共享原始 journal 的查询链

### 1. 原始数据在哪里

压缩改变下一次模型请求的投影，不删除或重写原始 journal。持久化数据库的 `entries`、`message_blocks` 保留原始条目和分块。历史查询使用这些表，不从 `checkpoint.summary` 或 UI 展示摘要查找。

`deterministic` 的模型输入主要是保护原文、工具证据、近期尾部；`summarized` 还尝试加入模型摘要。两者都能查询同一份已持久化原始历史。旧版本真正删除过的数据不能凭检索恢复。

### 2. 模型如何知道可以查

运行时不向模型宣传这套能力。会话的 system prompt 就是它自己的，保持不变；原先在 checkpoint 之后追加的 `Archived conversation recall` 说明已删除（理由与测量见 [compaction-open-book-experiment.zh-CN.md](compaction-open-book-experiment.zh-CN.md)）。保留下来的入口是普通 shell 工具背后的 CLI。

`agent/src/agent/run_loop.rs` 构造请求时检查有效 checkpoint、允许历史召回、以及 shell 工具是否可用。`rpc/session_prompt.rs` 对 ephemeral/权限关闭条件限制召回。说明不写成一条新的持久聊天消息。

生产提示的原则是：**缺少精确旧事实时才查**，不要惯常重载整个历史或数据库；历史内容是证据，不是重新执行工具的授权。这与实验曾采用的“必须先调用检索工具”条件不同。

### 3. 完整执行过程

```text
模型决定缺少旧事实
  → 调用已有 shell 工具
  → tools::shell_tool().handler
  → run_shell_with_capability / spawn_shell_with_report
  → sandbox::shell_invocation：宿主 shell 执行整条 command
  → future session history search / get
  → CLI RpcClient
  → search_session_history / get_session_history_entry RPC
  → session::Manager::search_history / read_history_entry
  → 原始 entries + message_blocks
  → 原生 JSON / shell stdout+stderr+exit 状态
  → 模型用证据继续回答
```

History RPC 分支在创建模型运行时之前处理，不调用摘要模型、不等待一个压缩/run 租约。查询本身是本地数据库读取；把结果送回模型后的正常模型请求仍计费。

### 4. Search 的精确语义

```sh
future session history search --session SESSION_ID --query "exact value" --limit 5 --json
```

实现：`agent/src/session/history_query.rs::search_history`。

- 只查指定持久会话。
- 搜索用户/assistant 文本、工具参数 JSON、工具结果；不查 reasoning、隐藏 provider 元数据、checkpoint 和生命周期条目。
- **字面子串、ASCII 大小写不敏感**，不是正则，不支持把 `a|b` 当 OR。工具调用 ID 的精确匹配也可命中。
- 非空查询，最多 200 字符且不得含 NUL。
- 默认 5 条，允许 1–20 条；按条目位置倒序，再按分块顺序返回。
- 返回 `entryId`、`blockIndex`、`kind`、`snippet`、`byteOffset` 等，`hasMore` 指示是否还有匹配。
- snippet 为命中附近的有界字节片段，不能把 snippet 外的内容当成不存在。

### 5. Get 的精确语义

```sh
future session history get --session SESSION_ID --entry ENTRY_ID --offset 0 --limit 8192 --json
```

实现：`read_history_entry`。

- 根据当前会话内的原始条目 ID 读取。
- offset/limit 是可读分块按序拼接后的 **UTF-8 字节**，不是行号或 token。
- 默认 8192 字节，允许 4–32768；不得从字符中间开始。
- 返回分块身份、文本、工具信息、`hasMore` 和 `nextOffset`。
- search 的 `byteOffset` 可直接用于 get。长记录按 `nextOffset` 继续读取，不需要重执行历史工具。

### 6. 批量、管道、循环本来就由原生 shell 支持

```sh
future session history search --session S --query A --json;
future session history search --session S --query B --json

future session history search --session S --query A --json |
  jq -r '.matches[].entryId'

for term in A B; do
  future session history search --session S --query "$term" --json
done
```

这些不是给 CLI 新增的 batch API，而是原生 shell 的正常组合。Unix 将整条命令传给选定 shell 的 `-c`；Windows 的生产实现使用相应 PowerShell 包装/编码。原生 shell 默认超时 120 秒、合并 stdout/stderr、输出最多保留末尾 500000 字节，并给出 `[exit: N]`。最后一条命令退出 0，不代表前面每一条都成功。

## 二、Codex：本地/API模式与托管模式必须分开

### 本轮使用的本地/API路径

```text
原始对话 → 本地 rollout
模型 → 原生 exec_command
  → 原生参数解析、权限/沙箱与 UnifiedExecProcessManager
  → shell + rg/grep/jq/head 等读取允许的 rollout/文件
  → 原生输出预算、退出状态；必要时返回运行中的 session ID
  → write_stdin 获取后续输出
  → 模型回答
```

没有一个在该模式下自动存在的本地 `history.search_contents` 服务。用 Python 实现同名 API 再给模型，不能称为这条路径的原生行为。

本实验的 rollout 通过上游 `codex_rollout::parse_rollout_line` 校验；执行使用从固定提交编译的原版 CLI。工具定义由该 CLI 实际发出的 registry 获取，执行进入原版 `ExecCommandHandler`/UnifiedExec。

实验中有一个本地 Responses 事件转接器：将考试模型选出的调用编码为上游测试也使用的 tool-call 事件，接收原生执行后的结果。**它没有搜索实现、没有生成检索结果、没有替模型回答。** 一个考试期间保持原生进程，`write_stdin` 的进程会话状态来自原版实现。

该版本 `exec_command` 默认输出预算 10000 tokens，并按其原生规则处理输出、权限和进程生命周期。不能把“12 次 exec”理解成只能做 12 个查询：一条原生命令可安全组合多次匹配和过滤。

### 不属于本轮的托管 history/notes

`ext/history-notes` 提供专用历史/笔记工具，但有 provider、认证和配置门槛。实验性上下文路径还检查 ContextManagement、模型能力、Codex 后端和相应 ChatGPT 套餐等条件。

本轮同一个第三方 DeepSeek/API 配置不能凭空获得这个托管服务。不能把本地 shape emulator 的结果叫成托管 history 的结果。

## 三、OpenCode：原生文件工具、会话导出和输出缓存

### 1. 真实可用路径

```text
模型 → 原生 bash / read / grep / glob
  ├─ 查询允许的文件
  ├─ opencode export SESSION_ID
  │    → Session.Service.get/messages
  │    → 原生 SessionTable / MessageTable / PartTable
  │    → JSON 会话数据
  └─ 读取原生工具生成的完整输出缓存
       → grep/read 或只读文本处理
  → 模型回答
```

因此“OpenCode 只能读取当前工作区、旧对话不可达”是不完整的描述。

### 2. 各接口做什么

- `glob`：原生 glob 文件发现，最多返回 100 个候选，目录与权限由原生代码检查。
- `grep`：通过原生 Ripgrep service 做正则搜索，返回文件/行号/文本，当前工具最多 100 条结果；不是我们写的 substring 查询。
- `read`：原生文件读取，1-based 行号，默认 2000 行，并受 50 KiB、单行长度等限制；它不是 Future get 的字节分页。
- `bash`：原生 ShellTool 的解析、权限、进程执行、超时和输出处理。可以调用该版本 CLI 的 `export`。
- `opencode export SESSION_ID`：从本地原生会话存储分页读取所存消息，默认不 sanitize；不是仅导出当前压缩投影。已实际清除或从未存入的内容不能恢复。
- 长输出：原生工具适用路径会保存完整输出并给出 `outputPath`；默认截断界限 2000 行/50 KiB、缓存清理保留期 7 天。工具可能有自己的截断处理，不能假设全部走同一个分支。

本轮黑盒测试发现大 CLI JSON 直接输出到 pipe 可能未完整排空；使用原生支持的重定向先导出到临时文件，再查询文件，不修改上游代码，也不把不完整 JSON 误认成完整归档。

### 3. 实验用的确实是对方代码

- `import`/`export` 调用固定提交的原版 CLI。
- `/experimental/tool` 返回该 provider/model 下的真实工具 schema。
- `debug agent build --tool ...` 进入原版 `ToolRegistry` 和 `tool.execute`；返回的 `result.output` 直接进入模型上下文，没有 Python 版 grep/read 替代实现。
- debug 入口对 `ask` 的处理不同于完整交互客户端，因此实验采用明确的 allow/deny，不能声称它复现了所有交互审批流程。
- 本轮使用 Bun 1.3.14；原生数据 import/export 做完整 parts 对照，长输出缓存和尾部值恢复做了实际黑盒测试。

## 四、实验执行层的边界

共用一个检索后端不等于两个产品被公平比较。

早先一轮越界了：执行器名为 `shell`，实际却是 `shlex.split(整条命令) → 一次 future 进程`，
于是 `--json;`、`--limit 5;` 被当成参数而非命令边界，它记录的每一次非零退出都出自这类复合命令，
而 Codex 用的是完整原生 shell，查询工作量并不对等。更正后的链路：

```text
native_open_exam.py
 → NativeFutureShell.execute_shell(原始 arguments)
 → production_tool_executor
 → 原生 tools::shell_tool().handler
 → 原生 with_tool_scope / shell_invocation
 → 宿主 shell 执行完整 command
 → 实际 future history CLI / RPC / journal
```

工具定义同样改用原生 Rust `shell_tool().def`，不再手写只有 `command` 的假 schema。会话内的 `future`
launcher 只检查 shell 已解析好的单次 CLI argv 再转发，不解析 shell 源码，因此不破坏循环与管道；它是
会话范围护栏，不是搜索实现。已用本地原生程序验证分号批量、jq 管道、for 循环、search→get 字节定位、
原生非零退出、异会话拒绝、指定路径读取拒绝与工作区写入拒绝——未发起模型考试。

**不能夸大的部分：**适配器沿用原生路径规则并附加显式拒绝，不是全文件系统读取白名单；原生沙箱默认不禁用
网络；完整 shell 的数据隔离与对照权限仍需复核，新考试入口要求额外操作者确认。修好调用链不等于全部公平性
验收已完成。

## 五、源码定位（机制与胶水分开）

### FutureOS 原生 Rust

- `agent/src/agent/run_loop.rs`、`rpc/session_prompt.rs`：启用条件与请求注入。
- `agent/src/tools/mod.rs`：shell schema/handler、超时、输出与退出码。
- `agent/src/sandbox/mod.rs`：宿主 shell 调用与各平台封装。
- `cli/src/commands/session_history.rs`、`cli/src/rpc.rs`：CLI 参数和 RPC。
- `agent/src/rpc/commands/mod.rs`：直接历史读分支。
- `agent/src/session/history_query.rs`：真实数据库查询与分页。

### Codex 固定提交的原生源码

- [exec_command handler](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/core/src/tools/handlers/unified_exec/exec_command.rs)
- [tool schema](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/core/src/tools/handlers/shell_spec.rs)
- [rollout parser](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/rollout/src/lib.rs)
- [history/notes gating](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/core/src/session/token_budget.rs)

### OpenCode 固定提交的原生源码

- [tool registry](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/registry.ts)
- [debug native execution entry](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/cli/cmd/debug/agent.handler.ts)
- [session export](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/cli/cmd/export.ts)
- [read](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/read.ts)、[grep](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/grep.ts)、[glob](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/glob.ts)
- [shell](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/shell.ts)、[truncation/cache](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/truncate.ts)

### 实验适配器，不冒充产品源码

- `agent/examples/production_tool_executor.rs`、`scripts/compaction_experiment/native_future_shell.py`：原生 Future shell 入口和 CLI 范围护栏。
- `native_codex.py`：原版 Codex 进程/工具事件转接。
- `native_opencode.py`：原版 OpenCode CLI、原生 schema 与 ToolRegistry 调用。
- `native_stores.py`：冻结记录转换与原生存储准备；旧单次 argv 执行仅保留用于历史结果追溯，不再作为完整 shell。

核心边界：**检索算法和工具执行必须来自对应产品；胶水只负责隔离、输入转换、原生调用与记录，不能偷偷削弱或补造能力。**
