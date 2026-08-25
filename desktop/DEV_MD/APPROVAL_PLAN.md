# FutureOS 审批规则方案（APPROVAL_PLAN）

状态：**方案 v2 已实现；Windows unelevated 的能力请求、一次/项目审批、共享 UI 与受限执行内部链已实现，Windows 真机验证及产品启用待完成**（v2 定稿 2026-07-04，落地见 SANDBOX_PLAN R1/R2/R3 ✅、`src-tauri/src/approval_rules.rs`、`commands/approvals.rs`；取代 SANDBOX_PLAN v1 的"规则引擎 + 审批策略"部分）。2026-08 更新：工具名 `bash`→`shell`；敏感文件守卫上移为不可覆盖层（§3）；`auth.json` 暂放行（§3.1）；Windows `sandbox` 档定义为 unelevated **写保护**后端，使用路径能力审批（§5、§6.4、SANDBOX_PLAN §11）。

> 本文是审批系统的**语义主文档**：规则模型、规则文件、判定流程、审批 UI。OS 层如何强制执行（Seatbelt 编译、escalation、降级模式、返工计划）见 [SANDBOX_PLAN.md](SANDBOX_PLAN.md)。产品语义基线 PRODUCT.md §4.6 需在实现后同步。

## 0. TL;DR

- **一切审批围绕"文件路径访问"**：读/写哪个路径，按分层规则得出 `ask / allow / deny`。**网络访问完全放开、不审批**；命令本身不再按前缀审批（纯文件模型）。
- **分层规则，首个匹配即返回**：内置安全覆盖 → 敏感文件守卫（均不可覆盖）→ 会话临时规则 → workspace 规则文件 → user 规则文件 → 兜底（读全放、写限 workspace / temp）。
- **两个规则文件**：`${WORKSPACE_DIR}/.future/approval_rule.json`（项目级，可进 git）和 `~/.future/approval_rule.json`(用户级)。JSON、可手改、agent 直接读取。
- **审批弹窗按场景提供三个语义**：不允许 / 仅允许这一次 / **此项目以后都允许**（把 allow 规则写进 workspace 规则文件，下次不再问；具体文案可后续打磨）。
- **Windows shell 不做模糊脱沙盒放行**：shell 必须声明额外写能力的具体路径；共享审批卡片用后端生成的标题直接说明“行为 + 目标”，批准后只给这些路径增加本次或本项目能力，命令仍在 RestrictedToken 下重跑。
- **只有 GUI 启用**这套规则系统；TUI / CLI / channels 不启用，行为与现状一致。
- 设计目标排序：**配置简单易懂 > 使用流程顺畅 > 安全性尚可**（不是极致安全）。

## 1. 设计目标与核心理念

三个目标（按优先级）：

1. **配置简单易懂**：规则是一个能 `cat` 的 JSON 文件，模型一句话说清——"路径 → ask/allow/deny，从上往下第一个匹配算数"。
2. **使用流程顺畅**：日常开发（npm/pip/cargo/git，含联网）不被打扰；只有碰敏感文件、写到项目外时才问。
3. **安全性尚可**：真 OS 边界（Seatbelt）+ 凭证护栏 + 防自提权。不追求极致（已明确放弃断网防泄与命令级护栏）。

核心理念：**审批的对象是文件访问，不是命令**。命令允许与否不构成安全边界（多功能命令、可执行任意代码、字符串匹配可绕过）；真正可强制、可理解的边界是"能读什么、能写什么"。命令在沙盒里自由跑，撞到文件边界才停下来问。

## 2. 规则模型

一条规则：

```json
{ "path": "<glob>", "access": "read" | "write" | "both", "action": "ask" | "allow" | "deny" }
```

