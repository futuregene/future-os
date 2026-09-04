# Linux：Bubblewrap 沙箱

更新：2026-09-04。L0–L4 与 L6 产品接入已实现；用户已反馈原生 Linux 能跑通，上一轮有 bwrap 0.11.1 的 7/7 smoke 记录。**最新私有报告/资源加固尚未原生复验，L5 发行版、架构、安装包与独立安全 review 仍未完成。** 本页统一设计、实现、安装、异常、验证与计划；公共规则和参考见 [COMMON.md](COMMON.md)。

## 1. 已确认范围

| 决策 | 当前契约 |
|---|---|
| L-D1 / D4 | Linux 唯一后端为系统 Bubblewrap；不 bundle、不下载、不自动换 Landlock |
| L-D2 / D11 | 网络开放；一期没有 seccomp filter，后续单独做纵深防御 |
| L-D3 | sandbox 开发分支直接接入产品，不另加隐藏开关；可用性与正式发布门槛分开 |
| L-D5 | 启动前既有 glob 匹配硬保护，命令内新匹配仅事后检测 |
| L-D6 | 一期整命令脱沙箱审批；二期 execution_grants 与 macOS 一起改造 |
| L-D7 / D10 | 安全 PATH、版本/参数、真实运行三层 probe；最低 bwrap **0.9.0** |
| L-D8 / D9 | 仅原生 Linux；不支持/不专门检测 WSL，不扫描额外兼容安装目录，不做 argv0 兼容分支 |
| 2026-09-04 补充 | 缺失 Ask/Deny（包括 approval_rule）、复杂 reopen、缺失 writable allow 不再作为本期必改项；详见 §4 |

0.9.0 是产品兼容下限，不承诺包含全部上游安全修复。只要求 bwrap 文件 root-owned，不递归审计父目录权限与模式位；这是维护者明确接受的阶段性取舍。若攻击者已可用 root 修改系统程序，本沙箱不再是其安全边界。普通用户可修改的 root-owned 路径链等进一步加固留后续，不能描述为已验证。

## 2. 安装、可用性与排障

通过发行版受信任渠道安装；安装到低于 0.9.0 时仍须升级：

```bash
# Ubuntu / Debian
sudo apt update && sudo apt install bubblewrap
# Fedora
sudo dnf install bubblewrap
future agent --probe-sandbox
future doctor
```

安装或修复后**完全退出并重启 FutureOS**。Linux Settings/Composer 保持选项布局稳定：检测中/不可用时禁用，成功才可选，不是失败时把选项删除。提示结构是“原因 + 方案 + code”，例如“系统未安装 Bubblewrap，请安装 Bubblewrap。(`binary_missing`)”；安装命令与重启提示可在设置页查看。普通卡片不额外堆叠网络说明。

probe CLI 不启动常驻 Agent，不受 singleton 锁阻塞；统一 `future agent` 与独立 `future-agent` 使用同一实现，doctor 消费同一结果。JSON 含 `available/backend/code`，成功时含 system path/version/capabilities；不输出完整敏感路径集合。

| code | 原因 / 处理 |
|---|---|
| `available` | 完整探测通过 |
| `binary_missing` | 未找到系统 bwrap；安装并确保可信绝对系统目录在 PATH |
| `path_rejected` | 相对/项目目录候选或非 root-owned；使用发行版系统包 |
| `binary_invalid` | 非可执行普通文件或身份读取不安全；修复系统包 |
| `version_unreadable` / `version_too_old` | 版本无法解析或低于 0.9.0；升级 |
| `required_feature_missing` | 缺 `--args` 等必需参数；升级完整系统包 |
| `user_namespace_disabled` | 主机/容器/安全策略禁止 userns；由管理员评估支持配置 |
| `proc_mount_restricted` | 无法建立 fresh `/proc`；检查主机策略 |
| `probe_timeout` / `probe_failed` | 探测超时或失败；查看 doctor/本机日志 |
| `binary_identity_changed` | 探测后文件替换，拒绝旧凭据并重新探测 |
| `probe_transport_error` | UI/Agent 连接问题，不当作永久不支持 |

