# Linux Bubblewrap 沙盒实施计划与验收矩阵

状态：**开发执行基线；此前 Linux 开发机的 product probe 与 5 个 ignored Bubblewrap smoke 已实际 PASS；2026-09-03 安全 review 修订后共有 7 项 smoke，需要在 Linux 主机重新执行。当前 macOS 已完成跨平台单测、fmt/clippy；L5 目标真机矩阵与安全 review 复验尚未完成**。产品与安全语义以 [`LINUX_SANDBOX_PLAN.md`](LINUX_SANDBOX_PLAN.md) 为准；本文把 L0–L6 转成代码落点、修订记录、自动化门禁和真机验收项，不改变 L-D1–L-D9。

## 1. 开发基线

- 当前开发分支：`sandbox`（按维护者要求，review 修订直接落在该分支）
- 分支基线：`fd3e1771`（`sandbox` / `origin/sandbox`，`docs: plan Linux bubblewrap sandbox`）
- 主干同步：2026-09-03 已将 `origin/main` 的 `15d7df79` 合并到 `sandbox`，merge commit 为 `0867b0fd`；合并无冲突。
- 初始实现的历史审计环境：Ubuntu 26.04 LTS、Linux 7.0、x86_64；system bwrap 为 `/usr/bin/bwrap` 0.11.1。
- 该 Linux 主机当时的最小能力预检：`--new-session --die-with-parent --unshare-user --unshare-pid --unshare-ipc --cap-drop ALL --ro-bind / / --dev /dev --proc /proc -- /bin/true` 返回 0。
- 上述结果只说明历史开发机具备基础能力。Ubuntu 26.04 不属于 L5 目标发行版，且证据早于本轮修订，不能替代当前 7 项 smoke、目标发行版、aarch64 或安装包实测。

Linux 初始实现来自 `claude/linux-bwrap-sandbox`，本轮安全修订以 `sandbox` 为唯一交付分支。后续同步继续以 `origin/main` 为主干来源；不得把未确认的本地 `main` 状态当作远端基线。

### 1.1 2026-09-03 安全 review 修订索引

这张表是后续 reviewer 的入口。每一项都同时列出安全语义、实现落点和主要回归证据；真机项尚未执行时不得从单元测试推断为 PASS。

| ID | 修订 | 实现落点 | 主要证据 |
|---|---|---|---|
| SR-01 | system `bwrap` 候选必须由 root 所有；明确接受“只查文件 owner、不递归检查父目录和模式位”的阶段性取舍 | `linux/probe.rs` | `rejects_bwrap_not_owned_by_root_before_executing_it`；NEG-02/SEC-03 待真机复核 |
| SR-02 | Linux mount plan 与 `RuleSet::evaluate()` 一致，跨层及同层均按原始顺序 first-match；被更早规则命中的后续规则不再产生 mount | `linux/plan.rs` | `same_layer_first_match_wins_for_overlapping_write_rules` 及 plan 单测 |
| SR-03 | 目标要求为受保护 host 路径零写入；helper 不直接创建源，但 bwrap 自身 mkdir 尚未隔离 | `linux/helper.rs`、`linux/request.rs` | **2026-09-04 Review 更正：P0 设计缺口**，`--tmpfs` 不保证 namespace-only 创建，见异常报告 §3 |
| SR-04 | 只有匹配当前 policy digest 的非 detection-only `filesystem_denied` marker 才可直接进入 escalation；检测 marker 不触发脱沙盒重跑 | `linux/violation.rs`、`tools/mod.rs` | classifier 与 post-hoc escalation 单测 |
| SR-05 | 命令结束后的 glob 复扫失败只发 `dynamic_glob_scan_failed` 检测 marker，保留原命令 exit/signal，避免副作用后二次执行 | `linux/helper.rs` | `glob_rescan_failure_preserves_the_completed_command_status` 待 Linux 重跑 |
| SR-06 | production helper JSON 通过继承匿名文件 FD 传输，argv 只保留短 `fd:3` 引用；outer→inner 同样使用匿名 FD | `sandbox/backend.rs`、`linux/request.rs`、`linux/runner.rs`、`linux/helper.rs` | payload 单测；`production_request_fd_transport_reaches_both_helper_phases` 待 Linux 重跑 |
| SR-07 | 可变长 bwrap mount argv 改为 `--args FD` NUL 分隔匿名文件，避免触发 `execve` 总 `ARG_MAX`；FD payload 上限为 16 MiB | `linux/helper.rs`、`linux/probe.rs` | NUL 编码单测；全部真实 bwrap smoke 待 Linux 重跑 |
| SR-08 | 最低 system bwrap 版本固定为 0.9.0；版本、必需参数与真实 runtime probe 三层均必须通过 | `linux/probe.rs` | 0.8.0 typed failure、0.9.0/更新版本比较单测；目标发行版待真机复核 |
| SR-09 | inner helper 在 `--cap-drop ALL` 后用 `capget` 复核 effective/permitted capability 均为零；一期不实现 seccomp | `linux/helper.rs`、设计稿 | Linux smoke 待重跑；seccomp 取舍见设计稿 L-D11 |
| SR-10 | Linux Desktop 始终保留沙箱选项以稳定布局；检测中/不可用时禁用，并按稳定 code 展示“原因 + 方案 + code”，明确修复后重启 FutureOS | `GeneralPage.tsx`、`Composer.tsx`、`linuxSandboxStatus.ts`、中英文 i18n | status mapping、disabled menu 与 availability 测试 |

