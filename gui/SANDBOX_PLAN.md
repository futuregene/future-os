# FutureOS Sandbox 方案（SANDBOX_PLAN）

状态：**v2 —— 强制执行层设计 + v1→v2 返工计划**（2026-07-04 按纯文件路径规则模型重写）

> 规则系统的**语义主文档**是 [APPROVAL_PLAN.md](APPROVAL_PLAN.md)（规则模型、分层、规则文件、审批 UI、决策记录）。本文只管**强制执行**：Seatbelt 如何从规则编译、escalation、工具层拦截、协议、降级模式，以及 v1 已实现资产的复用/返工清单。
>
> v1（Codex 式"三模式×三策略 + SQLite 规则"）已实现并全绿（原 Phase 1/2，见 §6），随 v2 模型被部分取代——v1 的历史设计见 git history 本文件旧版。

## 1. v2 强制模型一览

一切审批围绕文件路径的 `ask / allow / deny`（判定逻辑见 APPROVAL_PLAN §3）。判定结果在两条执行路径上分别强制：

```
                     ┌──────────────────────────────┐
   规则判定           │  4 层规则 + 读写分车道兜底      │  （APPROVAL_PLAN §3）
   (agent 进程内)     │  → ask / allow / deny         │
                     └──────┬───────────────┬───────┘
                            │               │
              ┌─────────────▼──┐      ┌─────▼──────────────────┐
   工具路径    │ read/write/edit │      │ bash（+grep 子进程）     │   bash 路径
              │ 逐调用真三态：   │      │ 规则编译进 Seatbelt：    │
              │ ask→前置弹窗    │      │ allow→放行              │
              │ allow→执行      │      │ ask/deny→OS 层拒        │
              │ deny→拒绝       │      │ 失败→escalation 审批     │
              └────────────────┘      └────────────────────────┘
```

与 v1 的关键差异：

| | v1（已实现） | v2（本计划） |
|---|---|---|
| 规则对象 | 命令前缀 + 路径 glob，存 SQLite，gRPC 下发 | **纯文件路径**，两个 JSON 文件，agent 直接读 |
| 网络 | 默认断网，escalation 放行 | **完全放开** |
| bash 前置审批 | 白名单外可能预弹（untrusted/降级） | **无前置审批**，全靠 Seatbelt + escalation |
| read 工具 | 不受控（漏洞） | 接入三态审批 |
| 模式/策略 | 3 模式 × 3 策略 | **单布尔 enabled** |
| workspace 内 `.env` | 无保护（盲区） | 内置 ask 覆盖 |
| 决策持久化 | 本会话/始终 → SQLite | 本工作区允许 → workspace 规则文件 |

## 2. Seatbelt：从规则编译 profile（macOS）

沿用 v1 的 `sandbox-exec -p <profile>` 包裹机制（`agent/src/sandbox/seatbelt.rs`，含 SBPL 转义、canonicalize 注入、进程组 kill、smoke tests——全部复用），**profile 内容改为从判定后的规则集编译**：

```scheme
(version 1)
(deny default)
(allow process-fork) (allow process-exec) (allow process-info*)
(allow signal (target same-sandbox)) (allow pseudo-tty)
(allow sysctl-read) (allow mach-lookup) (allow ipc-posix*) (allow file-ioctl)

; ── 读：默认全放（兜底 read→allow），扣除判定为 ask/deny 的读规则 ──
(allow file-read*)
(deny file-read* <每条 ask/deny + access∈{read,both} 的规则，glob→SBPL>)

; ── 写：白名单式。workspace + temp + 用户 allow-write 规则的路径 ──
(allow file-write-data <伪设备：/dev/null /dev/fd/* /dev/tty* ...>)
(allow file-write* (subpath "<workspace>") (subpath "<TMPDIR真实路径>") (subpath "/private/tmp")
                   <每条 allow + access∈{write,both} 且在 workspace 外的规则路径>)
; 写侧的 ask/deny 规则若落在上述可写区域内，追加显式 deny（后写规则赢）：
(deny file-write* <第0层：两个规则文件、agent 凭证文件> <workspace 内 ask/deny 写规则>)

; ── 网络：v2 恒放开 ──
(allow network*) (allow system-socket)
```

