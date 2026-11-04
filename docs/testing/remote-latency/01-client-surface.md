# 01 客户端命令面：远程桌面真正被发出的每一条命令

> 审计对象：`agent.db` 2.4 GB / 706 sessions、`app.db` 54 MB / 679 threads 的真机上，手机（`mobile/`）与「桌面作为客户端」（`desktop/` + `host/src/remote_peer/`）实际发出的远程命令。
> 本文只回答「谁在什么时候、用什么参数、发了什么」，不测耗时；耗时留给 02（服务端读路径）与 03（实测）。
> 所有结论都带 `file:line`；未标注的地方即是本文没有核实的地方。

---

## 0 结论先行

1. **决定「打开会话有多慢」的命令只有 4 类**，其余命令都挂在用户动作或低频定时器上：
   `get_state`、`get_session_entries`（+ 尾部回填页）、`get_events_since`（+ 快照/尾页）、`get_read_chunk`（分块补读）。见 §2.1。
2. **一次打开不是一次往返，而是一串**：手机最小 1×`get_state` + 1×`get_session_entries` + 1×`get_read_chunk`（超过 192 KB 就必须补读）+ 有活跃 run 时的 1×`get_events_since`；`get_session_entries` 被 `trimmed` 时还要串行回填 ≥1 页。倍数表见 §3.2。
3. **真的有轮询，而且周期各不相同**：手机 `client_presence` 15 s、`get_queue` 2 s（仅队列面板）、`list_sessions` 10 min；桌面客户端 `remote_peer_list` 5 s，并且**每 5 s 对每台已连接主机拉一次 `list_sessions` + `list_workspaces`**（60 s 那档只是旧主机兜底）。桌面端同时还在收推送快照，两条路是重复的。见 §4。
4. **手机端命令集与服务端 `PHONE_COMMANDS` 已经不同步**：6 条手机真的在发的命令不在那张测试镜像表里（§1.2）。`route()` 表本身是完整的——脚本比对显示**没有任何客户端发出了一条未被路由的命令**，所以本轮的性能问题不会以「Unsupported command」的形式出现。

---

## 1 权威清单：谁是真相

### 1.1 四个候选来源

| 来源 | 位置 | 内容 | 是否权威 |
|---|---|---|---|
| **路由表 `route()`** | `host/src/remote_host/business/mod.rs:43-105` | 68 个命令名 → 8 个处理模块（Catalog/History/Transfers/Prompt/Compaction/Settings/Tasks/Providers） | **是**。生产路径唯一的分发入口：`host/src/remote_host/mod.rs:36-46` 先特殊处理 `get_read_chunk`，其余交给 `business::execute`，`route()` 返回 `None` 即回 `Unsupported command` |
| **桥层命令**（不进 `route`） | `packages/remote-server/src/commands.rs:907-1066`；`secure_ready` 的激活在 `:488`，`get_read_chunk` 的单飞旁路在 `:695` | 11 条：`secure_open` / `secure_finish`（握手负载，`:915-920`）、`pair_handshake`、`pair_handshake_confirm`、`takeover`、`get_presence`、`client_presence`、`client_disconnect`、`unpair`、`secure_ready`、`get_read_chunk` | **是**，且**必须与 `route()` 合起来读**：这些命令永远到不了业务处理器 |
| `PHONE_COMMANDS` | `host/src/remote_host/business/mod.rs:254-315` | 60 条「手机发的命令」镜像 | **否**，是测试夹具（`every_command_the_phone_sends_is_routed`，`:317-326`）。它比真实客户端**滞后**（§1.2） |
| `command.schema.json` | `packages/remote-protocol/command.schema.json` | 命令**形状**：`type` 声明为自由字符串（**无 enum**），逐字段 `x-rust-name` / `x-rust-type` / 默认值，`x-version: 1` | **只对字段形状权威，对命令名不权威**。把它当命令清单会漏掉全部 68+11 条名字 |
| 客户端源码 | `mobile/src/remote/`、`packages/remote-client/src/`、`desktop/src/features/remote-peer/` | 真正发出的命令与实填参数 | **事实清单**（本文 §2 的主体） |

