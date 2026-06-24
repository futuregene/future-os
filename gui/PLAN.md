# FutureOS GUI Development Plan

更新时间：2026-06-24

`P0 / P1 / P3 / P4 / P7` 主路径已可用；`P2` 数据模型与结构化展示就位；Shadow Review（Run 级「上一轮变更」）Phase 1/2 已落地。下一阶段重点是把 `P5 / P6` 从占位变可用，并补 `P9` 测试与打包可靠性。产品语义见 `PRODUCT.md`，数据模型见 `ER.md`。

> 本文件只覆盖 `gui/` 的开发路线。Shadow Review 的产品语义见 PRODUCT.md §4.7，数据模型见 ER.md §4.10，设计取舍见 ER.md §6.8。

## Current Baseline

GUI 当前已具备：

- `gui/` 独立作为 Tauri + React + TypeScript 模块运行。
- GUI 通过 `future-agent` gRPC 发送 prompt、接收事件、提交 approval 决策和 abort 请求。
- 模型列表从 agent 获取，不使用前端硬编码模型。
- Chat / Workspace thread 可创建、恢复、重命名、置顶、归档、删除。
- 普通 Chat 自动创建临时 workspace。
- 新建空白对话：中间区只保留标题，无多余 header 操作按钮；新对话未落地前不展示右侧上下文面板。
- 右侧面板按对话类型切换（详见 PRODUCT.md §5.4）：
  - Workspace 对话：Runs / Review（不展示 Artifacts）。
  - 普通 Chat：Runs / Artifacts（不展示 Review）。
- Runs 面板收敛为后台程序列表：
  - 只展示有 shell command 的后台程序。
  - 运行中蓝点，完成 / 失败 / 取消灰点。
  - 支持运行中终止、二次确认、终止失败提示、清空已完成程序。
  - 支持进入 Run Inspect。
- Approval 位于 composer 上方：
  - 同时只展示一个待审批项。
  - 不超时，始终等待用户允许或拒绝。
  - `Esc` 拒绝，`Cmd/Ctrl + Enter` 允许一次。
  - 输入框聚焦时不误触发审批快捷键。
  - GUI 启动时取消旧 pending approval。
- Agent run 主路径已具备基础防护：
  - thread 切换 / 组件卸载时清理流式轮询。
  - 在途请求不写错当前 UI 状态。
  - 同一 session 并发 prompt 会被拦截。
  - 失败 assistant message 提供 Retry / Continue。
  - Continue 会注入最近 run 的工具与关键事件摘要。
- Agent 连接状态已进入可用基线：
  - Header 展示 connected / checking / not running / model unavailable。
  - Composer 上方展示 agent 不可用提示和 Retry。
  - 提示 `make run-agent` 作为本地启动命令。
- Run Inspect 已进入可用基线：
  - 展示 run 状态、模型、时间、错误信息。
  - 展示 tool call、tool output、timeline。
  - tool input / output 支持复制，长 output 可展开。
  - timeline 支持分类过滤、摘要 / 原始 payload 切换、全文搜索。
  - tool 详情展示 command、cwd、path、exit status、duration、stdout、stderr 等可提取结构化字段。
- Review 已进入可用基线（已切换到 Shadow Review 双数据源模型）：
  - 覆盖所有 Workspace 对话；普通 Chat 不展示 Review。
  - **Git changes**（仅 Git workspace）：基于真实 `git diff` 展示工作树相对所选 base（默认 `HEAD`）的未提交变更，含文件级 additions / deletions、未跟踪文本文件、hunk 行号、diff base 选择。
  - **上一轮变更**（两类 workspace 都有）：当前 Thread 最近一轮结束 Run（completed / failed / cancelled）的 before/after 影子快照 diff，含 `bash` 改的文件；运行中 Run 不进入。
  - 顶部下拉切换视图（非 Git workspace 只有「上一轮变更」）。
  - diff 只做 unified；文件默认收起，顶部「全部展开 / 全部收起」开关；二进制文件专用行；命中凭证规则的敏感文件只标记「内容未保存」。
  - 状态横幅：overlapped（并发）/ recovered（重启恢复）/ partial / incomplete / unavailable / 目录过大（非 Git 体积红线）。
  - Review embed 可打开右侧 Review。
