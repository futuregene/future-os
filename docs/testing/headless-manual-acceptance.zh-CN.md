# Headless 人工验收记录

> [English](headless-manual-acceptance.md)
>
> 状态：已交付主工作区体验版本，尚无确认的人工结果；不是最终验收通过记录。

## 候选与顺序

主工作区 `/Volumes/ZHITAI/Workspace/future-os` 的体验分支为 `test/headless-primary`，提交为 `a0d4d8402064dab1b82b0ad39bbd645837446236`。已构建 macOS ARM64 debug CLI/GUI，CLI 自报 `0.0.2-a0d4d840+local.dirty`；dirty 来自保留的 `skills` 子模块改动。详情见[交付记录](headless-primary-handoff-2026-10-20.json)。该目录和运行程序在用户测试期间保持不动；开发继续使用独立实施 worktree。

实施分支可以继续修复和同步主干，但结果须记录其完整 SHA。当前主工作区体验结果不能自动归给后续候选。最终候选的完整提交、自动化 run、Desktop/CLI 构建身份、Mobile 构建版本、平台及日志证据均待填写。

按用户最新安排，先进行人工体验/功能验证，实际发行打包、签名及安装升级验收后置。既有 CI 安装器回归仍需修复并通过。未执行和未反馈的项目均保持待确认；人工期间源码变化后重新验证受影响项目，不拼接不同候选结果。

## 当前可启动的 macOS 版本

从主工作区执行以下命令。GUI 和 headless 通常轮流运行；M11 再专门验证互斥。

```bash
cd /Volumes/ZHITAI/Workspace/future-os
BUILD_CLI=0 make run-desktop
```

正常退出 GUI 后，在真实交互终端启动 headless：

```bash
cd /Volumes/ZHITAI/Workspace/future-os
./target/debug/future headless
```

在另一终端查看设备：

```bash
cd /Volumes/ZHITAI/Workspace/future-os
./target/debug/future remote devices
```

运行 `./target/debug/future version --json` 核对完整提交。最终同候选 Mobile 构建及其他主机平台的版本/启动路径尚未安排，不能据此声称 Android、iOS、Windows 或 Linux 人工验收通过。现有手机 App 可先验证旧配对兼容；其实际版本、系统和结果需要记录。

## 用例与预期