probe 各子命令当前超时 1 秒，成功 receipt 缓存 300 秒并核验二进制 identity；失败不永久缓存。忽略空/相对 PATH 与 workspace/cwd 内候选，canonicalize 后固定同一绝对路径执行 version/help/runtime。

runtime probe 运行只读 root、user/PID/IPC namespace、cap-drop、最小 dev/proc 和 mode-0700 `/tmp` tmpfs，验证临时写入与只读 root。**它不是完整真实 workspace/HOME 生产计划的预演**，probe 成功不能保证每条规则组合、资源状态、cwd 都可启动。

基础 probe 明确不可用时 Agent/解析层转 manual，Desktop 持久化回退并显示原因；瞬时连接失败保留设置。运行期 plan/helper 错误返回工具失败，既不自动降级也不裸跑。模型可显式申请整命令脱沙箱，用户批准才执行；对话本身不必停止。避免对可能已有副作用的命令盲目重放。

## 3. 实现原理与代码地图

```text
RuleSet 快照 → LinuxSandboxPlan → PreparedShell + request FD
  → 当前 future/future-agent 自重入 outer helper
  → 固定 bwrap executable FD + --args OPTIONS FD + 短 COMMAND argv
  → inner helper：mount/capability 复核、no_new_privs、FD 收敛
  → shell / 后代 → status pipe → outer 复扫 → 私有报告 → Agent 工具输出
```

隐藏 helper 在正常 runtime/singleton 前分派，不加载模型、不启动 RPC。Desktop 仍只打包统一 `future` sidecar，不增加第二 helper 制品；helper 使用显式子命令而非 argv0 分派。

| 模块（`agent/src/sandbox/` 下） | 职责 |
|---|---|
| `backend.rs`、`linux/runner.rs` | PreparedShell、自重入参数、request/report 文件与 spawn |
| `linux/probe.rs` | system binary、版本/参数/runtime、receipt/cache |
| `linux/plan.rs` | 规则快照、writable/read-only/unreadable/reopen、missing 省略、digest |
| `linux/glob_scan.rs` | 同根合并、有界 no-follow 扫描，pre/post 共用 |
| `linux/request.rs` | 版本化请求、大小/FD/path/phase 校验 |
| `linux/helper.rs` | mount FD、bwrap、内层复核、status、信号/后代、复扫 |
| `linux/post_scan.rs` | 缺失目标逐项检测、失败/未检查统计 |
| `linux/report.rs`、`linux/violation.rs` | 私有报告认证、英文检测说明、非权威输出清洗、拒绝启发式 |

### 3.1 Mount 与进程边界

生产使用 `--new-session --die-with-parent --unshare-user --unshare-pid --unshare-ipc --cap-drop ALL`；先 `--ro-bind / /`，再开放 workspace/temp/allow-write 根，叠加保护及受支持的窄重开。`--dev /dev`、fresh `/proc`；不加 `--unshare-net`。

写保护通常 read-only bind；读保护使用 mode-000 opaque 源遮罩，不能以“读到空文件成功”冒充拒绝。同一路径 read+write 保护只保留更强 opaque mask，不再重复挂载。缺失保护目标完全省略 mount，不造宿主占位对象。`[]`/`{}` 等不支持matcher明确报错；只读重开保持写deny，若write allow会绕过仍有效的read deny则拒绝编译，不静默放宽。

mount source 以 O_PATH FD 固定，bwrap 通过 `/proc/self/fd/N` 挂载；内层核对 dev/inode 与类型/权限。目录 size/mtime 会因正常宿主写入改变，不作为普通 mount 身份；bwrap executable 仍做严格 receipt 校验并经固定 FD 执行。路径检查只有真实 NotFound 是缺失，权限错误、ENOTDIR、dangling symlink 不当成空路径忽略。对可达性/重叠组合不作无条件支持承诺。