本轮整理还修复了 SR-03 引入后的一个构造回归：`MissingProtected` 没有 host source FD，生成 bwrap 参数时必须直接走 `--perms 000 --tmpfs <target>`，不能先读取 `source_fd`。该行为由 missing-path 真机 smoke 覆盖。

2026-09-04 后续现场与复查：[异常分支审查报告](LINUX_SANDBOX_FAILURE_REVIEW.md)。已修复缺失路径重复 bind、现存读写 guard 重复遮罩和普通 mount size/mtime 误判；新报告明确区分本轮修复与仍开放的 missing-parent P0。此前“bwrap 创建目标只影响 namespace”的表述不成立，不能据此宣传无宿主残留或默认 HOME 下必定启动成功。

### 大仓库扫描修复（2026-09-04）

现场 `.env.*` 报 `sandbox glob scan limit exceeded`：旧实现每条规则递归遍历整个静态根，即使 `.env.*` 只匹配第一层；每个目录项还会重新构造匹配状态。仓库的 `node_modules`/`target` 因而能使 `pwd` 在启动前失败。

实现入口：`linux/glob_scan.rs`；`plan.rs` 启动前与 `helper.rs` 结束后共同调用。参考本地 Codex `f20b63e85c` 的扫描根分组和预编译思想，但不照搬外部 `rg`、files-only 结果或 globset 语法。