> 计划修订：任务书里点名的 `host/src/remote/commands.rs`（5413 行）**不含生产代码**——它每个 `use` 都挂着 `#[cfg(test)]`（`:1-60`），`host/src/remote/mod.rs:7` 虽 `pub(crate) mod commands;` 但整个文件在非 test 构建里编译为空。桥层生产实现在 `packages/remote-server/src/commands.rs` + `host/src/remote_host/`。
> 另：`mobile/src/remote/useRemoteConnection.ts` 与 `mobile/src/remote/syncEngine.ts` **不存在**，实际是 `useDesktopConnection.ts` 与 `packages/remote-client/src/syncEngine.ts`。

### 1.2 名单差异（实测，非推测）

`route()` − `PHONE_COMMANDS` = 8 条，`PHONE_COMMANDS` − `route()` = 0 条：

| 命令 | 在 `route()` | 在 `PHONE_COMMANDS` | 手机真的在发？ |
|---|---|---|---|
| `get_queue` | ✅ Prompt 组 `:51` | ❌ | ✅ `mobile/src/remote/sessionQueue.ts:126` |
| `cancel_queued_run` | ✅ Prompt 组 `:51` | ❌ | ✅ `sessionQueue.ts:166` |
| `release_queued_run` | ✅ Prompt 组 `:51` | ❌ | ✅ `sessionQueue.ts:178` |
| `get_fusion` | ✅ `:46` | ❌ | ✅ `mobile/src/remote/useConversationController.ts:291` |
| `get_event_content_range` | ✅ `:46` | ❌ | ✅ `packages/remote-client/src/replay.ts:69` |
| `rename_workspace` | ✅ `:80` | ❌ | ✅ `mobile/src/remote/useSessionCatalog.ts:811` |
| `get_messages` | ✅ `:46` | ❌ | ❌ 无人发送（仅 `desktop/src/features/remote-peer/remotePeerClient.ts:1381` 的 chunked-read 名字白名单里出现） |
| `get_available_models` | ✅ Settings 组 `:67` | ❌ | ❌ 无人发送 |

结论：`PHONE_COMMANDS` 漏登记 6 条手机正在发的命令，测试因此对它们是盲的（其中 `get_queue` 还是 2 s 轮询，见 §4）。
另有 7 条只存在于服务端、无任何客户端发送：`get_available_models`、`get_messages`、`apply_task_revision`、`list_task_revisions`、`list_task_deps`/`set_task_dep`/`remove_task_dep`（后三条为过渡期保留，见 `business/mod.rs:93-97` 注释）。

---

## 2 客户端真正发送的命令（一命令一行）

列说明：**关键路径**＝打开一个会话、用户等到第一屏内容之前必须返回的读。

### 2.1 打开会话的关键路径（4 类）

| 命令 | 手机 | 桌面 | 手机调用点 | 实填参数（值来源） | 触发 | 关键路径 |
|---|---|---|---|---|---|---|
| `get_state` | ✅ | ✅ | `mobile/src/remote/useConversationController.ts:182`（无 engine 时的兜底）、`mobile/src/remote/useTimelineController.ts:570`（engine 依赖）、`mobile/src/remote/timeline/useCompactionOutcomes.ts:124` | `{ sessionId }`，lane = sessionId。**仅 sessionId，无其它字段** | 打开会话 / 重连恢复 / 余额面板 `:150` / compaction 等待中 5 s 轮询 | **是**（与历史页并发发出，`packages/remote-client/src/syncEngine.ts:608-611`） |
| `get_session_entries` | ✅ | ✅ | `mobile/src/remote/timeline/historyPaging.ts:26`、`mobile/src/remote/timeline/useOlderHistory.ts:86`；桌面 `desktop/src/features/remote-peer/useRemoteTimeline.ts:447` | `{ sessionId, before, limit: 3, chunkedRead: true }`。`limit` = `HISTORY_PAGE_USER_EXCHANGES = 3`（`packages/thread-projection/src/historyPage.ts:29`，按**用户轮次**计，不是消息条数）；`before` 首次 = `HISTORY_TAIL_CURSOR = Number.MAX_SAFE_INTEGER`（`historyPage.ts:48`），向上翻页 = 上一页的 `nextOffset`；`chunkedRead` 由 `packages/remote-client/src/readPages.ts:57` 统一加；回填页额外带 `untrimmed: true`（`historyPaging.ts:32`） | 打开会话 / 上滑翻页 / 重连 reconcile | **是** |
| `get_read_chunk` | ✅ | ❌（Rust 侧自己重组） | `packages/remote-client/src/readPages.ts:85` | `{ sessionId, replyId, offset, runId?, bridgeInstanceId? }`。`replyId` = 首个回复 `readChunk.id`；`offset` 从 0 起按 `CHUNK_BYTES = 192 KB` 递增，**并发 4**（`readPages.ts:5-7`）；单页上限 `MAX_READ_BYTES = 16 MB`（`:4`） | 任一分块读的回复超过 192 KB 即自动补读 | **是**（超过一页就必须先补完才解码） |
| `get_events_since` | ✅ | ✅ | `packages/thread-projection/src/replay.ts:298-311`（手机经 `packages/remote-client/src/replay.ts:34-53`，桌面经 `desktop/src/features/remote-peer/remoteRunReplay.ts:43-63`） | `{ sessionId, runId, sinceIdx, limit: 1000, offset, preferSnapshot?, replayUntilIdx?, chunkedRead: true }`。`limit` = `REPLAY_PAGE_EVENTS = 1000`（`thread-projection/src/replay.ts:170`）；`sinceIdx = -1` 冷读整段，否则为本端高水位；`preferSnapshot` **只在首页**（`sinceIdx=-1 && cursor=-1 && offset=0 && 无 watermark`，`:305-308`）；`replayUntilIdx` 从首页 watermark 起**每页都带**（`:311`） | **仅当 `state.activeRun` 存在**（`syncEngine.ts:618-620`、`useRemoteTimeline.ts:562`） | **是**（有活跃 run 时） |