- `path`：路径 glob。
  - workspace 规则文件里的**相对路径**相对 workspace 根解析；`~/` 展开到真实 HOME；user 文件建议只写绝对路径或 `~/`。
  - **无通配符**的模式匹配"该路径本身及其子树"（`~/.ssh` = `~/.ssh` 和 `~/.ssh/**`），符合直觉。
  - 通配符：`*` 段内任意、`**` 跨段任意、`?` 单字符。
  - 匹配前先做路径规范化（symlink 解析到最终路径、`..` 折叠、macOS 大小写不敏感——见 SANDBOX_PLAN §路径规范）。**symlink 按目标路径判定**：workspace 里指向 `~/.ssh` 的链接不算 workspace 内。
- `access`：规则管读、管写还是都管。缺省 `both`。**读写必须分车道**——读的默认期望是"全放开"（构建链要读全盘），写的默认期望是"限 workspace"；一个路径一个笼统判定表达不了这件事。
- `action`：
  - `allow`：直接执行，不问。
  - `deny`：直接拒绝，错误返回给模型（不问用户）。
  - `ask`：问用户。**在 read/write/edit 等工具调用上是真正的前置弹窗**；在 shell 里无法逐文件中途询问，**编译进沙盒时按 deny 处理**，命令失败后走 escalation（那一次失败-询问就是 shell 版的 "ask"，见 §5）。

## 3. 分层与优先级

从上到下评估，**第一个匹配立即返回**：

```
第 0 层  内置安全覆盖（不可改，workspace/user 都盖不过）
         · 两个规则文件本身的【写】 → deny
         · 应用自身配置 models.json 等【读写】 → deny（auth.json 暂放行，见 §3.1）
第 1 层  敏感文件守卫（不可改，workspace/user 规则盖不过）
         · 凭证/隐私文件（.env、*.pem、~/.ssh 等）→ ask（access: both）
第 2 层  会话临时规则（本对话/工作区「允许一次」的当轮内存注入，不落盘）
第 3 层  workspace 规则文件  ${WORKSPACE_DIR}/.future/approval_rule.json
         · 用户自配 ask/allow/deny；「本工作区允许」写进这里
第 4 层  user 规则文件       ~/.future/approval_rule.json
         · 用户自配 ask/allow/deny；后期经设置菜单编辑
兜底     读写分车道
         · read  → allow                          （读默认开放）
         · write → 在 workspace / temp 内 ? allow : ask
```

完整判定伪代码（对每次文件访问：路径 P、操作 A ∈ {read, write}）：

```
P ← canonicalize(P)                       # §2 规范化
for layer in [安全覆盖, 敏感守卫, 会话临时规则, workspace 文件, user 文件]:
    for rule in layer:                    # 文件内按书写顺序
        if rule 匹配 (P, A): return rule.action
# 兜底
if A == read:  return allow
return P ∈ (workspace ∪ temp) ? allow : ask
```

### 3.1 第 0 层：防自提权（为什么必须存在）

workspace 规则文件在 workspace 里 = 兜底可写；它又是最高优先级。若无第 0 层，agent 一条 `echo '...' > .future/approval_rule.json` 就能给自己放开 `~/.ssh`。所以：

- **写**以下文件恒 deny，任何层覆盖不了：
  - `${WORKSPACE_DIR}/.future/approval_rule.json`
  - `~/.future/approval_rule.json`
  - `~/.future/agent/models.json`（及 `agent-app` 变体）
- `~/.future/agent/auth.json` 当前**暂放行**：skills 有时会 shell 到官方 `future` CLI 读凭证，硬 deny 会拦死这些流程。待专用凭证通道（短时 scoped token 注入或 peer-credential 反查）落地后恢复 deny；放行期间任何 shell 命令都可读写 auth.json，仅本地测试可接受。
- 读规则文件不限制（内容无密，agent 看到规则反而有助于理解边界）。
- 精确到文件、不封整个 `.future` 目录——普通 Chat 的临时 workspace 就在 `~/.future/agent/workspace/` 下，封目录会砸掉 Chat 自己的工作区。
- 修改规则文件的合法通道只有两个：**用户手改**、**GUI 代写**（「本工作区允许」按钮，走 Tauri 可信路径，不经 agent 工具）。
- macOS 已知残留：escalation 批准后整条命令出 Seatbelt 跑，理论上可借此写规则文件——但用户批准时看到的就是那条命令本身，属知情同意，接受。Windows write-protect 不提供这种整命令放行。

