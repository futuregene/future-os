# 模块审计报告 03：超大模块与职责拆分建议

> 范围：Rust agent（`agent/src/`）、GUI Rust 层（`gui/src-tauri/src/`）、React 层（`gui/src/`）中的大文件。
> 方式：全部文件先做函数级结构清单（`grep -n` 声明），再逐段阅读关键区间；行号均经实际核对。多数文件含大量内联测试，下文区分"生产代码"与测试行数。
> 一句话结论：18 个候选中 **3 个强烈建议拆分（Tier 1）、9 个建议拆分（Tier 2）**；另有 6 个大文件经核查是内聚的，不拆。

---

## 1. 严重程度总排名

| # | 文件 | 行数 | 判定 | 核心问题 |
|---|---|---|---|---|
| 1 | gui/src-tauri/src/agent_bridge/mod.rs | 1343 | 杂物箱 | 6 个主题堆在已有 13 个兄弟模块的 mod.rs |
| 2 | agent/src/session/mod.rs | 3624 | 混合 | 类型+持久化+修复+摘要+fork+消息转换，5 主题 |
| 3 | gui/src/features/agent/Composer.tsx | 704 | 混合→杂物箱 | God component：9 useState+8 useRef 跨 5 个无关关注点 |
| 4 | agent/src/rpc/commands.rs | 2865 | 混合 | 870 行 match，~12 个 arm 内联业务逻辑 + 15 个跨域 helper |
| 5 | agent/src/tools/mod.rs | 1925 | 混合 | 工具注册+作用域+580 行 shell 引擎+fs 操作+路径安全 |
| 6 | gui/src-tauri/src/lib.rs | 763 | 混合 | 事件合并推送基建(~180 行)+Win32 图标(~130 行)混入装配文件 |
| 7 | gui/src-tauri/src/remote/mod.rs | 1084 | 混合 | 生命周期+事件镜像+presence 快照+手写 HTTP 服务器 |
| 8 | gui/src/features/agent/threadRunProjection.ts | 697 | 混合 | 3 条互相独立的对账管线同居 |
| 9 | agent/src/rpc/approval.rs | 1539 | 混合 | 门控生命周期+形状分类+UI 建议构造 3 主题 |
| 10 | agent/src/rpc/protocol.rs | 1590 | 混合 | wire 类型 vs 广播器+日志持久化引擎 |
| 11 | agent/src/types/mod.rs | 1603 | 杂物箱（类型层） | 消息/usage/流事件/工具/配置/模型/provider trait 跨域混装 |
| 12 | agent/src/models/mod.rs | 1519 | 混合 | 模型类型+builtin 表+用户模型合并富化+Registry |
| 13 | gui/src/components/layout/ActivityRail.tsx | 917 | 混合 | 批量选择状态机+账户菜单可剥离 |
| 14 | gui/src-tauri/src/remote/commands.rs | 1198 | 混合 | 握手密码学(~200 行)+分页截断(~125 行)嵌入分发器 |
| 15 | gui/src-tauri/src/store/runs.rs | 1500 | 混合 | 420 行 tool 投影引擎放在 store 文件里 |
| 16 | agent/src/rpc/session_prompt.rs | 1586 | 基本内聚 | 文件级问题小；`prompt_internal` 单函数 766 行 |
| 17 | agent/src/rpc/session.rs | 2127 | 文件内聚/结构体 God | ServerSession 30+ 字段跨 8 类关注点 |
| 18 | gui/src-tauri/src/store/cleanup.rs | 826 | 轻微混合 | run 恢复查询放错名字；与 agent_bridge 有映射重复 |

**内聚、跳过**：agent_bridge/observer.rs（1219）、MentionEditor.tsx（745）、agentActivity.ts（594）、parseFutureMarkdown.ts（559）、AppShell.tsx（597，编排器属既定设计）、SkillsView.tsx（654，单一领域，可选提数据 hook）。理由见 §6。

---

## 2. Tier 1：强烈建议拆分

### 2.1 `gui/src-tauri/src/agent_bridge/mod.rs`（1343 行，生产 ~1256）

