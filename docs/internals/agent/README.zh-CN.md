# Agent 内部结构地图

> [English](README.md)
>
> **本文是地图，不是设计文档。** 它只说明 `agent/` 里各部分在哪里、改动前该先读什么，
> 不讨论设计取舍：每一行指向真正解释设计的文档，或模块自己的 `//!` 头注释。

`agent/` 构建 `future-agent`——拥有 `~/.future/agent/` 的 gRPC 后端。这个 crate 同时是
库：`agent/src/main.rs` 只是 `future_agent::cli::run_from_args` 的 11 行包装，
`future agent` 运行的也是同一个入口，因此独立二进制与内嵌子命令不会分叉
（`agent/src/cli.rs:1-6`）。

`agent/src/lib.rs` 声明 22 个公开模块，外加私有的 `system_one`。下表覆盖全部。

## 一次请求的路径

1. 客户端调用 `FutureAgent` gRPC 服务（`grpc/`），所有命令统一经
   `handle_command_internal` 分发（`agent/src/rpc/commands/mod.rs`）。
2. run-control 命令把这次运行交给 `runtime::SessionRuntime`，由它独占该会话的取消状态
   和唯一的后台任务槽位。
3. 轮次本体在 `agent/src/agent/run_loop.rs`：模型流 → 工具调用 → 循环，期间发出 `RunEvent`
   （`agent/src/agent/events.rs`）。
4. 模型访问是 `llm/`（HTTP + SSE，每种线协议一个 adapter）；内置工具是 `tools/`；
   工具进程与文件访问由 `sandbox/` 把关。
5. 历史由 `session/` 写入 `~/.future/agent/agent.db`；模型看到的是替代完整历史的投影，
   由 `compaction/` 生成。

## 顶层模块

| 模块 | 职责 | 从这里读起 |
|---|---|---|
| `agent/` | 轮次循环：流式调用模型、执行工具调用、循环；对外发布 `RunEvent` | `agent/src/agent/run_loop.rs`（`run_streaming_with_messages`）、`agent/src/agent/events.rs` |
| `types/` | 核心协议类型（`AgentMessage`、`ContentBlock`、`LLMProvider`、`ToolDef`） | `agent/src/types/mod.rs` |
| `engine/` | 把 provider + model + tools + session + loop 绑成一次运行 | `agent/src/engine/mod.rs`；在 `agent/src/service/startup.rs` 中构造 |
| `service/` | 与宿主无关的启动、服务与关闭，持有作用域内的实例锁 | `agent/src/service/mod.rs`（`AgentService::prepare`、`AgentHandle`） |
| `grpc/` | 基于 tonic 的 `FutureAgent` 服务，默认本地 IPC，可选 TCP | `agent/src/grpc/mod.rs`（`AgentListener`） |
| `rpc/` | 命令分发（`agent/src/rpc/commands/`，按领域分文件）、事件投递（`agent/src/rpc/broadcaster/`）、控制面版本号、中断运行恢复（`agent/src/rpc/resume.rs`） | `agent/src/rpc/commands/mod.rs`、`agent/src/rpc/broadcaster/mod.rs`、`agent/src/rpc/control_plane.rs`、`agent/src/rpc/resume.rs` |
| `runtime/` | 每会话的执行归属：生命周期、取消确认、队列预算 | `agent/src/runtime/session_runtime.rs`、`agent/src/runtime/scheduler_queue.rs` |
| `session/` | 持久化转录：entry、有序 SQLite worker、投影、fork、历史分页、修复 | `agent/src/session/mod.rs`（子模块清单）、`agent/src/session/persistence.rs` |
| `compaction/` | 替代完整历史发送的有界投影 | `agent/src/compaction/mod.rs`、`agent/src/compaction/budget.rs`、`agent/src/compaction/durable.rs` |
| `llm/` | HTTP + SSE 模型客户端与逐协议 adapter | `agent/src/llm/mod.rs`、`agent/src/llm/adapters/`、`agent/src/llm/schema.rs` |
| `prompt/` | 系统提示组装与 cwd 项目指令 | `agent/src/prompt/mod.rs`（`build_prompt`）、`agent/src/prompt/project_context.rs` |
| `models/` | 模型目录与解析：内置目录、Future 平台缓存、`models.json` | `agent/src/models/mod.rs`、`agent/src/models/future.rs` |
| `config/` | `settings.json` 读写（`Settings`）与 provider 设置 | `agent/src/config/mod.rs`、`agent/src/config/providers.rs` |
| `auth/` | 从 `~/.future/agent/auth.json` 读 API 凭据（回退 `~/.future/agent-app/auth.json`） | `agent/src/auth/mod.rs` |
| `tools/` | 内置 read / write / edit / shell 工具 | `agent/src/tools/mod.rs`（`all_tools`）、`agent/src/tools/shell/` |
| `sandbox/` | 操作系统沙箱后端与基于路径的审批规则 | `agent/src/sandbox/mod.rs`、`agent/src/sandbox/backend.rs`、`agent/src/sandbox/rules.rs` |
| `approval_review/` | 对 Ask 请求分类的模型审查；执行与否由确定性策略决定 | `agent/src/approval_review/mod.rs`、`agent/src/approval_review/policy.rs` |
| `skills/` | 从 `~/.future/agent/skills/` 与 `~/.agents/skills/` 发现技能 | `agent/src/skills/mod.rs` |
| `skill_reco/` | Jev 驱动的 `suggest_skill` 推荐 | `agent/src/skill_reco/mod.rs` |
| `cli/` | `future-agent` 与 `future agent` 共用的 clap 入口 | `agent/src/cli.rs` |
| `logfile.rs` | 日志文件镜像：终端与文件内容保持一致 | `agent/src/logfile.rs` |
| `utils/` | home/session 目录、ID 生成、路径辅助 | `agent/src/utils/mod.rs` |
| `system_one.rs`（私有） | 共享的 Future System One 传输；分类策略由调用方负责 | `agent/src/system_one.rs` |

