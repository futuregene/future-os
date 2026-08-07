# 渲染审计报告 04：React 层渲染性能（流式热路径）

> 范围：`desktop/src/` React 前端，重点是流式 agent 响应的热路径（后端 40ms 合并推送，流式期约 **25 次/秒**）。
> 方式：只读审计，未修改任何文件。行号基于审计时的仓库状态。
> 一句话结论：架构上已做了大量正确的性能投资（增量投影、窗口化、缓存），但 **4 个 HIGH 级问题让这些投资大部分失效**——核心是"每次推送全应用树重渲染 + 全代码库唯一的 memo 边界被击穿 + 流式尾部 markdown 全量重解析（累计 O(n²)）"。

---

## 1. 状态管理架构概要（与渲染相关）

- **无第三方状态库**（无 zustand/redux/jotai），React 19。状态分两层：
  - **AppShell 域 hooks**（`useThreadStore` / `useAgentConnection` / `useAppSettings` 等）用普通 `useState` 拥有全局状态，经 **props 逐层下传**。`threadRunStatuses`（每线程运行状态）住在 AppShell 里，因此**每次流式推送都会重渲染整棵应用树**。
  - **线程消息状态**在按 `thread.id` 加 key 的 `AgentThread` 实例内（`useThreadMessages` 的 `useState<AgentMessage[]>`）。流式更新来自 Tauri `thread-runtime-updated` 推送（后端 `desktop/src-tauri/src/lib.rs:286-330` 做 **40ms 合并** → 流式期间约 **25 次/秒**），每次推送 → 增量投影（`listRunEventsSince`）→ `setMessages`。
- 横切缓存是**模块级外部 store**（`agentStateCache`、`futureReferenceStore`、shiki store、`useNow` ticker、markdown 解析 LRU、投影器 LRU），经 `useSyncExternalStore` 消费，subscribe/snapshot 都做了稳定化——这部分总体做得好。
- 全代码库 **`React.memo` 只用了 1 处**（`MessageBlock`，MessageBlock.tsx:46，grep 确认），其余组件（MessageList/Composer/ActivityRail/ThreadListItem/MarkdownContent…）全部无 memo。

---

## 2. 热路径单次推送（≈每 40ms）的实际代价链

```
thread-runtime-updated 到达
 ├─ useThreadStore reduce 必返新对象 → AppShell 全树渲染 [H2]
 │    └─ ActivityRail O(W×T) 推导 + 全部侧栏行空转 [M3]
 └─ 同事件 → reattach/pipeline tick → 增量投影 IPC → setMessages → AgentThread 渲染
      ├─ handleFork 新引用 → 全部可见行重渲染 [H1]
      │    └─ 流式行 MarkdownContent 全量 remark 重解析 [H3]
      ├─ Composer(+745行 MentionEditor) 子树空跑 [H4]
      └─ layout effect → setScrollbar 新对象 → 第二次提交 [M2]
```

---

## 3. 发现（按严重度排序）

### HIGH

**H1 — `handleFork` 依赖 `messages`，每个流式推送都击穿全列表唯一 memo** —— ✅ **已修复（commit `306cf05f`）**：`AgentThread.tsx` 现用 `messagesRef` 镜像消息列表，`handleFork`/`handleRetryRun` 不再依赖 `messages` 数组本身（代码注释直接引用 H1）。下述为修复前证据。
- 证据：`AgentThread.tsx:195-221`（deps 在 221 行：`[thread, messages, onForked, t]`）→ 经 `MessageList.tsx:91`（`onFork={onFork}`）传给每个 `MessageBlock`（`MessageList.tsx:76-97`）。
- 机制：`messages` 数组每个推送（≈25/s）都换新引用 → `handleFork` 恒为新函数 → `MessageBlock` 的 `memo` 浅比较对**所有行**失败 → 窗口内全部已定稿消息（最多 10 轮对话）每推送整体重渲染，包括每行重跑 `MarkdownContent`（解析有缓存，但整棵元素树每推送重建+diff 一遍）。这是热路径上最大的 React 开销，也是唯一 memo 边界被废的地方。
- 修复方向：`handleFork` 用 `messagesRef` 读消息（或改为接收 messageId 的稳定回调），deps 去掉 `messages`。