内层 `capget` 确认 effective/permitted capability 均为零，再设 `PR_SET_NO_NEW_PRIVS`，关闭不需要的 FD 后启动 shell。Agent listener、日志、数据库等不应继承给命令。PID namespace 内 helper 转发信号、回收后代；status pipe 传原始 wait status。正常完成、信号、timeout/abort、父死必须协同，不保留 detached 后代。

### 3.2 ARG_MAX、payload 与资源预算

| 资源 | 当前上限 / 语义 |
|---|---|
| helper JSON request | v3，8 MiB；生产 outer/inner 都用匿名文件 FD，argv 为短 `fd:3` 引用 |
| mount / shell argv | 16,384 mounts；shell argv 合计 96 KiB，仍可能受系统环境大小限制 |
| bwrap OPTIONS 文件 | NUL 分隔，16 MiB；连同真实 argv 合计最多 9000 参数 |
| FD | 打开 mount 前读取 `/proc/self/fd` 与 RLIMIT_NOFILE，内部保留 16 个位置 |
| report | 64 KiB、version 1、最多 4 个匹配 digest 的 detection-only 事件 |

`--args FD` 解决大量 mount 参数占用 execve ARG_MAX，但 **COMMAND 不能放参数文件**：bwrap 的递归 OPTIONS 解析在 `--` 停止，不会把文件尾 COMMAND 交回外层。真实 argv 必须保留 `-- current_exe helper-args`。用户命令/环境、FD、临时磁盘、内核 mount 仍有资源限制；预检不能消除并发竞争，ENOSPC/EMFILE/spawn 失败仍返回错误，不放宽沙盒。

请求拒绝未知版本、非法/重复/阶段不符 FD、非绝对目标、NUL、超限 payload；有界 base64 路径仅供直接调用/负向测试，不是生产大 payload 通道。旧 `MissingProtected` wire variant 保留但两阶段明确拒绝，不再发射 tmpfs missing mount。

### 3.3 大仓库扫描

相同静态根只扫描一次，预编译 pattern，按前缀与最大匹配深度剪枝：`.env.*` 只看根层；有限 `pkg-*/secrets/*.key` 不全仓递归；`**` 才允许需要的递归。不跳过隐藏、gitignored、`node_modules`、`target`、`.git`，匹配目录也保留；目录 symlink 不下探，匹配链接保留 lexical 与 canonical target。

取消旧 100,000 节点硬限制；节点数仅统计。当前共享预算：30 秒、256 唯一 pattern、2048 唯一路径、4 MiB 结果关联字节估算、64 层。pre-launch 和 post-command 分别计时；post 的 glob 与缺失目标检查共用预算。启动前在 `spawn_blocking` 做扫描并响应 Abort，完成后主任务再次确认取消，不让扫描 worker 自行启动命令。

30 秒为协作式预算，不可抢占卡死的文件系统 syscall。错误带 phase/root/pattern 与 visited/matches/elapsed/limit；code 包括 `glob_scan_timeout`、`glob_scan_match_limit`、`glob_scan_result_bytes_limit`、`glob_scan_pattern_limit`、`glob_scan_depth_limit`、`glob_scan_io_error`、`glob_scan_cancelled`、`glob_scan_pattern_invalid`。启动前失败不执行；结束后失败只报告不完整，不改变已完成状态。

### 3.4 检测报告与重试可信度

`omitted_missing_protected_paths` 表示：**启动时不存在，所以没有安装保护 mount；结束后只检查是否出现，不阻止创建，也不撤销修改。** 它不等于没有规则或已获得 allow。

| 事件 | `affectedCount` 含义 |
|---|---|
| `missing_protected_created` | 原来缺失、现在存在的目标数；创建者未知 |
| `missing_protected_scan_failed` | 检查失败或未检查的目标数，**不是违规次数** |
| `dynamic_glob_created` | 新的敏感规则匹配数，多条规则可能命中同一路径 |
| `dynamic_glob_scan_failed` | 检测不完整，0 不表示无违规 |

