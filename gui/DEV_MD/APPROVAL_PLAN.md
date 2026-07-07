# FutureOS 审批规则方案（APPROVAL_PLAN）

状态：**方案 v2 已实现**（定稿 2026-07-04，落地见 SANDBOX_PLAN R1/R2/R3 ✅、`src-tauri/src/approval_rules.rs`、`commands/approvals.rs`；取代 SANDBOX_PLAN v1 的"规则引擎 + 审批策略"部分）

> 本文是审批系统的**语义主文档**：规则模型、规则文件、判定流程、审批 UI。OS 层如何强制执行（Seatbelt 编译、escalation、降级模式、返工计划）见 [SANDBOX_PLAN.md](SANDBOX_PLAN.md)。产品语义基线 PRODUCT.md §4.6 需在实现后同步。

## 0. TL;DR

- **一切审批围绕"文件路径访问"**：读/写哪个路径，按分层规则得出 `ask / allow / deny`。**网络访问完全放开、不审批**；命令本身不再按前缀审批（纯文件模型）。
- **四层规则，首个匹配即返回**：内置安全覆盖（不可覆盖）→ workspace 规则文件 → user 规则文件 → 内置默认 → 兜底（读全放、写限 workspace）。
- **两个规则文件**：`${WORKSPACE_DIR}/.future/approval_rule.json`（项目级，可进 git）和 `~/.future/approval_rule.json`(用户级)。JSON、可手改、agent 直接读取。
- **审批弹窗三个选项**：拒绝 / 允许一次 / **本工作区允许**（把 allow 规则写进 workspace 规则文件，下次不再问）。
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
  - `ask`：问用户。**在 read/write/edit 等工具调用上是真正的前置弹窗**；在 bash 里无法逐文件中途询问，**编译进沙盒时按 deny 处理**，命令失败后走 escalation（那一次失败-询问就是 bash 版的 "ask"，见 §5）。

## 3. 分层与优先级

从上到下评估，**第一个匹配立即返回**：

```
第 0 层  内置安全覆盖（程序内置，workspace/user 都盖不过）
         · 两个规则文件本身、agent 凭证文件的【写】 → deny
第 1 层  workspace 规则文件  ${WORKSPACE_DIR}/.future/approval_rule.json
         · 用户自配 ask/allow/deny；「本工作区允许」写进这里
第 2 层  user 规则文件       ~/.future/approval_rule.json
         · 用户自配 ask/allow/deny；后期经设置菜单编辑
第 3 层  内置默认（程序内置，可被 1/2 层覆盖；无 deny，只有 ask 和 allow）
         · 凭证/隐私文件 → ask（access: both）
         · 临时目录（$TMPDIR、/tmp）→ allow（读写全开）
第 4 层  兜底（读写分车道）
         · read  → allow                          （读默认开放）
         · write → 在 workspace 内 ? allow : ask
```

完整判定伪代码（对每次文件访问：路径 P、操作 A ∈ {read, write}）：

```
P ← canonicalize(P)                       # §2 规范化
for layer in [安全覆盖, workspace 文件, user 文件, 内置默认]:
    for rule in layer:                    # 文件内按书写顺序
        if rule 匹配 (P, A): return rule.action
# 兜底
if A == read:  return allow
return P ∈ workspace ? allow : ask
```

### 3.1 第 0 层：防自提权（为什么必须存在）

workspace 规则文件在 workspace 里 = 兜底可写；它又是最高优先级。若无第 0 层，agent 一条 `echo '...' > .future/approval_rule.json` 就能给自己放开 `~/.ssh`。所以：

- **写**以下文件恒 deny，任何层覆盖不了：
  - `${WORKSPACE_DIR}/.future/approval_rule.json`
  - `~/.future/approval_rule.json`
  - `~/.future/agent/auth.json`、`~/.future/agent/models.json`（及 `agent-app` 变体）
- 读规则文件不限制（内容无密，agent 看到规则反而有助于理解边界）。
- 精确到文件、不封整个 `.future` 目录——普通 Chat 的临时 workspace 就在 `~/.future/agent/workspace/` 下，封目录会砸掉 Chat 自己的工作区。
- 修改规则文件的合法通道只有两个：**用户手改**、**GUI 代写**（「本工作区允许」按钮，走 Tauri 可信路径，不经 agent 工具）。
- 已知残留：escalation 批准后整条命令出沙盒跑（SANDBOX_PLAN §escalation），理论上可借此写规则文件——但用户批准时看到的就是那条命令本身，属知情同意，接受。