**H2 — `threadRunStatuses` reducer 流式期间从不 bail-out → AppShell 全树 25Hz 重渲染** —— ✅ **已修复（commit `306cf05f`）**：`useThreadStore.ts` 的 reducer 现增加语义不变即返回 `previous` 的 bail-out（status/runId/endedAt 未变时跳过，代码注释直接引用 H2）。下述为修复前证据。
- 证据：`useThreadStore.ts:42-60`（`reduceThreadRunStatus` 仅在 `revision` 未变时返回旧值；流式中 status 恒为 `"running"` 但 revision 递增 → 每次推送返回新对象）；监听在 `useThreadStore.ts:256-267`；AppShell 消费整个 store（`AppShell.tsx:111-123`）并下传。
- 机制：每个合并推送（≈40ms）→ 新 `threadRunStatuses` → **AppShell 重渲染** → ActivityRail（排序+多轮过滤+分组，见 M3）、所有 ThreadListItem、ContextPanel、AgentThread（再次）、各 dialog 全部重跑，显示状态其实没变。
- 修复方向：reducer 在 `{status, runId, endedAt}` 语义未变时返回 `previous`；或把 run-status 拆出 AppShell（只让侧栏订阅）。

**H3 — 流式尾部 markdown 每推送全量重解析（O(n²) 累计）**
- 证据：`MarkdownContent.tsx:35`（`useMemo(() => parseFutureMarkdown(content), [content])`），解析器 `parseFutureMarkdown.ts:58-78`（unified/remark-gfm 全量 parse）；缓存 `parseCache` 只命中**相同**字符串。
- 机制：流式中 `segment.text` 每推送增长 → `content` 恒变 → memo 恒 miss → **对整段已累计文本**做全量 remark parse + 全树 `document.nodes.map(renderBlock)` 重建（另见 L4 的索引 key）。回复越长每推送越贵，整条回复累计 O(n²)。`live` 标志只豁免了代码高亮，没豁免解析。
- 修复方向：对 live 尾部做增量/分块解析（只重解析最后一个 block），或对渲染更新节流（把 25Hz 的 state 写入合并到 ~8-10Hz）。

**H4 — Composer(+745 行 MentionEditor)每推送重渲染** —— ✅ **已修复（commit `306cf05f`）**：`Composer` 现为 `memo(ComposerImpl)`，且 `onAbort`/`onSend` 由 AgentThread 以 useCallback 稳定传入（`Composer.tsx:712` 注释：「with stable props … it must not re-render at all」）。下述为修复前证据。
- 证据：`AgentThread.tsx:317-333`（`Composer` 非 memo，且 `onAbort={() => void handleAbort()}`、`onSend={payload => void handleSend(payload)}` 内联箭头每渲染恒新）；`Composer.tsx:98` 起整个组件体；`MentionEditor.tsx` 745 行。
- 机制：AgentThread 因 `messages` 变化 + AppShell 推送两条路径每推送重渲染 → Composer 子树（附件列表、三个 SelectMenu、MentionEditor 全部 hooks/JSX）每推送空跑一遍。
- 修复方向：`memo(Composer)` + 稳定化 `onAbort`/`onSend`（useCallback）；或把 `isSending` 下沉。

### MED

**M1 — `MarkdownContent` 未 memo → 流式消息内已定稿 segment 每推送重渲染**
- 证据：`MarkdownContent.tsx:34-46`（无 memo）；`MessageBlock.tsx:144-178` 每渲染 map 所有 segments。
- 机制：流式行每推送重渲染时，该行内已定稿 segment 的 `MarkdownContent` props（content/live）未变但仍重跑 render：解析命中缓存，元素树照建照 diff。
- 修复方向：`memo(MarkdownContent)`。

**M2 — `updateFloatingScrollbar` 每次 set 新对象 → 每推送额外一次提交**
- 证据：`useFloatingScrollbar.ts:41-54`（无条件 `setScrollbar({height, top, visible})`，字面量恒为新引用）；调用链 `AgentThread.tsx:111-116`（`onContentSettled: () => updateFloatingScrollbar(false)`）← `useStickyAutoScroll.ts:68-84`（layout effect 以 `contentKey=messages` 为依赖，每推送触发）。
- 机制：每个推送在 messages 提交后，layout effect 再触发一次 `setScrollbar`（值常相同但引用新）→ AgentThread 子树**二次提交**（2 次 commit/推送）。
- 修复方向：set 前比较三值，相等则跳过。