补充事实（对后续优化直接有用）：
- 打开时会**并发**发出 `get_state` 与第一页历史：`syncEngine.ts:596-615` 明确写了「两个请求同时在路上，因为这是读者全部的等待」。
- 手机 `get_session_entries` 有一个**串行回填循环**：尾页被桥层 `trimmed` 且用户轮次不足 3 时，继续用 `before = nextOffset` 读，直到满 3 轮或 `hasMore=false`（`mobile/src/remote/useTimelineController.ts:260-284`）。真机上「3 轮次 / 页」被字节预算裁到 1 轮次时，一次打开就变成 2-3 次往返。
- 首屏不等 replay：历史到就上屏，replay 作为 preview 后补（`syncEngine.ts:648-660`），但 live 事件要等 replay 完成才应用。
- 手机超时：单次请求 10 s（`packages/remote-client/src/client.ts:1408`），`get_`/`list_` 前缀重试 3 次（`:1471-1479`），即最坏 ~30 s；历史页另有 30 s 的事务级超时（`historyPaging.ts:11`），界面 15 s / 45 s 双层判定（`useTimelineController.ts:31-32`）。

### 2.2 会话内按需读（不在打开路径）

| 命令 | 手机 | 桌面 | 调用点 | 实填参数 | 触发 |
|---|---|---|---|---|---|
| `get_fusion` | ✅ | ✅ | `useConversationController.ts:291`；`desktop/src/features/remote-peer/remoteToolTarget.ts:54` | `{ sessionId, runId, toolCallId, entryId, offset }`，`entryId` = fusion stepId、`offset` 来自打开的步骤 | 点开 fusion 步骤 |
| `get_tool_call_args` | ✅ | ✅ | `useConversationController.ts:302`；`remoteToolTarget.ts:63` | `{ sessionId, runId, toolCallId }` | 点开一条工具行（结果按 `session\0run\0call` 缓存，`useConversationController.ts:283-306`） |
| `get_event_content_range` | ✅ | ✅ | `packages/remote-client/src/replay.ts:69`；`useRemoteTimeline.ts:747`、`remoteRunReplay.ts:65` | `{ sessionId, runId, contentRequest, chunkedRead }` | 事件 `data` 含 `content_refs` 或 > 32 KB 且缺必需内容时补读（`replay.ts:60-66`） |
| `list_session_files` | ✅ | ✅ | `useConversationController.ts:318`；`desktop/src-tauri/src/commands/remote_peer.rs:225-236` | `{ sessionId, filePath, chunkedRead }`，`filePath` 空串=会话根目录 | 打开文件面板 / 进目录 |
| `get_queue` | ✅ | ✅ | `mobile/src/remote/sessionQueue.ts:126`；`desktop/src/features/remote-peer/remoteQueue.ts:172` | `{ sessionId }` | 队列面板；**有排队行时 2 s 轮询**（§4） |
| `get_prompt_receipt` | ✅ | ✅ | `mobile/src/remote/usePromptOutbox.ts:118`；`remotePeerClient.ts:519` | `{ promptId }`，lane = `list` | 发送前查重 / 恢复未确认的发送 |
| `cancel_queued_run` / `release_queued_run` | ✅ | ✅ | `sessionQueue.ts:166`、`:178`；`remoteQueue.ts:209`、`:223` | `{ runId, sessionId }` | 用户取消 / 立即执行 |

