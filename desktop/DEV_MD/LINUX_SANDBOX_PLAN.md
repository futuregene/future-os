# FutureOS Linux 沙盒调研与开发计划

状态：**方案已完成第一轮决策收口；开发分支已完成 L0–L4 及 L6 本地接入：system bwrap probe、执行链与规则强制、结构化 violation/escalation、平台统一 availability、Settings/Composer、doctor 和显式 manual 回退。当前受控执行环境禁用了 user namespace，因此真 bwrap smoke 记录为环境限制；L5 多发行版/架构/安装包真机矩阵与安全 review 尚未完成，本分支仅达到“可供真机实测”，不代表主干发布门槛已满足**（2026-09-03）。

本文是 [`SANDBOX_PLAN.md`](SANDBOX_PLAN.md) 的 Linux 专项设计稿。审批规则语义仍以 [`APPROVAL_PLAN.md`](APPROVAL_PLAN.md) 为准；本文只讨论如何把现有 `RuleSet` 强制到 Linux shell 子进程，以及产品启用前需要补齐的证据。代码落点、实施波次与逐项验收命令见 [`LINUX_SANDBOX_IMPLEMENTATION.md`](LINUX_SANDBOX_IMPLEMENTATION.md)，用户安装、稳定诊断码和能力限制见 [`LINUX_SANDBOX_USER_GUIDE.md`](LINUX_SANDBOX_USER_GUIDE.md)。

### 已确认决策（2026-09-02）

| # | 决策 | 直接影响 |
|---|---|---|
| L-D1 | **Bubblewrap 是 Linux 唯一 OS 沙盒方案。** 不实现或自动回退到 Landlock 等第二后端 | bwrap 不可用时 sandbox probe 失败；不以较弱后端冒充同一档位 |
| L-D2 | **网络保持开放。** Linux 与当前 macOS/Windows 产品语义一致 | 不使用 `--unshare-net`，本期不引入 managed proxy/域名审批 |
| L-D3 | **在独立 `sandbox` 开发分支中可以直接显示 Linux“沙箱保护”入口。** 测试与 review 完成前不合入主干 | 不再为开发分支增加独立隐藏开关；分支内仍必须按 probe fail closed，主干合入门槛保持不变 |
| L-D4 | **只使用 system bwrap，不随 FutureOS 打包 bundled bwrap。** | probe 失败或未安装时不能启用沙箱；UI 提供安装提示，官网提供各支持发行版的详细安装/排障教程 |
| L-D5 | **接受 glob 的有界保证。** | 每条 shell 命令启动前保护已有匹配；同一命令中新生成的 glob 匹配不宣称动态硬保护，下一条命令重新扫描后生效 |
| L-D6 | **一期沿用 macOS 的整命令脱沙盒 escalation；路径级能力放二期，并与 macOS 一起改造。** | Linux 一期先对齐当前交互；二期建立跨平台 `execution_grants`，不在一期混入新审批协议 |
| L-D7 | **前置检测分三层：安全 PATH 查找、版本/必需参数检查、真实运行探测。** | 任一层失败都 fail closed，并向 UI 返回稳定原因 code |
| L-D8 | **不支持 WSL，不实现 WSL1/WSL2 检测或专用分支。** | 官网支持范围和测试矩阵只覆盖原生 Linux |
| L-D9 | **不增加兼容搜索或 `argv0` 兼容分支。** 只从安全 PATH 选择 system bwrap；FutureOS helper 使用明确子命令，不依赖 `--argv0` | 不扫描额外安装目录，也不为旧包维护另一套 helper 启动链；FD mount identity 复核仍是主路径安全措施 |

剩余工程参数只有 system bwrap 的最低版本和必需参数集合；它们在 L0 根据目标发行版系统包与最终实现实际使用的参数收口，不再作为产品方向问题反复讨论。

## 1. 调研范围与当前事实

