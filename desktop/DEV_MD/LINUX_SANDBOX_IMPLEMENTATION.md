# Linux Bubblewrap 沙盒实施计划与验收矩阵

状态：**开发执行基线；Wave 1–5（L0–L4 与 L6 本地产品接入）已完成，等待本地总门禁与 L5 真机矩阵**（2026-09-03）。产品与安全语义以 [`LINUX_SANDBOX_PLAN.md`](LINUX_SANDBOX_PLAN.md) 为准；本文把 L0–L6 转成代码落点、提交顺序、自动化门禁和真机验收项，不改变 L-D1–L-D9。

## 1. 开发基线

- 开发分支：`claude/linux-bwrap-sandbox`
- 独立 worktree：`.claude/worktrees/linux-bwrap-sandbox`
- 分支基线：`fd3e1771`（`sandbox` / `origin/sandbox`，`docs: plan Linux bubblewrap sandbox`）
- 本机审计环境：Ubuntu 26.04 LTS、Linux 7.0、x86_64；system bwrap 为 `/usr/bin/bwrap` 0.11.1。
- 本机最小能力预检：`--new-session --die-with-parent --unshare-user --unshare-pid --unshare-ipc --cap-drop ALL --ro-bind / / --dev /dev --proc /proc -- /bin/true` 返回 0。
- 上述结果只说明当前开发机具备基础能力。Ubuntu 26.04 不属于 L5 目标发行版，不能替代 Ubuntu 22.04/24.04、Debian stable、Fedora、aarch64 或安装包实测。

所有实现和修复只在该 worktree/branch 完成。不得直接修改本地 `sandbox` 或 `main`；后续同步只允许把上游分支合入开发分支，不能把用户本地 `main` 合入开发 worktree。

## 2. 现状审计

### 2.1 Agent 规则与平台选择

| 接缝 | 开工时基线 | Linux 实施影响 |
|---|---|---|
| `agent/src/sandbox/mod.rs` | `ResolvedSandbox` 只保存 `available: bool`；macOS 检查 `/usr/bin/sandbox-exec`，Windows 使用完整 probe，Linux固定 `false`；`build_shell_command()` 直接构造 Seatbelt 或普通 shell | 需要把“布尔可用”升级成携带 backend、固定 bwrap 路径、版本、binary identity、能力和过期时间的成功凭据；Linux 不能只做 `which bwrap` |
| `agent/src/sandbox/rules.rs` | `RuleSet` 是高到低优先级的私有 matcher 层；Seatbelt 通过 `profile_layers()` 获取快照；损坏/不可读规则文件目前记录 warning 后跳过 | Linux plan 需要稳定、纯数据的规则快照接口；sandbox tier 下无法编译或读取安全规则必须 typed fail closed，不能沿用“跳过坏层” |
| `agent/src/sandbox/paths.rs` | 已有 tilde、宽松 canonicalize、symlink ancestor 与 component boundary 处理 | 可复用作输入规范化，但 mount source 仍需 FD/identity 复核；一次 canonicalize 不能作为 TOCTOU 保证 |
| `agent/src/sandbox/seatbelt.rs` | 直接从 `RuleSet` 生成 SBPL 并返回 `tokio::process::Command` | 先适配到 `PreparedShell`，保持 profile 和调用语义不变；Linux 不翻译 SBPL 字符串 |
| `agent/src/sandbox/windows*` | Windows 有 host probe、request/capability、专用 runner 与 Job 生命周期 | 可借鉴 typed probe、runner 和 capability 测试结构；不得把 ACL 或 Windows additional permissions 混入 Linux 一期 |

### 2.2 shell 执行与 escalation

