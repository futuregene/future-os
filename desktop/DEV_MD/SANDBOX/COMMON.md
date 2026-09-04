# Sandbox：公共规则、审批与参考

更新：2026-09-04。本文与平台文档以当前 `sandbox` 工作区源码为基线；“已实现”不等于发布认证。历史测试只证明对应提交，不能自动覆盖后续修改。

## 1. 文档边界与平台概览

本目录只有四份主文档：本文维护公共语义、协议、决策和参考；[MACOS.md](MACOS.md)、[LINUX.md](LINUX.md)、[WINDOWS.md](WINDOWS.md) 各自维护实现、差异、验收、历史证据与计划。[产品说明](../PRODUCT.md#46-approval) 保留用户承诺，[数据结构](../ER.md) 保留持久化语义。

| 维度 | macOS | Linux | Windows |
|---|---|---|---|
| OS 后端 / 档名 | Seatbelt / 沙箱保护 | system Bubblewrap / 沙箱保护 | Unelevated RestrictedToken + NTFS ACL / 写保护 |
| shell 读保护 | SBPL 路径规则 | 启动时已有目标的遮罩 | **不提供** |
| shell 写保护 | SBPL 动态路径规则 | 只读 root + 可写/保护 mount | capability SID 写边界，受既有 ACL 与删除权限限制 |
| 缺失保护目标 / 新 glob | 路径出现后仍按 profile 匹配 | 缺失目标省略；新目标仅结束检测 | 不存在对象无法附 ACE；glob 不强制 |
| 额外授权 | 整命令出沙箱一次 | 整命令出沙箱一次 | 声明具体 write capability，批准后仍受限 |
| 网络 | 开放 | 开放，无 seccomp | 开放 |
| 可用性检查 | `sandbox-exec` 存在检查 | PATH / 版本与参数 / 实际 namespace-mount probe | 实际 token/ACL/private desktop/shell/cleanup probe |
| 进度 | v2 已实现，历史 smoke 通过 | L0–L4/L6 接入已实现；最新加固与 L5 发布矩阵待复验 | W1–W7 已接入，有原生及安装包历史 PASS；部分产品交互需逐项复验 |

注意：三个平台共用规则，不代表 OS 能表达同等规则。工具层审批也不等于任意 shell 子程序都受相同保护。

## 2. 启用、档位与执行链

产品目标排序：配置简单易懂、开发流程顺畅、安全性尚可；不是敌对宿主或 root 攻击者隔离方案。

| `tier` | 原生 read/write/edit 等工具 | shell |
|---|---|---|
| `manual`（默认） | 路径规则三态 | 只读白名单免问，其余命令先审批；无 OS 沙盒 |
| `sandbox` | 同一套路径规则 | 当前平台 OS 后端；授权语义见平台文档 |
| `off` | 不发审批 | 无 OS 包装直接运行 |

GUI 建立会话时通过 `set_sandbox_policy` 下发策略。未知 tier 按 manual；未下发策略的 TUI/CLI/channels 不自动启用本系统，保留各自 `permission_level`/工作区边界。无审批 UI 的调用方不能被假定能完成 GUI 交互。`off` 在 Agent 层放行，不靠前端自动点击批准。

```text
GUI policy + workspace → ResolvedSandbox + RuleSet
  ├─ native tool：规范化具体路径 → allow / ask 前置审批 / deny
  └─ shell：manual 命令审批 / off 直跑 / sandbox 平台执行器
审批事件 → RPC → Desktop 存储与推送 → Desktop/手机卡片 → 决策回 Agent
```

基础 probe 明确不可用时，sandbox 解析为 manual，工具规则仍在；Desktop 对明确结果保存回退，瞬时连接故障不应当作永久不支持。**manual 不是 OS 沙盒**，白名单命令或批准命令使用当前用户权限。真实命令初始化失败不等于基础 probe unavailable，不自动切 off、裸跑或重复执行。

单次工具失败作为 tool result/error 返回模型，对话可继续。当前不新增命令级自动降级、临时手动模式、熔断或恢复 UI；模型可以另行显式请求脱沙箱审批，不能自行批准，也不保证每次失败都能恢复。

## 3. 规则模型与持久化

```json
{
  "version": 1,
  "rules": [
    { "path": "dist", "access": "write", "action": "allow" },
    { "path": "~/notes", "access": "write", "action": "ask" },
    { "path": "private-data", "action": "deny" }
  ]
}
```

- 两个文件：`${WORKSPACE}/.future/approval_rule.json`、`~/.future/approval_rule.json`。Agent 每轮 prompt 读取，GUI 可信 Tauri 路径代写；用户可手改、进 git。
- `access` 为 read/write/both，省略为 both。action 为 allow/ask/deny。读写分别求值。
- 相对路径按 workspace 解析，用户规则建议使用绝对路径或 `~/`。无通配符匹配自身及子树；`*` 段内、`**` 跨段、`?` 单字符，不宣称支持全部 shell glob 语法。
- `paths.rs` 处理 `~`、`..`、最近存在祖先、symlink、路径组件边界及 macOS 大小写匹配。链接按最终目标判定；Linux/Windows 执行时还要复核对象，不能把一次 canonicalize 当作无竞态证明。
- 优先级：**overrides → guards → session → workspace 文件 → user 文件 → fallback**，层内按原书写顺序，首匹配返回。fallback 读 allow；写 workspace 和 `temp_roots()` 内 allow、外部 ask。
- temp 是代码实际解析的环境/系统临时根，不能泛称所有平台都有隔离的“每会话专属 temp”。`.git` 不从可写 workspace 排除。
- 整个规则文件缺失视为空；损坏/不可读层记录 `resolution_errors`，Linux sandbox 编译拒绝执行；其他路径仍按加载器已有层处理，不能宣称全平台同样 fail closed。个别缺 path/非法 action 的条目会被解析器跳过，**不存在统一 GUI 坏规则告警保证**。

### 3.1 内置不可覆盖层与凭据例外

| 路径 | 规则 |
|---|---|
| workspace 与 HOME 的 `.future/approval_rule.json` | write deny，允许读取 |
| `~/.future/agent/models.json`、`~/.future/agent-app/models.json` | read + write deny |

这保护的是规则判定与平台可强制的范围，不是跨平台“永远写不了”承诺：Linux/Windows 缺失规则文件创建存在已接受缺口；macOS/Linux 经用户批准的整命令脱沙箱也不受该 OS 规则限制。普通 `.future` 目录不能全封，因为 Chat workspace 等位于其中。

`auth.json`（agent/agent-app）为官方 `future` CLI 测试暂从 hard-deny 清单移除；**不是新增无条件 allow-write**，仍受其他规则和写根限制，但默认读取可能暴露凭据。路径沙盒无法只相信同命令里的官方 CLI 而不相信 `cat`。

后续候选：短时、作用域受限凭据或带 peer-credential 验证的 Agent RPC；完成后恢复 auth.json deny。验收须证明官方 CLI 正常认证、任意 shell 无法直接读写凭据、令牌不进入日志/历史/工具输出或不受控子进程环境。当前未排期，不把临时例外宣传为安全通道。

### 3.2 敏感守卫清单（read + write ask）

HOME 下：

| 类别 | 相对 HOME 路径 |
|---|---|
| SSH/GPG | `.ssh`、`.gnupg` |
| 包管理 | `.npmrc`、`.pypirc`、`.cargo/credentials`、`.cargo/credentials.toml`、`.gem/credentials` |
| 明文凭据 | `.netrc`、`.git-credentials`、`.env` |
| 云/编排 | `.aws`、`.azure`、`.config/gcloud`、`.terraform.d`、`.kube/config` |
| 容器/CLI/系统 | `.docker/config.json`、`.config/gh`、`Library/Keychains` |

workspace 下：`.env`、`.env.*`、`**/*.pem`、`**/*.key`、`**/*.p12`、`**/id_rsa*`。根 `.env.*` 不等同递归所有目录的 `.env.*`。

守卫高于 session/user/workspace allow，宽目录 allow 不解除守卫。原生工具敏感访问只提供“允许一次”，不持久允许。Linux 存在性差异、Windows shell 不拒读等不能从此清单直接推导出完整硬保护。

### 3.3 保存与当轮生效

Desktop `approval_rules.rs` 读改写整份 workspace 文件，保留未知字段、已有规则并去重。保存后 `inject_session_rule` / `add_session_rule` 注入当前共享 `SessionRules`，同轮后续调用立即看见；下轮重新从文件读取。这不是把所有“允许一次”升级为持久目录规则。敏感守卫仍压过 session。

## 4. 审批协议与界面

Desktop 复用 `ApprovalPrompt`，手机使用原生卡片但共享可信语义投影。审批持久化的是请求/决定，不是旧 SQLite 规则真源。推送与轮询负责恢复 pending 状态，不能只靠内存弹窗。

- 可信后端生成“行为 + 目标”标题；单目标不重复字段，多目标完整列出，Windows 单次最多 8 个目标，不用“另有 N 项”隐藏范围。
- 命令/文件预览放折叠详情；模型 reason/justification 是辅助原因，不能覆盖真实授权范围。
- 按场景显示不允许、仅允许一次、此项目以后都允许。持久允许只写明确路径规则；敏感路径、manual shell、macOS/Linux 整命令 escalation 不提供持久规则按钮。
- 普通卡片不展示/编辑 raw glob、ACL、SID、backend、hash 或原始 JSON。可信 payload 无法解析或类型不匹配时不显示批准按钮。
- 手机第一版仅拒绝/允许一次；不能把本地记忆决定冒充项目规则持久化。多目标整组决策，改变范围须新请求。

### 4.1 主动与被动脱沙箱（macOS/Linux）

| 触发 | payload trigger | 多语言 key | 中文标题 |
|---|---|---|---|
| 模型显式 `escalated: true` + justification | `model_request` | `approval.escalationRequestTitle` | 模型请求在沙箱外运行此命令 |
| 沙箱命令失败、命中拒绝分类 | `sandbox_failure` | `approval.escalationRetryTitle` | 需要在沙箱外运行此命令 |
| 历史请求缺失/未知 trigger | fallback | `approval.escalationTitle` | 在沙箱外运行此命令 |

批准的是**整条命令脱离 OS 沙箱执行一次**，不是只访问卡片路径；不改变全局 tier。拒绝则不执行该次脱沙箱命令。首次失败可能已有副作用，重跑前需核对结果。

诊断路径仅为展示推断：提取 `Operation not permitted`、Linux `Permission denied`/`Read-only file system` 中的绝对路径，支持引号、空格、`program: line N: /path: error`；去重保序最多 5 项。不猜相对 cwd、不把 URL/helper 初始化错误当目标，不保证所有语言/程序报错可解析。主动申请没有失败输出时可以没有路径。展示解析不改变 escalation 判定或授权范围，也不扩展持久规则建议来源。

Windows 不使用此整命令批准，详见 [路径 capability](WINDOWS.md#3-路径-capability-审批)。

### 4.2 协议与代码索引

`SandboxPolicy` 的 proto 字段 1–6 保留不复用，`string tier = 7`。旧三模式×三策略、命令前缀规则和 SQLite `approval_rules`/`sandbox_config`/`approval_policy_config` 已移除；三表及 `approval_config.rs` 于 2026-07-05 清理。

| 代码（仓库根相对） | 职责 |
|---|---|
| `agent/src/sandbox/{mod,rules,paths,backend}.rs` | 档位、规则、路径、PreparedShell 与 receipt |
| `agent/src/tools/mod.rs` | native 工具、shell 执行、超时/取消、重试 |
| `agent/src/rpc/{approval,session_prompt}.rs` | 审批请求、路径诊断、session 注入 |
| `agent/src/rpc/commands/settings.rs` | policy/probe RPC |
| `packages/rpc/proto/future.proto`、`packages/thread-projection/src/approval.ts` | wire 与共享审批投影 |
| `desktop/src-tauri/src/{approval_rules.rs,commands/approvals.rs,agent_bridge/}` | 保存、决策、连接与回退 |
| `desktop/src/features/agent/ApprovalPrompt.tsx`、`desktop/src/integrations/agent/useSandboxAvailability.ts` | 卡片与可用性 |
| `mobile/src/components/TimelineCard.tsx`、`mobile/src/remote/types.ts` | 手机原生卡片/远程数据 |

## 5. 进度、决策与后续计划

2026-07-04 v2 R1/R2/R3 已完成：文件规则、read 前置审批、Seatbelt 编译、敏感守卫、GUI 保存与当轮注入。历史结果：R1 Agent 55 lib + 10 规则 + 9 smoke；R2 GUI/前端 39；R3 Agent 58 lib + 9 smoke、GUI 72、前端 39，lint/check-desktop 通过。早期 v1 的 Agent 67、GUI 69、前端 39、smoke 9 仅是旧架构基线，不是当前总测试数。

保留的决策：V1–V9（2026-07）确立纯路径规则、网络开放、分车道 fallback、文件真源、三档与项目允许；V10 的“仅 macOS 显示”已被 V11–V14（2026-08 Windows unelevated、具体 capability、共享可信 UI、接受身份/删除限制）及 Linux L-D1–L-D11 取代。不再保留互相冲突的旧状态段落。

近期重点：平台当前版本原生复验、安全 review、文档承诺与执行器一致；Windows 与 Linux 各自验收清单见平台文档。明确暂不做命令前缀持久规则、网络审批/域名过滤、auto-review agent、通用 MCP/新工具沙盒规范、设置页完整 user 规则编辑器。手动只读白名单是免打扰机制，不是任意 shell 静态安全分析；批准后 `git push --force`、`npm publish` 等仍能执行。

### 5.1 macOS/Linux 二期 execution_grants（未实现）

将具体 read/write 路径、access、scope、command hash、request id 绑定到单次批准，重新编译**仍在 OS 沙箱中**的计划。不能简单复用 session allow：它低于 guards。候选优先级为 hard deny > 已批准 execution_grants > secret ask guards > session/workspace/user；只接收原判定 ask，任何 deny 和 layer 0 不可覆盖。

Seatbelt 可添加临时 literal/subpath allow；Linux 需证明 mount/reopen 能表达同一范围。Windows 的 request capability 可提供协议经验，但不能照搬其不拒读/ACL deny-wins 限制。先冻结跨平台模型再修改 UI；整命令脱沙箱若保留应为显式高级选项。此项与 macOS 一起推进，不混入 Linux 一期。

## 6. 参考资料与历史检索

参考仅说明取舍来源，**不代表 FutureOS 已实现参考项目全部能力**。本地 Codex 调研快照为 `~/workspace/codex` @ `f20b63e85c`（2026-09-02/04），后续复核应记录新 commit。

| Codex 仓库路径 | 备查内容与 FutureOS 取舍 |
|---|---|
| `codex-rs/sandboxing/src/manager.rs` | PermissionProfile、平台选择、helper 接缝；FutureOS 保留自己的 RuleSet |
| `codex-rs/sandboxing/src/bwrap.rs` | system 搜索、userns probe、WSL；FutureOS 不支持/不专门检测 WSL |
| `codex-rs/linux-sandbox/README.md`、`src/launcher.rs` | system/bundled、能力检测；FutureOS system-only，不下载或 bundle |
| `codex-rs/linux-sandbox/src/bwrap.rs` | 同根 glob 分组、窄重开、missing roots、writable symlink；FutureOS 内部 no-follow walker，不照搬外部 rg/files-only/globset |
| `codex-rs/linux-sandbox/src/linux_run_main.rs`、`landlock.rs` | outer/inner、PID 1、cap/no_new_privs、seccomp/网络策略、synthetic target cleanup；FutureOS 不采用 Landlock fallback 或宿主占位+清理 |
| `codex-rs/vendor/bubblewrap/{bubblewrap.c,utils.c}` | tmpfs setup 的 ensure_dir/mkdir：mount namespace 不隔离宿主目录内容写入 |
| `codex-rs/core/src/tools/{orchestrator,sandboxing}.rs` | typed SandboxErr::Denied 与批准后 retry；不等于任意初始化失败自动降级 |
| `codex-rs/sandboxing/src/{violation,denial}.rs`、`codex-rs/cli/src/doctor/sandbox.rs` | 诊断与拒绝判断 |
| `codex-rs/windows-sandbox-rs/src/{token,desktop}.rs` | legacy unelevated SID 兼容、私有 desktop；elevated User SID 属于独立沙盒账号，不能直接加入 FutureOS restricting 集 |

Codex 旧包兼容包括不支持 `--argv0` 时改用可执行路径、无 `--ro-bind-fd` 时使用 `/proc/self/fd/N` 并内层复核。FutureOS 明确子命令自重入，不依赖 argv0；FD-backed mount 复核是主路径安全措施，不是可删除的兼容负担。

上游资料（保留审查时版本，不作为“最新”声明）：

- [OpenAI Windows 沙箱设计](https://openai.com/zh-Hans-CN/index/building-codex-windows-sandbox/)：unelevated 与独立用户/elevated 的身份差异。
- [Bubblewrap v0.9.0 参数解析](https://github.com/containers/bubblewrap/blob/v0.9.0/bubblewrap.c#L1527)、[v0.11.1](https://github.com/containers/bubblewrap/blob/v0.11.1/bubblewrap.c#L1637)：`--args` 只递归解析 OPTIONS，合计 9000 参数限制。
- [GHSA-pxhw-h44j-8pfx](https://github.com/containers/bubblewrap/security/advisories/GHSA-pxhw-h44j-8pfx)：旧文档记录 0.12.0 修复 setup 绝对 symlink traversal；0.9.0 是兼容下限，不是安全补丁保证。FutureOS 已删除 missing-target 创建路径，但不能据此声称所有上游风险消失。

迁移索引：旧 APPROVAL_PLAN 与 SANDBOX_PLAN 公共内容收至本文，Seatbelt/Windows 章节分别入平台文档；五份 LINUX_SANDBOX 文档合入 LINUX.md；WINDOWS_SANDBOX_REAL_MACHINE_VALIDATION 合入 WINDOWS.md。旧逐轮 diff、被替代设计与原始长验收表可从 Git history 查阅，当前四文档保留有效契约、关键取舍和证据索引，不再平行维护旧稿。