初始调研阶段只补文档；当前开发分支已经修改 Agent 运行时代码，并在本机 system bwrap 上运行 L2 integration smoke。该结果仍不替代 L5 目标发行版、架构和安装包真机矩阵。

代码快照：

- FutureOS：当前工作区代码（2026-09-02）。
- Codex：`~/workspace/codex`，commit `f20b63e85c`。

FutureOS 当前实现：

- `agent/src/sandbox/mod.rs` 的平台统一 product probe 让 macOS Seatbelt、Windows RestrictedToken 与 Linux Bubblewrap 仅在各自完整 host probe 通过时返回可用，并提供稳定 backend/code；Linux 不再固定为 `false`。
- 开发分支的 Linux 会话在 typed probe 成功时选择 Bubblewrap backend；probe 不可用时 Agent policy 设置、执行时 backend 解析和 Desktop 持久设置三处都会显式回退到手动审批档，UI 同时显示诊断码，绝不静默裸跑。read/write/edit 的进程内路径规则继续生效。
- shell OS 包装已引入平台中立 `PreparedShell`，Linux 走“规则快照 -> `LinuxSandboxPlan` -> 自重入 helper -> Bubblewrap -> structured violation”链路；Windows 仍在 `tools::spawn_shell()` 走专用 restricted runner。
- 当前网络策略明确为完全开放；Linux 第一版不应偷偷改变为断网。
- 内置 workspace secret 包含 `.env`、`.env.*`、`**/*.pem`、`**/*.key`、`**/*.p12`、`**/id_rsa*`。这些 glob 是 Linux 设计最难与 Seatbelt 动态匹配完全等价的部分。

现有文档曾把 Linux 简化为“写侧 bind 白名单、读侧用 tmpfs/ro-bind 遮盖”。这不足以作为开发规格：遮盖不一定产生拒绝错误，可能让读取看起来像空文件/空目录；不存在路径、可写 symlink、窄规则重新放开、进程生命周期和 bwrap 能力探测也都需要单独设计。

## 2. Codex 当前 Linux 实现可借鉴的部分

Codex 不是把 macOS profile 原样翻译到 Linux，而是共享 `PermissionProfile`，在执行边界选择原生后端：macOS Seatbelt、Linux bubblewrap + 进程内 hardening、Windows 原生 runner。

Linux 主链路如下：

```text
PermissionProfile
  -> SandboxManager 生成 Linux helper argv
  -> helper 外层构造 bubblewrap 文件系统/namespace
  -> helper 内层再施加 no_new_privs / seccomp
  -> exec 用户命令
```

值得复用的设计原则：

| Codex 能力 | 价值 | FutureOS 建议 |
|---|---|---|
| 系统 `bwrap` 优先、bundled bwrap 兜底 | Codex 兼顾发行版版本与开箱可用的选择 | FutureOS 只采用前半段：固定并 probe system bwrap；缺失时引导安装，不提供 bundled fallback |
| 真正执行 user namespace probe，而非只看二进制是否存在 | 能识别禁用 unprivileged userns、受限容器等“装了但跑不了”环境 | P0，探测失败时隐藏产品入口并返回稳定 code；FutureOS 不增加 WSL 判断 |
| 根文件系统默认只读，再叠加 writable roots 和更窄的保护 mount | 与 FutureOS“读默认开、写限 workspace/temp”基本同构 | P0 |
| `--unshare-user/pid/ipc`、`--cap-drop ALL`、fresh `/proc`、`--die-with-parent` | 降低跨进程观察、残留进程和 capability 风险 | P0；fresh `/proc` 需有受限容器兼容探测 |
| bwrap 外层之后再施加 `PR_SET_NO_NEW_PRIVS` | 兼容依赖 setuid 的系统 bwrap，同时阻止 sandbox 内提权 | P0 |
| symlink/missing-path fail-closed、FD-backed mount、内层身份复核 | 避免 mount 计划到执行之间的 TOCTOU 与软链接逃逸 | P0，不能只 canonicalize 一次就认为安全 |
| glob 启动前展开；`rg --files --hidden --no-ignore` 优先，内部 walker 兜底；异常 fail closed | 能把已有敏感文件转成具体 mount | P0，但必须向产品声明它是启动时快照 |
| 窄 writable child 可重新打开宽 read-only/deny parent | 保留“高优先级窄规则胜出”的表达能力 | P1，纯计划单测必须先覆盖 |
| helper 作为 PID 1 转发信号、回收子进程并保留原始 exit 语义 | abort/timeout 不遗留孙进程 | P0 |
| structured violation、doctor/capability diagnostics | UI、日志和 escalation 不靠一条模糊 stderr 猜测 | P1 |
| managed network proxy | 可做域名/代理级网络策略 | 本期不做；FutureOS 已决定网络开放，不能顺手改变产品语义 |
| legacy Landlock 显式开关，复杂 split policy 不自动降级 | 避免不支持 deny-read/窄重开时静默弱化 | 不把 Landlock 作为自动 fallback；后端不可用就保持 manual，不伪装 sandbox |

