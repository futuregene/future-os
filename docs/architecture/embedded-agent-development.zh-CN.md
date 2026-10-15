# Desktop 内置 Agent 开发计划

> [English](embedded-agent-development.md)
>
> 设计日期：2026-10-13。源码基线：`88fe90a8`。
> 状态：阶段一至六已实现。macOS 验收见第 10.7 节；用户报告 Windows 全量单测通过，见第 10.8 节。阶段七进行中：原生生命周期 CI、升级 fixture、macOS ad-hoc 打包和 Linux/Windows daily 未签名包验证已通过，见第 10.9–10.13 节。阶段八、正式签名/公证、原生 Linux 沙箱、真实安装升级和性能对比尚未完成。

Desktop 在默认本地端点没有外部 Agent 且成功取得实例锁时，以 Rust library 的方式在自己的进程内运行完整 Agent。兼容外部服务继续连接，不兼容或持锁不可用的服务明确报错。外部 Agent、CLI、TUI、channels 和 loop 继续使用统一 RPC。改动的主要目的，是减少 Windows 启动和升级过程中 Desktop 与 sidecar 生命周期不同步的问题，同时保留实例锁、完整工具能力、持久化和外部客户端接入。

实施应先提取服务层，再接入宿主，最后切换开发、构建与升级链路。第一版保持现有存储格式和业务 RPC，保留打包的 `future` CLI。单独裁剪 CLI、迁移数据库和修改工具权限不属于本次目标。

## 1 产品行为与范围

### 1.1 服务选择

| 场景 | Desktop 行为 | 数据与所有权要求 |
|---|---|---|
| 默认本地端点存在兼容外部 Agent | 握手通过后连接 | 不取得其服务所有权，不重复启动 |
| 外部 Agent 不兼容 | 显示版本或协议冲突并停止 Agent 业务调用 | 不启动内置服务，不自动结束外部进程 |
| 外部 Agent 无响应且实例锁被占用 | 有界等待后报告占用或不可用 | 不删除锁文件，不打开或迁移 Agent 数据库 |
| 默认本地端点没有服务，且成功取得实例锁 | 启动内置完整 Agent | 从初始化到最后一个后台工作退出一直持锁 |
| 多个客户端同时尝试启动服务 | 只有取锁成功者可以初始化 | 其他客户端等待并重新握手；冲突不触发绕锁重试 |
| 用户显式配置 TCP 或其他外部端点 | 只连接该端点，失败时报告错误 | 不静默改连默认本地实例，也不擅自绑定指定地址 |
| 安装器持有升级租约 | 显示暂时不可用并禁止启动 | 租约结束后重新取锁和握手 |

显式端点采用外部服务语义。开发或测试需要内置 TCP 服务时，通过受控启动配置指定监听地址，不把连接失败当作绑定地址的授权。

### 1.2 退出与客户端

内置模式下，退出 Desktop 会结束该 Agent 上所有客户端的任务，包括 CLI、TUI、手机、channels 和 loop 发起的任务。退出提示使用 Agent 的全局活动快照，覆盖运行、排队、等待审批和会修改会话的长时操作。用户选择继续运行时，不提前关闭接单入口。

用户确认强制退出后，先停止接单、取消全部相关工作、等待持久化与清理，再退出。清理超过总期限时，由宿主执行进程级强制退出；不承诺异常退出也能送达取消事件。正常取消必须把明确终态和退出原因发送给客户端。

外部模式下，普通 Desktop 退出不得全局停止外部 Agent，也不得因为 CLI 会话被导入 GUI 而取消它。对 Desktop 自己发起任务的退出处理须基于任务归属，不能仅看本地镜像。安装、卸载对外部服务的影响另见第 7 节。

关闭窗口继续运行、托盘驻留等交互保持现有产品语义。本次绑定的是宿主进程退出，不把关闭窗口自行改成永久后台运行。

### 1.3 保留与排除

保留完整模型、工具、审批、沙箱、技能、会话、压缩、事件、队列和持久化功能；CLI 客户端仍通过 RPC 访问，不直接读写活跃 Agent 的内存或数据库。

保留 `future agent` 和独立 `future-agent` 入口。TUI 可继续在没有服务时启动自己的外部 sidecar。GUI 与 headless 共用服务层，Agent 核心不依赖 Tauri、WebView、GTK 或桌面应用 crate。

第一版不提供单进程多个 Agent 数据目录，不把服务锁丢失变成自动抢占，不在无法确认旧工作已经结束时重启内置服务，也不修改既有 `shutdown` RPC 的“拒绝新 prompt、已有运行继续”语义。

## 2 当前结构与改动依据

| 当前入口 | 已有行为 | 本次需要处理的差异 |
|---|---|---|
| [Agent CLI](../../agent/src/cli.rs)与[退出信号处理](../../agent/src/cli/shutdown.rs) | 取得实例锁，初始化环境、日志和 runtime；必要时退出整个进程 | 服务层不解析进程参数、不设置全局日志、不独立注册退出信号、不调用进程退出 |
| [GUI supervisor](../../desktop/src-tauri/src/agent_supervisor.rs) | 探测、启动并管理 `future agent` 子进程；退出前重置 Windows 权限并 kill | 改为内置句柄与外部连接两种模式，共用完整停机流程 |
| [GUI 启动](../../desktop/src-tauri/src/lib.rs)与[宿主锁](../../desktop/src-tauri/src/instance.rs) | setup 启动后台工作，Tauri build 后取得 GUI/headless 锁 | 依赖 Agent 的工作要延后到宿主锁与兼容性检查之后 |
| [业务连接](../../desktop/src-tauri/src/agent_bridge/client.rs) | 缓存 Channel，检查业务连通性；兼容性另由状态接口报告 | 所有业务请求和自动重连都必须受兼容性与实例身份约束 |
| [headless Agent 管理](../../desktop/src-tauri/src/headless/agent.rs)与[TUI supervisor](../../tui/src/agent_supervisor.rs) | 各自管理外部子进程，现有 Agent 可被复用 | headless 接入共享内置宿主；TUI 保留独立宿主并复用握手与冲突诊断 |
| [SQLite worker](../../agent/src/session/database/mod.rs)、[会话 Manager](../../agent/src/session/manager/mod.rs)、[事件 writer](../../agent/src/rpc/broadcaster/journal_writer.rs) | Agent SQLite 主存储，有序写入、终态事务、后台维护和析构等待 | 需要明确 drain、停止、join 和超时结果，不能靠进程死亡代替正常完成 |
| [RPC shutdown](../../agent/src/rpc/commands/session_lifecycle.rs)、[会话调度](../../agent/src/rpc/session.rs) | shutdown 拒绝新 prompt；完成任务后 scheduler 可继续出队 | 新增宿主停机控制，原子关闭接受与出队，持久化排队取消 |
| [RPC 服务](../../agent/src/grpc/mod.rs)与[全局 broadcaster](../../agent/src/rpc/mod.rs) | 生产服务永久等待；全局控制面事件存在进程级静态对象 | 服务需要显式 shutdown，并结束全局与会话订阅 |
| [任务循环](../../desktop/src-tauri/src/tasks.rs) | 持续调度用户任务，没有明确的宿主停止入口 | 停机前停止生产任务，并收敛 `tasks.db` 的运行状态 |
| [自动更新](../../desktop/src-tauri/src/commands/update.rs) | download_and_install，在 Windows on_before_exit 清理 sidecar | 先完成可返回错误的停机，再开始安装；退出钩子幂等兜底 |
| [技能初始化](../../desktop/src-tauri/src/skills_bootstrap.rs) | 用 sidecar `future init` 安装内置技能 | CLI 仍然需要打包和管理，不随 Agent 内置删除 |
| [Linux runner](../../agent/src/sandbox/linux/runner.rs) | current_exe 作为沙箱 helper | 宿主变化后须明确 helper 入口，不能把 GUI 当成现有 CLI |
| [CI](../../.github/workflows/ci.yml) | desktop_tauri 过滤未包含 agent 路径；Windows/macOS 主要是编译检查 | 扩大依赖触发范围，新增原生生命周期和打包验证 |

Agent 主存储为 `agent/agent.db`，Desktop 主存储为 `app/app.db`，任务账本为 `tasks.db`。遗留 JSONL 在 Agent 锁保护下导入。参见 [SQLite 运维](sqlite-migration.zh-CN.md)。本次不改变它们的权威边界。

## 3 服务层与宿主接口

### 3.1 依赖方向

```text
GUI Desktop ──────┐
headless Desktop ─┼── future-agent 服务层 ── RPC、runtime、存储、工具与沙箱
future agent ─────┤
future-agent ─────┘

CLI / TUI / channels / loop / Desktop bridge ── 统一 RPC ── 活跃 Agent
```

在 `agent/src/service/` 提取服务层，模块名为实施时的建议组织：`config`、`instance`、`startup`、`activity`、`shutdown` 和后台工作登记。GUI/headless 复用不依赖 Tauri 的宿主管理部分，GUI 仅负责窗口、提示和事件适配。

服务层负责取锁、Agent 数据初始化、服务启动、活动快照、任务取消、存储完成边界和释放资源。宿主负责参数、启动环境、日志和 TLS provider、进程信号、用户确认、升级和最终进程退出。

### 3.2 建议接口

以下是接口职责草案，不代表已有类型或可直接编译的签名：

```rust
AgentService::start(config) -> Result<AgentHandle, StartError>
AgentHandle::wait_ready(deadline) -> Result<AgentIdentity, StartError>
AgentHandle::activity_snapshot() -> Result<AgentActivity, ActivityError>
AgentHandle::shutdown(request) -> Result<ShutdownReport, ShutdownError>
```

`AgentConfig` 固定数据根目录、监听端点、工具环境和资源配置。生产进程的数据根目录在启动时解析一次并保持不变；逐步用显式路径替换运行期间的全局环境读取。测试可显式注入目录和端点。

`AgentIdentity` 至少包含实例 ID、版本与协议兼容信息。数据目录身份用于确认本地实例范围；不通过远程接口泄露凭据或无关本机路径。实例 ID 每次服务重新启动都变化，不能复用 Desktop PID 作为实例 ID。

`ShutdownRequest` 区分用户退出、重启、升级和宿主错误，携带总截止时间与确认后的强制策略。`ShutdownReport` 区分完整停止、取消或持久化失败、权限清理失败、超时和仍存活的工作。只有全部工作结束的结果允许释放服务锁和在同一进程内重启。

服务 owner 使用专用后台线程和明确归属的 Tokio runtime，避免 UI 阻塞。runtime 的线程数与阻塞池预算需与 Desktop 总资源一起测量。句柄与锁的 Send、生命周期和析构顺序要在接口实现阶段确认；去掉仅适合“一次进程启动”的泄漏式锁生命周期。

### 3.3 状态机

```text
选择服务 → 外部验证 → 外部已连接
        → 等待实例锁 → 初始化 → Ready
        → 不兼容或占用失败 → 明确错误

Ready → Quiescing → Cancelling → Draining → Cleaning → Stopped
                 任一清理阶段超过期限 → FailedStopping
```

`Stopped` 表示 RPC、异步任务、阻塞任务、writer、压缩线程和受管理子进程均已完成；锁才可释放。`FailedStopping` 不允许创建第二份内置服务。宿主可以保持故障界面，或在已确认退出时结束整个进程。

启动错误需回滚已创建的资源。退出与启动竞争时，取消 startup，等待资源清理；若做不到，继续持锁直至宿主结束。服务不能在宿主退出后迟到发布 Ready。

区分慢启动提示阈值与真正的初始化截止时间。当前 GUI 有 15 秒启动宽限，headless 的外层等待可达 180 秒；提取服务时统一状态语义，但不要因为冷存储、导入或模型初始化较慢而重复启动。只有业务 readiness 成功才进入 Ready；登录取得一次性授权凭据之前也必须通过该门禁。

## 4 连接与锁的执行规则

