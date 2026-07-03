# FutureOS Sandbox 方案（SANDBOX_PLAN）

状态：**方案定稿，待最终 review**（2026-07-03 起草，同日 review 合入全部决策，见 §6）

> 本文是 P2 Approval「后续细化」中"沙盒执行"的完整设计。产品语义基线见 PRODUCT.md §4.6 / §6，数据模型基线见 ER.md §4.8 / §5，现有脚手架见 gui/CLAUDE.md 原则 8。设计以 Codex 的 sandbox 模型为中心，规则引擎借鉴 opencode 的 permission 体系。
>
> 平台顺序：**macOS 最先完成（Phase 1），Linux 放最后（Phase 4），Windows 原生沙盒不在本计划内**（恒为降级模式）。

## 0. TL;DR

- **双控模型**：沙盒（sandbox）定义技术边界——命令能写哪里、能不能联网；审批（approval）决定越界时是否停下来问用户。目标：**边界内自动执行不打扰，边界外必须显式决策**。
- **沙盒落在 agent 侧**（Rust，包裹 `bash` 工具 spawn 的子进程）：macOS 用 Seatbelt（`sandbox-exec`），Linux 用 bubblewrap（`bwrap`），Windows / 沙盒不可用时降级为现有"白名单 + 审批"模式。
- **三种沙盒模式**：`read-only` / `workspace-write`（默认）/ `danger-full-access`；配置项：`writable_roots`、`network_access`。复用已预留的 `sandbox_config` 表。
- **三种审批策略**：`untrusted` / `on-request`（默认）/ `never`。复用已预留的 `approval_policy_config` 表。
- **规则例外**（借鉴 opencode）：`approval_rules` 表实现 allow / deny / ask 三态 + 通配符匹配 + 后匹配优先；审批按钮从「允许一次」扩展为「允许一次 / 本会话 / 始终允许」。**allow 只跳过审批，不绕过沙盒**——命中 allow 的命令仍在当前沙盒模式内执行。
- **升级流（escalation）**：命令在沙盒内因越界失败（写被拒 / 网络被拒）→ 产生 `sandbox_escalation` 审批 → 用户允许后**该次命令出沙盒重跑**（按命令放行，见 §2.6）。这是 Codex "falls back to the approval flow" 的对应物。
- 分四个 Phase 落地，Phase 1（macOS 沙盒 + 最小升级流）即可显著降低审批疲劳。

---

## 1. 背景与现状

### 1.1 现状小结（2026-07 调查）

**已就位（P1）**：

- Approval 闭环完整：agent 侧 `agent/src/rpc/approval.rs`（shape 构建、SSE 广播、mpsc 阻塞等待、无超时）；GUI 侧持久化（`store/approvals.rs`）、决策回传（`agent_bridge/approval.rs`）、UI（`ApprovalPrompt.tsx` + `useApprovals.ts` 1.5s 轮询）。
- 简单只读命令自动放行：`approval.rs::is_workspace_read_command`——11 个白名单程序（`cat/echo/find/future/grep/head/ls/pwd/rg/sed/tail/wc`），禁止链式 / 重定向 / 命令替换，绝对路径必须在 workspace 内。
- workspace 边界判定：`approval.rs::path_is_outside_workspace`（canonicalize 后前缀比较）。
- 结构化 action payload + `sandbox_boundary` 字段已贯通 agent → GUI store → ApprovalPrompt（越界警告徽章）。

**是桩（P2 预留，未接线）**：

- `agent/src/rpc/approval_policy.rs::evaluate_policy` 恒返回 `AskUser`。
- GUI 三张预留表 `sandbox_config` / `approval_policy_config` / `approval_rules`（schema + CRUD stub 在 `store/approval_config.rs`，无 Tauri command、无 UI）。
- `decision_scope`（once/session/always）、`decision_source`（user/rule/sandbox）、`reviewer`（user/auto_review）字段存在但恒为 `once` / `user` / `user`。

**完全缺失**：

- OS 级沙盒执行：`agent/src/tools/mod.rs::run_bash` 直接 `Command::new("bash").args(["-c", cmd])` spawn，无任何文件系统 / 网络隔离（仓库内无 seatbelt / bwrap / landlock 任何痕迹）。
- 网络访问控制（`network_access` 枚举有名无实）。
- 沙盒失败后的升级审批流。

### 1.2 现状的问题

1. **审批疲劳与安全性双输**：白名单外的一切 bash 命令都要人工审批（`cargo build`、`npm test`、`git status` 全要点允许）；而用户一旦打开"自动审批"开关，又变成完全无防护——`rm -rf ~`、`curl | sh` 都会直接执行。中间没有"有边界的自主"档位。
2. **`sandbox_boundary` 是装饰**：payload 里 `"inside_sandbox": false` 硬编码，UI 显示的"沙盒越界"警告并没有真实沙盒背书。
3. **字符串启发式易绕过**：白名单判定基于文本扫描（`;`、`$(`、`>` 等），`ls $(dangerous)` 之类总有绕过面；应用层判定永远只能是第一道闸，不能是唯一一道。

### 1.3 参考来源

- **Codex（设计中心）**：沙盒与审批是两个正交控制——沙盒定技术边界（`read-only` / `workspace-write` / `danger-full-access` + `writable_roots` + 网络开关），审批策略定何时停下来问（`untrusted` / `on-request` / `never`）；沙盒作用于 spawn 的一切子命令；macOS Seatbelt / Linux bubblewrap；沙盒内失败回落到审批流；rules 机制处理沙盒外的命令前缀例外。
- **opencode（规则引擎借鉴）**：无 OS 沙盒，纯应用层 permission——`{action, resource, effect}` 规则，effect ∈ allow/deny/ask，通配符匹配（`*`/`?`），`findLast` 后匹配优先，deny 优先于已保存的 allow；审批回复 once/always/reject，always 写库跨会话生效；agent 级规则覆盖全局。
- **本仓既有约束（PRODUCT.md）**：审批串行、composer 上方、不超时；默认工具集固定为 `read/bash/edit/write`；Review（Shadow Review）负责"改了什么"的事后审查，与 Approval 分工不变。

---