- 相同静态根只遍历一次，保留每个 pattern 的独立结果，随后按原规则层级/顺序生成 mount；`.env.*` 只扫描根目录，`pkg-*/secrets/*.key` 剪掉不可能匹配的目录，`**` 才允许递归。语义上不需下探时正常完成，不误报深度上限。
- 隐藏文件、被 gitignore 忽略的内容、匹配的目录都包含；不跳过 `node_modules`、`target`、`.git`。不跟随目录 symlink，匹配到的 symlink 同时保留 lexical 和 canonical target；坏链/读取失败 fail closed。不存在的静态根可为空，其他 metadata 错误不得当成不存在。
- 取消 100,000 节点硬上限，节点数仅作统计。启动前/结束后分别共享 **30 秒**预算（初始值，待 Linux 性能校准），最多 **256 个唯一 pattern、2,048 个唯一结果路径、4 MiB 结果关联字节估算、64 层递归深度**；后续 request 的 8 MiB、mount 数量和 bwrap args 限制保持。全局结果限制比旧的逐 pattern 限制更保守，不直接提升到 Codex 的 8,192，因为 helper 还要为 mount 打开 FD。
- 30 秒是协作式 wall-clock 预算：目录 I/O 与匹配之间检查，不保证抢占卡死的内核文件系统调用。启动前接入 Abort 原子标志；执行前扫描预算与命令执行 timeout 分开。结束后由 helper 重新扫描，不复用旧文件集合；该复扫受命令整体生命周期/timeout 管理。
- Linux 启动前准备在 `spawn_blocking` 中执行，避免扫描占住异步事件循环导致 Abort 无法处理；worker 只返回 prepared request，不启动用户命令，主任务返回后再次检查 Abort。
- 错误携带 `phase`（`pre_launch`/`post_command`）、root、相关 pattern、visited/matches/elapsed 和上限；code 区分 `glob_scan_timeout`、`glob_scan_match_limit`、`glob_scan_result_bytes_limit`、`glob_scan_pattern_limit`、`glob_scan_depth_limit`、`glob_scan_io_error`、`glob_scan_cancelled`、`glob_scan_pattern_invalid`。日志不输出完整敏感文件清单。
- helper 在 exec 前将 stderr 合并到已捕获的 stdout，保留 bwrap/inner 初始化诊断。bwrap 未返回 inner command status 且非信号终止时作为基础设施失败退出 125，不把其 stderr 中的 EPERM 误认作用户命令拒绝并邀请脱沙盒重跑。
- 复扫失败仅报告检测失败与原始诊断，仍保留已经完成的命令 exit/signal，不重跑；运行中新匹配仍只是 detection-only，不宣称动态硬保护。

新增回归覆盖单层/有限前缀剪枝、合并遍历、隐藏/忽略/目录匹配、symlink、共享结果和 pattern 预算、超时/取消、matcher 对照、stderr 捕获及超过十万目录项的显式大夹具。后者为 ignored 验收，已接入真机脚本。完整 Linux sandbox `pwd`、冷/热缓存以及 helper Linux-only 测试当前仍为 **NOT RUN**，以真机证据更新，不继承下文历史 PASS。

本轮 macOS 本地结果：`cargo test -p future-agent --lib sandbox::linux -- --test-threads=1` 为 40 PASS、1 ignored；另行显式运行大夹具 1 PASS。夹具的完整默认 glob 遍历 100,013 项，first/repeat 约 777/783 ms；单独 `.env.*` 仅访问 3 项。未清理操作系统缓存，因此这些数字不是 cold-cache 基准，更不是 Linux bwrap 全链路性能。`cargo fmt --all --check`、future-agent all-targets Clippy 与脚本语法/diff 检查用于静态验证。

### 1.2 仍开放的 review 项

以下项目没有混入本轮六项定向修复。优先级以“进入主干并面向真实用户”为基准；开发分支可继续迭代，但不得把开放项写成已完成。

| ID | 优先级 | 开放项 | 风险与建议 |
|---|---|---|---|
| OR-01 | P0 / 发布阻断 | 当前 7 项 Linux smoke 与目标发行版矩阵未执行 | helper、FD 和 bwrap mount 路径受 `cfg(target_os = "linux")` 保护，macOS 编译与单测不能覆盖。至少先在一台原生 Linux 跑 7/7，再完成 L5 矩阵。 |
| OR-03 | P1 | 非结构化 stderr denial heuristic 仍可能误报 | 本轮已禁止 detection-only/digest mismatch marker 触发 escalation；但普通程序自己输出 `Permission denied`/`Read-only file system` 且非零退出，仍可能弹出整命令脱沙盒审批。无法从合并 stdout/stderr 证明 errno 来源；长期应以显式路径能力取代自动整命令重跑，短期至少补负向 corpus 并收窄启发式。 |
| OR-06 | P0 / 修复待真机验收 | 大仓库 glob 扫描与预算 | 2026-09-04 改为同根合并、有限深度剪枝、共享预算；详见下方“大仓库扫描修复”。Linux 大仓库 `pwd` 和冷热缓存性能仍需验收，不以 macOS 静态检查代替。 |
| OR-07 | P3 / 已接受 | bwrap 仅校验 root owner，不审计父目录和模式位 | 这是维护者明确接受的威胁模型取舍，已记录在代码和设计稿。除非威胁模型扩展，不阻断本期；后续可集中加固完整路径链。 |
| OR-08 | P1 / 已知版本取舍 | 0.9.0 是产品兼容下限，不是上游安全补丁下限 | Bubblewrap 0.12.0 修复了 setup 阶段在攻击者可控目录内容下创建目标时的绝对 symlink traversal（GHSA-pxhw-h44j-8pfx）；当前 `MissingProtected` 会要求 bwrap 在 sandbox view 创建 mount point。维持 0.9.0 时必须在 L5 做定向安全复核，后续优先升级最低版本或消除受影响的创建路径。参考：https://github.com/containers/bubblewrap/security/advisories/GHSA-pxhw-h44j-8pfx |