### 4.1 锁与数据初始化

沿用相同 `agent-instance.lock` 路径、Windows 字节范围协议和安装器租约，保留 `agent-instance.json` 的身份字段。metadata 用于诊断，OS 锁决定能否启动。不得截断占用中的锁文件，不得删除锁文件后重建来绕过占用。

启动服务的路径在成功取得锁之前，不进行 Agent 数据库打开、迁移、旧数据导入、共享沙箱权限清理或实例 metadata 覆盖。既有 sessionless 维护命令保留其独立路径与 capability 等专用锁，不要求先取得长期服务锁，也不能成为绕过服务锁启动第二份实例的入口。锁文件存在但可成功加锁不等于仍有旧服务，metadata 过期也不等于锁可被抢占。

对 `FUTURE_HOME`、默认 home、命名管道名、Unix socket、metadata 和安装器环境使用一致解析。检查大小写、软链接或 junction、UNC、空值和相对路径。取锁路径必须指向同一实际数据目录，端点别名差异不能让客户端绕过数据锁。

Desktop 宿主锁与 Agent 锁用途不同，不合并成一把锁。修正宿主锁对数据根目录的解析：当前 `InstanceGuard` 按 HOME 下的 `.future/app` 取锁，需与 Desktop 实际 `FUTURE_HOME` 数据根保持一致；单实例窗口激活也要遵守相同的实例范围。

### 4.2 兼容性与重连

首次连接先取得 readiness，再判定协议和版本。最低方案是严格沿用仓库同步升级契约并消除数字版本相同的开发构建误判；需要额外字段时，通过共享 RPC crate 定义显式协议版本和必要能力。

未知 readiness、缺少必要字段、不支持必要协议均返回明确不兼容错误。能力新增不构成业务协议新旧混搭支持。兼容性失败应在统一 bridge 入口拦截任务、配置、导入、删除和后台观察器，不仅关闭前端发送按钮。

Channel 缓存必须绑定已验证实例。连接断开或实例 ID 改变时重新握手，并停止旧观察器、清空失效的流游标和进程内队列假设。自动传输重连不能把旧授权直接带到新实例；必要时让修改请求携带预期实例 ID，在服务端验证，避免握手与写入之间的实例切换竞态。

本地端点保留当前用户命名管道 DACL、Unix socket 访问保护和显式 TCP 语义。内置模式不新增公开监听端口。

## 5 完整停机协议

### 5.1 有序清理

1. 宿主停止 tasks tick、自动提交、远程发送入口和会产生新业务请求的后台循环。观察客户端可暂时保留以接收终态。
2. 服务原子进入 Quiescing。关闭修改操作的接受入口和 scheduler 出队；与进行中的接受请求同步，不只设置一个在加锁前检查的布尔值。
3. 登记并完成已经接受的修改操作。只保留状态查询、取消、必要持久化与受控清理操作；压缩、配置、删除和技能写入都服从停止边界。
4. 取消排队项并产生可观测取消结果，取消等待审批，中止所有活跃运行和会话修改操作。任务来自哪个客户端不影响内置 Agent 的全局停止范围。
5. 等待受管理的模型请求、工具调用与子进程结束。沿用 shell 取消与 Windows Job Object，不对任意用户进程树执行 kill；成功命令主动脱离管理的进程按既有契约处理。
6. 等待运行终态、会话和事件 writer 的持久化完成边界。已有 `run_terminal` 与事件事务语义不得被新停机流程削弱；失败不能伪装成成功取消。
7. 发送停机原因和可用终态，结束会话及全局 RPC 订阅，停止 listener。长连接不得使清理无限等待。
8. 停止手动压缩、journal 压缩、后台刷新及 SQLite worker；等待所有服务工作完成并关闭存储连接。
9. 在受管理沙箱工作结束后清理 Windows capability。失败保留修复 metadata，供下一次启动、重置或卸载重试。
10. 移除本实例 metadata，释放 Agent 锁；宿主完成终端、远程连接与本地账本清理后退出或交给安装器。

关闭 broadcaster、worker、存储和锁的实际顺序以资源依赖为准；上面的阶段要保证最后一个写入者在释放锁之前结束。`agent.db`、`app.db` 和 `tasks.db` 的状态收敛分别由各自 owner 负责，不直接跨库制造假终态。

### 5.2 并发与总期限

关闭窗口、菜单退出、更新、重启和重复点击共用一个 single-flight 协调器。后续调用复用第一次清理结果，不能重复取消、join 或清权限。确认对话框不阻塞 UI 主线程，清理不能靠主线程等待持有 UI 或 runtime 锁的任务。

正常退出优先完成清理；超时结果由宿主策略处理。强制退出只在用户已确认中断后执行；升级需要明确的中断确认或已有等价授权。首版建议清理总预算 15 秒，以原生验收调整；该值是预算，不是现有行为或性能承诺。

独立 watchdog 不依赖 Agent runtime、日志锁或数据库锁，负责已确认退出时的最后期限。`shutdown_timeout` 不是阻塞线程终止证明；超时后不可放锁并在同进程内启动替代实例。

已提交 SQLite 事务依赖 WAL 恢复，未持久化的尾部可能丢失，下一次启动应将未完成运行收敛为中断。不要删除 WAL 文件、伪造持久化成功或为了退出强行重建损坏数据库。

### 5.3 退出保护与恢复

全局活动快照列出运行、排队、审批、会话修改操作与停止状态。snapshot 失败返回 Unknown，不能当作零任务；允许用户根据“状态无法确认，退出可能中断任务”明确选择继续或强制退出。

确认后重新获取活动集合，并通过服务级停止覆盖确认期间的新任务。确认前不改变接单状态。取消对话框恢复正常操作；进入停机后不简单翻回 Ready。

第一版只允许在确认完整 Stopped 后恢复内置服务。故障清理不完整时保持实例锁，提示重启宿主。全局 broadcaster、日志初始化、模型缓存和 revision 等进程静态状态要标明服务生命周期；允许进程内重启的状态移入服务实例或显式重置，不能残留旧订阅和旧工作。

## 6 各宿主与跨平台适配

### 6.1 GUI 与 headless

GUI 在取得宿主锁后异步执行服务选择和初始化。任务、登录和依赖 Agent 的同步等待 ready 门禁，数据库错误进入可操作的错误状态。首次模型未配置不属于启动失败。

headless 使用同一服务层与选择规则，宿主继续负责 Ctrl+C、关闭终端和 Unix 终止信号。保留首次登录的交互式终端门禁和无 GUI 的依赖边界；不因内置 Agent 在 `--help` 或非交互首次启动时提前打开数据库和监听端点。

TUI 与独立 CLI 入口复用服务初始化、锁和停机逻辑，但保留各自的参数与信号处理。TUI 创建的 Agent 对 Desktop 是外部服务；TUI 退出后 Desktop 重新确认锁与身份，才能决定接管。

### 6.2 日志与环境

宿主只初始化一次 tracing 与 TLS provider。保留 Agent 文件日志和 stdout/stderr 兼容输出，给日志补充宿主模式、实例 ID、生命周期阶段与清理结果，避免记录凭据、完整用户 prompt 或敏感本机路径。

Desktop 释放 Windows console 后仍应有可查看的启动错误和 Agent 日志。reqwest blocking 初始化不得直接放在异步上下文或 UI 线程。当前 Agent reqwest 0.12 选择 ring，Desktop reqwest 0.13 由宿主安装 ring；合并后检查实际 feature graph，不把存在两个 reqwest 版本等同于必然冲突。

macOS/Linux 登录 shell 环境补全在多线程启动前完成，或改为工具子进程的显式环境。`FUTURE_HOME`、PATH 和 shell 配置在服务启动后不得通过环境修改影响别的模块。模型会话 cwd 继续是会话级值，不调用进程级 chdir。

发布 channel 的平台环境策略必须在 Agent 首次读取 auth/config 前应用；正式版生产环境锁定和开发版默认测试环境保持一致。不要因为宿主重排让 Agent 与 Desktop 分别连到不同平台。

profiling、dhat 全局 allocator、Ctrl+C 强制退出和 maintenance 参数属于 CLI/宿主功能，普通 Desktop 构建不启用 profiler feature。依赖层不能把 GUI 拉入 `future agent` 或 headless。

### 6.3 沙箱 helper 与工具定位

首版优先把 Linux helper 路径作为启动配置注入，使用匹配版本的打包 `future agent --linux-sandbox-helper`，保留现有 helper 协议与安全边界。若采用宿主内隐藏 helper 入口，必须在 GUI、宿主锁、Agent 锁、存储及网络初始化前分流；二者择一实现，不靠 executable basename 猜宿主角色。

Linux helper 请求版本、真实隔离执行与取消须有原生集成验证。helper 仍是工具子进程，不计为重新引入长期 Agent sidecar。

工具 PATH 应明确包含匹配 CLI 所在目录。开发环境的根 `target/debug/future` 与 Desktop 输出不相邻；便携包、macOS app 内部目录、Linux deb 路径和单独 headless 发布分别验证。CLI 缺失不应阻止内置 Agent 核心启动，但依赖 CLI 的技能或 Linux helper 应清楚报错并保持沙箱边界，不能降级成未隔离执行。

## 7 自动升级与安装

### 7.1 应用内升级

```text
检查更新 → 下载并验签 → 确认中断行为 → 完整停机 → 调用安装
                        下载或验签失败：服务继续运行
                        停机失败：不调用安装，显示恢复或强制策略
```

拆分当前 `download_and_install`，使用可返回错误的异步协调过程完成停机后再安装。当前 updater 的 `on_before_exit` 不返回 Result，仅保留为幂等最后兜底，不用 panic 或 process exit 表示可恢复清理错误。

下载验签完成不等于安装包一定可解包或安装器能启动。需要定义清理后、安装调用前后的失败恢复：完整 Stopped 且安装未接管时允许重启服务并生成新实例 ID；失败清理不得恢复半停机实例。Windows updater 启动安装器后退出的行为保留，不能依赖 `RunEvent::Exit`。

确认实际 ShellExecute/安装启动失败的可观测性：当前依赖实现的启动结果处理可能不足。优先通过宿主适配或经验证的依赖修正提供明确失败路径，不能以 hook 返回或 app 消失认定安装成功。安装器接管后的结果通过安装日志、回滚和下一次启动诊断验证。

升级流程同时协调远程入口、嵌入终端、tasks 和本地 writer。Windows 便携版继续不使用 NSIS 原地更新；macOS/Linux 升级与重启也使用完整服务停机。

### 7.2 Windows 安装与卸载

修改 [预检](../../desktop/src-tauri/windows/installer-preflight.ps1)、[NSIS hooks](../../desktop/src-tauri/windows/installer-hooks.nsh) 和 [Windows 运维说明](../internals/desktop-windows.zh-CN.md)。新增持锁宿主身份识别，支持 `futureos`；headless 进入同一发布路径时一并支持并验证。

验证实际路径、PID 创建时间、数据根与宿主角色，metadata 只辅助身份确认。保留旧 `future-agent` 与 `future-desktop` 名称兼容、文件可写检查、安装租约、加载探测和可执行文件回滚。

普通退出不停止外部服务。安装器需要停止同数据目录的外部服务时，沿用已授权的安装/卸载行为并明确显示受影响任务；应用内升级在安装前说明范围。跨安装目录关闭必须基于验证与明确授权，不因新增 `futureos` 名称而静默扩大关闭范围。通用静默或被动安装仍不自动关闭任务，保留占用 32、访问失败 5 等退出语义。

旧卸载器执行的是其自身逻辑，新 hooks 不能修复所有旧版本卸载前行为。保留旧版 VC++ runtime 等迁移限制的恢复指引。安装成功前阻止新服务取得锁，失败回滚和异常退出应释放安装租约。

升级回滚只恢复程序文件；本次保持数据库 schema 不变。以后同时迁移 schema 时需独立评审向后兼容与备份策略，不能承诺旧二进制恢复后必然能读取新 schema。