不建议直接复制 Codex crate。它的 `PermissionProfile`、multitool arg0、managed proxy、受保护 `.git/.codex` 语义和 FutureOS 当前 `RuleSet`/审批协议并不相同；直接搬运会把未决定的产品语义一起带进来。应借鉴边界、探测、测试和 fail-closed 方式，在 FutureOS 内做更小的 Linux 后端。

## 3. 目标能力与 macOS 对齐边界

第一版目标是让 Linux 也能显示“沙箱保护”，保持现有三档协议和 UI 心智：

| 能力 | macOS 当前 | Linux 目标 | 第一版是否可等价 |
|---|---|---|---|
| 根文件系统读取 | 默认允许 | `--ro-bind / /` | 是 |
| workspace/temp 写入 | 允许 | 对规范化根做 `--bind` | 是，需处理 symlink |
| workspace 外 allow-write | profile allow | 额外 writable bind | 是，目标必须已安全解析 |
| ask/deny 的既有文件写入 | SBPL 动态拒绝 | read-only bind 覆盖 | 基本等价 |
| ask/deny 的既有文件/目录读取 | SBPL 返回拒绝 | 不可读 inode/mount 覆盖 | 可做；不能用“空内容遮盖成功”冒充拒绝 |
| 不存在的精确受保护路径 | 路径一旦出现即命中 | 创建仅存在于 sandbox view 的不可读/只读 mount target | 可做，但需证明不污染 host 且无竞态 |
| glob 已有匹配 | SBPL 动态匹配 | 启动前展开为具体路径 | 启动时等价 |
| glob 的运行中新匹配 | 自动命中 | 普通 bwrap mount 无法感知新路径 | **不等价** |
| 网络 | 开放 | 保留 host network namespace | 是 |
| 越界审批 | 整命令脱 Seatbelt 重跑一次 | 一期整命令脱 bwrap 重跑；二期改路径级临时能力 | 一期行为对齐；二期与 macOS 一起收窄权限 |
| abort/timeout | 进程组 kill | PID namespace + 信号转发 + 父进程死亡联动 | 可达到或更强 |

### 3.1 必须公开的 glob 限制

bubblewrap 的 mount view 在命令启动时确定。对 `**/*.pem` 这类规则，可以遮住启动前已经存在的匹配，但命令若在可写目录中新建 `new.pem`，内核 mount 规则不会按 glob 再匹配它。Landlock 也是 allowlist 模型，不能可靠表达“父目录可写，但其中未来匹配某个名称的子项不可读写，再允许更窄子树”这一完整层级语义。

本项目已选择路线 1；其余路线仅保留为取舍记录：

