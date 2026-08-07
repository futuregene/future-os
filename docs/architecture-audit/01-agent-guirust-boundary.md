# 边界审计报告 01：agent ↔ gui_rust

> 范围：Rust agent 后端（`agent/`）与 GUI 的 Tauri Rust 层（`gui/src-tauri/`）之间的边界。
> 方式：只读审计，未修改任何文件。所有行号基于审计时的仓库状态（与本地 `dev` 树内容一致）。
> 一句话结论：**边界是泄漏的（leaky），且双向泄漏。** gRPC 是"信封"，信封里装的是无 schema 的 JSON 字符串 + 一捆共享文件。agent 与 GUI 之间是"共享磁盘状态 + 影子 JSON 契约"的联邦，而不是 proto 隔离的客户-服务端。

---

## 1. 边界通道清单

### 1.1 gRPC 通道（proto/future.proto：`ExecuteCommand` + `StreamEvents`）

GUI Rust 端使用的 RPC 命令（构造点集中在 `gui/src-tauri/src/agent_bridge/client.rs`，另有散落调用）：

| 命令 | GUI 构造位置 |
|---|---|
| `prompt` | client.rs:321-344 |
| `new_session` | client.rs:217-237 |
| `get_state`（含 run_id 变体） | client.rs:164-173 |
| `list_sessions` / `list_streaming_sessions` | client.rs:202-211 |
| `get_session_entries` / `get_messages` / `get_events_since` / `get_session_events_since` | client.rs:213-215; mod.rs:117-181; observer.rs:830-833 |
| `list_models` / `set_model` / `set_default_model` | client.rs:175-177, 239-253; models.rs:44 |
| `set_thinking_level` / `set_session_name` / `set_cwd` / `set_permission_level` | client.rs:255-281 |
| `set_sandbox_policy`（唯一 typed 子消息） | client.rs:283-291 |
| `add_session_rule` | client.rs:294-304 |
| `approval_decision` | client.rs:346-358 |
| `fork` / `delete_session` / `prune_run_events` | client.rs:179-200 |
| `abort` | run_control.rs:51, 74 |
| `reload_auth` / `sync_future_models` | mod.rs:327 / mod.rs:364 |
| `get_commands` / `refresh_skills` | agent_bridge/skills.rs |

事件流词汇（stringly-typed，`StreamEvent.type`）：

- run 事件：`agent_start` / `agent_end` / `user_message` / `text_chunk` / `thinking_*` / `tool_start` / `tool_delta` / `tool_end` / `approval_request` / `approval_decision` / `usage` / `error` / `stream_gap`
- session 设置事件：`model_changed` / `thinking_level_changed` / `permission_level_changed` / `session_name_changed` / `cwd_changed` / `config_reloaded` / `sandbox_policy_changed`

GUI 白名单硬编码于 `observer.rs:71-81`（`FORWARDED_EVENTS`）。

### 1.2 绕过 gRPC 的文件系统通道（GUI 直接读写 agent 拥有的文件）

| 通道 | 方向 | GUI 位置 | Agent 对应 |
|---|---|---|---|
| `~/.future/agent/auth.json` | **GUI 读+写** | auth_store.rs:25-54, 84-149; future_login.rs | agent/src/auth/mod.rs（`AuthStore::load`）; models/future.rs:108-131 |
| `~/.future/agent/models.json` | **GUI 读+写** | agent_providers/catalog.rs:104-106; write.rs:92-126, 144-258; mod.rs:138 | agent/src/models/mod.rs（loader） |
| `~/.future/agent/.future-models-cache.json` | **GUI 读**（agent 写） | agent_providers/catalog.rs:108-122 | agent/src/models/future.rs:93-95 |
| `~/.future/agent/skills/` | **GUI 写+删**（install/uninstall） | skills.rs:68-81, 167-223 | agent skills 发现（`refresh_skills` RPC 仅做缓存失效） |
| `~/.future/agent/sessions/{id}.jsonl` | **GUI 探测存在性** | store/cleanup.rs:173-241 | agent/src/session/（Manager 持久化） |
| `${WS}/.future/approval_rule.json` | **GUI 写，agent 直接读** | approval_rules.rs:24-58; commands/approvals.rs:45-51 | agent/src/sandbox/rules.rs:269, 457 |
| agent 源码本身 | **GUI 编译期引入** | agent_providers/catalog.rs:15-16 `#[path = "../../../../agent/src/models/builtin/mod.rs"]` | agent/src/models/builtin/mod.rs |
| agent 进程生命周期 | GUI spawn/kill sidecar | agent_supervisor.rs:59-86, 241 | 固定端口 127.0.0.1:50051（client.rs:26-28） |
| webview 原始文件读取 | GUI 可读 agent sessions/settings 原始字节（仅 auth.json/models.json 拉黑） | commands/files.rs:48-80, 655-667（测试显式放行 `agent/sessions/s.jsonl`、`agent/settings.json`） | — |

