# FutureOS Sandbox 方案（SANDBOX_PLAN）

状态：**v2 macOS 已实现；Windows unelevated 写保护底层已通过首台真机验收，产品入口仍关闭**（2026-07-04 按纯文件路径规则模型重写；2026-08-24 收口 Windows 底层与发布验证）

> 规则系统的**语义主文档**是 [APPROVAL_PLAN.md](APPROVAL_PLAN.md)（规则模型、分层、规则文件、审批 UI、决策记录）。本文只管**强制执行**：Seatbelt 如何从规则编译、escalation、工具层拦截、协议、非 macOS 的两档回退，以及 v1 已实现资产的复用/返工清单。
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
| bash 前置审批 | 白名单外可能预弹（untrusted/降级） | 沙箱保护档靠 Seatbelt + escalation；手动审批档靠只读白名单免问 + 卡片审批 |
| read 工具 | 不受控（漏洞） | 接入三态审批 |
| 模式/策略 | 3 模式 × 3 策略 | **单一 `tier` 三态**（manual/sandbox/off） |
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

## 3. macOS escalation（沿用 v1）

macOS shell 在 Seatbelt 内失败且匹配拒绝特征（`Operation not permitted` 等，保守启发式），或模型显式带 `escalated: true` + `justification` 重试 → `sandbox_escalation` 审批 → **批准后该条命令脱离 Seatbelt 完整重跑一次**（原 Q2“按命令放行”）。

- v1 的 `EscalationRequester` 回调架构（rpc 层构造、经 `ToolExecutionScope` 注入、tools 层不碰 RPC）原样复用。
- 失败特征里删掉网络类 marker（`Could not resolve host` 等）——网络已放开，这些不再是沙盒拒绝的信号。
- Seatbelt 拒绝不保证向父进程报告准确路径。卡片可展示从 stderr 推断的路径，但必须标注为推断；无法归因时显示“未知”，不得生成持久路径规则。
- macOS escalation 仅提供拒绝/允许一次，不提供“本工作区允许”。“允许一次”批准后命令无文件限制；卡片必须如实展示命令全文和该风险。
- Windows **不复用**本节的整命令脱沙盒语义，改用 §11.4 的声明式具体路径 capability；仅有 error 5 时不得弹出模糊批准。
- 配套（源头治理）：系统提示强化为“写文件一律用 write/edit（支持绝对路径 / 工作区外），不要用 shell 重定向写文件”，减少 shell 旁路写（也修掉“文件找不到”——shell 重定向写的文件不登记 artifact）。

## 4. 工具层强制

- **read（新增拦截点）**：`run_read` 执行前评估规则；`ask` → 经 before_tool_call 同款审批流前置弹窗；`deny` → 直接错误。v1 中 read 完全不受控，是本次补上的真实漏洞。
- **write / edit**：`ensure_workspace_access` 从"writable_roots 集合判定"改为完整规则判定（含第 0 层写保护、workspace 内 ask/deny）。路径规范化（`~`→真实 HOME、最近存在祖先 canonicalize、symlink 最终路径、macOS 大小写不敏感）沿用 v1 的 `sandbox/paths.rs`，原样复用。
- **grep / ls 工具**（非默认工具集，但存在）：`run_grep` spawn 的系统 `grep` 子进程必须同样包 Seatbelt（否则是旁路读通道）；`run_ls` 按目录读评估规则。
- **审批弹窗**：复用现有 ApprovalPrompt 链路（gRPC 事件流 → SQLite 落库 → `approvals-updated` 推送、15s 轮询兜底 → composer 上方卡片，串行、不超时），按钮改为"拒绝 / 允许一次 / 本工作区允许"（APPROVAL_PLAN §6）。
- **当轮即时生效**：`approval_decision` 回传附带已保存规则，agent 注入当前 session 内存规则集（机制类比现有 `approve_outside_path`）。

## 5. 协议与配置

- `SandboxPolicy` 消息瘦身为单一 `tier` 字符串（proto 字段号不复用，防混版本歧义；连 `bool enabled = 6` 也一并 reserve，杜绝布尔版歧义）：

```protobuf
message SandboxPolicy {
  reserved 1 to 5;          // v1: sandbox_mode / writable_roots / network_access / approval_policy / rules
  reserved 6;               // v2 布尔 enabled（已废）
  string tier = 7;          // "manual" | "sandbox" | "off"
}
```

- GUI 在 session 建立时（现有 `set_sandbox_policy` 时机）下发当前档位 `tier`（默认 `manual`）。agent 端 `SandboxTier::parse` 未知值一律落到 `manual`。
- **配置真源反转**（v2 有意为之）：规则在两个 JSON 文件里、agent 直接读，不再经 SQLite + gRPC 下发。`approval_rules` 表及 `save_approval_rule` / `list_effective_rules` / `prune_session_rules` 链路废弃，代码路径移除。`approval_rules` / `sandbox_config` / `approval_policy_config` 三张表连同 `store/approval_config.rs` 已于 2026-07-05 删除（死结构清理）。
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
- ApprovalPrompt 按钮与保存流程（R2 已从“本会话/始终”改为“本工作区允许”并加入规则预览；Windows W5 继续复用组件，同时按 APPROVAL_PLAN §6 收敛为“行为 + 目标”标题、折叠详情和非技术化持久允许，不保留普通卡片的 raw glob 编辑）。

**废弃**：
- `is_workspace_read_command` 白名单（启用会话中 bash 无前置审批；非启用会话保留现状，待 v2 稳定后随旧路径一起清理）。
- `command_prefix` 规则、`save_suggestion` 的命令建议（路径建议保留）。
- SQLite 规则链路（§5）；proto `SandboxPolicy` 旧字段；三模式/三策略枚举。

## 7. 非 macOS：当前回退与 Windows 目标态

> 当前发布态仍只有 macOS 提供 OS 沙盒；Windows unelevated 后端完成并通过 Windows smoke 之前，UI 不得提前显示 `sandbox`。完成后的 Windows 档名为“写保护”，能力不等同 macOS“沙箱保护”。