`message` 由后端按 kind/count 生成英文说明，明确“detection only / creation was not blocked / no changes were undone / does not authorize a retry”。收到的 message 不参与可信判定，旧记录缺 message 仍可解析。信息附在 shell 工具输出中给用户和模型看，不新增弹窗/通知。

生产每次 spawn 创建独立匿名 report 文件，writer 只给 outer helper；outer 从 inner request 移除 `report_fd`，设 CLOEXEC 且不加入 bwrap keep-list。命令与后代不能继承 writer。命令完成、后代回收、复扫后 outer 才写完整报告，Agent reader 校验长度/version/digest/事件类型，不信任 stdout marker。

命令自己打印 `__FUTURE_SANDBOX_VIOLATION__:` 被标成 `untrusted command text; not a sandbox report`；可信事件在命令输出截断后附加。直接 helper 调试仍可换行输出 marker，但不构成来源认证。存在 detection-only 事件、私有报告缺失/损坏/超限/不匹配时，禁止**被动**脱沙箱重试，保留原 exit 并提示检测未知；不阻止模型另行显式申请审批。

有效空报告之后，普通失败仍使用 `Permission denied`/`Operation not permitted`/`Read-only file system` 文本启发式，排除 0、2、125、126、127。它不是 errno 认证：程序可以伪造普通错误文本；最终仍须用户批准。私有通道不防御已控制宿主同用户 Agent 的攻击者。

## 4. 平台差异与已接受缺口

以下取舍已经确认，不再保留旧稿“missing target 必须硬保护否则阻断本期”的结论：

| 情况 | 当前行为 / 风险 |
|---|---|
| 已存在规则文件、models、HOME/workspace secret | 按读写规则安装保护；不把缺失策略扩大成现存文件 allow |
| 缺失 `approval_rule.json`、自定义 Deny、`.ssh`/`.env`/models 等 | 不挂载，仅结束检查；若父目录可写，可能创建并读写。**Linux 不承诺缺失 Deny 的创建拦截**；规则文件创建还可能影响下一轮规则，接受风险不等于无安全风险 |
| 缺失目标位于只读域 | 命令通常不能自行创建，但宿主其他进程并发创建后，不保证动态拒读 |
| 可写域中新 glob | 命令内创建、读取、使用不会被动态拦截；下一条命令重新扫描已有对象 |
| 宽 deny/read-only + 窄 allow、重叠 mask | 可能因 mode-000 祖先不可遍历或最终 mount view/identity 冲突而失败；不保证完整 first-match 等价 |
| 缺失 allow-write/reopen 根、失效 cwd | 可能无法打开 mount source/chdir，拒绝该次启动；不自动创建源或扩大父目录授权 |

例如高优先级 allow `private/output` + 较宽 deny `private`，不能只按 mount 深度排序就宣称可重开；外部 allow 根尚不存在时也不能保证 `pwd` 启动。这两类暂不改，用户可调整规则或显式申请整命令审批。

扫描是快照不是审计：“创建→使用→删除”会漏报，其他宿主进程创建可能被检测为出现但无法归因。单项 metadata 失败继续检查后续目标，另报不完整；取消/预算记录未检查数。outer 持续记录 TERM/INT/HUP/QUIT，但 SIGKILL、崩溃、缺 status 或报告写入失败仍可能没有完整结果，**无报告不等于无违规**。

不采用宿主 synthetic placeholder + cleanup。bwrap 的 tmpfs setup 先 ensure_dir/mkdir：只读父目录 EROFS，可写父目录会真的创建宿主对象，mount namespace 本身不隔离这种写入。若将来要求未来名称硬保护，需独立设计父目录隔离视图/broker/FUSE 等并保持写回语义；不能简单 tmpfs 遮父目录导致输出丢失。Codex 对照见 COMMON §6。

## 5. 现场故障与修复沿革