### 3.2 第 1 层：敏感文件守卫清单

凭证/隐私 → `ask`（`access: both`；**守卫层在 user 规则之上，不可被 workspace/user 规则覆盖**——一条宽 allow 无法解除目录里落到敏感文件上的 ask，敏感文件只能「允许一次」，不能持久放行）：

| 类别 | 路径 |
|---|---|
| SSH / GPG | `~/.ssh`、`~/.gnupg` |
| 包管理 token | `~/.npmrc`、`~/.pypirc`、`~/.cargo/credentials{,.toml}`、`~/.gem/credentials` |
| 明文凭证 | `~/.netrc`、`~/.git-credentials`、`~/.env` |
| 云厂商 | `~/.aws`、`~/.azure`、`~/.config/gcloud`、`~/.terraform.d`、`~/.kube/config` |
| 容器/CLI | `~/.docker/config.json`、`~/.config/gh` |
| Keychain | `~/Library/Keychains` |
| **workspace 内** | `.env`、`.env.*`、`**/*.pem`、`**/*.key`、`**/*.p12`、`**/id_rsa*`（相对 workspace） |

> workspace 内凭证条目正是 v1 的"项目内 secret 盲区"（原 SANDBOX_PLAN §2.3.1）的解法：`.env` 进守卫 ask，read 工具与 shell 都拦得住；"帮我看下 .env" 场景由用户点一次"允许一次"解决（守卫不可覆盖，不提供"本工作区允许"），不再反直觉。

## 4. 规则文件

### 4.1 格式

```json
{
  "version": 1,
  "rules": [
    { "path": "dist",        "access": "write", "action": "allow" },
    { "path": ".env",        "access": "read",  "action": "allow" },
    { "path": "~/notes",     "access": "write", "action": "allow" },
    { "path": "secrets",     "action": "deny" }
  ]
}
```

- 同层内按**书写顺序**匹配，先写先赢（想要例外规则，放在宽规则前面）。
- 未知字段忽略（向前兼容）。
- **解析失败 fail-safe**：某个文件 JSON 坏了 → 跳过该层、记日志、向 GUI 发告警事件；其余层照常生效（内置 + 兜底仍在，不会 fail-open 到全放行）。
- 加载时机：agent 每轮 prompt 开始时读取两个文件（无缓存失效问题）；同轮内新增规则经 §6.2 的内存注入即时生效。

### 4.2 谁读谁写

- **agent 直接读**这两个文件（启用时）。这是有意从 v1 的"GUI SQLite 真源 + gRPC 下发"反转——文件可 `cat`、可手改、可 git 追踪，且为将来 TUI/CLI 复用铺路。
- **写**：用户手改，或 GUI「本工作区允许」代写（Tauri fs，可信路径）。agent 工具永远写不了（第 0 层）。

## 5. ask 在两类执行路径上的落地

| 执行路径 | ask 的表现 |
|---|---|
| **read / write / edit 工具**（agent 进程内，路径确切已知） | 真前置审批：工具执行前弹卡片，等用户决定。**read 工具本次新接入审批**（v1 它完全不受控，是已知漏洞）。 |
| **shell / macOS sandbox** | 子进程无法中途询问；`ask` 与 `deny` 编译为 Seatbelt 拒绝。命中后进入现有 escalation：卡片展示原命令、失败摘要和可推断路径；当前批准语义仍是整条命令脱离 Seatbelt 重跑一次。无法可靠归因的路径必须标为“未知”，不得伪装成精确路径审批。 |
| **shell / Windows write-protect** | RestrictedToken 无法可靠报告刚才拒绝的对象路径，因此采用**声明式路径能力前置审批**：shell 调用附带 workspace/session temp 之外所需的额外写路径；GUI 在执行前以普通用户能理解的“行为 + 目标”展示实际能力。批准后只扩展本次 capability 或持久路径规则，命令仍在 RestrictedToken 下执行。未声明的外部写只失败，不提供模糊的整命令放行。普通 NTFS ACL 无法完整强制 workspace 内尚不存在文件名/glob 的 shell ask/deny，这项取舍必须在开发文档和发布说明中明确，不加入普通用户的具体审批卡片。 |
| **shell / manual** | 无 OS 沙盒，使用只读白名单：`ls/cat/grep/git status` 等免问，其余命令弹 `shell_command` 卡片前置审批（仅“允许一次”，不落规则）。 |

