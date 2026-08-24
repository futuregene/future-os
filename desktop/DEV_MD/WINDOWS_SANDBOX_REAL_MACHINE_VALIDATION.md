# FutureOS Windows Unelevated Sandbox 真机验证手册

状态：**底层批量验收已 PASS；安装包生命周期与发布矩阵待执行**

适用分支：`codex/windows-unelevated-sandbox`

目标平台：Windows 11，普通非管理员用户，本地 NTFS

自动化范围：Agent、sandbox、RPC、Desktop Rust 后端；**不包含 UI 自动化，也不接入 CI**

本文只记录必须在 Windows 真机完成的工作。平台无关的生命周期、审批语义、失败路径、格式和 Clippy 测试应在提交前由开发端完成，不在这里重复。

## 1. 验证目标

本轮必须确认以下结论：

1. `RestrictedToken + WRITE_RESTRICTED + capability SID` 能在目标 Windows 主机上启动 PowerShell，并保持预期写边界。
2. Agent 是用户级单例；修改 gRPC 端口不能绕过，强杀后 OS 会释放锁。
3. workspace/session temp 可写，未批准的外部路径不可写；一次批准不扩展到 sibling 或父目录。
4. 活动 Job 使用 capability 时，GC/reset 不撤销其 ACE；Job 结束后可以清理。
5. 正常退出、运行期错误和下次启动回收不会遗留 FutureOS capability 记录。
6. Unicode 路径、PowerShell 5.1/pwsh 7、重定向、管道和大输出不乱码、不死锁。
7. probe/reset 经发布使用的统一 `future agent` CLI 路由工作，并且所有不支持/失败情况 fail closed。
8. Desktop 后端的退出顺序始终是“清理权限，再终止 bundled Agent”；超时或 reset 失败仍能继续退出。

这些验证通过也**不表示 Windows 与 macOS Seatbelt 等价**。Windows 第一版仍只承诺写保护；shell 读取和网络开放，glob、未来文件名、`FILE_DELETE_CHILD`、Everyone/Logon 宽 ACL 等边界以 `SANDBOX_PLAN.md` §11.6 为准。

## 2. 执行前准备

### 2.1 主机要求

- 使用普通 Windows 用户登录；不要以管理员身份打开 PowerShell。
- 仓库和 `%TEMP%` 必须位于本地 NTFS。
- 关闭 FutureOS，并停止手工启动或服务管理的 Future Agent。
- 不要删除或清理仓库中的未跟踪文件；测试脚本只读取 Git 状态。
- Rust/Cargo 使用仓库 `rust-toolchain.toml` 指定版本。
- 已安装 Visual Studio Build Tools 的 MSVC C++ 工具链和 Windows SDK。

脚本会拒绝管理员会话和非 NTFS `%TEMP%`。诊断时可传 `-AllowElevated`，但管理员结果不能作为产品验收结果。

### 2.2 获取待测版本

```powershell
cd C:\Workspace\future-os
git switch codex/windows-unelevated-sandbox
git pull --ff-only
git rev-parse HEAD
git status --short
```

将 `git rev-parse HEAD` 的值保留在报告中。若工作区存在自己的未提交修改，可以继续测试，但反馈时必须同时提供 `git status --short`；不要为了测试执行 `git clean`、`git reset --hard` 或删除构建产物。

### 2.3 确认普通用户环境

```powershell
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
rustc -Vv
cargo -V
```

第一条结果必须为 `False`。

## 3. 主验收：一次运行完整批量脚本