## 2. 总体设计

### 2.1 三层控制模型

```
                 ┌─────────────────────────────────────────┐
  第 1 层        │ 规则（approval_rules，opencode 式）      │  显式例外：
  规则例外       │ allow / deny / ask + 通配符 + 后匹配优先 │  deny 最先拦截，
                 └───────────────┬─────────────────────────┘  allow 可放行沙盒外命令
                                 │ 无规则命中
                 ┌───────────────▼─────────────────────────┐
  第 2 层        │ 沙盒边界（sandbox_config，Codex 式）     │  边界内 → 自动执行
  技术边界       │ read-only / workspace-write / full      │  （decision_source=sandbox）
                 │ writable_roots + network_access         │  边界外 → 进第 3 层
                 └───────────────┬─────────────────────────┘
                                 │ 需要越界（或沙盒内失败请求升级）
                 ┌───────────────▼─────────────────────────┐
  第 3 层        │ 审批（approval_policy_config）           │  untrusted / on-request / never
  人工决策       │ 现有 ApprovalPrompt 流程，串行、不超时    │  决策范围 once / session / always
                 └─────────────────────────────────────────┘
```

一次 bash 工具调用的完整决策流：

```
bash(command) 到达
  │
  ├─ 1. 规则评估（evaluate_policy 实现体）
  │    ├─ 命中 deny  → AutoReject，错误返回给模型（不执行）
  │    ├─ 命中 allow → 跳过审批，但仍按当前沙盒模式执行（不绕过沙盒！）
  │    │   （allow 的价值在 untrusted 策略和降级模式下：免问直接执行；
  │    │     在 on-request + 沙盒可用时它与默认行为重合，仅作显式记录）
  │    └─ 命中 ask / 无命中 → 继续
  │
  ├─ 2. 沙盒可用性
  │    ├─ 可用（macOS sandbox-exec / Linux bwrap）
  │    │    └─ 按 sandbox_config 包裹执行 → 自动放行（无审批卡片）
  │    │         ├─ 成功 → 正常返回
  │    │         └─ 失败且匹配"沙盒拒绝"特征（或模型显式请求升级）
  │    │              → 产生 sandbox_escalation 审批
  │    │                  ├─ 允许 → 该次命令出沙盒重跑（§2.6）
  │    │                  └─ 拒绝 → 错误返回给模型
  │    └─ 不可用（Windows / 无 bwrap / full-access 模式）
  │         └─ 降级为现有逻辑：is_workspace_read_command 白名单自动放行，
  │            其余按 approval_policy 走审批（= 今天的行为）
  │
  └─ 3. 审批策略修饰整个流程
       ├─ untrusted：连沙盒内执行也先问（白名单只读命令除外）
       ├─ on-request（默认）：如上图，沙盒内自动、越界才问
       └─ never：从不弹审批；越界操作直接失败返回给模型（沙盒仍然生效！）
```

关键语义（与 Codex 对齐）：

- **沙盒 ≠ 审批的替代品，而是审批的前提**。"自动放行"之所以安全，是因为放行的东西真的被技术边界锁住了。`never` 策略下沙盒依然生效——这正是 Codex "approval_policy=never 但 sandbox=workspace-write" 的低风险自动化档位。
- **`danger-full-access` + `never` = 现在的"自动审批"开关**，作为显式的危险档位保留，UI 上明确标红。
- **read/write/edit 工具不进 OS 沙盒**（它们是 agent 进程内的 Rust 文件操作），继续走应用层边界判定，但判定标准从"cwd 前缀"升级为"`writable_roots` 集合"，与 bash 沙盒共享同一份配置，保证两条路径边界一致。

### 2.2 沙盒模式定义

复用 `sandbox_config` 表（字段已预留：`mode` / `writable_roots` / `network_access`，按 workspace 维度）：

| 模式 | 文件读 | 文件写 | 网络 | 适用场景 |
|---|---|---|---|---|
| `read-only` | 全盘可读（敏感路径除外，见 §2.3） | 全部拒绝 | 拒绝 | 只调查不动手；结对审查 |
| `workspace-write`（默认） | 全盘可读（敏感路径除外） | 仅 `writable_roots` | 默认拒绝，可按 workspace 开 | 日常开发/科研主档位 |
| `danger-full-access` | 无限制 | 无限制 | 无限制 | 用户显式选择；不包裹沙盒 |

`writable_roots` 默认集合（workspace-write）：

1. 当前 workspace 目录（session cwd）；
2. 系统临时目录（`$TMPDIR` / `/tmp`）——**读写完全开放**，绝大多数构建工具需要；
3. `/dev/null`、`/dev/stdout` 等伪设备（literal 允许）；
4. 用户在 Settings 里追加的额外目录（Codex 的 "writable roots let you extend the places it can modify"）。

普通 Chat 的临时 workspace 天然是 `writable_roots[0]`，语义不变。

### 2.3 敏感路径读屏蔽

`workspace-write` 允许全盘读是为了让构建链（读 `/usr/lib`、`~/.cargo` 等）不碎裂，但以下路径在 Seatbelt / bwrap 层面**连读也拒绝**（实现见 `sandbox/seatbelt.rs::sensitive_read_denials`，实测清单 2026-07-04）：

| 类别 | 路径 | 匹配 |
|---|---|---|
| SSH / GPG | `~/.ssh`、`~/.gnupg` | 整目录 |
| FutureOS 自身配置 | `~/.future/agent/{auth,models}.json`、`~/.future/agent-app/{auth,models}.json` | 单文件 |
| 包管理 / registry token | `~/.npmrc`、`~/.pypirc`、`~/.cargo/credentials{,.toml}`、`~/.gem/credentials` | 单文件 |
| 网络 / git 明文凭证 | `~/.netrc`、`~/.git-credentials` | 单文件 |
| home 级 env | `~/.env` | 单文件 |
| 云厂商凭证 | `~/.aws`、`~/.azure`、`~/.config/gcloud`、`~/.terraform.d`（整目录）、`~/.kube/config`（文件） | 混合 |
| 容器 registry auth | `~/.docker/config.json` | 单文件 |
| CLI token / Keychain | `~/.config/gh`、`~/Library/Keychains` | 整目录 |