- `agent/src/tools/mod.rs` 的 `run_shell_with_capability()` 决定 pre/post-hoc escalation，`spawn_shell()` 统一 cwd、环境、输出、timeout、abort 和 Unix process-group kill。
- Unix 当前先把命令变成 `( command ) 2>&1`，再调用 `ResolvedSandbox::build_shell_command()`；Linux helper 必须接收结构化 argv/request，不能把 mount 参数拼进 shell 字符串。
- `looks_like_sandbox_denial()` 目前只看 `Operation not permitted` / `sandbox-exec`。Linux 必须优先消费可信 helper violation 元数据，并谨慎补充 `EACCES`/`EPERM`/`EROFS`；exit 2/126/127 与 sandbox infrastructure error 不得触发自动脱沙盒建议。
- 当前 `spawn_shell()` 已有 timeout、interrupt 和 Unix process-group kill；Linux PID namespace/helper PID 1 的 signal forwarding、reaping、parent death 必须与这层协同测试，不能产生两套互相竞争的终止逻辑。

### 2.3 RPC、CLI、Desktop 与打包

| 接缝 | 当前实现 | 计划 |
|---|---|---|
| `agent/src/rpc/commands/settings.rs` | `set_sandbox_policy` 返回 `sandboxAvailable`；错误文本写死为 Windows；只有 `probe_windows_sandbox` | 增加平台中立 product probe 响应，包含稳定 code/backend/capabilities；保留 Windows 兼容入口直到调用方迁移完成 |
| `agent/src/cli.rs` | `--probe-windows-sandbox` / `--reset-windows-sandbox` 在 singleton lock 之前执行 | Linux probe 与隐藏 helper 同样必须在 Agent singleton lock 之前分派；helper 采用明确参数/子命令，不依赖 `argv[0]` |
| `cli/src/main.rs` | `future agent <args>` 原样进入 `future_agent::cli::run_from_args()` | `future agent <隐藏 helper>` 与 standalone `future-agent` 必须走同一实现；Desktop 仍只打包统一 `future` sidecar |
| `desktop/src/integrations/agent/useSandboxAvailability.ts` | macOS 直接 true，Windows RPC 重试，其他平台固定 false | 泛化为平台 probe；Linux 连接失败是非 definitive，稳定 probe 失败是 definitive unavailable；暴露 reason code 供安装/排障文案使用 |
| `desktop/src/features/settings/GeneralPage.tsx` 与 `Composer.tsx` | sandbox option 仅按 availability 显示；只有 Windows 专用说明 | Linux probe 成功时直接显示；失败时提供本地化原因与安装链接；中英文 key 同步 |
| `desktop/src-tauri/src/agent_bridge/*` | Windows 专名命令与 response struct；sandbox 不可用时持久化回退 manual | 泛化 response 和命令；保持“明确不可用才持久回退、瞬时 RPC 失败不改用户设置” |
| `cli/src/commands/doctor.rs` | 无 sandbox 检查 | 加入与 Agent 共用的 Linux probe 结果；机器可读 probe 由 Agent CLI 输出，doctor 只呈现，不重写探测逻辑 |
| `desktop/src-tauri/tauri.conf.json` / release scripts | 单一 `future` externalBin | 不 bundle bwrap 或第二 helper；包只需现有 sidecar，自重入 helper 不新增制品 |

### 2.4 测试结构

- 纯 Rust 单测主要位于各模块 `#[cfg(test)]`；Linux plan/probe/request 必须把 PATH、文件元数据、clock、process runner 等外部依赖注入，确保 macOS/Windows CI 也能测试纯逻辑。
- `agent/tests/sandbox_smoke.rs` 是 macOS-only ignored 真 sandbox smoke；新增平行的 `agent/tests/linux_sandbox_smoke.rs`，Linux-only、ignored、单线程运行。
- `agent/tests/cli_smoke.rs` 覆盖真实二进制与 singleton；增加 probe 输出、helper 绕过 singleton、非法/超限 helper request 的子进程测试。
- Desktop availability 已有 `useSandboxAvailability.test.ts`；迁移时覆盖 Linux success、stable unavailable、transient RPC error、共享缓存、reason code 与 manual fallback。
- RPC settings、dispatcher、session policy 与 Tauri bridge 已有邻近单测，新增行为放在现有测试文件，不另建重复 harness。

## 3. 实施顺序与代码落点

每一波先跑目标测试再提交；下一波建立在上一波已提交状态上。类型名可在实现中微调，但安全边界和输出字段不可弱化。

### Wave 1 — L0 probe 合同