编译要点：

- **glob → SBPL**：无通配符且为目录 → `subpath`；无通配符文件 → `literal`；含 `*`/`**`/`?` → `regex`（glob 转正则，转义其余元字符）。SBPL regex 是全功能正则，任意 glob 都可表达。
- 所有路径 canonicalize 后嵌入（`/tmp → /private/tmp`、`$TMPDIR → /private/var/folders/...`），SBPL 字符串经 `sb_quote` 转义（防注入）——v1 机制不变。
- **规则顺序即安全语义**：SBPL 后写的规则赢，所以 deny 子句必须排在对应 allow 之后。第 0 层（规则文件写保护）永远编译在最后。
- rename/mv 绕过（`mv x .future/approval_rule.json`）：SBPL `file-write*` 对 rename 目标路径生效，smoke test 显式覆盖这一条。
- `mach-lookup`/`sysctl` 沿用 v1 的"先放宽、按 smoke tests 收窄"策略；v1 的 9 项 smoke tests 全部保留，网络两项改为断言"默认放行"。

## 3. escalation（沿用 v1，语义不变）

bash 在沙盒内失败且匹配拒绝特征（`Operation not permitted` 等，保守启发式），或模型显式带 `escalated: true` + `justification` 重试 → `sandbox_escalation` 审批 → **批准后该条命令出沙盒完整重跑一次**（原 Q2"按命令放行"）。

- v1 的 `EscalationRequester` 回调架构（rpc 层构造、经 `ToolExecutionScope` 注入、tools 层不碰 RPC）原样复用。
- 失败特征里删掉网络类 marker（`Could not resolve host` 等）——网络已放开，这些不再是沙盒拒绝的信号。
- 在 v2 里 escalation 是 bash 版的 "ask"：规则里写 ask 的路径，bash 撞上时的体验就是"失败 → 弹窗问是否放行重跑"。
- **escalation 也可「本对话/工作区允许」**（2026-07-04 补）：当被拦文件**都非敏感**时，卡片从被拦路径推出父目录 glob（写权限）作 `save_suggestion`，点「本对话允许」→ 写规则文件 + 当轮注入 → 之后 bash 写该目录在沙箱内放行、不再 escalation。这样 bash 越界写和 write 工具越界写的持久化体验一致。**敏感文件**（`~/.gnupg` 等）`save_suggestion=None`，仍只能"允许一次"（守卫不可被绕过）。
- 已知残留（知情接受）："允许一次"批准的命令出沙盒后无任何限制；卡片如实展示命令全文与被拦文件。
- 配套（源头治理）：系统提示强化为"写文件一律用 write/edit（支持绝对路径 / 工作区外），不要用 bash 重定向写文件"，减少 bash 旁路写（也修掉"文件找不到"——bash `>` 写的文件不登记 artifact）。

## 4. 工具层强制

- **read（新增拦截点）**：`run_read` 执行前评估规则；`ask` → 经 before_tool_call 同款审批流前置弹窗；`deny` → 直接错误。v1 中 read 完全不受控，是本次补上的真实漏洞。
- **write / edit**：`ensure_workspace_access` 从"writable_roots 集合判定"改为完整规则判定（含第 0 层写保护、workspace 内 ask/deny）。路径规范化（`~`→真实 HOME、最近存在祖先 canonicalize、symlink 最终路径、macOS 大小写不敏感）沿用 v1 的 `sandbox/paths.rs`，原样复用。
- **grep / ls 工具**（非默认工具集，但存在）：`run_grep` spawn 的系统 `grep` 子进程必须同样包 Seatbelt（否则是旁路读通道）；`run_ls` 按目录读评估规则。
- **审批弹窗**：复用现有 ApprovalPrompt 链路（SSE → SQLite → 1.5s 轮询 → composer 上方卡片，串行、不超时），按钮改为"拒绝 / 允许一次 / 本工作区允许"（APPROVAL_PLAN §6）。
- **当轮即时生效**：`approval_decision` 回传附带已保存规则，agent 注入当前 session 内存规则集（机制类比现有 `approve_outside_path`）。