- `platform_sandbox_available()` 非 macOS 恒 false → 即便某会话误发 `tier=sandbox`，`wraps_bash()` 也为假，bash 退回**手动审批档**行为（只读白名单免问 + 非白名单弹卡片审批），不裸跑无闸。
- **工具层规则照常生效**（判定在 agent 进程内，不依赖平台）——read/write/edit 的 ask/deny、凭证 ask、第 0 层写保护在 Linux/Windows 依然工作。差别只在 bash 没有 Seatbelt 硬拦：`cat ~/.ssh/id_rsa` 若不在只读白名单里会弹卡片，但用户批准后仍能读（无 OS 强制）。
- Linux bwrap 仍按"最后再做"排期：写侧 bind 白名单同构可编译；读侧 ask/deny 用 `--tmpfs`/`--ro-bind` 遮盖近似。届时可为 Linux 也开放"沙箱保护"档。
- **Windows 原生后端见 §11**：把 shell 从“白名单+命令卡片”升级到 RestrictedToken + ACL 的写边界；落地后提供“写保护”档，并使用具体路径 capability 审批，不宣称 shell deny-read。

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
- [x] 拆除 SQLite 规则链路（`list_effective_rules`/`prune_session_rules`/SQLite `save_approval_rule` 导出移除、启动 prune 移除、`set_sandbox_policy` 不再展平规则）；`approval_rules` / `sandbox_config` / `approval_policy_config` 三张死表 + `approval_config.rs` 已删除（2026-07-05）。
- 验收：`make check-desktop` + vitest(39) + tsc + eslint 全绿。
- **已知未做（R3 补）**：**当轮即时生效**（§6.2 内存注入）——写文件后 agent 下一轮 prompt 才重读。当前"本工作区允许"让本次操作通过（写走 `approve_outside_path`，读经审批放行），但同一轮内对该目录下**其他**文件的同类操作仍会再问一次。需给 agent 加 session 规则注入命令，留 R3。read 审批卡片沿用 file_write 模板渲染（够用）。

### Phase R3 — 敏感守卫 + 当轮即时生效 + 文档（2026-07-04）

- [x] **敏感文件守卫**（方案 A）：`builtin_guards` 层置于 overrides 之下、用户规则之上 → 敏感文件（`.env`/`*.pem`/`*.key`/`*.p12`/`id_rsa*` + home 凭证）**不可被规则覆盖**，只能“允许一次”；宽目录 allow（`config/*`）盖不住里面的密钥（Q1 修复）。`is_secret_path` + 单测（broad_allow_does_not_ungate_secret_in_dir 等）。
- [x] 敏感文件抑制持久化建议：`approval_shape` 命中 secret → `save_suggestion = None`（GUI 自动隐藏“本工作区允许”，只剩拒绝/允许一次）。Seatbelt profile 按新层序发射（守卫 deny 在高位）。
- [x] **当轮即时生效**：`RuleSet.session` 改 `Arc<Mutex<Vec<PathRule>>>`（`SessionRules`）；`ServerSession.session_rules` 每轮清空、`resolve_with_session` 共享进 live sandbox；`add_session_rule` gRPC 命令；GUI `save_approval_rule` 写文件后经 `inject_session_rule` 注入当前 agent 会话 → 本轮同目录后续操作立刻不再问。守卫压得住 session（密钥仍每次问）。
- [x] 文档：PRODUCT.md §4.6、ER.md §4.8 同步 v2；本文件 + APPROVAL_PLAN 更新。
- 验收：agent 58 lib + smoke 9 + GUI 72 + 前端 39 全绿；`make lint`/`check-desktop` 干净。
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

## 11. Windows 原生写保护（unelevated：RestrictedToken + ACL）

状态：**W1–W6 底层与本机可执行生命周期已在 Windows 11 Home 真机 PASS；W7 默认关闭的灰度接入已实现。** AppContainer SID、private desktop、PowerShell/CLM、Unicode/大输出、用户级单例、活动 capability lease、跨进程占用锁、旧代际 ACE/metadata GC、完整 host probe、reset、Desktop graceful shutdown 及 RPC/CLI 调用均已有自动化证据；RM-01、RM-02、RM-04～RM-07 已在本机 PASS。RM-03 及额外主机矩阵仍待完成。在这些发布闭环完成前，Windows 默认启动必须保持不可用，只有显式设置 `FUTURE_WINDOWS_SANDBOX_ROLLOUT` 的测试会话可显示该档。

Windows 与 macOS 共用 `SandboxTier::Sandbox` 协议值，但 UI 和保证不同：

| | macOS 沙箱保护 | Windows 写保护 |
|---|---|---|
| OS 后端 | Seatbelt profile（每命令、无状态） | `WRITE_RESTRICTED` token + capability SID + NTFS ACL（有状态） |
| 硬保证 | shell 读写路径规则 | shell 写路径边界 |
| 读取 | 敏感路径可由 Seatbelt deny | 普通用户读取保持开放；shell deny-read 不保证 |
| 网络 | 开放 | 开放 |
| `ask` 放行 | 现有命令级脱 Seatbelt重跑 | 具体写路径 capability；仍在 RestrictedToken 下重跑 |
| UI 名称 | 沙箱保护 | 写保护 |

规则引擎和 read/write/edit 工具审批继续跨平台复用；Windows 的差别只在 shell OS 强制层与 shell 能力审批。

### 11.1 选型：为什么是 unelevated

对照 Codex 三条腿（elevated 独立用户 + 防火墙 / unelevated 受限令牌 + ACL / WSL 复用 bwrap）与 Low-IL，按本项目约束（**读默认开、写白名单、网络全开、免管理员**）取 **unelevated**：

| 候选 | 免管理员 | 读默认开 | bash 拦密钥 | 与我们模型贴合 | 结论 |
|---|---|---|---|---|---|
| elevated（独立用户）| ❌ 要管理员/企业易崩 | ❌ 需补读 | ✅ 天然 | 低（还自带多余的网络隔离）| 暂缓 |
| **unelevated（`WRITE_RESTRICTED` + ACL）** | ✅ | ✅ 沿用当前用户正常读能力 | ❌ 不可靠参与 read AccessCheck | **高（接受只做写保护）** | **选它** |
| Low-IL（完整性级别）| ✅ | ✅ 天然 | ❌ 拦不住 | 中 | 备选 |
| WSL2 复用 bwrap | ✅ | — | ✅ | — | 兜底腿，排在 Linux bwrap 之后 |

关键差异 vs Codex：FutureOS 网络全开，不做防火墙、offline-user、本地账号 provisioning。与 macOS 的关键差异则是 `WRITE_RESTRICTED` 只适合构建兼容性良好的写边界，不能可靠复刻 Seatbelt deny-read。第一版接受这一差异并在 UI 明示，不用提示词或 stderr 猜测冒充 OS 读保护。

### 11.2 强制模型

`CreateRestrictedToken` 使用 `DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED` 派生 token。写访问必须同时通过当前用户正常 SID 与 restricting SID/capability SID 的检查；读取保持当前用户兼容性。规则映射如下：