> ⚠️ 本表为审计时点（2026-08-05）状态：第 1/2 行（auth.json/models.json 写）与第 7 行（`#[path]` include）已在 commit `306cf05f` 修复——配置写改由 agent 经 `set_auth`/`upsert_provider`/`delete_provider` RPC 自持，`#[path]` 改走 `list_models` RPC；第 5 行（`{id}.jsonl` 探测）同批改走 `list_session_ids` RPC。详见对应 H2/H3/H4 条目与 README 时效性说明。

---

## 2. 耦合 / 坏味道清单（按严重度）

### HIGH

**H1. proto 契约实质上是"JSON-in-string"，typed 消息是摆设**
- `RpcResponse.data`（proto/future.proto:220）与 `StreamEvent.data`（proto:392）均为 `string`，承载即兴 JSON。
- proto 定义了 typed `SessionState`（proto:237-320），但 agent 的 `get_state` 从不序列化它，而是返回 ~35 个键的 ad-hoc JSON（agent/src/rpc/mod.rs:339-376），其中 `activeRun` / `queuedRuns` / `interruptedRun` / `requestedRun` / `pendingApprovals` / `createdBy` / `sourceMeta` 等键**在 proto 中完全不存在**，且大小写混乱（`session_name` snake_case 与 `sessionId` camelCase 并存，rpc/mod.rs:347-348）。
- GUI 在至少 8 处逐键解析这些未契约化的键：mod.rs:765-783、approval.rs:163-172、session.rs:46-61、observer.rs:606-612、import.rs:447-468 等。
- 同理 `list_sessions` / `list_models` / `get_session_entries` / `get_events_since` 的载荷全是 stringly JSON（agent/src/rpc/commands.rs:1000-1022, 931-964, 1481-1536, 473-519）。
- **为何是问题**：proto 丧失了契约功能；任何 agent 端 JSON 键改名都不会有编译期保护，只能靠 GUI 的多别名兜底（见 M4）掩盖漂移。

**H2. GUI 直接写 agent 拥有的配置文件（auth.json / models.json）** —— ✅ **已修复（commit `306cf05f`）**：改由 agent 通过 `set_auth` / `upsert_provider` / `delete_provider` RPC 自行写盘；GUI 本地 read-modify-write 降级为 agent 不可达时的 fallback（`auth_store.rs` / `agent_providers/write.rs` 头部注释均声明「RPC-first (audit item 2)」）。下述描述为修复前状态。
- auth_store.rs:1 注释自述："Strict, atomic, 0600 read/write for `~/.future/agent/auth.json`"、"single write path for the agent auth file"。写入逻辑：auth_store.rs:84-149（set_provider_key / set_future_login / set_future_base_url / remove_provider_entry）。
- models.json 的 read-modify-write：agent_providers/write.rs:92-126（baseUrl 覆盖）、131-228（自定义 provider upsert）、230-263（删除）。
- 一致性靠事后补一刀 RPC：`reload_auth`（mod.rs:310-335，注释明确说明 agent 内存缓存 key、不 reload 会"退出登录后仍可继续回答"）。
- **为何是问题**：agent 的核心配置被两个进程按各自实现的格式解析/写回（agent 侧 auth/mod.rs、models/mod.rs 各有一套解析），没有 schema、没有跨进程锁（config_io.rs 的 `with_config_lock` 只是 GUI 进程内锁），并发写（如 CLI 登录 + GUI 登录）可能互相覆盖。