## 5. 协议与配置

- `SandboxPolicy` 消息瘦身为单布尔（proto 字段号不复用，防混版本歧义）：

```protobuf
message SandboxPolicy {
  reserved 1 to 5;          // v1: sandbox_mode / writable_roots / network_access / approval_policy / rules
  bool enabled = 6;
}
```

- GUI 在 session 建立时（现有 `set_sandbox_policy` 时机）下发 `enabled: true`；「自动审批」开关打开时下发 `false`。
- **配置真源反转**（v2 有意为之）：规则在两个 JSON 文件里、agent 直接读，不再经 SQLite + gRPC 下发。`approval_rules` 表及 `save_approval_rule` / `list_effective_rules` / `prune_session_rules` 链路废弃（表保留不删，防降级；代码路径移除）。`sandbox_config` / `approval_policy_config` 两张预留表继续闲置。
- GUI 写 workspace 规则文件：`src-tauri` 新增规则文件读写模块（serde 结构与 agent 侧解析对齐；写入走"读-改-写整文件"，保留未知字段）。

## 6. v1 资产：复用 / 返工清单

v1（原 Phase 1 + Phase 2）已全部实现并通过验证（agent 67 测 + GUI 69 + 前端 39 + smoke 9 + lint），代码在 `sandbox` 分支。逐项处置：

**原样复用（不动）**：
- `sandbox/paths.rs` 路径规范化全套 + 单测。
- `sandbox/seatbelt.rs` 的包裹机制：`sb_quote`、canonicalize 注入、`sandbox-exec` 命令构造、进程组 kill、`/dev/fd` 等伪设备经验、smoke test 框架。
- `EscalationRequester` 架构、bash `escalated/justification` 参数、失败特征启发式（删网络 marker）。
- ApprovalPrompt / useApprovals / 审批持久化 / `sandbox_boundary` payload / `tool_sandboxed` 事件等整条 UI 链路。
- opt-in 骨架：`ServerSession.sandbox_policy: Option<_>`、非 GUI 客户端休眠（`ResolvedSandbox::disabled` 路径）。

**改造**：
- 规则类型 `SandboxRule{match_kind,match_value,decision}` → `PathRule{path,access,action}`；`evaluate_policy` 重写为四层判定（通配符匹配代码可复用）。
- `ResolvedSandbox` 去掉 mode/approval_policy/network_access，改挂"已解析规则集"（内置层 + 两文件解析结果 + 兜底参数）。
- `seatbelt::build_profile` 改为从规则集编译（§2）。
- `approval_shape`：bash 分支删除（无前置审批）；write/edit 分支按规则判定产出 ask 卡片；**新增 read 分支**。
- `ensure_workspace_access` → 规则判定。
- ApprovalPrompt 按钮与保存流程（"本会话/始终" → "本工作区允许" + 规则预览编辑，前端组件大半可改造复用）。

**废弃**：
- `is_workspace_read_command` 白名单（启用会话中 bash 无前置审批；非启用会话保留现状，待 v2 稳定后随旧路径一起清理）。
- `command_prefix` 规则、`save_suggestion` 的命令建议（路径建议保留）。
- SQLite 规则链路（§5）；proto `SandboxPolicy` 旧字段；三模式/三策略枚举。

## 7. 降级模式（Linux / Windows，现状语义延续）