### 7.3 重启与重置

将 [clear_app_data 与环境切换](../../desktop/src-tauri/src/commands/debug.rs)、[升级后重启](../../desktop/src-tauri/src/commands/update.rs) 接入统一协调器。先暂停 producers、停止需要停止的 Agent 与 writer，再删除 Desktop 数据或修改全局环境配置，最后重启。

外部服务模式的清空与环境切换需要保持其所有权边界；不能通过修改共享 auth 文件并期待外部 Agent 自动重启而掩盖配置不一致。

## 8 构建与开发工具改动清单

| 范围 | 本次任务 |
|---|---|
| 根与 Desktop Cargo | 添加 Desktop 到 Agent 的单向 path 依赖，更新两套 lockfile；核查 workspace feature、Rustls、SQLite、pprof 与 release 配置 |
| Tauri config 与 build script | 保留 `future` externalBin 与合法 CLI placeholder；移除“placeholder 表示开发时必须手动启动 Agent”的假设 |
| Makefile | 更新 run-desktop、build-desktop、headless、检查目标与注释；CLI staging 仍保留 |
| `scripts/dev/start-desktop-macos.sh` | 默认运行内置模式，移除默认外部 Agent PID、启动与监控链；保留显式外部开发模式 |
| `scripts/dev/start-desktop-linux.sh` | 同上，增加实际 Linux helper 路径与工具 PATH 验证 |
| `scripts/dev/start-desktop-windows.bat` | 同上，重写 BUILD_AGENT/REUSE_AGENT 等旧开关的迁移语义，保留终端编码与可诊断日志 |
| `scripts/build/build-desktop-macos.sh` 与 `scripts/build/build-desktop-linux.sh` | 保留 CLI staging，验证内置 GUI/headless 和各平台产物 |
| `scripts/build/build-desktop-windows-installer.ps1` 与 `scripts/build/build-desktop-windows-portable.ps1` | 保留两份程序签名、版本和 CRT 检查，增加无 Agent 子进程的默认启动验收 |
| 发布 workflows | Windows/macOS/Linux 同步版本注入、签名、打包、产物验证和内置模式 smoke；headless 保留 musl 与无 GUI 检查 |
| doctor 与状态 UI | 显示 embedded/external、实际实例与版本、锁冲突、stopping 和清理失败；doctor 不把未安装独立 future-agent 判为内置服务异常 |
| 用户与开发文档 | 同步 build/install、headless、目录布局、Windows 运维、CLI、沙箱与双语说明 |

不要将所有 Cargo workspace 合并作为这次改动的前置条件。Desktop 现有独立 workspace 可保留；依赖构建和 lockfile 必须分别验证。也不应因内置而删除 `tauri-plugin-shell`，技能初始化仍在使用它。

## 9 分阶段开发与交付

阶段按顺序执行。每一阶段完成接口和针对性 CI 覆盖后，才能改变后续宿主默认行为。以下为实施任务，不代表已有实现。

### 阶段一 提取 Agent 服务层

- [x] 提取配置、实例锁与 metadata、storage/engine/AppState 初始化、listener 和 ready 通知。
- [x] 让 `future-agent` 与 `future agent` 共用服务层，CLI 保留参数、日志、信号、维护与 profiling。
- [x] 建立服务实例 ID、startup 取消、资源 owner 和错误结果；初始化失败不留下活跃工作。
- [x] 明确服务实例与进程静态缓存边界，保持协议、存储和工具行为。

验收：两种 CLI 入口行为保持一致；锁冲突不初始化存储；无账号可 ready；损坏数据库拒绝启动；正常停止释放资源，失败不放锁。依赖本阶段的提交不得要求 GUI 才能编译 Agent。

阶段一实现记录：工作分支 `codex/agent-service-layer`，实现基线 `73ed4183`。实际 API 是 `AgentService::with_instance`、`prepare`、`PreparedAgent::initialize` 与 `RunningAgent::serve`；锁的借用生命周期要求宿主在 scope 返回前完成 runtime 退出；同一 scope 仅允许一次初始化，防止一把锁下创建两套服务。服务不初始化全局日志或信号，不修改环境、不创建宿主 runtime、不执行进程退出。

实现位于 [service](../../agent/src/service/mod.rs)，CLI 参数、维护、日志、信号、profiling 保留在 [cli](../../agent/src/cli.rs)。[startup](../../agent/src/service/startup.rs)复用原 engine、SQLite 导入和 AppState 初始化；[instance](../../agent/src/service/instance.rs)保持锁路径、Windows 字节协议与 metadata 字段；正常路径不再泄漏锁对象，失败停机则保留所有权直到宿主进程退出。Ready 在初始化、绑定与 server 首次成功 poll 后通知。共享 Unix transport 修复预绑定后未 poll 就取消时的 socket 清理。

[隔离服务回归](../../agent/tests/service_startup.rs)覆盖实际 TCP/本地 IPC、锁冲突、同 scope 重复初始化、取消、损坏存储、端点占用和原格式导入。已有生命周期测试迁到对应模块，审批中断回归由服务级停机测试覆盖。本地通过 Agent/RPC 和 CLI/TUI/channels/loop 的 `clippy --all-targets -D warnings`，以及 `dhat-heap` feature 的静态编译、fmt 与 diff 检查。2026-10-14 的本地单测与集成验收记录见第 10.4 节；原生 Windows/Linux 与 CI 回归结果尚未取得。

阶段一保留存储实现、schema、protobuf、两个二进制入口和 CLI flag 声明。阶段二增加 writer 关闭和后台工作登记，仍不改变 schema、持久化格式或业务 RPC。全局 broadcaster 的订阅通过服务级关闭信号结束；丢弃 serving future 会使停机失败，不能作为成功清理的依据。Desktop 在阶段四接入。

### 阶段二 完整停机与活动状态

- [x] 实现全局活动快照、接受请求与 scheduler 停止门禁。
- [x] 实现队列取消结果、审批取消、运行与压缩取消，以及 writer 完成边界。
- [x] listener 与所有 RPC 订阅支持显式结束；服务级工作登记包含原生线程和阻塞任务。
- [x] 实现清理总期限、幂等结果与宿主强制退出策略；保留 Windows 启动修复。

验收：只由 CLI 发起的任务也能被全局停止；排队不再启动；终态可重读；保持订阅的客户端不阻塞停机；故意卡住的工作导致 FailedStopping 而非错误释放锁。

阶段二实现：新增 [activity](../../agent/src/service/activity.rs)、[shutdown](../../agent/src/service/shutdown.rs) 和 [work](../../agent/src/service/work.rs)。`RunningAgent::handle()` 返回可克隆的 `AgentHandle`；宿主调用 `activity_snapshot()`、`request_shutdown(Duration)` 或异步 `shutdown(Duration)`。宿主必须继续轮询 `RunningAgent::serve`，并发等待停机报告与 server 完成；不能先等待停机、再开始运行 server。

停机入口同步关闭请求与 scheduler 门禁，等待已经进入的命令，再取消所有 session 的排队、运行、shell 和审批。未执行的持久会话排队请求使用现有 `run_terminal` 格式记录 `cancelled` 和原 `run_sequence`；ephemeral 会话不因此新增持久化。排队终态在当前运行的收尾写入完成之后保存，避免修复性 rewrite 覆盖取消结果。正常运行、手动压缩、模型目录刷新、LLM 流泵、收尾阻塞任务、图片读取、Linux sandbox 准备和 Windows process wait 都纳入排空。空闲 scheduler 和取消监视任务有独立登记，不让空闲会话被误判为用户活动。

随后等待 scheduler/取消监视退出，关闭并 join session writer、刷盘并 join event writer，结束 session/global RPC 订阅，等待 listener 完成，再停止 journal compactor、关闭并 join SQLite worker，最后清理 Windows sandbox 权限。活动快照包含正在运行、排队、审批、手动压缩、命令和后台工作；journal compaction 的在途请求也计算在后台活动中。短锁不可用、快照期间发生工作变更或服务已 `FailedStopping` 时返回不可判定，不伪装成 idle。

独立原生线程监督总期限，不依赖 Agent Tokio runtime、日志或数据库。首次停机请求确定期限；后续调用取得相同终态报告。超时、worker panic、writer/权限清理失败或直接丢弃 serving future 都返回 `FailedStopping`，保留实例锁的 guard、文件描述符及 metadata 直到整个宿主进程退出，同时禁止同一进程再次建立服务。成功停机可以重新获取同一数据根；由于现有进程缓存和配置 helpers，宿主必须冻结 `FUTURE_HOME`，同一进程切换数据根被拒绝。

CLI 接入完整停机，保留原有 Ctrl-C 第二次中断/5 秒独立强制退出保护；服务层本身不调用 process exit。现有 `shutdown` 业务 RPC 仍只拒绝新 prompt、允许已经接收的运行完成，不等同于宿主级全局停止。Windows 启动修复在每个 instance scope 内至多执行一次。Desktop 退出确认与强制退出策略、自动升级和安装器清理仍属于后续阶段。

[服务回归](../../agent/src/service/tests.rs)、[真实审批回归](../../agent/src/service/approval_tests.rs)和[隔离启动回归](../../agent/tests/service_startup.rs)已补齐上述边界，包括真实 prompt 收尾与排队不再启动、`Starting` 时取消、锁与 metadata 保留、全局/会话订阅结束、错误与幂等报告。静态检查包含全部测试 target；2026-10-14 的本地执行记录见第 10.4 节，原生 Windows/Linux 的运行结果与 PR CI 验收仍待完成。

### 阶段三 连接门禁与多客户端

- [x] 扩展 readiness 的明确兼容契约与共享类型，保持业务 RPC 的同步升级要求。
- [x] 把版本、协议、数据实例与身份验证接入统一业务连接及后台观察器。
- [x] 管理缓存 Channel、实例变化、重连与请求到达时的预期实例。
- [x] 对齐 TUI/headless 的外部连接、锁冲突与显式端点规则。

验收：不兼容服务不接到 Desktop 配置、删除、prompt 或自动同步；服务替换后重新握手；TCP 显式失败不改连本地；同时启动只出现一名存储 owner。

阶段三实现集中在 [共享连接门禁](../../packages/rpc/src/connection.rs)。只读 `get_agent_readiness` 增加协议版本、数据根标识和存储 schema 版本；连接要求发布版本的三段数字一致（忽略预发布/构建后缀）、协议为 1、schema 为 6，以及有效的实例身份。旧服务缺失字段、握手报错或超时均拒绝业务调用，不回退到 `get_agent_info`，也不授权启动。新增 protobuf 字段不改变数据库或现有业务命令的存储格式。

数据根标识是规范化目录路径的 SHA-256，不是数据库 GUID，也不写入新存储文件。默认本地连接核对当前 `FUTURE_HOME`；显式 TCP 连接以配置目标为准，核对其契约并绑定首次握手得到的根标识与实例 UUID。HTTPS 端点在未配置 TLS connector 时明确拒绝，不发送明文。

CLI/TUI 保持已有显示版本；门禁使用共享 `EXPECTED_AGENT_VERSION`。协调构建取注入的 `FUTURE_VERSION`，裸 Cargo 开发构建取 RPC/Agent 的 `0.0.0` release 基准，避免 CLI/TUI 的 `0.0.2` 显示版本误拒绝同源码服务。真实服务隔离回归使用这一共享预期版本验证握手，未来调整 Cargo release 基准时必须同步 Agent/RPC。

GUI、headless、CLI 和 TUI 的业务请求及事件订阅统一使用 `VerifiedAgentClient`，每次派发前重新核对 readiness。已验证 Channel 禁止透明重连到替换服务；新物理连接必须重新握手。请求携带预期实例及数据根 header，Agent 在命令转换、接单和订阅初始化之前验证。旧的直接 RPC 调用仍可省略 header；已接入门禁的客户端始终发送它们。Desktop 缓存按端点及数据根配置区分，校验失败清除缓存，状态 UI 显示具体阻塞原因。channels/loop 保留原客户端接口，未在此阶段切换连接封装。