| 现场 / 审查编号 | 根因及当前修复 |
|---|---|
| `.env.*: sandbox glob scan limit exceeded`（`d469719b`） | 旧逐规则全仓扫描误触十万节点上限；同根分组、剪枝、共享预算，§3.3 |
| `pwd; whoami` exit 125，`.aws mount source is unavailable`（`d51963c9`） | 缺失 guard 同时进普通 bind 与 missing 列表；分类去重、严格 NotFound、普通 mount 不比较目录 mtime/size |
| `Can't mkdir ... .aws: Read-only file system` | missing tmpfs target 会 ensure_dir；改为省略+检测，不造宿主目标 |
| bwrap usage / 未报告 command status（`01b7e413`） | OPTIONS 文件吞 COMMAND；COMMAND 回真实 argv；补真实默认规则生产全链路 smoke |
| SR-01 / SR-02 | root-owner 校验及取舍注释；跨层/同层按 first-match 编译，避免后规则重复生效 |
| SR-03 | 旧“namespace-only 创建”假设已撤销；当前省略，legacy wire mount 拒绝 |
| SR-04 / SR-05 | 原 stdout marker 信任方案被私有报告替代；复扫失败 detection-only、保留 exit，避免副作用后二次执行 |
| SR-06 / SR-07 | helper JSON 和 bwrap OPTIONS 双 FD transport；后补9000参数与RLIMIT预算 |
| SR-08 / SR-09 / SR-10 | 0.9.0、capget/no_new_privs、无 seccomp；UI 稳定布局/禁用项/原因方案code/重启 |

bwrap/inner 没有 completion 且非信号终止时 helper 返回 infrastructure 125，stderr 合并捕获。**用户命令自己也能 exit 125，缺 completion 不能证明完全没执行**；不得仅看退出码自动重放。

## 6. 历史证据与开发进度

- 初始分支基线 `fd3e1771`，Linux 实现来自 `claude/linux-bwrap-sandbox`；2026-09-03 曾合并 `origin/main@15d7df79`（`0867b0fd`）。这些是历史定位，不是当前最新 main 声明。
- 2026-09-03 Ubuntu 26.04/Linux 7.0/x86_64、`/usr/bin/bwrap` 0.11.1，初始 5/5 ignored smoke、基础 probe PASS；不是目标发行版完整认证。
- 同日历史跨平台记录：sandbox 115 tests、Rust workspace 测试通过；Tauri 1095 项首次1个 remote runtime 时序失败、定向重跑通过。旧 Desktop 687/Mobile 551、availability 12 的 PASS 保留为历史；部分后续主机缺 Node 未重跑，不能合称同一候选全绿。
- 2026-09-04 大仓库 macOS 夹具：Linux 模块40 PASS/1 ignored；显式大夹具1 PASS，100,013项first/repeat约777/783ms，单 `.env.*` 访问3项。未控OS缓存，不是Linux冷缓存/bwrap性能。
- 第一轮 `.aws` 修复 macOS：45 PASS/1 ignored；Linux-only/helper未跑。
- `01b7e413` 第二轮提交者记录：Linux53 PASS/1 ignored、7/7 bwrap smoke、Linux Clippy/fmt通过。全Agent1651 PASS/2 FAIL（`models::future::cache_save_and_concurrent_load_never_torn`、`models::tests::registry_injects_future_models_from_disk_cache`）；独立重跑通过，记录为共享缓存并行疑似flaky，**不是全量首次全绿**。
- 最新明确项/私有报告加固：macOS fmt、Agent all-targets Clippy、diff check通过；新增测试未执行。Linux cross-check 因缺 `x86_64-linux-gnu-gcc` 在 ring 构建受阻，不能声称Linux helper编译通过。用户反馈真机跑通不覆盖最新所有异常分支。

| 阶段 | 当前状态 |
|---|---|
| L0 probe / L1 plan与接缝 / L2 helper | 已实现；真实生产计划与边界仍需目标主机复验 |
| L3 规则、扫描、escalation | 已实现有界版本，不是完整动态路径等价 |
| L4 打包诊断 / L6 产品入口 | 本地接入完成；system-only、统一probe/doctor、Desktop中英文 |
| L5 发布验证 | 原生发行版/架构、制品与独立安全review待完成；不因开发分支入口可选而豁免 |