- `platform_sandbox_available()` 非 macOS 恒 false → bash 裸跑、无 OS 强制、escalation 不触发（无沙盒可逃）。
- **工具层三态照常生效**（规则判定在 agent 进程内，不依赖平台）——read/write/edit 的 ask/deny、凭证 ask、第 0 层写保护在 Linux/Windows 依然工作。**只有 bash 是无强制的**：`cat ~/.ssh/id_rsa` 在降级模式下不会被拦。此差异写入文档与（后期）GUI 降级提示，不做额外补偿。
- Linux bwrap 仍按"最后再做"排期：写侧 bind 白名单同构可编译；读侧 ask/deny 用 `--tmpfs`/`--ro-bind` 遮盖近似。
- **Windows 原生沙盒方案已定（见 §11）**，把 bash 从"降级裸跑"升级到"restricted token + ACL 强制"；本节的 Windows 降级语义仅作为 §11 落地前的现状。

## 8. 实施阶段

### Phase R1 — agent 侧规则引擎与强制（核心）— ✅ 已完成（2026-07-04）

- [x] `PathRule` + 规则文件解析（fail-safe：坏文件跳层 + `tracing::warn`）+ 四层判定 + 兜底分车道；glob→regex（无通配符=子树，复用路径规范化）；单测（分层优先级、首匹配短路、子树、symlink、第 0 层不可覆盖、坏文件不 fatal）。
- [x] 内置清单：第 0 层（规则文件写 deny + app 凭证文件 `auth.json`/`models.json` 读写 deny，不可覆盖）+ 第 4 层（home/workspace 凭证 ask）；temp 并入写兜底（不作规则，避免遮蔽 secret）。
- [x] `ResolvedSandbox` 挂 `RuleSet` + 单 `enabled` + `seatbelt::build_profile` 从规则编译（低→高优先级发射，SBPL last-match=引擎 first-match）+ `(allow network*)`；smoke tests 全绿（网络放行、`.env` 读写被拒、规则文件写 + `mv` 改名被拒、`auth.json` 读被拒、`~/.ssh` 读被拒、workspace/temp 写通过、cargo/git/python 不碎）。
- [x] read 工具接入审批；write/edit 改 `evaluate()` 判定；`approval_shape` 删 bash 前置、加 read 分支；命令级规则/白名单/`approval_policy.rs` 全删。（grep 子进程沙盒：grep 非默认工具集，暂缓，见 §9。）
- [x] proto `SandboxPolicy` 瘦身（reserved 1-5 + `enabled = 6`）；`ServerSession`/grpc/commands 简化。
- [x] escalation 网络 marker 移除（网络放开，只留 fs EPERM 特征）。
- 验收：agent 55 lib + 10 规则单测 + 9 Seatbelt smoke 全绿；`make lint` 干净。

### Phase R2 — GUI 侧 — ✅ 已完成（2026-07-04）

- [x] `set_sandbox_policy` 改发 `enabled: true`（GUI 会话启用；自动审批开关发 false 留 R3）。
- [x] ApprovalPrompt 三按钮（拒绝 / 允许一次 / **本工作区允许**）+ 路径预览内联编辑；`save_suggestion` 前端解析改 v2 `{path, access}`；agent 侧建议路径改为 **workspace 相对**（可进 git）。
- [x] `approval_rules.rs`（新）读-改-写 `${WS}/.future/approval_rule.json`（保留既有规则 + 未知字段 + 去重）；`save_approval_rule` 命令改写文件（GUI 走 Tauri 可信路径，绕过 agent 沙盒——正是第 0 层写保护针对 agent 的意义所在）；单测 3 项。
- [x] 拆除 SQLite 规则链路（`list_effective_rules`/`prune_session_rules`/SQLite `save_approval_rule` 导出移除、启动 prune 移除、`set_sandbox_policy` 不再展平规则）；`approval_rules` 表闲置（保留不删）。
- 验收：`make check-gui` + vitest(39) + tsc + eslint 全绿。
- **已知未做（R3 补）**：**当轮即时生效**（§6.2 内存注入）——写文件后 agent 下一轮 prompt 才重读。当前"本工作区允许"让本次操作通过（写走 `approve_outside_path`，读经审批放行），但同一轮内对该目录下**其他**文件的同类操作仍会再问一次。需给 agent 加 session 规则注入命令，留 R3。read 审批卡片沿用 file_write 模板渲染（够用）。