在仓库根目录执行：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox.ps1 -IncludeClippy
```

脚本会把完整报告写入：

```text
target\windows-sandbox-results\windows-sandbox-<时间戳>.log
```

脚本执行以下底层验证：

| 验证组 | 主要覆盖 |
|---|---|
| Windows sandbox native/end-to-end | token、SID、ACL、路径冻结、reparse/UNC、Job、PowerShell、Unicode、大输出、真实写边界 |
| Capability approval/receipt | 行为与目标绑定、request/hash/path/scope、防篡改、一次/项目审批语义 |
| Agent singleton lifecycle | 不同端口仍互斥；维护 probe 不受单例阻塞；强杀释放锁；可重新启动 |
| Desktop graceful shutdown lifecycle | bundled Agent 才清理；先清理后终止；成功、失败、超时均有确定结果 |
| Release CLI probe | `future agent --probe-windows-sandbox` 走真实统一 CLI，必须返回 `available:true` 才通过 |
| Agent/Desktop Clippy | 两个 Rust 后端均使用 `--all-targets -- -D warnings` |
| Persistent state check | 结束时 `~/.future/windows-capabilities.json` 中记录数必须为 0 |

Desktop Rust 测试需要 Tauri sidecar 文件。脚本只在文件不存在时创建空占位文件，并在成功或失败后删除自己创建的占位；已有真实 sidecar 不会被覆盖或删除。

## 4. 结果判定

### 4.1 PASS

报告末尾必须同时出现：

```text
Remaining persisted Windows capability records: 0
RESULT: PASS
```

所有命令的 `EXIT CODE` 必须是 `0`。只有这种结果可以进入下一阶段的安装包生命周期验证。

### 4.2 UNSUPPORTED

若报告为：

```text
RESULT: UNSUPPORTED
```

表示主机拒绝了当前 RestrictedToken 后端。它是 fail-closed 结果：没有命令被自动改为 unsandboxed 执行。请提供完整日志；不要改为管理员运行后把管理员结果视为通过。

### 4.3 FAIL

任何测试失败、probe `available:false`、残留 capability record、Desktop 编译失败或脚本异常都属于 `FAIL`。请保留现场，不要先手工修改 ACL。

反馈以下文件和信息：

```powershell
git rev-parse HEAD
git status --short
Get-ChildItem .\target\windows-sandbox-results\windows-sandbox-*.log |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1 -ExpandProperty FullName
```

发送完整 `.log` 文件，不要只复制最后一行或截图。

## 5. 主脚本通过后的安装包生命周期验证

此阶段需要包含本分支代码的 Windows portable/installer 构建。仍使用普通用户会话。

### RM-01：bundled Agent 单例与归属

1. 确认没有 FutureOS/Future Agent 进程。
2. 启动 FutureOS，等待 Agent 可连接。
3. 再次启动 FutureOS。
4. 检查进程：

```powershell
Get-CimInstance Win32_Process |
    Where-Object { $_.Name -in @('future.exe', 'future-agent.exe') } |
    Select-Object ProcessId, Name, CommandLine
```

预期：同一用户只有一个带 `agent` 子命令的 Agent；第二次启动应用不会产生第二个 Agent。

### RM-02：正常退出

1. 正常关闭 FutureOS。
2. 等待最多 10 秒。
3. 再次执行上述进程查询。
4. 检查 capability 状态：

```powershell
$agentHome = @($env:HOME, $env:USERPROFILE) |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and [IO.Path]::IsPathRooted($_) } |
    Select-Object -First 1