```
token = 当前用户 primary token
flags = DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED
restricting SIDs = {本次有效写根对应的 capability SIDs}
  → 读：保持当前用户读取兼容性，不声称 capability SID deny-read 生效
  → 写：正常用户检查 ∧ restricting SID 检查；没有 capability write ACE 就失败

写白名单 = 每个“规范化写根 + 有效规则指纹”分配 capability SID，并给该 SID 加 **写 ACE**，仅贴在：
  workspace  +  每会话专属 temp  +  用户 allow-write 规则路径

写保护 = workspace/session temp 之外默认不可写；对已存在的关键对象可附加 deny-write ACE，作为硬化而非完整规则保证

读规则 = read/write/edit 工具层继续强制；shell 层记入 unenforced diagnostics

网络 = 不动（v2 全开）
进程组 kill = CreateJobObject + JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE（对标 mac 进程组 kill）
```

deny-write ACE 必须压过 writable root 的 allow-write ACE。但 ACL 只能附着在已存在对象上：它不能在允许目录创建普通文件的同时，按一个未来文件名阻止尚不存在的 `.env`，也不能表达 glob。因此第一版**不声称 Windows shell 在 workspace 根内完整强制 ask/deny 规则**；现有字面对象的 deny ACE 只是额外硬化。若产品要求 workspace 内 shell 规则与 macOS 完整对齐，需要 minifilter/elevated broker，或改成所有 shell 写入都必须预声明的低兼容模式。

不能只按写根永久复用一个 SID：workspace 根 SID 若在 `.env` 等 `ask` 子路径上命中 deny ACE，Windows deny-wins 意味着再给 token 增加一个更窄的 allow SID 也无法重新开放该路径。FutureOS 第一版因此拒绝显式 `ask` carveout 的 shell capability，而不是展示一个批准后仍会失败的对话框。workspace/temp 外、由默认 fallback `ask` 产生且不与显式 carveout 冲突的具体目标，仍可按以下 request SID 精确批准：

- **基础计划**：SID 键为“规范化写根 + 当前 deny/ask carveout 规则指纹”。同一策略代际可稳定复用；token 只装入当前代际 SID。
- **仅允许这一次**：本次命令为每个基础写根和每个批准目标生成 request-scoped ephemeral SID；批准目标保留独立 SID，不能因父根去重而消失。capability identity 同时绑定 request id、路径和 `file/subtree` scope，避免历史 subtree ACE 被同名 file 批准复用。
- **此项目以后都允许**：仅对可表达的外部具体目标先持久化规则，再生成新的规则指纹和基础 SID；不得删除或改写仍可能被旧进程使用的旧代际 deny ACE。
- 旧代际和一次性 SID 的残留 ACE 因未来 token 不再包含对应 SID 而不产生新授权；后台 GC/卸载清理负责回收，但安全性不依赖命令结束时同步清理成功。

SID 映射与代际文件属于应用安全配置，必须原子写入、限制普通子进程修改并接受启动完整性检查。

Codex 的 legacy unelevated 后端还把 logon SID 与 Everyone 放入 restricting SID 集以兼容部分 Windows 对象；其包含 token User SID 的 helper 明确用于 elevated 后端的独立沙盒账号。Codex 另外把 logon、Everyone 与 capability 写入 token default DACL，并在交互 window station 中创建只授予 logon SID 的私有 desktop。真机验证确认：仅 capability SID 不足以初始化 Windows PowerShell（CLR 在进程启动后以 `E_ACCESSDENIED`/`HRESULT 80070005` 退出），而 capability + logon SID + Everyone 可以启动；真实 User SID 会命中普通用户文件 ACL，明显破坏 workspace 外写边界，因此不得加入 `SidsToRestrict`。FutureOS 暂时沿用 Codex 的 capability + logon + Everyone restricting 集，但把新建对象的授权收得更窄：restricted token 的 default DACL 与 `Winsta0` 私有 desktop 都仅包含“当前用户 + capability”，足以通过普通与 restricting 两次检查，不把对象开放给 Everyone。真机同时确认此普通用户进程调用 `CreateWindowStationW` 返回 `ERROR_ACCESS_DENIED`，因此独立 window station 方案不能作为当前后端的兼容基线。需要明确的是，logon/Everyone 一旦进入 restricting 集，任何既有文件对象若本身授予这些 SID 写权限，第二道检查就可能通过；所以现阶段不能绝对宣称所有 workspace 外写入只由 capability 决定。发布前必须补测 capability + logon、capability + Everyone 的最小启动组合，并对 Everyone/logon 可写的外部 NTFS 目录做逃逸矩阵；若无法移除宽 SID 或可靠拒绝这类 ACL，产品说明与支持探测必须保留该限制，或改用 broker/minifilter/elevated 独立账号方案。

### 11.3 能力边界与明确取舍

| 规则/能力 | Windows shell 第一版 | 说明 |
|---|---|---|
| workspace、session temp 写 | OS 强制 allow | 基础 writable roots |
| workspace 外写 | OS 默认拒绝 | 必须经 §11.4 申请具体路径 capability |
| allow-write 字面路径/子树 | OS 强制 allow | 编译为 capability SID write ACE |
| workspace 内 ask/deny-write | **不作完整 shell 强制保证** | 已存在字面对象可加 deny ACE 硬化；不存在对象、未来文件名和 glob 无法由目录 ACL 精确表达。工具层仍完整强制 |
| read ask/deny | **不由 shell OS 后端强制** | 工具层仍完整强制；UI 明示差异 |
| glob read/write | shell 第一版不强制 | 分别计入 `unenforced_read_rules` / `unsupported_write_globs`；工具层仍强制 |
| 网络 | 开放 | 与 macOS v2 一致 |

不采用“完整 restricted token + 大量 read ACE”路线：它虽然可以限制读取，却会显著破坏 `%APPDATA%`、Cargo、Rustup、npm、Git、Python 和用户安装工具链，且需要广泛修改真实目录 ACL。第一版优先保证可验证的写边界。

### 11.4 Windows 路径能力审批

RestrictedToken/NTFS 不向父进程可靠报告“刚才拒绝了哪个对象路径”。因此 Windows 禁止仅凭 `Access is denied` / error 5 弹出整命令放行；shell 必须在调用前声明额外写能力：

```json
{
  "additional_permissions": {
    "write": [
      { "path": "D:\\release", "scope": "subtree", "reason": "创建发布产物和符号文件" }
    ]
  }
}
```

处理流程：