1. **已确认：有界等价。** 精确路径和启动时已有 glob 匹配由 OS 强制；运行中新产生的 glob 匹配列为 Linux 已知限制。内置 workspace secret 在 shell 执行前完整扫描，命令结束后若发现新增敏感文件则记录 policy violation，但这只是检测/减损，不宣称消除竞态。
2. **严格等价后再发布。** 引入能够逐次拦截文件操作的 broker（如 seccomp user notification + 安全路径解析、FUSE view，或受信任的一次性 filesystem worker），解决动态名称匹配后才显示 Linux 沙箱档。安全性更强，但实现和兼容成本明显更高。
3. **拒绝含 glob 的 sandbox shell。** 最保守但会因内置 secret glob 让正常开发频繁不可用，不推荐。

路线 1 的产品说明必须区分“命令启动时硬保护”和“命令内新匹配检测”；不能把检测器写成“动态硬保证”。若未来生产风险变化，再单独评估路线 2，不影响本期 Bubblewrap 主链。

## 4. 建议架构

### 4.1 平台中立执行接缝

不要继续在 `build_shell_command()` 中增加越来越多的 `cfg` 分支。建议拆成：

```text
ResolvedSandbox + command + cwd
  -> SandboxBackend::prepare()
  -> PreparedShell { program, argv, env_delta, backend, boundary }
  -> 统一 spawn / stdout / timeout / abort
  -> backend-specific violation classifier / cleanup
```

- macOS 适配现有 Seatbelt builder，不改变 profile 语义。
- Linux 新增纯 `LinuxSandboxPlan` 与 helper argv builder。
- Windows 保留专用 runner，但把 backend/boundary/violation 元数据接回统一结果。
- helper 构造失败属于基础设施错误，**不得**被当成普通路径越界并自动建议整命令无沙盒重跑。

### 4.2 `LinuxSandboxPlan`（纯数据、可跨平台测试）

输入：`RuleSet` 的当前快照、workspace、temp roots、command cwd。

输出至少包含：

- `writable_roots`：workspace、实际 session temp、明确 allow-write 根；去重并消除被更宽根覆盖的冗余项。
- `read_only_paths`：位于 writable roots 内的 ask/deny-write 具体路径。
- `unreadable_paths`：ask/deny-read 的精确路径和已展开 glob 匹配。
- `reopened_paths`：高优先级窄 allow 位于更宽 deny/read-only 下时的重开顺序。
- `missing_protected_paths`：不存在但需要阻止创建/读取的精确目标。
- `unsupported_dynamic_globs`：无法形成运行时硬保证的 glob，供 probe、日志和 UI 能力说明使用。
- `policy_digest`：执行前后诊断使用；不得含原始 secret 内容。

计划生成器不 spawn、不弹审批、不读 RPC，也不允许“遇到不支持规则就跳过”。无法安全编译时返回 typed error，shell 不执行。

### 4.3 helper 与自重入

FutureOS Desktop 目前只打包统一 `future` sidecar，Agent 也可由独立 `future-agent` 启动。建议在两个入口进入正常 runtime 前识别一个隐藏的内部 Linux helper 模式，并由当前可执行文件自重入；不要依赖 PATH 中另一个同名 helper，也不要让 helper 获取 Agent 单例锁。

helper 分两阶段：

1. 外层验证请求和 host capability，固定绝对 bwrap 路径，准备 mount source FD/目标，启动 bubblewrap。
2. bubblewrap 内层核对 FD/mount identity，设置 `no_new_privs` 和最小 seccomp hardening，再 exec shell。

内部请求应使用版本化、长度有界的序列化 payload 或继承 FD；禁止把不可控路径拼成一段 shell 字符串。helper 只接受父 Agent 生成的结构化 argv，不对外提供任意“以沙盒名义执行”的宽接口。

### 4.4 bubblewrap 基线

建议第一版固定：