两条硬约束：
1. **不能整目录屏蔽 `~/.future`**——普通 Chat 临时 workspace 就在 `~/.future/agent/workspace`，只屏蔽其中的具体凭证文件（`auth.json` / `models.json`）。
2. **只屏蔽真正含密的文件/目录，不整目录屏蔽构建工具会读的目录**（`~/.cargo`、`~/.config`、`~/.docker` 只挑里面的 secret 文件）。个别文件（`~/.npmrc` / `~/.pypirc`）同时含非密的 registry 配置，屏蔽后私有源安装可能在沙盒内失败——由 escalation 流（§2.6）兜底，这是刻意的安全/便利取舍。

后续按需补 `sensitive_read_denials`。

#### 2.3.1 项目内 secret（`.env` 等）——已知盲区，待决策

上面这些都是 **home 下、workspace 之外** 的凭证。**项目根目录里的 secret**（`.env`、`config/secrets.yaml`、`*.pem`）**当前完全不受沙盒保护**——workspace 是 agent 工作区，按设计可读可写。

关键点：**沙盒断网挡不住这条泄露路径**。真正的路径是 `cat .env` / read 工具的输出 → agent → **LLM provider 请求**（agent 进程自己发，不受沙盒网络限制）。断网只挡沙盒内命令主动外联，挡不住工具输出回流到模型。所以要防住项目内 secret，唯一办法是**从源头拒读**。

现状：
- bash `cat .env`：可读（workspace 在沙盒可读范围）。
- read/write/edit 工具：可读可改（这三个工具**不走 Seatbelt**，是 agent 进程内 fs 调用；`.env` 在 workspace 内 → 通过边界检查）。
- Shadow Review：已有敏感文件规则，标记但不落盘内容（`.env`/`*.pem`/`*.key`）。

**未做的原因**：硬屏蔽会误伤合法任务（"帮我看下 `.env` 配置"是正常请求），且跨层——要同时改 Seatbelt 拒读 + read/write/edit 工具拒绝，不是一行 deny 能覆盖。

**决策（2026-07-04）：记为 Phase 2/3，opt-in**——可配置的"项目内 secret 文件 glob"：
- 默认 pattern `.env` / `.env.*` / `*.pem` / `*.key` / `*.p12` / `id_rsa*` / `credentials.json`（可编辑），与 Shadow Review 敏感文件规则共用。
- 命中文件：Seatbelt 拒读 + read/write/edit 工具直接拒绝（即使在 workspace 内），走 escalation 放行一次。
- 放 Phase 2/3 与规则引擎 / Settings UI 一起做：它需要配置 UI 才好用，硬编码默认拒读项目文件会反直觉（"帮我看下 `.env`" 是正常任务）。Phase 1 不做。

### 2.4 审批策略

复用 `approval_policy_config` 表（字段已预留：`policy` / `reviewer`）：

| policy | 语义 |
|---|---|
| `untrusted` | 白名单只读命令以外的一切执行前都问（≈ 今天的默认行为） |
| `on-request`（默认） | 沙盒内自动执行；越界（escalation / workspace 外写 / 网络）才问 |
| `never` | 从不弹审批；越界操作直接失败返回给模型，沙盒仍生效 |

`reviewer` 维持 `user`；`auto_review` 留作 Phase 4+（见 §7）。

组合速查（对应 Codex 文档的组合语义）：

| sandbox mode × approval policy | 效果 |
|---|---|
| workspace-write × on-request | **推荐默认**：低摩擦本地自动化 |
| workspace-write × never | 无人值守跑任务（沙盒兜底） |
| read-only × on-request | 只读调查档 |
| danger-full-access × never | 完全放开（= 现"自动审批"），UI 标红 |

默认档位切换（存量用户从"事事审批"变为 `workspace-write × on-request`）**不做一次性告知**，直接生效（已决策，Q7）。

### 2.5 规则引擎（approval_rules 接线）

复用 `approval_rules` 表（字段已预留：`scope` / `match_kind` / `match_value` / `decision` / `enabled` / `expires_at`），语义按 opencode 收敛：

- `match_kind`：第一期支持 `command_prefix`（bash 命令，通配符 `*`）与 `path_glob`（write/edit 路径）。`path_glob` 转正则时必须转义全部正则元字符、只展开 `*`/`?`，匹配前先做 §3.5 的路径规范化。
- `decision`：`approve`（= allow，**只跳过审批、仍在沙盒内执行**）/ `reject`（= deny）；"ask" 即无规则命中的默认态，不需要存。
- **不提供"allow 即出沙盒"的规则语义**：否则一条保存的 `cargo *` allow 规则就成了 full-access 后门（可写 workspace 外、联网、读敏感路径）。枚举预留 `approve_unsandboxed`（显式"始终允许且不受沙盒限制"的高危规则，需专门的警示 UI 与审计），**本计划不实现**（见 §7）。需要单次出沙盒执行时走 escalation 审批（§2.6），需要长期联网的 workspace 打开 `network_access` 开关。
- 匹配算法：通配符转正则（`*` → `.*`，转义其余元字符），**后匹配优先**（按 `created_at` 排序取最后命中），**deny 恒优先于 allow**（安全侧偏置：同一命令同时命中新 allow 旧 deny 时取 deny——与 opencode 的纯 last-match 不同，取更保守的语义）。
- `scope`：`workspace`（默认，随 `workspace_id`）/ `global`（`workspace_id IS NULL`）。
- `expires_at`：decision_scope=session 的规则写入时带过期时间（会话结束/应用退出即失效），always 的规则不带。

审批卡片按钮扩展（PRODUCT.md §4.6 需同步更新）：

```
[ 拒绝 (Esc) ]                [ 允许一次 (⌘↵) ] [ 本会话允许 ▾ / 始终允许 ▾ ]
```