1. agent 将路径解析为绝对路径并 canonicalize。第一版 `file` 只接受已存在普通文件，`subtree` 只接受已存在目录；创建、替换或 rename 必须由调用方明确申请已存在父目录 `subtree`，不得在后端静默扩大。拒绝 device/reparse、卷根和用户 Home 根。
2. 同一 `RuleSet` 分别评估每个 `(path, write)`；`allow` 不询问，`ask` 合并为一张路径能力卡，`deny` 直接拒绝且不可审批覆盖。
3. agent 根据实际授权范围生成可信的“行为 + 目标”语义。单目标直接组成标题；多目标完整列出且最多 8 项。原始/规范化路径、ACL 根、规则层、backend、hash 等只进入内部 payload，不进入普通用户界面。
4. GUI 复用 macOS 的 `ApprovalPrompt`。普通卡片只显示标题、决策按钮和折叠的“查看命令”；不重复显示行为/目标/用途，不展示或编辑 glob、ACL、SID、规则层等技术结构。
5. “仅允许这一次”把冻结的路径集合对应 SID 加入本次 token；“此项目以后都允许”保存同一行为和目标为 allow-write 路径规则并注入 session。两者都重新校验规则、重新编译计划，命令始终在 RestrictedToken 下运行。
6. approval 绑定 `request_id + command hash + normalized paths + scopes`。GUI 对整组批准或拒绝，不允许原地增删/改范围；任何变化必须生成新请求。ACL 应用前再次按 handle 校验 final path，TOCTOU/reparse 变化使批准失效。
7. 若可信行为或目标无法生成、payload 类型不匹配或解析失败，审批 fail closed，不显示批准按钮。

若命令未声明能力而越界，命令失败并提示模型携带具体路径重试。stderr 中抽取的路径只能作为 `suspected path` 帮助重试，不能直接授权。Windows 第一版不提供“整条命令脱写保护运行一次”；确需完全放开由用户显式切换 `off` 档，不能藏在普通路径审批中。

批准只增加 restricting/capability SID 的写许可，不提升当前用户自身权限。若当前用户对目标无写权限，即使批准也必须失败；unelevated 后端不得改 owner、覆盖无关 DACL 或触发 UAC。

工具层 read/write/edit 继续使用现有真前置审批；Windows shell capability 卡片与其复用同一用户语义和共享组件，详见 APPROVAL_PLAN §6。Windows“只保护写入，读取和网络开放”的模式差异放在模式选择、首次启用说明和设置页，不在每次具体路径审批中重复。

### 11.5 代码接缝

执行层拆分为：

- `windows_plan.rs`：纯规则投影，字段为 `writable_roots`、`write_carveouts`、`unenforced_read_rules`、`unsupported_write_globs`；`write_carveouts` 保留 `ask` / `deny` 决策，避免后续审批把不可覆盖的 deny 当成 ask。
- `sandbox/windows/capability.rs`：用“规范化写根 + 规则指纹”生成稳定 capability name，再从该名称确定性构造仅供 FutureOS ACL trustee 与 `WRITE_RESTRICTED` 使用的 account-domain-shaped SID（不使用 AppContainer `DeriveCapabilitySidsFromName` SID）；一次批准为基础根和每个批准目标分别生成 request-scoped capability，identity 绑定 request id、规范化路径与 `file/subtree` scope。状态文件只持久化名称、语义及实际写入 deny ACE 的 carveout 路径，不持久化进程内 SID 指针，并采用同目录原子替换；只向 token 装入当前有效代际/请求 SID，旧 ACE 不等于当前授权。
- `sandbox/windows/token.rs`：创建并验证 `WRITE_RESTRICTED` primary token；restricting 集含 capability + logon SID + Everyone，但不加入真实 User SID（避免普遍命中用户文件 ACL）。恢复 `SeChangeNotifyPrivilege` 以允许路径遍历，并把 token default DACL 收敛为当前用户 SID + capability SID，使子进程自建内核对象同时通过普通与 restricting 两次检查，但不把对象授予兼容性宽 SID，也不修改任何已有文件 ACL。
- `sandbox/windows/acl.rs`：通过已冻结的对象 handle 和 `SetSecurityInfo` 幂等增加 capability allow-write ACE 与受保护子路径 deny-write ACE；保留原 DACL，只操作 FutureOS 自己的 SID/ACE，不设置 owner、不整体覆盖 DACL，也不向父目录授予会绕过受保护子项的 `FILE_DELETE_CHILD`。回收沿用 Codex 的 `REVOKE_ACCESS` 模式，但只对 FutureOS 确定性派生的 capability SID 执行。
- `sandbox/windows/audit.rs`：只接受绝对本地 NTFS 路径，以 `FILE_FLAG_OPEN_REPARSE_POINT` 打开并拒绝 reparse target，校验 handle final path 与已规范化审批路径完全一致；启动和按需检查 SID 映射/代际、关键 ACE 和异常宽写 ACL。可修复自身缺失 ACE 并 GC 不再被活动进程引用的旧 SID，但不得改 owner 或覆盖无关 DACL。
- `sandbox/windows/process.rs`：每次启动在交互 `Winsta0` 上创建 UUID 命名的私有 desktop，DACL 只含当前用户 SID + 本次 capability SID（真机上使用自定义 station 名称的 `CreateWindowStationW` 对普通用户返回 `ERROR_ACCESS_DENIED`，故不把独立窗口站作为当前兼容基线）。随后以 `CreateProcessAsUserW(CREATE_SUSPENDED)` 配合 `STARTUPINFOEXW` handle allowlist，只继承 stdin/stdout/stderr；加入不允许 breakaway 的 Job Object后再 `ResumeThread`。失败时在恢复线程前终止进程；封装 Unicode command line/environment、cwd、stdout/stderr、wait 和整棵 Job terminate。restricted shell 正常退出后也关闭残留后代，不复用 unsandboxed 模式允许 detached browser 的 `BREAKAWAY_OK` / `disarm` 行为。每个 child 同时持有 capability lease；首个 lease 还持有同目录 byte-range 文件锁，Windows 在进程崩溃时自动释放。只要本进程或另一个 agent 的 Job 尚可能使用某 SID，GC/reset/卸载就不会撤销对应 ACE。
- `sandbox/mod.rs`：将“构造 `tokio::process::Command`”升级为可承载 Windows 自定义 spawn driver 的 `spawn_shell` 抽象；approved capability 作为显式输入。
- `agent/src/tools/mod.rs`：shell 参数增加 `additional_permissions.write[]`，执行前完成路径冻结、规则评估和 capability 请求；不解析 PowerShell 猜权限。
- `agent/src/rpc/approval.rs`、`packages/rpc/proto/future.proto`、`packages/thread-projection/src/approval.ts`：传递可信用户语义和内部 enforcement payload，批准回执绑定 request/hash/path/scope。
- `desktop/src/features/agent/ApprovalPrompt.tsx`：复用现有共享卡片，增加 Windows capability 变体并同步收敛普通审批 UI；解析失败不再回退到可批准的原始 JSON。
- `mobile/src/components/TimelineCard.tsx`、`mobile/src/remote/types.ts`：移动端原生审批卡消费同一可信语义 payload，显示相同“行为 + 目标”；第一版只支持拒绝/允许一次，malformed payload 同样 fail closed。
- GUI 设置：Windows 显示“写保护”及其一次性能力说明；后端完成并通过 smoke 后才让 `platform_sandbox_available()` 返回 true。