- `--new-session --die-with-parent --unshare-user --unshare-pid --unshare-ipc --cap-drop ALL`。
- 文件系统从 `--ro-bind / /` 开始，挂最小 `/dev`，优先 fresh `/proc`。
- 网络保持 full access，不加 `--unshare-net`。
- writable roots 由宽到窄 bind，随后按规则优先级叠加 read-only/unreadable，再按需要重开更窄 allow。
- command cwd 必须在最终 view 中存在；逻辑 cwd 是 symlink 时显式进入规范化目标并记录差异。
- 子进程不得继承 Agent 的 gRPC listener、数据库、日志或审批内部 FD；只白名单保留 stdio 与 mount/request FD。

### 4.5 bwrap 来源、前置检测与失败语义

可用性不能等价于 `which bwrap`：

1. **安全 PATH 查找**：按 PATH 顺序寻找可执行的 `bwrap`，但忽略空/相对 PATH 项，并排除 canonical workspace/current directory 及其子路径；候选文件 canonicalize 后固定为绝对路径，后续版本检查与执行都使用同一路径，拒绝项目内伪造的 `bwrap`。第一版不自行扫描 PATH 之外的“兼容安装目录”，也不接受 workspace 内显式路径。
2. **版本与必需参数检查**：对固定路径执行有界的 `bwrap --version`，解析并记录版本；低于支持下限时返回 `version_too_old`。再解析 `bwrap --help`，逐项确认实现实际使用的参数，不能仅靠版本推断，因为发行版可能 backport 功能。最低版本和参数表由 L0 对目标发行版包实测后冻结。
3. **真实运行探测**：用 500ms～1s 有界子进程执行接近生产基线的 bwrap 计划，实际创建 user/PID/IPC namespace、只读 root、fresh `/proc`、最小 `/dev`、capability drop，并由可信内层 helper 验证“root 写失败、probe 临时目录写成功”；最终运行 `/bin/true` 等价动作。已决定网络开放，因此 probe 也不加 `--unshare-net`，避免探测一个生产中并不使用的能力。

三层任一失败都 fail closed。稳定结果至少区分：`available`、`binary_missing`、`path_rejected`、`version_unreadable`、`version_too_old`、`required_feature_missing`、`user_namespace_disabled`、`proc_mount_restricted`、`probe_timeout`、`probe_failed`。UI 和真正执行必须消费同一份带固定绝对路径、版本、能力与过期时间的成功结果；缓存过期或二进制 identity 改变后必须重新完成三层检测。

#### Codex 所说的“兼容路径”是什么

这里不是 WSL 兼容，主要有两件独立的事：

- upstream bubblewrap 从 v0.9.0 才支持 `--argv0`。Codex 的内外层 helper 依赖 multitool arg0 分派，所以遇到 Ubuntu 20.04/22.04 等旧包时，不传 `--argv0`，改用当前可执行文件路径进入内层 helper。它只解决 helper 如何启动，不会放宽文件系统或 namespace 策略。
- 部分 system bwrap 没有 Codex 使用的 `--ro-bind-fd`。Codex 会改写为标准的 `--ro-bind /proc/self/fd/<fd> <目标>`，再由可信内层 helper 验证 FD 对应的 mount identity，避免悄悄退化成只按字符串路径挂载。

FutureOS 的隐藏自重入 helper 使用明确子命令/参数分派，不依赖进程 `argv[0]`。因此**不要求 `--argv0`，也不实现 no-argv0 分支**，从设计上删除这项兼容复杂度。FD-backed mount 的 identity 复核属于 TOCTOU 防护，不能随兼容分支一起删除；system-bwrap-only 方案直接把标准 `/proc/self/fd/<fd>` + 内层复核作为主路径，而不是旧版 fallback。

缓存与失败语义：

- transient probe error 不写成进程生命周期永久“不支持”；允许在明确的缓存过期点重新探测。
- UI 和执行消费同一份成功结果，避免 UI 承诺可用而执行侧使用了不同的二进制或能力判断。
- FutureOS 不打包 bundled bwrap，也不在运行时下载。system bwrap 缺失、过旧或实际 probe 失败时保持 manual/off；设置页说明原因并给出官网安装教程入口。

