# Supervisor 评审——第一个修复检查点

> 本文是历史快照 [Supervisor review — first remediation checkpoint](./supervisor-review.md)（2026-09-11，commit `914f27f6`）的忠实逐段中文翻译，保留原文结论、时间与 commit 边界；译文不是新的复核结论。

## 状态（2026-09-11）

**部分完成，未准备好做单个最终 PR。** Goal `goal_bughunt_fix_20260911`，supervisor 会话 `20260911-122531-184689`。授权的四 worker 段从 04:30:30 运行到 05:30:30 UTC。确定性 monitor 停止本 goal 的 worker 并创建延续门禁 `todo_60242d8b0774`。四个 worker 会话投影随后全部报告 `streaming: false`。未经用户重新授权，不重启任何 worker。

原始 224 个发现划分为 79 个 agent、55 个 loop/TUI、61 个 CLI/channels/RPC 与 29 个 apps/build；重复与推测机制**不**计入不同的已修复生产缺陷。Supervisor/验证者新增项在各 worker 账本中单独跟踪。

## 已评审并集成：apps/build

评审提交 `6ae8f550` 与 `2d062dcd`；合并进 `claude/bughunt-all`。阅读 29 行账本与源码/测试差异。特别检查 approval-finally 与可编辑 Escape、错误 UI 边界、搜索窗口收窄、稳定输出依赖、移除死代际管道而不移除实际基线对账、保源 markdown 深度回退、数字 token 边界、有界移动端重放保留/重试、上传 EOF 取消、结构化错误分类、补丁头/hunk 分离、home 路径验证、Windows 静态服务器路径拒绝、选项终止 macOS 打开器、profile 隔离与安装器规范化。

独立 supervisor 执行（非仅复制 worker 声明）：

- `git diff --check origin/main...HEAD`：apps 切片通过。
- 实际 `scripts/test-generate-models.py`：通过，包括 None/空源与既有目录保留。mock 测试无网络调用。
- 实际 `scripts/test-profile-isolated.py`：2 个测试通过；子 home 隔离与退出码传播；未启动实时 agent。
- 实际 `scripts/test-android-config.py`：通过，四个平台/ABI 子用例使用 mock SDK 工具。
- `bash -n scripts/start-mobile-android.sh scripts/agent-profile-bench.sh`：通过。
- 集成桌面端 `tsc --noEmit`：通过；全部 `src/**/*.{ts,tsx}` 上的 ESLint：通过。
- 集成桌面端 Vitest：**98 个文件 / 862 个测试通过**。更早的针对性通过也为 10 个文件 / 72 个测试。临时 Vite 别名显式将共享包指向集成 worktree，而非祖先 main 检出。
- 集成移动端针对性 Jest：**3 个套件 / 111 个测试通过**（`syncEngine`、`files`、`ComposerDock`），使用 worktree 本地共享包映射与现有 Expo 兼容 React 依赖。

Worker 报告的完整移动端与 Tauri 通过仍归 worker；supervisor 在本检查点未独立重跑。Worker 记录了初始 Tauri 超时与一次 DatabaseBusy 失败，随后未更改的完整重跑通过；此历史保留在 `fix-apps.md`。

范围本地接受，**不是** Windows 原生或实时服务认证。仍未完成：Windows 上离线 PowerShell 安装器回归；原生剖析/路径检查；平台 JWT TTL 证据（w30-3）；移动端重放事故的真实 journal/发布时序证据。正确转义 UNC 解析器反例与 w29-6 永久失败反驳是成立的；未集成危险路径重解释或 create_new 弱化。

## 第二轮：模型身份 / 模态流水线 / Windows 脚本 CI 评审

用户授权另外 120 分钟，截止 epoch1789115561（本地 16:32:41）。评审并集成 `6d96e393`、`bbc5efe1`、`56384018`：实际桌面/移动命令路径上的 provider 限定身份，保留的输出模态 schema/生成器/读取器，文本输出默认选择，仅附件的图像边界，以及现有必需各平台 CI 状态内的离线脚本测试。检查了现有 job/check 名称与仅脚本 checkout/skip 条件；Windows 执行仍待最终 PR。

在集成源码上的独立 supervisor 检查：