只有默认本地连接明确返回端点不存在或拒绝连接，且现有实例锁可以取得时，`select_agent` 才产生不可自行构造的 `LocalStartPermit`。握手失败、版本/协议/schema/数据根不符、锁占用或权限错误、未知连接错误及显式端点失败均直接报错。预检不创建或改写锁文件、不初始化数据库；`AgentService::with_local_start` 消耗许可并重新取得权威锁，抢锁失败发生在宿主初始化回调之前。现有 sidecar 启动也复查许可，并显式传递 `--grpc-addr auto`。这段记录描述阶段三完成时的范围：该阶段未切换默认运行模式，内置宿主随后由阶段四接入。

[共享门禁回归](../../packages/rpc/src/connection/tests.rs)、[隔离启动回归](../../agent/tests/service_startup.rs)、Agent 请求 header 校验和 Desktop/TUI 回归覆盖兼容外部连接、旧契约、实例及数据根变化、物理连接被旧服务替换、锁占用、预检后抢锁、显式 TCP 失败及 UI 诊断。失败场景断言无业务请求/订阅、无存储初始化且锁 metadata 保持。已完成 Rust 全部测试 target 的 Clippy、GUI/headless 静态构建、Desktop type-check/ESLint、fmt 和文档检查；2026-10-14 的 macOS 行为验收记录见第 10.4 节；原生 Windows/Linux 运行与 PR CI 仍待完成。

### 阶段四 GUI 与 headless 内置宿主

- [x] 添加 Desktop path 依赖与共享宿主管理，重排宿主锁、初始化、ready 与后台启动。
- [x] GUI supervisor 切换内置句柄，保留外部服务连接和诊断。
- [x] headless 使用同一服务层，保留无 GUI、首次交互和信号语义。
- [x] 退出提示查询全局活动，确认后覆盖所有客户端；tasks、远程、终端和 UI 退出共用协调器。
- [x] 接入重启、重置和环境切换；完整停止才允许进程内恢复。

验收：默认 GUI/headless 没有长期 Agent 子进程；CLI/TUI 能连入并完成完整工具对话；重载 WebView 不重启 Agent；真正退出会取消 CLI 对话；外部 Agent 普通退出后存活。

实现说明：GUI/headless 共用 `agent_host` 原生线程宿主。先取得 Desktop 实例锁，再初始化应用存储；Agent 门禁通过、权威实例锁取得和服务 ready 后，才启动依赖 Agent 的调度器与观察者。GUI 默认路径不再创建长期 Agent 子进程；headless 保留首次非交互拒绝、终端登录与 Ctrl+C 的入口顺序。CLI 仍用于短时技能初始化和维护，打包中暂不删除。

启动中的退出会取消初始化；并发启动共用一次启动结果。内置模式查询整个 Agent 的活动，包含 CLI/TUI 的运行、队列、审批、压缩和阻塞命令；未知状态也要求退出确认。外部模式只登记 Desktop 自己提交的精确 run ID，包括响应丢失时的 requested ID，不停止外部 Agent，也不取消其他客户端的运行。

确认退出后，协调器关闭 Desktop RPC、任务与终端创建入口，通知远程客户端并停止重连；内置模式请求完整 Agent 停机，外部模式只对登记的运行进行精确队列取消和 abort。后台 future、原生维护线程、任务子 collector、终端监听与连接有明确的停止/完成登记；任务中断写回原有任务记录，终端必须确认 OS 已回收 shell。清理错误保持失败状态，重复调用不会变成成功；重置数据、写入新环境和重启均在成功清理之后执行。强制退出在失败时记录原因并结束进程，持有的 Agent 锁由进程退出释放。

RPC serving 与停机报告并行等待，连接排空超时不会让宿主永久卡住。Agent drain 为 5 秒，宿主等待为 10 秒；Desktop 命令、外部精确取消和后台完成也各有边界。完整停止后可以重新取得锁并以新的实例 ID 启动；失败停止不得在原进程中重启。升级入口已拆成下载验签、可失败的清理、安装，防止不可返回错误的退出 hook 忽略清理失败；安装失败恢复、安装器身份/租约与真实 Windows 升级仍由阶段六完成。

### 阶段五 跨平台工具与开发启动

- [x] 实现 Linux helper 的确定入口和版本校验，保持实际 sandbox 执行。
- [x] 固定日志、TLS、环境、CLI PATH 和资源预算的宿主职责。
- [x] 三个平台开发脚本默认覆盖内置模式，保留显式外部调试。
- [x] 更新 Makefile、doctor、状态 UI、本地构建与 placeholder 注释。

验收：从桌面图标和开发脚本启动均能找到工具；CLI 缺失给出局部诊断；Linux helper 不拉起 GUI、不争 Agent 锁、不绕过沙箱；脚本退出后没有自建 Agent 残留。

实现说明（2026-10-14）：三个开发入口和 `make run-desktop` 现在直接启动 Desktop，不再预先构建/启动独立 Agent、不使用 TCP 端口作为 ready、不清理旧 PID 或直接更新 SQLite。默认 `FUTURE_DESKTOP_AGENT_MODE=auto` 使用共享连接门禁；`external` 为严格仅连接模式，端点缺失也不启动内置服务，请另行启动外部 Agent。显式地址、FUTURE_HOME 与 socket override 原样传递。`BUILD_CLI` 仍默认构建真正的 CLI 并暂存为 Tauri externalBin，支持 CARGO_TARGET_DIR；空占位文件仅供编译，运行时拒绝。旧 BUILD_AGENT/REUSE_AGENT/CLEAN_STALE_APP_TASKS 不再参与启动。RUN_CHECKS 只做静态检查，DRY_RUN 不构建、写文件或启动进程。

共享 `cli_tool` 优先定位打包 CLI、开发构建产物，再搜索绝对 PATH；拒绝目录、空占位文件和 Unix 不可执行文件，不搜索当前目录。GUI/headless 技能初始化都使用此定位；找不到 CLI 只诊断依赖 CLI 的能力，不阻止内置 Agent。宿主在 Tauri/Tokio 线程之前执行已有登录 shell 环境补全，再把 CLI 目录放入 PATH，选择一次 ring TLS provider 和 tracing。取得 Desktop 锁后写 `app/logs/desktop.log`，包含内置 Agent 记录、宿主模式、版本、实例及生命周期结果；独立 CLI Agent 的 `agent/logs/agent.log` 不变，也不打开外部 Agent 的日志。headless/内置 Agent Tokio 各使用 2 个 worker、2 MiB 栈、32 个 blocking thread 上限；这不是整个进程线程数，性能/包体仍需阶段七测量。CLI 自身运行时和参数保持不变。

Linux helper 继续由当前宿主可执行文件进入，保留统一 CLI 的 `agent` 前缀；外层/内层都携带期望 Agent 构建版本，解析后先校验、再进入既有匿名请求 FD 链路，在 HOME 目录创建、GUI、服务锁、存储和 runtime 之前返回。系统 Bubblewrap 路径/身份/版本探测、请求协议、挂载规则、审批/授权、私有报告及 fail-closed 规则不变；版本不匹配不执行命令、不提供无沙箱回退。

本阶段遵循用户“先不跑测试”的要求：仅执行 fmt、Clippy、ESLint/type-check、Bash 语法和双语文档检查，不运行单测、脚本 dry-run、应用启动或沙箱 smoke。上面的验收矩阵尚未执行；Windows PowerShell/batch 与 Linux 原生构建/沙箱、工具对话、开发进程回收及资源预算实际效果仍需后续验证。第 10.4/10.5 节是更早工作区的历史测试证据，不能替代本阶段验证。

### 阶段六 更新与 Windows 安装器

- [x] 拆分下载、验签、确认、停机和安装；补全失败恢复与 Windows 启动结果诊断。
- [x] 添加新宿主身份和 fixture，保留旧进程、租约、回滚、静默和卸载行为。
- [x] 统一普通退出、升级、升级后重启和 debug 重启清理。
- [x] 补充外部服务被安装器影响时的范围说明与授权语义。

验收：下载或验签失败不停止任务；清理错误不隐式安装；旧 sidecar 版本可升级；安装失败可恢复程序并诊断；运行中 CLI 文件锁和外部实例不被忽略。

本阶段仅完成实现与静态检查；上述行为验收、PowerShell/NSIS 原生编译与故障注入尚未执行。

### 阶段七 构建 CI 与发布验收

- [x] 修正路径过滤，让 Agent 及根 manifest/lock 变化触发 Desktop GUI/headless 编译与服务集成检查；Agent 改动也触发实际依赖它的 CLI/TUI 检查。
- [x] 更新 Windows 生命周期断言，增加真实 IPC、停机与隔离 fixture 原生检查入口（三平台原生专项已通过，完整打包生命周期仍待实机）。
- [ ] 构建签名 Windows NSIS/便携、macOS app/DMG、Linux deb 与静态 headless 产物。
- [ ] 对上一发布版本、内置首次版本、相同版本修复安装和两种升级方式做完整安装 smoke。
- [ ] 测量冷启动、线程、内存、包体和清理时间，记录与原 sidecar 基线的差异。

验收：PR CI 必需检查通过，打包文件验证和原生安装 smoke 通过；未测的平台不能标为通过。覆盖率不能用不运行真实服务的 mock 结果代替生命周期验收。

实施和剩余验收见第 10.9 节。前两项勾选表示代码与入口已实现，不表示三平台 CI、PowerShell/NSIS 或发布产物已经验收通过。

### 阶段八 默认切换与文档收敛

- [x] 核对 GUI/headless 已默认使用共享内置宿主，旧 sidecar supervisor 已替换为 GUI 适配器；保留兼容外部连接门禁。
- [x] 保留独立 Agent、TUI 外部宿主、CLI 打包与旧安装兼容。
- [x] 更新双语产品、开发、排障与随包说明；移除复制逻辑的旧查找测试，覆盖生产 CLI 查找器与原生启动脚本 dry-run。
- [x] 在用户生命周期指南及随包说明中写清退出中断全部客户端任务、外部冲突与升级恢复；没有发布新版本。

验收：普通用户走内置路径，开发脚本与发布产物一致；没有依靠默认外部启动才能通过的隐性依赖。

## 10 测试矩阵与执行策略

### 10.1 必须覆盖的行为