已关闭项：OR-02 由 SR-07 的 `--args FD` 和 16 MiB payload 上限关闭；OR-05 由 SR-08 的 0.9.0 版本检查关闭。原 OR-04 经维护者取舍关闭为“不属于一期”：当前明确没有 seccomp，后续若引入必须单独定义 syscall policy、兼容矩阵和 fail-closed 测试，不能把文档记录误读成现有能力。

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
2. 安全 PATH 查找只接受绝对 PATH 项；拒绝空/相对项、workspace/cwd 及其子路径；候选必须是 root-owned 可执行普通文件，canonicalize 后固定绝对路径。本期按产品取舍只校验文件 owner，不递归校验父目录权限，完整路径链加固留待后续。
3. 对同一固定路径执行有界 `--version`、`--help` 参数检查和真实基线 probe。最低版本固定为 0.9.0；`--args` 属于必需参数，缺失时即使版本足够也 fail closed。
4. 成功缓存携带 path/version/identity/capabilities/expiry；执行前 identity 不一致或缓存过期必须重新 probe。失败不做进程生命周期永久缓存。
5. 第一版最低版本为 0.9.0，不依赖未使用的 `--argv0` 或 `--ro-bind-fd`；目标发行版仍须验证“版本 + 参数存在 + 真实 probe”，任一不满足都不可用。
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

1. 版本化、有长度/数量上限的 request；production outer/inner helper 都通过继承匿名文件 FD 读取 JSON，argv 只传固定短 FD 引用。隐藏入口暂保留有界 base64 形式供兼容与负向测试使用；拒绝未知版本、重复/非法 FD、NUL、非绝对 mount target 和超限 payload。
2. `future agent <hidden-helper-mode>` / `future-agent <hidden-helper-mode>` 在 singleton lock 之前分派，不读取模型配置、不启动 gRPC。
3. 外层固定 probe 凭据中的 bwrap，构造只读 root、writable roots、保护覆盖、fresh `/proc`、最小 `/dev`、user/PID/IPC namespace、cap drop、parent death；不 unshare network。
4. mount source 使用继承 FD 和 `/proc/self/fd/<n>`；完整 bwrap 参数通过 `--args FD` 传输；内层 helper 复核 dev/inode/type/目标身份及 effective/permitted capability 为零，设置 `PR_SET_NO_NEW_PRIVS`，再 exec 真实 shell argv。一期明确不安装 seccomp。
5. helper 作为 PID 1 时转发信号、回收后代并保留原始 exit/signal。Agent timeout/abort 后断言无残留后代。
6. `spawn_shell()` 调用 prepared backend；helper/probe/request/identity 失败作为 infrastructure error 返回，不进入 post-hoc escalation。

### Wave 4 — L3 完整规则与 violation/escalation（已完成，2026-09-03）