- 三个实际离线 Python 回归脚本全部通过（生成器 3 测试；剖析 2；Android 平台 fixture）。
- 桌面端完整 `tsc --noEmit` 与 ESLint 通过；新 identity/helper/hook 测试 **4/4 通过**。
- 移动端 identity/controller/outbox 测试 **61/61 通过**。首次尝试因该全新集成 worktree 缺少生成的 `src/version.generated.ts` 而失败一个套件；运行现有 `npm run gen-version` 构建前置条件后，三个套件均通过。这不是源码测试失败，且未弱化任何断言。
- 共享包别名显式指向集成源码；临时配置/依赖链接/生成版本在检查后移除。

**额外独立发现：仅流水线修复不能对发布目录关闭 w10-BUG6。** 旧条目仍缺输出元数据，因此获得兼容文本默认值。apps 第二轮交付对剩余范围记为返工，同时已评审代码被集成。跟进 `todo_64d9f49b8ea7` 在未更改截止时间内恢复同一 worker，以获取公共来源溯源并对现有目录条目做仅输出元数据修复。不授权全量刷新、记录插入/移除/重排、端点/定价/上下文变更、模型名推断、凭据或 token 捕获。精确未匹配覆盖必须保持显式。最终协调依赖此跟进及所有其他生产任务。

## 第二轮：agent 集成与独立 Windows 修正

集成 agent 提交至 `b418d41a` 以供跨 crate 检查，保留 apps 的两处 model 模态/默认过滤变更与 agent 的解析器/上下文/缓存顺序变更。Worker 的自动全 crate 验证器通过（1749 个库测试加 42 个集成测试，10 个忽略的原生/其他用例）。这不是原生 Windows 通过或 supervisor 对整个组合套件的独立重跑。阅读对账后的 79 行账本并检查 shell 审批/取消、FIFO 恢复、有界 SSE、provider 快照、修复排序、背压与 Windows token/重写路径的关键差异；`git diff --check` 通过。

**独立评审在所交付补丁中抓到两个具体 Windows 缺陷：**

1. `everyone_sid()` 分配 `Vec<usize>` 后构造 `OwnedSid(Vec<u32>)`，是 cfg(windows)-only 编译错误。将其 SID 存储修正为 DWORD 字。
2. `current_user_sid()` 为 `TOKEN_USER` 分配 `Vec<u32>`，其包含指针且在 64 位 Windows 上需要机器字对齐。将查询缓冲区修正为 `Vec<usize>`。Worker 账本声称其已机器字对齐是不准确的。

新增 `native_sid_buffers_are_valid_and_clone_byte_exact`，仅查询测试进程的 TOKEN_QUERY 身份并构造 Everyone SID，带有效性/克隆检查。在现有必需 Windows CI 构建 job 中新增原生 Windows token、PowerShell stdin 与 shell 状态/后代回归。在隔离 HOME/USERPROFILE 前保留 CARGO_HOME/RUSTUP_HOME；每次 cargo 调用失败后立即失败。这些检查**已定义、未在这台 Mac 上执行**。`cargo fmt -p future-agent --check`、`git diff --check`、CI YAML 解析/过滤器存在性与原生测试名检查在修正后通过。组合全 crate/直接消费者检查仍待全部切片集成。未打开 PR，未声称最终原生验收。

## 其他分区——未独立验收/集成

- Agent：检查点提交 `d4781920`、`1f677403`；账本覆盖 79 个分配 ID。Worker 显式报告部分实现、专用回归缺口、未解决的已确认发现与原生/协议集成缺口。它报告 1719 个库测试串行通过，但更早的并行缓存测试失败仍需调查，完整 crate 集成/doc 测试未完成。标记 done 的 loop 交付**不是**验收；需要返工/延续。
- Loop/TUI：检查点提交 `d2905a9d`；两个后续修改文件（`compat.rs`、`cli_registry_contract.rs`）在停止后仍未提交。账本覆盖 55 个原始项加 SF/NF/验证者新增。它报告 847 个 TUI 库测试与 7 个 loop 回归测试通过，但完整检查与若干已确认修复仍未完成。不要重置未提交变更。
- CLI/channels/RPC：34 个修改的受跟踪文件加新回归/报告文件在停止时**仍未提交**。不要丢弃或声称已评审。账本初始行不是最新的最终处置。需要检查差异、完成检查并在集成前做检查点提交；不要将代码改动计为已修复发现总数。

