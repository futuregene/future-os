# 边界审计报告 02：gui_rust ↔ gui_react

> 范围：Tauri Rust 层（`gui/src-tauri/src/`）与 React 前端（`gui/src/`）之间的边界。
> 方式：只读审计，未修改任何文件。所有路径相对 `gui/`，行号均已逐条核对。
> 一句话结论：**架构层面边界是干净的，契约层面是全手工且已开始漂移的。** 边界划分（谁拥有什么）是对的，缺的是契约机制（类型如何不漂移）。
>
> ⚠️ **2026-08-06 漂移注记**：结论方向不变（截至复核日仍有 0 处裸 `invoke(`，单一入口 `integrations/tauri/invoke.ts` 未变），但数字与路径已变：`invokeCommand` 调用数 102→**125**；引用文件多处移动（`ResetPage.tsx`→`features/settings/`、`AppShell.tsx`→`components/layout/`、`agentStateCache.ts`→`integrations/agent/`、`typeGuards.ts`→`integrations/storage/`、`MarkdownContent.tsx`→`features/markdown/`、`MessageList/MessageBlock.tsx`→`features/agent/`、`useThreadStore.ts`→`components/layout/hooks/`）。引用具体 file:line 前请按当前工作树核对。

---

## 1. IPC 表面清单

### 1.1 Commands（TS→Rust 调用面）

共 **103 个 `#[tauri::command]`**，全部注册于 `src-tauri/src/lib.rs:600-704`（`generate_handler!`），无遗漏、无未注册命令（定义集 = 注册集，已用 `comm` 比对）。按域分布：

| 域 | 数量 | 域 | 数量 |
|---|---|---|---|
| threads | 21 | approvals | 4 |
| files | 13 | artifacts | 4 |
| runs | 13 | agent | 4 |
| workspaces | 6 | review | 4 |
| skills | 6 | update | 3 |
| remote | 6 | debug | 3 |
| providers | 5 | settings | 2 |
| login | 5 | references | 2 |
| | | app | 2 |

`commands/mod.rs` 注明设计意图："thin wrappers that delegate to store/agent_bridge"——实际也如此，命令层几乎无逻辑（例外见 S6/S12）。注意：`remote/commands.rs` **不是** Tauri 命令，是手机桥 NATS 指令路由（不经 webview）。

### 1.2 Events（Rust→TS 推送面）

共 **8 个事件名**，8 个 emit 点：

| 事件 | emit 点 | payload |
|---|---|---|
| `agent-event` | `agent_bridge/observer.rs:940` | **无类型 `serde_json::Value` 透传**（:921-942） |
| `thread-runtime-updated` | `lib.rs:317`（40ms 合并通道 :290-322） | 类型化 struct `ThreadRuntimeUpdate` `lib.rs:231-241` |
| `approvals-updated` | `lib.rs:359` | 类型化 struct `lib.rs:370-374` |
| `thread-streaming-updated` | `lib.rs:395` | 类型化 struct `lib.rs:343-346` |
| `app-update-progress` | `commands/update.rs:184` | 类型化 struct `update.rs:29-34` |
| `review-updated` / `remote-activity` | `lib.rs:216` / `lib.rs:225` | 裸 `String`（thread_id） |
| `open-settings` | `lib.rs:431` | `()` |

### 1.3 前端消费面

- `invokeCommand` 调用 **102 处**，**0 处裸 `invoke(`**（纪律完美，`integrations/tauri/invoke.ts:25` 是唯一 invoke 点）。
- `listen` 生产调用 **11 处**（9 个直接 + `useTauriEvent` 包装，`lib/useTauriEvent.ts:11`），覆盖全部 8 个事件。
- 前端 **无浏览器侧 gRPC**（零 grpc/connect-web import）——agent 通信全部经 Rust 中转，符合 `DEV_MD/PRODUCT.md §2` 设计。

---

## 2. 耦合 / 异味清单（含双侧 file:line）

### S1【high】agent 域数据以无类型 JSON 穿越边界，三层手工解析

