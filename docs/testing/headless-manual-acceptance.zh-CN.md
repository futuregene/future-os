# Headless 人工验收记录

> [English](headless-manual-acceptance.md)
>
> 状态：用户于 2026-10-20 报告人工测试完成，并要求整理后提交 PR。逐项平台与证据未单独反馈；体验版本结果不移记到后续源码候选。

## 最新状态与历史构建

[PR #1259](https://github.com/futuregene/future-os/pull/1259) 已合并至 `a5f91752b4e470f90fbe5f4bc303ec0ca814944d`。对应候选 `dac8832f` 的 CI 39408277224 已全部完成且成功，包括最后的 Windows Host。详见[最终 CI 证据](headless-ci-39408277224-2026-10-20.json)及[后置发布清单](headless-release-acceptance.zh-CN.md)。这补齐自动化结论，不自动把旧体验版人工结果移到新候选。

用户随后要求清理本地分支/worktree，主工作区已切回 `main`，`test/headless-primary` 及实施分支已删除，原 `skills` 改动保留。下面的体验身份与命令是历史交付记录；现有 `target/debug` 文件不能因源码切换而视为新候选，使用前必须核对 `future version --json`。后置人工步骤应改用版本明确的实际产物。

## 候选与顺序

人工体验时，主工作区 `/Volumes/ZHITAI/Workspace/future-os` 的体验分支为 `test/headless-primary`，提交为 `a0d4d8402064dab1b82b0ad39bbd645837446236`。已构建 macOS ARM64 debug CLI/GUI，CLI 自报 `0.0.2-a0d4d840+local.dirty`；dirty 来自保留的 `skills` 子模块改动。详情见[交付记录](headless-primary-handoff-2026-10-20.json)。该目录和运行程序在用户测试期间保持不动；开发继续使用独立实施 worktree。

实施分支可以继续修复和同步主干，但结果须记录其完整 SHA。当前主工作区体验结果不能自动归给后续候选。已通过自动化的完整候选及 run 见本文开头；最终发行验收的 Desktop/CLI 产物身份、Mobile 构建版本、平台及逐项人工证据仍待填写。

按用户最新安排，先进行人工体验/功能验证，实际发行打包、签名及安装升级验收后置。既有 CI 安装器回归仍需修复并通过。未执行和未反馈的项目均保持待确认；人工期间源码变化后重新验证受影响项目，不拼接不同候选结果。

## 历史 macOS 体验构建

从主工作区执行以下命令。GUI 和 headless 通常轮流运行；M11 再专门验证互斥。

```bash
cd /Volumes/ZHITAI/Workspace/future-os
BUILD_CLI=0 make run-desktop
```

正常退出 GUI 后，在真实交互终端启动 headless：

```bash
cd /Volumes/ZHITAI/Workspace/future-os
./target/debug/future host
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
| M02 | 交互终端 + 手机；独立空 Future home | 在独立目录用 `FUTURE_HOME=<临时绝对路径> ./target/debug/future host` 启动；按终端完成登录，在第二个终端用 `future remote devices add` 配手机 | 登录/配对完成后可实际发消息、同步事件；只创建原有布局的文件；重启复用凭据 | 待用户反馈 |
| M03 | 空临时目录；非交互终端 | 设置独立 `FUTURE_HOME` 后执行 `future host </dev/null`，记录退出码及输出 | 退出 1、提示平台登录需交互终端；不输出授权材料，不创建 app.db、配对或凭据 | 待用户反馈 |
| M04 | 用户已有自建中继 + 手机 | 在独立测试根运行 `future host --relay <中继 URL>`；私有 CA 时带原有 `--relay-ca <PEM>`；用 `future remote devices add` 配对并重启 | 邀请使用所选中继，凭据可复用；不因包名改变配对；origin/TLS 和 CA 边界保持 | 待用户反馈 |
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

用户已报告本轮人工测试完成，并授权准备 PR；未报告遗留失败。体验提交为 `a0d4d8402064dab1b82b0ad39bbd645837446236`。上表保留逐项反馈状态，避免将概括反馈写为每个平台/每个用例的独立通过证据。后续主干同步与 CI 夹具修复不属于该体验提交；后续 unsigned 产物验证及旧 Linux 静态 smoke 已通过，详见[构建结果](headless-unsigned-results.zh-CN.md)；签名产物、真实安装/升级/卸载及最终同候选人工结果仍待汇总。用户于 2026-10-20 取消性能对比，不再作为验收条件。创建或合入 PR 不等于整体 Goal 最终验收完成。