- Artifacts 已进入可用基线：
  - 仅用于普通 Chat；Workspace 对话文件由 Review 管理。
  - workspace 外写入不计入 Artifacts。
  - 可展示、软删除、导出、打开文件、复制 path/content、转入 Research。
  - 支持 artifact 详情视图、inline 预览、文本文件预览、图片预览、PDF 预览。
  - 图片加载失败、类型不支持时显示 fallback。
  - PDF 预览支持页码导航、自适应缩放、Canvas 渲染。
- Markdown 已进入可用基线：
  - 使用 `unified` / `remark-parse` / `remark-gfm` 管线。
  - 支持常用 Markdown、GFM table/task list、code fence 复制。
  - 支持 `futureos://artifact|run|tool|approval|review|research/<id>` inline reference。
  - 支持 `futureos-artifact|run|tool|approval|review|research` fenced block embed。
  - 引用解析限制在当前 workspace，过滤已删除 artifact。
  - 引用状态使用 React 原生数据驱动模型更新。
  - 图片失败态和外链安全属性已补齐。
  - 代码块支持语法高亮（Shiki，28 种语言，GitHub Light 主题，行号显示）。
- Composer 支持 `@` 搜索并插入 FutureOS markdown 引用。
- 发送给 Agent 的 prompt 会注入引用对象摘要，消息原文仍保存 markdown。
- Store 已启用 SQLite `foreign_keys`、`busy_timeout`、`WAL`；schema 为单一真源、幂等应用（`IF NOT EXISTS`，无增量 migration 历史）。
- Settings 已可用：General、Models、Providers、自定义 Provider 配置。
- Research 可展示当前 workspace resource，并支持从 markdown embed 跳转选中 resource。
- Data、Skill 仍是入口级占位（`AppShell` 的 `ModulePlaceholder`）。

## Priority Pass Status

### P0: Run Reliability And Recovery — 主路径完成

已具备：Retry / Continue；失败 run 可见错误信息；Agent 连接状态提示和 retry；agent 未运行启动提示；abort 基础链路和本地 run 状态更新；后端 `run_error.rs` 结构化错误分类（stream 断开 / 命令失败 / 模型失败 / abort / timeout）。

后续细化：

- 端到端验证 abort 是否真实终止底层 shell 子进程。
- 等待审批、LLM streaming、shell command 的中断自动化测试。

### P1: Run Inspect And Tool Output — 主路径完成

已具备：Run Inspect 详情视图；tool call / output / timeline；搜索、复制、展开、事件过滤；command/cwd/path/exit status/duration/stdout/stderr 结构化展示。

后续细化：

- 长 stdout/stderr 的专门 output storage。
- 二进制或不可 UTF-8 输出 fallback。
- Read / Write / Edit tool 的专属恢复动作和入口。

### P3: Review Workflow — 主路径完成（已重构为 Shadow Review）

当前形态见上方 Baseline 与下面的「Shadow Review」章节。

> ⚠️ 旧 PLAN 的若干 P3 项随 Shadow Review 重构**已作废**：`file search`、`viewed 进度`、`pending/applied/discarded` 状态流、`真实 apply/revert`、`按 run 筛选 changesets`。新模型「上一轮变更」是**只读信息性视图**，不再做 apply/discard 决策流。

后续细化（仍有效）：

- 专门审查大视图（脱离右侧窄面板的全屏 diff 审查）。
- 大文件 diff 按需加载（突破固化时的展示截断）。

### P4: Artifacts And Chat Cleanup — 主路径完成