| 编号 | 场景 | 断言 |
|---|---|---|
| L1 | 两个宿主同时启动，或 Desktop 与 future agent 同时启动 | 一名 owner；失败者不迁移、不重置、不覆盖 metadata |
| L2 | 外部不兼容且持锁；持锁服务无响应；安装租约持锁 | 明确错误或有界等待；不启动第二份、不写业务数据 |
| L3 | 锁文件残留、metadata 残留、PID 复用、路径别名、不同 FUTURE_HOME | 以 OS 锁和完整身份为准，不结束无关进程 |
| S1 | 没有模型或登录、数据库损坏、旧数据导入失败、端点 bind 失败 | 空配置可 ready；真实故障有明确失败与资源回收 |
| S2 | 启动途中关闭、第二次 GUI 启动、GUI/headless 冲突、冷或慢存储 | 无迟到 ready、无半启动写入者；保留可诊断启动状态 |
| Q1 | GUI/CLI/TUI 同时运行、排队与审批；确认期间继续发请求 | 确认前不关门；确认后全局停止；无漏取消或新出队 |
| Q2 | 接单与停机竞争、压缩与停机竞争、多个退出入口并发 | 一次协调；已接受工作被登记；终态与持久化边界正确 |
| Q3 | 无运行但手动压缩、写入队列、全局与会话订阅仍存在 | 后台工作纳入清理；长连接结束，正常退出完成 |
| Q4 | 存储队列满、磁盘不可写、writer/阻塞线程卡住、清权限失败 | 不伪装成功、不提前放锁；总期限与修复 metadata 正确 |
| D1 | 正常取消后重开、强制结束后重开、停机中进程崩溃 | SQLite 事务保持一致；未完成运行收敛；历史保留 |
| C1 | CLI 接入内置、Desktop 真退出、CLI/TUI 重连、新实例替换 | 取消与断线可理解；重新握手；旧游标和队列状态失效 |
| C2 | 外部模式退出与重启；显式远端不可达；不兼容重连 | 外部任务按归属处理；无默认端点接管；业务门禁有效 |
| C3 | 手机、channels、loop 长连接在停机与重新启动期间 | 断线与取消不记为成功；审批和队列状态收敛；新实例重新握手 |
| U1 | 下载、签名、解包或安装启动失败 | 下载/验签不打断任务；停机后失败有明确恢复路径 |
| U2 | 升级中有 CLI/技能进程、旧孤儿 sidecar、其他安装持锁服务 | 保留文件检查、范围授权、租约与退出码，不产生混合版本 |
| U3 | 替换失败、加载探测失败、手动重装、先卸载再升级、卸载清理超时 | 可执行文件回滚；用户数据不删；安装/卸载结果真实 |
| P1 | 原生 Windows 命名管道、受限工具、取消与沙箱 Job | IPC 权限、工具行为、子进程停止及 capability 修复正确 |
| P2 | Linux helper、Unix socket、macOS 图标启动和环境补全 | 真隔离、正确路径、无 GUI helper、无环境竞争 |
| B1 | GUI/headless default/debug/release、Linux musl、Windows 无 VC runtime | 依赖边界、native loader、静态链接与版本注入正确 |
| B2 | 打包 CLI 缺失、旧 CLI、签名、便携更新策略和开发脚本 | 明确局部失败；不绕沙箱；两份程序一致，便携不误用 NSIS |

### 10.2 测试层级

服务单元测试验证接受门禁、状态机、幂等、队列取消和清理结果。集成测试运行真实服务与真实 RPC，验证锁、取消、终态重读、实例切换和流关闭；不能只检查调用过某个 cleanup 回调。

锁、环境、强制退出和 watchdog 使用独立 fixture 进程，避免污染并行测试或结束 runner。每个 fixture 显式设置隔离 `FUTURE_HOME`、HOME/USERPROFILE 和 IPC 端点；依赖沙箱路径判断的 fixture 放在项目 target 目录，不能因系统 temp 默认放行得到假通过。

Tauri mock app 不自动启动真实服务。注入 service factory 或选择器，用 mock 测 UI；真实内置集成测试显式启动隔离服务。不要依赖 dependency 的 `cfg(test)` 来隔离父 crate 测试。

原生 Windows 执行命名管道、Job、文件锁与 NSIS fixture；完整发布安装器在干净 VM 中验收。macOS 验证 app 内路径、GUI 主线程与日志；Linux 验证 bubblewrap helper、Unix IPC 与无 GUI musl 发布。

现有入口需要更新：[Windows sandbox](../../scripts/tests/test-windows-sandbox.ps1)、[Windows 生命周期](../../scripts/tests/test-windows-sandbox-lifecycle.ps1)、[安装预检](../../scripts/tests/test-windows-installer-preflight.ps1)、[Windows 安装脚本检查](../../scripts/tests/test-install.ps1)、[Linux 沙箱实机](../../scripts/tests/test-linux-sandbox-real-machine.sh) 和 [headless Linux 产物](../../scripts/ci/check-headless-linux.sh)。生命周期脚本从“一个 Agent 子进程”改为“正确宿主持锁并提供完整服务”。

### 10.3 本地与 PR 检查

默认遵守仓库 PR 偏好：本地不运行单元测试，单元测试和自动集成测试交给 PR CI。2026-10-14 用户明确要求本地单测与前面阶段验收，本次按该要求执行，记录见第 10.4 节。原生完整安装 smoke 是隔离 VM 上的发布验收，不对开发者真实安装做升级、卸载或进程清理。

本地静态检查使用现有命令，新增服务与 headless feature 组合另外补入 CI：

```sh
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings
cargo check --manifest-path desktop/src-tauri/Cargo.toml
cargo check --no-default-features --features headless --bin futureos-headless --manifest-path desktop/src-tauri/Cargo.toml
npm run typecheck --workspace @futureos/desktop
npm run lint --workspace @futureos/desktop
npm run stylelint --workspace @futureos/desktop
npm run build --workspace @futureos/desktop
python3 scripts/docs/check-docs.py --strict-pending
git diff --check
```

只有文档变更时运行文档与 diff 检查即可。实际 Rust/前端实现需按阶段执行对应静态检查，不能用 `make test` 或包含本地单元测试的综合目标替代。

### 10.4 阶段一至三本地验收记录（2026-10-14）

环境：macOS arm64、Rust 1.97.0，工作分支 `codex/agent-service-layer`。测试使用隔离 HOME/USERPROFILE、临时数据库及随机本机 TCP/IPC 端点；审批相关 HOME 放在工作树的 ignored target 目录，避免系统临时目录的默认权限改变用例语义。沙箱禁止本机 bind 的初次结果不计为代码失败；允许 bind 后重跑。GUI 与 TUI 最终全量重跑使用 4 个测试线程，Agent 与 CLI/RPC 初次全量使用单线程。

| 范围 | 通过 | 忽略 | 执行方式 |
|---|---:|---:|---|
| Agent 库单测 | 2,202 | 1 | `cargo test -p future-agent --lib` |
| RPC 库单测（含连接门禁与 wire parity） | 146 | 0 | `cargo test -p future-rpc --lib` |
| CLI 库单测 | 963 | 0 | `cargo test -p future-cli --lib` |
| TUI 库单测 | 2,170 | 0 | `cargo test -p future-tui --lib` |
| Desktop GUI 后端库单测 | 1,812 | 11 | `cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib` |
| Desktop 前端（303 个测试文件） | 4,029 | 0 | `npm run test`（desktop 目录） |
| 无 GUI/headless 定向单测 | 17 | 0 | `cargo test --no-default-features --features headless --lib headless::`（desktop/src-tauri 目录） |
| 隔离服务启动集成回归 | 10 | 0 | `cargo test -p future-agent --test service_startup` |
| 真实 Agent CLI 集成回归 | 25 | 0 | `cargo test -p future-agent --test cli_smoke` |
| SQLite 启动集成回归 | 3 | 0 | `cargo test -p future-agent --test sqlite_startup` |
| 统一 CLI 二进制集成回归 | 16 | 0 | `cargo test -p future-cli --test bin` |

最终结果：11,339 次单测执行通过、54 项集成回归通过，0 失败；12 项原有忽略测试未计为通过。headless 定向测试与 GUI 变体存在同名用例，因此总数是执行次数，不代表全部不同的测试。

本次通过日志保存在本地构建产物目录 `agent/target/phase123-acceptance/results/`（不提交入 Git）。

上述命令使用 `--offline --locked`；具体测试线程数通过 `-- --test-threads=N` 指定。前四个库的初次全量在同一个 Cargo 命令中运行；出现失败的 Agent/TUI 库在修正后单独全量重跑。

验收中修正了测试夹具和等待条件：合成运行/排队测试先建立真实持久化会话及 metadata；每个独立 Desktop mock fixture 使用新的连接验证，业务请求统计与 readiness 诊断分别查询，原始握手记录仍保留并验证；macOS 设置测试遵循内置 Seatbelt 与 Windows/Linux 探测的实际平台差异；TUI 历史分页等待分页完成，Unix argv 工具夹具使用符号链接，避免复制 macOS 系统二进制后在执行时被系统终止。没有修改存储 schema、CLI 参数或生产停机/连接门禁策略来让测试通过。

服务与门禁回归覆盖锁占用/抢锁前不初始化数据库、正常停止与失败保锁、审批和队列取消、持久化终态、全局/会话订阅结束、幂等停机、readiness 拒绝/超时、版本/协议/schema/数据根/实例变化、旧服务替换及显式端点失败。真实 CLI 回归包括 SIGINT 正常停机。修正后再次执行 fmt、Clippy 与 diff 检查。

本记录是阶段四改动开始前，对阶段一至三 macOS 代码路径的历史验收。该次测试没有覆盖原生 Windows pipe/锁/Job/权限清理、Linux sandbox/helper、Desktop 内置宿主、升级安装器、真实打包及安装升级 smoke；阶段四的新增证据见第 10.5 节。

### 10.5 阶段四本地验收记录（2026-10-14）

工作分支 `codex/agent-service-layer`，基线提交 `73ed4183`；macOS arm64、Rust 1.97.0、未提交的阶段四工作区。保持 Agent schema、存储格式与 CLI 参数不变。测试使用临时 HOME/FUTURE_HOME、独立 Agent socket 和 mock 回环端口，没有停止开发者现有 Agent。GUI/headless 全量测试顺序执行，避免已有远程模块 fixture 固定使用的 8022 端口发生跨进程冲突。

| 验收范围 | 通过 | 忽略 | 结果日志（见下方目录） |
|---|---:|---:|---|
| Agent library 全量单测 | 2,202 | 1 | `agent-tests.log` |
| RPC 兼容性与连接门禁全量单测 | 146 | 0 | `rpc-tests.log` |
| GUI 后端全量单测 | 1,800 | 11 | `gui-tests.log` |
| GUI-free headless 后端全量单测 | 1,461 | 11 | `headless-tests.log` |
| 内置宿主隔离专项 | 9 | 0 | `host-tests.log` |
| 最终退出、后台、重置和更新专项复验 | 36 | 0 | `shutdown-tests.log`、`runtime-tests.log`、`debug-tests.log`、`update-tests.log` |
| headless 最终停机与入口/信号专项复验 | 19 | 0 | `headless-shutdown-tests.log`、`headless-entry-tests.log` |
| Agent 服务启动、真实 CLI、SQLite 集成 | 38 | 0 | `agent-integration.log` |
| 前端 AgentStatusGate | 6 | 0 | `status-gate-tests.log` |

专项与全量测试名称存在重叠，不把这些数字解释为互不重复的用例。全量通过后补充了清理 runtime 创建失败的故障注入，并分别在 GUI/headless 特性下复验退出协调器、重置/环境切换、升级与入口；后台 registry 的完成记录回收也再次单独验收。上述日志只保留最终通过记录，既有 ignored 测试没有算作通过。

真实宿主 fixture 验证：实例 metadata PID 就是宿主 PID；并发启动只创建一个服务；停止/重启更换实例 ID；启动中取消不发布 ready；持锁错误不创建数据库或 auth；附着者退出不停止外部所有者；CLI 发起的流式对话与排队追问持久化为取消；CLI 长时间 shell 被终止；不排空的客户端连接会在 deadline 内报错，保留实例锁和 metadata，并拒绝原进程重启。另有全局活动/未知状态退出决策、后台派生门禁、原生工作完成、任务中断持久化、终端回收和创建门禁、清理失败禁止重置/环境写入/重启的回归。下载验签失败断言 Agent 状态与 Desktop admission 不变。

GUI、headless 与 Agent/RPC/CLI/TUI 的 all-targets Clippy（`-D warnings`）、两个 manifest 的 fmt、Desktop ESLint/type-check、双语文档检查均通过。headless 二进制构建通过，依赖树没有 Tauri/Wry/GTK/WebKit；独立进程验证 `--help` 不创建 FutureOS 根目录，首次非交互拒绝不初始化 Agent 数据库、实例锁或监听器。

日志保存在工作区忽略目录 `agent/target/phase4-acceptance/results/`，可用表中命令范围和相应 feature 重跑。GUI 终端传输 fixture 的监听器跨 HOME guard 复用，单测中由 fixture 持有；生产监听器/连接仍登记在 Desktop 清理 registry。Standalone updater fixture 显式初始化与宿主相同的 TLS provider，避免依赖其他测试先行初始化。