### 3.2 第 3 层：内置默认清单

凭证/隐私 → `ask`（`access: both`；此层无 deny——内置不替用户做"永不"的决定，用户可在 1/2 层自行 deny 或 allow）：

| 类别 | 路径 |
|---|---|
| SSH / GPG | `~/.ssh`、`~/.gnupg` |
| 包管理 token | `~/.npmrc`、`~/.pypirc`、`~/.cargo/credentials{,.toml}`、`~/.gem/credentials` |
| 明文凭证 | `~/.netrc`、`~/.git-credentials`、`~/.env` |
| 云厂商 | `~/.aws`、`~/.azure`、`~/.config/gcloud`、`~/.terraform.d`、`~/.kube/config` |
| 容器/CLI | `~/.docker/config.json`、`~/.config/gh` |
| Keychain | `~/Library/Keychains` |
| **workspace 内** | `.env`、`.env.*`、`**/*.pem`、`**/*.key`、`**/*.p12`、`**/id_rsa*`（相对 workspace） |

临时目录 → `allow`（`access: both`）：`$TMPDIR`、`/tmp`（canonicalize 后的真实路径）。

> workspace 内凭证条目正是 v1 的"项目内 secret 盲区"（原 SANDBOX_PLAN §2.3.1）的解法：`.env` 进内置 ask，read 工具与 bash 都拦得住；"帮我看下 .env" 场景由用户点一次"允许一次"或"本工作区允许"解决，不再反直觉。

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
| **bash**（子进程，无法预知会碰哪些文件，Seatbelt 只有二态且不能中途问人） | 依审批档位而定（见 §7）。**沙箱保护档**：`ask` 与 `deny` 都编译进 Seatbelt profile 当"拒"，命令直接跑，撞上被拒文件 → 失败 → **escalation 审批**（"命令疑似被沙盒拦截，是否放行重跑"），批准 = 该命令出沙盒跑一次（详见 SANDBOX_PLAN §escalation）。**手动审批档**：无 Seatbelt，改用只读白名单——`ls/cat/grep/git status` 等只读命令免问直跑，其余命令弹 `shell_command` 卡片前置审批（仅"允许一次"，不落规则）。 |

推论：bash 的**路径级**前置审批不存在（Seatbelt/白名单以命令粒度处理）——v1 的 `command_prefix` 路径规则、`untrusted/on-request/never` 三档策略全部退役。手动审批档保留一个**极简只读命令白名单**（`bash_auto_allow`）作为免打扰闸；沙箱保护档则完全交给 Seatbelt。

## 6. 审批 UI

### 6.1 弹窗三选项

```
[ 拒绝 (Esc) ]        [ 允许一次 (⌘↵) ]  [ 本工作区允许 ]
```

- **拒绝**：该次访问失败，错误返回模型。
- **允许一次**：仅本次放行（现状语义）。
- **本工作区允许**：GUI 把一条 allow 规则写入 workspace 规则文件（§4.2），并当轮即时生效（§6.2）。写入前展示**可编辑的规则预览**（path glob + access），防止把窄路径泛化成危险模式。建议模式由 agent 随审批请求给出（`save_suggestion`：目标文件的父目录 glob 或该文件本身）。
- user 级规则不从弹窗产生，留给设置菜单（后期）。v1 的「本会话允许 / 始终允许」按钮被本设计取代。

### 6.2 当轮即时生效

GUI 写完文件后 agent 下一轮 prompt 才重读。为避免同轮内同一路径再问一次：审批决策回传时（`approval_decision`）附带所保存的规则，agent 把它注入**当前 session 的内存规则集**（类似现有 `approve_outside_path` 机制），即刻生效。

### 6.3 escalation 卡片

维持 v1 形态：原命令、失败摘要、justification、"允许后本次将不受沙盒限制"警示。escalation 不提供"本工作区允许"（它是命令级一次性放行，不对应路径规则）。

## 7. 启用范围与三档审批

审批以**单一枚举 `tier`** 表达，session 建立时经 `set_sandbox_policy { tier }` 下发（proto `string tier`）。三档：

| 档位（UI 名） | `tier` | read/write/edit 工具 | bash | OS 沙盒 | 平台 |
|---|---|---|---|---|---|
| **手动审批** | `manual` | 按规则 ask/allow/deny（默认档） | 只读白名单免问，其余弹卡片前置审批 | 无 | 全平台（默认） |
| **沙箱保护** | `sandbox` | 按规则 ask/allow/deny | Seatbelt 包裹自动跑，越界经 escalation | Seatbelt 强制 | **仅 macOS**（其余平台 UI 不显示此项） |
| **完全放开** | `off` | 全放行、不问 | 直跑、不问 | 无 | 全平台 |