推论：macOS Seatbelt 与手动档仍无法自动获得 shell 将访问的完整路径；Windows write-protect 后端则通过 shell 调用显式声明 `additional_permissions` 建立路径级前置审批。它不是命令前缀规则，也不尝试静态解析 PowerShell，而是把模型声明的路径能力交给同一套规则引擎评估。未声明访问继续由 OS 写边界拒绝。

## 6. 审批 UI

### 6.1 共享审批卡片

桌面端的 macOS、Windows 和手动审批档复用现有 `ApprovalPrompt`，不为 Windows 新建一套对话框。移动端保留原生 `PendingApprovalCard`，但必须消费同一份可信语义 payload 并遵守相同展示原则。普通用户界面只回答两个问题：**FutureOS 要做什么**、**要对什么目标做**。单目标时由可信后端把两者合成标题，例如：

> **允许 FutureOS 在 `D:\Release` 中管理文件吗？**

标题已经包含“行为 + 目标”，不再在正文重复列出“行为 / 目标 / 用途”。卡片默认只显示标题和决策按钮；命令、文件预览等辅助信息放在“查看命令 / 查看详情”折叠区。模型提供的 `reason` 只能作为可选辅助说明，不能决定标题、行为或授权范围。

多目标请求无法只靠标题说清时，标题写行为和数量，下面完整列出目标，例如“允许 FutureOS 在以下 3 个位置创建和修改文件吗？”。单次最多 8 个目标且不得显示“另有 N 项”；超过上限由 agent 合并成用户能理解的父目录范围或拆成多个独立请求。

按钮语义保持三档，具体文案后续可继续打磨：

```
[ 不允许 (Esc) ]    [ 仅允许这一次 (⌘↵) ]    [ 此项目以后都允许 ]
```

- **不允许**：该次访问失败，错误返回模型。
- **仅允许这一次**：只批准当前冻结的行为和目标。
- **此项目以后都允许**：GUI 将同一行为和目标保存为 workspace allow 规则并当轮即时生效。普通审批卡片不显示或编辑 glob、规则文件名等技术结构；高级规则编辑留在设置页。**命中敏感文件守卫的路径不提供此选项**（守卫不可覆盖，只能允许一次）。
- user 级规则不从弹窗产生，留给设置菜单（后期）。v1 的「本会话允许 / 始终允许」按钮被本设计取代。

并非每种卡片都有三个按钮：macOS 整命令 escalation、手动 shell 命令和不可持久化的敏感路径只提供“不允许 / 仅允许这一次”。多目标请求按整组批准或拒绝；普通用户不在卡片里删除路径、调整 scope 或编辑规则。若只愿批准一部分，应拒绝本次请求，由 agent 拆分后重新申请。

移动端第一版继续只提供“不允许 / 仅允许这一次”；它不能把缺少持久化 API 的“批准”伪装成“此项目以后都允许”。后续若增加持久允许，必须复用同一保存规则与 session 注入语义，不能只在手机本地记住决定。

审批渲染必须 fail closed：若 action payload 缺少可信行为或目标、类型不匹配、解析失败，卡片显示“无法确认操作内容，已阻止”，不显示批准按钮。不得回退显示原始 JSON 后继续允许用户批准。

### 6.2 当轮即时生效