本轮验收覆盖 macOS 服务与后端代码。真实 GUI 对话框交互、打包 CLI/TUI 完整工具 smoke、原生 Windows pipe/锁/Job/ACL、Linux helper/sandbox、安装器身份/租约、安装失败恢复、签名包体与 Windows 安装升级仍待阶段五至八完成；不能据此声明 Windows 升级问题已经全面解决。当前开发分支的 GUI/headless 默认已使用内置宿主，发布默认切换仍需后续构建、脚本与安装验收。

### 10.6 阶段六实现与静态检查（2026-10-14）

下载和验签完成后才显示安装确认，说明内置 Agent 会停止 CLI 在内的全部工作，外部服务则保留。升级入口有单次安装互斥，互斥随原生清理/安装 worker 保持，即使 invoke 被取消也不会并行安装。所有退出、debug 重启、升级和升级后重启共用可失败的完整清理；清理失败不调用安装，也不重新打开生产者。

Windows 当前发布使用已验签的原始 NSIS EXE。格式检查、临时文件、参数保存和日志可写检查在清理前完成；ZIP/MSI 不猜测启动方式，直接报错。独立 PowerShell 观察进程在 10 秒内确认就绪，取得原 Desktop 进程句柄；等它退出后才安装，60 秒未退出则中止。安装过程不因超时强杀，以免打断回滚。`/P /R /UPDATE /FUTUREOS_CLEAN` 入口只检查占用，任何 Agent/CLI/未知文件锁均阻止安装。保留原启动参数与环境；成功由 NSIS 启动新版。失败会记录安装退出码，核对同目录全部新旧可执行文件的原始 SHA-256 及回滚失败标记，只有完整恢复时才启动旧 Desktop。诊断保存在 `FUTURE_HOME/app/logs/update-recovery.log`。回滚不完整、无法核实、无法启动恢复程序时保留诊断，要求修复安装。

安装器认识 `futureos.exe`、`futureos-headless.exe` 和旧 Desktop/Agent 身份，仍核对持锁 PID、可执行路径、数据根和创建时间。旧 `/UPDATE` 仅关闭本安装目录的已核实进程，不关闭其他目录的持锁 Agent；交互安装的“是”明确授权关闭使用同一数据根且已核实的外部宿主。一般静默/被动安装不隐式停止任务。租约覆盖权限清理、备份、替换、验证和回滚。安装包另带本次构建的维护 CLI，构建脚本及 Windows 发布工作流在 CLI 签名后复制为 `binaries/installer-maintenance.exe`，避免依赖旧维护命令；直接打 NSIS 包也必须先准备该文件。维护失败或超时，释放租约并中止，不替换文件。卸载仍保留既有尽力清理策略。

非 Windows 安装失败也不恢复原进程的冻结任务。只有原程序及 CLI 的内容/权限仍匹配时才提供进程重启恢复；macOS 核对完整 app bundle。平台安装器已改变或丢失原文件时提示修复安装，避免重启混合版本。

新增清理失败禁止安装、安装失败恢复提示、Windows 参数转义、原程序变化禁止重启的 Rust 用例定义。安装器 fixture 增加新宿主身份、自动更新外部实例门禁和清理失败不替换文件；`scripts/tests/test-windows-update-launcher.ps1` 增加成功、启动失败、安装失败和不完整回滚的离线观察进程 fixture。按用户要求，这些测试和真实安装均未运行。macOS GUI/headless Clippy、fmt、ESLint/typecheck 及文档结构检查是静态检查；本机没有 PowerShell/NSIS，也没有 Windows Rust target，原生编译及生命周期验收仍待阶段七。

### 10.7 macOS 验收与 Windows 交接（2026-10-14）

用户授权 macOS 测试后，新分支为 `codex/embedded-agent-macos-validation`。测试使用隔离 HOME/数据目录，不连接或停止用户的 Agent。首次受限环境拒绝本地网络的结果不算验收；解除执行沙箱限制后：

| 范围 | 通过 | 忽略 |
| --- | ---: | ---: |
| Agent library | 2206 | 1 |
| RPC library | 148 | 0 |
| CLI library | 963 | 0 |
| TUI library | 2170 | 0 |
| Desktop GUI library | 1809 | 12 |
| Desktop headless library | 1466 | 12 |
| Agent CLI/startup/SQLite 集成 | 53 | 0 |
| CLI 二进制入口与事件契约 | 20 | 0 |
| AgentStatusGate / UpdatePage / remote replay | 56 | 0 |

提交后合并 `origin/main`（`4bd5eab3`），重新执行受影响的 Agent、GUI 和 headless 全套测试，表中相应结果为合并后数据。Root/GUI/headless Clippy、fmt、ESLint/typecheck 和 260 份文档检查均通过。doctor 的诊断 golden 片段也同步更新；本轮未执行整个 CLI golden corpus。

修正了 helper 新版本参数、doctor 诊断和连接门禁文案的旧断言。RPC 默认根保持 Agent 的 `HOME → USERPROFILE → 平台目录` 优先级，避免 Windows 自定义 HOME 使宿主与客户端分离；安装器也拒绝驱动器相对或仅有根分隔符的覆盖路径。共享根修复后再次通过 RPC 148 例和真实宿主 9 例。GUI/headless 顺序执行，避免远程 fixture 端口冲突。日志位于忽略目录 `agent/target/phase6-macos-tests/results/`。

Windows 需要项目既有 Rust/MSVC、protoc 和 Node 环境，可运行：

```powershell
git fetch origin
git switch codex/embedded-agent-macos-validation
cargo test --locked -p future-agent -p future-rpc -p future-cli -p future-tui --lib
$triple = (rustc -Vv | Select-String '^host:').Line.Split(' ')[1]
$cli = "desktop/src-tauri/binaries/future-$triple.exe"
if (-not (Test-Path $cli)) {
    New-Item -ItemType Directory -Force desktop/src-tauri/binaries | Out-Null
    New-Item -ItemType File $cli | Out-Null # 编译占位，不覆盖真实 CLI
}
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --lib
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml --no-default-features --features headless --lib
```

Windows GUI/headless 同样必须顺序执行。占位 CLI 仅供编译，NSIS 打包必须准备真实维护代码。以上 macOS 结果不能替代 Windows named pipe、Job/ACL、PowerShell、NSIS 编译、回滚和真实升级验收。

### 10.8 Windows 单测交接结果（2026-10-15）

用户在 Windows 电脑报告全量测试通过；单独运行 `doctor` 用例也正常。全量并行执行时的 `running for over 60 seconds` 提醒未伴随最终失败，符合共享环境锁排队的可能性，尚无逐段耗时记录证明每项等待来源。此前两组 doctor fixture 会运行写有占位文本或 shell 脚本的 `.exe`，已在 `7b7bb7b5` 和 `420299f4` 改为有效系统程序；用户确认不再出现 16 位程序兼容性弹窗。

这是一条用户实机验收记录，没有附加用例数或归档日志，不补造通过数量。Windows 全量单测不能替代独立的 PowerShell/NSIS fixture、打包生命周期、升级/回滚验收。WSL 尝试曾遇到链接工具缺失和内存不足，尚未报告 Linux 测试通过；WSL 也不替代原生 Linux 沙箱验收。

### 10.9 阶段七 CI 接入与发布验收清单（2026-10-15）

本轮不在本地重跑单测，生命周期断言回归和原生 fixture 交给 CI。actionlint、Python 语法、四份 PowerShell 脚本的官方解析器校验、workspace fmt、`git diff --check` 以及 260 份文档结构检查通过；另外执行了第 10.11 节的 macOS 真实打包 smoke。PowerShell 语法检查运行于 macOS，不代表 Windows API/NSIS 行为已验证。

手动 CI 默认执行完整范围；仅调整 Windows 安装/观察器 fixture 时，可用 `gh workflow run ci.yml --ref <工作分支> -f scope=windows-update` 单独验收，避免重复三平台 Rust 构建和无关单测。PR 仍按路径依赖选取检查，不能用手动窄范围运行替代合并条件。

`scope=desktop-rust` 仅选取 Desktop GUI/headless 的静态检查、Linux 全量测试及三平台编译，不重跑 Agent/CLI/前端套件或原生宿主专项，适合本轮调度器测试隔离修复的复验。

`scope=headless-cli` 保留 Desktop 静态检查/编译，只执行 `--no-default-features --features headless --test headless_cli` 集成目标，用于单独修复入口和信号 fixture，避免再次执行已通过的 GUI/headless 全量 library 测试。

`scope=agent-lifecycle` 在该入口目标之外，执行三平台 RPC/服务启动/GUI/headless 原生宿主专项，用于宿主取消逻辑等生产边界修改；不重复全量 library 测试。

保留既有 required-check 名称；Windows 更新 fixture 的失败也会使 `Rust tests (Linux)` 汇总门禁失败，避免新增 job 尚未写入分支保护时出现漏拦截。headless 的 check、Clippy 和全量测试显式启用 `--features headless`，确保独立二进制也参与构建/静态检查。

`ci.yml` 的 Agent 依赖变化现在触发 Desktop GUI/headless 和 CLI/TUI 检查，根 manifest/lock 和共享 settings/workspaces/tasks 的影响也纳入过滤。Linux 全量单测继续按 crate 分组，不在三平台重复全部测试。`Rust build (Linux/Windows/macOS)` 增加 `scripts/ci/check-embedded-agent.py`，顺序运行 RPC 连接门禁、Agent `service_startup`、GUI 和 headless 的 `agent_host::tests`。fixture 在独立进程中隔离 HOME、FUTURE_HOME 和本地端点，实际覆盖 Windows pipe/Unix socket、持锁拒绝、启动取消、全局停机、CLI 来源任务终态、重启和外部所有者存活。零匹配测试视为失败；每组命令、结果、耗时和日志写入 `target/embedded-agent-ci/`，由 CI 归档。CI 将构建并发限制为 2、测试并发为 1，使用 `CARGO_PROFILE_TEST_DEBUG=1`，不扩大到三平台全量单测。

新增 `Windows update fixtures` PR 检查，用临时 C# 可执行文件和 NSIS 执行安装预检、清理失败门禁、租约、回滚、卸载及独立观察器成功/启动失败/安装失败/回滚不完整场景。既有签名构建调用同一预检入口，因此也会覆盖观察器。启动失败用不存在的安装器模拟，避免执行无效 EXE 触发系统模态弹窗。这些离线 fixture 不安装真实产品、不读写开发者真实 Agent 数据，也不能替代完整安装器验收。

Windows 手工生命周期脚本使用 `ExpectEmbedded`（`ExpectBundled` 保留为别名），要求 `-InstallDirectory` 限定安装目录，遵循有效绝对 FUTURE_HOME 的数据根覆盖。它验证没有独立 Agent 子进程、实例锁仍持有、metadata 中 PID/可执行路径/创建时间/数据根对应 Desktop 或 headless 本身，并用打包 CLI 完成兼容性/readiness 与只读 `list_sessions` RPC。停止要求锁释放、metadata 删除；外部模式要求已核实外部 owner 存活。探测前后核对 owner，任何错误都不算通过。脚本采集宿主、sidecar（基线）及子进程的线程/内存快照，方便同机比较。

在专用 Windows 测试用户或 VM、普通权限 PowerShell 中运行。下面的路径应替换为真实产物；所有相关进程必须继承同一数据根，并取消端点 override，使用默认本地 IPC：

```powershell
$env:FUTURE_HOME = 'C:\FutureOS-Acceptance\state'
$env:FUTURE_AGENT_GRPC_ADDR = $null
$env:FUTURE_AGENT_SOCKET = $null
# 先启动 C:\FutureOS-Acceptance\app\futureos.exe，再观察。
& ./scripts/tests/test-windows-sandbox-lifecycle.ps1 -Action ExpectEmbedded -InstallDirectory 'C:\FutureOS-Acceptance\app'
# 用户从应用退出后观察锁和清理结果。
& ./scripts/tests/test-windows-sandbox-lifecycle.ps1 -Action ExpectStopped -InstallDirectory 'C:\FutureOS-Acceptance\app'
```

