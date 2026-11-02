# Rust 诊断日志

[English](logging.md)

Rust 入口统一通过 `future-logging` 使用 `tracing`、`tracing-subscriber` 和
`tracing-appender`。文件为 JSON Lines，终端保持可读文本。本次不包含前端控制台采集、
运行时临时诊断切换。

## 文件位置

以下路径相对于实际 FutureOS 数据根，默认 `~/.future`，绝对 `FUTURE_HOME` 可覆盖整个根：

| 组件 | 路径 |
|---|---|
| Desktop/Tauri/应用宿主 | `app/logs/desktop.log` |
| Agent，包含内嵌和独立运行 | `agent/logs/agent.log` |
| 前台 `future host` | `app/logs/host.log` |
| Channel | `channels/logs/channel.log` |
| TUI | `tui/logs/tui.log` |
| 普通 CLI 诊断 | `cli/logs/cli.log` |

`future logs status [--json]` 列出标准文件位置和当前 CLI 构建的默认档位，不代表读取了
其他运行进程的实际过滤条件。`future logs path --component agent` 打印单个标准路径；
其他组件名为 `desktop`、`host`、`channel`、`tui`、`cli`。

内嵌 Agent 的文件写入器在取得 Agent 实例锁后注册，覆盖服务清理阶段，并在正常退出时
刷新缓冲。Desktop 连接外部 Agent 时不打开对方的文件。每个文件独立持有 OS 锁；并存的
TUI/CLI/Channel 使用旁边的 `<组件>.<pid>.log`，避免轮转另一个进程正在使用的文件。
锁文件存在不等于进程活着，以 OS 锁为准。

## 默认档位和保留

发行版本以 `0` 开头时使用 Detailed，记录业务 DEBUG；正式版本使用 Normal，记录业务
INFO。第三方库在两档中都默认 WARN。判断依据是发行渠道，不是 Cargo 优化档位。
频繁的 LLM 请求、逐步用量记录降为 DEBUG；运行结果、警告和错误在正式版保留。

`RUST_LOG` 使用标准 EnvFilter 语法，在进程启动时覆盖过滤条件。例如
`RUST_LOG=warn,futureos_lib::commands::history_transfer=trace`。
本次没有修改运行中进程档位的 CLI 命令。独立 Agent 的既有 `--verbose` 选择 Detailed；
`--log-file [PATH]` / `FUTURE_AGENT_LOG_FILE` 可覆盖文件路径，默认启用文件诊断。

每条文件记录带时间、级别、模块、组件、PID、进程实例、版本、结构化事件字段和 span。
CLI 正常命令输出与日志分开；TUI 仅落盘，避免覆盖全屏终端。流式 thinking/回复继续作为
终端输出和会话数据保存，不复制到诊断文件。

单文件达到 8 MiB 后轮转：Detailed 最多保留 11 份备份，含当前文件约 96 MiB；Normal
保留两份，约 24 MiB。打开文件时清理过期备份，分别为七天、三天。每次启动会轮转旧文件，
升级前的文本日志也会保留在备份中，新文件保持 JSONL。宿主还保留一份上次 1,000 行尾部
`*.prev.log`。Agent 的 `--log-max-lines N` 可额外设置按行轮转，默认零，仅使用容量限制。
每个后台写入器队列有 4,096 项上限；队列丢弃数在退出时尝试输出到 stderr。日志文件失败
不阻止应用启动；强制结束进程可能丢失尚未写出的记录。panic 保留独立的
`app/logs/crash.log` 尽力落盘路径。

## 存储与历史读取

PR #1393 引入的 schema/content 架构在操作边界记录诊断，不为日志额外扫描历史或序列化正文：

- Agent/App 数据库准入、原始与目标版本、升级快照创建/验证、迁移开始/提交/失败及耗时。
- 存储启动阶段、中断运行恢复、投影失效/重建、活动时间修复。
- 历史分页游标、缓存命中、投影重建、返回条数、耗时、失败及至少一秒的慢读取。
- 原生历史传输排队、创建/准备、大小/块数、到期、完成、主动释放、读取失败上下文。
  逐块 offset/快照年龄为 TRACE；成功生命周期为 DEBUG。原有返回错误码和传输有效期保持现状。

按 `event`、会话/运行 ID、`transfer_id` 关联 Agent 与宿主日志。
`history_transfer_retired` 在快照消失时记录 `completed`、`expired` 或 `released`，
后续读不到同一 ID 时可从先前事件确定原因。新增事件记录元数据，不记录提示词、thinking、
工具参数/结果正文或凭据。