## 5. Escalation、诊断与产品行为

- Linux 路径越界应识别 `EACCES`、`EPERM`、`EROFS`、helper 的稳定 policy-violation code，并排除 exit 2/126/127 等常见非沙盒失败；路径只能从可信 helper 元数据或 stderr 推断，推断值必须标记为推断。
- 一期越界批准与 macOS 一致：展示完整命令和“将脱离 Linux 沙盒重跑一次”的风险；批准后仅该次重跑不进入 bwrap。路径级临时能力与 macOS 改造一起进入二期。
- bwrap 缺失、userns 禁用、计划无法安全编译、helper 身份复核失败属于 sandbox infrastructure failure，不进入 escalation；返回明确错误并建议用户切换手动档或修复环境。
- `sandbox_boundary` 增加 `backend=linux_bubblewrap`、`policy_digest`、能力限制 code；普通 UI 不显示原始 host path 列表。
- 独立 `sandbox` 开发分支在 probe 成功后即可显示“沙箱保护”；合入主干仍需满足发布门槛。已保存为 sandbox 但环境后来失效时，回退 manual 并明确通知，不能悄悄把命令当作已受保护运行。

### 5.1 二期：跨 macOS/Linux 的路径级临时能力

Codex 的 additional permissions 思路适合 FutureOS：让 shell 在调用前声明本次需要的读/写路径，经现有规则引擎审批后重新编译**仍在沙盒内**的临时计划。Windows 已有 `additional_permissions.write` 的先例。

路径级方案可泛化到 macOS/Linux：

- `ask` 路径批准只增加本次 session capability；
- `deny` 和第 0/1 层不可覆盖；
- 未声明的越界仍由 OS 拒绝；
- 保留整命令脱沙盒作为显式高级选项，而非默认批准语义。

这比继续扩大 stderr 猜测更安全，也能逐步修复 macOS 当前“批准一个路径需求，却整条命令完全出沙盒”的权限放大接缝。

### 5.2 Seatbelt 是否能做路径级能力

**能。限制不在 Seatbelt，而在 FutureOS 当前审批协议和规则层级。** SBPL 可以在每次命令启动前加入一个临时 `allow file-read*` / `allow file-write*`，只开放已批准的 literal/subpath；命令仍由 `sandbox-exec` 包裹。Codex 当前也会先把 per-command `additional_permissions` 合并进 effective permission profile，再由 macOS Seatbelt 或 Linux 后端分别编译。

FutureOS 不能直接复用现有 session allow：当前优先级是 `overrides > guards > session > workspace > user`，所以 session allow 有意盖不过 secret guard。路径级“仅允许这一次”需要新增一个只属于本次命令的 `execution_grants` 层：

```text
layer 0 hard deny  >  approved execution_grants  >  secret ask guards  >  session/workspace/user
```

该层只能接收已经审批且原判定为 `ask` 的具体读/写路径，不能覆盖 `deny`、规则文件、自身配置等 layer 0 路径；同时绑定 command hash、request id、规范化路径、access 和单次执行。Seatbelt profile 发射顺序要让临时 allow 胜过对应 ask deny，但最终 hard deny 仍然胜出。

因此 Seatbelt 能支持该方案；已决定不混入 Linux 一期，二期建立跨平台 `execution_grants`，并把 macOS 从整命令脱沙盒迁移过来。

## 6. 分阶段开发计划

### L0 — Probe 规格与最低版本收口

- 固定“安全 PATH → 版本/参数 → 真实运行”三层检测协议、稳定 code、超时、binary identity 和缓存语义。
- 根据原生 Linux 支持矩阵确定最低 bwrap 版本与必需参数；不为 WSL 增加检测。
- FutureOS helper 协议不依赖 `argv[0]`，不实现 Codex 的 no-argv0 兼容分支。FD-backed mount 采用标准 `/proc/self/fd/<fd>` + 内层 identity 复核主路径。