- `get_session_entries` → `Result<serde_json::Value, _>`（`commands/threads.rs:302`），agent 原始 JSON 原样透传；TS 侧类型是 `{ entries: Record<string, unknown>[] }`（`integrations/storage/threads.ts:137`），再手写 snake_case `SessionEntry`（`features/agent/entryProjection.ts:14`，字段 `tool_args` / `output_tokens` / `duration_ms` / `meta.run_id`）。
- `get_thread_agent_state` → `Result<serde_json::Value, _>`（`commands/threads.rs:235`）；TS 以 `Record<string, unknown>` 逐字段收窄（`integrations/agent/agentStateCache.ts:92-103`）。**混合大小写实证**：Rust 自己的 fallback payload（`threads.rs:243-251`）同时含 camelCase（`thinkingLevel`、`sessionId`）和 snake_case（`session_name`）；TS 在相邻两行分别读 `raw.session_name`（:96）与 `raw.sessionId`。TS 的 `AgentSessionState` 接口（`agentStateCache.ts:11`，含 `activeRun` 状态机）**在 Rust 没有任何对应 struct**。
- 全栈无结构化 agent 事件枚举：proto `StreamEvent.data` 是裸字符串（`generated/proto.rs:287,308,321-327`）；`agent_proto.rs` 仅 11 行 `include!`；`stream.rs:60-82` 把 `data` **逐字**存进 SQLite，只在 `:379-410` 用 `Value` 临时抠字段。
- `agent-event` 推送：`observer.rs:921-942` 解析成 `Value` 注入 3 个 key 后 emit；TS `listen<Record<string, unknown>>`（`agentStateCache.ts:239`）手工 typeof 收窄。**死分支**：`agentStateCache.ts:263-271` 处理 `text_chunk` / `thinking_*` / `tool_*`，但 Rust 白名单（`observer.rs:71-82`）从不转发这些——TS 契约与 Rust 白名单各自维护、已不一致。
- `RunEventRecord.payload`（两侧都是 JSON 字符串）被 TS 在至少 6 处 `JSON.parse`：`features/agent/agentActivity.ts:529, 507`、`toolActivityModel.ts:39`、`approvalPayload.ts:39/67/94`、`agentMessageFormatters.ts:38`——对应 Rust 写入点 `agent_bridge/persist.rs:61/181/282`。**proto 字符串 → SQLite 字符串 → TS JSON.parse 三层手工契约**。

### S2【high】类型契约完全手工同步，无 codegen，且已出现漂移

- 无 tauri-specta / ts-rs / 任何 `*.gen.ts`；唯一 codegen 是 Rust-only protobuf（`build.rs:14-23`）。
- 手工配对 **39+ 对**（完整对照表见附录 A）。已漂移的 3 对：
  1. `ThreadRecord`（`store/threads.rs:13`）序列化 `archivedAt` / `deletedAt`，`StoredThread`（`integrations/storage/types.ts:1-15`）未声明，静默丢弃。
  2. `AgentModelOption`（`agent_bridge/models.rs:9`）三个 bool 非 Option；TS 声明为可选（`agentClient.ts:6-20`）。
  3. `AgentPromptResponse.session_id` Rust 必填（`agent_bridge/mod.rs:95`）；TS `sessionId?: string` 可选（`agentClient.ts:31-43`）。
- `invokeCommand<T>(command: string, ...)`（`invoke.ts:19-23`）命令名→返回类型无映射，泛型靠自觉；约 5 处完全无类型（`ResetPage.tsx:19`、`AppShell.tsx:131`、`OnboardingGate.tsx:192`、`agentStateCache.ts:294` 等）。

### S3【med】推送事件 payload 契约碎片化，无共享 TS 契约模块

- `thread-runtime-updated` 形状声明 **4 次**且已漂移：`useThreadStore.ts:29-35`（完整 5 字段）、`sendPipeline.ts:124-129`（内联同形）、`useRunReattach.ts:84-89`（再抄一遍）、`futureReferenceStore.ts:262`（**弱化版** `{ runId?; status? }`）。
- `approvals-updated` 两处监听两种写法：`useApprovals.ts:12-15`（有类型）vs `usePendingApprovalCounts.ts:35`（无泛型、忽略 payload）。
- 对照：invoke 侧有 `integrations/storage/types.ts` 集中定义，推送侧没有等价物。

### S4【med】违反自定规则 #3（禁裸 window CustomEvent）