**H3. GUI 编译期直接 `include` agent 源码** —— ✅ **已修复（commit `306cf05f`）**：`#[path]` include 已移除，内置 provider 目录改经 `list_models` RPC（`include_builtin_providers`）运行时获取，GUI 不再保留独立 id→name 映射（`agent_providers/catalog.rs` 头部注释）。下述代码为修复前状态。
- agent_providers/catalog.rs:13-16：
  ```rust
  #[path = "../../../../agent/src/models/builtin/mod.rs"]
  mod generated_model_catalog;
  ```
- **为何是问题**：这是源码级耦合——agent 改动该文件（或其 `include_str!("models.json")` 数据）会直接改变 GUI 二进制的行为，而两边可独立构建/发版；等于把"共享领域数据"用文件系统路径硬连接，完全绕过任何契约。

**H4. GUI 依赖 agent 会话文件布局（文件名约定）** —— ✅ **已修复（commit `306cf05f`）**：`reconcile_orphan_sessions` 改走 `list_session_ids` RPC 的「仅文件名」枚举，不再自行构造 `~/.future/agent/sessions/` 路径或探测 `{id}.jsonl`（`store/cleanup.rs` 头部注释：「The GUI no longer probes `{id}.jsonl` filenames itself」）。下述描述为修复前状态。
- store/cleanup.rs:173-198 `reconcile_orphan_sessions`：直接构造 `~/.future/agent/sessions/` 路径（177-180），并按 `{session_id}.jsonl` 探测文件存在性（236），据此**硬删除 GUI 线程**。
- **为何是问题**：扁平目录 + `{id}.jsonl` 命名是 agent 的存储内部实现（agent/src/session/ Manager）；agent 一旦迁移存储（分片、子目录、改扩展名），GUI 会静默误判"所有会话被外部删除"并删库。且该信息完全可以通过 RPC（如 `list_sessions` 差集）获得。

**H5. `new_session` 用 `custom_instructions` 字段走私 JSON**
- proto:60-63 定义 `custom_instructions` 是"compaction summariser 的自定义指令"；但 GUI 的 client.rs:225-229 塞入 `{"createdBy":"gui","sourceMeta":...}` JSON 字符串，agent 端 commands.rs:1308-1323 按此约定解析 `createdBy` / `sourceMeta`。
- **为何是问题**：字段语义被双重占用（同一字段在 `compact` 和 `new_session` 下含义完全不同），且是 string 里再套 JSON——proto 已有能力加 typed 字段却没有加。

### MEDIUM

**M1. Future 平台/base URL 解析逻辑双实现，且优先级不一致**
- GUI：future_platform.rs:32-51 —— 优先级 `platform_base_url` → `base_url`（剥 `/api`）→ 默认；model API = `{platform}/api/v1`。
- Agent：agent/src/models/future.rs:108-131 —— 优先级 **`base_url` 先**（legacy）→ `platform_base_url`（拼 `/api`）→ 默认。
- **为何是问题**：两边优先级顺序相反；目前只靠 GUI 的 auth_store 写盘时保证两字段不共存（auth_store.rs:143-149 删 `platform_base_url`）才没炸。这是典型的"靠调用方纪律维持的重复逻辑"。