目录下已拆出 13 个兄弟模块（approval/client/import/observer/persist/replica/review/session/stream 等），mod.rs 成了"剩下的都放这里"：

| 主题 | 行范围 | 内容 |
|---|---|---|
| 删除 outbox | 48-91 | `reconcile_delete_outbox`、`spawn_delete_outbox_worker` |
| 会话 RPC 包装 | 93-371 | `get_events_since`、`get_session_messages/entries/state`、`get_available_models`、`set_session_model/thinking_level`、`rename_session`、`reload_agent_credentials`、`sync_future_models` 等 13 个 |
| prompt 流水线 | 373-692 | `agent_prompt`(373-456)、`agent_prompt_inner`(458-648)、`auto_name_thread`(653-692) |
| 崩溃恢复 | 701-944 | `reconcile_interrupted_runs`、`check_and_reanimate_run`(739-821)、`reconcile_run_gone`(838-910)、`settle_from_agent_terminal`(916-944) |
| 活动 run 看门狗 | 946-1143 | `plan_active_run_reconciliation`、`reconcile_active_run_once`、`spawn_active_run_watchdog` |
| 远程流/工作区 | 1152-1255 | `attach_remote_stream`、`reconcile_thread_workspace` |

**拆分建议**：
- `agent_bridge/prompt.rs`（主题 3）
- `agent_bridge/reconcile.rs`（主题 4+5，共享 activeRun→requestedRun→interruptedRun 标记优先级逻辑，~550 行）
- `agent_bridge/session_api.rs`（主题 1+2，被 remote/commands、commands/* 跨场景复用）
- `attach_remote_stream` / `reconcile_thread_workspace` 移入现有 `session.rs`
- 拆后 mod.rs 只剩声明+re-export（~50 行）

### 2.2 `agent/src/session/mod.rs`（3624 行，生产 ~1850，测试 ~1774）

注意 `session/persistence.rs`（900 行，异步写入器）已拆出，但 mod.rs 仍有 5 个主题：

| 主题 | 行范围 | 内容 |
|---|---|---|
| 条目类型与常量 | 16-350 | `ENTRY_TYPE_*` / `RUN_STATE_*` 常量、`SessionEntry`、run 标记工具（`is_run_marker` 52、`find_unterminated_run` 65、`next_run_sequence` 103） |
| 持久化管理器 `impl Manager` | 474-1498（~1025 行） | 路径/追加/原子写（`session_path` 486、`append_entries` 537、`save` 761、`write_entries_atomically` 783） |
| 加载期修复 | 836-1021 | `strip_empty_assistants`、`dedupe_tool_entries`(873)、`repair_dangling_tool_calls`(928)——~190 行独立迁移逻辑 |
| 摘要/列表 | 1188-1468 | `summary_from_session`(1233)、`read_summary`(1278)、`list_summaries`(1375)、`list_all`(1388)、`find`/`delete`；另有 run-data GC（`gc_orphan_run_data` 509） |
| fork + AgentMessage↔entry 转换 | 1499-1850 | `fork_session`(1499)、`entries_to_agent_messages`(1630)、`build_context`(1718)、`agent_message_to_entry`(1748)、`truncate_visible`(1820) |

**拆分建议**：
- `session/repair.rs`（3 个修复函数+其测试）
- `session/summary.rs`（摘要扫描族）
- `session/convert.rs`（fork_session + 3 个转换函数，与 `mod fork_tests` 一起）
- `session/manager.rs`（Manager 本体的路径/追加/加载）
- mod.rs 保留类型与常量

### 2.3 `gui/src/features/agent/Composer.tsx`（704 行）

单个 `Composer`（98-704）承担 5 个互不相关关注点（本次审计 React 侧最严重）：

| 关注点 | 行范围 | 内容 |
|---|---|---|
| 技能目录加载 | 141-170 | listInstalledSkills+listAvailableSkills 合并 zh 回退 |
| 草稿持久化接线 | 172-216 | saveDraft + draftKey 加载 + 4 个 ref；存储层已在 `composerDraft.ts` |
| 附件管线 | 282-380 | `addAttachmentPaths` 分类/配额/并发合并、`attachImageFiles`、OS 文件对话框、删除 |
| Tauri 拖放 | 382-446 | dragState + 3 ref + `onDragDropEvent` 监听 |
| 工具栏渲染 | 531-700 | 审批层级/模型/thinking 三个 SelectMenu ~170 行 JSX |

大量 ref-mirror（`addAttachmentPathsRef`、`onDragStateChangeRef`）是为绕开 effect 重订阅——组件承载过多的典型症状。

**拆分建议**：
- `agent/useComposerSkills.ts`
- `agent/useComposerDraft.ts`
- `agent/useComposerAttachments.ts`
- `agent/useComposerDragDrop.ts`（命名已核对无冲突）
- 可选 `ComposerToolbarMenus.tsx`
- 拆后 Composer ~250-300 行

---

## 3. Tier 2：建议拆分

### 3.1 `agent/src/rpc/commands.rs`（2865 行，生产 ~1860，测试 ~1003）

`handle_command_internal`（23-890）是 ~45 arm 的 match；多数 arm 薄，但这些内联了真实逻辑：`prompt`(132-251，120 行)、`get_events_since`(436-498)、`prune_run_events`(285-333)、`cycle_model`(684-731)、`set_cwd`(784-815)、`export_html`(755-782)、`set_session_name`(652-679)。另有 15 个 `cmd_*` helper 横跨会话 CRUD（`cmd_new_session` 1223-1367，145 行；`cmd_get_session_entries` 1368-1546，179 行）、fork/clone(1547-1735)、skills/config 重载(1736-1862)、模型列表（`list_models_response` 905）。

**拆分建议**：保留薄 dispatcher；按域抽出 `rpc/cmd_session.rs`（new/switch/delete/list/fork/clone/get_entries）、`rpc/cmd_config.rs`（reload_config、refresh_skills、set_enabled_models、cycle_model/thinking）、`rpc/cmd_events.rs`（get_events_since、get_session_events_since、prune_run_events）。与已有 `prompt_helpers.rs`（622 行）模式一致。

### 3.2 `agent/src/tools/mod.rs`（1925 行，生产 ~1160，测试 ~764）

5 个主题：

| 主题 | 行范围 | 内容 |
|---|---|---|
| 作用域基建 | 17-128 | `ToolExecutionScope`、`with_tool_scope`、`with_workspace_scope*`、`approve_outside_path` |
| 工具注册/schema | 130-399 | `make_tool`、shell/read/write/edit 的 schema+handler、`coding_tools`、`all_tools` |
| shell 执行引擎 | 401-984（~580 行） | `reject_dangerous_commands`(427)、`is_protected_rm_target`(513)、`shell_segments`(560)、`run_shell`(600)、`spawn_shell`(710-925)、`strip_powershell_clixml`(926)、`format_shell_output`(942) |
| 文件读写/编辑 | 985-1088 | `run_read`、`run_write`、`run_edit`(1009)、`EditOp` |
| 路径安全 | 1090-1161 | `workspace_path`、`ensure_workspace_access`(1106)、`is_approved_outside_path` |

**拆分建议**：`tools/shell.rs`(401-984)、`tools/files.rs`(985-1161，含路径安全)、mod.rs 留作用域+注册（~400 行）。已有 `cmd_exe_rewrite.rs` 先例。

### 3.3 `gui/src-tauri/src/lib.rs`（763 行）

`run()`（410-763）本体是纯装配（合格），但混入两块真实逻辑：

- **事件合并推送基建**（242-407，~180 行）：`coalesce_runtime_updates`(246-276，独立排序/去重语义+测试)、`emit_thread_runtime_updated`(285-339，内含专用 std 线程+40ms 批处理窗)、`emit_approvals_updated`(354)、`start_thread_streaming_monitor`(384-407，1s 轮询)。这是被 store/observer/agent_bridge 到处调用的横切设施（`crate::emit_thread_runtime_updated`）。
- **Win32 图标实现**（86-209，~130 行 ICO 目录解析+`CreateIconFromResourceEx`）

**拆分建议**：新建 `emit.rs`(242-407 全部+`runtime_update_tests`)、`window.rs`(51-209)。拆后 lib.rs ~380 行纯装配。

### 3.4 `gui/src-tauri/src/remote/mod.rs`（1084 行，生产 ~1036）

4 主题：

| 主题 | 行范围 | 内容 |
|---|---|---|
| 桥生命周期 | 22-410 | `RemoteState`(44-76，**17 字段**)、`start`(143-246)、`establish`、`stop`、`status` |
| 事件镜像 | 429-576 | `publish_event`、`build_event_body`、`publish_snapshot`、`cap_event_data`、`spawn_event_publisher` |
| presence 心跳+目录快照 | 578-905（~330 行） | `spawn_presence_heartbeat`、`build_presence_snapshot`(746-812)、`build_sessions_snapshot`(821-868)、`build_workspaces_snapshot`、签名辅助 |
| 嵌入式 Web 服务器 | 917-1035 | `web_dir`、`bind_web_listener`、`lan_ip`、`spawn_web_server`、`handle_web_request`——手写 HTTP 静态文件服务，完全独立 |

**拆分建议**：`remote/presence.rs`、`remote/web.rs`、`remote/events.rs`（re-export 保持 `crate::remote::publish_event` 路径不变）；`spawn_credential_refresh`(644-728)深改 STATE 任务句柄，留在 mod.rs。拆后 ~500 行。

### 3.5 `gui/src/features/agent/threadRunProjection.ts`（697 行）

3 条独立管线：

| 管线 | 行范围 | 内容 |
|---|---|---|
| 增量直播投影+流式气泡 | 31-328 | `liveProjectionCache`、`projectRunForLivePreview`(57)、`streamingBubbleBase`(109)、`mergeStreamingPreview`、`upsertStreamingPreview`(199)、`updatePendingMessageFromRunEvents`(291) |
| reload 时 run↔message 对齐 | 379-571 | `applyRunMetadata`(398)+窗口匹配族 `runWindow`/`runMatchesAnyTurn`/`runMatchesUserTurn`/`isOrphanRun` |
| 丢失内容恢复 | 582-677 | `applyRecoveredEvents`、`recoverAbortedTurns`、`recoverFailedRuns` |

依赖方向：`liveRunPreview → alignment`（`isCompactionDivider`、`runDurationMs` 被两侧共用），无环。有 29KB 配套测试需同步重组。另：`projectRunForLivePreview`(93-94)在纯投影层 `emitFutureEvent("file-tree-refresh")`——副作用应上移到调用方。

**拆分建议**：`agent/liveRunPreview.ts`、`agent/runMessageAlignment.ts`、`agent/runRecovery.ts`；原文件留 `patchMessage`/`deriveRenderFields`/`safeListRunEvents`/`clientId`/`loadCurrentRun`（<100 行）。

### 3.6 `agent/src/rpc/approval.rs`（1539 行，生产 ~850，测试 ~686）

3 主题：

| 主题 | 行范围 | 内容 |
|---|---|---|
| 门控生命周期 | `impl ApprovalGate` 48-425 | `request`(51-176)、`request_escalation`(177-242)、`ask_user`(243)、`decide`(313)、`cancel_session`(381) |
| 形状/策略分类 | 426-690 | `approval_shape`(426)、`shell_auto_allow`(606)、`segment_is_read_only`(622)、`shell_command_shape`(654)、`command_summary` |
| UI 建议构造 | 513-853 | `path_save_suggestion`(513)、`escalation_save_suggestion`(734)、`argument_write_preview`(785)、stderr 阻塞路径提取（`extract_blocked_paths*` 691-733）、`repair_partial_json_object`(814，流式 JSON 修复) |

**拆分建议**：`rpc/approval/gate.rs`、`rpc/approval/shape.rs`、`rpc/approval/suggestions.rs`。

### 3.7 `agent/src/rpc/protocol.rs`（1590 行，生产 ~860，测试 ~730）

两个半主题：

| 主题 | 行范围 | 内容 |
|---|---|---|
| wire 类型 | 9-224, 800-860 | `RpcCommand`(9-90，~25 个扁平可选字段)、`RpcResponse`、`SseEvent`、`RunAttachment`/`RunProjectionSnapshot` |
| 广播+日志引擎（`SseBroadcaster`） | 225-799（~575 行） | 内存 broadcast（`subscribe`/`broadcast` 457）、run 生命周期（`start_run_with_sequence` 523）、**磁盘 journal**（`configure_journal` 267、`append_journal` 653、`read_journal` 700、`recover_storage` 328）、`attach` 重放(400-456)、`events_since`(566)、投影折叠 `apply_to_projection`(743) |

**拆分建议**：`rpc/protocol/wire.rs`（命令/响应/事件类型）、`rpc/broadcast.rs` 或 `rpc/event_journal.rs`（SseBroadcaster 全部）。

### 3.8 `agent/src/types/mod.rs`（1603 行，生产 ~870，测试 ~735）

典型"所有类型都叫 types"的跨域杂物箱：消息与内容块（`ContentBlock`+自定义 serde 19-224、`AgentMessage` 226、`Message`/`ToolCall` 352-397、附件/图片 398-448）、**用量计费**（`Usage` 449-544，含 `deserialize_credit_cost`）、**流事件**（`StreamEvent` 545）、**工具定义**（`ToolDef`/`AgentTool`/`ToolHandler` 589-636）、**agent 配置**（`AgentConfig` 637）、**模型类型**（`Model`/`ModelCost` 664-711）、**provider trait**（`LLMProvider` 712）、**格式转换**（`convert_to_llm`/`convert_from_llm` 783-868）。

**拆分建议**：`types/message.rs`、`types/tool.rs`、`types/usage.rs`+`types/stream.rs`、`types/config.rs`（AgentConfig+Model+LLMProvider）、`types/convert.rs`。纯类型移动，风险低、churn 高，**可最后做**。

### 3.9 `agent/src/models/mod.rs`（1519 行，生产 ~810，测试 ~706）

4 主题：模型类型+能力(18-153)、builtin 表(80-127)、**用户 models.json 加载+合并富化**(233-586，~350 行：`load_user_models_with_overrides`、`provider_similarity`(414)、`find_best_builtin_match`(461)、`enrich_user_models`(495))、`Registry`(595-773，resolve/override/scope)+`glob_match`(775)。

**拆分建议**：`models/enrich.rs`(233-586 全部，含 config 结构体 334-413)、`models/registry.rs`(595-813)；mod.rs 留类型+builtin（~250 行）。

---

## 4. Tier 3：可选 / 局部调整

### 4.1 `gui/src/components/layout/ActivityRail.tsx`（917 行）

主组件 90-691（11 个 useState）。可剥离块：
- **批量选择状态机**（148-253 的 `selectionMode`/`selectionScope`/`selectedThreadIds` + `toggleThreadSelection`/`enterChatSelectionMode`/`selectAll`/`handleStartBatchDelete` + Esc effect——完整自洽子功能）→ `layout/hooks/useThreadSelection.ts`
- **账户菜单**（`AccountMenuButton` 703-786 + `MenuRow`/`ActionBadge`，与 rail 零耦合）→ `layout/AccountMenuButton.tsx`

拆后 ~550 行纯布局+列表。

### 4.2 `gui/src-tauri/src/remote/commands.rs`（1198 行）

分发循环+单飞(172-546)合理，但混入：
- **配对握手密码学**(28-62、553-745，nkeys 签名/转录本绑定 ~200 行) → `remote/handshake.rs`
- **NATS 载荷分页/截断**(881-996，`paginate_messages`、`truncate_message_content`、`byte_cut`，自带 8 个测试) → `remote/paging.rs`

commands.rs 留传输+分发+应答（~600 行）。

### 4.3 `gui/src-tauri/src/store/runs.rs`（1500 行，生产 ~1043）

| 主题 | 行范围 | 内容 |
|---|---|---|
| run 行 CRUD+状态 CAS | 16-329（~250 行） | `create_run`、`latest_run_infos`、`update_run_status_if_active`、`fail_run_if_active` |
| run 事件 legacy 兼容读取器 | 331-617 | 生产代码仅 ~130 行——`RUN_EVENT_BUFFER`/`WriterMsg`/`spawn_disk_writer`/`append_run_event` 等全部 `#[cfg(test)]`（347/368/377/390/547 处，已逐行核实），注释明确"Agent journal 才是事件源" |
| tool-call 投影引擎 | 627-1042（~415 行） | `ToolProjectionState`、`advance_tool_projection`(675)、`apply_tool_event`(745)、shell 语义解析（`tool_end_status` 850、`nonzero_exit_code` 878、`is_soft_fail_command` 887）、`get_tool_call_input`(930)、`project_tool_outputs`(975) |

**拆分建议**：新建 `store/tool_projection.rs`（主题 C——这是事件解释逻辑不是存储，且与前端 `agentActivity.ts` payload 约定强耦合）、`store/run_events.rs`（主题 B）。

### 4.4 `agent/src/rpc/session_prompt.rs`（1586 行）——基本内聚，但有一个巨型函数

主题统一（prompt 提交管线）：排队（`enqueue_prompt` 79-235、`start_next_scheduled` 236）、执行（`prompt_internal` 340-1105）、持久化（`persist_user_message` 1244）、重写快照（`build_rewrite_snapshot` 1357-1548）。**问题在函数级**：`prompt_internal` 单函数 766 行，混合了租约获取、附件物化、消息构造、loop 执行、事件流、持久化。建议**不拆文件**，把 `prompt_internal` 按阶段抽 4-5 个私有方法（部分可用现有 `prompt_helpers.rs`）。

### 4.5 `agent/src/rpc/session.rs`（2127 行，生产 ~880，测试 ~1244）——文件内聚，结构体是 God object

文件本身=结构体+38 个访问器+测试，按"会话聚合根"算内聚。但 `ServerSession`(22-107) **30+ 字段跨 8 类关注点**：身份/元数据（session_id/name/created_by/source_meta）、执行（agent_loop/messages/runtime）、模型配置（model/thinking_level/model_registry）、持久化（session_manager/persistence/ephemeral）、队列（scheduler/scheduled_snapshots/scheduler_wake_rx）、计费（tokens_in/out/cache_r/cache_w/cumulative_cost/last_prompt_tokens）、审批（approval_gate）、安全（permission_level/sandbox_policy/session_rules）。

**建议**（低优先级，结构性）：把 6 个 token/cost 计数器收进 `SessionCounters` 子结构、sandbox/permission/session_rules 收进 `SessionSecurity` 子结构；文件可不拆。

### 4.6 `gui/src-tauri/src/store/cleanup.rs`（826 行，生产 ~455）——轻微

run 恢复查询（`list_active_runs` 57、`reanimate_run` 86、`settle_interrupted_run_from_agent` 107）是 **run 生命周期**语义（供 agent_bridge 看门狗消费），放在叫 cleanup 的文件里名不副实 → 可移 `store/run_recovery.rs`。孤儿回收(173-353)+`clear_finished_runs`(388-454)留在 cleanup 名副其实。

### 4.7 补充：中等尺寸杂物箱（不在候选清单）

**`agent/src/utils/mod.rs`（581 行）**：ID 生成(8-33)、cwd 编码(34)、**图片 MIME 检测+缩放**(43-152，`image_data_url_for_model` 内含解码/降采样逻辑——不属于 utils)、路径/目录 helper(153-200)、**工作区权限修复**(201-278，`ensure_workspace_accessible`+`repair_dir_permissions`)。建议至少把图片处理移出（`agent/src/images.rs`）。

---

## 5. 横切发现

1. **God objects**：
   - `RemoteState`（remote/mod.rs:44-76，17 字段）：NATS client+配对身份+event_tx+5 个 JoinHandle+web_url+配对状态，start/stop/publish/credential-refresh 都直接改它——按 §3.4 拆分后自然瘦身。
   - GUI **没有** AppState/tauri::State；全局状态是模块级 static（`APP_HANDLE`、`OBSERVERS`、`TOOL_PROJECTION_CACHE`、`AGENT_REPLICAS`），其中 `crate::emit_thread_runtime_updated` 等横切设施住在 lib.rs 是 §3.3 的根因。
   - agent 侧 `AppState`（rpc/mod.rs:24-59，15 字段）尚可，但 `welcome_version/cwd/skills/context/exts` 5 个字段可收进 `WelcomeConfig`。
2. **重复编码（漂移点）**：`store/cleanup.rs::agent_terminal_settlement`(131-149)与 `agent_bridge/mod.rs::settle_from_agent_terminal`(916-944)实现几乎相同的 agent 终态→status/error_type 映射，两处各自维护。
3. **三处退出码语义重复**：GUI `store/runs.rs`（`nonzero_exit_code` 878、`is_soft_fail_command` 887）、`gui/src/features/agent/agentActivity.ts`（`nonZeroExitCode` 569、`isSoftExit` 580、`SOFT_FAIL_COMMANDS` 550）、`toolActivityModel.ts`——Rust/TS 两层各维护一份软失败命令清单。
4. **循环导入**：未发现。TS 依赖单向：`threadRunProjection → agentActivity → toolActivityModel`；`Composer → MentionEditor`；`AppShell → ActivityRail`。Rust 单 crate 无模块循环问题。
5. **规范小疵**：AppShell.tsx 137-143 直接用 `window.addEventListener("future:cwd-changed")`，违反 gui/CLAUDE.md 的事件总线原则（非拆分项）。

---

## 6. 内聚文件跳过理由

- **`agent_bridge/observer.rs`（1219）**：全文件只回答"每 session 事件流如何被监听/校验/扇出"；注册表、LRU 淘汰、run 绑定、游标校验、`handle_event`(623-822)四路扇出互相引用密集，拆开制造跨文件状态。大但内聚。
- **`MentionEditor.tsx`（745）**：单个 contentEditable 编辑器+两个补全菜单；纯函数 helper 与命令式 DOM 操作共享 pill 隐式契约（`PILL_ATTR`），WebKit IME 约束下的刻意写法。保持原样。
- **`agentActivity.ts`（594）**：单一增量状态机（run 事件→渲染投影）+其私有解析器；`createRunProjector` 闭包状态与 `snapshot` 强耦合。退出码三件套（45 行）可移入现有 `toolActivityModel.ts`。
- **`parseFutureMarkdown.ts`（559）**：单条 mdast→FutureMarkdownDocument 管线；futureos 引用解析(~120 行)与主 switch 耦合，目录已有 `futureReferenceStore`/`futureMarkdownTypes`/`resolveFutureReferences` 分担。保持原样。
- **`AppShell.tsx`（597）**：12 个 useState 全部是布局编排状态，域逻辑已外置到 16 个 hooks（gui/CLAUDE.md 既定设计）。合格。
- **`SkillsView.tsx`（654）**：单一技能目录域，子组件已分列；可选抽 `useSkillsCatalog.ts`（非紧急）。

---

## 7. 建议执行顺序

1. `agent_bridge/mod.rs` 拆分（收益最大、兄弟模块模式现成）
2. `Composer.tsx` hook 化（核心 UX、改动最频繁）
3. `lib.rs` → `emit.rs`+`window.rs`（消除横切设施错位）
4. `threadRunProjection.ts` 三管线拆分（连带测试重组）
5. `session/mod.rs` 拆 repair/summary/convert
6. `tools/mod.rs` 拆 shell/files
7. 其余 Tier 2/3 按需；`types/mod.rs` churn 高，放最后

> 注：所有拆分均为纯移动+re-export，不改行为；建议每个文件单独一个 commit，便于回滚与 review。