## 常见改动 → 先读什么

| 想改…… | 先读 |
|---|---|
| 某个内置工具的参数或行为 | `agent/src/tools/mod.rs`（`all_tools`）、起进程看 `agent/src/tools/shell/`、可触碰范围看 `agent/src/sandbox/` |
| 模型看到的系统提示 | `agent/src/prompt/mod.rs`（`build_prompt` 的段落顺序）、`agent/src/prompt/project_context.rs` |
| 压缩多少历史、何时压缩 | `agent/src/compaction/budget.rs`（`trigger_tokens`、`TARGET_HISTORY`），再读[压缩](../compaction/compaction.zh-CN.md)与其[开发地图](../compaction/compaction-development.zh-CN.md) |
| 一个 RPC 命令或事件形状 | `packages/rpc/proto/future.proto`（再跑 `make generate-proto`）、`agent/src/rpc/commands/`、消费侧 `host/src/agent_commands.rs`；见 [RPC](../../architecture/rpc.zh-CN.md) |
| 运行生命周期、取消或排队 | `agent/src/runtime/`、`agent/src/rpc/commands/run_control.rs` |
| 回复可以怎样结束（stop / refusal / filter / pause） | `agent/src/llm/schema.rs`（`FinishReason`）、`agent/src/llm/adapters/`，再读[回复结束语义](../../architecture/response-outcomes.zh-CN.md) |
| 会话存储或 schema | `agent/src/session/database/`、`agent/src/session/sqlite_store/`、`agent/src/session/persistence.rs`；见 [SQLite 迁移](../../architecture/sqlite-migration.zh-CN.md)与 [ER](../desktop/ER.zh-CN.md) |
| 某家 provider 的线协议 | `agent/src/llm/adapters/`、`agent/src/llm/schema.rs` |
| 模型目录或某个 provider 条目 | `agent/src/models/`、`agent/src/config/providers.rs` |
| 沙箱行为或新增规则 | `agent/src/sandbox/`（后端：`agent/src/sandbox/seatbelt.rs`、`agent/src/sandbox/linux/`、`agent/src/sandbox/windows.rs`），再读 [SANDBOX](../desktop/SANDBOX/COMMON.zh-CN.md) |
| 审批审查策略或其提示词 | `agent/src/approval_review/`，再读[自动审批](../desktop/AUTO_APPROVAL.zh-CN.md) |
| Agent 启动、锁或退出 | `agent/src/service/`，再读 [Agent 启动仲裁](../../architecture/agent-supervision.zh-CN.md)与 [Desktop Agent 指南](../../guide/desktop-agent.zh-CN.md) |
| 技能发现路径 | `agent/src/skills/mod.rs`，再读[目录布局](../../guide/directory-layout.zh-CN.md) |
| settings 的键与默认值 | `agent/src/config/mod.rs`，读回路径见[自检](../../guide/self-inspection.zh-CN.md) |

## 已覆盖本 crate 某一部分的既有文档

- [compaction/](../compaction/compaction.zh-CN.md) — 两种策略、参数、请求形状与实验
- [desktop/ER.md](../desktop/ER.zh-CN.md) — Agent SQLite 存储、运行与 entry 模型
- [SANDBOX/](../desktop/SANDBOX/COMMON.zh-CN.md) — 审批语义与三个平台后端
- [architecture/rpc.md](../../architecture/rpc.zh-CN.md) — 本 crate 实现的线协议契约
- [architecture/response-outcomes.md](../../architecture/response-outcomes.zh-CN.md) — 回复结束语义
- [architecture/sqlite-migration.md](../../architecture/sqlite-migration.zh-CN.md) — schema 升级与 JSONL 导入
- [architecture/agent-supervision.md](../../architecture/agent-supervision.zh-CN.md) — 由谁启动 Agent
- [skill_reco/evaluation.md](../skill_reco/evaluation.zh-CN.md) 与 [integration-plan.md](../skill_reco/integration-plan.zh-CN.md)
- [guide/session-history.md](../../guide/session-history.zh-CN.md) — 压缩之后如何把历史读回来

## 不在地图范围内

与本 crate 对话的 Desktop 应用层另有地图：[host](../host.zh-CN.md)。通道桥与 TUI 属于
各自的 crate（`channels/`、`tui/`），这里不描述。