1. 启动前 glob 展开使用内部 no-follow walker；2026-09-04 起同根合并扫描，规则预编译并按匹配范围剪枝。取消节点数硬上限，采用整次扫描预算，详见“大仓库扫描修复”。暂不调用外部 `rg`，不引入额外 PATH 信任边界或两套 glob 语义。
2. 同时处理 lexical path 与 canonical target；对不存在 exact path 使用 MissingProtected，不直接打开或创建源。受保护 host 路径零占位对象是目标要求，但 bwrap mkdir 的父目录隔离尚未解决（2026-09-04 异常报告 P0），不能将参数构造视为该要求已实现。
3. 宽保护与窄重开按路径深度排列；read allow 只重开为只读，write allow 若会绕过仍生效的 read deny 则 typed fail closed；不存在的重开目标因无法无歧义创建 file/dir mount source 同样 typed fail closed，hard deny 不可重开。
4. helper 以稳定 marker 返回 violation kind/path provenance/policy digest/affected count，日志与普通 UI 不输出完整敏感路径集合。glob 新匹配和复扫失败都只做命令结束 detection-only；复扫失败保留已完成命令的原始 status，不称为动态硬保护。
5. 只有 Linux 文件系统拒绝可以触发一期整命令脱沙盒审批；detection-only marker、digest 不匹配、基础设施错误、普通 command error 和 2/125/126/127 不触发。

### Wave 5 — L4/L6 产品、诊断与文档（已完成，2026-09-03）

1. Desktop 平台 probe、Settings/Composer 和 remote bridge 已改用统一 `probe_sandbox` 结果；Linux probe 成功直接显示 sandbox，失败展示本地化稳定 code 与安装/排障命令。
2. 已保存 sandbox 后环境失效时，Agent 会把请求 tier 改为 manual，`ResolvedSandbox` 在执行边界再次强制同一回退；Desktop 仅在 definitive unavailable 时持久化 manual 并显示通知。瞬时 RPC 错误保留设置等待重试，任何路径都不会静默按 off 裸跑。
3. `future-agent --probe-sandbox`（以及 `future agent --probe-sandbox`）输出 machine-readable JSON；`future doctor` 使用同一 probe 并展示 backend/code/available、system bwrap 路径和版本。probe 只暴露 bwrap 可执行文件路径，不输出规则或受保护文件路径。
4. Settings/Composer 和中英文文档明确 network open、system bwrap only、no WSL、glob 启动时快照与结束后 detection-only；不随应用打包或下载 bwrap。详细步骤与 code 表见 [`LINUX_SANDBOX_USER_GUIDE.md`](LINUX_SANDBOX_USER_GUIDE.md)。

#### Linux 安装与诊断速查

- Ubuntu/Debian：`sudo apt install bubblewrap`
- Fedora：`sudo dnf install bubblewrap`
- 机器可读诊断：`future agent --probe-sandbox`；预期 JSON 至少包含 `available`、`backend`、`code`，Linux 成功时还包含 `path`、`version`、`capabilities`。
- 汇总诊断：`future doctor`；`binary_missing` 表示未找到可信 system bwrap，`path_rejected` 表示 PATH 候选不安全，`version_too_old` 表示版本低于 0.9.0，`required_feature_missing` 表示系统包缺少 `--args` 等必要参数，`user_namespace_disabled` / `proc_mount_restricted` 表示主机策略不允许生产基线，`probe_timeout` / `probe_failed` 表示探测未正常完成。
- WSL 不受支持；网络保持开放；glob 仅对命令启动时已有匹配提供硬保护，命令中新匹配仅在结束后报告 detection-only violation。

### Wave 6 — 本地总门禁与 L5 交付