### 11.6 已知取舍 / 缺口（知情接受）

- **shell deny-read 不保证**：不只是 glob；Windows write-protect 后端整体不声称拦截 shell 读取。工具层仍按规则审批。
- **glob 不可表达**：NTFS ACE 不表达 glob；shell write glob 第一版不强制，必须改成具体路径/subtree capability。
- **不存在对象不可按名称 carve out**：父目录拥有 `FILE_ADD_FILE` 时，普通 NTFS DACL 不能只拒绝某个尚不存在的未来子项名称。第一版保证的是 workspace/temp 外部写边界与精确外部 capability，不保证 workspace 内 shell 的未来 `.env`/规则文件名保护；模式说明必须明确，不能借用 macOS 的保证。若此取舍不可接受，Windows 产品入口必须继续关闭并升级为 broker/minifilter 方案。
- **NTFS 恒 deny-wins**：`windows_plan` 把 deny 路径与可写子树分开收集，deny ACE 永远赢——无法表达"高优先级 allow 盖低优先级 deny"（SBPL 靠 last-match 可以）。极少数该场景会**偏严**（多一次 escalation），不造成安全洞，errs safe。
- **`FILE_DELETE_CHILD` 不受 `WRITE_RESTRICTED` 限制**：`WRITE_RESTRICTED` 只对写数据/追加/新建子项/`DELETE` 做第二道 restricting-SID 检查，删除文件实际走的父目录 `FILE_DELETE_CHILD` 不在其列。因此 `scope=file` 只授 `FILE_GENERIC_WRITE`（不授 `DELETE`），只能阻止改文件内容，**不能阻止删除该文件**——只要当前用户对父目录有普通删除权，受限 token 就仍能 `Remove-Item`。真机 AccessCheck 已验证 `FILE_DELETE_CHILD` 在 external 目录上返回 true、`DELETE`/`FILE_WRITE_DATA` 返回 false。这是 write-protect 模型的知情限制，不是可修复的 ACL 缺陷；文档与测试均不得声称 file scope 阻止删除。
- **有状态**：写授权修改真实 NTFS ACL。基础策略代际 SID 和幂等 ACE 避免普通命令反复增删；workspace/temp 外的具体目标批准使用 request-scoped SID。并发创建代际/SID/确保 ACE 必须按规范化根串行或事务化；GC 必须知道活动 token/Job 引用，不能提前删除旧代际 deny。卸载/重置提供独立清理流程，安全性不依赖普通命令结束时同步回滚成功。
- **单例与退出回收**：长驻 Agent 持有 `~/.future/agent/agent-instance.lock` 的用户级文件锁，不能通过改 gRPC 端口启动第二份共享同一状态的 Agent；probe/reset 等无服务维护命令不占该锁。桌面正常退出、强制退出确认后的任务收敛、清数据重启、环境切换和更新重启，都会在 bundled Agent 仍存活时先请求 reset，再终止子进程；独立 Agent 的 Ctrl+C 正常退出也会幂等 reset。桌面不清理或终止外部管理的 Agent。该步骤有超时且是 best-effort：崩溃、强杀、活动 lease 或临时失败时保留元数据，由下次启动 GC、设置重置和卸载流程继续回收。
- **既有宽写 ACL**：若目标目录本身对 Everyone/相关 restricting SID 可写，可能削弱默认写边界；启动诊断与 smoke 必须覆盖，不能仅靠假设。
- **网络与读取开放**：任何 shell 可读取当前用户能读的文件并经网络外传；这是 Windows 写保护的明确非目标，在模式选择、首次启用说明和设置页持续说明，不塞进每次路径审批卡片。
- **shell**：Windows 使用现有 pwsh 7 / Windows PowerShell 5.1 选择逻辑；不再假设 Git Bash/WSL。
- **CLM 下的输出编码**：受限 token 使 Windows PowerShell 5.1 进入 Constrained Language Mode，而 CLM 禁止 `[Console]::OutputEncoding` 的 setter（也禁止 `GetBytes`/`OpenStandardOutput().Write` 等 .NET 方法），因此 5.1 的 stdout/stderr 只能用其控制台输出代码页输出，wrapper 里设置 UTF-8 均无效。stdout 被重定向到管道（无控制台）时，`[Console]::OutputEncoding` 回退到 **OEM 代码页 `GetOEMCP`**（而非 ANSI 代码页 `GetACP`），故捕获端用 `MultiByteToWideChar(CP_OEMCP)` 解码（`decode_restricted_shell_output`：5.1→OEM、pwsh 7→UTF-8，pwsh 7 硬编码 UTF-8 不受影响）。CJK 区域 ACP 与 OEMCP 相同（如简体中文均为 936/GBK），但西欧/俄文/希腊文不同（1252 vs 437/850、1251 vs 866、1253 vs 737），必须用 OEMCP 才能跨区域正确解码。native 子进程的管道输出同样经过 PowerShell 的 OEM 解码-重编码，`chcp 65001` 在 stdout 为 pipe（无控制台）时本就不生效，因此不引入额外不一致。
- **elevated（独立用户强隔离）暂缓**：留给将来"要独立安全主体"的企业诉求。

### 11.7 实施阶段

实现按可独立验证、默认关闭的纵向阶段推进。每个阶段都必须在上一阶段的退出条件满足后再扩大产品可见范围；不得为了联调提前让 Windows `platform_sandbox_available()` 返回 true。

