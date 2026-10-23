# 定时任务

定时任务是一件可复用的事：一个 prompt、一个工作目录、一个模型，加上你选定的触发方式。
在左栏打开 **Scheduled tasks（定时任务）** 即可创建、查看运行记录、编辑或停用。

> **定时任务以完全权限运行，从不询问审批。** 它适合你已经确认可以无人值守运行的工作。
> 如果任务需要处理不可信内容，请给它一个专用文件夹，而不是整个主目录。

## 创建

点 **New scheduled task（新建定时任务）** 并填写：

- **Name（名称）** 与 **Prompt** —— 任务要做什么。prompt 带版本，编辑会保留被替换的那一版。
- **Working directory（工作目录）** —— 任务工作的文件夹。
- **Model** 与 **Thinking level（思考级别）** —— 来自 **Models** 页已启用的模型。
- **Conversation type（会话类型）** —— 归档到工作目录下的 **Workspace** 会话（默认），
  或使用临时工作区的 **Chat** 会话。
- **Conversation after the run（运行后如何处理会话）** —— **Keep** 保留（默认），或
  **Delete** 在运行结束后删除。删除会话后，该次运行的状态、摘要和完整回答仍然保留。
- **Time zone（时区）** —— 触发时间按哪个时钟读取。留在本机时区即跟随实际执行任务的电脑；
  读不到时使用 `Asia/Shanghai`。
- **Trigger（触发方式）** —— 可选：
  - **Once on a date（指定日期一次）** —— 在某个日期时间运行一次。
  - **Every N minutes（每 N 分钟）** —— 锚定在固定网格上，不会漂移。
  - **每天 / 每周 / 每月 / 每年**，按本地钟点时间运行。没有该日期的月份在当月最后一天运行，
    因此“每月 31 日”的任务在四月会在 30 日运行。
  - **Manual (legacy)（手动，旧版）** —— 只保留给本来就是手动的任务；新任务必须选择真实触发方式。

**Save** 保存；**Enable / Disable** 只开关触发，不删除任何东西；**Delete** 停止任务，但保留
运行记录和它已经创建的会话。

## 运行记录

右侧列表是运行台账：每次运行的状态（排队、运行中、完成、失败、跳过）、使用的 prompt 版本
和结果摘要。

- **Open** 跳到该次运行创建的会话。
- 即使会话已被删除，运行的完整回答仍然可读。

任务由运行中的桌面应用（或 `future host`）执行。FutureOS 关闭时，到点的任务会等待，
下次打开后再运行——手机或 CLI 可以排队请求，但它们本身不执行任务。

## 命令行

```bash
future task list [--all]
future task add --name "Nightly review" --prompt "Review new issues" \
  --cwd ~/project --model <model> --daily --time 09:00
future task run "Nightly review" --wait
future task runs "Nightly review"
future task output <run-id>
```

`future task` 管理同一批任务：其他触发方式用 `--every`、`--at`、`--weekly`、`--monthly`、
`--yearly`，时钟用 `--timezone`，会话用 `--session new|existing` 与
`--session-retention keep|delete`，版本历史用 `prompt log` / `prompt apply` / `prompt revert`。
见 [[命令行工具(future)|CLI]]。

## 另见

- [[使用 FutureOS|Using-FutureOS]] —— 应用布局与批准模式。
- [[审批与沙箱|Sandbox]] —— “完全权限”意味着什么。
- [[命令行工具(future)|CLI]] —— `future task` 完整说明。