- 「允许一次」：现状不变（decision_scope=once）。
- 「本会话允许」：写一条 `expires_at = 会话结束` 的 approve 规则，`match_value` 取 agent 在审批请求里建议的保存模式（如命令前缀 `cargo *`、路径 glob）。
- 「始终允许」：同上但持久。写库前 UI 必须展示将要保存的模式并允许编辑（防止把 `rm -rf /tmp/x` 泛化成 `rm *`）。
- 保存模式由 agent 侧生成建议（对应 opencode `Request.save` 字段）：bash 取"程序名 + 第一个子命令 + \*"（`git push *`），write/edit 取目录 glob（`<dir>/*`）。

### 2.6 升级流（sandbox_escalation）

新增审批 kind：`sandbox_escalation`（ER.md §4.8 的 kind 枚举需扩展）。

**触发**（二选一即触发）：

1. **模型显式请求（主路径）**：给 bash 工具加可选参数 `escalated: bool` + `justification: string`（对应 Codex bash 工具的 `with_escalated_permissions`）。系统提示词告知模型：沙盒内失败且判断是边界问题时，带 justification 重试。
2. **失败特征匹配（兜底）**：沙盒内命令非零退出，且 stderr / 退出码匹配沙盒拒绝特征（macOS：`Operation not permitted` + seatbelt 日志特征；Linux：`ENETUNREACH` / `EACCES` 于 unshare-net 环境）。启发式允许漏报（模型会看到原始错误自行重试或请求升级），**不允许把普通编译错误误报成沙盒问题**——特征不确定时宁可不弹，只把原始错误返回给模型（已决策，Q6）。

**允许后的语义（已决策，Q2 = 方案 B，"按命令放行"）**：

- 用户允许后，**该次命令这一次完全出沙盒重跑**（等价 `danger-full-access`，仅本次执行），语义与手动批准一条命令完全相同——"这条命令我看过了，放行"。
- 选 B 不选"精确放宽"（只开网络/只加一个可写根）的理由：用户在审批卡片上审的对象是**那条完整命令**而不是权限清单，B 的心智模型与 UI 展示对齐；一次允许必然跑通，不会因多个越界点连环弹审批；无需实现易错的失败原因诊断。风险与无沙盒时代批准任何命令持平，没有更差。Codex 实际采用的也是此方案。
- 后续如有真实需求，可给升级卡片加"仅允许联网"之类的快捷降级选项（相当于把精确放宽作为 B 的增强）。

**升级审批卡片内容**：原命令、失败摘要（stderr 尾部）、justification、明确标注"允许后本次将不受沙盒限制"。

**架构承载点（后置审批）**：现有审批入口在 `agent/src/rpc/session_prompt.rs` 的 `before_tool_call` 闭包（执行**前**，闭包捕获 `approval_gate` / `broadcaster` / `session_id` / `cwd`），而 escalation 发生在命令失败**后**、tools 层内部——`run_bash` 拿不到这些 RPC 对象。Phase 1 需新增承载点，避免把 RPC/UI 细节塞进 tools 层：

- rpc 层构造 `EscalationRequester`（`Arc<dyn Fn(EscalationRequest) -> EscalationDecision + Send + Sync>` 或等价 trait），闭包捕获 approval_gate / broadcaster / session_id / tool_id，内部复用现有 `ApprovalGate::request` 的广播 + mpsc 阻塞等待机制（kind = `sandbox_escalation`）。
- 经 `ToolExecutionScope` 注入 tools 层；`run_bash` 沙盒执行失败且满足触发条件时调用它，拿到允许决策后在同一次工具调用内出沙盒重跑，拒绝则把沙盒错误 + 拒绝信息返回给模型。
- tools 层只依赖这个回调抽象，不 import rpc 模块（保持现有分层）。

### 2.7 与现有白名单的关系

`is_workspace_read_command` 白名单**保留**，角色变化：

- 沙盒可用时：它只在 `untrusted` 策略下起"免问"作用；`on-request` 下沙盒本身已经放行一切，白名单不再是主闸门。
- 沙盒不可用时（降级模式）：它仍是今天的第一道自动放行闸，行为不变。

---

## 3. OS 级实现

实现顺序：**macOS（Phase 1）→ Linux（Phase 4）→ Windows（本计划不做）**。

### 3.1 macOS — Seatbelt（Phase 1，最先完成）

用系统自带 `/usr/bin/sandbox-exec -p <profile>`（Codex / Claude Code 同款方案；API 标记 deprecated 但系统自身重度使用，短中期稳定——已决策接受此风险，Q5，sandbox 模块 trait 化以便未来替换）。执行包裹：

```
sandbox-exec -p <PROFILE> \
  -D WORKSPACE=<canonicalized ws> \
  -D TMP=<canonicalized $TMPDIR> \
  -D HOME=<canonicalized home> \
  bash -c <command>
```

所有 `-D` 参数注入 canonicalize 后的真实路径（macOS 上 `$TMPDIR` 实为 `/private/var/folders/...`，`/tmp` 实为 `/private/tmp`——Seatbelt 按真实路径匹配，注入 symlink 路径会导致规则不生效，见 §3.5）。`/tmp` 与 `$TMPDIR` 两者都进 writable roots。

Profile 草案（workspace-write，`network_access=false`）：

```scheme
(version 1)
(deny default)

; 进程：允许 fork/exec（工具链需要），信号限于沙盒内
(allow process-fork)
(allow process-exec)
(allow signal (target same-sandbox))

; 读：全盘可读，扣除敏感路径（§2.3）
(allow file-read*)
(deny file-read* (subpath (string-append (param "HOME") "/.ssh"))
                 (subpath (string-append (param "HOME") "/.gnupg"))
                 (subpath (string-append (param "HOME") "/.future")))

; 写：仅 writable roots + 伪设备
(allow file-write* (subpath (param "WORKSPACE"))
                   (subpath (param "TMP"))
                   (subpath "/private/tmp")
                   (literal "/dev/null") (literal "/dev/stdout") (literal "/dev/stderr")
                   (regex #"^/dev/tty.*"))
(allow file-read* (literal "/dev/random") (literal "/dev/urandom"))  ; deny default 下显式列出

; 系统基础设施（按实测最小化补齐）
(allow sysctl-read)
(allow mach-lookup)          ; 部分需要收窄到具体 service，实测定
(allow file-ioctl (literal "/dev/null") (regex #"^/dev/tty.*"))

; 网络：默认全拒；network_access=true 时替换为 (allow network*)
(deny network*)
```