建议新增 `agent/src/sandbox/linux/{mod.rs,probe.rs}`：

1. 定义 `LinuxSandboxProbe`、`LinuxSandboxProbeCode`、`BwrapIdentity` 和 capability 数据；稳定 code 至少覆盖设计稿 §4.5。
2. 安全 PATH 查找只接受绝对 PATH 项；拒绝空/相对项、workspace/cwd 及其子路径；候选必须是可执行普通文件，canonicalize 后固定绝对路径。
3. 对同一固定路径执行有界 `--version`、`--help` 参数检查和真实基线 probe。生产使用参数表必须与 probe 参数表来自同一常量。
4. 成功缓存携带 path/version/identity/capabilities/expiry；执行前 identity 不一致或缓存过期必须重新 probe。失败不做进程生命周期永久缓存。
5. 第一版最低版本不得因未使用的 `--argv0` 或 `--ro-bind-fd` 被抬高；最低版本与目标发行版包版本在 L5 真机矩阵冻结。本波在冻结前以“参数存在 + 真实 probe”为权威，版本仅提供诊断下限。
6. 增加平台中立 CLI/RPC probe；Linux 返回完整稳定结果，macOS/Windows 映射到同一 product shape。

**硬门禁：** workspace 伪造 bwrap、相对 PATH、超时、不可解析版本、缺参数、identity 改变、userns/proc 失败均在用户命令执行前 fail closed。

### Wave 2 — L1 规则快照、plan 与 `PreparedShell`

建议新增 `agent/src/sandbox/backend.rs`、`agent/src/sandbox/linux/plan.rs`：

1. 把当前私有 matcher 暴露为只读快照 DTO，不让 Linux 解析 SBPL regex；快照保留 layer/order/access/decision/original glob/canonical literal。
2. `LinuxSandboxPlan::compile()` 只做确定性规划，不 spawn：输出 writable roots、read-only/unreadable、reopen、missing exact、expanded glob、unsupported dynamic glob、policy digest 和 typed errors。
3. 规则解析错误在 sandbox tier 进入 plan error；manual/off 保持当前审批语义，避免无关行为变化。
4. 根去重和 mount 顺序显式测试：宽 writable → deny/read-only → 更窄 allow reopen；硬 deny 永不可 reopen。
5. 建立 `PreparedShell { program, args, env_delta, backend, boundary }`。macOS 先适配现有 Seatbelt；off/manual 适配普通 shell；Windows runner 可暂以 enum variant 保留专用 spawn，避免本波重写 Windows 生命周期。
6. `ResolvedSandbox` 持有可验证 backend receipt，而不是独立 bool；UI availability 与执行共享该结果。

**硬门禁：** unsupported matcher、扫描超限、规则层读取失败、危险 symlink/missing-path 形态都返回 typed error，不能静默删掉一条规则继续执行。

### Wave 3 — L2 自重入 helper 与执行链（已完成，2026-09-03）

已新增 `agent/src/sandbox/linux/{request.rs,helper.rs,runner.rs}`：

1. 版本化、有长度/数量上限的 request；只接受父 Agent 生成的结构化字段。拒绝未知版本、重复/非法 FD、NUL、非绝对 mount target 和超限 payload。
2. `future agent <hidden-helper-mode>` / `future-agent <hidden-helper-mode>` 在 singleton lock 之前分派，不读取模型配置、不启动 gRPC。
3. 外层固定 probe 凭据中的 bwrap，构造只读 root、writable roots、保护覆盖、fresh `/proc`、最小 `/dev`、user/PID/IPC namespace、cap drop、parent death；不 unshare network。
4. mount source 使用继承 FD 和 `/proc/self/fd/<n>`；内层 helper 复核 dev/inode/type/目标身份，设置 `PR_SET_NO_NEW_PRIVS`，再 exec 真实 shell argv。
5. helper 作为 PID 1 时转发信号、回收后代并保留原始 exit/signal。Agent timeout/abort 后断言无残留后代。
6. `spawn_shell()` 调用 prepared backend；helper/probe/request/identity 失败作为 infrastructure error 返回，不进入 post-hoc escalation。