if ([string]::IsNullOrWhiteSpace($agentHome)) {
    $agentHome = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
}
$statePath = Join-Path $agentHome '.future\windows-capabilities.json'
if (Test-Path -LiteralPath $statePath) {
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    @($state.records).Count
} else {
    0
}
```

预期：bundled Agent 已退出，记录数为 `0`。不要把完整状态文件内容贴入公开报告，因为其中可能包含本机路径。

### RM-03：应用重启路径

分别验证以下会调用 `app.restart()` 的路径：

- 清除应用数据后重启；
- 开发构建切换 FutureGene 环境后重启；
- 安装更新后选择重启。

每次重启前后检查：

- 旧 Agent 已退出；
- 新 Agent 只有一个；
- 新 Agent 使用预期配置；
- capability 记录数为 `0`；
- 没有旧 Agent 占用 gRPC 端口。

如果当前测试包无法触发某一条路径，在反馈中标为 `NOT RUN`，不要用其他路径的结果代替。

### RM-04：外部 Agent 不归桌面管理

1. 在普通 PowerShell 中手工启动与桌面配置相同地址的 Agent。
2. 启动 FutureOS，确认桌面连接到该 Agent，而不是再生成 bundled Agent。
3. 关闭 FutureOS。
4. 确认手工 Agent 仍在运行。
5. 用一次 Ctrl+C 正常结束手工 Agent。

预期：桌面不终止、不替外部 Agent 清理；外部 Agent 自己正常退出时执行清理。测试结束后确认 capability 记录数为 `0`。

### RM-05：异常退出与启动恢复

1. 启动 FutureOS/Agent。
2. 从任务管理器强制结束 FutureOS 与 bundled Agent，模拟无法执行退出回调的崩溃。
3. 确认 Agent 进程锁不会永久阻止下一次启动。
4. 再次启动 FutureOS。
5. 检查只有一个 Agent，且 capability 记录数最终为 `0`。

预期：强杀本身不承诺同步清理；下一次 Agent 成功获取单例锁后执行启动回收。若仍有活动 capability 文件锁，回收必须失败关闭而不是误删活动权限。

### RM-06：设置 reset 与统一 CLI reset

在没有活动任务时执行：

```powershell
cargo run --quiet --manifest-path .\cli\Cargo.toml -- agent --reset-windows-sandbox
```

预期输出形如：

```json
{"removedCapabilities":0}
```

若存在活动 sandbox Job，reset 应失败并保留记录，不能终止命令；任务结束后重试应成功。设置页的“重置写保护”必须调用同一 RPC 语义，但本手册不要求 UI 自动化。

### RM-07：卸载清理

使用包含本分支 NSIS hook 的安装包：

1. 确认 FutureOS 已退出且没有活动 Agent/Job。
2. 正常卸载。
3. 确认 FutureOS 自己记录的 capability ACE/metadata 已清理。
4. 确认用户文件未被删除，原有非 FutureOS DACL 未被覆盖。

如果卸载前 reset 失败，卸载必须保留可重试的应用/sidecar，而不是先删除清理工具。失败时保留安装器日志和 capability record 数量，不要手工使用 `icacls /reset`，因为它会修改与 FutureOS 无关的 ACL。

## 6. 建议主机矩阵

至少完成：

| 维度 | 最低覆盖 |
|---|---|
| Windows edition | Windows 11 Home；发布前再补 Windows 11 Pro |
| Shell | Windows PowerShell 5.1；安装 pwsh 时再跑 PowerShell 7 |
| 区域/路径 | 中文用户名或中文 workspace；ASCII workspace |
| 构建形态 | 源码 debug；portable；NSIS installer |
| 权限 | 普通用户为验收；管理员仅诊断 |
| 安全软件 | Windows Defender 默认开启 |

每台主机分别保存日志，不要覆盖或合并多台机器的输出。

## 7. 验收记录模板

```text
Commit:
Windows edition/build:
Architecture:
User elevated: False
Workspace filesystem: NTFS
TEMP filesystem: NTFS
PowerShell tested: 5.1 / 7

Batch script: PASS / UNSUPPORTED / FAIL
RM-01 bundled singleton: PASS / FAIL / NOT RUN
RM-02 normal exit: PASS / FAIL / NOT RUN
RM-03 restart paths: PASS / FAIL / NOT RUN
RM-04 external Agent ownership: PASS / FAIL / NOT RUN
RM-05 crash/startup recovery: PASS / FAIL / NOT RUN
RM-06 reset: PASS / FAIL / NOT RUN
RM-07 uninstall cleanup: PASS / FAIL / NOT RUN

