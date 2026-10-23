# Host 内部结构地图

> [English](host.md)
>
> **本文是地图，不是设计文档。** 它只说明 `host/` 里各部分在哪里、改动前该先读什么；
> 设计取舍（如果已有文档）在每行所指向的那篇里。

`host/` 构建 `future-host`——两个入口共用的无 GUI 应用后端：Desktop GUI（`futureos`）与
前台命令 `future host`（`cli/src/headless/`）。它拥有 `~/.future/app/app.db` 这个 Desktop
存储、Agent 托管与桥接层、远程、定时任务和关闭流程。crate 头注释写明了边界：入口负责运行
时创建与呈现，本库不依赖 Desktop、Tauri 或 CLI（`host/src/lib.rs:1-4`）。

`host/src/lib.rs` 声明 41 个模块——35 个 `pub mod` 与 6 个私有模块
（`error`、`git_diff_parse`、`proc`、`home`、`remote_host`、`remote_platform`）；
`test_support` 受 `test` / `test-support` feature 门控。下表覆盖全部。

注意这里的两种文件布局。`store`、`shadow_review`、`shutdown`、`tasks`、`agent_host`、
`test_support` 的模块根在 `host/src/` 下，例如 `host/src/store.rs` 加 `host/src/store/`
里的子模块；`agent_bridge`、`agent_providers`、`application`、`remote`、`remote_host`、
`remote_peer` 的模块根在目录里，例如 `host/src/agent_bridge/mod.rs`。

**GUI 与这个 crate 的分界。** `desktop/src-tauri` 保留薄转发模块和纯 GUI 部分：
`desktop/src-tauri/src/store.rs` 就是 `pub(crate) use future_host::store::*`，
`desktop/src-tauri/src/agent_bridge/mod.rs` 再导出 `future_host::agent_bridge::*`，
只额外加呈现层。存储数据、Agent 桥接或远程的改动属于这里，不属于那边。

## 与 Agent 通信

| 模块 | 职责 | 从这里读起 |
|---|---|---|
| `agent_client.rs` | gRPC 客户端构造、端点发现、错误映射 | `host/src/agent_client.rs` |
| `agent_commands.rs` | 构造 `RpcCommand`，把 `RpcResponse` 转成 `Result` | `host/src/agent_commands.rs` |
| `agent_host.rs`（+ `agent_host/`） | GUI/headless 共用的 Agent 服务所有者：启停阶段、贯穿拆卸过程的单例锁 | `host/src/agent_host.rs` |
| `agent_bridge/` | 应用侧桥接：消费一次 prompt 的事件流、每会话 observer、持久化投影、会话/审批/运行控制命令、headless prompt 管线 | `host/src/agent_bridge/mod.rs`、`host/src/agent_bridge/stream.rs`、`host/src/agent_bridge/observer.rs`、`host/src/agent_bridge/persist.rs`、`host/src/agent_bridge/headless.rs` |
| `agent_events.rs` | 宿主事件出口；observer 只认识这个 sink，不认识远程传输 | `host/src/agent_events.rs` |
| `agent_providers/` | 为 Providers 设置页读写 Agent 的模型与 provider 配置 | `host/src/agent_providers/mod.rs` |
| `request_scope.rs` | 每宿主的命令准入与精确运行归属 | `host/src/request_scope.rs` |

## 本地数据与 Desktop 操作

| 模块 | 职责 | 从这里读起 |
|---|---|---|
| `store.rs`（+ `store/`） | `~/.future/app/app.db`：schema、迁移、threads、runs、approvals、artifacts、评审快照、工作区文件 | `host/src/store.rs`、`host/src/store/schema.rs`、`host/src/store/migrations.rs` |
| `conversations.rs` | 「删除一个会话」在侧边栏、工作区删除、任务三种入口下是同一件事 | `host/src/conversations.rs` |
| `workspaces.rs` | 工作区应用服务 | `host/src/workspaces.rs` |
| `files.rs` | 文件/附件业务与路径访问边界 | `host/src/files.rs` |
| `git_review.rs` | 工作树 diff 评审 | `host/src/git_review.rs` |
| `shadow_review.rs`（+ `shadow_review/`） | 「上次运行的改动」：快照进一个独立的裸 git 仓库 | `host/src/shadow_review.rs`、`host/src/shadow_review/policy.rs`、`host/src/shadow_review/snapshot.rs` |
| `git_diff_parse.rs` | 按文件拆分 unified patch、解析 `--numstat`，两条评审路径共用 | `host/src/git_diff_parse.rs` |
| `auth_store.rs` | 严格原子、`0600` 地读 `~/.future/agent/auth.json`；仅在不可达时作为本地兜底写入 | `host/src/auth_store.rs` |
| `config_io.rs` | `models.json`、`auth.json`、`approval_rule.json` 的严格读 + 原子写 | `host/src/config_io.rs` |
| `approval_rules.rs` | 把工作区级 `allow` 规则追加到 `${WORKSPACE}/.future/approval_rule.json` | `host/src/approval_rules.rs` |
| `application/` | 入口适配层背后的共享应用操作（审批、评审、运行、技能、会话线程） | `host/src/application/mod.rs`、`host/src/application/runs.rs` |

