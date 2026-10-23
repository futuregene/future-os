# Sandbox：公共规则、审批与参考

更新：2026-09-07（默认值核对）。Linux 实现已合入 #496；平台文档保留各自候选版本的验收边界，“已实现”不等于发布认证。历史测试只证明对应提交，不能自动覆盖后续修改。

## 1. 文档边界与平台概览

本目录只有四份主文档：本文维护公共语义、协议、决策和参考；[MACOS.zh-CN.md](MACOS.zh-CN.md)、[LINUX.zh-CN.md](LINUX.zh-CN.md)、[WINDOWS.zh-CN.md](WINDOWS.zh-CN.md) 各自维护实现、差异、验收、历史证据与计划。[产品说明](../PRODUCT.zh-CN.md#46-approval) 保留用户承诺，[数据结构](../ER.zh-CN.md) 保留持久化语义。

| 维度 | macOS | Linux | Windows |
|---|---|---|---|
| OS 后端 / 档名 | Seatbelt / 沙箱保护 | system Bubblewrap / 沙箱保护 | Unelevated RestrictedToken + NTFS ACL / 写保护 |
| shell 读保护 | SBPL 路径规则 | 能力支持时遮罩启动时已有目标；仅写保护兼容模式不提供 | **不提供** |
| shell 写保护 | SBPL 动态路径规则 | 只读 root + 可写/保护 mount | capability SID 写边界，受既有 ACL 与删除权限限制 |
| 缺失保护目标 / 新 glob | 路径出现后仍按 profile 匹配 | 缺失目标省略；新目标仅结束检测 | 不存在对象无法附 ACE；glob 不强制 |
| 额外授权 | 单命令精确 read/write 授权，仍在沙箱 | 单命令精确 read/write 授权，仍在沙箱 | 声明具体 write capability，批准后仍受限 |
| 网络 | 开放 | 开放，无 seccomp | 开放 |
| 可用性检查 | `sandbox-exec` 存在检查 | PATH / 版本与参数 / 写保护和读保护独立运行探测 | 实际 token/ACL/private desktop/shell/cleanup probe |
| 进度 | v2 已实现，历史 smoke 通过 | L0–L4/L6 接入已实现；最新加固与 L5 发布矩阵待复验 | W1–W7 已接入，有原生及安装包历史 PASS；部分产品交互需逐项复验 |

注意：三个平台共用规则，不代表 OS 能表达同等规则。工具层审批也不等于任意 shell 子程序都受相同保护。

## 2. 启用、档位与执行链

产品目标排序：配置简单易懂、开发流程顺畅、安全性尚可；不是敌对宿主或 root 攻击者隔离方案。

| `tier` | 原生 read/write/edit 等工具 | shell |
|---|---|---|
| `manual`（可选） | 路径规则三态 | 只读白名单免问，其余命令先审批；无 OS 沙盒 |
| `sandbox` | 同一套路径规则 | 当前平台 OS 后端；授权语义见平台文档 |
| `off`（桌面默认） | 不发审批 | 无 OS 包装直接运行 |

GUI 建立会话时通过 `set_sandbox_policy` 下发策略。`SandboxTier` enum 的默认/未知值为 manual，但桌面持久设置的默认/未知值为 off，新 Agent 会话权限默认 all；不能混为产品默认。未下发策略的 TUI/CLI/channels 不自动启用本系统，保留各自 `permission_level`/工作区边界。无审批 UI 的调用方不能被假定能完成 GUI 交互。`off` 在 Agent 层放行，不靠前端自动点击批准。

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

### 4.1 单次文件授权与显式提权（macOS/Linux）

采用 on-request 流程：沙箱失败直接返回主模型，保留真实退出码、输出和已有副作用；不自动请求审批或重跑。模型先检查已完成效果，再提交剩余操作及所需 `additional_permissions`。

执行前声明 read/write 目标后，后端校验并冻结实际命令、规范化路径、文件/目录范围和规则快照。用户或既有审批审核器批准后，本次命令在带精确授权的沙箱内执行一次。卡片显示用途、命令、完整路径和每项读写/范围，仅允许一次或拒绝；不显示哈希、inode、规则摘要，也不提供持久规则按钮。

显式 `escalated: true` + justification 仍保留兼容：批准的是整条命令在沙箱外执行一次，不与精确权限混用。失败诊断不生成授权；历史 `sandbox_failure` 卡片仍可回放，但执行器不再生成这种被动重试请求。任何批准都不证明旧脚本可安全重放。

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

v2 的文件规则、read 前置审批、Seatbelt 编译、敏感守卫、GUI 保存与当轮注入均已实现。v1/v2 各轮的测试计数既不是当前总数、也不是当前验收结论，本文不再重录。

保留的决策：V1–V9（2026-07）确立纯路径规则、网络开放、分车道 fallback、文件真源、三档与项目允许；V10 的“仅 macOS 显示”已被 V11–V14（2026-08 Windows unelevated、具体 capability、共享可信 UI、接受身份/删除限制）及 Linux L-D1–L-D11 取代。不再保留互相冲突的旧状态段落。

近期重点：平台当前版本原生复验、安全 review、文档承诺与执行器一致；Windows 与 Linux 各自验收清单见平台文档。明确暂不做命令前缀持久规则、网络审批/域名过滤、auto-review agent、通用 MCP/新工具沙盒规范、设置页完整 user 规则编辑器。手动只读白名单是免打扰机制，不是任意 shell 静态安全分析；批准后 `git push --force`、`npm publish` 等仍能执行。

### 5.1 macOS/Linux 单次 execution_grants

`additional_permissions` 包含 read/write 数组，每项 `{path, scope, reason}`，合计最多 8 个 literal 目标。file 为已有普通文件，subtree 为已有目录及后代；创建文件声明已有父目录，若敏感 carveout 仍不能表达则使用已审批的 write/edit。禁止文件系统根和 home 根，无通配符。不支持的 OS 范围在审批前拒绝。

收据绑定工具调用 ID、命令哈希、cwd、规则摘要、规范化目标、access/scope 和目标 inode；执行前重新准备并匹配，一次消费。会话规则不被修改。只对原 ask 目标生成授权；已 allow 的父目录声明不能放开其敏感子项。所有显式 deny 保持高于 execution_grants，grants 高于 secret ask guards 和 session/workspace/user。

macOS 使用 literal/subpath SBPL；Linux 复用窄路径 mount/reopen 编译和私有报告；保留读拒绝的写授权、mode-000 不可读目录内的子路径授权在审批前拒绝，改用已审批的原生文件工具。Windows 继续已有 write capability，不提供 shell read grant，也不承诺 ACL deny-wins 的 ask carveout 可重新开放。三平台失败都不自动重放，不提供任意脚本内部续跑或回滚保证。

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

## 单命令 shell 执行结果（2026-10-09）

保留 `command` 输入，不提供 `steps` 协议，也不由宿主拆分命令字符串。模型
自行选择使用单个脚本或多个工具调用；依赖调用按顺序
执行并检查各自结果。依赖共享 `cd`、变量、函数或控制流的脚本保留为一个命令。
每次调用从项目工作目录启动独立宿主 shell，沿用继承环境及 PATH/PWD 覆盖；
前一个进程内的目录或变量变化不会传递给后一个调用。不全局注入 `set -e`，
也不修改管道行为。

复杂脚本只采集整个进程的状态。最后一个成功命令可能掩盖前面的失败；后续查询
非零也不证明此前操作失败。退出码 0 表示进程正常结束，不证明用户任务完成。
工具指导要求结合输出解释结果，并在重试前检查部分完成的效果。无法采集
Node/Python 程序内部每个操作的状态，也没有“恰好执行一次”保证。输出关键词
不能覆盖真实的成功退出码，也不会单独触发自动提权。

宿主先记录 `ShellResult` 再生成模型文本。结果包含命令、工作目录、总耗时、
状态、可空退出码、已有正常查询语义、审批结果、说明和有序执行尝试。每次尝试
记录真实状态、可空退出码、耗时、合并输出、截断及提权标志。新调用不覆盖原始结果；历史两次尝试仍可回放。`exited`、`launch_failed`、`execution_failed`、`timed_out`、
`cancelled`、`not_started` 分别表示进程退出、启动失败、读取或等待异常、超时、
取消及审批等原因导致进程未启动。无退出码不补造为 0，stdout 标记不能替代进程
状态。裸 grep/test 等退出 1 保留现有正常查询语义，同时显示实际退出码。

审批及平台限制保持现状：Jev 的三个数值 Choice 问题、审批矩阵、置信度阈值、
四类有来源证据、调查预算、取消、上下文有效性及 Linux helper 校验不变。
新审批对应模型重新准备的实际操作。批准表示允许执行，不证明旧脚本可安全重放。模型需先检查部分完成情况，再只提交剩余必要操作。网络行为不变。

输入限制为命令 65,536 字节、序列化参数 131,072 字节。执行预算 1–600 秒，默认 120 秒，从 shell handler 开始计时；执行前审批沿用原有生命周期。每次调用只有一次进程尝试，最多保留 250,000 字节输出，不再共享被动审批和重试预算。
结构化结果序列化上限为 1 MiB，JSON 转义超限时进一步缩减输出。模型摘要为
输出预留最多 60,000 字节，状态不受输出截断影响，整体低于现有 100K 字节
上下文上限。超时和取消保留已采集的部分输出。

`tool_end.shell_result` 贯通类型化 RPC，并进入权威 SQLite 工具消息元数据。
工具详情、历史、运行回放及远程 lean 通道保留相同事实。桌面和移动端 thread
消费宿主结论，保持原有工具行、合并折叠和摘要。运行详情页也沿用原有展示，
不新增执行或重试详情组件。结构化进程事实保留供宿主与模型使用，不证明用户
任务的结果。
旧记录没有结构化事实时沿用原展示，不补造尝试历史。
原有 `command` 调用兼容；不支持的 `steps` 输入在审批及执行前拒绝。

默认提示词将 `Tool Execution`、`Approval Feedback` 和 `User Communication`
分别组织为 `agent/src/prompt/` 下的 Markdown 模板。工具专属规则只进入执行
部分；调用方附加指导位于三类行为规则之后。项目上下文及追加提示词仍位于
后面；自定义提示词仍替换默认身份与行为部分。Jev 审批问题保持独立提示词。

默认身份描述为 FutureOS 的任务助手，不再限定为编码角色。沟通规则要求结果
优先、深度适应用户请求、语言自然清楚，技术细节仅用于回答问题、支持结论或
帮助决定。简单操作通常只需简短确认及相关结果或文件链接；较长任务提供有
意义的进展及自包含的最终回复。常规验证概括说明，不强制讲解代码、列风险
清单或额外询问审批。

这些原则借鉴本地 Codex `e974aad3b1`（2026-10-08）中
`codex-rs/models-manager/models.json` 的完整 `gpt-6.1-sol` 指令模板，覆盖全局
沟通及任务完成指导。该模板按请求调整解释深度，没有专门禁止代码示例，也
没有要求修改与验证必须拆成命令。FutureOS 保留自己的工具、文件链接格式、
记忆预算及审批策略，不照搬 Codex 的应用通道或权限模型。提示词可能影响
冗长程度，但源码对比本身不能证明此前回复的具体成因。

执行规则沿用同一任务、目标和范围内的既有用户授权，尊重后续限制，并在确需
用户决定前完成具体准备。审批反馈不能提供新的用户授权。拒绝后不能通过换
工具、解释器或包装方式实现同一被拒绝效果；真正缩小范围的允许操作仍须通过
正常检查。shell 指导在 `agent/src/tools/shell/guidance.rs` 中拆成短条目，由模型
选择执行单元的组织方式及诊断状态打印，仍要求结合真实进程状态、输出和预期
效果判断结果。
已知文件内容在 read 可用时优先使用该工具，不禁止 shell 读取。平台提示
描述外层 shell 的语法和真实版本限制，不限制程序或脚本使用自己的语法。
RPC 提示词拼装层不再追加相冲突的 shell 文件写入全面禁令；普通文件修改优先
使用 write/edit，用户明确要求脚本或命令行方式时通过 shell 执行并沿用现有
检查。后续用户指令可以提供新的授权以重新审核，但不能绕过策略或复活旧审批。

执行元数据定位为内部诊断信息。常规审批和重试恢复通过工具完成，不向用户
播报内部过程。恢复成功后报告任务结果，不回顾已恢复的失败、沙箱限制或审批
过程。工具返回获批不证明用户亲自确认过。回复报告任务结果和有用的验证；仅在用户要求调试，或尚未解决
的阻碍、决定确有需要时解释退出码、shell 机制和审批记录。失败、部分完成、
不确定性及有实际影响的副作用仍须用普通语言如实说明。模型文本保留原始及
重试证据，但不在每次调用中重复退出码 0 与成功审批的通用警告；审批说明仍
保留在结构化审计记录。这些是模型指导规则，不是对生成回复的确定性过滤。

回归测试覆盖独立操作与验证、复杂脚本原有语义、伪造 stdout 状态、可空退出码、
超时、取消、输出预算、审批拒绝与上下文失效、失败不重放、单次授权隔离，以及类型化、
持久化、lean、实时和历史结果。本地只编译测试目标，测试运行及跨平台原生验证
交给 CI。