| 阶段 | 实现内容 | 验证与退出条件 |
|---|---|---|
| **W0 — 契约与威胁模型冻结** | 本节和 APPROVAL_PLAN §6 作为实现契约；冻结第一版仅支持本地 NTFS 字面路径/子树、读和网络开放、deny 不可审批、无 error 5 整命令放行；列出用户语义字段与内部 enforcement 字段 | 文档无 macOS/Windows 保证混用；协议草案能表达行为、目标、scope、稳定请求绑定；UNC、非 NTFS、无法解析 reparse 明确拒绝而非静默降级 |
| **W1 — 纯计划生成器返工** | 重构 `windows_plan.rs`：输出规范化写根、带原始决策的 `write_carveouts`、`unenforced_read_rules`、`unsupported_write_globs`；移除旧 `deny_read` 可强制的假设 | 平台无关单测覆盖规则优先级、workspace/temp、外部 allow/ask/deny、glob 诊断、受保护文件、路径去重/包含关系；macOS/Linux CI 可运行；纯计划不碰 ACL |
| **W2 — capability、token 与 ACL 原语** | 新增“写根 + 规则指纹 → 基础 SID”代际存储、request-scoped ephemeral SID、`WRITE_RESTRICTED` token、幂等 allow/deny ACE、handle-based final-path 校验和启动 audit；先做独立测试工具，不接 shell/UI | Windows `AccessCheck` 矩阵证明：无 root SID 时外部写失败；当前代际 SID 只开放对应根；显式 ask/deny carveout 失败且不尝试用窄 SID 覆盖 deny；外部批准只开放精确目标；旧 SID/ACE 未装入 token 时无效；规则变化时新旧代际并存且旧进程权限不变；正常用户本无权限时仍失败；ACL 继承关闭、已有 DACL、并发创建、重启后映射均可预测。额外验证“仅 capability SID”与“增加 Logon/Everyone”的兼容/安全差异后冻结最小 SID 集 |
| **W3 — 受限进程 driver** | `CreateProcessAsUserW(CREATE_SUSPENDED)`，先加入禁止 breakaway 的 Job Object 再恢复；接通 cwd/env/stdio/TTY、wait、timeout、cancel、kill；封装为 `spawn_shell`，保留现有 PowerShell 7 → 5.1 选择 | PowerShell 7 和 5.1 smoke 覆盖单命令、管道、重定向、子进程、退出码、UTF-8/Unicode 路径、大输出、超时和取消；子进程不能脱离 Job/token；初始化任一步失败时命令从未恢复执行；现有 Job Object 代码通过真实 Windows 测试而非仅编译 |
| **W4 — shell 能力请求与审批核心** | shell schema 加 `additional_permissions.write[]`；路径冻结、规则评估、请求去重、最多 8 项、可信“行为 + 目标”生成；RPC 持久化内部 enforcement payload；批准回执绑定 request/command hash/path/scope；一次批准选择 request-scoped SID，持久批准写 workspace 规则并切换到新策略代际 | 单测/集成测试覆盖 allow 无弹窗、默认 fallback ask 前置弹窗、显式 ask carveout 因 deny-wins 在弹窗前拒绝、deny 不可覆盖、未声明越界只失败、过宽/过多/非法路径拒绝、file 不静默扩大、创建/rename 使用父目录、过期/篡改 hash 拒绝、reparse/TOCTOU 变化失效、多目标全有或全无、session 即时生效 |
| **W5 — 共享审批语义与 UI** | 桌面在现有 `ApprovalPrompt` 增加 capability action，不新建 Windows 对话框；移动端 `PendingApprovalCard` 消费同一可信语义 payload。单目标只显示“行为 + 目标”标题，命令折叠；多目标完整列出；桌面按场景显示“不允许 / 仅允许这一次 / 此项目以后都允许”，移动端第一版只提供拒绝/允许一次；移除普通卡片的 raw glob 编辑和技术字段 | desktop/mobile 组件、投影和远程 payload 测试覆盖 Windows/macOS/手动档；普通 UI 不出现 ACL、SID、backend、hash、规则层或原始 JSON；模型 reason 不能覆盖可信标题；malformed payload 不显示批准按钮；敏感/过宽/macOS escalation 不出现持久允许；手机批准与桌面“一次允许”绑定同一 request/hash/path/scope；中英文文案结构一致，具体措辞可后续微调 |
| **W6 — 端到端安全与兼容性** | 将 W1–W5 串入真实 agent/desktop；增加 ACL audit/repair、活动代际跟踪、旧 SID GC、重置/卸载清理、日志脱敏和 feature probe；用仓库脚本进行 Windows 真实机手工批量 smoke，不加入 CI | 必过矩阵：workspace/temp 写成功；workspace 外写失败；一次批准仅开放完整列出的现有 file/subtree，未批准 sibling 仍失败；项目批准仅当前项目和新代际生效；父目录不被扩大；当前用户无权目标仍失败；现有关键对象 deny 硬化有效；不存在对象/glob 缺口与模式说明一致；崩溃/强杀/重启后无权限扩张；常见工具链可用；所有初始化失败均 fail closed |
| **W7 — 灰度与发布** | 默认关闭 feature flag；仅对通过 capability probe 的本地 NTFS 工作区显示“写保护”；模式选择/首次启用/设置页说明“只限制写入，读取和网络开放”；保留一键退回 `manual` | Windows 目标版本手工 smoke 全绿；安全 review 无高优先级问题；升级、降级、重置 SID/ACL 可恢复；遥测不上传原始敏感路径；完成小范围灰度后才默认开放。发布后发现初始化/ACL 异常时自动回到 `manual`，不得无提示直跑 |

**当前实现状态（2026-08-24）**：Windows 11 Home 非管理员主机上的 `test-windows-sandbox.ps1 -IncludeClippy` 已通过 50 项原生/端到端测试、11 项 capability 审批测试、Agent 用户级单例、2 项 Desktop graceful shutdown、Agent/Desktop Clippy、release CLI probe，并确认退出后 capability record 为 0；测试明确不加入 CI。统一用户目录后的提交 `471a8cd7` 已再次通过 Windows 原生行为矩阵，P0 正式完成。P1 的 RM-01、RM-02、RM-04～RM-07 已在本机 PASS；RM-03 因当前包无法触发而 `NOT RUN`，P2 额外主机矩阵因无可用主机延后。W7 开发已接入默认关闭的 `FUTURE_WINDOWS_SANDBOX_ROLLOUT` 进程闸门；只有闸门开启且完整 host probe 通过时，Desktop/手机才显示 sandbox 档并让 Agent 运行受限命令。probe/Agent 失败会 fail closed 并将已保存的 sandbox 档回退为 `manual`；日志仅使用稳定 code，不向产品状态输出原始路径。回收顺序仍为先持久化并应用新集合，再按 Codex 的 `REVOKE_ACCESS` 模式回收无活动引用的旧 SID；失败时保留 metadata 供以后重试。reset/probe 已提供 sessionless agent RPC 与 `future agent` 维护命令；Windows 设置页提供普通用户文案的手动 reset，NSIS 卸载清理已真机 PASS。正式默认入口继续保持关闭，RM-03、P2 和安全 review 仍是发布门槛。普通 NTFS DACL 仍无法精确拒绝允许目录内尚不存在的未来文件名，`FILE_DELETE_CHILD` 与 Everyone/logon 宽 ACL 也保留 §11.6 的已知边界，不能宣称与 macOS SBPL 等价。