## 远程

| 模块 | 职责 | 从这里读起 |
|---|---|---|
| `remote/` | 宿主角色：设备配对**进入**本机。配对、运维命令、自建 relay、监督器、诊断 | `host/src/remote/mod.rs`、`host/src/remote/pairing.rs`、`host/src/remote/control.rs`、`host/src/remote/relay.rs`、`host/src/remote/supervisor/mod.rs` |
| `remote_host/` | 已配对设备命令的业务侧：目录、会话、文件、精简分页 | `host/src/remote_host/mod.rs`、`host/src/remote_host/business/mod.rs`、`host/src/remote_host/catalog.rs` |
| `remote_peer/` | 客户端角色：本机**配对出去**连另一台 Desktop | `host/src/remote_peer/mod.rs` |
| `remote_platform.rs` | Remote 两个角色与 relay 控制共用的 HTTP 客户端 | `host/src/remote_platform.rs` |

## 进程生命周期与共享服务

| 模块 | 职责 | 从这里读起 |
|---|---|---|
| `instance.rs` | 每个数据目录只有一个后端；文件与其 OS 锁在进程生命周期内保持打开 | `host/src/instance.rs` |
| `startup.rs` | 启动各阶段，以及让卡住的启动可诊断的看门狗 | `host/src/startup.rs` |
| `shutdown.rs`（+ `shutdown/`） | 所有退出/重置/更新调用共用的唯一进程级停止操作 | `host/src/shutdown.rs`、`host/src/shutdown/services.rs` |
| `runtime.rs` | 共享任务归属与有界清理；入口负责安装 runtime | `host/src/runtime.rs` |
| `scheduler.rs` | 进程级调度原语与固定间隔任务（应用更新、Future 余额、Future 模型） | `host/src/scheduler.rs` |
| `events.rs` | 已提交后端变更的呈现出口 | `host/src/events.rs` |
| `environment.rs` | 共享的进程准备与诊断 | `host/src/environment.rs` |
| `home.rs` | home 目录与 `future_home_root()`（识别 `FUTURE_HOME`） | `host/src/home.rs` |
| `device_identity.rs` | 安装级稳定 `device_id` | `host/src/device_identity.rs` |
| `build_info.rs` | 构建期注入的版本与分发通道 | `host/src/build_info.rs` |
| `cli_tool.rs` | 发现 CLI 二进制；绝不执行空占位符 | `host/src/cli_tool.rs` |
| `proc.rs` | 子进程辅助（Windows 上不弹控制台窗口） | `host/src/proc.rs` |
| `error.rs` | 统一的应用 `Error` | `host/src/error.rs` |
| `run_error.rs` | 把 Agent 的原始错误串分类成 UI 类别 | `host/src/run_error.rs` |

## 账号、任务与引导

| 模块 | 职责 | 从这里读起 |
|---|---|---|
| `future_login.rs` | FutureGene 设备码 OAuth；无状态，轮询由调用方负责 | `host/src/future_login.rs` |
| `future_platform.rs` | 从 `auth.json` 解析平台/模型 API 根地址 | `host/src/future_platform.rs` |
| `tasks.rs`（+ `tasks/`） | Desktop 侧 `future-tasks` 的 tick 循环与 `Executor` 接线 | `host/src/tasks.rs` |
| `skills.rs` | Skills 引导页使用的免鉴权平台指南 | `host/src/skills.rs` |
| `test_support.rs`（+ `test_support/`） | 库与入口测试共用的测试夹具 | `host/src/test_support.rs` |

