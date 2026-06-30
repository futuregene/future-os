# GUI 重构任务清单（REFACTOR.md）

> 本文件是 `gui/` 的重构待办清单，供随时开工。每条目标都**已对照当前代码复核**（行号为复核时实际行号，无编辑发生故稳定），并自带「现状 / 问题 / 改造方案 / 验证 / 关联」，**无需额外上下文即可着手**。
>
> 生成方式：5 个只读分析 agent 按文件域分工，逐条在 live code 复核 + 补全实现细节；复核中修正/撤销的项已在各条「复核结论」标注。

## 如何使用

1. 按下方「优先级与建议顺序」挑一个批次，或直接挑单条。
2. 读该条目的「现状/改造方案」，照「验证」跑命令。
3. 完成后在本文件把该条目标题前的状态从 `[ ]` 改为 `[x]`，并在 commit message 引用其 ID（如 `refactor(gui): C-1 删除死代码 ToolCallBlock`）。

### 状态图例
- `[ ]` 待办　`[~]` 进行中　`[x]` 已完成
- 严重度：**高** / **中** / **低**
- 类别：`module`（模块划分）/ `naming`（命名）/ `ui`（UI 抽象）/ `consistency`（一致性）/ `bug`（缺陷隐患）

### 约定速查（动手前必读）
- 权威文档：产品语义 `PRODUCT.md`、数据模型 `ER.md`、配色 `COLOR.md`、开发指南 `gui/CLAUDE.md`。
- 颜色只用 `COLOR.md` 语义 token，不写裸 Tailwind 色；状态徽章用 `<Badge tone>`；分类色（事件类别 / 错误子类型）是有意例外。
- 所有 `invoke` 走 `integrations/tauri/invoke.ts` 的 `invokeCommand`；跨组件事件走 `lib/futureEvents.ts`；异步加载用 `lib/useAsyncResource`、轮询用 `lib/usePolling`。
- AppShell 只做布局编排，域逻辑进 `components/layout/hooks/`，hook 用同名解构暴露（CLAUDE.md §5）。
- 后端命令返回 `Result<_, AppError>`，不要回退 `.map_err(|e| e.to_string())`。
- **勿删** P2 审批脚手架（`approval_config.rs`、`SandboxConfigRecord` 等，CLAUDE.md §8 预留）。

### 验证基线命令
```bash
# 前端（TS/React）
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run
# 后端（Tauri/Rust）
cd gui/src-tauri && cargo fmt --check && cargo clippy && cargo test
# 视觉 / 交互改动需实机确认
make run-gui
```
> 提交：非交互终端 GPG 会失败，用 `git commit --no-gpg-sign`。

---

## 优先级与建议顺序

按「风险低→高、收益直接→结构性」分 5 批；可整批做，也可挑单条。

**批次 1 — 死代码清理 + 明确正确性 bug（低风险、收益直接）**
`C-1` `C-2a` `C-2b` `C-2c` `C-11`（删死代码）；`B-1` `B-9` `B-2` `B-16`（前端 bug）；`B-5`（git diff bug）。

**批次 2 — UI 收口（一次性大幅减重复）**
`U-1`（删/采纳吃灰组件）→ `U-2` `U-3` `U-4` `U-5` `U-6`（按钮/select/复制/下拉收进 `ui/`）→ `U-7` `U-8` `U-10` `U-11`（焦点环/hover/配色）；`C-10` `M-9`（runs/review 组件去重）；`B-7` `B-8`（Overlay / PdfPreview bug）。

> **进度**：12/14 已落地（见各条 `[x]`）。`U-6`（SelectMenu）与 `M-9`（CollapsibleFileDiff）**暂缓**——属从零抽象/结构性重排，最需实机视觉 QA；且四个下拉结构差异大（Composer model/thinking 一致，但 NewConversation workspace 含搜索+底部动作、Composer reference 是键盘导航搜索菜单），强行统一有过度抽象风险。建议作为独立一次 pass 单独实机验证。

**批次 3 — Rust 一致性收口**
`C-3`（8× RpcResponse 样板）`C-4`（connect 样板 + 默认地址）`C-5`（agent_dir/FUTURE_PROVIDER_ID 去重）`M-6`（git diff 解析合并）`B-15`（错误判定类型化）。

**批次 4 — store 健壮性（事务 / 索引 / 列常量）**
`B-11`（共享连接 + 复合事务）`B-3`（去重竞态）`B-4`（cancel_stale 错位）`B-10`（TOCTOU）`B-12`（缺索引）`C-6`（列常量）`C-7`（元数据构造去重）`C-8`（读回样板 + 状态常量）`N-3`（type 列命名）。

> **进度**：4 个 store **bug** 已落地（`B-3` `B-4` `B-10` `B-12`，见各条 `[x]`）。`B-3`/`B-10` 采用 `BEGIN IMMEDIATE` 事务序列化「检查后写入」，而非加唯一索引——避免唯一索引影响 artifacts 的其它插入路径、以及在已有重复数据上建索引失败。
>
> **「store 一致性」专项 pass 已完成**：`C-8`（`loaded()` 读回 helper + 终态状态常量化）、`C-7`（markdown_refs 元数据 builder 去重）、`N-3`（`type` 列改名 `event_type`/`artifact_type`/`resource_type`）、`C-6`（11 张表列常量推广，逐表机检 ↔ `*_from_row` 顺序）均 `[x]`。`B-11` **部分落地**（`[~]`）：核心原子性 bug（`create_thread` 单事务）已修，架构级「全局共享连接/池」暂缓——理由见该条。

**批次 5 — 结构性大改（影响面大，最后做）**
`M-1`（拆 handleSend）`M-2`（AppShell 下沉 hooks）`M-3`（迁 prompt 逻辑）`M-5`（拆 records.rs）`M-7`（拆 agent_bridge/mod.rs）`M-8`（workspace 解析去重）`C-13`（审批决策收口）`C-9`（isRecord 共享 util）`N-1` `N-2` `N-4`（命名）。

> **进度**：8/11 已落地（`N-1` `N-2` `N-4` `C-9` `C-13` `M-1` `M-3` `M-8`，见各条 `[x]`）。`M-1` 只做了安全的 `patchMessage` 抽取（prepare/start/finalize 全量分解暂留——generation-guard 穿线是更易出错的部分）。**暂缓**（独立的「大文件拆分」专项，逐项单独验证）：`M-2`（AppShell 下沉 3 个 hook——前端 runtime 行为风险高，effect/乐观更新交织，最好配 `make run-gui` 实机验证）、`M-5`（拆 `records.rs` 867 行）、`M-7`（拆 `agent_bridge/mod.rs` 574 行）——后两者是编译器可保证的纯文件搬移、零功能变化，但很繁琐，宜在专注的一次 pass 里一起做。

**最高优先级（任意批次内先做）**：`B-1`（流式正文被覆写，用户可见数据丢失）、`B-7`（嵌套弹层 Esc 双关）、`B-11`（连接/原子性）、`C-3`（最大体量复制粘贴）。

### 索引

| ID | 类别 | 严重度 | 一句话 |
|---|---|---|---|
| M-1 | module | 中 | `handleSend` 235 行上帝函数拆分 |
| M-2 | module | 中 | AppShell 域逻辑下沉到 hooks |
| M-3 | module | 中 | AgentThread 底部 prompt 逻辑迁出 |
| M-4 | module | 中 | tool-input JSON 解析三处合一 |
| M-5 | module | 中 | `records.rs`(867 行) 按域拆分 |
| M-6 | module | 中 | git diff 解析两套合并 |
| M-7 | module | 中 | `agent_bridge/mod.rs`(574 行) 拆分 |
| M-8 | module | 中 | workspace 解析样板去重 |
| M-9 | module | 中 | ReviewPanel 折叠文件 diff 组件合并 |
| N-1 | naming | 低 | `AgentConnection` 返回类型改名 |
| N-2 | naming | 中 | 三个 review 概念命名统一 |
| N-3 | naming | 低 | DB `type` 列与 `*_type` 字段对齐 |
| N-4 | naming | 低 | `getOrCreateDefaultChatThread` 改名 |
| U-1 | ui | 中 | 删/采纳 5 个零引用 ui 组件 |
| U-2 | ui | 中 | 图标操作按钮 ~12 处抽 Button |
| U-3 | ui | 中 | 主按钮手搓 3 处改 Button primary |
| U-4 | ui | 中 | 原生 select 3 处改 ui/Select |
| U-5 | ui | 中 | 复制按钮/状态 4 处抽 hook+组件 |
| U-6 | ui | 中 | 下拉菜单 popover 4 处抽 SelectMenu |
| U-7 | ui | 低 | focus ring token 统一 |
| U-8 | ui | 低 | `hover:bg-surface` no-op 修复 |
| U-10 | ui | 低 | ReviewPanel 裸橙色 + 分类色收窄 |
| U-11 | ui | 低 | `bg-black/10` 背景遮罩裸色 |
| C-1 | consistency | 低 | 死代码 ToolCallBlock/ToolCall/toolCalls |
| C-2a | consistency | 低 | 死类型 AgentMode |
| C-2b | consistency | 低 | 不可达 PlanBlock/plan |
| C-2c | consistency | 低 | 死代码 lib/ids.ts createId |
| C-3 | consistency | 高 | RpcResponse 判定样板 8 处 |
| C-4 | consistency | 中 | gRPC connect 样板 6 处 + 默认地址 |
| C-5 | consistency | 中 | agent_dir/FUTURE_PROVIDER_ID 重复 |
| C-6 | consistency | 中 | `*_COLUMNS` 列常量推广 |
| C-7 | consistency | 中 | markdown_refs 元数据构造去重 |
| C-8 | consistency | 中 | 读回样板 + 状态字面量常量化 |
| C-9 | consistency | 中 | isRecord/截断 工具 8 处合一 |
| C-10 | consistency | 中 | run 状态文案双真相 + RunError 重复 |
| C-11 | consistency | 中 | recentRunEventCount 死状态 + 多余请求 |
| C-13 | module | 中 | 审批决策逻辑收口进 useApprovals |
| B-1 | bug | 高 | waiting_approval 覆写流式正文 |
| B-2 | bug | 中 | Promise.all 让 run 失败拖垮消息加载 |
| B-3 | bug | 中 | check-then-insert 去重竞态 |
| B-4 | bug | 中 | cancel_stale 两 UPDATE 集合错位 |
| B-5 | bug | 中 | git_review 缺 core.quotePath |
| B-6 | bug | 中 | materialize 任务恢复缺口 |
| B-7 | bug | 高 | 嵌套 Overlay Esc 双关 |
| B-8 | bug | 中 | PdfPreview 渲染 effect 依赖 ref |
| B-9 | bug | 中 | setAttachError 在 updater 内 |
| B-10 | bug | 中 | mark_run_overlapped TOCTOU |
| B-11 | bug | 高 | 每调用新连接 + 复合写非原子 |
| B-12 | bug | 中 | 热点查询缺索引 |
| B-13 | bug | 低 | decide_approval 非原子续跑 |
| B-14 | bug | 中 | run-status 双触发源重复 fan-out |
| B-15 | bug | 中 | 错误字符串子串匹配脆弱 |
| B-16 | bug | 低 | PDF worker reject 时泄漏 |

---

## 文档卫生（一并处理）

- `gui/CLAUDE.md` 的「文档地图」只认 `PRODUCT.md` / `ER.md` / `COLOR.md`，但仓库里还有 `PLAN.md` / `ATTACHMENT_PLAN.md` / `MEMORY_PLAN.md` 三份历史规划稿（均标「已实现 / 已落地」）。建议把仍有效的决策折进 `PRODUCT.md`/`ER.md`，其余归档或删除，避免误读。
- `ATTACHMENT_PLAN.md` 的「⚠️ 前置任务：assetProtocol 未启用」已过期——`src-tauri/tauri.conf.json` 的 `assetProtocol.enable=true` 且 scope 已配（`$APPCACHE`/`$APPDATA`/`$HOME/.future`/`$TEMP`）。
- 多处 finding 涉及同步更新文档：`C-2c` 删 `lib/ids.ts` 后改 `gui/CLAUDE.md` 第 29 行 `lib/` 清单去掉 `ids`；`U-10` 收窄分类色后改 `COLOR.md` 第 9 行 `eventCategoryClass` 描述。

---

# 模块重新划分（Module）

### [x] M-1. `handleSend` 是约 235 行的「上帝函数」
- **类别 / 严重度**: module / 中
- **位置**: `src/features/agent/useAgentThreadState.ts:129-364`（`handleSend`）
- **现状**: 单个 `useCallback` 顺序完成：乐观 user+assistant 气泡注入（142-166）、附件导入+缩略图（171-173）、inline-context+messageContent（178-181）、`buildReferencePrompt`（182-186）、持久化 user message（198-204）、`createRun`（220-224）、流式轮询 timer 生命周期（239-246）、gRPC `sendPromptToFutureAgent`（249-263）、temp 附件清理（269-271）、run 状态收尾（273-277）、追加 assistant message（281-288）、最终重投影（292-317），catch 分支失败落库+气泡更新（321-363）。期间 6 处 `setMessages` 调用（165、189、207、230、300、345），其中后 5 处是 `c.map(m => m.id === id ? {...} : m)` 形态。
- **问题**: 单函数职责过多、`isCurrentSend()` 守卫散落、按 id patch 一条的样板重复 5 次，难测难改。所有 send-path bug（B-1、B-9）都根植于此。
- **改造方案**: 抽小函数（同文件或新 `agentSend.ts`）：
  - `patchMessage(setMessages, id, patch): void` —— 收敛 5 处 `setMessages(c=>c.map(...))`。
  - `prepareOutgoingMessage(thread, payload, modelId)` → `{ importedAttachments, messageContent, promptContent, inlineContext }`（封装 171-186）。
  - `startStreamPoll(runId, pendingId, setMessages, isCurrentSend)` → timer handle（封装 239-246）。
  - `finalizeAssistantMessage(...)` → 落库+重投影+patch（封装 281-317）。
  - **坑**：`isCurrentSend()`/`sendGenerationRef`/`streamTimerRef` 跨步骤共享，抽函数时作为参数传入而非闭包捕获，保「最新一次 send 获胜」语义。