Windows 真机从仓库根目录用普通（非管理员）PowerShell 执行：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\test-windows-sandbox.ps1
```

完整的执行前提、结果判定、安装包生命周期场景、主机矩阵和反馈模板见独立手册 [`WINDOWS_SANDBOX_REAL_MACHINE_VALIDATION.md`](WINDOWS_SANDBOX_REAL_MACHINE_VALIDATION.md)。该手册是 Windows 真机验收的唯一操作清单；本文件保留设计契约与实现边界。

脚本要求 `%TEMP%` 位于本地 NTFS，强制 `--test-threads=1`，并通过 `future agent --probe-windows-sandbox` 再验证安装包 sidecar 使用的真实 CLI 路由；记录系统/Rust/Git 信息与完整测试输出到 `target/windows-sandbox-results/windows-sandbox-<时间>.log`。失败时把该日志原样反馈；不要只截最后一行。可选 `-IncludeClippy` 追加 Agent Clippy，但它不属于原生行为验收。测试不包含 UI，也不接入 CI。

发布/维护入口同样必须以普通用户运行：

```powershell
future agent --probe-windows-sandbox
future agent --reset-windows-sandbox
```

probe 成功执行时始终输出 JSON；`available=false` 是受支持的 fail-closed 结果而非崩溃。reset 只撤销状态文件中记录的 FutureOS capability ACE，存在活动 sandbox Job 时返回失败，不会终止用户命令。设置页只消费 `available` 和稳定 `code`；内部 diagnostic 仅写本机 agent 日志，不进入普通用户文案。

### 11.8 开发切分与合并顺序

建议按以下独立变更提交，所有中间状态都保持功能开关关闭：

1. **PR W1**：只改纯计划和单测，修正文档已指出的 deny-read 漂移。
2. **PR W2**：Windows capability/token/ACL 原语与 AccessCheck 测试程序，不接产品执行路径。
3. **PR W3**：受限 spawn driver 与 Windows 进程生命周期测试。
4. **PR W4**：shell 参数、approval/RPC 契约、规则注入和后端绑定测试；桌面尚不展示入口。
5. **PR W5**：共享 `ApprovalPrompt` 和本地化/投影测试；同时收敛 macOS/手动档的普通用户呈现。
6. **PR W6**：端到端接线、Windows 手工批量 smoke、audit/repair/清理和隐藏设置开关。
7. **PR W7**：灰度开关与正式平台可用性判断。

W2 与 W4 可在 W1 契约冻结后并行开发，但 W4 的批准结果不得在 W2/W3 验证完成前启动真实命令；W5 依赖 W4 的可信语义 payload；W6 必须同时通过 W2、W3、W4、W5 的退出条件。

### 11.9 第一版完成定义

只有同时满足以下条件，Windows unelevated 后端才算完成：

- 用户看到的每个审批都有后端生成的明确“行为 + 目标”，且批准内容与实际 capability 完全一致。
- 不批准时，shell 只能写 workspace、session temp 和既有 allow-write 根；批准不会扩展到 sibling、父目录或其他项目。
- restricted token 及其全部后代始终处于 Job Object 和写边界内；任何初始化、解析或校验失败都不执行命令。
- 读和网络开放的取舍只在模式说明中准确呈现，不伪装成 macOS 等价隔离，也不反复塞进具体审批卡片。
- Windows 真实环境兼容性、ACL 恢复/重置、升级降级和失败回退均有自动化或可重复的 smoke 证据。

**明确不做（第一版）**：shell deny-read、glob ACE、elevated 独立用户、防火墙/网络隔离、WSL 捆绑、笼统 error 5 整命令脱沙盒放行。

---

## 附录 A：Windows 真机调试踩坑记录

> 记录 2026-08-21 在 Windows 11（10.0.26200，x86_64）真机上把 unelevated 写保护后端从「可编译」跑到「`test-windows-sandbox.ps1` 全绿」过程中踩过的坑。按调试时间顺序排列，每个条目给出症状、根因与修复。这些大多是无法从代码静态看出的 Win32 运行时行为，值得给后续维护者留下现场证据。

### A1. 使用自定义名称的 `CreateWindowStationW` 对普通用户返回 `ERROR_ACCESS_DENIED`

- **症状**：测试矩阵 8 个进程类用例全部 `拒绝访问 (os error 5)`，分步打点后定位在 `PrivateDesktop::create` 的 `CreateWindowStationW`，而非 `CreateProcessAsUserW`。
- **根因**：初版向 `CreateWindowStationW` 传入 FutureOS 自定义 station 名称；Win32 契约明确只有 Administrators 可以指定名称，并不存在 `SeCreateWindowStationPrivilege`。传空名称虽可让系统按 logon session 生成名称，但不能提供我们需要的每进程 UUID 隔离语义。
- **修复**：参考 Codex `windows-sandbox-rs/src/desktop.rs`，改为在交互 `Winsta0` 上创建 UUID 命名的私有 desktop，只授「当前用户 SID + capability SIDs」ACL；删除 `CreateWindowStationW`/`SetProcessWindowStation` 及配套互斥锁。`Winsta0` 自身 DACL 已授予普通用户 `CreateProcessAsUserW` 挂载子进程所需的读取权。

### A2. 仅 capability SID 无法初始化 Windows PowerShell（CLR `E_ACCESSDENIED`）

- **症状**：修好 A1 后进程能创建，但 PowerShell 主 CLR 初始化即失败，stderr 解码后是 `HRESULT 80070005`（`E_ACCESSDENIED`），命令从未执行。
- **根因**：PowerShell/CLR 启动时要对 session-scoped、Everyone 可访问的内核对象做写入；这些写入要同时过 `WRITE_RESTRICTED` 的第二道 restricting-SID 检查，而 token 里只有 capability SID，检查失败。
- **修复**：参考 Codex `windows-sandbox-rs/src/token.rs` 的 legacy 后端，把 **logon SID + Everyone** 加入 `SidsToRestrict`（**不**加真实 User SID——它会普遍命中用户文件 ACL）。当前真机 fixture 的 `access_check` 验证 external 目录 `FILE_ADD_FILE` 仍被拒；这不是全局保证，Everyone/logon 本身可写的既有 ACL 仍属于 §11.6 的明确限制。

### A3. Constrained Language Mode 下 wrapper 编码设置污染 `$Error`

- **症状**：受限 shell 里命令本应成功却 `exit 1`，stderr 出现 `CannotCreateTypeConstrainedLanguage`（CLIXML 序列化）。
- **根因**：受限 token 让 Windows PowerShell 5.1 进入 Constrained Language Mode，`[System.Text.UTF8Encoding]::new($false)` 这类 .NET 类型构造被拒；异常写进 `$Error`，wrapper 的 `elseif ($Error.Count -gt 0) { exit 1 }` 把成功命令误判为失败。
- **修复**：改用静态 `[System.Text.Encoding]::UTF8`；两个编码赋值各套一个 `try`；命令执行前先记录 `$Error.Count`，退出判定改为「增量 > 0」。

### A4. `FILE_DELETE_CHILD` 不受 `WRITE_RESTRICTED` 限制（设计限制，非 bug）

- **症状**：`file_approval_does_not_expand_to_parent_or_delete` 断言「file scope 批准后 `Remove-Item` 必须失败」真机失败——文件被删掉了。
- **根因**：`WRITE_RESTRICTED` 只对写数据/追加/新建子项/`DELETE` 做第二道 restricting-SID 检查；删除文件实际走的是**父目录**的 `FILE_DELETE_CHILD`，该权限只用普通用户 SID 检查，不参与 restricted 检查。`access_check` 实测：`FILE_DELETE_CHILD=true`、`DELETE=false`、`FILE_WRITE_DATA=false`。
- **修复**：这是 write-protect 模型的知情限制，无法用 ACL 修复（capability SID 的 deny ACE 也拦不住只用 normal SID 检查的删除）。删除测试里的「防删除」断言，改为只验证「file scope 不扩展到父目录/sibling」，并把限制写入 §11.6。

### A5. 受限 PowerShell 5.1 无法输出 UTF-8（CLM 双重封死）

- **症状**：新增的 `Write-Output '中文-stdout'` 断言失败，捕获到 `����-stdout`。
- **根因**：CLM 下 `[Console]::OutputEncoding` 的 **setter 静默失效**（赋值不报错但读回仍是 936/GBK）；`.NET 方法调用（`GetBytes`/`OpenStandardOutput().Write`/`WriteAllBytes`）也一律抛 `MethodInvocationNotSupportedInConstrainedLanguage`；`chcp 65001` 在 stdout 为 pipe（无控制台）时本就不生效。三条路全封，受限 5.1 只能按控制台输出代码页发出字节。
- **修复**：在**捕获端**按 shell 选择解码器——`powershell`（5.1）走 `MultiByteToWideChar`、`pwsh`（7）走 UTF-8（pwsh 硬编码 UTF-8，不受 CLM 影响）。这是唯一可靠的方案，PowerShell 端无解。

