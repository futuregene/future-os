# Desktop Agent 生命周期与恢复

[English](desktop-agent.md)

GUI 和 `futureos-headless` 默认在自身进程中运行完整 Agent。CLI、TUI、手机和其他客户端仍通过同一 RPC 使用服务，现有配置、数据库、会话和凭据格式不变。打包的 `future` CLI 继续用于技能、维护和独立 `future agent`，不再是 Desktop 默认启动 Agent 的必需进程。

## 启动与外部服务

Desktop 先检查当前系统用户、数据根及端点上的外部 Agent。兼容且就绪则连接它；默认端点没有服务且成功取得实例锁时，才启动内置 Agent。版本、协议或存储契约不兼容，以及实例锁被占用但服务不可用时，直接报告错误，不启动另一份 Agent，也不删除锁或打开第二份业务数据库。

显式设置 `FUTURE_AGENT_GRPC_ADDR` 的连接和 `FUTURE_DESKTOP_AGENT_MODE=external` 都只连接外部服务，失败不会自动接管。开发环境可先在另一终端运行 `future agent`，再以 external 模式启动 Desktop。默认 `auto` 同时支持兼容外部服务和内置服务，没有强制绕过外部 owner 的开关。

CLI/TUI 连接内置服务时，必须使用同一系统用户、`FUTURE_HOME` 和端点设置。需要独立常驻服务时，先退出 Desktop，再独立启动 `future agent`，然后重新打开 Desktop。不能通过更换端口绕过同一数据根的实例锁。

## 退出、重启与升级

- 内置宿主退出会停止所有客户端的活动、排队和审批任务，包括从 CLI/TUI 发起的对话。GUI 检测到工作或无法确认活动状态时会提示；选择强制退出会先执行清理，再退出。其他客户端会观察到取消或断线。
- 连接外部 Agent 时，只取消本 Desktop 提交的运行并关闭自身远程入口，外部服务和其他客户端继续运行。
- headless 的 Ctrl+C，以及 Unix 的 SIGTERM/SIGHUP，走有界停止流程。登录和配对数据保留，再次启动通常无需重新配对。
- 更新先下载、验签并确认中断范围，再完整清理。清理失败不启动安装器，也不在同一进程启动第二份 Agent。重启同样要求上一实例完成清理。
- 系统强杀、崩溃或开发工具链中断可能绕过正常退出；下次启动按已有事务恢复和权限回收机制处理。不要手动删除数据库、WAL、锁文件或 capability metadata 来“修复”启动。

Windows 自动更新使用独立观察进程等待 Desktop 退出，再运行安装器。安装失败且原程序完整恢复时才重启旧程序；回滚不完整时保留诊断，用户可重新运行可信完整安装器修复。便携版手动完整替换文件，避免混用不同版本的 Desktop 和 CLI。详细安装门禁见 [Windows 安装器预检](../internals/desktop-windows.zh-CN.md)。

## 排障与日志

默认数据根是 `~/.future`；有效绝对 `FUTURE_HOME` 替换整个根。内置宿主日志在 `<数据根>/app/logs/desktop.log`；独立 CLI Agent 的日志仍在 `<数据根>/agent/logs/agent.log`（开启文件日志时）；Windows 更新恢复日志在 `<数据根>/app/logs/update-recovery.log`。

| 现象 | 处理 |
| --- | --- |
| 外部服务版本不兼容 | 核对报错中的版本/协议，正常退出或升级外部服务，再重开 Desktop |
| 实例锁占用、服务无响应 | 确认是否还有 GUI、headless 或独立 Agent 使用同一数据根；正常退出它，必要时重启系统；不删除锁 |
| 显式端点不可达 | 检查地址、对应服务和用户环境；不需要远端时取消显式覆盖，再重开 |
| 缺少 CLI，但 Agent 正常 | 放回匹配 `future` 或加入绝对 PATH；这影响技能和维护，不代表内置服务需要另一个 Agent 进程 |
| 清理失败，更新未安装 | 保存报错和日志，正常退出相关客户端后重试；持续失败时重启系统并运行完整安装器 |
| 安装失败且没有自动重开 | 查看恢复日志与安装诊断，完整修复安装；不强行启动混合或不完整回滚的程序 |

反馈时提供版本、平台、服务模式、错误和相关日志片段，去除 API key、配对链接等凭据。`future doctor` 可辅助检查环境；它不授权绕过实例锁。

## 开发启动入口

macOS/Linux 使用 `scripts/dev/start-desktop-macos.sh` 或 `scripts/dev/start-desktop-linux.sh`；Windows 使用 `scripts/dev/start-desktop-windows.bat`（委托 PowerShell）。它们构建/暂存 CLI，再启动 Tauri，服务选择由 Desktop 完成。移动端的 iOS/Android 脚本只管理移动开发会话，不启动 Desktop Agent。

`DRY_RUN=1` 只打印配置；`BUILD_CLI=0` 跳过 CLI 构建；`RUN_CHECKS=1` 运行静态检查，不跑单元测试；`DESKTOP_DEV_PORT` 指定 Vite 端口。端口占用直接失败，不自动换端口或结束占用者。相对 `CARGO_TARGET_DIR` 从仓库根解析并转换为绝对路径。

macOS/Linux 启动前会检查通过 `RUSTC_WRAPPER`、`RUSTC_WORKSPACE_WRAPPER` 或对应 `CARGO_BUILD_*` 环境变量选择的 `sccache`。缓存程序失败、缺失或检查超过 10 秒时，只有直接调用 rustc 成功才会在本次启动中绕过缓存；CLI 构建、可选 Cargo 检查和 Tauri 都继承该设置。正常缓存和其他 wrapper 保持原样，原始诊断写入 `.logs/rust-cache-preflight.log`，不修改缓存文件、共享服务或全局配置。真实编译错误仍按原有方式报错，不自动重启整个 Desktop 会话。

该检查不解析 Cargo TOML 配置，也不处理检查通过之后才发生的缓存故障。如果 wrapper 只在 Cargo TOML 中配置，可使用 `RUSTC_WRAPPER='' RUSTC_WORKSPACE_WRAPPER='' ./scripts/dev/start-desktop-macos.sh` 临时绕过（Linux 使用相应入口）。Windows 启动脚本目前不执行此缓存检查。

应用内退出用于协调清理。开发脚本的 Ctrl+C 中断开发工具链；Unix 清理仅作用于本次启动的进程组，超时可能强杀。它不保证等同应用内正常退出，不扫描、终止外部 Agent，也不改写数据库或锁。真实 headless 程序直接处理 Ctrl+C，参见[无头模式指南](desktop-headless.zh-CN.md)。

## 本次改动说明与验收范围

本次将 Desktop 默认 Agent 从独立子进程切换到完整进程内服务，保留外部连接、独立 CLI/TUI、工具能力及存储格式。退出中断范围包含其他客户端，这是升级前应告知用户的行为变化。

三平台原生生命周期 CI、Windows 离线更新 fixture、macOS ad-hoc 包和 Linux/Windows daily 未签名包已有通过记录。正式签名/公证、真实安装升级/回滚、完整 GUI 工具交互、原生 Linux Bubblewrap 及性能对比仍需验收；代码收尾不等于发布批准。证据与剩余清单见[开发计划](../architecture/embedded-agent-development.zh-CN.md)。