**M3 — 侧栏每次 AppShell 渲染做 O(W×T) 推导；ThreadListItem 无 memo 且收内联回调**
- 证据：`ActivityRail.tsx:204-264`（`sortThreads(threads.filter(...))` + pinned/chat/workspace 三轮 filter + `workspaceGroups` 嵌套 filter）+ `344-364/448-469/511-532`（每行 `onMenuOpenChange={open => ...}`、`onSelectThread={selectionMode ? (...) : onSelectThread}` 内联）；`ThreadListItem.tsx:14` 非 memo，内部还各自 `useCachedAgentState`（`ThreadListItem.tsx:68`）。
- 机制：由 H2，这些推导与全部行渲染以 ≈25Hz 空转。
- 修复方向：`useMemo` 化 visibleThreads/groups；`memo(ThreadListItem)` + 稳定回调（把 thread 作为参数）。

**M4 — 流式气泡更新把消息搬到数组末尾 + updater 内每推送 O(n) 分配**
- 证据：`threadRunProjection.ts:231`（`[...base.filter((_, i) => i !== existingIndex), updated]`）与 `:321`（同型）；`streamingBubbleBase` 在每推送的 updater 内两次 `current.map(m => m.role).lastIndexOf(...)`（`:120`、`:132`）。
- 机制：每推送 filter+append 重排位置（气泡若非末尾会触发 DOM 移动），并分配两个临时 role 数组。
- 修复方向：用 `.map` 原位替换；lastIndexOf 改反向 for 循环。

**M5 — `entryProjection` 用不稳定 id → 每次重载/结算整列表 remount**
- 证据：`entryProjection.ts:120-123`（`segId()` = `ep_${Date.now()}_${++_seq}`，消息与 segment 全部用它）；结算路径 `useRunReattach.ts:95-98`（终态推送 → `reloadMessagesQuiet(force)` 全量替换）。
- 机制：运行结算或切线程时所有消息/segment key 全变 → React 无法复用，整窗口 remount（解析/高亮缓存兜底了最贵的部分，但 DOM 全量重建）。
- 修复方向：id 从 entry/run 派生（agent JSONL entry 有 `id` 字段）。

**M6 — `recover-run` 监听每推送重订阅**
- 证据：`AgentThread.tsx:174-193`（`handleRetryRun` deps 含 `messages`（:185），effect deps `[handleContinueRun, handleRetryRun]`（:193））。
- 机制：`messages` 每推送变 → effect 每推送 remove/addEventListener。
- 修复方向：`handleRetryRun` 用 ref 读 messages。

### LOW

**L1 — 每 token 的 window CustomEvent 中转无人消费**
- 证据：`agentStateCache.ts:263-278`（每个 `text_chunk`/`thinking_delta`/`tool_delta` 都 `window.dispatchEvent`）；唯一消费者 `useThreadMessages.ts:271-303` 只处理 `user_message`/`agent_start`/`agent_end`，其余提前 return。
- 修复方向：在分发端过滤掉无消费者的事件类型。

**L2 — `useFutureReferences` effect 每推送重跑**（已有防护，开销小）
- 证据：`MarkdownContent.tsx:36`（deps `[references, workspaceId]`，references 随每次解析恒新）；防护在 `futureReferenceStore.ts:88-100`（已解析记录跳过，注释明确承认此 churn）。

**L3 — `useAgentConnection` 每 10s 轮询必 set 新对象**
- 证据：`useAgentConnection.ts`（`setAgentConnection({checkedAt: Date.now(), ...})`，成功/失败分支均含 `checkedAt: Date.now()`）→ 即使结果未变，AppShell 每 10s 重渲染一次。
- 修复方向：结果等价时不 set。

**L4 — markdown 渲染用位置索引 key**
- 证据：`MarkdownContent.tsx:40`（`b${index}`）与 `withStableKeys`（`:209-215`）；CodeBlock 高亮行同样用 index key（有 eslint-disable 注释，`CodeBlock.tsx:61-69`）。追加式增长下可接受，但流式中 block 结构抖动（段落合并/拆分）时会错配子树。

**L5 — render 中构造 `Intl.NumberFormat`**：`MessageMeta.tsx:38`（流式行每推送+每秒一次）、`MessageBlock.tsx:323-327`。可提为模块级缓存。

**L6 — `AgentThread` 渲染内 O(n) 扫描**：`AgentThread.tsx:148-150`（`messages.some(...)` 求 `isSending`），每推送两次渲染各一遍；量小。