完成定义：官网支持范围内通过包管理器可获得的 bwrap 版本与所需参数相容；不支持的版本在执行命令前返回明确错误。

### L1 — 纯计划生成器与平台接缝

- 新建 Linux plan 类型和 typed errors；从 `RuleSet` 快照编译 writable/read-only/unreadable/reopen/missing/glob diagnostics。
- 抽出平台中立 `PreparedShell` 接缝，先用现有 macOS/Windows 测试证明无行为变化。
- 单测覆盖规则层级、窄 allow 重开、workspace/temp、外部 allow、ask/deny、symlink、路径不存在、重复根、glob 上限和坏规则 fail closed。

完成定义：纯计划测试可在 macOS CI 运行；`sandbox` 开发分支可在 Linux 基础 probe 接通后直接显示入口，不增加独立隐藏开关。probe 未通过仍不得标为 available。

### L2 — Linux helper、bwrap 与生命周期

- 实现隐藏自重入 helper、结构化请求、FD 继承白名单和身份复核。
- 实现只读 root、writable bind、namespace、cap drop、no_new_privs 和 fresh `/proc`。
- 实现 PID 1 reaping、信号转发、timeout/abort/parent-death，保持原始 exit code/signal。
- 实现系统 bwrap 搜索与有界真实 probe；任何构造/验证失败 fail closed。

完成定义：Linux integration smoke 能证明 workspace/temp 写通过、workspace 外写拒绝、敏感具体路径读写拒绝、网络仍开放、孙进程不残留。

### L3 — 完整规则覆盖与 escalation

- 按已确认的有界 glob 方案加入启动前展开（rg + 内部 walker）、匹配数/深度上限、symlink 双路径和结束检测；所有扫描/计划异常必须 fail closed。
- 加入不存在精确路径保护；证明 host 不留临时对象，或清理使用 inode identity/CAS，不删除用户并发创建的对象。
- 接入 structured violation、Linux denial 分类和审批卡片；一期实现与 macOS 相同的整命令脱沙盒重跑。
- 对运行中新敏感 glob 做结束检测并明确标注 detection-only。

完成定义：规则矩阵与 macOS 对照测试通过；所有不等价项在文档、probe capability 和 UI 文案中一致。

### L4 — 打包与诊断（本地可交付部分已完成）

- 实现 system bwrap 探测、各支持发行版 UI 安装提示与官网详细教程；覆盖 x86_64/aarch64、glibc 目标和实际桌面分发格式。
- 增加 `future doctor`/Agent probe 等价诊断入口，输出稳定 JSON code；日志不泄露敏感路径。

完成定义：安装包内 helper/bwrap 来源可追溯、签名/更新路径明确，诊断能区分环境不支持与实现故障；开发分支入口可见不等于允许合入主干。

### L5 — 发布验证与安全 review

自动化矩阵至少覆盖：

- Ubuntu 22.04/24.04、Debian stable、Fedora 当前支持版；x86_64 和 aarch64 各至少一台真实环境。
- unprivileged userns 禁用、容器禁止 `/proc` mount、bwrap 缺失/过旧/参数不足、PATH 命中 workspace 伪造二进制的负向结果；WSL 不在支持或测试范围内。
- bash/cargo/npm/python/git 常用链路，Unicode、大输出、cwd symlink、外部 writable root、嵌套规则、敏感 glob、并发创建和 abort/timeout。
- `.deb` 和 portable tarball 安装、更新、卸载及 helper 完整性；本期不发布 AppImage/rpm。
- 安全 review：mount TOCTOU、FD 泄漏、setuid bwrap 边界、namespace/capability、临时对象 CAS 清理、错误路径是否会无沙盒重跑。