| 剩余验收 | 入口与要求 | 当前状态 |
| --- | --- | --- |
| 三平台 CI | 新的原生 fixture、既有静态检查和 Linux 分组全量测试 | 三平台编译/最终原生专项通过，GUI/headless 全量 library 和入口专项通过（分次运行，见 10.10）；最终 PR CI 待创建 PR 后执行；未在本地重跑单测 |
| Windows 离线更新 | `test-windows-installer-preflight.ps1`（含 observer）、生命周期断言及 `test-install.ps1` | `9caaff29` 原生 Windows CI 已通过，见第 10.10 节；不代替真实产品安装 |
| 原生 Linux 沙箱 | `scripts/tests/test-linux-sandbox-real-machine.sh`；真实 Bubblewrap ≥ 0.9.0、helper 和子进程回收 | 待原生 Linux；WSL 不计通过 |
| 签名与发布产物 | 既有 Windows/macOS 签名工作流、Linux deb/musl 工作流及 `scripts/ci/check-headless-linux.sh` | macOS 本地 ad-hoc app/DMG 通过；Linux 双架构及 Windows daily 未签名包构建/包内验证/OSS 下载校验通过（10.13）；正式签名、公证待验收，未更新发布清单 |
| 真实安装升级 | 干净 VM 上的旧 sidecar 版→新版、内置版→新版、同版本修复；手动与应用自动升级 | 待执行；验证清理失败不安装、外部占用阻止替换、回滚保留用户数据 |
| 产品交互 | GUI/CLI/TUI 真实工具对话、排队/审批退出提示、取消后重开、外部服务普通退出存活 | 后端已有 fixture，打包 UI/完整工具 smoke 待执行 |
| 性能与体积 | 同机器、同配置、同任务，sidecar 与内置各至少 5 次；保留原始样本 | 待测量；没有宣称性能改善 |

性能记录包含首次与重复启动到真实 RPC ready、idle 与并发对话的整个相关进程树线程/内存、各签名产物字节数、正常与强制清理耗时。Windows 快照分别记录 Working Set 与 Private Bytes，不把工作集相加当成无重复物理内存；其他平台使用一致口径。sidecar 基线使用上一发布产物；同一数据快照分别复制到隔离目录，完整退出后再切换，不能共享活跃数据库。缺少可比基线时记录绝对值并保持该项待验收。启动/清理必须满足已有 deadline；资源和包体回归阈值在取得基线后评审确定，不能以 mock 测试耗时替代。

2026-10-15 用户授权继续阶段八代码与文档收尾；上述真实安装、签名、沙箱与性能验收仍为发布前要求，不能因收尾完成而标为通过。此节不是发布批准。

### 10.10 阶段七原生 CI 执行记录（2026-10-15）