- `agentStateCache.ts:275-277` `window.dispatchEvent(new CustomEvent("future:agent-event", ...))`、`:298` `future:cwd-changed`；消费端靠 cast 链：`useThreadMessages.ts:271-277` `(ev as CustomEvent).detail as {...}`；`AppShell.tsx:141` 直接监听裸 `future:cwd-changed`。
- 而 `gui/CLAUDE.md` 原则 3 明确要求一律走 `lib/futureEvents.ts` 类型化总线（该文件自己在 :300 用了 `emitFutureEvent("toast")`，说明是遗漏不是有意）。

### S5【med】`ResolvedMarkdownReference.data: Option<serde_json::Value>`（`store/records.rs:45-49`）

- 一个命令返回 5 种 record 之一的无类型联合；TS 靠手写运行时守卫兜底（`typeGuards.ts:17-56`，5 个 guard）。这是 invoke 面上唯一的"多态无类型"出口。

### S6【low】跨边界逻辑重复

- `derive_thread_title`（`remote/commands.rs:857-869`，28 字符截断规则）镜像 TS `deriveThreadTitle`（`components/layout/hooks/useNewConversation.ts:122`）——注释明示"matching the GUI new-chat draft"，同一产品规则两处实现。
- remote 桥 `list_sessions` 手工 `json!` 拼装 session 列表（`remote/commands.rs:309-317`），复刻了 threads+最新 run 状态的组合逻辑。

### S7【low】集成层纪律的少量破口

- 14 处 `invokeCommand` 绕过 `integrations/` 域客户端直接从 features/components 调用（`UpdatePage.tsx:52,82,99`、`OnboardingGate.tsx:192,282`、`AppShell.tsx:131`、`useThreadMessages.ts:291` 等）；`features/remote/remoteClient.ts` 是住在 features 里的第 6 个客户端。
- `attach_remote_stream`（`commands/threads.rs:372-375`）把单个 `run_id` 包成 ad-hoc `json!({"runId"})` 而非 struct。
- `agent_prompt` 7 个标量参数（`commands/agent.rs:26-35`），偏离自家"structured input via `{ input }`"约定（invoke.ts:14-17 注释）。

### S8【low】僵尸类型：`StoredMessage`（`types.ts:32-42`）无 Rust 对应物

- SQLite `messages` 表已删（ER.md §4.3），该接口仅在 `threadStore.ts:15` 被 re-export，属遗留。

---

## 3. 附录 A：重复类型对照（39 对，择要）

| Rust（file:line） | TS（file:line） | 状态 |
|---|---|---|
| `ThreadRecord` `store/threads.rs:13` | `StoredThread` `types.ts:1` | **漂移**（少 archivedAt/deletedAt） |
| `RunRecord` `store/runs.rs:16` | `StoredRun` `types.ts:44` | 12 字段全等 |
| `RunEventRecord` `store/runs.rs:37` | `StoredRunEvent` `types.ts:59` | 全等（payload 均为 JSON 字符串） |
| `ToolCallRecord` `store/runs.rs:48` / `ToolOutputRecord` `:62` | `types.ts:68` / `:80` | 全等 |
| `ApprovalRequestRecord` `store/approvals.rs:13` | `types.ts:88` | 21 字段全等 |
| `WorkspaceRecord` `store/workspaces.rs:12` | `types.ts:17` | 全等 |
| `ReviewChangesetRecord` / `ReviewFileChangeRecord` `store/review_snapshots.rs:14/41` | `types.ts:141/167` | 23/21 字段全等 |
| `AppSettings` `store/app_settings.rs:11` | `appSettings.ts:6` | 全等 |
| `AgentModelOption` `agent_bridge/models.rs:9` | `agentClient.ts:6` | **optionality 漂移** |
| `AgentPromptResponse` `agent_bridge/mod.rs:95` | `agentClient.ts:31` | **sessionId 必填 vs 可选** |
| `ProvidersView` 等 6 个 `agent_providers/mod.rs:50-126` | `providers.ts:7-88` | 全等 |
| login 4 个 `future_login.rs:22/35/78/93` | `providers.ts:96/115/144/151` | 全等 |
| `LastRunReviewData` `shadow_review/last_run.rs:17` | `types.ts:201` | 全等 |
| 输入类 struct（`CreateThreadInput` 等，`store/records.rs:131+`） | 各调用点内联对象 | 逐点手工对齐 |
| （无 Rust 对应） | `AgentSessionState` `agentStateCache.ts:11`、`SessionEntry` `entryProjection.ts:14` | 纯 TS 侧契约 |

---