已具备：普通 Chat 的 artifact 列表；artifact 详情、打开、复制、导出、删除、转 Research；inline / text / image / PDF 预览；preview fallback；workspace 外文件不计入 Artifacts；PDF 页码导航、自适应缩放、Canvas 渲染（PDF.js）。

后续细化：

- 表格预览。
- PDF 缩放控制、全屏模式、文本选择和复制、搜索功能。
- 大 artifact 的文件存储策略和 metadata。
- 临时 workspace 删除前的清理摘要和保留 / 导出流程。
- 文件缺失、过大时的更细恢复操作。

### P4b: Attachments (Images) — 主路径完成

目标：输入框图片附件作为多模态输入，落盘与 Artifact 归属按对话类型判断（详见 PRODUCT.md §4.12）。

已具备：落盘 / Artifact 归属按对话类型判定（Chat 落盘并登记 Artifact，Workspace 对话直传路径）；三种上传方式（附件按钮 / 复制粘贴 / 拖拽）；入口即限制图片类型；每轮最多 4 个文件。

后续细化：

- 粘贴 / 拖拽产生的临时图片文件的清理策略。
- 超出 4 个或非图片时的更明确用户提示。

### P7: Markdown And Object Embeds — 主路径完成

已具备：标准 Markdown + GFM；FutureOS inline reference 和 block embed；React 原生数据驱动引用刷新；code fence copy；代码块语法高亮（Shiki）；各类 embed 基础交互；图片失败态和外链安全属性。

后续细化：

- Tool / Review / Artifact embed 的更丰富摘要。
- 更多 store 更新来源和跨窗口 Tauri event bridge。
- Markdown export 策略。
- Agent 输出约束继续加强，鼓励优先输出 `futureos://` 引用。

### P2: Approval Model — 数据模型与结构化展示完成

已具备：结构化 `action` payload（tool / category / command / paths / writes / deletes / scope）；`sandbox_boundary` 字段；`reviewer` / `decision_scope` / `decision_source` 字段（当前固定 `user` / `once` / `user`）；`sandbox_config` / `approval_policy_config` / `approval_rules` 三张预留表；Agent 侧 `approval_policy.rs` 策略评估桩；ApprovalPrompt 按 category 渲染结构化卡片 + 沙盒越界警告徽章；action 提取 / sandbox_boundary 计算 / policy 桩的单测。

> 预留表与桩点已就位但**未接线**（CLAUDE.md 原则 8 标注勿删）。设计细节见 ER.md §4.8 与 git history（原 `P2_APPROVAL_MODEL.md`）。

后续细化：

- 沙盒执行（`sandbox_config` 接入 Agent，沙盒内自动通过）。
- 自动审批策略（`evaluate_policy` 实现规则匹配）。
- 决策范围扩展（session / always 按钮和规则缓存）。
- Settings UI（沙盒、策略、规则三个配置面板）。
- `auto_review` reviewer（审查 agent）。

### Shadow Review：Run 级「上一轮变更」— Phase 1 / 2 完成

后端模块 `src-tauri/src/shadow_review/{repository,snapshot,diff,policy}.rs` + `store/review_snapshots.rs`。

已完成：

- Phase 1 核心闭环：Run 前后 before/after 影子快照 → after 立即固化 diff 入 SQLite（真源）→「上一轮变更」可看；Git / 非 Git 两档；零变化 changeset；移除旧 `write/edit` 推测投影与 auto `git init`；并发 Run `overlapped` 标记。
- Phase 2 Git 可靠性：`isStreaming` 二次确认（`wait_for_agent_idle`）；重启恢复（`confidence=recovered`）；retention（每 Thread 留 10 + prune + `git gc --auto`）；敏感文件 metadata-only；启动一致性检查（commit 丢失标 `unavailable`）；rename/copy 检测。
- Phase 3 部分：C1 finalize 异步化（capture 同步 + materialize `spawn_blocking`，`review-updated` 事件刷新）；C3 整树 git diff 仅在 Review tab 激活时跑；E1 删除旧 apply/discard 死代码。