| ID | 平台/准备 | 操作步骤 | 预期结果 | 当前结果 |
|---|---|---|---|---|
| M01 | macOS GUI + 已有手机；原数据目录 | 正常退出旧宿主，记录原工作区/会话和配对设备；用本版本 GUI 打开同一数据目录，再换为 headless，手机重连 | 原数据库、工作区、身份、配对簿和控制设备保持；无需重新配对；会话可继续 | 待用户反馈 |
| M02 | 交互终端 + 手机；独立空 Future home | 在独立目录用 `FUTURE_HOME=<临时绝对路径> ./target/debug/future headless` 启动；按终端完成首次登录，手机领取邀请 | 登录/配对完成后可实际发消息、同步事件；只创建原有布局的文件；重启复用凭据 | 待用户反馈 |
| M03 | 空临时目录；非交互终端 | 设置独立 `FUTURE_HOME` 后执行 `future headless </dev/null`，记录退出码及输出 | 退出 1、提示交互终端；不输出授权/配对材料，不创建 app.db、配对或凭据 | 待用户反馈 |
| M04 | 用户已有自建中继 + 手机 | 在独立测试根运行 `future headless --relay <中继 URL>`；私有 CA 时带原有 `--relay-ca <PEM>`；完成配对并重启 | 邀请使用所选中继，凭据可复用；不因包名改变配对；origin/TLS 和 CA 边界保持 | 待用户反馈 |
| M05 | 至少两台已配对客户端 | 运行 `future remote devices`、`add`，领取新邀请；用 `use <id或序号>` 切换在控设备，重启宿主后再查看 | 原设备不被替换；配对簿和控制指针一致；只有在控设备可执行原有控制动作 | 待用户反馈 |
| M06 | M05 的独立测试配对 | 用 `remove <id或序号>` 撤销一台，确认另一台仍可连接；网络故障条件下撤销后重启，再恢复网络 | 精确撤销目标；其他设备不受影响；待撤销记录按原规则恢复清理 | 待用户反馈 |
| M07 | 手机和 GUI 各一轮；临时工作区 | 发起会触发审批的读/写/命令；分别拒绝和批准；保存一条工作区规则后再次请求 | 审批状态同步且决定实际生效；规则原子持久化并按原规则实时注入；不扩大权限 | 待用户反馈 |
| M08 | 有系统沙箱能力的测试主机 | 在当前沙箱模式下访问允许的临时工作区和边界外测试文件；触发需升级授权的动作并拒绝 | 允许/拒绝与原模式一致；沙箱拒绝不会被宿主包或命令变化绕过；错误可见 | 待用户反馈 |
| M09 | 手机 + 临时文件/工作区 | 查看、传输、修改测试文件；创建并执行一次测试任务，查看其会话、事件、结果和 review | 文件内容和任务结果可用；会话/事件同步无遗漏或重复业务动作；路径边界保持 | 待用户反馈 |
| M10 | GUI/headless 各一轮；有活动请求 | 在消息生成、文件传输、任务运行或等待审批时正常退出；headless 用 Ctrl+C；重启并重连手机 | 先关闭接入，再按有界预算清理；不遗留自行托管 Agent、迟到写入或复活任务；数据和配对可继续 | 待用户反馈 |
| M11 | macOS；同一 Future home | GUI 运行时启动 headless；退出 GUI 后启动 headless，再尝试 GUI；同时用第二终端做设备管理 | 第二宿主明确拒绝，不删除锁或抢占；运行宿主继续工作，本地设备控制请求仍可用 | 待用户反馈 |
| M12 | Unix/macOS；仅本次自行启动的子进程 | 在首次登录等待、配对等待和正常运行时，分别对该次 headless PID 发送 SIGTERM、SIGHUP，再重启 | 各阶段都可有界退出；恢复后凭据、配对与控制指针保持；不留下自托管 Agent | 待用户反馈 |
| M13 | 独立 Future home、全新未占用端口、外部 Agent | 先在另一终端对该隔离目录启动 `future agent --grpc-addr 127.0.0.1:<新端口>`；以相同 home、显式端点和 `FUTURE_DESKTOP_AGENT_MODE=external` 启动 headless；另一个独立 CLI 会话发起持续任务，再退出 headless | 外部 Agent PID 保留、其他客户端会话继续；仅本宿主提交的 run 按原归属清理；不扩大取消范围。最后仅正常退出本次自行启动的 Agent | 待用户反馈 |
| M14 | GUI/headless 各一轮；平台网络可用 | 验证模型列表刷新、登录状态复用及内置技能初始化；在实际会话中使用一项已安装技能 | 模型/技能可用，重启后配置保持；维护使用当前 CLI，不需要外置 headless 或 Desktop 程序 | 待用户反馈 |

M02/M03/M04/M06/M13 使用独立测试数据目录；不删除、改写或终止已有外部 Agent。Windows 的 Ctrl+C/控制台关闭与平台沙箱行为由原生自动化和后续明确的 Windows 人工版本记录覆盖，不能用 macOS 的 M12 代替。

## 结果记录与后置验收

每项结果记录：完整候选 SHA、主机系统/架构、Desktop/CLI 实际版本、手机系统/App 构建、所用数据根、操作与预期、实际结果、日志或截图引用、是否重测。涉及凭据的值不作为公开验证材料。未提供证据的结果保持待确认。

当前确认的人工通过项：无。当前确认的人工失败项：无。用户体验结果、最终同候选人工结果、签名产物、真实安装/升级/卸载、旧 Linux 静态 smoke 与匹配性能记录均待最终汇总。自动化和用户确认全部满足之前不关闭整体 Goal。