### 2.3 目录、设置、任务（低频，但基数大）

| 命令 | 手机 | 桌面 | 手机调用点 | 实填参数 | 触发 |
|---|---|---|---|---|---|
| `list_sessions` | ✅ | ✅ | `mobile/src/remote/useSessionCatalog.ts:333`；桌面 `remotePeerClient.ts:333-340` → `desktop/src-tauri/src/commands/remote_peer.rs:131-133` → `host/src/remote_peer/runtime.rs:206-208` → `packages/remote-client/src/runtime.rs:2941-2945` | `{}`（无参数、无分页、无 limit，全量返回） | 见 §4：手机 10 min 自愈 + presence 版本前进 + 事件；桌面 **每 5 s / 每台**（`useRemotePeers.ts:236`），另有 60 s 兜底（`:356`） |
| `list_workspaces` | ✅ | ✅ | `useSessionCatalog.ts:479`、`RemoteContext.tsx:1027`；桌面 `useRemotePeers.ts:246` → `commands/remote_peer.rs:136-138` → `packages/remote-client/src/runtime.rs:2947-2952` | `{}` | 同 `list_sessions`（桌面侧只在 5 s 的 `refresh` 里读，60 s 兜底不读） |
| `list_models` | ✅ | ✅ | `useSessionCatalog.ts:410`（`MODEL_RECOVERY_DELAYS_MS = [1s,5s,15s,30s]`，`:31`）；桌面 `remoteHostCatalog.ts:59` | `{}` | 重连恢复（`useDesktopConnection.ts:375`）、`provider_config_changed`/`model_visibility_changed` 事件（`useTimelineController.ts:145`）、设置页 `list_settings_models`（`useDesktopManagement.ts:42`） |
| `list_skills` | ✅ | ✅ | `useConversationController.ts:256`、`useDesktopManagement.ts:43`；桌面 `remotePeerClient.ts:1071` | `{}` | `/` 菜单、技能页 |
| `list_available_skills` | ✅ | ✅ | `useConversationController.ts:42`（`WeakMap` 每连接一次）；`remotePeerClient.ts:1076` | `{}` | 同上（中文名回退） |
| `get_settings` | ✅ | ✅ | `useSessionCatalog.ts:449`；`remotePeerClient.ts:1130` | `{}` | 重连恢复、`app_settings_changed` 事件 |
| `get_desktop_settings` / `get_fusion_settings` | ✅ | ✅ | `useDesktopManagement.ts:35`、`:37`；`remotePeerClient.ts:931`、`useRemoteFusionAvailability.ts:12` | `{}` | 设置页；桌面端另在每次连接后预热一次（`remoteHostCatalog.ts:64-66`） |
| `list_tasks` / `get_task` / `list_task_runs(limit=20)` / `create/update/delete_task` / `set_task_enabled` / `run_task` | ✅ | ✅ | `useDesktopManagement.ts:73-82`、`:84`；`remoteTaskBackend.ts:142` | 见调用点，`listTaskRuns` 默认 `limit: 20`（`useDesktopManagement.ts:84`） | 任务页 |
| `list_providers` / `update_builtin_provider` / `upsert_custom_provider` / `delete_custom_provider` | ✅ | ✅ | `useDesktopManagement.ts:66-71`；`remotePeerClient.ts:1317-1365` | 见调用点 | Provider 页 |
| `suggest_skill` / `skill_reco_today` / `record_skill_reco` / `install_skill` / `uninstall_skill` | ✅ | ✅ | `useDesktopManagement.ts:54-63`；`remotePeerClient.ts:1090-1106` | `suggest_skill` 还带 `candidates[]` | 技能推荐 / 安装 |

### 2.4 写命令（都在用户动作上，单次）