## 7. 真机验收操作与矩阵

在候选仓库根目录、原生Linux普通环境执行（VM可，容器/WSL不能替代）：

```bash
./scripts/test-linux-sandbox-real-machine.sh
# 可选完整 Rust workspace 测试
./scripts/test-linux-sandbox-real-machine.sh --full
# GUI 开发启动
./scripts/start-desktop-linux.sh
```

脚本收集环境、构建probe、Linux单测、大于十万项夹具、stderr捕获回归、所有ignored smoke、fmt/clippy，输出 `linux-sandbox-evidence-*.tar.gz`。不安装软件/改变系统策略；出现 `skipping Linux sandbox smoke` 必须失败。**不要固定写“7个测试”**，以候选源码实际套件为准。

GUI开发启动脚本不是只读probe：它会构建/启动独立Agent并运行Tauri，默认清理stale runs/approvals，退出回收自己启动的Agent。需要保留这些记录时设置`CLEAN_STALE_APP_TASKS=0`；`DRY_RUN=1`仅诊断。还可设置`REUSE_AGENT`、`BUILD_AGENT`、`BUILD_CLI`、`RUN_CHECKS`、`FUTURE_AGENT_GRPC_ADDR`、`DESKTOP_DEV_PORT`。bwrap不可用不阻止Desktop启动，便于验证不可用提示；权威安全检查仍由Agent负责。

独立执行：

```bash
cargo test -p future-agent --test linux_sandbox_smoke -- --ignored --test-threads=1 --nocapture
cargo test -p future-agent --lib sandbox::linux::glob_scan::tests::large_workspace_exceeds_old_node_limit_without_failing -- --ignored --test-threads=1 --nocapture
```

重要smoke：真实默认HOME规则生产链 `production_plan_with_real_default_rules_starts_a_shell`；no_new_privs/exit、unreadable/FD、原signal、parent-death、production request FD、missing创建检测、glob复扫失败；最新 `missing_scan_reports_partial_failure_after_unterminated_command_output` 与 `command_cannot_write_or_forge_private_helper_report` 必须执行。无换行输出不能吞诊断，伪造stdout不能污染私有报告，partial扫描失败后仍发现其他目标，exit23不变。

证据模板：Tester、UTC日期、Host ID、原生/VM、发行版/内核/架构/glibc、desktop session、candidate commit+dirty diff、应用版本、制品SHA256、bwrap path/version/package version、日志目录、reviewer。采集 `git rev-parse HEAD`、`git status --short`、`cat /etc/os-release`、`uname -a`、`bwrap --version`、probe、doctor；不上传凭据/完整规则/敏感路径清单。

状态只用 PASS/FAIL/NOT RUN/ENVIRONMENT LIMIT；PASS必须有对应机器/提交日志。以下目标矩阵仍待逐行执行；不把版本过旧的正确拒绝记为正常可用PASS：

| ID | 原生主机 | 架构 / 预期 |
|---|---|---|
| RH-01 | Ubuntu22.04 | x86_64；官方包若<0.9.0验证version_too_old，正常运行需受信任升级 |
| RH-02 / RH-03 / RH-04 | Ubuntu24.04 / Debian stable / Fedora支持版 | x86_64，记录具体版本 |
| RH-05 | Ubuntu24.04 | aarch64 |
| RH-06 | Debian stable或Fedora | aarch64，再覆盖一种发行版 |

安装包PKG-01～04：两架构 `.deb` 全新安装、原位升级、卸载，以及portable tarball。记录实际制品；GUI/sidecar自重入正常、配置会话保留、不含bwrap、卸载不删system bwrap。按制品README安装；仅对选定测试包执行 `sudo apt install ./FutureOS_<version>_<arch>.deb` 与后续 `sudo apt remove futureos`。本期不发布AppImage/rpm。