**L7 — 加载路径 O(runs×messages)**：`applyRunMetadata` / `recoverFailedRuns`（`threadRunProjection.ts:398-479, 634-677`）对每个 run 做 `runMatchesAnyTurn` 全消息扫描 + `Date.parse`。仅切线程/结算时发生，非逐 token。

---

## 4. 做得好的地方（平衡项）

- **增量运行投影器**：`createRunProjector`（`agentActivity.ts:76-295`）+ `listRunEventsSince` 取代了整日志重投影（注释明示避免了 O(n²)），LRU 上限 8（`threadRunProjection.ts:38-40`）。
- **消息窗口化**：`useMessagePaging` 只渲染最近 10 轮对话（等价虚拟化的分页方案），带滚动锚定（`useMessagePaging.ts`）。
- **`MessageBlock` memo + `patchMessage` 保持未变行引用稳定**（`threadRunProjection.ts:11-23`）——设计正确，只是被 H1 的 `onFork` 废掉。
- **`LiveMarkdownContext` 让增长中的代码块跳过 shiki 高亮**（`CodeBlock.tsx:21-25`、`LiveMarkdownContext.ts`），配合 LRU 高亮缓存 + 按需 grammar 加载（`useCodeHighlighter.ts:165-167`）。
- **跨实例 markdown 解析 LRU 缓存**（`parseFutureMarkdown.ts:48-52`）。
- **`useNow` 全局单一 1s ticker + 按订阅者分桶**（`useNow.ts`），避免每消息一个 interval。
- **外部 store 规范**：subscribe 模块级稳定、snapshot 引用稳定（`agentStateCache.ts:62-67, 222-224`），`useSyncExternalStore` 用法正确。
- **`MessageList.tsx:67-72` 显式避免了 O(n²)** 的 previous-user 计算（有注释）。
- **`AppShell.tsx:264-270` `userWorkspaces` useMemo** 并有注释解释稳定引用原因；多处 generation-counter 防陈旧写入（`useThreadStore.ts`、`useThreadMessages.ts`、`useAgentConnection.ts`）。
- **后端 40ms 推送合并**（`lib.rs:246-330`）——问题在于前端对每个合并推送仍做全树渲染（H1/H2）。

---

## 5. 修复优先级建议

| 优先级 | 项 | 改动量 | 预期收益 |
|---|---|---|---|
| 1 | H1：`handleFork` 去掉 `messages` 依赖（用 ref） | ~5 行 | 恢复唯一 memo 边界，已定稿消息不再每推送重渲染 —— ✅ **已实施（commit `306cf05f`）** |
| 2 | H2：reducer bail-out（语义未变返回旧引用） | ~10 行 | AppShell 全树不再 25Hz 重渲染（连带消除 M3 空转）—— ✅ **已实施（commit `306cf05f`）** |
| 3 | H3：live 尾部 markdown 增量解析或渲染节流 | 中等 | 消除累计 O(n²)，长回复流式显著变顺 —— ⚠️ 截至 2026-08-06 **未修复** |
| 4 | H4 + M1：`memo(Composer)`、`memo(MarkdownContent)` + 稳定化回调 | 小 | Composer/MentionEditor/已定稿 segment 不再空跑 —— H4 部分 ✅ **已实施（commit `306cf05f`，Composer 已 memo）**；M1 未修复 |
| 5 | M2：scrollbar set 前比较 | ~3 行 | 消除每推送二次提交 |
| 6 | M5：`entryProjection` 稳定 id | 小-中 | 结算/切线程不再整窗口 remount |
| 7 | M4/M6/L1-L7 | 小 | 局部优化 |

**关键洞察**：1、2 两项是"一行级改动、全局级收益"——它们让代码库里已经存在的正确投资（MessageBlock memo、patchMessage 引用稳定、窗口化、各级缓存）真正生效。⚠️ 这两项已在 commit `306cf05f` 实施（代码注释引用 H1/H2）；建议用 React Profiler 验证收益，并评估 H3 方案（增量解析 vs 节流）。

---

## 6. 验证方法建议

- React DevTools Profiler 开启 "Record why each component rendered"：修复前后各录一次流式输出，对比 commit 频率与每次 commit 的组件数。
- 预期修复后：流式期每次 commit 只包含 AgentThread 子树内变化的部分；AppShell/ActivityRail/Composer 不再出现在 commit 里。
- H3 可用 Performance 面板观察 `parseFutureMarkdown` 在长回复后期的占用是否随长度线性增长（修复前）vs 恒定（修复后）。