- **复核结论**: 已修正——129-364 共 236 行属实；`setMessages` 精确为 6 处（非泛指），其余描述 CONFIRMED。
- **验证**: 前端基线三连；行为保持，回归：正常发送、附件发送、流式预览、失败落库、线程切换中途取消。
- **关联**: C-11（顺带清理 `recentRunEventCount` set 点 229）、C-9（共享 util）。

### [x] M-2. AppShell 仍持有大量本应进 `layout/hooks/` 的域逻辑
- **类别 / 严重度**: module / 中
- **位置**: `src/components/layout/AppShell.tsx`
  - (a) app-settings：`useAsyncResource(getAppSettings)` 69-73、镜像 `appSettings` useState 74、同步 effect 107-109、`handleChangeSettings` 158-167（默认字面量 `{autoApprove:false,hiddenModels:[]}` 写两遍：72 与 74）
  - (b) rename 状态机：`handleConfirmRenameThread` 248-279（`handleRenameThread` 设初值 239-246）
  - (c) delete 状态机：`handleDeleteThread` 339-365（含 cleanup-summary fetch 348-364）、`handleConfirmDeleteThread` 367-388
  - (d) model/thinking 选择：`handleModelChange` 286-305、`handleDraftModelChange` 307-311、`handleThinkingLevelChange` 313-324、draft thinking effect 111-117
- **现状**: 镜像 useState + 同步 effect 会短暂显示陈旧设置（reload 时闪回默认再回填）；rename/delete 是带 `submitting/error/cleanupSummary` 的完整状态机；model/thinking 三 handler + `draftThinkingModelRef` effect 维护「选中模型→默认 thinking level」联动。约 585 行，混入 settings/dialog/model 三块域逻辑。
- **问题**: 违反 CLAUDE.md §5「AppShell 只编排布局」。
- **改造方案**（同名解构暴露）抽三个 hook 进 `src/components/layout/hooks/`：
  1. `useAppSettings(): { appSettings, changeSettings }` —— 单一 `useAsyncResource` + 本地乐观 state，消除镜像 state 与同步 effect；默认抽成 `appSettings.ts` 导出的 `const DEFAULT_APP_SETTINGS`。SettingsDialog `onChangeSettings`（545）改用之。
  2. `useThreadDialogs({ activeThreadId, refreshStore }): { renameDialog, deleteDialog, setRenameDialog, setDeleteDialog, openRename, confirmRename, openDelete, confirmDelete }` —— 搬入 239-279、339-388。`AppShellDialogs`（533-540）props 同名直传。
  3. `useModelSelection({ activeThread, selectedModelId, setSelectedModelId, visibleModelOptions, refreshStore }): { selectedThinkingLevel, changeModel, changeDraftModel, changeThinkingLevel }` —— 搬入 286-324 + 111-117 + `draftThinkingModelRef`。
  - **坑**：`refreshStore` 属 thread-store 域、`selectedModelId/setSelectedModelId` 来自 `useAgentConnection`，均作参数注入，别在新 hook 内重持一份。
- **复核结论**: CONFIRMED（行号已逐段核对一致）。
- **落地结论**: 抽出三个 hook 进 `src/components/layout/hooks/`：`useAppSettings`、`useThreadDialogs({activeThreadId, refreshStore})`、`useModelSelection({activeThread, selectedModelId, setSelectedModelId, visibleModelOptions, refreshStore})`，均同名解构暴露。`DEFAULT_APP_SETTINGS` 抽到 `integrations/storage/appSettings.ts`（消除两处默认字面量）。`refreshStore`/`selectedModelId`/`setSelectedModelId` 作参数注入未重持。hook 调用顺序：`useAppSettings`（先于消费它的 `useApprovals`/`useAgentConnection`）→ `useThreadStore` → `useApprovals` → `useAgentConnection` → `useModelSelection`/`useThreadDialogs`（依赖前两者）。新增 `useModelSelection.syncSelection(modelId, thinkingLevel)` 收敛 `handleStartNewConversation` 原先直接写 `setSelectedModelId/setSelectedThinkingLevel/draftThinkingModelRef` 的三行。**为保「零功能变化」未消除 `useAppSettings` 的镜像 state + 同步 effect**（属纯搬移，原乐观更新/reload 行为保持）。`function handle` 由 ~22 降至 14。
- **验证**: 前端基线三连 + 生产 `vite build` 均过；审批/模型切换/对话流待 `make run-gui` 实机确认。
- **关联**: C-13、N-1；CLAUDE.md §5、§4。

### [x] M-3. prompt 构造逻辑堆在视图组件 `AgentThread.tsx` 底部
- **类别 / 严重度**: module / 中
- **位置**: `src/features/agent/AgentThread.tsx:287-418`：`buildContinuePrompt`(287-314)、`loadRunResumeSummary`(316-338)、`summarizeRunForPrompt`(340-374)、`previousUserForRun`(376-385)、`toolCommand`(387-409)、`truncateForPrompt`(411-414)、`isRecord`(416-418)
- **现状**: 这些函数（除依赖 `listRunEvents/listToolCalls/listToolOutputs` 的两个外均无 React 依赖）定义在视图组件尾部，被 `handleContinueMessage`(81-85)/`handleContinueRun`(87-93)/`handleRetryRun`(95-106) 调用。
- **问题**: 视图组件混入大段 prompt 构造/run 摘要逻辑，违背就近内聚；`isRecord`/`truncateForPrompt` 在此重复（见 C-9）。
- **改造方案**: 新建 `src/features/agent/buildContinuePrompt.ts`（与 `buildReferencePrompt.ts` 并列），迁入前五个函数并 export。`truncateForPrompt`/`isRecord` 不复制——改从 C-9 的 `src/lib/objects.ts` import。迁移时一并移动 `listRunEvents/listToolCalls/listToolOutputs` 与 `StoredRunEvent/StoredToolCall/StoredToolOutput`、`AgentMessage` 等 import。
- **复核结论**: CONFIRMED（七函数行号核对一致）。
- **验证**: 前端基线三连；纯迁移，回归「继续上一个任务 / 重试 run」（`onFutureEvent("recover-run",...)` AgentThread.tsx:108-114）。
- **关联**: C-9、M-1。

### [x] M-4. tool-input JSON 解析逻辑三处重写（各带私有 isRecord/stringField）
- **类别 / 严重度**: module / 中
- **位置**:
  - `src/features/runs/RunsPanel.tsx`：`toolCommand`(257-266)、`parseToolInput`(291-308)、`stringField`(310-313)、`isRecord`(315-317)
  - `src/features/runs/RunInspectPanel.tsx`：`toolDetails`(438-454)、`toolInputObject`(456-459)、`toolOutputObject`(461-463)、`stringField`(500-509)、`numberOrStringField`(487-498)、`parseJsonish`(511-529)、`isRecord`(541-543)
  - `src/features/markdown/renderers/ObjectEmbed.tsx`：`toolCommand`(169-191)、`isRecord`(193-195)
- **现状**: 三个文件各实现「最多 3 层反复 JSON.parse 直到拿到对象」+ 各自 `isRecord`。差异：`parseToolInput`（严格，失败 `return null`）vs `parseJsonish`（宽松，失败保留原值 + trim/空串判定）；`RunsPanel.stringField`（单 key）vs `RunInspectPanel.stringField`（多 key 数组）。
- **问题**: 同一健壮解析逻辑三份拷贝；改解析层数/容错要改多处。
- **改造方案**: 新建 `src/features/runs/toolInput.ts`，导出 `isRecord`（不含 array）/`parseJsonish`（宽松版）/`recordOf`/`stringField(rec, keys: string|string[])`/`numberOrStringField`/`toolCommand`。三处删本地实现改 import。
  - **坑**：统一为宽松 parse 即可（只取 command 字段时与严格版等价）；务必保留 `numberOrStringField` 同时接受 number/string（RunInspectPanel exitStatus 依赖）。`MarkdownContent.tsx:433` 的 `isRecord`（允许 array，语义不同）**不并入**。
- **复核结论**: CONFIRMED（函数与行号核对一致）。`isRecord` 此前已随 C-9 收口到 `lib/objects`，本项只合并 parse/字段提取逻辑。
- **落地结论**: 新建 `src/features/runs/toolInput.ts` 导出 `parseJsonish`（宽松版，统一三处）/`recordOf`/`stringField(rec, string|string[])`/`numberOrStringField`/`toolCommand`。RunsPanel 删 `parseToolInput`+`stringField`+`toolCommand`（调用点改 `toolCommand(tool.input)`）；RunInspectPanel 删 `parseJsonish`+`stringField`+`numberOrStringField`+`toolInputObject`（改 `recordOf`）；ObjectEmbed 删本地 `toolCommand`+`isRecord` import。新增 `toolInput.test.ts`（双层 stringify / 非 JSON 串 / exit_code=0 数字 / array 拒绝等 8 例）。
- **验证**: 前端基线三连通过（vitest 32 例，含新增 8 例）。
- **关联**: M-9、C-9。

### [x] M-5. `records.rs`（867 行）是 god-module：record + `*_from_row` + 列常量与查询分置两文件
- **类别 / 严重度**: module / 中
- **位置**: `src-tauri/src/store/records.rs`（867 行）。14 个 `*_from_row`：thread(588)、message(610)、run(624)、workspace(641)、run_event(658)、tool_call(669)、tool_output(683)、approval_request(693)、review_changeset(728)、review_file_change(764)、review_snapshot(797)、artifact(818)、research_collection(836)、research_resource(849)。告警注释在 584-586。
- **现状**: 单文件含 ~40 类型（output records、`*Input` DTO、P2 config records 547/560/572）+ 全部 `_from_row` + 3 个 `REVIEW_*_COLUMNS`。映射器靠列顺序对应散在他处的 SELECT（如 `thread_from_row`@588 vs `threads.rs:11/26/94`），schema 改一列要动两个相距很远的文件。
- **问题**: 文件过大、关注点混杂；映射器与 SELECT 物理分离，是 C-6/N-3 静默错位的根因。
- **改造方案**: 把每个 record + 其 `*_from_row` + 列常量（C-6 后）co-locate 到对应 store 域模块（Thread*→`threads.rs`、Run*→`runs.rs`、Workspace*→`workspaces.rs`、Message*→`messages.rs`、Artifact*→`artifacts.rs`、Research*→`research.rs`、Review*+`REVIEW_*_COLUMNS`→`review_snapshots.rs`、ApprovalRequest*→`approvals.rs`）。`records.rs` 留作共享 DTO 仓（跨模块 `*Input`、markdown-ref 类型 276-317、`AppDataPath`、`ThreadCleanupSummary`）。P2 config records(547-582) 保留。
  - **坑**：`store.rs:37` `pub use records::*;` 是对外出口——移动后保证仍 re-export；`_from_row` 保持 `pub(super)`，更新 `crate::store::records::*` use 路径。
- **复核结论**: 已修正——`*_from_row` 实为 **14** 个（非 16）；867 行、注释 584-586 准确。建议与 C-6 同批渐进重构。
- **验证**: 后端基线三连。
- **关联**: C-6、C-7、N-3。

### [x] M-6. git-diff 解析在 git_review.rs 与 shadow_review/diff.rs 重复两套
- **类别 / 严重度**: module / 中
- **位置**: `src-tauri/src/git_review.rs:284-321`（`split_git_diff_by_path`/`flush_diff_chunk`/`diff_path_from_header`）、`:336-361`（`normalize_numstat_path`/`parse_numstat`，numstat 消费 117-148）；`src-tauri/src/shadow_review/diff.rs:244-279`（`split_patch`/`parse_diff_git_new_path`）、`:208-223`（`parse_numstat`）
- **现状**: 两处都把 unified diff 按 `diff --git ... b/<path>` 与 `+++ b/` 切成 per-path map 并解析 `--numstat`，但 split 细节有别：git_review `line.split(" b/").nth(1)` vs diff.rs `strip_prefix("diff --git ")? + find(" b/")` 切片；numstat 处理也不同（rename 归一化 vs 位置+binary 标志）。
- **问题**: 两套 split 在含 ` b/` 子串的怪异路径上分歧，quotePath/binary/rename 处理不一致（见 B-5），bug 只在一侧被修。
- **改造方案**: 抽共享 `git_diff_parse`（无 store/shadow 依赖，放 `src/git_diff_parse.rs`），提供 `split_unified_patch_by_path(&str) -> HashMap<String,String>` 与 `parse_numstat(&str) -> Vec<NumstatRow{additions,deletions,path,binary}>`。两侧改调；split 统一保留更严格的 `strip_prefix+find` 实现，并保留 `+++ b/` 优先覆盖 header 路径的语义。连同 B-5 的 quotePath 一并处理。
  - **坑**：两侧现有单测（git_review 383-404、diff.rs 304-367）迁到共享模块并全过。
- **复核结论**: CONFIRMED（两套实现及行号核对一致）。
- **验证**: 后端基线三连。
- **关联**: B-5、N-2；ER.md §6.8。