### Phase R3 — 敏感守卫 + 当轮即时生效 + 文档（2026-07-04）

- [x] **敏感文件守卫**（方案 A）：`builtin_guards` 层置于 overrides 之下、用户规则之上 → 敏感文件（`.env`/`*.pem`/`*.key`/`*.p12`/`id_rsa*` + home 凭证）**不可被规则覆盖**，只能“允许一次”；宽目录 allow（`config/*`）盖不住里面的密钥（Q1 修复）。`is_secret_path` + 单测（broad_allow_does_not_ungate_secret_in_dir 等）。
- [x] 敏感文件抑制持久化建议：`approval_shape` 命中 secret → `save_suggestion = None`（GUI 自动隐藏“本工作区允许”，只剩拒绝/允许一次）。Seatbelt profile 按新层序发射（守卫 deny 在高位）。
- [x] **当轮即时生效**：`RuleSet.session` 改 `Arc<Mutex<Vec<PathRule>>>`（`SessionRules`）；`ServerSession.session_rules` 每轮清空、`resolve_with_session` 共享进 live sandbox；`add_session_rule` gRPC 命令；GUI `save_approval_rule` 写文件后经 `inject_session_rule` 注入当前 agent 会话 → 本轮同目录后续操作立刻不再问。守卫压得住 session（密钥仍每次问）。
- [x] 文档：PRODUCT.md §4.6、ER.md §4.8 同步 v2；本文件 + APPROVAL_PLAN 更新。
- 验收：agent 58 lib + smoke 9 + GUI 72 + 前端 39 全绿；`make lint`/`check-gui` 干净。
- **不做**（本期）：设置菜单编辑 user 级规则、规则列表查看；Linux bwrap；降级提示徽标。

## 9. 明确不做

- 命令级审批规则（allow/ask/deny by command prefix）——纯文件模型，试用后再评估（APPROVAL_PLAN §8）。
- 网络审批 / 域名过滤——不做。将来若确需，经"Seatbelt 锁出口到本地代理 + 代理读 CONNECT/SNI"实现，届时再加规则类型（本版 schema 不预留）。
- escalation 精确放宽（只开单项权限）。
- ~~Windows 原生沙盒~~（**改为做**，方案见 §11）；bwrap 捆绑 helper。
- `auto_review`（审查 agent 作 reviewer）。
- MCP / 新工具的沙盒接入规范（工具集扩展时再定）。

## 10. 决策记录

v2 决策（V1–V9）见 APPROVAL_PLAN §9。v1 期间沿用有效的：escalation 按命令放行（Q2）、channels 无审批 UI 按失败返回（Q3）、`.git` 不排除（Q4）、`sandbox-exec` deprecated 接受（Q5）、失败启发式保守（Q6）、默认切换不告知（Q7）、temp 读写全开、macOS→Linux→（无 Windows）平台顺序、R1–R6 安全 review 修正（详见 git history 本文件 v1 版）。

## 11. Windows 原生沙盒（unelevated：restricted token + ACL）

状态：**方案已定（2026-07-05）**，待实现。目标是把 Windows 的 bash 从 §7 的"降级裸跑"升到与 macOS Seatbelt 同级的"**读松写紧 + 密钥拒读**"OS 强制。规则引擎（`rules::RuleSet`）、escalation 架构、审批 UI 全部复用——本节只描述 Windows 的**强制后端**，对标 §2（Seatbelt）。

### 11.1 选型：为什么是 unelevated