- `read-only` 模式：去掉 `file-write*` 的 allow 子句。
- Profile 用 Rust 模板生成（参数注入用 `-D`，路径不做字符串拼接进 profile 正文，防注入）。
- 进程树中断：`sandbox-exec` 直接 exec 子进程、继承进程组，现有 `process_group(0)` + `killpg` 机制不变。
- **实测校准项**：`mach-lookup` / `sysctl` 的最小集合需要跑真实工具链（cargo、npm、git、python）校准，允许 Phase 1 先放宽这两类、后续收窄。

### 3.2 Linux — bubblewrap（Phase 4，刻意放最后）

```
bwrap --ro-bind / / \
      --dev /dev --proc /proc \
      --tmpfs /tmp --bind <TMPDIR> <TMPDIR> \
      --bind <WORKSPACE> <WORKSPACE> \
      --bind <extra_writable_root_i> <extra_writable_root_i> \
      --unshare-net \                     # network_access=false 时
      --unshare-pid --die-with-parent \
      -- bash -c <command>
```

- 敏感路径读屏蔽：对 `~/.ssh` 等叠加 `--tmpfs` 遮盖。
- `bwrap` 不存在或用户命名空间受限（AppArmor）：启动时探测一次，探测失败 → 降级模式 + GUI 显示警告（对应 Codex 的 startup warning），不阻塞使用。
- 不做捆绑 helper（Codex 的 fallback helper 不在本期范围）。

### 3.3 Windows / Linux 降级模式（现状实现）

Phase 1 只实现了 macOS Seatbelt。**Linux（bwrap 未做，Phase 4）和 Windows（永不做原生沙盒）当前都走降级模式**。

判定链（`sandbox/mod.rs`）：
- `platform_sandbox_available()`：macOS 查 `/usr/bin/sandbox-exec` 是否存在；**非 macOS 恒 `false`**。
- `ResolvedSandbox.available` = 上式结果；`wraps_bash()` = `available && mode != full-access` → **Linux/Windows 恒 `false`**。
- `build_bash_command()`：`wraps_bash()` 为 false → 一律 `Command::new("bash").args(["-c", cmd])`，**不包裹**（与沙盒出现前的执行方式逐字相同）。

降级模式下的实际行为（Linux/Windows，GUI 已下发 `workspace-write × on-request`）：

| 维度 | 行为 |
|---|---|
| bash 执行 | **无 OS 隔离**，裸 `bash -c` 直接跑 |
| bash 审批 | `approval_shape` 走 `sandboxed=false` 分支 = **旧白名单**：只读白名单命令（`ls`/`cat`/`grep`…）自动放行，其余（含链式/重定向/替换）→ 审批卡片，`violation = shell_command_not_in_allowlist` |
| write/edit 边界 | `writable_roots`（workspace + 临时目录 + 追加根），越界 → 审批。注意：策略激活时临时目录仍是可写根，所以 write 工具写 `/tmp` 不弹审批——但这只是"不审批"，**没有 OS 强制**（宿主可写任意处，只是被应用层审批拦着） |
| 网络 | 无沙盒网络隔离；`network_access` 开关对降级模式无效（沙盒才读它） |
| escalation | 不触发（`wraps_bash()` 为 false，前置/后置 escalation 分支都跳过）——没有沙盒可"逃出"，bash 审批通过后直接跑 |
| `never` 策略 | 非白名单 bash / 越界写 → 直接失败返回模型（`ApprovalGate` 检查 `approval_policy == Never`），与 macOS 一致 |

**安全模型的本质差异**：macOS 下"沙盒内自动放行"是安全的，因为真有 OS 边界锁着；降级模式下**唯一的防线是应用层审批**——非白名单命令、越界写都会弹卡片让用户看。所以降级 ≠ 无防护，而是"回退到沙盒出现前那套白名单 + 审批"。风险差异：macOS 上恶意 bash 最多写 workspace+tmp（OS 强制）；降级模式下用户点了允许，命令就无边界地跑。

已知未做（诚实标注）：
- **GUI 未渲染"沙盒不可用"徽标**。`sandbox_boundary.sandbox_available: false` 已在每次审批 payload 里如实带出，但前端目前不消费这个字段（grep 无引用）。所以 Linux/Windows 用户不会看到"当前无 OS 沙盒"的显式提示，容易高估隔离强度。这个徽标属于 Phase 3 GUI 工作（§4.4 / §5 Phase 3）。
- 降级是**运行时逐 session 判定**（`platform_sandbox_available()` 每次 resolve 都查），不是编译期开关。
- Windows 的 `bash` 可用性沿用旧逻辑（`Command::new("bash")`，需 WSL/git-bash），本计划未改。

### 3.4 read/write/edit 工具的应用层边界（与沙盒同源）

`agent/src/tools/mod.rs` 的 `ToolExecutionScope` 扩展：

```rust
pub struct ToolExecutionScope {
    workspace: PathBuf,
    approved_outside_paths: Arc<Mutex<Vec<PathBuf>>>,
    permission_level: String,
    interrupt_flag: Arc<AtomicBool>,
    // 新增
    sandbox: SandboxSettings,   // mode, writable_roots, network_access, availability
}
```

- `write`/`edit` 的越界判定从"是否在 cwd 内"改为"是否在 `writable_roots` 任一根内"（`path_is_outside_workspace` 改造），保证与 bash 沙盒一致。
- `read-only` 模式下 `write`/`edit` 工具直接进审批（或按 policy 拒绝），不静默执行。
- `.git` 目录**不**从 writable_roots 内排除（已决策，Q4：Shadow Review 已提供变更可见性，排除会破坏 `git commit` 等正常操作）。

### 3.5 路径判定规范（应用层判定必须与沙盒真实行为一致）

`path_is_outside_workspace` 现状有几处与 OS 沙盒语义不一致的地方，Phase 1 统一为以下规范（独立函数 + 单测）：