| 命令 | 手机 | 桌面 | 手机调用点 | 实填参数 |
|---|---|---|---|---|
| `prompt` | ✅ | ✅ | `usePromptOutbox.ts:124-171`；`remotePeerClient.ts:481` | `{ id, bridgeInstanceId?, sessionId, message, modelId, providerId, level, attachments?[{uploadId}], busyPolicy?, mode?, workspaceId? }`；`id` 是幂等键，重试复用（`usePromptOutbox.ts:115-123` 先查 `get_prompt_receipt`） |
| `continue_run` | ✅ | ✅ | `usePromptOutbox.ts:173-199`；`remotePeerClient.ts:993` | `{ id, sessionId, runId }` |
| `abort` | ✅ | ✅ | `useConversationController.ts:399`；`remotePeerClient.ts:410` | `{ sessionId }` |
| `approval_decision` | ✅ | ✅ | `useConversationController.ts:520`；`useRemoteApprovals.ts:114` | `{ sessionId, entryId, mode }` |
| `compact_context` | ✅ | ✅ | `useConversationController.ts:384`；`remotePeerClient.ts:583` | `{ sessionId }` |
| `set_model` / `set_thinking_level` / `set_approval_tier` | ✅ | ✅ | `useConversationController.ts:412`、`:433`、`:446`；`remotePeerClient.ts:954`、`:969`、`:1154` | `{ sessionId, modelId, providerId }` / `{ sessionId, level }` / `{ tier }`（lane=`list`） |
| `fork_session` | ✅ | ✅ | `useConversationController.ts:487-495`（超时 60 s）；`remotePeerClient.ts:555` | `{ id, sessionId, sourceEntryId }`，`id` = `mobile-fork:<random>`，同一「父+源条目」复用直到成功（`:483-487`） |
| `set_session_name` / `set_session_pinned` / `delete_session` | ✅ | ✅ | `useSessionCatalog.ts:637`、`:732`、`:667`；`RemoteContext.tsx:986/1001/1005` | `{ sessionId, name }` / `{ sessionId, threadId, pinned }` / `{ sessionId, threadId }`（`threadId` 来自 presence 行） |
| `generate_session_title` | ✅ | ✅ | `useSessionCatalog.ts:624`（超时 **65 s**）；`remotePeerClient.ts:1300` | `{ sessionId, mode: <界面语言> }` |
| `create_workspace` / `rename_workspace` / `set_workspace_pinned` / `delete_workspace` | ✅ | ✅ | `useSessionCatalog.ts:764`、`:811`、`:784`、`:691` | `{ path, name }` / `{ workspaceId, name }` / `{ workspaceId, pinned }` / `{ workspaceId }` |
| `upload_init` / `upload_complete` / `upload_cancel` | ✅ | ✅ | `packages/remote-client/src/transfer.ts:204`、`:241`、`:251`（桌面 `host/src/remote_peer/transfer.rs`） | `upload_init`: `{ name, transferName, mimeType, kind, originalSize, transferSize }`，lane=`transfer`；`upload_complete`: `{ transferId }` |
| `download_prepare` / `download_cancel` | ✅ | ✅ | `transfer.ts:283-296`、`:72` | `{ id, sessionId, filePath, mode: "preview"\|"original", name }`；`name` **必须带**，否则主机要重读整段历史反查（`transfer.ts:290-293`） |

### 2.5 桥层：握手、在线、控制（不进 `route()`）

| 命令 | 谁发 | 调用点 | 参数 | 触发 |
|---|---|---|---|---|
| `secure_open` / `secure_finish` | 手机 ✅ / 桌面 ✅ | `packages/remote-client/src/client.ts:1840`、`:1844`；Rust `packages/remote-client/src/session.rs:1448`、`:1484` | `{ type, pairing, message }` / `{ type, id, message }`，subject `p.{pair}.cmd.handshake`，**明文**（加密通道还没建立） | 每次连接（首次配对额外带 secret 走 `secure_finish`） |
| `secure_ready` | 手机 ✅ / 桌面 ✅ | `client.ts:1624-1639`；`session.rs:1544-1551` | `{ features[], deviceName, deviceKind, deviceVersion? }`，lane=`handshake` | 订阅完成且 flush 之后、发布 ready 之前（`client.ts:777-786`） |
| `client_presence` | 手机 ✅ / 桌面 ✅ | `client.ts:1696-1706`；`session.rs:756` | `{ type, syncStage? }`（publish，不等回复） | 连接后立即一次 + **每 15 s**（`client.ts:44`、`:1717`；`session.rs:339`） |
| `client_disconnect` | 手机 ✅ / 桌面 ✅ | `client.ts:1668`；`session.rs:772` | `{ type, reason: "disconnected"\|"background" }` | 用户断开 / App 进入后台 |
| `get_presence` | 手机 ✅ | `client.ts:501` | `{}`，lane=`list`，超时 4 s（`client.ts:31`） | **只在**前台恢复 / presence 过期后的探测（`client.ts:631-672`） |
| `takeover` | 手机 ✅ / 桌面 ✅ | `RemoteContext.tsx:627`、`:953`；`remotePeerClient.ts:201` | `{}`，lane=`list` | 用户点「接管这台机器」 |
| `unpair` | 手机 ✅ | `useDesktopConnection.ts:782`（750 ms 竞速） | `{}` | 用户解除配对 |
| `pair_handshake` / `pair_handshake_confirm` | **无人发送** | 生产环境只作兼容入口（`packages/remote-server/src/commands.rs:915-920`），调用点仅存在于测试 `host/src/remote/commands.rs:466/592` | `{ type, ... }` | 历史客户端 |