对照 Codex 三条腿（elevated 独立用户 + 防火墙 / unelevated 受限令牌 + ACL / WSL 复用 bwrap）与 Low-IL，按本项目约束（**读默认开、写白名单、网络全开、免管理员**）取 **unelevated**：

| 候选 | 免管理员 | 读默认开 | bash 拦密钥 | 与我们模型贴合 | 结论 |
|---|---|---|---|---|---|
| elevated（独立用户）| ❌ 要管理员/企业易崩 | ❌ 需补读 | ✅ 天然 | 低（还自带多余的网络隔离）| 暂缓 |
| **unelevated（受限令牌+ACL）** | ✅ | ⚠️ 可调到基本全开 | ✅ deny ACE | **高** | **选它** |
| Low-IL（完整性级别）| ✅ | ✅ 天然 | ❌ 拦不住 | 中 | 备选 |
| WSL2 复用 bwrap | ✅ | — | ✅ | — | 兜底腿，排在 Linux bwrap 之后 |

关键差异 vs Codex：**我们网络全开**，故 Codex 最重、最容易在企业机器上崩的那半（防火墙 / offline-user / 本地策略，elevated 的 1385 之类）**全部不做**——只做文件系统边界这一半，比 Codex 简单。

### 11.2 强制模型

`CreateRestrictedToken` 派生**受限令牌**：带 restricting SIDs 后每次访问检查跑两遍（正常 SID 列表 ∧ 受限 SID 列表，两遍都过才放行）。据此把规则引擎的语义映射到 NTFS ACL：

```
受限 SID 集 = { Users / Authenticated Users, RESTRICTED }   ← 读的宽度旋钮
  → 读：系统 / Program Files / 授 Users 的东西"两遍都过"= 基本全开（免逐条补 ACE）
  → 写：用户文件普遍只授本人 SID 写、不授 Users → 受限那遍挂掉 = 默认写不动

写白名单 = 给一个自定义"沙盒 SID"加 **写 ACE**，仅贴在：
  workspace  +  每会话专属 temp  +  用户 allow-write 规则路径

密钥拒读 = 给密钥集 + 规则文件加**显式 deny ACE**（deny 永远赢）：
  .env / *.pem / *.key / *.p12 / id_rsa* / home 凭证 / .future/approval_rule.json

网络 = 不动（v2 全开）
进程组 kill = CreateJobObject + JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE（对标 mac 进程组 kill）
```

**层序即安全**（同 §2）：deny ACE 必须在 allow 之前解析（NTFS 规范序里 deny 在前），密钥 deny 压过任何宽目录 allow —— 对应引擎里 `builtin_guards` 不可覆盖。

### 11.3 读摩擦与已定取舍

受限令牌天生"读也受限"（对象 ACL 不含沙盒可用 SID 就读不到）。按四个旋钮定档，全部偏**低摩擦**：

1. **#1 受限集放 `Users`**：系统 / Program Files / 大部分工具链"两遍都过",读基本全开、**免大规模补读 ACE**。副作用是"被授 `Users`/`Everyone` **可写**的异常目录"会成写洞 → **启动时扫描并警告**（仿 Codex 的 Everyone-writable 提示），不自动收紧。
2. **精简 #2 预置读授权**：用户目录配置多按**所有者本人**授权、`Users` 盖不到（`~/.gitconfig`/`.config`、`~/.npmrc`、`~/.cargo`、`~/.rustup`、常用 `%APPDATA%` 工具目录）→ 只给这一小撮预置沙盒 SID 读 ACE，首跑不炸；系统/Program Files 靠 #1 免预置。
3. **#3 密钥 deny ACE**：读放开的同时对密钥集显式 deny（§11.2）——**这是 unelevated 比 Low-IL 强的点：读默认开 + 精确拦密钥**，正好复刻引擎语义，直接补上 §7 里"bash 能读 `~/.ssh`"那个缺口。
   - 白赚项：owner-ACL 的用户密钥本就不授 `Users`，即使不加 deny 也读不到；deny ACE 是对"落进 workspace / 被 #2 放宽目录里的密钥"的兜底。