GUI 写完文件后 agent 下一轮 prompt 才重读。为避免同轮内同一路径再问一次：审批决策回传时（`approval_decision`）附带所保存的规则，agent 把它注入**当前 session 的内存规则集**（类似现有 `approve_outside_path` 机制），即刻生效。

### 6.3 escalation 卡片

macOS 继续复用同一张卡：标题直接说明“允许 FutureOS 在沙箱外运行此命令吗？”，原命令放入“查看命令”；只有会改变用户判断的风险才保留为简短提示，例如“允许后，本条命令将不受沙箱限制运行一次”。macOS escalation 不提供“此项目以后都允许”（它是命令级一次性放行，不对应可靠的路径规则）。

Windows 不复用这种“整条命令脱沙盒”批准语义；它使用 §6.4 的路径能力卡片。若 Windows 命令仅返回 `Access is denied` / error 5 而没有预声明路径，GUI 只显示失败和“请明确声明需要访问的路径后重试”，不出现批准按钮。stderr 中解析出的路径只能标记为 `suspected` 提示，不可直接生成授权。

### 6.4 Windows 路径能力审批（后端/UI 契约与受限执行接线已实现，真机矩阵待完成）

Windows `sandbox` 档的 shell 调用新增显式能力请求；示意结构：

```json
{
  "command": "Copy-Item C:\\build\\artifact.zip D:\\release\\artifact.zip",
  "additional_permissions": {
    "write": [
      { "path": "D:\\release", "scope": "subtree", "reason": "在发布目录创建产物" }
    ]
  }
}
```

审批数据严格分成两层：

| 层 | 内容 | 是否展示给普通用户 |
|---|---|---|
| **可信语义层** | 后端生成的行为、显示目标、单文件/目录范围、是否可持久允许 | 是；用于标题、必要的多目标列表和按钮 |
| **强制与审计层** | 原始路径、规范化绝对路径、实际 ACL 根、规则层与判定、backend、`request_id`、command hash、SID、reparse/TOCTOU 校验信息 | 否；只在执行器、审计日志和开发诊断中使用 |

原始命令放在折叠的“查看命令”中。模型给出的 `reason` 可作为内部重试上下文或简短辅助说明，但不能替代可信语义层。Windows“读取和网络不由写保护限制”的能力差异放在模式选择、首次启用说明和设置页，不在每次具体写路径审批中重复干扰用户。

后端必须按**实际授权能力**生成行为文案：已存在单文件且只授内容写入时可说“修改文件”；目录 `subtree` 能力必须说“在该目录中创建、修改、重命名和删除文件”，不能只显示含义不明的“write”。NTFS 无法只授权创建某个尚不存在的文件名；创建、替换或 rename 必须申请父目录能力，并让标题中的目标就是这个父目录，禁止界面显示文件名而后台静默扩大到父目录。

路径列表必须完整；单次最多 8 项，超过则拒绝请求并要求收敛为用户可理解的父目录 `subtree` 或拆分。多个子路径只有在模型明确请求同一父目录 `subtree` 时才允许合并。

卡片按钮与能力语义：

| 操作 | Windows 行为 | 持久化 |
|---|---|---|
| **不允许** | 不启动命令；返回被拒路径列表 | 无 |
| **仅允许这一次** | 将已批准路径作为本次 ephemeral write capability 加入 `WindowsSandboxPlan`，重新做规则校验后在 RestrictedToken 下启动 | 仅本次 command hash；不落规则文件 |
| **此项目以后都允许** | 直接保存卡片所表达的同一行为和目标，写入 workspace allow-write 规则并注入 session；重新编译计划后仍在 RestrictedToken 下启动 | `${WORKSPACE}/.future/approval_rule.json`；仅当前项目 |

以下情况不提供“此项目以后都允许”：命中第 0 层、命中不可覆盖守卫、路径是规则文件/agent 配置、路径无法规范化、路径经过无法安全解析的 reparse point、请求是磁盘根/用户根等过宽范围。`deny` 规则默认不允许通过审批覆盖；只有 `ask` 才产生能力卡片。