1. **`~/` 解析到真实 `$HOME`**。现有实现把 `~/x` join 到 workspace（`workspace.join(relative)`）——这是错的：应用层判定"在 workspace 内"而 Seatbelt 按真实路径 `$HOME/x` 拒绝写，两层结论相反。统一改为展开到 `dirs::home_dir()`。
2. **不存在的路径 canonicalize 其最近存在的祖先**。写入目标常常尚不存在，直接 `canonicalize` 失败后现有代码退回原始路径（symlink 不展开）。改为：向上找到最近存在的祖先目录 canonicalize，再拼回剩余部分。
3. **symlink 按最终真实路径判定**。写入目标（或其祖先）是 symlink 时，以 canonicalize 后的目标路径为准——workspace 内一个指向 `~/.ssh` 的 symlink 不能被判成"workspace 内写入"。
4. **macOS 大小写不敏感**。APFS 默认 case-insensitive：`/Users/tao/WS` 与 `/users/tao/ws` 是同一目录。macOS 上前缀比较用大小写不敏感比较（以 canonicalize 返回的规范形为准），其他平台保持敏感。
5. **`/tmp`、`/var` 的 symlink 事实**（macOS）：`/tmp → /private/tmp`、`/var → /private/var`，`$TMPDIR` 实际在 `/var/folders/...`。writable_roots 与被判定路径都必须在 canonicalize 之后比较，注入 Seatbelt 参数的也必须是 canonicalize 后的真实路径（§3.1）。

`path_glob` 规则匹配（§2.5）复用同一套规范化后再匹配。

---

## 4. 数据与接口改动

### 4.1 配置真源与下发路径

- **GUI SQLite 是配置真源**（`sandbox_config` / `approval_policy_config` / `approval_rules` 按 workspace 维度），agent 不新增本地配置文件（遵守"GUI 配置不落 agent 文件"的既有分界；TUI/CLI 用户后续可经 settings.json 提供等价配置，本期不做）。
- **gRPC 新增命令 `set_sandbox_policy`**（proto/future.proto）：GUI 在 new_session / switch_session 后、每次 prompt 前若配置有变即下发。**协议形态**：现有 `RpcCommand` 是扁平字段结构（`type` 字符串分发，各命令复用 `message`/`mode`/`entry_id` 等标量，GUI 侧经 `agent_bridge/client.rs::base_command` 构造）。沙盒策略是结构化数据，**不塞字符串标量、不走 JSON-in-string**，而是给 `RpcCommand` 加一个 typed 子消息字段（proto3 向后兼容，老客户端不受影响）：

```protobuf
// RpcCommand 新增字段（type == "set_sandbox_policy" 时读取；编号取现有空段）
message RpcCommand {
  // ... 现有扁平字段不动 ...
  SandboxPolicy sandbox_policy = 90;
}

message SandboxPolicy {
  string sandbox_mode = 1;          // read-only | workspace-write | danger-full-access
  repeated string writable_roots = 2;  // 追加根；workspace 与 TMP 由 agent 恒含
  bool network_access = 3;
  string approval_policy = 4;       // untrusted | on-request | never
  repeated ApprovalRule rules = 5;  // 展平后的有效规则（含 session 级）
}
message ApprovalRule {
  string match_kind = 1;   // command_prefix | path_glob
  string match_value = 2;
  string decision = 3;     // approve | reject
}
```

  GUI 侧在 `agent_bridge/client.rs` 增加对应的 `set_sandbox_policy_command` 构造函数；session_id 沿用 `base_command` 既有传法。后续若 RpcCommand 向 typed oneof 演进，`SandboxPolicy` 可整体迁移，不影响字段定义。

- agent 把它存进 `ServerSession`，工具执行时经 `ToolExecutionScope` 读取。**规则评估在 agent 侧执行**（`evaluate_policy` 实现体），GUI 只负责存储与编辑——避免每个审批往返一次 GUI。
- 未收到 `set_sandbox_policy` 的 session（TUI/CLI/channels 客户端）：agent 使用内置默认 `workspace-write × on-request × 空规则`，行为对这些前端仍等于今天（它们的审批 UI 已存在）。channels（Feishu/DingTalk）无审批 UI，按 `never` 语义处理：越界直接失败返回给模型（已决策，Q3）。

### 4.2 审批 payload 真实化

`approval_shape` 的 `sandbox_boundary` 从硬编码改为真实值：

```json
{
  "mode": "workspace-write",
  "inside_sandbox": true,            // 真实反映本次执行是否在沙盒内
  "sandbox_available": true,         // 新增：平台沙盒是否可用
  "violation": "network_blocked",    // 新增枚举：outside_workspace_write | network_blocked | sandbox_escalation | shell_command_not_in_allowlist(降级模式)
  "cwd": "...",
  "writable_roots": ["...", "/tmp"]
}
```

kind 枚举新增：`sandbox_escalation`；`decision_source` 开始真实使用：沙盒内自动执行的工具调用**不再产生审批记录**（保持"审批 = 需要人决策的事"的信噪比），但 run event 里记 `tool.sandboxed` 事件供 Run Inspect 展示。

### 4.3 存储改动（ER.md 同步）

- `approval_rules` 增加 `created_from_approval_id`（可空，追溯规则来源）与 `match_kind` 枚举注释更新。
- `sandbox_config.writable_roots` 语义明确为"追加的额外根"（workspace 与 TMP 恒含，不落库，防止 workspace 迁移后失配）。
- schema 变更走既有"单一真源、幂等应用"模式（`IF NOT EXISTS` / 新列 `ALTER TABLE ... ADD COLUMN` 幂等守卫）。

### 4.4 GUI 界面

1. **Settings ▸ 审批与沙盒**（新页，Phase 3）：
   - 全局默认：沙盒模式三选一（`danger-full-access` 标红 + 二次确认）、网络开关、审批策略三选一。
   - per-workspace 覆盖：在 workspace 详情 / 右键菜单进入，同款控件 + 额外可写目录列表。
   - 规则管理：列表（match/decision/scope/来源/过期）、启用/禁用、删除；不做复杂编辑器，规则主要经审批卡片沉淀。