4. **#4 漏读兜底**：真·长尾漏读 → 弹一次审批"本工作区允许读该目录"（对标 Codex `/sandbox-add-read-dir`），批准落成规则。read 兜底不如 write escalation 可靠（发生在命令中途、`Access is denied` 太泛、归因难、要整命令重跑），故只当长尾，不作主力——主力是 #1+#2。

### 11.4 escalation / 工具层（复用）

- bash 写被拒 → stderr 出现 `Access is denied` / 错误码 5 → `looks_like_sandbox_denial` 加 Windows 特征 → 批准后该命令**出沙盒以正常令牌重跑一次**（同 §3）。`EscalationRequester` 架构原样复用。
- 工具层三态（read/write/edit 的路径 ask/allow/deny + 密钥守卫）本就跨平台，无需改。

### 11.5 代码接缝

只改三处 + 新增一个后端文件，规则引擎/审批/escalation 不动：

- `sandbox/mod.rs::platform_sandbox_available()`：Windows 分支返回 true（现代 Windows 恒可用）。
- `sandbox/mod.rs::build_bash_command()`：加 `#[cfg(windows)]` → 委托 `windows::build_command`（对标现有 `seatbelt::build_command`）。
- `sandbox/mod.rs::looks_like_sandbox_denial()`：加 Windows 拒绝特征（`Access is denied` / OS error 5）。
- **新增 `agent/src/sandbox/windows.rs`**（对标 `seatbelt.rs`）：从 `rule_set()` 编译成"受限令牌 + 一组 ACE 变更 + Job 对象"，返回 `tokio::process::Command`。用 `windows-sys`：`CreateRestrictedToken`、`SetNamedSecurityInfo`（ACE 增删）、`CreateJobObject`/`AssignProcessToJobObject`、`CreateProcessAsUser`/`STARTUPINFOEX`。

### 11.6 已知取舍 / 缺口（知情接受）

- **有状态**：写授权与密钥 deny 靠在真实路径上增删 ACE，非 macOS 的纯 per-exec 无状态。需 **RAII guard** 在命令结束/崩溃时回滚；每会话独立 temp 目录；并发命令对同一 workspace 的 ACE 变更要串行或幂等。
- **世界可写目录洞**：#1 放 `Users` 的代价，靠启动扫描 + 警告暴露，不自动改用户目录权限。
- **网络不拦**：同 v2，外传不在本方案射程内（防泄露靠"密钥读不到"这一端）。
- **shell 前提**：工具现走 `Command::new("bash")`，Windows 需 Git Bash/WSL bash 存在；沙盒对 shell 无关（包的是令牌 + Job），"Windows 该用哪个 shell / 要不要支持 PowerShell"是独立产品决策。
- **elevated（独立用户强隔离）暂缓**：留给将来"要独立安全主体"的企业诉求。

### 11.7 实施阶段

**Phase W1 — 核心强制**：`windows.rs`（受限令牌 = {Users, RESTRICTED} + 沙盒 SID 写 ACE 贴 workspace/temp/放行路径 + 密钥 deny ACE + Job kill + RAII ACE 回滚 + 每会话 temp）；`platform_sandbox_available`/`build_bash_command`/`looks_like_sandbox_denial` 三处 cfg 分支；精简 #2 预置读根;单测 + smoke（写 workspace 通过、写外部被拒、`.env`/`~/.ssh` 读被拒、规则文件写被拒、git/python 不碎、escalation 触发）。

**Phase W2 — 打磨（可选）**：#4 漏读兜底的审批 UX（`add-read-dir` 等价）；世界可写目录扫描警告；GUI 降级/启用提示按平台区分。

**明确不做（本期）**：elevated 独立用户 + 防火墙；网络隔离；WSL 捆绑。