### [x] M-7. `agent_bridge/mod.rs`（574 行）混杂多职责
- **类别 / 严重度**: module / 中
- **位置**: `src-tauri/src/agent_bridge/mod.rs`（574 行）。已有子模块 `client.rs`（薄请求构造）/`persist.rs`/`review.rs`/`stream.rs`。
- **现状**: mod.rs 一文件承担：模型列举(`list_agent_models` 55-78 + 类型 33-53)；prompt 生命周期(`agent_prompt` 80-156、`agent_prompt_inner` 158-275)；并发守卫(`PromptSessionGuard` 277-306 + `ACTIVE_AGENT_PROMPTS` 25 + `wait_for_agent_idle` 311-337 + `mark_run_failed_if_active` 339-349)；会话/权限(`ensure_agent_session` 476-510、`create_agent_session` 512-532、`set_agent_permission_level` 534-557、`workspace_path_for_thread` 568-574、`prior_user_message_count` 559-566)；审批/中止(`notify_agent_approval_decision` 351-381、`abort_agent_thread` 383-408、`abort_run` 413-429、`decide_approval` 434-465 + matcher 467-474)。
- **问题**: 单文件跨 5 个关注点，与已有细分子模块风格不一致。
- **改造方案**: 拆为同目录子模块，mod.rs 收敛为 prompt coordinator + re-export：
  - `session.rs`：ensure/create_agent_session、set_agent_permission_level、workspace_path_for_thread、prior_user_message_count。
  - `approval.rs`：notify_agent_approval_decision、decide_approval、is_stale_approval_error。
  - `run_control.rs`：abort_agent_thread、abort_run、mark_run_failed_if_active、wait_for_agent_idle、is_agent_unavailable_error。
  - `models.rs`（可选）：list_agent_models + 两 model 类型。
  - mod.rs 留 agent_prompt/_inner、PromptSessionGuard、ACTIVE_AGENT_PROMPTS 与 `pub use`。
  - **坑**：`ACTIVE_AGENT_PROMPTS`(OnceLock) 与 guard 共用，guard 留 mod.rs；拆出函数引用的类型需 `pub(super)`。与 C-3/C-4 helper 同批做。
- **复核结论**: CONFIRMED（574 行、各函数行号核对）。
- **验证**: 后端基线三连（无未用 import/可见性告警）。
- **关联**: C-3、C-4、B-13、B-15。

### [x] M-8. workspace 解析样板重复 4 处
- **类别 / 严重度**: module / 中
- **位置**: `src-tauri/src/agent_bridge/persist.rs:263-276`（`path_is_inside_run_workspace`）、`:318-334`（`artifact_is_allowed_for_run`）；`agent_bridge/mod.rs:568-574`（`workspace_path_for_thread`）；`agent_bridge/review.rs:37-57`（`resolve`）
- **现状**: persist.rs 两函数近乎相同（run→thread→workspace、git 即 `Ok(false)`、`canonical_or_raw`+`starts_with`），仅 path 必选性不同（后者 `Option<&str>`，`None → Ok(true)`）。mod.rs/review.rs 又各自重做前半段查找。
- **问题**: 三段查找 + git 判定 + 前缀比对在 persist.rs 抄两遍。
- **改造方案**:
  1. 加 `store::workspace_for_run(run_id) -> Result<Option<WorkspaceRecord>, AppError>`。
  2. persist.rs 两函数合并为 `path_allowed_for_run(run_id, path: Option<&str>) -> Result<bool, AppError>`：git → `Ok(false)`，`None → Ok(true)`，否则 canonical+`starts_with`。调用点 `:232` 传 `Some(&path)`、`:292` 直接传 `path`。
  3. mod.rs/review.rs 可复用 `store::workspace_for_thread(thread_id)`，但 review.rs `resolve` 的 mode/is_dir/is_git 判定(41-51)保留。
  - **坑**：`canonical_or_raw`/`is_git_workspace` 是 `git_review` 的 `pub(crate)`，合并函数可直接调用。
- **复核结论**: CONFIRMED（四处行号、persist 两函数差异核对一致）。
- **验证**: 后端基线三连。
- **关联**: N-2；ER.md §6.8。

### [x] M-9. ReviewPanel 两个近重复的可折叠文件 diff 组件 + 两套展开状态机
- **类别 / 严重度**: module / 中
- **位置**: `src/features/review/ReviewPanel.tsx`：`ChangesetFileChange`(325-381)、`GitFileDiff`(504-547)；状态机 `WorkingTreeReview`(157-188, 按 `file.path`)、`LastRunReview` openFiles(216、249-267, 按 `file.id`)；`ExpandCollapseAll`(190-198)已共用。
- **现状**: 两组件 header 结构相同（FileDiff 图标 + 路径 + `+/-` 计数 + chevron），body 都是 `open ? <DiffView> : null`；`ChangesetFileChange` 是超集（额外 previousPath 箭头 344-346、changeType 标签 349-351、binary/sensitive/truncated 366-375）。两套状态机仅 key 字段不同。
- **问题**: header+折叠壳重复；`text-orange-500` 两处各写一遍（见 U-10）；状态机重复。
- **改造方案**: 抽 `src/features/review/CollapsibleFileDiff.tsx`，props `{ title, headerExtras?, additions?, deletions?, showCounts?, open, onToggle, children }`；两组件退化为组装 title/extras/body 后渲染它。可选抽 `useExpandableFiles<T>(files, keyOf)` 供两视图共用。图标色统一在组件里改语义 token（联动 U-10）。
- **复核结论**: CONFIRMED（两组件 header 一致、两套状态机属实）。
- **落地结论**: 抽出 `src/features/review/CollapsibleFileDiff.tsx`（props 同方案）+ `src/features/review/useExpandableFiles.ts`（泛型展开状态机，工作树按 `path`、上一轮按 `id`，hook 因 react-refresh lint 单独成文件）。`ChangesetFileChange`/`GitFileDiff` 退化为组装 title/extras/body 渲染 `CollapsibleFileDiff`，`WorkingTreeReview`/`LastRunReview` 改用 hook（LastRunReview 的 hook 调用上移到早 return 之前，`files = review?.files ?? []`）。图标色仍是语义 `text-ink-soft`（U-10 已统一，未回退）。class 串逐字保留。
- **验证**: 前端基线三连已过；两个 diff 视图折叠/计数/重命名箭头待 `make run-gui` 实机确认。
- **关联**: U-10、M-4。

---

# 命名（Naming）

### [x] N-1. `useAgentConnection` 返回包类型 `AgentConnection` 与状态类型 `AgentConnectionState` 撞名
- **类别 / 严重度**: naming / 低
- **位置**: `src/components/layout/hooks/useAgentConnection.ts:22`（返回包 `AgentConnection`）、`:8`（状态对象 `AgentConnectionState`）、`:23`（包内字段 `agentConnection: AgentConnectionState`）；消费 `AppShell.tsx:94-101`
- **现状**: 返回包 `AgentConnection` 与状态 `AgentConnectionState` 仅差后缀，且包里就有 `agentConnection` 字段。兄弟 hook 按域命名返回（`ApprovalsState`、`ThreadStore`）。
- **问题**: 命名不一致 + 自我撞名，难分清「返回包」与「状态」。
- **改造方案**: 返回包改名 `UseAgentConnectionResult`。改 `:22`（声明）、`:54`（函数返回注解）。`AgentConnectionState` 与字段 `agentConnection` 保持不动（后者经 AppShell re-export 被 AgentThread 消费，改动面大无收益）。返回包类型无外部 import，重命名零外部影响。
- **复核结论**: CONFIRMED（类型/字段/消费站点核对，无外部 import）。
- **验证**: `cd gui && grep -rn "AgentConnection\b" src/` + 基线三连。
- **关联**: M-2；CLAUDE.md §5。

### [x] N-2. 三个重叠的「review」概念缺乏命名纪律
- **类别 / 严重度**: naming / 中
- **位置**: Git 源 `src-tauri/src/git_review.rs`（模块 `git_review` / 结构体 `GitReview` :13 / view `"git_changes"` `commands/review.rs:53`）；Shadow 源 `src-tauri/src/shadow_review/` + `agent_bridge/review.rs`（驱动影子流水线，模块名却是无限定 `review`）/ 命令层 `RunReview`（`commands/review.rs:24`）/ view `"last_run"`（51-58）。无 `ShadowReview` 类型。
- **问题**: 同一源在 模块名/结构体名/view 字符串 三层用词不统一：Git 三层带 `git`；Shadow 的 view 叫 `last_run`、类型叫 `RunReview`、模块叫 `shadow_review`、wiring 又叫裸 `review`。ER.md §6.8 明确只有两源。
- **改造方案**（仅建议）每源贯穿一套词汇。最小改动：`RunReview` → `LastRunReview`（对齐 view `last_run`；`commands/review.rs:24,69,77,85,110` 及前端消费同步）；`agent_bridge/review.rs` → `agent_bridge/shadow_review.rs`（`mod.rs:3` `mod review;`、`:6` `pub use review::retry`、调用点 `review::capture_before/capture_after/materialize_changeset` 同步）。Git 源不动。保守版：四文件顶部加 module doc 交叉引用 ER.md §6.8。
  - **坑**：`store::ReviewChangesetRecord`/`ReviewFileChangeRecord` 是两源共享持久层类型，**不加**源前缀。
- **复核结论**: CONFIRMED（已收敛为命名建议；各名核对一致，确无 `ShadowReview`）。
- **验证**: 后端基线三连；前端若引用 `RunReview` 序列化字段需同步 `cd gui && npx tsc --noEmit`。
- **关联**: M-6、M-8；ER.md §6.8、PRODUCT.md §4.7。

### [x] N-3. 三个 DB 列名 `type` 映射到 Rust 的 `*_type` 字段（靠位置而非名字）
- **类别 / 严重度**: naming / 低
- **位置**: `run_events.type`(schema.rs:72)→`RunEventRecord.event_type`(records.rs:89, 映射 663)；`artifacts.type`(schema.rs:228)→`ArtifactRecord.artifact_type`(records.rs:237, 映射 825)；`research_resources.type`(schema.rs:252)→`ResearchResourceRecord.resource_type`(records.rs:266, 映射 858)
- **现状**: SQL 列叫 `type`（裸写，如 `artifacts.rs:16`/`runs.rs:121`/`research.rs:13`），Rust 字段叫 `*_type`，`*_from_row` 按位置取值，列名↔字段名不一致在映射点不可见。对照 `tool_calls.kind`/`workspaces.kind` 全程同名。
- **问题**: 列重命名/增删时，映射靠位置 + 命名不一致，错位更隐蔽（与 C-6/M-5 同源）。
- **改造方案**（app pre-release 无迁移成本）方案 A（推荐）：列改名 `event_type`/`artifact_type`/`resource_type`（schema.rs:72/228/252），同步改所有 SELECT/INSERT 列名（`runs.rs:121/136`、`db.rs:95`、`artifacts.rs:16/47/139/210`、`resolve.rs:128`、`research.rs:13/41/61/89`、`sync.rs:73/237`、`search.rs:46/230` 等），`markdown_refs/` 内 `r.type`/`type` 一并改。方案 B（最小）：保留列名，在三个 `*_from_row` 紧邻处加注释标注列↔字段重命名。
  - **坑**：`ADDED_COLUMNS`(schema.rs:356) 不含这三列（在初始 CREATE 里），无需动迁移数组。
- **复核结论**: CONFIRMED（列名/字段名/映射位置核对一致）。
- **验证**: 后端基线三连（方案 A 后确认 SELECT 列名与 `*_from_row` 顺序仍一致）。
- **关联**: M-5、C-6；ER.md §3。

### [x] N-4. `getOrCreateDefaultChatThread` 名实不符（实为「最近线程或新建」）
- **类别 / 严重度**: naming / 低
- **位置**: `src/integrations/storage/threads.ts:60-67`；去重 promise `defaultChatThreadPromise`(58/61/63)；唯一调用 `src/components/layout/hooks/useThreadStore.ts:107`（import :7）
- **现状**: `(await getRecentThread()) ?? createDefaultChatThread()` —— 常见路径返回最近一条线程（可能是 workspace 线程），仅无任何线程时才新建。useThreadStore 已把返回值命名为 `recentThread`(107/115)。
- **问题**: 名实不符。
- **改造方案**: 重命名导出函数 `getRecentOrCreateDefaultThread`，改 threads.ts:60、useThreadStore.ts:7/107。去重变量可一并改 `recentOrDefaultThreadPromise`（模块私有）。`createDefaultChatThread`(44)/`getRecentThread`(32) 不动。
- **复核结论**: CONFIRMED（行为/调用点/新名可用均核对）。
- **验证**: `cd gui && grep -rn "getOrCreateDefaultChatThread" src/`（应零）+ 基线三连。
- **关联**: B-14；CLAUDE.md §6。

---

# UI 抽象（UI）

> 颜色总体已 token 化良好；本节剩余裸色仅 U-10/U-11 两处真违规，其余多为「该用 `ui/` primitive 却手搓」。

### [x] U-1. ui 目录下零外部引用的死组件
- **类别 / 严重度**: module / 中
- **位置**: `src/components/ui/`：`Tabs.tsx`(33行)、`SegmentedControl.tsx`(38)、`Tooltip.tsx`(13)、`Drawer.tsx`(21)、`Panel.tsx`(14)。无 `ui/index.ts` barrel。
- **现状**: 5 个组件零外部 importer（grep 确认）。对照：`ReviewPanel.tsx:143-154` 手搓 `ViewTab`，三处手搓 `<select>`（U-4），而 `Tabs`/`SegmentedControl` 闲置。
- **问题**: 死代码 + 「该用却不用」反模式并存。
- **改造方案**: 二选一，建议先删后按需重建。方案 A：删 5 文件（删前再确认零引用）。方案 B：用 `Tabs`/`SegmentedControl` 替换 ReviewPanel `ViewTab`、用 `ui/Select` 替换 U-4 的三处（需先核对 API 满足 active/onChange）。
- **复核结论**: CONFIRMED（5 组件零外部 importer，确无 barrel）。
- **验证**: `grep -rn "Tabs\|SegmentedControl\|Tooltip\|Drawer\|Panel" gui/src --include=*.tsx --include=*.ts | grep -i import` + 基线三连。
- **关联**: U-4、ReviewPanel ViewTab。