- **默认 `manual`**，全平台一致。非 macOS 的下拉/设置页**不出现"沙箱保护"选项**——不存在"降级"概念，mac 只是多一个档位。
- 若某会话下发 `sandbox` 但平台无 `sandbox-exec`（`available=false`），`wraps_bash()` 为假，bash 退回**手动审批档的白名单行为**（安全兜底），工具审批照常。
- `off` 档在 **agent 层**就不再发审批请求（`request()` 直接放行），前端不再有"自动点批准"的补丁逻辑。
- TUI / CLI / channels 走各自的 `permission_level` 语义，规则文件与沙盒不参与（等同 `off` 的对外行为，但保留既有工作区边界）。

v1 的 `read-only / workspace-write / danger-full-access` 三种模式与 `untrusted / on-request / never` 三档策略**收敛为这一个三态枚举**。这是"配置简单易懂"的直接体现。

## 8. 安全边界与已接受的取舍

**这套模型保证的**：
- 凭证/隐私文件（内置清单 + 用户自配）读写有闸——工具层前置问、bash 层 Seatbelt 硬拦。
- 写破坏半径被限制在 workspace + temp（Seatbelt 强制，macOS）。
- read 工具漏洞补上（v1 它可无审批读 `~/.ssh` 喂给模型）。
- 规则文件自提权被第 0 层堵死。

**明确接受的取舍**（均已拍板）：
- **网络完全放开**：未列入清单的 workspace 内密文可被读取并经网络外发，防线只有内置清单的覆盖度。换来的是 npm/pip/git 等零打扰。
- **弱命令级护栏**：手动审批档只有一个**只读白名单**（`bash_auto_allow`：`ls/cat/grep/git status` 等；含重定向/管道到非只读命令/`&&`/命令替换一律落到"问"）——它是免打扰闸，不是安全边界。非白名单命令弹卡片但仅"允许一次"，用户点批准后 `git push --force`、`npm publish`、`rm -rf .` 照跑（Shadow Review 事后可见）。命令级持久 allow/deny 整体不做。
- escalation 批准 = 整条命令出沙盒（含其一切文件访问），非精确放宽（仅沙箱保护档存在）。
- Linux / Windows 无 OS 强制：仅"手动审批 / 完全放开"两档；bash 靠只读白名单 + 卡片审批把关，无 Seatbelt。

## 9. 决策记录（v2，2026-07-04）

| # | 决策 |
|---|---|
| V1 | 审批对象从"命令 + 路径"收敛为**纯文件路径**；命令级规则（command_prefix）整体移除，先试用再评估 |
| V2 | 网络访问完全放开、不审批；不做域名过滤（若将来确需，经本地代理层实现，届时再加规则类型） |
| V3 | `ask` 对 bash 编译为 deny + escalation 兜底；对 read/write/edit 工具做真前置审批 |
| V4 | 兜底读写分车道：读默认 allow，写默认"workspace 内 allow / 外 ask" |
| V5 | 第 0 层安全覆盖：规则文件与 agent 凭证文件的写恒 deny，不可被 workspace/user 层覆盖 |
| V6 | agent 直接读两个规则文件；仅 GUI 会话启用，TUI/CLI/channels 保持现状 |
| V7 | 弹窗三选项：拒绝 / 允许一次 / 本工作区允许（写 workspace 文件 + 当轮内存注入）；取代 v1 的"本会话/始终允许" |
| V8 | 三模式 × 三策略收敛为单一 `tier` 三态枚举（`manual`/`sandbox`/`off`）；`off` 在 agent 层即不发审批 |
| V9 | 接受"网络放开 + 清单外密文可泄"与"弱命令级护栏"两项残留风险 |
| V10（2026-07-05） | 审批分三档：手动审批（默认，全平台）/ 沙箱保护（仅 macOS，bash 走 Seatbelt）/ 完全放开。非 mac 不显示沙箱档，无"降级"概念。手动档复活 bash 只读白名单免问（Option B） |

沿用 v1 未变的决策：escalation 按命令放行（原 Q2）、`.git` 不排除（Q4）、`sandbox-exec` deprecated 风险接受（Q5）、失败特征启发式保守（Q6）、temp 目录读写全开、macOS 最先 / Linux 最后 / Windows 不做。