每台正常主机SM-01～06：全smoke无skip、probe/doctor一致、workspace写成功/外部写拒绝、infra及检测事件不被动重试、测试者本地HTTP服务可访问、Settings/Composer与manual回退正确。另查主动/被动标题、具体路径仅诊断、拒绝不执行、一次批准不改全局；大仓库Desktop反复pwd记录first/repeat耗时（未控缓存不叫cold/warm），隐藏/ignored secret仍保护。

| 负向ID | 专用VM/fixture场景 | 断言 |
|---|---|---|
| NEG-01 /02 | 缺bwrap、相对/项目假binary、非root owner | 不执行假binary；可继续找可信系统候选，否则稳定失败 |
| NEG-03 /04 | userns禁用、fresh proc受限 | 稳定code、不可用，不以较弱后端替代 |
| NEG-05 /06 | 版本旧/输出坏/缺参数、超时 | version/feature/timeout code；可修复后重探测 |
| NEG-07 | probe后替换binary inode | 旧receipt不执行 |
| NEG-08 | WSL | 仅记不支持范围，不要求不存在的WSL检测code，不计入原生认证 |

不得在日常主机绕过组织安全策略；无法安全构造场景记ENVIRONMENT LIMIT并换合适VM，不删失败行。

### 7.1 独立安全 review（SEC-01～15）

review实际候选diff，逐项给PASS/FAIL与源码/日志；仅运行测试不能代替：

1. mount lexical/canonical/symlink/FD identity与替换竞态。
2. request/mount/status/report FD阶段隔离，用户命令不继承Agent资源。
3. root-owned/setuid bwrap、capget零值、no_new_privs。
4. user/PID/IPC、dev/proc、网络开放契约。
5. outer/bwrap/inner/后代在等待、执行、复扫阶段的signal/abort/timeout/父死。
6. missing目标**不挂载、不创建宿主占位**；出现检测不作创建拦截保证。
7. 整体坏规则、unsupported matcher、扫描预算/I/O/cancel启动前失败；保留个别非法条目解析差异。
8. deny/reopen最终可达性，不把已接受复杂组合失败说成完整等价。
9. probe receipt与执行path/version/identity/expiry一致。
10. 私有报告认证、stdout不可信、检测/无效报告不被动重试；普通文本误判负向语料。
11. UI/日志/RPC脱敏，必要诊断可定位不泄secret。
12. helper版本/大小/数量/FD/NUL/path验证与singleton前入口。
13. 明确当前无seccomp，不宣称syscall或网络过滤。
14. --args OPTIONS/真实COMMAND分离，9000/16MiB/FD预算及ENOSPC/EMFILE。
15. GHSA-pxhw-h44j-8pfx相关setup路径复核；删除MissingProtected不等于所有symlink风险已免疫。

sign-off记录Reviewer/UTC/Candidate/Decision/Blocking issues/Follow-ups/Evidence。L5-01目标主机、L5-02实际包、L5-03review都完成且无阻断项后才声明发布达标；已接受范围缺口按§4记录，不伪造“已拦截”的PASS。

## 8. 后续优先级

| 优先级 | 工作 |
|---|---|
| P0 发布证据 | 最新Linux-only代码编译/全smoke、大仓库默认规则、RH/PKG/SEC矩阵；旧OR-01/OR-06 |
| P1 | 普通stderr启发式误报与重放副作用（OR-03）；二期execution_grants；bwrap旧版本安全复核（OR-08） |
| P2 | seccomp纵深防御：先冻结syscall policy、ptrace/process_vm/io_uring兼容与失败语义，再实现；不是一期已启用 |
| P2 | 诊断包、扫描预算与FD性能校准、helper/cwd可达性提示；凭据通道见COMMON |
| P3 / 已接受 | root owner之外路径链加固（OR-07）；缺失Deny、复杂reopen、缺失allow保持现状，不擅自升级本期范围 |

OR-02（mount argv的ARG_MAX）以双FDtransport关闭但保留资源上限；OR-05最低版本已收口；OR-04 seccomp转为后续。任何“所有环境都能启动”的承诺均不成立：无法安全准备该次命令时应明确失败，继续对话/显式审批，不静默降低保护。