---

## 3 时序

### 3.1 手机冷启动（打开 App）

按发生顺序，标 ✅ = 远程命令，⛔ = 只做本地/HTTP：

1. ⛔ 读本地配对表 + 每 30 s 排空待撤销撤销（HTTP `POST /pair/revoke`，`mobile/src/remote/usePairedDesktops.ts:150`，`pairing.ts:196`）。
2. 每个配对桌面挂一个 runtime（`mobile/src/remote/useDesktopRuntime.ts`），各自 `loadCredentials`（本地）→ `client.open()`（`useDesktopConnection.ts:668-701`）。
3. ⛔（仅 JWT 临期时）`POST /auth/token` 换 JWT（`pairing.ts:153`）。**配对与续期都是控制面 HTTP，不是远程命令**。
4. ⛔ WSS 连 NATS（`client.ts:740-760`，超时 10 s，`maxReconnectAttempts: 0`）。
5. ✅ `secure_open`（+ 首次配对时 `secure_finish`）→ 拿到 `HandshakeConfirmation`（含 `bridgeInstanceId`、`features`、`presence.catalogEpoch`）。
6. ⛔ 订阅 4 类主题并 flush：`p.{pair}.evt.{session}`（或 `evt.>`）、`p.{pair}.presence`、`p.{pair}.state.>`、`p.{pair}.xfer.down.>`（`client.ts:777-783`、`:1112`、`:1138`、`:1160`、`:1193`）。
7. ✅ `secure_ready`（声明 6+1 个能力，`client.ts:1749-1770`）。
8. ✅ 立即发一次 `client_presence`，之后每 15 s。
9. ⛔ 主机**推送** `presence` 与 `state.sessions` 快照——**手机端目录不是轮询来的**，是订阅后主机推的（分发在 `client.ts:1193-1240`）。
10. ✅ 仅当 presence 的 `catalogVersion.sessions` 高于已应用版本才 `list_sessions`（`useSessionCatalog.ts:513-533`），`workspaces` 同理。
11. ✅ 若有上次未确认的发送：`get_prompt_receipt` → 视情况 `prompt`/`continue_run`（`usePromptOutbox.ts:697-710`，仅在 `phase==="ready" && businessReady`）。
12. ⛔ 每 60 s 扫一遍非活动桌面，把 `failed`/`stopped` 的 runtime 重建（会重跑第 4-8 步，`RemoteContext.tsx:442`、`:834`）。
13. ⛔ 每 10 min `list_sessions` 自愈（`mobile/src/remote/useCatalogSelfHeal.ts:19` + `useDesktopRuntime.ts:145`）。
14. ⛔ 在线判定每 10 s 跑一次，**不发命令**（`useDesktopConnection.ts:862-874`）；若心跳 15 s 未到则触发 `recover("presence-stale")`。

**冷启动不发**：`list_sessions`（靠推）、`list_models`、`get_settings`、`get_state`。没有任何「恢复上次打开的会话」逻辑（无持久化 selectedSessionId），所以冷启动不打开会话。

### 3.2 手机打开一个会话（用户点一条会话）