**M2. GUI 解析 agent 工具输出的展示文案与内部格式**
- persist.rs:270-280 `file_path_from_tool_output`：解析 agent 工具的成败散文 `"Written to <path>"` / `"Edited <path>"`，注释直接引用 agent 内部函数（agent/src/tools/mod.rs:299 `format!("Written to ...")`、:369 `"Edited ..."`）。
- persist.rs:353-359 `nonzero_exit_code`：解析 agent shell 工具的 `"[exit: N]"` 尾行格式（agent/src/tools/mod.rs:961 `format!("[exit: {}]", ...)`）。
- persist.rs:361-388 `is_soft_fail_command`：GUI 自行实现"grep/diff/cmp/test/findstr 退出码 1 不算失败"的 shell 语义启发式——这本质是 agent 应在 `tool_end.error` 字段里给出的结论。
- **为何是问题**：agent 改一句输出文案或尾行格式，GUI 的工件提取/失败判定就静默失效（persist.rs:209-212 注释自己也承认散文"不是契约"）。

**M3. GUI 重实现 agent 会话条目的投影逻辑（消息/工具事件重建）**
- session.rs:357-451 `synthesize_run_events_from_entries`：解析 agent entry 的 LLM wire 形状（`tool_calls[].id`、`function.name`、`function.arguments`、`tool_call_id` 匹配），凭空合成 `tool_start` / `tool_end` 事件写入 GUI store。
- session.rs:259-283：fork 后从 `session_info` entry 的 content JSON 里挖 `session_name` / `model`，依赖 agent fork 的内部写法（含 `"(fork)"` 哨兵值）。
- session.rs:453-462 `split_model`：重复实现"provider/model"斜杠拆分——该规则在 proto:48-50 有文档、agent commands.rs:80 也实现了一遍。
- import.rs:405-423：导入时按 assistant entry 计数合成历史 run。
- **为何是问题**：agent 的 entry 格式（agent/src/session/mod.rs:139+ `SessionEntry`）成了隐性第二契约；`get_session_entries` 返回什么形状，GUI 就得跟着解析什么，两边没有共享类型。

**M4. 事件载荷字段名漂移，靠多别名兜底**
- persist.rs:62-97、182-188 `value_string(value, &["approval_request_id","approvalRequestId"])`、`&["tool_id","toolID","tool_call_id"]`、`&["tool_name","toolName"]`、`&["risk_level","riskLevel"]`……
- agent 内部 `StreamEvent` 结构用 camelCase（types/mod.rs:545-580，`toolName` / `toolID`），而事件 data JSON 里混用 snake_case。
- **为何是问题**：别名列表本身就是"没有 schema"的化石证据；每加一个别名都是对一次历史漂移的修补。

**M5. GUI 向 agent 的 skills 目录写盘 + RPC 失效缓存的握手**
- skills.rs:72 安装目标 `agent_dir()?.join("skills")`；install_skill / uninstall_skill（skills.rs:167-223）直接 `remove_dir_all`；随后调 `refresh_skills`（agent commands.rs:1736-1769，注释明确说"GUI/CLI 在 install/uninstall 后调用"）。
- **为何是问题**：目录布局、SKILL.md frontmatter 版本解析（skills.rs:262-277）、"app scope 优先"的发现顺序都是 agent 内部知识；正确边界应是 agent 提供 install/uninstall RPC。

**M6. run 终态映射逻辑重复**
- GUI：mod.rs:916-944 `settle_from_agent_terminal`（completed/cancelled/error→本地状态）+ cleanup.rs:131-149 `agent_terminal_settlement`（同一映射的第二份拷贝，多一个 incomplete）。
- Agent：agent/src/session/mod.rs:40-47 `RUN_STATE_*` 常量族。
- **为何是问题**：agent 增加一种终态（如已有的 `interrupted_by_restart`），GUI 两处映射都要人肉跟进。

**M7. 会话标题推导三处重复**
- mod.rs:653-692 `auto_name_thread`（注释："matching the TUI's `first_message` behavior"、"same as the TUI's truncate_visible"，截 40 字符）；import.rs:133-170 `session_title` 又一份；agent 侧 `list_sessions` 提供 `first_message`（commands.rs:1012）本身也是为这个逻辑服务的。

### LOW