### [x] U-2. 「带边框图标-文字操作按钮」className 复制粘贴约 12 处
- **类别 / 严重度**: ui / 中
- **位置**: h-7/px-2 变体（7）：`ObjectEmbed.tsx:59,89,128`、`ArtifactEmbed.tsx:61,71,83`、`RunEmbed.tsx:47`；h-8/px-2.5 变体（5）：`ArtifactDetailPanel.tsx:228,239,250`、`RunInspectPanel.tsx:120,128`。另 `features/agent/MessageBlock.tsx:83,95` 同串（agent 域，仅记录不在本条改动面）。
- **现状**: 同串 `inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink` 逐字重复；danger 变体见 `RunsPanel.tsx:208`、`ArtifactDetailPanel.tsx:259`。`ui/Button` 已有 variants + sizes(md=h-9/sm=h-8) 但这些工具按钮都没用（缺 xs/h-7 与 leftIcon）。
- **问题**: ~12 处重复；与 U-7/U-8 样式漂移叠加。
- **改造方案**: 扩展 `ui/Button`：`ButtonSize` 加 `"xs": "h-7 px-2 text-xs"`；加 `leftIcon?: ReactNode`（xs/sm 用 `gap-1.5`）。各站点替换为 `<Button variant="secondary" size="xs" leftIcon={...}>`，danger 站点用 `variant="danger-soft"`。**顺带修 U-8**：secondary 的 `hover:bg-surface-subtle`、danger-soft 的 `hover:brightness-95` 正好消除 no-op hover。MessageBlock 两处属 agent 域、本条不动。
- **复核结论**: CONFIRMED（scope 内 12 处 = 7+5）。
- **验证**: `grep -rn "border border-line bg-surface px-2 text-xs font-medium text-ink-soft" gui/src/features/{markdown,runs,artifacts} gui/src/components/layout`（替换后归零）+ 基线三连 + `make run-gui`。
- **关联**: U-8（合并修复）、U-1。

### [x] U-3. 主按钮（accent）手搓 3 处，未用 `Button variant="primary"`
- **类别 / 严重度**: ui / 中
- **位置**: `src/features/agent/Composer.tsx:447`（发送，`size-7` 图标按钮）、`src/features/agent/NewConversation.tsx:484`（`h-8`）、`src/features/agent/ApprovalPrompt.tsx:118`（`h-9`，approve）
- **现状**: 三处各写 `bg-accent text-white hover:bg-accent-hover [disabled:bg-accent-disabled]`，而 `ui/Button.tsx:16` 的 `primary` = `border-accent bg-accent text-white hover:bg-accent-hover`（颜色一致，仅缺 disabled 态与图标 size）。
- **问题**: 与 `Button` primary 重复；disabled 处理不统一（Composer/NewConversation 用 `disabled:bg-accent-disabled`，ApprovalPrompt 用 `disabled:opacity-60`）。
- **改造方案**: NewConversation/ApprovalPrompt 改 `<Button variant="primary" size="sm|md">`；Composer 的发送是 `size-7` 纯图标按钮，需 `ui/Button` 支持 xs/icon-only（与 U-2 的 size 扩展协同），或保留但统一 disabled token。统一 disabled 用 `disabled:bg-accent-disabled`（COLOR.md 有该 token）。
- **复核结论**: CONFIRMED（我亲自核对：三处 `bg-accent` 行号 447/484/118；Button primary 变体存在，仅 md/sm 两 size，无 leftIcon）。
- **验证**: 前端基线三连 + `make run-gui` 确认三处按钮态（hover/disabled）一致。
- **关联**: U-2（Button size/icon 扩展）、U-6。

### [x] U-4. 原生 `<select>` 标记重复三处（含绝对定位 ChevronDown），未走 `ui/Select`
- **类别 / 严重度**: ui / 中
- **位置**: `src/components/layout/ContextPanel.tsx:280-290`（自带 ChevronDown :290）、`src/features/runs/RunInspectPanel.tsx:176-185`（h-7，靠浏览器原生箭头）、`src/features/review/ReviewPanel.tsx:478-488`（h-8）
- **现状**: `ui/Select` 已封装 `appearance-none` + 绝对定位 ChevronDown，但三处手写；`ui/Select` 固定 `h-9`，三处分别 h-8/h-7。
- **问题**: 重复的 select 壳 + chevron；尺寸/焦点环（U-7）各写各。
- **改造方案**: 给 `ui/Select` 加 `size?: "xs"|"sm"|"md"`（xs:h-7 text-xs / sm:h-8 / md:h-9 默认），焦点环统一 `focus:ring-focus`（联动 U-7）。三处改 `<Select size=... />`，删各自 `<select>`+ChevronDown+import。
  - **坑**：ContextPanel 有 `w-fit min-w-24 max-w-full` 定制宽度——需确认 `className` 合并到 `<select>`，外层 wrapper 也要 `w-fit`（可加 `wrapperClassName` prop）。RunInspectPanel 需 `xs` size。
- **复核结论**: CONFIRMED（三处手写 select，scope 内仅这三文件；RunInspectPanel 无自带 chevron）。
- **验证**: `grep -rn "<select" gui/src/components/layout gui/src/features`（替换后归零）+ 基线三连 + `make run-gui`。
- **关联**: U-1、U-7。

### [x] U-5. copy-to-clipboard 按钮 + 瞬时「copied」状态重复四处（timeout 未清理）
- **类别 / 严重度**: bug / 中
- **位置**: `setTimeout(setCopied, 1400, …)`：`src/features/markdown/MarkdownContent.tsx:140`、`src/features/artifacts/ArtifactDetailPanel.tsx:83`、`src/features/markdown/renderers/ArtifactEmbed.tsx:29`、`src/components/ui/CopyablePre.tsx:23`（canonical）。浮动按钮：MarkdownContent 146-173、ArtifactEmbed 60-78、ArtifactDetailPanel 128-189。
- **现状**: 同模式四处拷贝（`copyText → setCopied(true) → setTimeout(reset,1400)`），timeout 句柄从不保存：卸载后仍 `setCopied`（React 警告/泄漏），1400ms 内重复点击叠加多个 timer 提前复位。ArtifactDetailPanel 是 `copied: "content"|"path"|null` 三态。
- **问题**: 逻辑四份重复 + timeout 未清理。
- **改造方案**:
  - 新增 `src/components/ui/useCopyState.ts`：`{ copied, copy(text) }`，内部用 `useRef` 存 timer，`copy` 时先 `clearTimeout` 再设，`useEffect` 卸载清理。
  - 新增 `src/components/ui/CopyButton.tsx`：受控 `copied ? <Check/> : <Clipboard/>`，封装浮动角标样式（`bg-surface/90 … shadow-sm ring-1 ring-line-soft`），支持 variant（浮动/行内）。
  - `CopyablePre`/`CodeBlock`(MarkdownContent 两版)/`ArtifactEmbed` 改用之。ArtifactDetailPanel 三态需 `useCopyState` 泛型化（返回 `{ copiedKey, copy(key, text) }`，保留 `null` 复位语义）。
- **复核结论**: CONFIRMED（四处 `setTimeout(setCopied,1400)`，全仓无 `clearTimeout`）。
- **验证**: `grep -rn "setTimeout(setCopied" gui/src`（替换后归零）+ 基线三连；可加单测「快速二次 copy 只保留一个 timer」「unmount 清理」。
- **关联**: U-2。

### [x] U-6. 下拉菜单 popover 模式重复四处（仅迁 model/thinking）
- **类别 / 严重度**: ui / 中
- **位置**: `src/features/agent/Composer.tsx`：model 菜单(382-410)、thinking 菜单(427-440)、reference 菜单(490 起)；`src/features/agent/NewConversation.tsx`：workspace 菜单(265-292)
- **现状**: 四处同构 popover：`useDismissableLayer`(Composer 63/67、NewConversation 97) + 绝对定位 `bg-surface shadow-panel border-line-soft` 面板 + 列表项 `hover:bg-surface-subtle` + 选中项尾随 `<Check className="size-4 text-ink-soft">`(404/439/292)。
- **问题**: 四份近重复，弹层样式/选中态/dismiss 逻辑各写一遍。
- **改造方案**: 抽通用 `<SelectMenu options renderOption selected onSelect anchor>`（含 `useDismissableLayer` + 面板 + MenuItem + Check），放 `src/components/ui/`。四处改用之。注意各菜单定位不同（bottom-9 right-0 / bottom-full / top-9）——经 `placement` prop 或透传 className 支持。
  - **坑**：reference 菜单(490)宽度/内容较特殊（`w-[min(30rem,...)]`），先迁 model/thinking/workspace 三个结构最一致的，reference 视情况。
- **复核结论**: CONFIRMED（我亲自核对：useDismissableLayer 3 处、四个 `shadow-panel` 绝对定位面板 + `<Check>` 选中态）。
- **落地结论**: 抽出 `src/components/ui/SelectMenu.tsx`（`SelectMenu` + `SelectMenuItem`，含 `useDismissableLayer` + `shadow-panel` 面板 + 选中 `<Check>`），**仅迁 Composer 的 model / thinking 两个结构一致的菜单**（class 串逐字保留，placement `bottom-9 right-0` 内置，宽度/overflow 经 `panelClassName` 透传）。**NewConversation workspace 菜单（搜索头 + 底部动作）与 Composer reference 菜单（键盘导航搜索 + `w-[min(30rem,…)]`）结构差异大，按原计划暂缓**——强行统一会让 SelectMenu 长出搜索/动作/键盘导航等分支，过度抽象、收益为负。
- **验证**: 前端基线三连已过；model/thinking 两个下拉开合/选中/外点关闭待 `make run-gui` 实机确认。
- **关联**: U-3、`lib/useDismissableLayer`。