### A6. `CP_ACP` 还是 `CP_OEMCP`（跨区域兼容性）

- **症状**：中文系统上 A5 的修复「看起来」没问题，但在西欧/俄文/希腊文区域会解出乱码。
- **根因**：`[Console]::OutputEncoding`（`Console.OutputEncoding`）映射 `GetConsoleOutputCP()`；stdout 被重定向到管道（无控制台）时它回退到 **`GetOEMCP()`（OEM 代码页）**，而非 `GetACP()`（ANSI 代码页）。CJK 区域两者恰好相同（如简体中文均 936/GBK），掩盖了差异；西文 1252 vs 437/850、俄文 1251 vs 866、希腊文 1253 vs 737 均不同。
- **修复**：解码参数由 `CP_ACP` 改为 `CP_OEMCP`。注：OEM 代码页本身不含全部 Unicode，超出该页的字符在 PowerShell 端编码时已丢失为 `?`，这是 CLM 边界的必然产物，解码端只能保证「已发出的字节」被正确还原。

---

## 附录 B：Windows 真机测试报告

**执行方式**：仓库根目录普通（非管理员）PowerShell 运行 `scripts/test-windows-sandbox.ps1`。脚本强制 `--test-threads=1`，完整输出落盘 `target/windows-sandbox-results/windows-sandbox-<时间戳>.log`，显式不接入 CI。

**环境**（2026-08-21）：

- OS：Microsoft Windows NT 10.0.26200（x86_64）
- PowerShell：5.1（非管理员，`Elevated: False`）
- Rust/Cargo：1.97.0（repo `rust-toolchain.toml` 锁定）
- 工作区与 TEMP：本地 NTFS（脚本前置校验通过）

**结果**：`RESULT: PASS`

| 测试目标 | 结果 |
|---|---|
| `cargo test sandbox::windows`（原生 AccessCheck 矩阵 + 端到端 runner） | **44 passed, 0 failed** |
| `cargo test windows_capability`（审批语义与回执绑定） | **10 passed, 0 failed** |
| `cargo clippy --lib -- -D warnings`（Agent Clippy，非脚本默认项） | **通过，0 warning** |

**覆盖要点**：capability SID 只开放对应写根且 deny carveout 恒赢；restricted PowerShell 保留 cwd/env/stdout/退出码；Job Object 树形终止；workspace 写成功/未声明的外部写失败；subtree 一次性批准精确且旧 ACE 不可复用；file scope 不扩展到父目录；reparse/UNC 在 ACL 变更前 fail-closed；Unicode 路径 + 管道重定向 + 70 万字节大输出不卡死；受限 5.1 中文 stdout 按 OEM 代码页正确还原。

**该次报告未覆盖（后续代码已补但仍待真机复验）**：活动代际 GC、跨进程占用锁、reset、完整 release probe、NSIS 卸载清理，以及 Agent 用户级单例与正常退出权限回收。Agent 的正常、错误和 unwind 退出均由 RAII 清理守卫覆盖；明确强退/崩溃由下次启动回收。上述生命周期测试已加入同一个 Windows 手工脚本，具体步骤见 [`WINDOWS_SANDBOX_REAL_MACHINE_VALIDATION.md`](WINDOWS_SANDBOX_REAL_MACHINE_VALIDATION.md)；额外支持主机矩阵以及模式选择层仍未实现；`platform_sandbox_available()` 在 Windows 继续保持 false，产品入口关闭。