### Wave 4 — L3 完整规则与 violation/escalation（已完成，2026-09-03）

1. 启动前 glob 展开使用内部 walker 作为可信实现；可选 `rg` 只能是优化，缺失/失败必须安全回到内部 walker。固定最大匹配数、节点数、深度和总耗时，任何上限命中 fail closed。
2. 同时处理 lexical path 与 canonical target；对不存在 exact path 在 sandbox view 中建立保护目标；host 占位对象 cleanup 核对 inode identity/CAS，绝不删除并发用户对象。
3. 宽保护与窄重开按路径深度排列；read allow 只重开为只读，write allow 若会绕过仍生效的 read deny 则 typed fail closed；不存在的重开目标因无法无歧义创建 file/dir mount source 同样 typed fail closed，hard deny 不可重开。
4. helper 以稳定 marker 返回 violation kind/path provenance/policy digest/affected count，日志与普通 UI 不输出完整敏感路径集合。glob 新匹配只做命令结束 detection-only，不称为动态硬保护。
5. Linux 路径拒绝可以触发一期整命令脱沙盒审批；基础设施错误、普通 command error 和 2/125/126/127 不触发。

### Wave 5 — L4/L6 产品、诊断与文档（已完成，2026-09-03）

1. Desktop 平台 probe、Settings/Composer 和 remote bridge 已改用统一 `probe_sandbox` 结果；Linux probe 成功直接显示 sandbox，失败展示本地化稳定 code 与安装/排障命令。
2. 已保存 sandbox 后环境失效时，Agent 会把请求 tier 改为 manual，`ResolvedSandbox` 在执行边界再次强制同一回退；Desktop 仅在 definitive unavailable 时持久化 manual 并显示通知。瞬时 RPC 错误保留设置等待重试，任何路径都不会静默按 off 裸跑。
3. `future-agent --probe-sandbox`（以及 `future agent --probe-sandbox`）输出 machine-readable JSON；`future doctor` 使用同一 probe 并展示 backend/code/available、system bwrap 路径和版本。probe 只暴露 bwrap 可执行文件路径，不输出规则或受保护文件路径。
4. Settings/Composer 和中英文文档明确 network open、system bwrap only、no WSL、glob 启动时快照与结束后 detection-only；不随应用打包或下载 bwrap。详细步骤与 code 表见 [`LINUX_SANDBOX_USER_GUIDE.md`](LINUX_SANDBOX_USER_GUIDE.md)。

#### Linux 安装与诊断速查

- Ubuntu/Debian：`sudo apt install bubblewrap`
- Fedora：`sudo dnf install bubblewrap`
- 机器可读诊断：`future agent --probe-sandbox`；预期 JSON 至少包含 `available`、`backend`、`code`，Linux 成功时还包含 `path`、`version`、`capabilities`。
- 汇总诊断：`future doctor`；`binary_missing` 表示未找到可信 system bwrap，`path_rejected` 表示 PATH 候选不安全，`required_feature_missing` 表示系统包缺少必要参数，`user_namespace_disabled` / `proc_mount_restricted` 表示主机策略不允许生产基线，`probe_timeout` / `probe_failed` 表示探测未正常完成。
- WSL 不受支持；网络保持开放；glob 仅对命令启动时已有匹配提供硬保护，命令中新匹配仅在结束后报告 detection-only violation。

### Wave 6 — 本地总门禁与 L5 交付

1. 执行 §4 自动化和当前 Linux 主机 smoke，结果逐项写为 PASS/FAIL/NOT RUN/ENVIRONMENT LIMIT。
2. 输出独立真机手册，覆盖目标发行版、架构、userns/proc 负向环境和 AppImage/deb/rpm；不得用容器或当前 Ubuntu 26.04 结果替代。
3. security review 至少逐项复核 mount TOCTOU、FD 泄漏、setuid bwrap、namespace/capability、临时对象清理、错误 escalation 与日志脱敏。

## 4. 可执行验收矩阵

状态含义：`PASS` 已实测通过；`FAIL` 已实测失败；`NOT RUN` 尚未执行；`ENVIRONMENT LIMIT` 当前环境不能给出目标结论。新增实现前除基线项外均为 `NOT RUN`。