批准前后必须使用同一组规范化路径：agent 生成 approval payload 后冻结行为和目标集合；GUI 只能对整组批准或拒绝，任何扩大、缩小、增删都必须产生新的 `request_id` 并重新评估。执行器应用 ACL 时再次按 handle 校验最终路径，发现 reparse target 改变则使批准失效。

路径能力只是在 RestrictedToken 的第二次写检查中增加许可，**不会提升当前用户本来没有的 NTFS 权限**。若正常用户 SID 对目标也无写权，批准后仍应返回原生权限错误，不得尝试改 owner、绕过 DACL 或请求 UAC。

本期 Windows 后端明确是 Unelevated：shell 仍以当前真实用户为主体，路径能力卡表示“允许这次内容写入范围”，不表示已经获得 Elevated 独立用户等级的防删除保证。`scope=file` 不授 capability `DELETE`，但当前用户对父目录已有的 `FILE_DELETE_CHILD` 仍可能允许删除目标；既有 Everyone/Logon/`Users`/`Authenticated Users` 宽 ACL 也属于知情边界。普通卡片不堆叠这些实现细节，但开发文档、测试与 review 不得把“修改文件”解读成“OS 已绝对禁止删除”。完整边界与单专用用户后续路线见 SANDBOX_PLAN §11.6、§11.10。

## 7. 启用范围与三档审批

审批以**单一枚举 `tier`** 表达，session 建立时经 `set_sandbox_policy { tier }` 下发（proto `string tier`）。三档：

| 档位（UI 名） | `tier` | read/write/edit 工具 | shell | OS 沙盒 | 平台 |
|---|---|---|---|---|---|
| **手动审批** | `manual` | 按规则 ask/allow/deny（默认档） | 只读白名单免问，其余弹卡片前置审批 | 无 | 全平台（默认） |
| **沙箱保护** | `sandbox` | 按规则 ask/allow/deny | Seatbelt 包裹自动跑，越界经命令级 escalation | 读写规则均由 Seatbelt 强制 | macOS |
| **写保护（Windows）** | `sandbox` | 按规则 ask/allow/deny | RestrictedToken 自动跑；额外写路径走 §6.4 能力审批 | **只强制写边界**；读和网络开放 | Windows（后端完成并通过 smoke 后显示） |
| **完全放开** | `off` | 全放行、不问 | 直跑、不问 | 无 | 全平台 |

- **默认 `manual`**，全平台一致。Windows 在 unelevated 后端完成并通过 smoke 之前不显示 `sandbox`；完成后以“写保护”名称显示，不能沿用 macOS“凭证拒读也受 OS 强制”的说明。Linux 仍不显示。
- 若某会话下发 `sandbox` 但平台无 `sandbox-exec`（`available=false`），`wraps_shell()` 为假，shell 退回**手动审批档的白名单行为**（安全兜底），工具审批照常。
- `off` 档在 **agent 层**就不再发审批请求（`request()` 直接放行），前端不再有"自动点批准"的补丁逻辑。
- TUI / CLI / channels 走各自的 `permission_level` 语义，规则文件与沙盒不参与（等同 `off` 的对外行为，但保留既有工作区边界）。

v1 的 `read-only / workspace-write / danger-full-access` 三种模式与 `untrusted / on-request / never` 三档策略**收敛为这一个三态枚举**。这是"配置简单易懂"的直接体现。

## 8. 安全边界与已接受的取舍

**这套模型保证的**：
- 凭证/隐私文件在 read/write/edit 工具层有前置闸；macOS shell 由 Seatbelt 硬拦。Windows write-protect shell **不保证拒读**，必须在 UI 中明确区别。
- 写破坏半径在 macOS 由 Seatbelt、Windows write-protect 档由 RestrictedToken + ACL 限制到 workspace、session temp 和已批准写能力。
- read 工具漏洞补上（v1 它可无审批读 `~/.ssh` 喂给模型）。
- 规则文件自提权被第 0 层堵死。