Remaining capability records:
Log file:
Notes:
```

## 8. 发布门槛

以下条件全部满足前，`platform_sandbox_available()` 在 Windows 必须继续返回 false，桌面不得向普通用户开放 sandbox 档：

- 目标 Home/Pro 主机的批量脚本均 `PASS`；
- RM-01 至 RM-07 的适用项目通过；
- 没有高优先级安全问题；
- 不支持主机稳定 fail closed；
- 升级、退出、崩溃恢复、reset、卸载均无不可恢复的 FutureOS ACL；
- 产品说明明确“写保护，不限制 shell 读取和网络”；
- 保留退回 `manual` 的恢复路径。

通过后再进入 W7：接通 Windows 平台可用性判断、隐藏灰度开关和小范围发布。不得为了联调提前把 Windows 产品入口默认打开。

---

## 9. 真机验证运行记录（2026-08-24）

### 9.1 结果：PASS

主验收脚本 `powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox.ps1 -IncludeClippy` 完整通过，报告末尾满足 §4.1 的全部 PASS 条件：

```text
{"available":true,"code":"available"}          ← Release CLI probe
Remaining persisted Windows capability records: 0
RESULT: PASS
```

日志文件：`target\windows-sandbox-results\windows-sandbox-20260824-102215.log`

| 验证组 | 结果 |
|---|---|
| Windows sandbox native/end-to-end | 50 通过 |
| Capability approval/receipt | 11 通过 |
| Agent singleton lifecycle | 1 通过 |
| Agent Clippy（`-D warnings`） | 通过 |
| Desktop graceful shutdown | 2 通过 |
| Desktop Clippy（`-D warnings`） | 通过 |
| Release CLI probe | `available:true` |
| 残留 capability 记录 | 0 |

### 9.2 主机环境

```text
Commit: a55c558200a80d2c2008c6ee2ef0c0c0ce86aa8e
Windows edition/build: Windows NT 10.0.26200（AMD64）
User elevated: False
Workspace filesystem: NTFS
TEMP filesystem: NTFS
PowerShell tested: 5.1
Rust toolchain: 1.97.0（x86_64-pc-windows-msvc）
```

### 9.3 过程中修复的问题（6 处，均已落到分支）

1. **`agent/Cargo.toml`** — `probe_host()`（生产代码，供 `--probe-windows-sandbox` 调用）使用 `tempfile::tempdir()`，但 `tempfile` 只声明在 `[dev-dependencies]`，Windows 编译 lib 报 `E0433`。已补到 `[target.'cfg(windows)'.dependencies]`。
2. **Agent 用户目录解析** — 单例测试失败（"first agent did not acquire its instance lock"）。根因：Windows 上 `dirs::home_dir()` 读取 token profile、忽略测试或 portable 环境设置的 `HOME`/`USERPROFILE`。第一次修复只覆盖了单例锁，仍可能让 capability 状态、认证、模型设置、规则和默认工作区落到真实用户目录。现已收敛为唯一解析入口：绝对且非空的 `HOME` → `USERPROFILE` → 系统 profile；相对/空值不接受。主测试脚本按同一规则记录 `Agent home` 并检查该目录下的 capability 状态。
3. **`desktop/src-tauri/build.rs`** — Desktop 测试二进制启动即崩 `0xc0000139`（STATUS_ENTRYPOINT_NOT_FOUND）。根因链：`tauri-plugin-dialog`→`rfd` 静态链接 `TaskDialogIndirect`（仅存在于 comctl32 v6），而 Tauri 仅通过 `rustc-link-arg-bins` 将含 v6 manifest 的 `resource.lib` 给 bin，`cargo test` 的 lib-test 二进制无 manifest → 回退到 comctl32 v5 → 入口点缺失。修复：禁用 Tauri 的 bin-only manifest（`new_without_app_manifest()`），改由 build.rs 统一为所有目标嵌入 v6 manifest（已用 mt.exe 验证 bin 与 lib-test 均正确，正式 bin 无回归）。
4. **`desktop/src-tauri/src/lib.rs`** — Rust 1.97 clippy 新 lint（`unnecessary_map_or`→`is_none_or`、`needless_borrow`）被 `-D warnings` 拦截。
5. **`desktop/src-tauri/src/remote/transfer.rs`** — `ensure_private_dir` 仅在 `#[cfg(unix)]` 测试中使用，Windows 上触发 `unused import`，改为 `#[cfg(unix)] use`。
6. **`agent/tests/cli_smoke.rs`** — 日志 smoke 会继承开发机的 `RUST_LOG`；当外层设置为 `warn` 时，测试要求出现的 info 日志被过滤，造成与产品逻辑无关的假失败。该用例现固定使用 `RUST_LOG=info`，不再依赖调用者环境。