**L1. 观察者硬编码 agent 设置事件名**（observer.rs:71-81）：agent 新增/改名 session 事件，GUI 白名单静默丢事件。

**L2. webview 可读 agent 原始数据**（commands/files.rs:48-80 + 测试 655-667）：denylist 只挡 auth.json/models.json，`agent/sessions/*.jsonl`、`agent/settings.json` 明确放行——存在一条非正式的"webview→agent 磁盘"原始字节读通道。

**L3. agent 的持久化格式反向为 GUI 渲染定制**（agent 侧的 GUI-specific 泄漏）：commands.rs:1494-1518 `get_session_entries` 注释："so the GUI can rebuild attachment chips after reload — the JSONL is the only message source"、"the GUI's message footer ('time · N tokens')"；proto:164-167 `Attachment.thumbnail` 注释同样承认 GUI 渲染需求。agent 的存储 schema 与 GUI 展示需求互相渗透（双向泄漏）。

**L4. 部署耦合**：agent_supervisor.rs GUI 直接 spawn/kill `future` sidecar（`future agent --grpc-addr ...`，内嵌 agent），固定 127.0.0.1:50051（client.rs:26-28）。对捆绑发版是合理的，但意味着 GUI 进程对 agent 生命周期有完全控制权（含 force-quit 时 abort 会话，agent_supervisor.rs:231-241）。

**L5. 审批卡载荷映射在 GUI 内部也重复两份**：persist.rs:61-115（实时事件路径）与 approval.rs:224-294 `heal_pending_approval_from_agent`（get_state 重建路径）逐字段重复同一 agent JSON 的解析，注释互相引用对方（approval.rs:220-223）。

---

## 3. 重复领域逻辑对照表

| 逻辑 | GUI 位置 | Agent 位置 |
|---|---|---|
| Future 平台/base URL 解析 | future_platform.rs:32-51 | models/future.rs:108-131（**优先级相反**） |
| auth.json 条目格式（`type`/`key`/`base_url`/`platform_base_url`） | auth_store.rs:60-149 | auth/mod.rs + models/future.rs:115-126 |
| 内置模型目录（~900 模型） | catalog.rs:15-16 **#[path] 直接编译 agent 源码** | models/builtin/mod.rs |
| `provider/model` 斜杠拆分 | session.rs:453-462 | proto:48-50 文档 + commands.rs:80 |
| run 终态映射 | mod.rs:916-944 + cleanup.rs:131-149（两份） | session/mod.rs:40-47 |
| shell 退出码/软失败语义 | persist.rs:339-405 | tools/mod.rs:692-698, 961 |
| write/edit 目标路径提取 | persist.rs:249-280 | tools/mod.rs:299, 369（输出文案） |
| 标题推导（40 字符截断） | mod.rs:653-692 + import.rs:133-170 | commands.rs:1012（`first_message`）+ TUI |
| 审批卡 JSON→记录映射 | persist.rs:61-115 + approval.rs:224-294（GUI 内两份） | rpc/approval.rs（事件发射方） |
| session 条目结构解析（role/tool_calls/tool_call_id/session_info/meta） | session.rs:182-218, 259-288, 357-451; import.rs:405-411 | session/mod.rs:139+ `SessionEntry`; commands.rs:1368-1545 |

---

## 4. 总体结论：边界是泄漏的，且双向泄漏

名义上 gRPC proto 是唯一契约，实际上存在 **至少 7 条文件系统旁路** 和一套 **未类型化的 JSON-in-string 影子契约**。

**最强证据（按说服力排序）：**