结果必须分为 `PASS / FAIL / NOT RUN / ENVIRONMENT LIMIT`，不能用 macOS 单测或容器 smoke 代替 Linux 真机发布结论。

### L6 — 主干合入与产品发布（本地接入已完成，发布仍受 L5 阻塞）

- 收口开发分支已经接入的 UI availability hook；probe 失败回退 manual，并给出可操作说明。
- 同步 PRODUCT/ER/APPROVAL/SANDBOX 文档和中英文 UI，准确写出 glob 有界保证。
- 独立 `sandbox` 分支允许在开发期间直接显示“沙箱保护”；只有 L5 主机矩阵和安全 review 达标后才允许把该入口和后端合入主干。灰度期间保留快速关闭开关和稳定诊断 code。

完成定义：UI 承诺、Agent 实际 backend、probe 结果和发布文档四者一致；任何失败都不会静默无沙盒执行。

## 7. 实施前剩余收口项

产品方向已经记录在文首 L-D1–L-D9。开始主体开发前，L0 还需要用原生 Linux 环境冻结以下工程参数：

1. 支持发行版自带 system bwrap 的最低版本；不能为了一个未使用的 `--argv0` 人为抬高版本线。
2. 最终 bwrap 必需参数表；版本检查只负责快速诊断，`--help` 参数检查和真实运行 probe 才是能力依据。
3. probe 超时、成功缓存有效期和 binary identity 字段；所有失败映射到 §4.5 的稳定 code。
4. 设置页安装提示和官网教程覆盖的发行版、包管理器与排障命令。

这些参数必须随 L0 的发行版矩阵一起提交 review；在冻结前不得让 Linux sandbox availability 返回 `available`。

## 8. 代码证据索引

FutureOS 当前实现：

- `agent/src/sandbox/mod.rs`：`SandboxTier`、`ResolvedSandbox`、平台 availability、Seatbelt 构造和 denial heuristic。
- `agent/src/sandbox/rules.rs`：分层 `RuleSet`、内置 secret glob、规则文件保护和 session rule。
- `agent/src/sandbox/seatbelt.rs`：SBPL 编译、last-match 顺序和 shell wrapper。
- `agent/src/tools/mod.rs`：shell spawn、Windows runner 分支、timeout/abort 与 escalation。
- `agent/src/rpc/approval.rs`：shell 前置审批和 Windows `additional_permissions.write` 路径能力。
- `agent/src/rpc/commands/settings.rs`：产品 availability 与 `set_sandbox_policy`。
- `agent/src/cli.rs`、`cli/src/main.rs`、`desktop/src-tauri/src/agent_supervisor.rs`：独立 Agent、统一 `future agent` 和 Desktop 单 sidecar 打包/启动边界。

Codex 对照（`~/workspace/codex` @ `f20b63e85c`）：

- `codex-rs/sandboxing/src/manager.rs`：平台后端选择、permission profile 转换和 Linux helper 接缝。
- `codex-rs/sandboxing/src/bwrap.rs`：系统 bwrap 搜索、user namespace probe、WSL 判定和 warning。
- `codex-rs/linux-sandbox/README.md`：当前行为、system/bundled 选择、split policy、网络和 WSL 支持说明。
- `codex-rs/linux-sandbox/src/bwrap.rs`：mount plan、glob 展开、symlink/missing path、窄规则重开。
- `codex-rs/linux-sandbox/src/launcher.rs`：system/bundled capability 探测、FD mount 兼容和 exec。
- `codex-rs/linux-sandbox/src/linux_run_main.rs`：外层 bwrap、内层 seccomp、PID 1、信号和 cleanup。
- `codex-rs/linux-sandbox/src/landlock.rs`：`no_new_privs`、seccomp 与 legacy Landlock 边界。
- `codex-rs/sandboxing/src/violation.rs`、`denial.rs`、`codex-rs/cli/src/doctor/sandbox.rs`：结构化违规、拒绝判断和诊断输出。