完整 CI 首轮为 [38672532606](https://github.com/futuregene/future-os/actions/runs/38672532606)，源码 `07e3c932`。Windows 升级观察器四个场景通过，随后 NSIS `/WX` 因 `futureos_preinstall_cleanup_failed` 标签只有测试构建中的顺序落入、没有显式分支引用而失败；`9caaff29` 添加明确失败分支。健康检查失败 fixture 也改为有效程序返回失败，避免运行损坏 EXE 产生系统模态弹窗。

Windows 专项 [38672888594](https://github.com/futuregene/future-os/actions/runs/38672888594) 在 `9caaff29` 全部通过：NSIS 严格编译、安装预检、强制清理门禁、租约、外部占用、替换/健康检查失败回滚、卸载、四个独立观察器场景、生命周期纯断言和安装脚本失败说明。原始日志保存于工作区忽略目录 `agent/target/phase7-ci-results/windows-update-9caaff29.log`。本次 `scope=windows-update` 的其他 Rust job 仅执行 no-op，不能算作完整 CI 或 Rust 测试通过；完整 CI 的其他平台结果须单独记录。

首轮三平台编译和原生生命周期均通过，源码 `07e3c932`，每个平台的结果如下。数字包含 fixture 入口自身，不代表全部为互不重叠的独立产品场景；汇总秒数包含编译时间，不是启动性能。

| 原生平台 | RPC 连接门禁 | 服务启动集成 | GUI 宿主 | headless 宿主 |
| --- | ---: | ---: | ---: | ---: |
| Linux | 6 | 10 | 9 | 9 |
| Windows | 6 | 10 | 9 | 9 |
| macOS | 6 | 10 | 9 | 9 |

三份 `summary.json` 和分组日志已归档于 `agent/target/phase7-ci-results/native-{linux,windows,macos}/`。首轮的 Agent、CLI/TUI、channels/RPC/loop/tasks、前端以及 fmt/Clippy 检查通过；Linux GUI 全量中 `scheduler::tests::a_spawned_job_fires_at_its_deadline_and_reanchors` 失败（1,804 通过、1 失败、12 ignored）。该用例未持有与全局后台清理相同的 fixture guard，其他并行用例可取消它的循环。`887c5b77` 给两个启动后台循环的调度器用例加上已有 HomeGuard，保持产品调度不变；[Desktop Rust 范围复验](https://github.com/futuregene/future-os/actions/runs/38673788870) 单独验证此修复和 headless feature 检查。

macOS 本地真实打包还发现：在已有 mode 0644 编译占位文件上执行 `cp` 会保留不可执行权限，使 app 正常但包内 CLI 无法运行。Unix 开发、打包、Makefile 和发布 CI 的 CLI 暂存统一改用 `install -m 755`；此问题必须以重新封装后的包内 CLI smoke 验收，不能只验证源码目录中的 CLI。

`887c5b77` 的 Desktop 范围复验中，GUI library 1,805 通过/12 ignored，headless library 1,470 通过/12 ignored，调度器修复有效。随后显式启用 headless feature 后新执行的 `headless_cli` 集成目标为 7 通过/1 失败：信号用例把 stderr 第一行硬编码为 foreground 提示，而 Linux 先打印登录 shell 环境补全日志。用例改为持续排空 stderr、在 15 秒内等待 foreground 标记，再发送信号；不吞掉退出码、超时或数据变化断言。子进程还显式隔离 FUTURE_HOME 并清除 socket override。该集成目标另行复验，不能把上述 run 整体记为通过。

入口专项 [38676125549](https://github.com/futuregene/future-os/actions/runs/38676125549) 在 `cebd5140` 越过日志断言后暴露实际边界：外部握手中收到 SIGINT，连接失败仍被记为宿主失败，进程退出 1。宿主改为每次启动使用独立 Notify 取消握手；未取得服务所有权时的正常停止完成后进入 Stopped，不打开 Agent 数据库。类型化启动取消仅在服务完整清理范围内转为正常结果，真正的清理失败仍保留失败与锁。新增原生 `cancel-probe` fixture，先证明确已连接一个不答 HTTP/2 的自建端点，再停止、检查无数据库/metadata/所有权，并验证再次启动不会消耗旧停止通知；外部端点始终由测试持有。入口测试在退出断言前收集完整 stderr，后续失败可以直接定位原因。

修复 `b2f83f2e` 的 [原生生命周期专项 38677580964](https://github.com/futuregene/future-os/actions/runs/38677580964) 中，Linux/Windows/macOS 均通过 RPC 连接门禁 6、服务启动集成 10、GUI 宿主 10、headless 宿主 10；`headless_cli` 8 例全部通过，覆盖 SIGINT/SIGTERM/SIGHUP 和原外部端点/配对数据保留。三平台摘要及日志保存于 `agent/target/phase7-ci-results/native-final-{linux,windows,macos}/`，入口日志为 `headless-cli-b2f83f2e.log`。该专项没有重新执行无关全量 library 测试；全量 library 证据来自上面的 `887c5b77`，Windows 离线安装证据来自 `9caaff29`，不能描述为最新提交一次全量 CI 全通过。

### 10.11 macOS 实际产物验收（2026-10-15）

源码 `b2f83f2e`，运行 `scripts/build/build-desktop-macos.sh --skip-deps --unsigned` 完成 CLI release、生产前端、Desktop release、app 和 DMG 的完整构建。先前编译占位文件留下的 CLI mode 0644 已通过统一 `install -m 755` 修正；新 DMG 内 CLI mode 为 0755。初次受限环境的 hdiutil 返回 `Device not configured` 不算封装通过；使用系统磁盘映像服务完成了实际封装、校验、只读挂载和卸载。

| 项目 | 结果 |
| --- | --- |
| `hdiutil verify` | 通过 |
| DMG 内 app `codesign --verify --deep --strict` | 通过；ad-hoc，不是 Developer ID/公证验收 |
| DMG 内 `future --version` | `future v0.0.2-b2f83f2e+local`，退出 0 |
| DMG 内 `futureos --help` | 正确 GUI 入口帮助，退出 0 |
| 探测隔离 | 独立 HOME/USERPROFILE/FUTURE_HOME；未创建 Agent 数据目录 |
| DMG 字节数 | 40,029,973 |
| app 逻辑文件字节数 | 85,815,433；非内存、非 sidecar 对比数据 |
| DMG SHA-256 | `9bab6755e6b8b0ac668f85af028937c7b1cc62d411b797285d0e11d93d3a3ed9` |

产物与日志在忽略目录 `agent/target/phase7-macos-artifacts/`：`FutureOS_0.0.2_aarch64.dmg`、`build-final.log`、`artifact-smoke.json`、DMG 校验和包内命令日志。没有安装、启动真实 GUI 会话或发布该产物。无 sidecar 可比基线，因此只记录绝对包体，性能项仍待验收。

macOS 发布工作流增加实际 app 内 CLI 可执行权限、精确版本、Desktop 帮助入口和无 Agent 状态初始化检查；同一检查代码已对本地产物执行通过。Linux 发布工作流先增加 deb 入口探测，再扩展为第 10.12 节的共享包验证脚本。二者都使用隔离 HOME 和 15 秒单命令超时，不能仅以签名正确或暂存源文件可执行判断产物通过。


### 10.12 daily 未签名包验证脚本 review（2026-10-15）

Windows 既有流程只运行暂存 CLI 和 C#/NSIS 模拟安装器，不能证明真实产品安装包包含正确入口。Linux 既有 deb 探测未验证 portable 和独立 CLI 压缩包。新增共享 `scripts/ci/check-packaged-entrypoints.py`，在平台原生 runner 上解包实际 NSIS/deb、portable 和 CLI 产物；运行包内 CLI 精确版本、Agent 帮助、Desktop 帮助，并验证 Windows 安装器内维护 CLI 的版本/重置能力。所有探测使用临时 HOME/USERPROFILE/FUTURE_HOME、清除端点 override、禁用 stdin 并设置超时；不得创建 Agent 数据目录。它不执行安装器、不启动真实 GUI/Agent，不代替安装、升级与沙箱完整验收。

Linux 对各 CLI 包检查静态链接，对 deb 和 portable 内 headless 执行 `check-headless-linux.sh`：真实 ELF、无网络 Rocky 8/旧 glibc、无 TTY 首次运行拒绝、未写凭据/配对/数据库。容器和内部程序均有超时，退出时清理专用命名容器。验证错误阻止产物上传，日志和 JSON 汇总单独以 Actions artifact 保存。

手动 Linux 构建与 Windows 使用同一 `+dev` 显示版本/无后缀公共文件版本规则。两平台独立 daily 构建可上传到 `dev/<artifact-version>/`，不修改正式或 nightly 更新清单。共享 ossutil 安装脚本改为按 x86_64/aarch64 runner 选择 amd64/arm64 下载。工作流、Shell、Python 语法和双语文档检查已通过；宿主 CI 结束后实际执行的两平台打包结果见第 10.13 节。

### 10.13 GitHub Actions daily 未签名构建验收（2026-10-15）

在宿主专项 CI `38677580964` 整体成功后，从源码 `c60d5754b46043300f08962fc1805ad766fa475b` 启动两平台现有打包工作流。CI 浅克隆使用 7 位短哈希，显示版本为 `0.0.2-c60d575+dev`，公共产物版本为 `0.0.2-c60d575`，对应同一份源码。后续 `b6606e53` 仅收紧 Linux 独立发布条件：由协调器传入版本时不再额外上传 dev 目录；没有修改包内产品代码或本次独立构建路径。

[Linux 构建 38679367594](https://github.com/futuregene/future-os/actions/runs/38679367594) 的 x86_64、aarch64 原生 job 全部成功，生成 deb、portable 和静态 CLI 压缩包。每种架构对实际包内程序执行 10 个早期入口探测，另外检查三份 CLI 静态链接、deb/portable 内 headless 的旧 glibc/非交互拒绝/无敏感状态写入。CLI/GUI/headless 均使用正确入口、精确 CLI 版本和 0755 权限。包验证摘要及逐命令日志在 `agent/target/phase7-ci-results/package-linux-{x86_64,aarch64}/`；完整 job 日志为 `linux-{x86_64,aarch64}-package-c60d5754.log`。

Linux 六份包均上传 OSS 后，通过公开下载地址下载回本地，再逐一验证字节数与 SHA-256 和 CI 摘要一致。地址均位于 `https://dl.future-os.cn/dev/0.0.2-c60d575/`；本地文件及 `download-verification.json` 在忽略目录 `agent/target/phase7-daily-packages/linux-{x86_64,aarch64}/`。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `FutureOS_0.0.2-c60d575_amd64.deb` | 54,395,610 | `dfeec37f6e07fe0e08c17b56f97aaebed49f9886c6a16ab41e5d3f285a9fed14` |
| `FutureOS_0.0.2-c60d575_linux_x86_64-portable.tar.gz` | 53,048,069 | `876993616963716bd05e18a472aa2d93d7432f131d00eb16d6a97b91937e83ee` |
| `FutureOS_0.0.2-c60d575_linux_x86_64-cli.tar.gz` | 17,313,503 | `d4544b9b933dd193fd2fb652db62fe517aec7105e7041fb830e229a60e0d9696` |
| `FutureOS_0.0.2-c60d575_arm64.deb` | 51,731,124 | `3e5d3260c03be28a8aeb2ab078a294061be54ee345978d93a2fe0094ac4217e7` |
| `FutureOS_0.0.2-c60d575_linux_aarch64-portable.tar.gz` | 49,854,349 | `84a03942c5620704b69a430e1bfb06b58ceff9dd69b684c8a3e15c91f8e1504f` |
| `FutureOS_0.0.2-c60d575_linux_aarch64-cli.tar.gz` | 15,998,400 | `bd41807494a172587d01907c590421d661fff978cabdef4a2778543d14231c9f` |
| `FutureOS_0.0.2-c60d575_x64-setup-unsigned.exe` | 27,640,493 | `925b6fd3581bc8acb1fee825a1d2625bd69e9a14fb8897542a92a8e10caf1f7c` |
| `FutureOS_0.0.2-c60d575_windows_x64-portable-unsigned.zip` | 38,203,476 | `3b48eff347104a146960056b11f78f8cfb7d293bca3e38516da892149c6fd8c3` |

[Windows 未签名构建 38679370525](https://github.com/futuregene/future-os/actions/runs/38679370525) 整体成功：CLI 兼容性、Desktop/NSIS 构建、四个升级观察器场景、NSIS 安装预检/清理门禁/租约/占用/失败回滚/卸载、安装脚本回归、updater 签名文件存在性、真实 NSIS/portable 解包后的 8 个入口探测，以及 GitHub/OSS 上传均通过。安装器内 `futureos-maintenance.exe` 的精确版本与 `--reset-windows-sandbox` 帮助也通过。摘要及命令日志在 `agent/target/phase7-ci-results/package-windows/`，完整 job 日志为 `windows-package-c60d5754.log`。Windows 权限由原生执行验证，不将 Python 的 mode 0777 视为 Unix 权限或 ACL 验收。

Windows 两份包也通过公开 OSS 地址下载回本地、校验大小和 SHA-256，保存在 `agent/target/phase7-daily-packages/windows/`；对应 `.exe.sig` 非空并可下载。此处“未签名”指没有 Authenticode 平台签名，Windows 仍保留 Tauri updater 签名文件；当前检查不声称独立完成 updater 签名密码学验证。

没有更新正式版、test 或 nightly 的更新清单，也没有安装这些包。以上数据是绝对包体，不能当作 sidecar 对比或运行性能证据。真实安装/升级/回滚、GUI 完整交互、原生 Linux Bubblewrap 沙箱和性能基线仍按第 10.9 节保留待验收。

### 10.14 阶段八代码与文档收尾（2026-10-15）

用户授权先提交/push 当前成果，再完成最后阶段收尾。开始时工作区干净，`b502f378` 已同步远端；没有创建空提交。GUI/headless 已默认通过共享 `AgentHost` 内置完整服务，`agent_supervisor.rs` 仅是 GUI 状态/退出适配器，不再保留默认启动 sidecar 的支路。独立 Agent、TUI、CLI 打包和旧安装兼容继续保留。

新增双语[生命周期与恢复指南](../guide/desktop-agent.zh-CN.md)，同步根 README、wiki CLI、产品边界、headless、构建与安装、Windows 安装及沙箱文档、六份随包说明和脚本索引。说明内置退出影响所有客户端、外部 owner 保留、冲突不接管、日志位置和失败恢复。移除“没有 CLI 就无法启动 Agent”与“后台未连接一定是下载标记”的旧排障结论；没有把新的说明回写进先前 `c60d5754` 已构建的 daily 文件。

生产 CLI 查找器提取实际候选选择入口，保持顺序和过滤不变。headless 直接使用同一解析器，移除其复制候选逻辑的测试；新行为测试覆盖 bundle 优先、空占位和目录跳过、拒绝相对 PATH，以及 Unix 不可执行文件回退。fixture 只做文件检查，从不执行临时伪程序。原生 CI 新增 `cli-tools` 分组，Unix 4 例、Windows 3 例（无 Unix 权限位用例），执行结果另行记录。

| 脚本范围 | 审查与修正 | 验证边界 |
| --- | --- | --- |
| macOS/Linux 包装与公共启动脚本（3 份） | CLI 暂存先于静态检查；Vite strictPort；相对 Cargo 输出目录从根解析；端口十进制与范围检查；仅清理本次开发进程组 | Shell 语法和 dry-run；不启动真实 GUI/Agent |
| Windows bat/PowerShell（2 份） | bat 仍只委托 PowerShell；strictPort、绝对输出目录及明确退出提示；模式大小写和端口格式与 Unix/宿主一致 | 官方解析器；跨平台 PowerShell dry-run；原生 CI 经 bat 执行 8 个 dry-run 场景 |
| iOS/Android 启动脚本（2 份） | 审查 SDK/模拟器/Metro 入口，没有 Desktop Agent 启停逻辑 | Shell 语法；本轮没有构建移动 SDK 或运行模拟器 |
| 4 份桌面打包脚本及 Makefile | 保留 CLI 暂存、维护副本、旧安装兼容；更新便携版说明与失效注释 | 源码审查；Shell/PowerShell 解析，不等于本轮重新执行四种本地打包 |

所有 14 份 PowerShell 脚本解析通过；Desktop GUI/headless all-target Clippy（`-D warnings`）、两套 fmt、Desktop ESLint/typecheck、actionlint 和 262 份文档校验通过。ESLint 有一条未修改文件 `desktop/shot/main.tsx` 的既有 JSDoc warning，退出码 0。没有在本地运行单元测试。启动入口变更现触发 Desktop 和原生生命周期 CI；dry-run 覆盖 auto/external、非法模式、0/超界/非法端口、含空格的相对输出目录，每条有 30 秒上限。原生专项复验待执行/记录，不复跑无关全量套件。

开发 Ctrl+C 是工具链中断：Unix 发送 TERM，超过 10 秒可能 KILL 本次进程组，不承诺 Agent 完整清理；正常清理由应用内退出触发。真实 headless 信号语义保持原有有界清理。阶段七未完成的真实安装、正式签名、原生沙箱与性能验收继续保留，阶段八代码完成不等于发布批准。

## 11 发布条件与回退

默认切换前必须有四类证据：真实服务功能与持久化、原生跨平台生命周期、完整安装/升级/回滚、开发和发布入口一致。验证记录包含构建版本、平台、包类型、服务模式、场景、结果和日志位置；不要只记录“CI 绿了”。

重点测量首次与重复启动到 ready、idle 和并发运行的线程/内存、签名包体、正常与强制清理时间。先建立 sidecar 基线，再比较同机器、同配置、同任务的内置结果；阶段七确定可接受回归阈值，不预先声称更省内存或更快。

测试迁移期可以保留受控的 sidecar 宿主选项用于 A/B 和回退，但默认路径必须被 CI 与 smoke 覆盖。该选项不能绕过锁、不兼容检查或完整停机；最终保留与否在发布验收时决定。

代码回退只在同存储契约下进行。完整停机才允许切换宿主或重启服务；安装回滚保留用户数据与 WAL，不降级 schema。外部不兼容由用户升级或退出外部服务解决，不通过自动删除数据或杀未知进程恢复。

PR 使用工作分支；提交前获取并合并最新远程主干，解决冲突，完成 type-check、ESLint、fmt 和 Clippy，不在本地运行单元测试。创建 ready-for-review PR，启用自动合并，等待 CI 与仓库条件。是否拆分 PR 按阶段接口可独立验证决定，不把所有未完成阶段标为一次已交付。

## 12 完成定义

- [ ] GUI/headless 默认完整内置 Agent，CLI/TUI 可连入并完成真实工具任务。
- [ ] 不兼容或持锁外部服务不被绕过；后台业务也受门禁约束。
- [ ] 全局活动提示与取消包含其他客户端；完整清理完成后才释放锁。
- [ ] 升级、退出、重启、重置和 headless 信号使用一致清理语义。
- [ ] 超时或卡死不会导致同进程两份 Agent；异常重开能恢复事务与运行状态。
- [ ] Linux helper、Windows 沙箱、工具 PATH、文件日志和无 GUI 发布不回归。
- [ ] CLI、签名、旧 sidecar 升级、文件预检、租约和安装回滚继续有效。
- [ ] 开发脚本、Makefile、CI 过滤、原生测试和完整打包 smoke 全部覆盖新默认路径。
- [ ] 性能与体积回归有基线和验收记录，双语文档与产品说明同步。

## 13 相关资料

- [RPC 架构](rpc.zh-CN.md)、[共享包](packages.zh-CN.md)、[SQLite 运维](sqlite-migration.zh-CN.md)。
- [构建与安装](../guide/build-and-install.zh-CN.md)、[无头 Desktop](../guide/desktop-headless.zh-CN.md)、[目录布局](../guide/directory-layout.zh-CN.md)。
- [Windows 安装更新](../internals/desktop-windows.zh-CN.md)、[Linux 沙箱](../internals/desktop/SANDBOX/LINUX.zh-CN.md)、[Windows 沙箱](../internals/desktop/SANDBOX/WINDOWS.zh-CN.md)。
- [Tauri updater API](https://v2.tauri.app/reference/javascript/updater/) 描述 Windows 安装后退出的行为。
- [Tokio runtime](https://docs.rs/tokio/latest/tokio/runtime/struct.Runtime.html) 描述 runtime shutdown 与阻塞工作的限制。
- [Cargo feature 规则](https://doc.rust-lang.org/cargo/reference/features.html) 描述依赖 feature 合并；实际发布以两套 lockfile 与构建结果为准。