1. 执行 §4 自动化和当前 Linux 主机 smoke，结果逐项写为 PASS/FAIL/NOT RUN/ENVIRONMENT LIMIT。
2. 已输出独立的 [`LINUX_SANDBOX_REAL_MACHINE_VALIDATION.md`](LINUX_SANDBOX_REAL_MACHINE_VALIDATION.md)，覆盖目标发行版、架构、userns/proc 负向环境以及 `.deb` 与 portable tarball 实际发布包；本期明确不发布 AppImage/rpm，不得用容器或当前 Ubuntu 26.04 结果替代真机结论。
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
| R-03 | rules/plan | symlink、missing exact、glob 快照/上限/异常 fail closed | 同上 | 历史 PASS（2026-09-03：旧逐 pattern walker）。2026-09-04 扫描实现与预算已替换，当前验证见“大仓库扫描修复”，不沿用旧 PASS 证明新 Linux 真机状态。 |
| R-04 | cross-platform | Seatbelt/Windows/manual/off 行为不回归 | `cargo test -p future-agent sandbox:: tools:: rpc::commands::settings` | PASS（2026-09-03：sandbox 115 tests；Agent clippy all tests） |
| H-01 | helper parser | version/size/count/FD/path 输入校验 | `cargo test -p future-agent sandbox::linux::request` | PASS（2026-09-03：version/path/NUL/phase/FD identity 与重复 FD；size/count 常量已强制，边界补测留 L3） |
| H-02 | helper boundary | helper 绕过 Agent singleton，但非法直接调用失败 | `cargo test -p future-agent --test linux_sandbox_smoke` | PASS（2026-09-03：非法 payload 返回 infrastructure exit 125，未创建 singleton lock） |
| H-03 | mount smoke | workspace/temp 写成功，workspace 外写失败且 host 无文件 | `cargo test -p future-agent --test linux_sandbox_smoke filesystem_ -- --ignored --test-threads=1` | PASS（2026-09-03：当前 Ubuntu 26.04/system bwrap；workspace 写成功、外部写拒绝、exit 23 原样） |
| H-04 | secret smoke | 已有 secret 精确/glob 文件读写均失败 | 同上 | PASS（2026-09-03：本机真实 bwrap；unreadable file 由 mode-000 opaque source 覆盖，读写均失败且 inner command 仅见 stdio FD） |
| H-05 | missing/symlink | missing exact 不能创建；symlink 不越界；host 无临时对象 | 同上 | **P0 BLOCKED BY DESIGN GAP**：helper 不直接创建 host placeholder，但 bwrap 自身 mkdir 仍可能修改宿主或因只读父目录失败；详见异常报告，待设计修复和 Linux 重跑 |
| H-06 | network | 未 unshare network；本地 TCP/namespace identity 验证网络保持开放 | 同上 | NOT RUN（argv 单测确认未使用 `--unshare-net`，仍需网络 smoke） |
| H-07 | lifecycle | 正常 exit/signal 原样；abort/timeout/parent death 无后代残留 | 同上 | PARTIAL PASS（2026-09-03：本机真实 bwrap 的 exit、原始 signal 与 parent-death 后代清理均 PASS；Agent timeout/abort 的 Linux 专项集成仍 NOT RUN） |
| H-08 | FD security | request/mount/status FD 只在对应 helper 阶段存活；用户命令仅见 stdio，Agent listener/db/log FD 不继承 | 同上 | NEEDS LINUX RE-RUN（旧 smoke 已证明 inner command 未见 fd > 2；新增 production request-FD 双阶段 smoke 尚未执行） |
| V-01 | violation | EACCES/EPERM/EROFS 结构化分类；普通失败和 2/126/127 不误判 | `cargo test -p future-agent sandbox::linux::violation sandbox::tests::linux_denial` | PASS（2026-09-03：可信 marker 优先、推断 provenance；2/125/126/127 与普通错误排除） |
| V-02 | escalation | Linux policy violation 可审批单次脱沙盒；infra failure 绝不 escalation | `cargo test -p future-agent tools:: rpc::approval` | PASS（2026-09-03：Linux classifier 接入既有整命令 post-hoc escalation；prepare/helper exit 125 不触发） |
| G-01 | glob | 启动前已有匹配被硬保护；命令中新匹配只报告 detection-only | ignored Linux smoke | PASS（2026-09-03：本机真实 bwrap；missing target 保持不可创建，命令中新建 glob 命中并输出 detection-only marker） |
| U-01 | RPC/Desktop | Linux availability/retry/reason/manual fallback | `cd desktop && npx vitest run src/integrations/agent/useSandboxAvailability.test.ts`；Rust bridge/settings tests | PASS / ENVIRONMENT LIMIT（2026-09-03：既有记录显示 Desktop availability 12 tests PASS；本轮 Agent workspace tests、Tauri 1095 tests 与 fmt/clippy PASS；当前非交互 PATH 无 `node`/`npm`/`npx`，按 bounded 策略未安装、未重跑 Vitest） |
| U-02 | i18n/UI | Settings/Composer 安装与限制文案中英文齐全 | `cd desktop && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run` | PASS / ENVIRONMENT LIMIT（2026-09-03：既有记录显示 TypeScript、ESLint、Vitest 69 files / 687 tests PASS；本轮环境无 `node`/`npm`/`npx`，未安装依赖、未重跑） |
| C-01 | CLI | machine-readable probe 与 doctor code 一致 | `cargo test -p future-agent --test cli_smoke && cargo test -p future-cli doctor` | PASS（2026-09-03：Agent machine-readable probe smoke PASS；future-cli doctor 20 tests PASS；本机实测 JSON 为 `linux_bubblewrap/user_namespace_disabled`） |
| Q-01 | Rust gate | workspace + Tauri fmt/clippy | `make lint-rust` | PASS（2026-09-03 本轮分项有界执行：`cargo fmt --all --check`、workspace `cargo clippy --workspace --all-targets -- -D warnings`、Tauri fmt/clippy 均 PASS；当前 PATH 无 Node，未重跑依赖 Node 解析 toolchain 的 Make wrapper） |
| Q-02 | all tests | Rust、Desktop、Mobile 全量单测 | `make test` | PARTIAL PASS / ENVIRONMENT LIMIT（2026-09-03 有界分项执行：`cargo test --workspace -- --test-threads=1` PASS；Tauri 1095 项首次 1094 PASS/1 个 remote runtime 时序测试 FAIL，单测定向重跑 PASS，判定为非本改动 flaky；当前 PATH 无 `node`/`npm`/`npx`，Desktop/Mobile Node 门禁未重跑且未安装。既有记录的 Desktop 687 与 Mobile 551 tests PASS 保留为历史证据） |
| L5-01 | real hosts | Ubuntu 22.04/24.04、Debian stable、Fedora；x86_64/aarch64 | [`LINUX_SANDBOX_REAL_MACHINE_VALIDATION.md`](LINUX_SANDBOX_REAL_MACHINE_VALIDATION.md) RH/SM 矩阵 | NOT RUN |
| L5-02 | packages | `.deb` 与 portable tarball 安装、`.deb` 升级/卸载及 system bwrap 引导 | 真机手册 PKG 矩阵 | NOT RUN（本期明确不发布 AppImage/rpm，不属于验收范围） |
| L5-03 | security review | TOCTOU/FD/setuid/namespace/cleanup/escalation/logging review | 真机手册 SEC-01～SEC-15 + reviewer sign-off | NOT RUN |

## 5. 提交与完成规则

- 每个 Wave 至少一个独立提交；提交说明只描述该 Wave，禁止顺手重构无关模块。
- 修改 proto 后运行 `make generate-proto` 并提交两个受影响的生成文件；若平台 probe 仍能使用现有 untyped command/JSON 响应，则不为未来可能性扩 proto。
- Tauri Rust 检查前运行 `make desktop-sidecar-placeholder`。
- 集成测试必须默认 ignored，并在缺 bwrap/不支持环境时给出明确 skip 或 stable probe code；单元测试不能依赖开发机 HOME、PATH 或真实用户规则。
- PR 前按仓库流程再次 fetch 并确认 `origin/main` 已是 `sandbox` 的祖先，执行完整门禁。不得用可能滞后的本地 `main` 代替远端基线。
- “代码完成”不等于“发布验证完成”。L5-01～03 未全部给出真实 PASS 前，交付状态只能是“可供用户真机实测”，不能宣称 Linux sandbox 已满足主干发布门槛。