1. **`RpcResponse.data` / `StreamEvent.data` 全为 JSON 字符串，proto 的 `SessionState` 消息无人使用**；真实 get_state 载荷（agent/src/rpc/mod.rs:339-376）携带 `activeRun` / `interruptedRun` / `requestedRun` / `pendingApprovals` 等 proto 中不存在的键——GUI 的崩溃恢复、watchdog、审批重建全部构建在这套影子 JSON 上（mod.rs:739-944, approval.rs:120-218）。
2. **GUI 是 agent auth.json 的"唯一写入者"**（auth_store.rs:1-5 自述），并直接 RMW models.json（write.rs）、读取 agent 的模型缓存文件（catalog.rs:108-122）——agent 配置域被 GUI 旁路接管，一致性靠 `reload_auth` 事后通知。⚠️ 修复前状态：commit `306cf05f` 起 GUI 改为 RPC-first（`set_auth`/`upsert_provider`/`delete_provider`），本地写仅作 fallback。
3. **`#[path = "../../../../agent/src/models/builtin/mod.rs"]`**（catalog.rs:15）——GUI 二进制物理编译 agent 源码，任何"契约"讨论在此失效。⚠️ 修复前状态：commit `306cf05f` 起内置目录改经 `list_models` RPC 运行时获取。
4. **cleanup.rs:177-180, 236**：GUI 按 `{session_id}.jsonl` 命名约定探测 agent 会话文件存在性，并据此删除自己的线程——对 agent 存储布局的直接依赖。⚠️ 修复前状态：commit `306cf05f` 起改走 `list_session_ids` RPC。
5. **persist.rs:270-280, 353-359**：GUI 解析 agent 工具的展示文案（"Written to …"）与 `[exit: N]` 尾行格式，注释直接点名 agent 内部函数 `tools::run_write` / `tools::run_shell`——连"输出散文"都成了事实契约。

---

## 5. 值得记录的正面事实（避免偏颇）

- 会话导入/fork/历史重建走 RPC（`list_sessions` / `get_session_entries`）而非直接读 JSONL（import.rs、session.rs）——读路径基本守住了边界，只有 cleanup.rs 的存在性探测例外。
- GUI 有意不复制 agent 的原始事件日志（persist.rs:41-44："Raw events are durable in the Agent event journal. Do not create a second GUI JSONL copy"）。
- `set_sandbox_policy` 是唯一演进出 typed 子消息的命令（proto:135-137）；审批规则文件旁路是**成文设计**（proto:175-178 reserved 注释 + gui/APPROVAL_PLAN.md），不是失控产物。
- 附件改为路径过线、agent 自行读图（proto:148-168）是近期有意的契约收紧。

---

## 6. 修复方向建议（按优先级）

1. **给影子 JSON 契约上类型**：把 `RpcResponse.data` / `StreamEvent.data` 从裸 `string` 演进为 `oneof` typed 消息，或至少把 get_state / list_sessions / get_session_entries / get_events_since 的载荷定义为 proto message。这是消除最多下游解析代码（GUI 的 8+ 处逐键解析、多别名兜底）的单点杠杆。⚠️ 截至 2026-08-06 **未修复**（对应 H1）。
2. **收敛配置写路径**：GUI 不再直接写 auth.json / models.json，改为调用 agent 新增的 `set_auth` / `upsert_provider` / `delete_provider` RPC；`reload_auth` 从事后补救变成不再需要。✅ **已实施（commit `306cf05f`）**（对应 H2）。
3. **消除 `#[path]` 编译期耦合**：内置模型目录要么走 `list_models` RPC 运行时获取，要么抽成独立 crate 由两边共同依赖。✅ **已实施（commit `306cf05f`）**：改走 `list_models` RPC（对应 H3）。
4. **用 RPC 替代文件探测**：cleanup.rs 的孤儿会话判定改用 `list_sessions` 差集，不再依赖 `{id}.jsonl` 文件名约定。✅ **已实施（commit `306cf05f`）**：改走 `list_session_ids` RPC（对应 H4）。
5. **给 `new_session` 加 typed 字段**：`created_by` / `source_meta` 独立字段，停止占用 `custom_instructions`。
6. **统一 Future URL 解析**：两侧优先级对齐，或只在 agent 侧解析、GUI 只消费结果。
7. **把工具语义结论放进事件**：`tool_end` 携带 `exit_code` / `is_soft_fail` / `target_path` 等结构化字段，GUI 不再解析输出散文。