**明确接受的取舍**（均已拍板）：
- **网络完全放开**：未列入清单的 workspace 内密文可被读取并经网络外发，防线只有内置清单的覆盖度。换来的是 npm/pip/git 等零打扰。
- **弱命令级护栏**：手动审批档只有一个**只读白名单**（`shell_auto_allow`：`ls/cat/grep/git status` 等；含重定向/管道到非只读命令/`&&`/命令替换一律落到"问"）——它是免打扰闸，不是安全边界。非白名单命令弹卡片但仅"允许一次"，用户点批准后 `git push --force`、`npm publish`、`rm -rf .` 照跑（Shadow Review 事后可见）。命令级持久 allow/deny 整体不做。
- macOS escalation 批准 = 整条命令出 Seatbelt（含其一切文件访问），非精确放宽；Windows write-protect 不采用此语义。
- Linux 暂无 OS 强制。Windows write-protect 第一版接受 shell 读取和网络开放、glob 不可转 NTFS ACE、以及 NTFS deny-wins 与规则层级不能完全等价；详见 SANDBOX_PLAN §11。

## 9. 决策记录（v2，2026-07-04）

| # | 决策 |
|---|---|
| V1 | 审批对象从"命令 + 路径"收敛为**纯文件路径**；命令级规则（command_prefix）整体移除，先试用再评估 |
| V2 | 网络访问完全放开、不审批；不做域名过滤（若将来确需，经本地代理层实现，届时再加规则类型） |
| V3 | `ask` 对 shell 编译为 deny + escalation 兜底；对 read/write/edit 工具做真前置审批 |
| V4 | 兜底读写分车道：读默认 allow，写默认"workspace 内 allow / 外 ask" |
| V5 | 第 0 层安全覆盖：规则文件与 agent 凭证文件的写恒 deny，不可被 workspace/user 层覆盖 |
| V6 | agent 直接读两个规则文件；仅 GUI 会话启用，TUI/CLI/channels 保持现状 |
| V7 | 弹窗三选项：拒绝 / 允许一次 / 本工作区允许（写 workspace 文件 + 当轮内存注入）；取代 v1 的"本会话/始终允许" |
| V8 | 三模式 × 三策略收敛为单一 `tier` 三态枚举（`manual`/`sandbox`/`off`）；`off` 在 agent 层即不发审批 |
| V9 | 接受"网络放开 + 清单外密文可泄"与"弱命令级护栏"两项残留风险 |
| V10（2026-07-05） | 审批分三档：手动审批（默认，全平台）/ 沙箱保护（仅 macOS，shell 走 Seatbelt）/ 完全放开。非 mac 不显示沙箱档，无"降级"概念。手动档复活 shell 只读白名单免问（Option B） |
| V11（2026-08-21） | Windows `sandbox` 定义为 unelevated **写保护**，不声称具备 macOS Seatbelt 的 shell deny-read；UI 必须明确能力差异 |
| V12（2026-08-21） | Windows shell 额外写权限必须声明具体路径；批准只扩展对应 capability 并保持 RestrictedToken，不以笼统 error 5 触发整命令脱沙盒重跑 |
| V13（2026-08-21） | 所有平台复用 `ApprovalPrompt`；普通卡片由可信后端用标题表达“行为 + 目标”，单目标不重复字段，技术 enforcement 数据不展示，原始命令折叠；多目标最多 8 项且整组决策，payload 无法可信解析时 fail closed |
| V14（2026-08-25） | Windows 本期范围冻结为 Unelevated；真实用户主体、`FILE_DELETE_CHILD` 与既有宽 ACL 是知情能力边界，不以 Elevated 保证验收本期。后续若升级独立安全主体，优先单独评估一个专用本地用户，不混入当前 PR |

沿用 v1 未变的决策：macOS escalation 按命令放行（原 Q2）、`.git` 不排除（Q4）、`sandbox-exec` deprecated 风险接受（Q5）、失败特征启发式保守（Q6）、temp 目录读写全开。Windows 改用路径能力放行；Linux 后续再做。