### [x] U-7. focus ring token 不一致
- **类别 / 严重度**: consistency / 低
- **位置**: `focus:ring-accent-soft`：`ui/Select.tsx:14`、`ui/TextInput.tsx:6`；`focus:ring-2 focus:ring-focus`：`RunInspectPanel.tsx:145,178`、`ReviewPanel.tsx:479,492`、`ContextPanel.tsx:282`；`focus:ring-accent/15`：`AppShellDialogs.tsx:71`
- **现状**: 三种焦点环并存。COLOR.md（33、65 行）定义 `focus`(#93c5fd) 为唯一 ring token；`accent-soft`/`accent/15` 是强调浅底，非 ring 语义。
- **问题**: 焦点环不统一；`accent/15` 是裸 opacity 用法。
- **改造方案**: 统一 `focus:ring-2 focus:ring-focus`（+ 视需 `focus:border-focus`）。改 `Select.tsx:14`、`TextInput.tsx:6`；`AppShellDialogs.tsx:71` 的裸 rename input 改走 `<TextInput>`（消手写 input + 继承统一环）。RunInspectPanel/ReviewPanel/ContextPanel 随 U-4 走 `ui/Select`/`TextInput` 后自然统一。
- **复核结论**: CONFIRMED（各行号核对一致）。
- **验证**: `grep -rn "focus:ring-accent-soft\|focus:ring-accent/" gui/src`（改后趋零）+ 基线三连 + `make run-gui`。
- **关联**: COLOR.md `focus`、U-4、AppShellDialogs 裸 input。

### [x] U-8. `hover:bg-surface` 对 `bg-surface` 按钮是 no-op
- **类别 / 严重度**: ui / 低
- **位置**: `bg-surface … hover:bg-surface`：`RunsPanel.tsx:82`、`ArtifactDetailPanel.tsx:228,239,250`；danger no-op（`bg-danger-soft … hover:bg-danger-soft`）：`RunsPanel.tsx:208`、`ArtifactDetailPanel.tsx:259`
- **现状**: 这些按钮底色已是 `bg-surface`，hover 又写 `hover:bg-surface`，悬停无变化；danger 同理。
- **问题**: hover 反馈缺失，与同类按钮（`hover:bg-surface-subtle`）不一致。
- **改造方案**: surface 底改 `hover:bg-surface-subtle`；danger-soft 底改 `hover:brightness-95`（与 Button danger-soft 一致）。**最佳**：这些本属 U-2 的图标按钮，改用 `ui/Button` 后自动修复，**建议并入 U-2**。
  - **坑**：排除「无底→hover 才出底」的合法用法（`ArtifactDetailPanel.tsx:105,130`、`RunInspectPanel.tsx:86,420`），别误改。
- **复核结论**: 已修正——原列 `RunInspectPanel:86` 等无 bg-surface 底，属合法用法，**已剔除**；真正 no-op 为 RunsPanel:82、ArtifactDetailPanel:228/239/250 + danger RunsPanel:208、ArtifactDetailPanel:259。
- **验证**: 前端基线三连 + `make run-gui` 悬停确认有底色过渡。
- **关联**: U-2（合并修复）。

### [x] U-10. ReviewPanel 裸 `text-orange-500` 装饰图标 + RunInspect 分类色可收窄
- **类别 / 严重度**: bug / 低（配色违规）
- **位置**: 裸色 `src/features/review/ReviewPanel.tsx:342`、`:521`（`<FileDiff className="… text-orange-500" />`，全仓仅此两处）；分类色白名单 `src/features/runs/RunInspectPanel.tsx:258-274`（`eventCategoryClass`）
- **现状**: ReviewPanel 的 FileDiff 图标是装饰性用色，非「区分并列种类」，不属 COLOR.md 例外。`eventCategoryClass` 6 类中 approval/error/tool/default 可映射到语义 token（warning/danger/info/neutral），仅 artifact(紫)/review(橙) 真需分类色。
- **问题**: ReviewPanel 两处违反 COLOR.md 反模式（71 行）；eventCategoryClass 把可语义化的 4 类也写裸色，扩大了例外面。
- **改造方案**:
  - ReviewPanel:342/521 改 `text-ink-soft` 或 `text-accent`（与 ObjectEmbed ReviewEmbed 图标一致）；若随 M-9 抽 `CollapsibleFileDiff`，在共享组件统一改一次。
  - eventCategoryClass：approval→`bg-warning-soft text-warning`、error→`bg-danger-soft text-danger`、tool→`bg-info-soft text-info`、default→`bg-surface-subtle text-ink-muted`；保留 artifact/review 裸色为真正分类色例外。
  - **同步改 COLOR.md 第 9 行** `eventCategoryClass` 描述从「6 种」收窄为「artifact/review 2 种」。
- **复核结论**: CONFIRMED（两处裸色 + 4 类可语义化、2 类需保留）。
- **验证**: `grep -rn "text-orange-500" gui/src`（改后归零）+ 基线三连 + `make run-gui`。
- **关联**: COLOR.md 第 9/71 行、M-9、CLAUDE.md 原则 1。

### [x] U-11. `NewConversation` 工作区创建弹层用裸 `bg-black/10` 遮罩
- **类别 / 严重度**: ui / 低
- **位置**: `src/features/agent/NewConversation.tsx:406`（`<div className="absolute inset-0 z-40 … bg-black/10 …">`）
- **现状**: 该内嵌弹层背景用裸 `bg-black/10`，而 `ui/Overlay.tsx:26` 的标准遮罩用 `bg-ink-strong/20`。
- **问题**: 裸色违反 COLOR.md；与标准 Overlay 遮罩不一致。
- **改造方案**: 改 `bg-ink-strong/20`（或 `bg-ink-strong/10` 视视觉）。若该 create-workspace 步骤改走标准 `Overlay`/`Dialog` 更佳，可一并消除手写遮罩。
- **复核结论**: CONFIRMED（我亲自核对 NewConversation.tsx:406 `bg-black/10`；Overlay 用 `bg-ink-strong/20`）。
- **验证**: `grep -rn "bg-black/" gui/src`（改后归零）+ 基线三连 + `make run-gui`。
- **关联**: COLOR.md 反模式、`ui/Overlay`。

---

# 一致性 / 可维护性（Consistency）

### [x] C-1. 死代码：`ToolCallBlock` / `ToolCall` / `AgentMessage.toolCalls`
- **类别 / 严重度**: consistency / 低
- **位置**: `src/features/agent/ToolCallBlock.tsx`（整文件）；`agentThreadTypes.ts:5-12`（`ToolCall`）、`:69`（`toolCalls?` 字段）
- **现状**: `ToolCallBlock` 除自身零引用；`ToolCall` 仅 ToolCallBlock 自引 + 类型定义；`toolCalls` 字段全仓无赋值。工具活动实际走 `agentActivity` + `MessageSegment`。
- **问题**: 死代码；`ToolCall` 与 `StoredToolCall` 同名概念易混。
- **改造方案**: 删 `ToolCallBlock.tsx` + `ToolCall` 接口(5-12) + `toolCalls` 字段(69)。
- **复核结论**: CONFIRMED（grep：`ToolCallBlock` 仅自身；`ToolCall` 仅 ToolCallBlock+类型定义；`toolCalls:` 赋值全仓 0；`ContextPanel.tsx:149/162` 的 `toolCalls` 是 `StoredToolCall[]` 局部变量、无关）。
- **验证**: 前端基线三连，全绿即删除安全。
- **关联**: C-2a、C-2b。

### [x] C-2a. 死代码：`AgentMode` 类型零引用
- **类别 / 严重度**: consistency / 低
- **位置**: `src/features/agent/agentThreadTypes.ts:1`
- **现状**: `export type AgentMode = "plan" | "research" | "build" | "review";` 全仓无引用。
- **改造方案**: 删该行。
- **复核结论**: CONFIRMED（`AgentMode` 命中均为 `AgentModelOption` 或本定义）。
- **验证**: 前端基线三连。
- **关联**: C-1、C-2b。

### [x] C-2b. 死代码：`PlanBlock`/`AgentMessage.plan`/`AgentPlanStep` 不可达
- **类别 / 严重度**: consistency / 低
- **位置**: `MessageBlock.tsx:75`（条件渲染）、`PlanBlock.tsx`（整文件）、`agentThreadTypes.ts:14-19`（`AgentPlanStep`）、`:68`（`plan?` 字段）
- **现状**: `{message.plan ? <PlanBlock steps={message.plan} /> : null}` —— `message.plan` 全仓无赋值，恒 `undefined`，PlanBlock 永不渲染。
- **改造方案**: 删 MessageBlock.tsx:75 条件 + import(9) + `PlanBlock.tsx` + `AgentPlanStep`(14-19) + `plan` 字段(68)。
- **复核结论**: CONFIRMED（`plan:`/`AgentMessage.plan` 无赋值点；PlanBlock 仅 MessageBlock import+渲染）。
- **验证**: 前端基线三连。
- **关联**: C-1、C-2a。

### [x] C-2c. 死代码：`lib/ids.ts` 的 `createId`
- **类别 / 严重度**: consistency / 低
- **位置**: `src/lib/ids.ts:1-3`（唯一定义，零调用）；AppShell 自带 id 生成 `AppShell.tsx:580-585`
- **现状**: `createId(prefix)` 全仓零调用，`lib/ids` 从未被 import。AppShell 用单调计数器 `newPendingPromptId`/`pendingPromptCounter`。
- **改造方案**: 删 `src/lib/ids.ts`。**不要**把 AppShell 接到 `createId`（语义不同：单调计数器 vs `Math.random()` 有碰撞面）。**同步改 gui/CLAUDE.md 第 29 行** `lib/` 清单去掉 `ids`。
- **复核结论**: CONFIRMED（`createId` 仅定义行；`lib/ids` 零 import）。
- **验证**: `cd gui && grep -rn "createId\|lib/ids" src/`（删后零）+ 基线三连。
- **关联**: gui/CLAUDE.md 第 29 行。

### [x] C-3. RpcResponse 成功/错误判定样板在 `agent_bridge/mod.rs` 重复 8 次
- **类别 / 严重度**: consistency / 高（按出现量）
- **位置**: 否定式（4）`mod.rs:66,202,222,248`；肯定式（4）`mod.rs:372,399,523,548`。（`:492` 只读 data、无 fallback-err，**不算**。）
- **现状**: 两等价形态各 4 处，例如 `if !response.success { return Err(if response.error.is_empty() {fallback} else {response.error}.into()) }` 与 `if response.success { Ok } else if response.error.is_empty() {fallback} else {err}`，仅 fallback 文案不同。
- **问题**: 同一逻辑写 8 遍，文案散落，易漏改/写错形态。最大体量复制粘贴。
- **改造方案**: 加扩展 trait（放 `client.rs`，`pub(super)`）：
  ```rust
  pub(super) trait RpcResponseExt {
      fn ok_or_rpc_error(self, fallback: &str) -> Result<RpcResponse, crate::AppError>;
  }
  impl RpcResponseExt for RpcResponse {
      fn ok_or_rpc_error(self, fallback: &str) -> Result<RpcResponse, crate::AppError> {
          if self.success { Ok(self) }
          else if self.error.is_empty() { Err(fallback.to_string().into()) }
          else { Err(self.error.into()) }
      }
  }
  ```
  调用处统一 `...into_inner().ok_or_rpc_error("...")?;`，返回 `()` 的几处 `.map(|_| ())`。
  - **坑**：`list_agent_models`(60-77) 成功后还要 `serde_json::from_str(&response.data)`，故 helper 必须返回 `RpcResponse`（保 data），不能返回 `()`。
- **复核结论**: CONFIRMED（8 处行号核对一致；`:492` 已排除）。
- **验证**: 后端基线三连；`grep -c "response.error.is_empty()" agent_bridge/mod.rs` 改后应为 0。
- **关联**: C-4、M-7。

### [x] C-4. gRPC connect 样板重复 6 次 + 默认地址两处独立定义
- **类别 / 严重度**: consistency / 中
- **位置**: connect 6 处 `mod.rs:57,177,183,313,358,387`；默认地址 `agent_bridge/client.rs:15-23`（`agent_endpoint`）与 `agent_supervisor.rs:24-30`（`bare_addr`）
- **现状**: 5 处形态相同 `FutureAgentClient::connect(endpoint.clone()).await.map_err(|e| format!("Unable to connect to Future Agent at {endpoint}: {e}"))?`；第 6 处 `:313`(`wait_for_agent_idle`) 是 let-else 静默返回。默认 `127.0.0.1:50051` 在两文件各硬编码一份（supervisor 注释自承 mirrors client）。
- **问题**: connect+文案样板 5 处复制；默认地址两份，改一处忘另一处会让 GUI 与 sidecar 监督器端口对不上。
- **改造方案**:
  1. `client.rs` 加 `pub(super) async fn connect_agent() -> Result<FutureAgentClient<Channel>, AppError>`（落地 B-15 时返回 `AppError::AgentUnavailable(endpoint)`）。mod.rs 5 处带文案 connect 改调；`:313` 保留静默 `let Ok(c) = connect_agent().await else { return; }`。
  2. 默认地址单源：共享处放 `pub(crate) fn raw_agent_addr() -> String`（supervisor 现 `bare_addr` 逻辑，去 scheme），`agent_endpoint` 基于它加 `http://`，`bare_addr` 调它。`127.0.0.1:50051` 只出现一次。
- **复核结论**: CONFIRMED（6 处行号核对，含 `:313`，原 claim 写 :312 已更正；两份默认地址属实）。
- **验证**: `grep -rn '127.0.0.1:50051' src/` 改后应仅 1 处 + 后端基线三连。
- **关联**: B-15、C-3、M-7。

### [x] C-5. `agent_dir()` 与 `FUTURE_PROVIDER_ID` 在两文件各定义一份
- **类别 / 严重度**: consistency / 中
- **位置**: `auth_store.rs:19`(const)、`:21-24`(`agent_dir`)；`agent_providers.rs:17`(const)、`:376-379`(`agent_dir`)
- **现状**: `agent_dir()`（home→`.future/agent`）两份逐字相同；`const FUTURE_PROVIDER_ID = "future"` 两处同值。
- **问题**: agent 配置目录与内置 provider id 是 ER.md §6.9 关键约定，两份独立易分叉。
- **改造方案**: 提升到 `pub(crate)`（以 auth_store 为家——它已是 auth.json 唯一写入口）：`pub(crate) fn agent_dir()` + `pub(crate) const FUTURE_PROVIDER_ID`，agent_providers 删本地版改引用。`models_json_path`(381-383) 引用共享 `agent_dir` 后保留。
- **复核结论**: CONFIRMED（两文件各一份，无第三处）。
- **验证**: `grep -rn 'fn agent_dir\|const FUTURE_PROVIDER_ID' src/` 改后各 1 处 + 后端基线三连。
- **关联**: ER.md §6.9。

### [x] C-6. 把 `*_COLUMNS` 列常量模式推广到所有表
- **类别 / 严重度**: consistency / 中
- **位置**: 现有常量（仅 3）`records.rs:722/758/793`；threads SELECT 重复 3×（`threads.rs:11-13/26-28/94-96`）；workspaces 4×（`workspaces.rs:11/49/86` + `db.rs:139-141`）；artifacts 读取 3×（`artifacts.rs:16/210` + `resolve.rs:128`）；runs 3×（`runs.rs:31` + `db.rs:81` + `resolve.rs:145`）；approval_requests 3×（`approvals.rs:64` + `db.rs:117` + `resolve.rs:182`）
- **现状**: 只有 review 三表用共享常量 + `format!` 注入；其余手写 SELECT 列清单，与 `*_from_row`(588 起) 靠位置对应。
- **问题**: schema 加列若漏改任一处 → 列错位映射（编译期发现不了的静默错误）。
- **改造方案**:
  1. 为每个 record 加列常量（与 `*_from_row` 顺序逐字一致）：`THREAD_COLUMNS`/`WORKSPACE_COLUMNS`/`MESSAGE_COLUMNS`/`RUN_COLUMNS`/`RUN_EVENT_COLUMNS`/`TOOL_CALL_COLUMNS`/`TOOL_OUTPUT_COLUMNS`/`APPROVAL_REQUEST_COLUMNS`/`ARTIFACT_COLUMNS`/`RESEARCH_RESOURCE_COLUMNS`/`RESEARCH_COLLECTION_COLUMNS`。
  2. 各查询改 `format!("SELECT {THREAD_COLUMNS} FROM threads WHERE ...")`（同 `review_snapshots.rs:65`）。
  3. **坑**：带 JOIN 需别名限定时，沿用 `review_snapshots.rs:193-197`/`resolve.rs:205-209` 的 `COLUMNS.split(", ").map(|c| format!("r.{}", c.trim()))`。`research_resources` 的 `workspace_id` 来自 join 的 `research_collections.workspace_id`，**不能简单加 `r.` 前缀**，建议该表单列处理或暂不常量化。
- **复核结论**: 已修正计数——threads=3×（非 4，第 4 处是 INSERT）；workspaces=4×（含 db.rs:139，上修）；artifacts 读取=3×（上修）。3 个现有常量位置准确。
- **验证**: 后端基线三连；另人工核对每个常量与对应 `*_from_row` 字段顺序（列错位会在运行期 panic）。
- **关联**: M-5、N-3；ER.md §3/§5。

### [x] C-7. markdown_refs 每类对象的 `{title, subtitle, search_text}` 构造在 sync/search 两处重复
- **类别 / 严重度**: consistency / 中
- **位置**: `store/markdown_refs/sync.rs:65-264`（`resolve_reference_target_metadata`，6 分支）vs `search.rs:39-258`（6 个 `search_*_targets`）。（`resolve.rs:122-240` 只取整行、不构造展示元数据。）
- **现状**: 同对象类型展示元数据两处各写一遍。subtitle 公式 tool/approval 完全相同（`format!("{kind} · {status}")`，sync 159/195、search 139/177）；review subtitle 两处重复（sync 226-228、search 209）；search_text sync 用手写 `flatten().join("\n")`、search 用 `compact_search_text`(294)——两实现做同一件事但不一致，是潜在分歧点。
- **问题**: 任何文案改动须每类改 2 处，且两套 search_text 算法不同易只改一处。
- **改造方案**: 新建 `store/markdown_refs/metadata.rs`，每类一个函数返回统一三元组：
  ```rust
  pub(super) struct ReferenceMetadata { pub title: String, pub subtitle: Option<String>, pub search_text: Option<String> }
  pub(super) fn artifact_metadata(...) -> ReferenceMetadata; // run/tool/approval/review/research 同款
  ```
  sync 的 `ReferenceTargetMetadata`(58-63) 与 `ReferenceTargetSearchResult`(records.rs:308) 都改填充 `ReferenceMetadata`；统一用 `compact_search_text`(提升到 metadata.rs)。
  - **坑**：run title 用 `format!("Run {}", short_id(&id))`(sync 127/search 95)，需 `use super::short_id`；artifact/research subtitle 回退是 `path.or(Some(type))`，保持 `.or()` 不要改 `unwrap_or`。
- **复核结论**: 已修正——共享构造器只需覆盖 sync+search **两处**（resolve 不构造展示元数据，原说「3 places」修正）。
- **验证**: 后端基线三连。
- **关联**: M-5；ER.md §4.20。

### [x] C-8. 读回样板 + 散落的状态字符串字面量
- **类别 / 严重度**: consistency / 中
- **位置**:
  - (a) `get_X(&id)?.ok_or_else(|| "X could not be loaded.".into())` 共 **25** 处：`threads.rs:56/88/121/142/158/171/185/206`、`runs.rs:25/90/148`、`workspaces.rs:79`、`messages.rs:62`、`artifacts.rs:13/65/72/231`、`research.rs:30`、`review_snapshots.rs:56/169`、`approvals.rs:98`、`cleanup.rs:11/13`、`db.rs:164/185`
  - (b) `status IN ('completed','failed','cancelled')`：`cleanup.rs` **10** 处(83/94/105/114/124/135/145/156/165/172) + `runs.rs:44`(matches!)/`:112`(NOT IN)；`'waiting_approval'` 仅 `cleanup.rs:51`
- **改造方案**:
  - (a) `util.rs` 加 `pub(super) fn loaded<T>(opt: Option<T>, what: &str) -> Result<T, crate::AppError>`，调用处 `loaded(get_thread(&id)?, "Created thread")?`。（B-11 落地后复合写改 `RETURNING` 直接拿回插入行，可消除大部分读回。）
  - (b) 加模块级常量 `pub(super) const TERMINAL_RUN_STATUSES: &str = "'completed', 'failed', 'cancelled'";`（放 db.rs 或新 status.rs），cleanup SQL 用 `format!`；`runs.rs:44/112` 另用 `&[&str]` 切片常量保持单一真相。
- **复核结论**: 已修正——(a) 实为 **25** 处（更全）；(b) cleanup 实为 **10** 处（非 8，上修）。**撤销**原报告「cleanup.rs 仍写裸 `'run_snapshot'`」——经核 `cleanup.rs` 无任何 `run_snapshot` 字面量，且 `RUN_SNAPSHOT_STATUS`(review_snapshots.rs:14="n/a") 是 `status` 列哨兵，与 `source_kind='run_snapshot'` 是两个不同列，不应混谈。
- **验证**: 后端基线三连。
- **关联**: B-11（RETURNING 消除读回）、M-5。

### [x] C-9. `isRecord` 与「空白折叠/截断」工具在多处重复；`buildReferencePrompt` 版缺 `!Array.isArray` 守卫
- **类别 / 严重度**: consistency / 中
- **位置**: `isRecord` 8 处：`agentActivity.ts:426`(有守卫)、`buildReferencePrompt.ts:144`(**缺**守卫)、`ApprovalPrompt.tsx:288`(有)、`AgentThread.tsx:416`(有)、`MarkdownContent.tsx:433`、`ObjectEmbed.tsx:193`、`RunsPanel.tsx:315`、`RunInspectPanel.tsx:541`；空白/截断：`compactTarget`(agentActivity.ts:418)、`singleLine`(buildReferencePrompt.ts:192)、`truncateForPrompt`(AgentThread.tsx:411)、`escapeMarkdownLinkLabel`(Composer.tsx:554)
- **现状**: 4 个 agent 域 `isRecord` 中 3 个含 `!Array.isArray(value)`，`buildReferencePrompt.ts:144` 缺该守卫（会把数组判为 record）。`compactTarget`/`singleLine` 字节相同（`value.replace(/\s+/g," ").trim()`）。
- **问题**: 同谓词 8 份拷贝且语义不一致（数组判定漂移，潜在隐患）。
- **改造方案**: 新建 `src/lib/objects.ts`（无业务依赖），导出统一 `isRecord`(带 `!Array.isArray`)、`singleLine`、`truncate`、`truncateForPrompt`。替换 8 处 `isRecord`（注意替换 `buildReferencePrompt.ts:144` 时语义收紧为「数组不算 record」——确认 `isArtifact` 等守卫负载不是数组，当前看均为对象，安全）；统一 `compactTarget`/`singleLine`；`truncateForPrompt` 与 M-3 协同。`escapeMarkdownLinkLabel` 内部先调 `singleLine` 再转义。`MarkdownContent.tsx:433` 的版本若允许数组（语义不同）需单独确认。
- **复核结论**: CONFIRMED（8 处 isRecord 核对；`buildReferencePrompt.ts:144` 确缺守卫）。
- **验证**: 前端基线三连；为 `lib/objects.ts` 加单测（`isRecord([])===false` 等）。
- **关联**: M-3；CLAUDE.md `lib/` 约定。

### [x] C-10. Run 状态文案两套真相 + RunError 摘要组件重复
- **类别 / 严重度**: consistency / 中
- **位置**: `runDisplayFormatters.ts:3-18`(`formatRunStatus`，小写) vs `RunsPanel.tsx:233-248`(`runStatusLabel`，Title-case 且用词不同)；`RunsPanel.tsx:319-339`(`RunErrorSummary`) vs `RunInspectPanel.tsx:545-565`(`RunErrorBanner`)
- **现状**: 同 `StoredRun["status"]` 两套映射、用词不一致（`completed`→`completed` vs `Success`；`waiting_approval`→`approval` vs `Waiting`）。两错误组件都用 `formatErrorType` 渲染 icon+label+message，仅容器样式不同（banner 有边框/不截断 vs inline `line-clamp-2`）。
- **改造方案**:
  - 单一真相：`runDisplayFormatters.ts` 为唯一来源，给 `formatRunStatus` 加 `style: "label"|"badge"` 或新增 `runStatusLabel` 也放此并两处 import；删 `RunsPanel.tsx:233-248` 本地版（唯一调用 :144）。先统一用词（如 `completed→"Completed"`）。
  - 统一组件：新建 `src/features/runs/RunError.tsx`，`RunError({ errorMessage, errorType, variant: "summary"|"banner" })`，按 variant 切容器样式。RunsPanel :175 / RunInspectPanel :115 改用之，删两本地组件。
  - **坑**：`formatErrorType` 的 `color` 是分类色例外，合并时不改那些裸色。
- **复核结论**: CONFIRMED（两套映射 + 两组件近重复属实）。
- **验证**: 前端基线三连 + `make run-gui` 确认 RunRow/RunInspect/RunEmbed 状态词一致。
- **关联**: COLOR.md「分类色」例外、U-10。

### [x] C-11. `recentRunEventCount` 死状态 + 每个轮询 tick 多发一次 `listRunEvents`
- **类别 / 严重度**: consistency / 中（含无谓性能开销）
- **位置**: 声明 `useAgentThreadState.ts:50`；set 122/125/229；return 473；多余调用块 117-126（`const events = await listRunEvents(latestRun.id)`）
- **现状**: `recentRunEventCount` 只写不读（`AgentThread.tsx:56-63` 解构未取）；`refreshRecentRun` 每个 1.5s tick + handleSend 都为它额外发一次 `listRunEvents`。
- **改造方案**: 删 state(50)、set 点(122/125/229)、return(473)、为它而存在的 `listRunEvents` 块(117-126 整段)。**注意**：`updatePendingMessageFromRunEvents`(486) 的独立 `listRunEvents` 用于流式预览，必要，勿删。
- **复核结论**: CONFIRMED（全仓仅 useAgentThreadState 出现，AgentThread 未消费；删后每 tick 省一次 gRPC）。
- **验证**: 前端基线三连；删后确认 hook 返回类型变化无外部报错。
- **关联**: B-1、B-2。

### [x] C-13. 审批决策逻辑被 AppShell 与 `useApprovals` 割裂
- **类别 / 严重度**: module / 中
- **位置**: 手动决策 `AppShell.tsx:326-337`；自动审批 `useApprovals.ts:50-71`；`pendingApprovals` 导出 `useApprovals.ts:10/73`（AppShell 未消费）；`activeApproval` 重过滤 `:42-48`；loader 已只返 pending `:28-30`
- **现状**: `useApprovals` 拥有 loader + 自动审批引擎（用 `decideApprovalRequest`），AppShell 又另写一份手动决策（也 import+调 `decideApprovalRequest`）。`pendingApprovals` 无外部消费者（AppShell:90 仅取 activeApproval/reloadApprovals）。`activeApproval` memo 重过滤 `status==="pending"`（loader 已过滤，冗余）。
- **改造方案**（同名解构暴露）：
  1. `useApprovals` 新增并返回 `decideApproval(approval, status): Promise<void>`（搬 AppShell 330-335 的 decide+reload）。`refreshStore` 不属审批 hook：AppShell `handleApprovalDecision` 收缩为 `await decideApproval(...); await refreshStore(...)`。
  2. `ApprovalsState` 加 `decideApproval`；AppShell 解构改 `{ activeApproval, decideApproval, reloadApprovals }`。
  3. 删导出 `pendingApprovals`(10/73)（内部仍可用该变量名）。
  4. 删 `activeApproval` memo 内的 `.filter(... status === "pending")`(45)。
- **复核结论**: CONFIRMED（四点全核对；`decideApprovalRequest` 双 import AppShell:21/useApprovals:3；pendingApprovals 无外部消费）。
- **验证**: `cd gui && grep -rn "decideApprovalRequest" src/`（改后仅 useApprovals）+ 基线三连。
- **关联**: M-2；CLAUDE.md §5、§8。

---

# Bug / 隐患（Bug）

### [x] B-1. 轮询时按 `pending_` 前缀粗暴覆写消息内容，冲掉流式正文并误伤乐观用户气泡
- **类别 / 严重度**: bug / 高
- **位置**: `src/features/agent/useAgentThreadState.ts:104-115`
- **现状**: `refreshRecentRun` 每个 1.5s tick 拉到 `waiting_approval` 时，把**所有** id 以 `pending_` 开头的消息 `content` 改成固定文案。乐观气泡 id 由 `clientId("pending")`（助手）与 `clientId("pending_user")`（用户）生成（142-143），两者都 `startsWith("pending_")`。
- **问题**: (1) 乐观**用户**气泡正文也被改成审批文案；(2) 每 1500ms 反复把助手流式正文（`updatePendingMessageFromRunEvents` 在 220ms tick 写入，504）覆写成固定文案，已流出文本被清空/闪烁。
- **改造方案**: 收窄到「正在流式的助手消息」且不动 `content`。最稳妥：**删掉这段 `setMessages`**，审批提示完全由 `ApprovalPrompt` 承担（AgentThread 已通过 `activeApproval` 渲染它，本段是冗余 UI）。若保留：判定改 `message.role === "assistant" && message.status === "streaming"`，且仅在 `!message.content.trim()` 时填充。
- **复核结论**: CONFIRMED（前缀范围属实，pending/pending_user 都命中；两段对同一 pendingId 助手消息竞争写 content）。
- **验证**: 前端基线三连 + `make run-gui` 触发需审批的 run，确认用户气泡与助手流式正文在等待期间不被改写。
- **关联**: C-11；CLAUDE.md 原则 4。

### [x] B-2. `Promise.all` 让 run 加载失败拖垮整屏（消息已加载也报错）
- **类别 / 严重度**: bug / 中
- **位置**: 调用 `useAgentThreadState.ts:410`；`refreshRecentRun` 定义 93-127（无 try/catch）
- **现状**: `await Promise.all([listMessages(threadId), refreshRecentRun(...)])`。`refreshRecentRun` 内两处 `await`(95/118)无 try/catch，一旦 reject，`Promise.all` 整体 reject，落外层 catch(417-429) 显示「FutureOS 消息读取失败」，即便 `listMessages` 成功。其他 loader 均有容错(570-578、540-547、581、588-593)。
- **改造方案**: 行 410 改 `Promise.allSettled` 分别处理；或给 `refreshRecentRun` 包 try/catch 静默返回（保留旧 `recentRun`）。后者对轮询(450)/handleSend(279/331) 调用同样有益。
- **复核结论**: CONFIRMED（refreshRecentRun 无 try/catch，其余 loader 有）。
- **验证**: 前端基线三连；可加单测 mock `listRuns` reject + `listMessages` resolve，断言 `messages` 为真实消息。
- **关联**: C-11、B-1。

### [x] B-3. check-then-insert 缺唯一索引/事务 → 并发下可能插入重复行
- **类别 / 严重度**: bug / 中
- **位置**: `store/approvals.rs:7-56`（`ensure_approval_request`，SELECT 10-20 → 条件 INSERT 28-54，dedup 键 `(tool_call_id, kind)`）；`store/artifacts.rs:111-157`（`ensure_artifact`，dedup 键 `(run_id, title, COALESCE(path,''))`）。唯一约束仅 `schema.rs:168` `UNIQUE(run_id, phase)`。
- **现状**: 两者都在单条 `connect()`（自动提交）上「先查后插」，无事务、无唯一索引。`upsert_tool_call`(runs.rs:177) 已用 `ON CONFLICT(id) DO ...` 正确处理同类问题。
- **问题**: agent 事件流并发触发同键 ensure → 两次 SELECT 都空 → 双 INSERT → 重复行。
- **改造方案**:
  1. 加部分唯一索引（放 `schema.rs` 的 `ADDED_INDEXES`）：
     ```sql
     CREATE UNIQUE INDEX IF NOT EXISTS idx_approval_requests_dedup
       ON approval_requests(tool_call_id, kind) WHERE tool_call_id IS NOT NULL;
     CREATE UNIQUE INDEX IF NOT EXISTS idx_artifacts_dedup
       ON artifacts(run_id, title, COALESCE(path, '')) WHERE deleted_at IS NULL AND run_id IS NOT NULL;
     ```
  2. INSERT 改 `INSERT ... ON CONFLICT DO NOTHING`（同 runs.rs:184），可删先行 SELECT。
  - **坑**：建索引前确认现有数据无冲突；artifacts 部分索引须带 `WHERE deleted_at IS NULL`（否则与 `delete_artifact` 软删冲突）。app pre-release 无迁移成本。
- **复核结论**: CONFIRMED（两处 check-then-insert + dedup 键准确；schema 确无这两表唯一索引）。
- **验证**: 后端基线三连；建议补并发回归测试（同 input 连续两次 ensure 后断言行数=1）。
- **关联**: B-11、`runs.rs:177` upsert 范式。

### [x] B-4. `cancel_stale_approval_requests` 两条 UPDATE 的 WHERE 不对齐 → run/approval 状态错位
- **类别 / 严重度**: bug / 中
- **位置**: `store/cleanup.rs:41-71`
- **现状**: 第一条 UPDATE 只把 `status='waiting_approval'` 且有 pending approval 的 run 置 cancelled(45-59)；第二条把**每条** `status='pending'` approval 置 cancelled，不看 run 状态(60-68)。两条同一事务内(44/69)。
- **问题**: 挂在 `running` run 上的 pending approval 被取消，但 run 仍 running → 状态错位（run 永远等不到已取消的审批）。
- **改造方案**: 让两条作用于同一 run 集合。方案 A（推荐对称）第二条限定到第一条选中的 run：
  ```sql
  UPDATE approval_requests SET status='cancelled', ... WHERE status='pending'
    AND (run_id IS NULL OR run_id IN (SELECT id FROM runs WHERE status IN ('waiting_approval','cancelled')));
  ```
  方案 B：放宽第一条，任何持 pending approval 的 run 都 cancel。需与产品语义确认重启后是否取消挂在 running 上的 pending approval。
- **复核结论**: CONFIRMED（WHERE 集合错位属实；同事务内，故是逻辑错位非原子性问题）。
- **验证**: 后端基线三连；补测：running run 挂 pending approval，调用后断言 approval 与 run 状态一致。
- **关联**: B-10、B-11；ER.md §4.8。

### [x] B-5. `git_review.rs` 缺 `-c core.quotePath=false`，非 ASCII 文件名 diff 丢失
- **类别 / 严重度**: bug / 中
- **位置**: `src-tauri/src/git_review.rs:117`(numstat)、`:120`(unified diff)、`:244`(status)，其余 git 调用 57/60/64/98/171/187/217。对照 `shadow_review/diff.rs:133/229`（已带）。
- **现状**: git_review 所有 git 调用都不传 `core.quotePath=false`。`split_git_diff_by_path`(284) 按 `+++ b/` 与 `diff --git ... b/` 取路径，`tracked_diff_files`(111) 用 numstat 路径做 key 查 diff map。
- **问题**: git 默认 `quotePath=true`，非 ASCII 路径被引号+octal 转义，numstat（`splitn(3,'\t')` 字面取、不 unquote）与 diff header 路径形式不一致 → `diff_by_path.get(...)` 落空 → 文件显示「modified」但 per-file diff 为空。`git_status_by_path`(241-263) 按字节切片同样不 unquote。
- **改造方案**: 最稳妥在 `git_output` 内统一注入：`Command::new("git").arg("-C").arg(workspace_path)` 后、`.args(args)` 前插 `.args(["-c", "core.quotePath=false"])`(367-371)。或仅对 numstat(117)/diff(120)/status(242-245) 三处加，与 diff.rs 一致。
- **复核结论**: CONFIRMED（git_review 无 quotePath；diff.rs 两处有；numstat/diff/status 行号 117/120/244）。
- **验证**: `git init` 建中文名文件 commit+改动，调 `get_git_review` 断言对应 file 的 `diff` 非空 + 后端基线三连。
- **关联**: M-6（抽共享 helper 时统一）；ER.md §6.8。

### [x] B-6. fire-and-forget materialize 任务的恢复缺口
- **类别 / 严重度**: bug / 中
- **位置**: spawn `agent_bridge/mod.rs:141-149`；恢复 `shadow_review/maintenance.rs:88-95` + `recover_one:97-162`；判定 SQL `store/review_snapshots.rs:338-361`
- **现状**: after 快照在 `agent_prompt` 同步捕获(mod.rs:130-138)，materialize 丢进 detached `tokio::spawn`，JoinHandle 丢弃。恢复 SQL 只覆盖「有 before、无 after」(345 `NOT EXISTS ... phase='after'`)。
- **问题**: 若 app 在 materialize 跑完前退出，DB 已有 before+after 但无 changeset 行 → `list_interrupted_runs` 的 `NOT EXISTS(after)` 把它排除 → 启动恢复(lib.rs:92) 不重算 → 该 Run「上一轮变更」永久缺失，`review-updated` 永不发出。
- **改造方案**（建议 A）扩恢复判定：新增 `list_unmaterialized_runs()`（去掉 after 的 `NOT EXISTS`、保留 changeset 的 `NOT EXISTS`），在 `recover_one` 复用 `materialize`（两端 commit 都在时用 `confidence="normal"`）。与现有 interrupted 集合去重（`upsert_run_changeset` 已幂等，重复执行安全）。方案 B（排空任务）无法覆盖崩溃，不如 A。
- **复核结论**: CONFIRMED（spawn+丢弃 handle、after 同步捕获、`NOT EXISTS(after)` 排除场景均核对）。
- **落地结论**: 采用方案 A。`list_interrupted_runs` → `list_unmaterialized_runs`（去掉 `NOT EXISTS(after)`，保留 `NOT EXISTS(changeset)`），覆盖 interrupted + 完成未 materialize 两种形态。**坑：原 `recover_one` 会把 run 标 cancelled 并重新 capture after——对「已正常完成、仅缺 changeset」的 B-6 是错的。** 故 `recover_one` 按 after 是否存在分支：after 存在→复用、不标 cancelled、不重 capture、`confidence="normal"`（与正常 materialize 一致）；after 缺失→沿用原 interrupted 路径（标 cancelled + capture + `confidence="recovered"`）。共用 materialize+upsert 尾段。抽 `list_unmaterialized_runs_in(conn)` 便于单测，补 3 例（含 B-6 形态被纳入、已 materialize 被排除）。
- **验证**: 后端基线三连通过（cargo test 56 例，含新增 3 例）。
- **关联**: review.rs materialize 流水线；maintenance.rs §6.6。

### [x] B-7. 嵌套 Overlay 的全局 Escape 同时关闭内外两层
- **类别 / 严重度**: bug / 高
- **位置**: `src/components/ui/Overlay.tsx:18-29`（全局 keydown）；嵌套：`SettingsDialog.tsx:74`（外层 Overlay）→ `:132`(`ProvidersPage`)→ `ProvidersPage.tsx:190/204`(`CustomProviderDialog`/`FutureLoginDialog`，经 `Dialog.tsx:25` 包 Overlay)
- **现状**: Overlay open 期间无条件挂 `window` keydown，任意 Escape 调自己 `onClose`，无 `stopImmediatePropagation` 也无「最顶层才响应」判定。设置弹层打开时外层+内层 Overlay 同时挂着监听。
- **问题**: 在自定义 Provider/登录弹层按 Escape，内层关的同时外层 SettingsDialog 也一起关。
- **改造方案**: 引入 overlay 栈让 Escape 只命中栈顶。新增 `src/components/ui/overlayStack.ts`：模块级 `stack` + `useOverlayLayer(open): { isTop() }`（effect 内 open 时 push 唯一 `Symbol()`、cleanup 移除）。`Overlay.tsx` 把判定改为 `if (event.key === "Escape" && layer.isTop()) onClose();`，调用点无需改。
  - **坑**：window 级监听靠「栈顶」判定而非 `stopPropagation`（后者在 window 监听器间无效）；push/pop 在同一 effect setup/cleanup 配对，用 effect 内 `Symbol()` 避免 StrictMode 双调用泄漏。
- **复核结论**: CONFIRMED（嵌套链 + 缺守卫属实）。
- **验证**: 前端基线三连 + `make run-gui`：设置→Provider→新建自定义 Provider→按 Escape，预期只关内层。
- **关联**: 无。

### [x] B-8. PdfPreview 渲染 effect 依赖 ref 而非文档身份，换路径可能画旧页 + innerHTML 清空竞态
- **类别 / 严重度**: bug / 中
- **位置**: `src/features/artifacts/PdfPreview.tsx:66-115`（渲染 effect，deps `[currentPage, loading, error]`，:115）；加载 effect 23-64（deps `[path]`）；`loadingTaskRef` :21
- **现状**: 渲染 effect 文档身份只经 `loadingTaskRef.current`（ref，非响应式）读取，依赖不含 path/文档身份。`renderPage` 先同步 `containerRef.current.innerHTML = ""`(79) 再 `await`，`appendChild` 前无 `cancelled` 守卫（仅末尾 `page.cleanup()` 有 :99）。
- **问题**: (1) 缺 path 依赖：path 切换主要靠 `loading` 翻转救活，但时序异常时可能画旧页/取越界页；(2) innerHTML 竞态：旧 await 链恢复后仍 `appendChild` 到新容器。
- **改造方案**: 把文档存 state：`const [pdfDoc, setPdfDoc] = useState<pdfjs.PDFDocumentProxy|null>(null)`，加载 effect 拿到后 `setPdfDoc`，渲染 effect 用 `pdfDoc`、deps 改 `[pdfDoc, currentPage]`；每个 `await` 后、`appendChild` 前补 `if (cancelled) return;`，`innerHTML=""` 延后到即将 append 前。最小改动版：仅把 `path` 加进 deps + append 前加 cancelled 守卫。
  - **坑**：换 path/卸载仍需 `destroy()` 旧 task（加载 effect cleanup 已有 57-63），避免对已 destroy 的 task 取 `.promise`。
- **复核结论**: CONFIRMED（deps :115 无 path；innerHTML :79 同步且 append 前无守卫）。降级：「换路径必画旧页」偏强（被 loading 翻转部分掩盖），故定**中**而非高。
- **验证**: 前端基线三连 + `make run-gui`：同面板连续切两个 PDF + 快速翻页，确认无错页/旧 canvas 残留。
- **关联**: CLAUDE.md 原则 4（加载可改走 `lib/useAsyncResource`）。

### [x] B-9. `setAttachError` 被调用在 `setAttachments` 函数式更新器内部（不纯更新器反模式）
- **类别 / 严重度**: bug / 中
- **位置**: `src/features/agent/Composer.tsx:188-206`（`addAttachmentPaths`）
- **现状**: 更新器回调里算 `next` 的同时调 `setAttachError(...)`(204)。
- **问题**: 更新器须纯函数；StrictMode 下被调两次 → `setAttachError` 执行两次；并发特性下更新器可能被丢弃/重放，副作用时机不可控。
- **改造方案**: 基于 `attachments`（加入 deps）在更新器**外**先算 `next`/`rejected`，再分别 `setAttachments(next)` + `setAttachError(...)`，两 setter 都在更新器外。
- **复核结论**: CONFIRMED（setAttachError 在 setAttachments 更新器内）。
- **验证**: 前端基线三连 + StrictMode 下粘贴超 `MAX_ATTACHMENTS_PER_TURN` 文件，确认错误提示只设一次。
- **关联**: M-1。

### [x] B-10. `mark_run_overlapped` 多步读写无事务（TOCTOU）
- **类别 / 严重度**: bug / 中
- **位置**: `store/review_snapshots.rs:217-268`（辅助 `set_overlapped` 270-278）
- **现状**: 单条 `connect()`（无事务）依次 SELECT before_ts(221-227)→ after_ts(231-238)→ peers(242-257)→ 逐个 `set_overlapped` UPDATE(263-266)。
- **问题**: peers-SELECT(242) 与 UPDATE(263) 之间，并发 run 若写入 after 快照/changeset，本次 peer 集会漏掉 → 重叠标记缺失；N 条独立 UPDATE 非原子。
- **改造方案**: `BEGIN IMMEDIATE` 事务包住整段（`conn.transaction_with_behavior(TransactionBehavior::Immediate)?` 全程用 tx，末尾 commit）；或合并为单条 set-based UPDATE 把 peer 子查询内联进 WHERE。
  - **坑**：B-11 未解时跨连接并发仍是多连接竞争，本修复只保证本函数内一致。
- **复核结论**: CONFIRMED（:218 裸 connect 全程无 transaction，TOCTOU 窗口真实）。
- **验证**: 后端基线三连。
- **关联**: B-11；ER.md §6.8。

### [~] B-11. 每个 store 函数各开新连接；复合操作跨多连接、非原子
- **类别 / 严重度**: bug/perf / 高
- **位置**: `store/db.rs:30-39`（`connect()`，每次 open + 3 条 PRAGMA）。典型：`threads.rs:40-89`(`create_thread`)、`messages.rs:21-63`(`append_message`)
- **现状**: `create_thread` 调 `get_or_create_*_workspace`/`get_workspace`（各 connect）→ connect 做 INSERT(68)→ `get_thread`（又一 connect，88）= 3-4 连接，workspace 创建与 thread INSERT 不在同事务；`append_message` 提交 tx 后(26) 又开第二条连接 `get_message` 读回(62)。
- **问题**: (1) 每调用 open + 3 PRAGMA（含 WAL 写）开销，高频路径显著；(2) `create_thread` 跨连接非原子 → 崩溃留孤儿 workspace；(3) 跨连接读回是不必要复杂度。
- **改造方案**:
  1. 共享连接：`OnceLock<Mutex<Connection>>`（或 r2d2/deadpool 池），PRAGMA 初始化设一次；`connect()` 改为返回 guard 或把 `&Connection` 传入各 helper。
  2. 复合写改单事务：`create_thread` 内 `let tx = conn.transaction()?;`，workspace 解析/创建与 INSERT 共用 tx；为此把 `get_or_create_*_workspace`/`get_workspace`/`get_thread` 等改造为接收 `&Connection`/`&Transaction` 的内部变体（保留对外自开连接的包装）。
  3. 写后读回改 `RETURNING`（SQLite≥3.35），消除第二次查询——同时落地 C-8(a)。
  - **坑**：`Mutex<Connection>` 串行化所有访问，需并发读用池+WAL；`clear_all_data`(store.rs:78) 的 `PRAGMA foreign_keys=OFF` 不能污染共享连接长期状态；测试 `db.rs:188-235` 用 in-memory DB，重构须保留可注入连接的测试入口（共享连接对 in-memory 是必要的）。
- **复核结论**: CONFIRMED（connect 每次新建 + 3 PRAGMA；create_thread 跨 3-4 连接非同事务；append_message 提交后第二条连接读回）。
- **进度（本批，部分落地）**: 已修复**核心正确性 bug**——`create_thread` 现在单连接单事务（workspace 解析/创建 + thread INSERT + 读回同一 `tx`），崩溃不再留孤儿 workspace。手段：给 `get_or_create_chat_workspace`/`get_workspace`/`get_or_create_user_workspace`/`get_thread` 加 `*_in(&Connection, …)` 注入变体（公有版退化为自开连接的薄包装），`&tx` deref 成 `&Connection`；补两条 in-memory 回归测试（原子写可见 / 回滚不留孤儿）。**暂缓**架构级的「全局共享连接 / 连接池 + RETURNING」部分，理由：(1) 把 `connect()` 改成返回 guard 会让每条「持锁后再调用另一个 store 函数」的复合链（create_thread→workspace、append_message→read-back、promote_artifact→get_artifact+collection、get_thread_cleanup_summary→get_thread+get_workspace 等数十处）变成**编译期发现不了的运行期死锁**，安全地做需要把 `&Connection` 串进整个 store 层（~60 个函数、每文件都动）；(2) 现有测试全部绕过 `connect()`（直接 `open_in_memory` 注入 `&Connection`），没有走 `connect()` 的集成测试能在 CI 兜住死锁回归，而此环境跑不起 Tauri GUI 实机验证；(3) 单用户桌面应用并发极低，「每调用 open + 3 PRAGMA」的性能收益相对上述风险偏小。建议把全局连接/池作为独立一次 pass，配 `make run-gui` 实机回归（含线程切换、复合写、`clear_all_data` 后 PRAGMA 状态）再做。
- **验证**: 后端基线三连（确保 in-memory 测试仍可运行）。
- **关联**: B-3、B-10、C-8(a)、M-5。

### [x] B-12. 热点查询缺索引
- **类别 / 严重度**: bug/perf / 中
- **位置**: 现有索引 `schema.rs:342-349` + `ADDED_INDEXES`(398-400)
- **现状**: 下列高频 WHERE/JOIN 列确认无索引：

  | 缺失索引 | 驱动查询 |
  |---|---|
  | `tool_calls(run_id)` | `runs.rs:154` list_tool_calls、resolve.rs:165/sync.rs:137/search.rs:121 JOIN |
  | `tool_outputs(tool_call_id)` | `runs.rs:167` list_tool_outputs、cleanup.rs:99 子查询 |
  | `review_file_changes(changeset_id)` | `approvals.rs:106`、cleanup.rs:129、review_snapshots.rs:96/287 |
  | `approval_requests(thread_id)` | `approvals.rs:68` list_approval_requests |
  | `approval_requests(run_id, status)` | `runs.rs:70-76`、cleanup.rs:54-56/119-127 |
  | `artifacts(workspace_id, deleted_at)` | `artifacts.rs:18-22`、search.rs:48、sync.rs:73 |
- **问题**: 这些查询走全表扫；`tool_calls`/`tool_outputs`/`review_file_changes` 随 run 数线性增长。
- **改造方案**: 加入 `schema.rs` 的 `SCHEMA` 末尾：
  ```sql
  CREATE INDEX IF NOT EXISTS idx_tool_calls_run ON tool_calls(run_id);
  CREATE INDEX IF NOT EXISTS idx_tool_outputs_call ON tool_outputs(tool_call_id);
  CREATE INDEX IF NOT EXISTS idx_review_file_changes_changeset ON review_file_changes(changeset_id);
  CREATE INDEX IF NOT EXISTS idx_approval_requests_thread ON approval_requests(thread_id);
  CREATE INDEX IF NOT EXISTS idx_approval_requests_run_status ON approval_requests(run_id, status);
  CREATE INDEX IF NOT EXISTS idx_artifacts_workspace ON artifacts(workspace_id, deleted_at);
  ```
- **复核结论**: CONFIRMED（逐一核对现有 9+1 索引，上述 6 个确实缺失；驱动查询已列）。
- **验证**: 后端基线三连；可选 `EXPLAIN QUERY PLAN` 确认改用索引。
- **关联**: B-3、ER.md §5。

### [x] B-13. `decide_approval` 续跑 run 用非原子 read-then-write，可覆盖并发 abort
- **类别 / 严重度**: bug / 低
- **位置**: `agent_bridge/mod.rs:451-463`；对照 CAS `store/runs.rs:97-116`(`fail_run_if_active`)；同形 `agent_bridge/persist.rs:140-158`(`update_run_status_if_active`)
- **现状**: `get_run` 读到非终态 → `update_run_status("running")` 无条件写。对照 `mark_run_failed_if_active` 刻意用 `fail_run_if_active` 单条 SQL CAS。
- **问题**: 中间 `abort_run`(423) 把状态写 `cancelled` → 本分支再覆盖成 `running`，run 被错误复活。
- **改造方案**: 新增 `store::set_run_running_if_active(run_id) -> Result<bool, AppError>`（仿 `fail_run_if_active`：单条 `UPDATE ... SET status='running' ... WHERE id=?1 AND status NOT IN ('completed','failed','cancelled')`）。mod.rs:452-463 改 `let _ = store::set_run_running_if_active(run_id);`，删 get_run+matches!。顺带把 `persist.rs:140-158` 用同款 CAS 收口（可共用 `set_run_status_if_active`）。
- **复核结论**: CONFIRMED（`set_run_running_if_active` 当前不存在；三处行号核对）。M-7 后 `decide_approval` 已在 `agent_bridge/approval.rs`。
- **落地结论**: 新增 `store::update_run_status_if_active(input) -> Result<bool, AppError>`（单条 `UPDATE ... WHERE id=?1 AND status NOT IN (terminal)` 的 CAS，命名与 `update_run_status` 对齐）。`approval.rs` 续跑改用之、删 `get_run`+`matches!`。**坑：`persist.rs` 的 `cancelled` 路径依赖 `update_run_status` 的级联**（取消 pending 审批 + running 工具调用），故把级联抽成 `cancel_run_side_effects(tx, run_id, now)` 共享，CAS 仅在 `affected>0 && status=="cancelled"` 时触发，行为不变。`persist.rs` 的本地 `update_run_status_if_active` 删除、两调用点改 `store::update_run_status_if_active`。补测 2 例（终态拒绝、活跃→cancelled 且级联）。
- **验证**: 后端基线三连通过（cargo test 53 例，含新增 2 例）。
- **关联**: B-15、C-3。

### [ ] B-14. `useThreadStore` 的 run-status 刷新有双触发源（重复 fan-out）
- **类别 / 严重度**: bug / 中
- **位置**: `src/components/layout/hooks/useThreadStore.ts:87`、`:114`、`:137-140`；guard `:67-80`(`runStatusGenRef`)
- **现状**: `refreshStore` 显式 `void refreshThreadRunStatuses(...)`(87)、bootstrap 同样(114)，加 1.5s 轮询(137-140，deps `[activeThreads,...]`)。`usePolling` 装载即 tick(usePolling.ts:44)、deps 变即重跑 effect 再 tick。`refreshStore` 的 `setThreads` → `activeThreads` memo 换引用(59-62) → poll effect 重触发立即再跑一次。每次 refresh 产生①显式+②poll 两次近乎同时的 `listRuns` 全量并发；`runStatusGenRef` 只丢弃旧写入(76-78)，不阻止重复工作。
- **改造方案**: 择一 kickoff 源。推荐**删掉 refreshStore/bootstrap 内显式调用**(87、114)，让 poll effect 驱动（`activeThreads` 引用变后 effect 立即 tick 已覆盖）。坑：bootstrap 首帧 `activeThreads=[]`、poll `enabled` 起初 false；若担心首帧空窗保留 bootstrap 那处、仅删 87。guard 保留。
  - **B-14b（低）**: `refreshStore` 的 `useCallback` 依赖含 `activeThreadId`(97)，每次选中变化重建引用并级联。可改用 `activeThreadIdRef`，依赖收敛到 `[refreshThreadRunStatuses]`（91 的 `else if (activeThreadId && ...)` 改读 ref）。
- **复核结论**: CONFIRMED（两触发源 + poll 立即 tick + memo 换引用 + guard 不防重复工作；B-14b 依赖数组 :97 核对）。
- **验证**: 前端基线三连；在 `refreshThreadRunStatuses` 起始处临时 `console.count`，切换/重命名线程改前 +2、改后 +1。
- **关联**: `lib/usePolling`；CLAUDE.md §4、§5。

### [x] B-15. 靠错误字符串子串匹配判定 agent 不可用 / 审批失效，脆弱且与 C-4 文案耦合
- **类别 / 严重度**: bug / 中
- **位置**: `agent_bridge/mod.rs:467-470`(`is_stale_approval_error`)、`:472-474`(`is_agent_unavailable_error`)；消费 `:418`(`abort_run`)、`:441`(`decide_approval`)
- **现状**: `is_agent_unavailable_error` 用 `starts_with("Unable to connect to Future Agent")`（与 C-4 的 6 处 format! 文案逐字耦合）；`is_stale_approval_error` 匹配 agent 返回串 `contains("approval request") && contains("not pending")`。
- **问题**: 文案改动/本地化静默破坏：改前者 → `abort_run` 不再容忍 agent 宕机，本应本地 cancel 的 run 报错卡 running；改 agent 端措辞 → `decide_approval` 失效对账失效。
- **改造方案**:
  - agent 不可用：源头类型化——`AppError` 加 `#[error("Unable to connect to Future Agent at {0}")] AgentUnavailable(String)`(error.rs)，C-4 的 `connect_agent()` 失败返回该变体；`is_agent_unavailable_error` 改 `matches!(err, AppError::AgentUnavailable(_))`，消费点 :418 改对 `&AppError` 判定。同时消除与 C-4 文案耦合。
  - 审批失效：该串来自 agent 无法类型化——保留子串匹配，但抽带文档注释的函数说明「契约依赖 agent 文案」，加 pin 措辞的单测。
- **复核结论**: CONFIRMED（两 matcher + 消费点核对；文案耦合属实）。
- **验证**: 加单测覆盖 stale-approval 匹配；agent-unavailable 改类型后 `cargo test`+clippy + 后端基线三连。
- **关联**: C-4、B-13。

### [x] B-16. `extractPdfText` 的 `await loadingTask.promise` 在 try 之外，reject 时泄漏 PDF worker
- **类别 / 严重度**: bug / 低
- **位置**: `src/features/agent/attachments.ts:182-204`
- **现状**: `const pdf = await loadingTask.promise;`(185) 在 try(186 起) 之外，`destroy()` 在 try 的 `finally`(202)。promise reject（损坏/加密 PDF）时异常直接抛出，`finally` 不执行 → worker 泄漏。
- **改造方案**: 把 `const pdf = await loadingTask.promise;` 移到 try 内第一行（`destroy()` 在文档未 resolve 时调用安全）。
- **复核结论**: CONFIRMED（结构属实）。
- **验证**: 前端基线三连；附加损坏/加密 PDF，以代码审查 + 不抛未处理异常为准。
- **关联**: 无。