2. **Composer 快捷切换**：现有"自动审批/手动审批"下拉升级为三档审批策略 + 沙盒模式显示（沿用同一全局开关位，`danger-full-access + never` 对应旧"自动审批"）。
3. **ApprovalPrompt**：新增「本会话允许 / 始终允许」按钮（含保存模式预览与编辑）；`sandbox_escalation` 卡片模板（失败摘要 + "本次将不受沙盒限制"提示）。
4. **Run Inspect / Runs**：工具调用行加沙盒徽标（in-sandbox / degraded / full-access）。

---

## 5. 实施阶段

### Phase 1 — macOS 沙盒执行 + 最小升级流（核心价值；**不含规则引擎**）— ✅ 已完成（2026-07-04）

范围刻意收窄为五件事：macOS 沙盒包裹、bash 自动沙盒执行、真实 `sandbox_boundary`、后置 escalation 架构、write/edit 同源 writable roots。规则引擎与「本会话/始终允许」整体放 Phase 2（含 UI 与审计一起做），full-access bypass 规则本计划不做（§2.5 / §7）。

Agent 侧为主：

- [x] `agent/src/sandbox/{mod,paths,seatbelt}.rs`：`SandboxPolicy`/`ResolvedSandbox`、平台探测（macOS `sandbox-exec` 存在性）、Seatbelt profile 生成（路径 SBPL 转义直嵌，未用 `-D`——多可写根无法单参数表达，转义集中在 `sb_quote` 一处）、命令包裹。
- [x] 路径判定规范化（§3.5，`sandbox/paths.rs` + 单测）：`~` 展开到真实 HOME（修复了旧的 workspace-join bug，同步修了 `workspace_path` / `resolve_workspace_path` / `approved_argument_path` 三处）、最近存在祖先 canonicalize、symlink 最终路径、macOS 大小写不敏感组件比较。
- [x] `run_bash` 接入（拆出 `spawn_bash` 执行内核）；full-access / 降级时保持现状；进程组 kill 机制不变（sandbox-exec exec 子进程、同组）。
- [x] proto `RpcCommand.sandbox_policy = 150` + `SandboxPolicy`/`SandboxRule` message + `set_sandbox_policy` 命令 + `ServerSession.sandbox_policy` + `ToolExecutionScope.sandbox`（GUI 每次 prompt 前固定下发 `workspace-write × on-request × 空规则`）。
- [x] 沙盒内自动放行：`approval_shape` 沙盒可用 + `on-request` 时对 bash 返回 None；`untrusted` 仍问（白名单只读命令除外）；`never` 越界直接失败返回模型；`tool_sandboxed` run event 广播。
- [x] **后置审批承载点**：`EscalationRequester` 闭包（`session_prompt.rs` 构造，捕获 gate/broadcaster/session_id，内部走 `ApprovalGate::ask_user` 公共等待逻辑）；bash 工具 `escalated/justification` 参数（仅沙盒实际包裹时生效，避免降级模式双重审批）+ 保守失败特征启发式 → `sandbox_escalation` 审批 → 允许后该次出沙盒重跑。
- [x] `sandbox_boundary` payload 真实化（`inside_sandbox` / `sandbox_available` / canonicalized `writable_roots`）；GUI ApprovalPrompt 渲染 `sandbox_escalation` 卡片（命令 + justification + 失败输出 + "无沙箱限制运行一次"警示）。
- [x] write/edit 边界判定改 `writable_roots` 集合（`ensure_workspace_access` 复用 §3.5 规范化；`read-only` 模式下 workspace 内写也进审批）。
- [x] **Profile smoke tests**（`agent/tests/sandbox_smoke.rs`，9 项，`#[ignore]`，`cargo test --test sandbox_smoke -- --ignored`）：本机全绿——`cargo check`、`git init/add/commit/status`、`python3 -c`、写 workspace/`$TMPDIR`/`/tmp`、home 写被拒（EPERM 特征确认）、`~/.ssh` 与 `auth.json` 读被拒、默认断网 + `network_access=true` 放行、read-only 拒写、`/dev/urandom`/`/dev/stdout`。实测修正：macOS `/dev/stdout` 解析到 `/dev/fd/N`，profile 需放行 `regex ^/dev/fd/`。
- 验收状态：`make test`（agent 54 + GUI tauri 69 + 前端 vitest 39）、`make lint`、`make check-gui`、smoke tests 9/9 全部通过。**尚未做**：`make run-gui` 实机走一轮真实模型对话验证审批卡片与升级流的端到端体验（需要真实 LLM key，建议用户实机确认）。

实现备注（与计划的偏差）：
- 敏感路径读屏蔽不能整体屏蔽 `~/.future`——普通 Chat 的临时 workspace 就在 `~/.future/agent/workspace` 下；实际只屏蔽 `~/.ssh`、`~/.gnupg` 两个目录 + `~/.future/agent/auth.json`、`~/.future/agent-app/auth.json` 两个凭证文件。
- 顺手修复：`DEFAULT_PERMISSION_LEVEL` 被无关 commit（49eab817）从 `workspace` 误改为 `all`（对应测试当时就红了），已恢复 `workspace`。
- 行为变化：temp 目录成为可写根后，"写 /tmp 下文件"不再触发审批（原本会）；三个依赖旧语义的既有测试已改用 home 路径作为越界目标。

### Phase 2 — 策略引擎与决策范围（规则 + UI）— ✅ 已完成（2026-07-04）

- [x] `evaluate_policy` 实现（`agent/src/rpc/approval_policy.rs`）：三态规则匹配（`command_prefix`/`path_glob`、通配符 `*`/`?`、**deny 恒优先**、`" *"` 结尾兼容裸命令前缀）；规则评估**移到 `ApprovalGate::request` 顶部**（§2.1）——deny 拦得住沙盒内自动放行的命令，approve 跳过 `untrusted` 提示但仍走沙盒（`approve` 只免审批、不出沙盒；full-access bypass 规则不做，§2.5/§7）。
- [x] ApprovalPrompt「本会话 / 始终允许」按钮 + 保存模式内联编辑（`TextInput`，Esc 先关编辑器再拒绝）；agent 审批请求携带 `save_suggestion`（bash「程序名+子命令+\*」，write/edit 父目录 glob；escalation 不建议规则）。
- [x] 规则写回 `approval_rules`：`save_approval_rule` 命令（从 thread 解析 workspace）；session 规则用非空 `expires_at` 作标记、启动 `prune_session_rules` 清理，always 规则持久；`list_effective_rules`（workspace + global，enabled）经 `set_sandbox_policy` 每次 prompt 下发。
- 验收状态：`make test`（agent 67 + GUI tauri 69 + 前端 vitest 39）、`make lint`、`make check-gui` 全绿；规则引擎单测覆盖通配符/优先级/deny-wins + 两个端到端 ApprovalGate 测试（deny 拦沙盒内 bash、approve 跳过 untrusted 提示）。**尚未做**：`make run-gui` 真实模型端到端点「始终允许 git push」→ 下一轮同前缀不再问的实机确认（需真实 LLM key）。