## 4. 正面发现（非异味，供对照）

- **状态零重复**：全部 6 处浏览器存储均为纯 UI 态——composer 草稿（`composerDraft.ts:38`）、未读标记（`useUnreadThreads.ts:8`，session 内边沿检测，Rust 根本没有 unread 概念）、右栏宽、语言、上次用 model/thinking（`agentClient.ts:141,143`，仅用作新会话默认值种子）。`integrations/storage/` 是纯 invoke 客户端，**不是 localStorage 镜像**；`threadStore.ts:1-27` 只是 barrel。
- Rust 自有状态（threads/runs/approvals/reviews/workspaces/artifacts/settings）的推送事件全部类型化 struct + camelCase；错误统一 `AppError` 字符串（符合 CLAUDE.md 原则 8）。
- 能力面收敛：`capabilities/default.json` 只授予 dialog/window 权限，前端无 shell/fs 直通；asset 协议限定 `$HOME/.future/**` 等 4 个目录（`tauri.conf.json` assetProtocol.scope）。
- ER.md 的"Agent JSONL 为唯一真源、GUI 经 gRPC 增量读"在实现中成立（`runs.rs:104-152` 从 agent journal 拉事件、失败才回退 legacy JSONL）。

---

## 5. 总体结论

- **宏观纪律罕见地好**：invoke 单一入口 102/102 零违规；浏览器存储与 SQLite 零状态重叠；前端不直连 gRPC；Rust 自有领域对象的命令/事件全部类型化；能力授权最小化。`gui/CLAUDE.md` 的原则 2、6、8、10、11 在代码里都成立。
- **真正的边界问题集中在 agent 透传域**：凡是数据源头是 agent gRPC 的（session entries、session state、run event payload、agent-event 推送），Rust 层退化为无类型管道（`serde_json::Value`），把 proto 的字符串语义原样泄漏给 React，形成"proto 字符串 → SQLite 字符串 → TS JSON.parse/typeof 收窄"的三层手工契约，且已出现死分支（TS 处理 Rust 从不转发的事件类型）与混合命名（同一 payload 里 `session_name` 与 `sessionId` 并存）。
- **最强证据链**：
  1. `commands/threads.rs:235,302` 两个高频命令返回裸 `Value` + `agentStateCache.ts:92-103` 手工收窄；
  2. 39 对手工类型已出现 3 处漂移（`threads.rs:13` vs `types.ts:1` 等）；
  3. `thread-runtime-updated` 的 TS 形状声明 4 次且 `futureReferenceStore.ts:262` 已弱化；
  4. `agentStateCache.ts:275-298` 裸 window CustomEvent 违反自家原则 3。

**一句话：边界划分（谁拥有什么）是对的，缺的是契约机制（类型如何不漂移）。** 引入 tauri-specta 类 codegen、或至少把 agent 域的 `Value` 出口收拢成 Rust 侧结构化 struct + TS 侧单一事件契约模块，即可消掉全部 high 级问题。

---

## 6. 修复方向建议（按优先级）

1. **收拢 agent 域的 `Value` 出口**（对应 S1）：在 Rust 侧为 `get_session_entries` / `get_thread_agent_state` 定义结构化 struct（`SessionEntry`、`AgentSessionState`），不再向 TS 透传裸 `Value`。这一步同时为报告 01 的"影子 JSON"问题在 GUI 侧建立防线。
2. **建立 TS 侧单一事件契约模块**（对应 S3）：把 `thread-runtime-updated` / `approvals-updated` 等 8 个事件的 payload 类型集中到 `integrations/tauri/events.ts`，消灭 4 处重复声明与弱化版。
3. **引入类型 codegen 或契约测试**（对应 S2）：tauri-specta / ts-rs 任选其一；短期可先写序列化快照测试，至少让漂移在 CI 里可见。优先覆盖已漂移的 3 对。
4. **消灭裸 CustomEvent**（对应 S4）：`agentStateCache.ts` 改走 `lib/futureEvents.ts` 类型化总线。
5. **给 invokeCommand 建立命令名→返回类型映射**（对应 S2/S7）：消除 5 处无类型调用与 14 处绕过域客户端的直调。
6. **清理遗留**（对应 S8/S6）：删除僵尸类型 `StoredMessage`；标题推导规则二选一（建议保留 TS 侧，remote 桥复用同一实现的文档约定）。