| 步 | 内容 | 位置 |
|---|---|---|
| 1 | 本地：`prepareTimelineOpen`（重置 epoch / 分页窗口；**无网络**） | `useTimelineController.ts:516-547` |
| 2 | `client.setVisibleSession(sessionId)` → 只切换 `evt.` 订阅主题（无命令） | `client.ts:189-197`、`:1136-1145` |
| 3 | **并发**发 `get_state(sessionId)` 与 `get_session_entries(before=MAX_SAFE_INTEGER, limit=3, chunkedRead)` | `syncEngine.ts:596-615`、`historyPaging.ts:26` |
| 4 | 每个回复 > 192 KB 的分块：`get_read_chunk`，并发 4（`get_state` 通常 1 块；`get_session_entries` 1-2 块） | `readPages.ts:57-92` |
| 5 | 若尾页 `trimmed` 且不足 3 个用户轮次 → 串行再读 1-3 页 `get_session_entries(before=nextOffset, untrimmed)` | `useTimelineController.ts:260-284` |
| 6 | 历史到即上屏（首屏完成） | `syncEngine.ts:648-660` |
| 7 | 若 `state.activeRun` 非空 → `get_events_since(runId, sinceIdx=-1, limit=1000, preferSnapshot, chunkedRead)`，按 `replayUntilIdx` 续页 | `replay.ts:280-311`、`syncEngine.ts:618-680` |
| 8 | replay 里含 `content_refs` 的大事件 → `get_event_content_range`（每个必填段一次） | `replay.ts:60-66` |
| 9 | 用户点工具行 → `get_tool_call_args`；点文件 → `list_session_files`；点模型 → `set_model` | §2.2 / §2.4 |

即：**一次打开最少 3 个往返**（`get_state` + `get_session_entries` + 至少 1 个 `get_read_chunk`），有活跃 run 时 +1～N（replay 页）+ M（内容补读），页被裁剪时再 +1～2。这就是 03 实测要量到的那条路径。

### 3.3 桌面作为客户端打开一个远程会话

调用链：TS `requestRemotePeer` → Tauri `remote_peer_request` → Rust `host/src/remote_peer/runtime.rs:215-221` → `packages/remote-client` 的 Rust `Client`（10 s 超时，`session.rs:75`；`get_`/`list_` 前缀重试，`session.rs:154-162`）。
`get_session_entries`/`get_event_content_range`/`get_events_since`/`get_tool_call_args` 走 `requestNativeRead`（`remotePeerClient.ts:1381-1382`），由 Rust 侧统一加 `chunkedRead` 并**在 Rust 内重组**，所以桌面端不出现 `get_read_chunk`。

| 步 | 内容 | 位置 |
|---|---|---|
| 1 | `get_session_entries(limit=3, before=尾部游标或 nextOffset)` | `useRemoteTimeline.ts:438-452` |
| 2 | 然后（**串行**）`get_state(sessionId)`，用来判断有无活跃 run、model/thinking/isCompacting | `useRemoteTimeline.ts:591-635`、`:517-530` |
| 3 | 有活跃 run → `readRunLog` → `get_events_since` | `useRemoteTimeline.ts:562`、`:361-378` |
| 4 | 事件到达如有缺前半段/有缝隙 → 再读一次整段日志 | `useRemoteTimeline.ts:709-719` |

与手机的关键差别：桌面**串行**（历史→状态），手机**并发**；桌面目录既有推送又有 5 s 主动拉（见 §4），手机只有推送 + 10 min 自愈；桌面有 5 s 的本机 peer 列表轮询。

---

## 4 重复发送与定时器（完整表）