后续：

- C4 大文件 diff 按需加载（与 P3「审查大视图」合并推进）。
- monorepo 子目录 pathspec（当前子目录被判为非 Git，只走「上一轮变更」，不显示原生 Git changes）。
- **已决定不做**：C2 fingerprint cache（持久化 shadow index 的 stat 增量已等价覆盖）；DiffView split 双栏。

## Next Priorities

当前优先 pass 已完成。下一阶段建议按以下顺序推进：

1. **P10 Provider 管理**：见下方「Provider 配置现状」。GUI 内 FutureGene 登录、提供商唯一性校验已完成；剩余的"模型按 `provider/id` 复合标识"需改 agent，单列为待办。
2. **P9 测试 / 打包**：扩前端组件与集成测试（现仅 3 文件 / 15 用例）；approval / streaming / abort 中断测试；DMG / 签名打包在真实 macOS 或 CI 验证。
3. **P8 Composer 引用**：引用 UX 打磨与统一 registry，复用 P7 的 embed 摘要增强。
4. **P2 Approval 后续细化**：沙盒执行、自动审批、决策范围、Settings 配置面板、`auto_review`。

并行可插入：Shadow Review C4 / monorepo 子目录；各 pass 的「后续细化」遗留项。

### 已下调优先级（入口已从左侧导航隐藏）

Research / Data / Skill 暂不投入，左侧导航图标已隐藏（`ActivityRail` 的 `featureItems` 置空）；后端 section 处理与 markdown research embed 跳转保留，恢复时把导航项加回即可。

- **P5 Research**：resource 创建、详情、collection 管理（现状仅单一展示视图 + embed 跳转）。
- **P6 Data / Skill**：从占位变最小可用（Data 源 CRUD + 凭证；Skill 列表 + global/workspace 启用）。Settings 已毕业，不属于此 pass。

## Provider 配置现状（P10 基线）

模型 Provider 配置落在 agent 的两个文件：`~/.future/agent/models.json`（providers + models，合并在内置 catalog 之上）与 `~/.future/agent/auth.json`（按 provider id 存 API key）。

- **CLI**（`cli/src/commands/auth.ts`）：只有 `future auth login / status / logout`，对内置 **FutureGene** provider 做设备码 OAuth（`/v1/oauth/device/code` → 轮询 `/v1/oauth/device/token`），把 `api_key` 写进 `auth.json` 的 `future` 条目。**CLI 没有添加自定义 Provider 的命令，login 也不写 models.json。**
- **GUI**（`agent_providers.rs` + `commands/providers.rs` + Settings ▸ Providers）：已具备**自定义 Provider 全量增删改**。
  - `list_agent_providers`：内置 FutureGene（只读，base_url 取 `auth.json.future.base_url`）+ 自定义（读 `models.json.providers`）。
  - `upsert_custom_provider`：写 `models.json.providers.<id>`（name / api / baseUrl / models，保留 `compat` 等未管理字段）+ 可选 API key 写 `auth.json.<id>`。
  - `delete_custom_provider`：从两个文件移除。
  - `CustomProviderDialog`：id / 名称 / API 类型（openai-completions / openai-responses / anthropic）/ Base URL / API Key / 模型列表。

### 已完成

- **FutureGene 登录**（设计见 LOGIN.md）：GUI 内独立设备码 OAuth（`future_login.rs` + `auth_store.rs` + `FutureLoginDialog`），Providers 页「连接 / 重新登录 / 退出登录」。不再依赖 CLI 登录。
- **提供商唯一性校验**：`upsert_custom_provider` 新增 `create` 标志——新建时 id 已存在则报错（防静默覆盖）；名称跨内置 + 自定义大小写不敏感去重。前端 `CustomProviderDialog` 同步即时校验。
- **`future` 过滤**：`list_agent_providers` 自定义区跳过 `future` id，避免手改 models.json 时 FutureGene 重复显示。