### 9.4 尚未执行

§5 的安装包生命周期验证（RM-01 ~ RM-07）需要包含本分支代码的 Windows portable/installer 构建，属于主脚本通过后的下一阶段，本次未执行。

### 9.5 本次收口后的复验要求

本次又统一了用户目录解析，因此 9.1 的 PASS 仍是核心后端有效证据，但不能替代对最新提交的回归。更新分支后先重跑 §3；报告中新增的 `Agent home` 必须是预期目录，末尾仍须为 `Remaining persisted Windows capability records: 0` 和 `RESULT: PASS`。不需要为该复验新增 CI，仍只使用本手册的 Windows 真机脚本。

## 10. 后续任务与执行顺序

下面是发布前唯一剩余清单。每一步都保留对应日志/截图或命令输出；前一优先级失败时先修复，不提前打开 Windows 产品入口。

| 优先级 | 任务 | 测试方法 | 通过标准 |
|---|---|---|---|
| P0 | 最新提交底层回归 | 普通 PowerShell 运行 §3 的完整脚本并带 `-IncludeClippy` | 所有命令 exit 0；probe `available:true`；Agent home 正确；残留记录 0；`RESULT: PASS` |
| P1 | portable/installer 单例与正常退出 | 按 RM-01、RM-02 检查进程与 capability record | 同用户仅一个 Agent；正常退出先清理再结束；记录数 0 |
| P1 | 三种桌面重启路径 | 按 RM-03 分别执行清数据、切环境、更新重启 | 旧 Agent 退出、新 Agent 单例、端口释放、记录数 0；不可触发的项目明确记 `NOT RUN` |
| P1 | 外部 Agent 归属 | 按 RM-04 手工启动 Agent，再启动/退出桌面 | Desktop 不接管或终止外部 Agent；外部 Agent 自行退出后清理为 0 |
| P1 | 崩溃恢复 | 按 RM-05 强杀桌面和 Agent，再重新启动 | 锁由 OS 释放；启动回收旧记录；不能回收时 fail closed，不误删活动授权 |
| P1 | reset 与活动 Job | 按 RM-06 分别在无任务、活动 sandbox Job 下运行统一 CLI | 空闲时成功且记录归零；活动时拒绝 reset 且不终止任务；结束后可重试成功 |
| P1 | NSIS 卸载 | 按 RM-07 使用真实安装包卸载 | 只撤销 FutureOS ACE/metadata；不删用户文件、不覆盖其他 DACL；失败时保留可重试工具 |
| P2 | 支持矩阵 | 在 Windows 11 Pro、PowerShell 7、中文用户名/路径上重跑 §3，并至少覆盖 portable | 各主机独立 PASS 日志；差异均 fail closed 或有已记录的产品边界 |
| P3 | W7 产品接入 | 完成上述证据后实现动态 probe gate、灰度开关、`manual` 回退和非敏感遥测 | 安全 review 无高优先级问题；不上传原始路径；探测/初始化异常自动回到 `manual` |

测试分层保持不变：P0 是无 UI 的底层逻辑/原生集成测试；P1 是进程、安装包和 ACL 生命周期手工验收，不要求 UI 自动化；P2 是兼容矩阵；P3 才允许接通产品入口。详细操作以 §3、§5、§6 为准，表格不替代这些步骤。