| 客户端 | 周期 | 发什么 | 条件 | 位置 |
|---|---|---|---|---|
| 手机 | **15 s** | `client_presence`（publish，不等回复） | 连接 ready 期间 | `packages/remote-client/src/client.ts:44`、`:1717` |
| 手机 | 10 s | （无命令）在线判定 + 心跳过期检查 | 常驻 | `mobile/src/remote/useDesktopConnection.ts:862-874` |
| 手机 | **2 s** | `get_queue` | **仅**队列面板打开且队列非空 | `mobile/src/remote/sessionQueue.ts:191`、`:335` |
| 手机 | 5 s | `get_state` | 仅 compaction 等待中，最长 30 min | `mobile/src/remote/timeline/useCompactionOutcomes.ts:9`、`:131-133` |
| 手机 | **10 min** | `list_sessions` | 前台 | `useCatalogSelfHeal.ts:19` + `useDesktopRuntime.ts:145` |
| 手机 | 60 s | 重建非活动桌面 runtime（重连+握手+`secure_ready`） | 仅 `failed`/`stopped` | `mobile/src/remote/RemoteContext.tsx:442`、`:834` |
| 手机 | 30 s | HTTP `POST /pair/revoke` | 仅有待撤销记录 | `mobile/src/remote/usePairedDesktops.ts:150` |
| 手机 | 事件驱动 | `list_sessions` / `list_workspaces` | presence 版本前进、`agent_end`、改名/删除后 | `useSessionCatalog.ts:513-533`、`useTimelineController.ts:229` |
| 手机 | 1 s→30 s 退避 | 重连（握手序列） | 连接失败 | `packages/remote-client/src/connectionState.ts:66-67` |
| 桌面 | **5 s** | `remote_peer_list`（**本地** Tauri 命令，不过网）**以及每台 connected 且非 standby 主机的 `list_sessions` + `list_workspaces`** | Remote 页/侧栏可见（`refresh()` 无版本门控，见 §5-4） | `useRemotePeers.ts:22`、`:316`（定时器）→ `:195-266`（`refresh`：`:236` sessions、`:246` workspaces） |
| 桌面 | 60 s | 仅 `list_sessions`（旧主机/丢包兜底，`CATALOG_POLL_MS`） | 同上，只对 connected | `useRemotePeers.ts:31`、`:349-365`（`:356`） |
| 桌面 | 60 s | liveness backstop：重连所有主机 | 同上 | `useRemotePeers.ts:45`、`:327` |
| 桌面 | **15 s** | `client_presence` | 连接期间 | `packages/remote-client/src/session.rs:339` |
| 桌面 | 2 s | `get_queue` | 队列面板打开 | `desktop/src/features/remote-peer/remoteQueue.ts:235`、`:354` |
| 桌面 | 每连接一次 | `list_models`、`get_desktop_settings`、必要时 `list_available_skills`+`list_skills` | 非 standby | `remoteHostCatalog.ts:107-118`、`:59-88` |

**两端目录机制不对称**：手机是推送驱动（+10 min 自愈），没有任何目录轮询；桌面既有推送（`useRemotePeers.ts:414`、`:429`）又在 5 s 轮询里主动拉全量，且拉取路径没有版本门控。这是后续优化最先要区分对待的地方。

---

## 5 交给后续文档的接口

1. **02（服务端读路径）请按本表命令名逐条对齐**，重点顺序：`get_state`（每次打开）、`get_session_entries`（每次打开 + 翻页）、`get_events_since`（每个活跃 run）、`get_read_chunk`（每次打开）、`list_sessions`（桌面 **5 s/台**、手机 10 min）、`list_workspaces`（桌面 5 s/台）、`get_queue`（2 s 轮询）。
2. 仓库里已有按命令的测量钩子，可直接复用而不是另造：`host/src/remote_host/command_diagnostics.rs`（逐条命令的 execute/reply 拆分，`get_read_chunk` 在 `:295`）、`host/src/remote_host/sync_measurement.rs:121`（把 `get_state`/`get_session_entries`/`get_events_since`/`get_read_chunk` 归为同一同步族）。
3. 本文暴露的三个「非读路径但很贵」的问题，也属于优化范围：
   - `PHONE_COMMANDS` 漏 6 条手机真在发的命令（§1.2），测试对它们盲；
   - 桌面端把历史页与状态读**串行**化（§3.3 第 1-2 步），手机端已并发（`syncEngine.ts:596-615`）；
   - **桌面端目录双路**：既收 `state.sessions`/`state.workspaces` 推送（`useRemotePeers.ts:414`、`:429`），又每 5 s 主动拉 `list_sessions`+`list_workspaces`（`:236`、`:246`）；而推送路径有 `CatalogVersionGate`（`:97`、`:429`），拉取路径**没有**。在多主机 + 679 threads 的真机上，这是每个客户机每 5 s × N 次全量目录读。

---

### 审计方法与可复现性

- 命令集用脚本抽取后比对：`route()` 名字取自 `business/mod.rs:43-105`，`PHONE_COMMANDS` 取自 `:254-315`，客户端命令字面量用
  `grep -rhoE 'type: "[a-z_0-9]+"' mobile/src packages/remote-client/src desktop/src packages/thread-projection/src` 取并集，再剔除事件类型（`agent_*`、`run_finished`、`model_changed` 等）。
- 比对结果：`PHONE_COMMANDS − route = ∅`；`客户端发送 − route − 桥层 = ∅`（即无未路由命令）；`route − PHONE_COMMANDS = 8`（§1.2 表）。
- 未核实项（本文不下结论）：服务端各命令的实际 SQL 与耗时（属 02）；`get_fusion`/`get_tool_call_args` 在大 run 上的数据规模（属 02/03）；桌面端 `remote_peer_list` 的本地开销（本机 Tauri 调用，未测）。