#### 自定义 Provider 字段校验

前端 `CustomProviderDialog` 即时校验 + 后端 `upsert_custom_provider` 权威兜底（两边规则一致）：

| 字段 | 必填 | 规则 | 长度 | 归一化 |
|---|---|---|---|---|
| Provider id | 是 | `^[a-z0-9_-]+$`，且 ≠ `future` | 2–40 | trim；**转小写** |
| 名称 name | 否（空则回退 id） | ASCII：字母/数字/空格/`_.()-`（不支持中文 / emoji / 全角） | ≤40 | trim |
| API 类型 | 是 | 枚举 `openai-completions` / `openai-responses` / `anthropic` | — | — |
| Base URL | 是 | 可解析且 scheme ∈ {http, https}（不强制 https） | ≤2048 | trim |
| API Key | 否 | ASCII、无控制字符 | ≤512 | trim |
| 模型 id（每项） | 行内是 | `^[A-Za-z0-9._:/-]+$`（允许 `/ : .`，无空格） | ≤100 | trim |
| 模型 name（每项） | 否（空则回退 model id） | ASCII、无控制字符 | ≤60 | trim |
| 模型条数 | — | ≤100 | — | — |

唯一性：新建时 id 不可与现有重复（防静默覆盖）；名称跨内置 + 自定义大小写不敏感唯一；同一 provider 内模型 id 不重复。

### 待办（需改 `agent/`，本期不做）

> `agent/` 是 TUI / CLI / channels 共用后端，以下改动会波及它们，故单独记录、暂缓。

- **模型按裸 id 去重 → 同 id 跨 provider 互相覆盖**：agent `all_models()`（[`agent/src/models/mod.rs:646`](../agent/src/models/mod.rs)）与 `new()`（[`:596`](../agent/src/models/mod.rs)）按裸 `id` 去重，user/custom 模型会**静默顶掉** FutureGene 的同 id 模型；`list_models` 经 `all_models()` 返回（`agent/src/rpc/commands.rs:851`），GUI 拿到的已是合并后的列表。
- **模型标识应复合化为 `provider/id`**：GUI 目前用裸 id 选择/解析模型（`agentClient.ts` `modelOption`、Composer `onModelChange(model.id)`、`thread.modelId`）。彻底解决"同名模型"需端到端用 `provider/id`（agent `resolve()` 已支持 `provider/id`，见 [`models/mod.rs:659`](../agent/src/models/mod.rs)），并让 TUI/CLI 也传复合 id。
- 影响评估与迁移（旧 `thread.modelId` 裸 id 兼容）需在动手前单独成文。
- **`enabledModels` 白名单会挡住新登录 provider 的模型**：agent `list_models` 在 `settings.json.enabledModels` 非空时只返回白名单匹配项（`rpc/commands.rs:843` → `resolve_scope`）。GUI 用的是自己的 `hiddenModels`（opt-out），从不写 `enabledModels`，所以当 `enabledModels` 非空（如旧 TUI/预置配置）时，FutureGene 登录后其模型虽被动态拉取却被过滤掉。解法二选一：登录成功后让 GUI 调 `set_enabled_models` 把 `<provider>/*` 并入；或 GUI 模式下 list_models 不受 `enabledModels` 限制（改 agent / 设计）。临时绕过：给 `enabledModels` 加 `future/*` 或清空。
- **次要**：新增/改 Provider 后 agent 需新会话或重启才加载新模型（Dialog 已提示）；自定义 provider 的 `compat` 字段 GUI 不可编辑。

## Out Of Current Scope

- 根 npm workspace。
- 根 Cargo workspace。
- 迁入源项目 Rust CLI。
- 将 Agent 编译进 GUI。
- 多 Agent 服务复杂切换 UI。
- 复杂数据分析工作台。
- 多数据库写入。
- Skill marketplace。
- 复杂文档、表格、多模态 artifact 的细粒度 review。