## 额外 120 分钟窗口结束（本地 16:32:41）

确定性第二检查点停止剩余 CLI 与 loop/TUI worker 并创建用户延续门禁 `todo_070dc3d252e1`。四个投影现在全部报告 streaming=false。无 PR 存在，不授权自动延长/重启。

发布模型跟进 `fd16677e` 被独立检查并集成。Supervisor 重跑其精确离线生成器验证器（**4 个测试通过**）并机械比较新旧 JSON：**3826 条有序记录、全部非输出字段未变、158 个输出新增、110 个分类非聊天条目**。七个已检查公共来源 fixture、获取时间戳/哈希与 **432 个未匹配身份**仍是显式溯源/限制。实际 cycle_model 输出过滤器与真实目录/RPC 回归已评审。Worker 交接因省略字面 `commits` 验收 token 被拒，尽管真实提交存在。Supervisor 用已验证提交、覆盖、已执行测试与缺口手工修复证据；标准未变，未重启 worker。跟进交付 `todo_64d9f49b8ea7` 对此本地范围已验证。

尝试将较早 apps 流水线交付从返工改为已验证被 loop API 拒绝（`delivery is already resolved as rework`）；该历史处置被保留，单独验证的跟进记录解决。不要重试状态转换或编辑账本事件。

停止时剩余：
- Loop/TUI 干净分支至 `3b60af69`（在 `05ca1f04`、`736830dd`、`d2905a9d` 之后），尚未集成/评审。必须检查完整最终验证/账本对账；较早部分通过不够。
- CLI/channels/RPC 提交 `dc642044`、`d9bff2ab`、`7e621a5c`，加 14 个修改文件保留未提交，包括 configure/auth/run、浏览器状态、channels、decode 与文档。账本记录完整 RPC/channels 成功、CLI clippy 成功但 719 通过/3 失败库运行；后续修正与完整集成/doc 套件需验证。不要丢弃脏文件或在无证据情况下标记这些已知失败已解决。
- 共享 RPC 增量 proto 变更需要在集成后做组合直接消费者构建/代码生成检查。
- 跨负责人 w32-BUG4 SSE 逐行后缀移动仍显式交给 supervisor 修复；w32-BUG5 WebUI 处理器加固可能在最终 loop 提交中，必须检查。
- 原生 Windows 执行、集成完整检查与单个最终 PR 加实际绿色必需 CI 仍未完成。Agent 本地通过不覆盖集成时引入的独立 Windows 修正。

## 资源记账

这四个精确 worker 会话 ID 的只读会话元数据报告输入 token 总计 23,504,426 + 27,730,610 + 23,470,340 + 24,781,031 与输出总计 38,700 + 40,575 + 43,172 + 41,284。输入含重复提供/缓存的上下文；它们不是唯一源 token。全部四个存储的 total_cost 值为 0.0，实时用量也报告零。**实际 Azure 费用不由这些计数器确立；此处为零不得被描述为免费执行。** Supervisor 用量不包含在四个 worker 总计中。

## 下一授权段应优先

1. 保留/检查点 CLI 与剩余 loop 编辑；将每个处置与当前源码和测试对账。
2. 关闭显式未实现的已确认问题与专用回归缺口，包括跨负责人模型身份/输出模态、loop worker 归属/列表宽度修复、浏览器事务/生命周期项与剩余协议契约。
3. 仔细验证 Manual shell 策略变更（不要重新引入基名/选项绕过），然后独立检查安全/终止/租约修复与跨 crate 消费者。
4. 在稳定构建环境中运行所有受影响的完整 crate/TS/mobile 检查。第一个并行段反复等待/失效共享 Cargo target；刻意安排昂贵构建而非启动冗余编译。
5. 为离线安装器测试添加/运行 Windows 验证路径，而不声称普通跨平台 cargo check 会执行 Windows 特定回归测试。
6. 仅在覆盖与验收缺口对账之后：同步 origin/main，创建**一个** PR，立即启用 auto-merge，阻塞在必需 CI 上，解决 BEHIND/失败，并在实际合并后仅清理本任务的 worktree。本检查点不存在 PR。