| ID | 层级 | 验收项 | 自动化/命令 | 当前状态 |
|---|---|---|---|---|
| B-01 | host baseline | system bwrap 可发现并报告版本 | `command -v bwrap && bwrap --version` | PASS：`/usr/bin/bwrap`, 0.11.1 |
| B-02 | host baseline | 生产基线 namespace/ro-root/dev/proc 参数可运行 | 使用 §1 记录的 `/usr/bin/bwrap ... /bin/true` 命令 | PASS：exit 0 |
| B-03 | support baseline | 当前机属于目标发行版矩阵 | `cat /etc/os-release; uname -m` | ENVIRONMENT LIMIT：Ubuntu 26.04 x86_64 非目标版本 |
| P-01 | probe unit | PATH 空项、相对项、cwd/workspace 候选拒绝 | `cargo test -p future-agent sandbox::linux::probe` | PASS（2026-09-03：safe PATH/workspace rejection） |
| P-02 | probe unit | version/help/timeout/identity code 稳定 | 同上，fake runner/clock/metadata | PASS（2026-09-03：typed failure、timeout、cache identity/expiry） |
| P-03 | probe integration | missing/old/missing-feature/userns/proc 各返回预期 code | `cargo test -p future-agent --test linux_sandbox_smoke probe_ -- --ignored --test-threads=1` | NOT RUN |
| R-01 | rules/plan | fallback roots、外部 allow、ask/deny、层级顺序 | `cargo test -p future-agent sandbox::linux::plan` | PASS（2026-09-03） |
| R-02 | rules/plan | 窄 allow reopen、hard deny 不可 reopen、根去重 | 同上 | PASS（2026-09-03） |
| R-03 | rules/plan | symlink、missing exact、glob 快照/上限/异常 fail closed | 同上 | PASS（2026-09-03：内部 no-follow walker、2048 match/100000 node/64 depth/2s 上限；lexical+canonical symlink；typed fail closed） |
| R-04 | cross-platform | Seatbelt/Windows/manual/off 行为不回归 | `cargo test -p future-agent sandbox:: tools:: rpc::commands::settings` | PASS（2026-09-03：sandbox 115 tests；Agent clippy all tests） |
| H-01 | helper parser | version/size/count/FD/path 输入校验 | `cargo test -p future-agent sandbox::linux::request` | PASS（2026-09-03：version/path/NUL/phase/FD identity 与重复 FD；size/count 常量已强制，边界补测留 L3） |
| H-02 | helper boundary | helper 绕过 Agent singleton，但非法直接调用失败 | `cargo test -p future-agent --test linux_sandbox_smoke` | PASS（2026-09-03：非法 payload 返回 infrastructure exit 125，未创建 singleton lock） |
| H-03 | mount smoke | workspace/temp 写成功，workspace 外写失败且 host 无文件 | `cargo test -p future-agent --test linux_sandbox_smoke filesystem_ -- --ignored --test-threads=1` | PASS（2026-09-03：当前 Ubuntu 26.04/system bwrap；workspace 写成功、外部写拒绝、exit 23 原样） |
| H-04 | secret smoke | 已有 secret 精确/glob 文件读写均失败 | 同上 | ENVIRONMENT LIMIT（2026-09-03：具体 unreadable mount 历史 PASS；本轮 glob 单测 PASS，但当前执行环境 probe=`user_namespace_disabled`，新增真 bwrap smoke 未运行） |
| H-05 | missing/symlink | missing exact 不能创建；symlink 不越界；host 无临时残留 | 同上 | ENVIRONMENT LIMIT（2026-09-03：CAS inode cleanup 与 symlink 双路径单测 PASS；新增真 bwrap smoke 因当前环境禁用 userns 跳过） |
| H-06 | network | 未 unshare network；本地 TCP/namespace identity 验证网络保持开放 | 同上 | NOT RUN（argv 单测确认未使用 `--unshare-net`，仍需网络 smoke） |
| H-07 | lifecycle | 正常 exit/signal 原样；abort/timeout/parent death 无后代残留 | 同上 | NOT RUN（2026-09-03：exit/signal/parent-death smoke 已 PASS；Agent timeout/abort 待补） |
| H-08 | FD security | 仅 stdio/request/mount FD 可见，Agent listener/db/log FD 不继承 | 同上 | PASS（2026-09-03：mount FD 仅传给 bwrap，inner command smoke 未见 fd > 2） |
| V-01 | violation | EACCES/EPERM/EROFS 结构化分类；普通失败和 2/126/127 不误判 | `cargo test -p future-agent sandbox::linux::violation sandbox::tests::linux_denial` | PASS（2026-09-03：可信 marker 优先、推断 provenance；2/125/126/127 与普通错误排除） |
| V-02 | escalation | Linux policy violation 可审批单次脱沙盒；infra failure 绝不 escalation | `cargo test -p future-agent tools:: rpc::approval` | PASS（2026-09-03：Linux classifier 接入既有整命令 post-hoc escalation；prepare/helper exit 125 不触发） |
| G-01 | glob | 启动前已有匹配被硬保护；命令中新匹配只报告 detection-only | ignored Linux smoke | ENVIRONMENT LIMIT（2026-09-03：展开/检测单测 PASS；新增 combined smoke 因当前环境 probe=`user_namespace_disabled` 跳过） |
| U-01 | RPC/Desktop | Linux availability/retry/reason/manual fallback | `cd desktop && npx vitest run src/integrations/agent/useSandboxAvailability.test.ts`；Rust bridge/settings tests | PARTIAL PASS（2026-09-03：Agent settings、dispatcher、Tauri bridge targeted tests 与 Tauri check/clippy PASS；TS 测试已补，当前环境无 `node`/`npx`，待 Q-02 执行） |
| U-02 | i18n/UI | Settings/Composer 安装与限制文案中英文齐全 | `cd desktop && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run` | IMPLEMENTED / NOT RUN（2026-09-03：中英文文案已同步；当前环境无 `node`/`npx`） |
| C-01 | CLI | machine-readable probe 与 doctor code 一致 | `cargo test -p future-agent --test cli_smoke && cargo test -p future-cli doctor` | PASS（2026-09-03：Agent machine-readable probe smoke PASS；future-cli doctor 20 tests PASS；本机实测 JSON 为 `linux_bubblewrap/user_namespace_disabled`） |
| Q-01 | Rust gate | workspace + Tauri fmt/clippy | `make lint-rust` | NOT RUN |
| Q-02 | all tests | Rust、Desktop、Mobile 全量单测 | `make test` | NOT RUN |
| L5-01 | real hosts | Ubuntu 22.04/24.04、Debian stable、Fedora；x86_64/aarch64 | 真机手册逐项执行 | NOT RUN |
| L5-02 | packages | AppImage/deb/rpm 安装、升级、卸载与 system bwrap 引导 | 真机手册逐项执行 | NOT RUN |
| L5-03 | security review | TOCTOU/FD/setuid/namespace/cleanup/escalation/logging review | review checklist + reviewer sign-off | NOT RUN |

## 5. 提交与完成规则

- 每个 Wave 至少一个独立提交；提交说明只描述该 Wave，禁止顺手重构无关模块。
- 修改 proto 后运行 `make generate-proto` 并提交两个受影响的生成文件；若平台 probe 仍能使用现有 untyped command/JSON 响应，则不为未来可能性扩 proto。
- Tauri Rust 检查前运行 `make desktop-sidecar-placeholder`。
- 集成测试必须默认 ignored，并在缺 bwrap/不支持环境时给出明确 skip 或 stable probe code；单元测试不能依赖开发机 HOME、PATH 或真实用户规则。
- PR 前按仓库流程合并最新 `origin/main`（若目标仍为 `sandbox`，先由维护者确认最终基线）、执行完整门禁并再次同步。不得把本地用户 `main` 合入开发 worktree。
- “代码完成”不等于“发布验证完成”。L5-01～03 未全部给出真实 PASS 前，交付状态只能是“可供用户真机实测”，不能宣称 Linux sandbox 已满足主干发布门槛。