实现备注：
- 决策范围建模：`approval_rules.expires_at` NULL=always（永久）、非 NULL=session（下次启动清理），运行期不按时间过滤——session 规则整轮有效。语义是"本 app 运行内"，非某个 agent 会话。
- `untrusted`/`never` 策略语义在 Phase 1 已就位（`approval_shape` + `ApprovalGate` 的 never 检查），Phase 2 未改。
- 审计（规则来源追溯 `created_from_approval_id`）本期未接线，留 Phase 3 规则管理 UI 一起做。

### Phase 3 — Settings UI 与 per-workspace 配置

- [ ] Settings ▸ 审批与沙盒页（全局 + per-workspace 覆盖 + 规则管理）。
- [ ] Composer 三档快捷切换改造；`danger-full-access` 红色警示与二次确认。
- [ ] Run Inspect / Runs 沙盒徽标；Header 降级警告。
- 验收：`make check-gui` 通过；实机确认三档切换即时生效（下一条命令生效）。

### Phase 4 — Linux bwrap 与收尾（平台支持刻意放最后）

- [ ] bwrap 包裹实现 + 启动探测 + AppArmor 受限提示（文案引导安装 `bubblewrap`）。
- [ ] Seatbelt profile 按实测收窄 `mach-lookup` / `sysctl`。
- [ ] 文档：PRODUCT.md §4.6 / ER.md §4.8 / CLAUDE.md 原则 8 同步更新。
- 后续（不在本计划内）：Windows 原生沙盒；`auto_review` reviewer；channels 客户端的审批呈现。

---

## 6. 决策记录（2026-07-03 review）

| # | 问题 | 决策 |
|---|---|---|
| Q1 | 敏感路径读屏蔽范围 | 第一期仅 `~/.ssh` / `~/.gnupg` / `~/.future` 凭证三类（§2.3） |
| Q2 | escalation 允许后的放宽粒度 | **方案 B：该次命令出沙盒裸跑**（"按命令放行"；理由与后续增强见 §2.6） |
| Q3 | channels 前端无审批 UI，越界如何处理 | 按 `never` 语义：越界直接失败返回模型（§4.1） |
| Q4 | `.git` 目录是否从 writable_roots 排除 | 不排除（Shadow Review 兜底；排除会破坏 `git commit`）（§3.4） |
| Q5 | `sandbox-exec` deprecated 风险 | 接受（业界现状一致）；sandbox 模块 trait 化留替换位（§3.1） |
| Q6 | 沙盒失败特征启发式的误报 | 特征不确定时不弹升级审批，只把原始错误给模型；模型 `escalated` 参数为主路径（§2.6） |
| Q7 | 默认档位切换是否一次性告知存量用户 | 不做告知，直接切换（§2.4） |
| — | 系统临时目录 | 读写完全开放，恒含于 writable_roots（§2.2） |
| — | 平台顺序 | macOS 最先（Phase 1），Linux 最后（Phase 4），Windows 不在本计划内 |

第二轮（第三方安全 review）合入的修正：

| # | 问题 | 修正 |
|---|---|---|
| R1 | 规则 allow "不进沙盒"是 full-access 后门 | allow 只跳过审批、仍在沙盒内执行；`approve_unsandboxed` 预留枚举、本计划不实现（§2.1 / §2.5 / §7） |
| R2 | escalation 是后置审批，`run_bash` 拿不到 ApprovalGate | 新增 `EscalationRequester` 回调，rpc 层构造、经 `ToolExecutionScope` 注入，tools 层不触碰 RPC 细节（§2.6） |
| R3 | `set_sandbox_policy` 协议形态不明 | 给扁平 `RpcCommand` 加 typed 子消息 `SandboxPolicy sandbox_policy`，不走 JSON-in-string（§4.1） |
| R4 | 路径语义与沙盒真实行为可能不一致 | 新增 §3.5 路径判定规范：`~`→真实 HOME（修既有 bug）、最近存在祖先 canonicalize、symlink 最终路径、macOS 大小写不敏感、`/tmp`/`/var`/`$TMPDIR` symlink 事实 |
| R5 | Seatbelt profile 实操缺口 | `-D HOME` 补齐、参数一律注入 canonicalize 真实路径、伪设备补 `/dev/stdout|stderr|tty|random|urandom`、Phase 1 增加 profile smoke tests（§3.1 / §5 Phase 1） |
| R6 | Phase 1 范围 | 收窄为五件事（沙盒包裹 / 自动沙盒执行 / 真实 boundary / escalation 架构 / write-edit 同源 roots），规则引擎连同 UI、审计整体放 Phase 2（§5） |

---

## 7. 明确不做（本计划范围外）

- Windows 原生沙盒（AppContainer / Job Object / WSL2 桥接）。
- `approve_unsandboxed` 规则（"始终允许且不受沙盒限制"的 full-access bypass）——枚举预留，需专门的高危警示 UI 与审计后再评估（§2.5）。
- `auto_review`（审查 agent 作为 reviewer）——字段与流程预留位保持。
- 网络细粒度控制（按域名/端口白名单）——只做布尔开关。
- escalation 的精确放宽（"仅允许联网"等快捷选项）——作为方案 B 的后续增强，有真实需求再做。
- TUI / CLI 的沙盒配置 UI（agent 默认值已覆盖其行为）。
- 捆绑 bwrap helper 二进制。
- MCP / 未来新工具的沙盒接入规范（等工具集扩展时再定）。