## 常见改动 → 先读什么

| 想改…… | 先读 |
|---|---|
| Desktop 数据模型或某一行的存储 | `host/src/store/schema.rs`、`host/src/store/migrations.rs`，再读 [ER](desktop/ER.zh-CN.md)与 [SQLite 迁移](../architecture/sqlite-migration.zh-CN.md) |
| 一次运行从 Agent 流里持久化什么 | `host/src/agent_bridge/stream.rs`、`host/src/agent_bridge/persist.rs` |
| 发送 prompt 的路径（GUI、手机、任务） | `host/src/agent_bridge/prompt.rs`、`host/src/agent_bridge/headless.rs`、`host/src/application/runs.rs` |
| 审批决定或审批界面数据 | `host/src/agent_bridge/approval.rs`、`host/src/application/approvals.rs`、`host/src/approval_rules.rs`，再读[自动审批](desktop/AUTO_APPROVAL.zh-CN.md)与 [SANDBOX](desktop/SANDBOX/COMMON.zh-CN.md) |
| 手机能向本机要什么 | `host/src/remote_host/business/mod.rs`、`packages/remote-server`，再读 [CONNECTION](desktop/CONNECTION.zh-CN.md)与 [REMOTE_E2EE](desktop/REMOTE_E2EE.zh-CN.md) |
| 配对或自建 relay | `host/src/remote/pairing.rs`、`host/src/remote/relay.rs`，再读[自建 relay 指南](../guide/self-hosted-relay.zh-CN.md) |
| 启动、锁或关闭行为 | `host/src/startup.rs`、`host/src/instance.rs`、`host/src/shutdown.rs`，再读 [Agent 启动仲裁](../architecture/agent-supervision.zh-CN.md)与 [Host 模式指南](../guide/desktop-host.zh-CN.md) |
| 新增一个周期维护任务 | `host/src/scheduler.rs` 以及驱动它的入口（`desktop/src-tauri/src/scheduler/mod.rs`） |
| app settings 的键或默认值 | `packages/app-settings/src/lib.rs`（唯一权威）与 `host/src/store/app_settings.rs` |
| 定时任务行为 | `host/src/tasks.rs`，再读[定时任务](../architecture/tasks.zh-CN.md) |
| 「上次运行的改动」diff | `host/src/shadow_review.rs`、`host/src/git_diff_parse.rs` |
| 技能引导文案 | `host/src/skills.rs`；真正的技能安装属于 Agent（`agent/src/skills/mod.rs`） |

## 已覆盖本 crate 某一部分的既有文档

- [Headless 包开发](../architecture/headless-package-development.zh-CN.md) — `future-host` 如何抽出、达到何种验收
- [Host 模式指南](../guide/desktop-host.zh-CN.md) — 运维 `future host`
- [Desktop Agent 指南](../guide/desktop-agent.zh-CN.md) — 内嵌/外置托管、日志、更新恢复
- [ER](desktop/ER.zh-CN.md) — Desktop 产品数据模型与 Agent 存储
- [CONNECTION](desktop/CONNECTION.zh-CN.md)、[REMOTE_E2EE](desktop/REMOTE_E2EE.zh-CN.md) — 远程传输与端到端加密
- [Remote 包开发](../architecture/remote-package-development.zh-CN.md) — 远程包边界（`packages/remote-server`、`packages/remote-client`、`packages/remote-protocol`、`packages/remote-crypto`）
- [定时任务](../architecture/tasks.zh-CN.md) — 本 crate 驱动的 `future-tasks` 内核
- [SQLite 迁移](../architecture/sqlite-migration.zh-CN.md) — `app.db` 与 Agent schema 策略
- [Agent 启动仲裁](../architecture/agent-supervision.zh-CN.md) — 各入口之间的实例锁归属
- [目录布局](../guide/directory-layout.zh-CN.md) — 本 crate 写入的每个 `~/.future/` 路径

## 不在地图范围内

Agent 自身的模块另有地图：[agent/README.zh-CN.md](agent/README.zh-CN.md)。GUI 剩余的模块
（`desktop/src-tauri/src/terminal/`、窗口与菜单、平台电源辅助、更新事务）仍在 `desktop/src-tauri`，由
[Desktop 文档](desktop/PRODUCT.zh-CN.md)描述，不在这里。
