# GUI 问题清单（2026-07-07 全量审查）

来源：五路并行审查 —— 模块结构 / 前端逻辑 / Tauri 后端逻辑 / 代码卫生与泄露 / 文档一致性。
每条含：严重度（高/中/低）、位置、问题描述、**处置**。

## 执行进度（2026-07-07 重构落地）

- **批次0 ✅** — SEC-01（skill id 白名单+越界断言，含单测）、SEC-03（`ensure_path_allowed` 拦 `~/.future`）、STR-05（useBuildInfo 归位）、DOC-08（globals.css 走 `theme()` token）、HYG-01（死变量）
- **方案一 ✅** — 全部 run 状态写入改 `update_run_status_if_active`，删除无守卫版本；`abort`/approval 事件/前端完成/远程完成路径全部 CAS 化；启动 `cancel_stale_approval_requests` 扩为收敛所有僵尸 run（含级联，新增测试）；`decide_approval_request` 加 pending 守卫；流截断 → `AgentResponse.complete` → 标 failed（RUN-01~07）
- **方案二 ✅** — `useAgentThreadState` 918→654 行，拆出 `threadRunProjection.ts`/`threadAttachments.ts`/`useStickyAutoScroll.ts`（+9 单测）；FE-01（局部定时器）、FE-02（`targetThreadId` 校验）、FE-03（`canSend` 统一门禁 + toast）、FE-04（generation guard + 5s connect 超时）、FE-05/06/07（per-thread catch / refreshStore guard / 保留审批列表）
- **方案三 ✅** — 新增 `config_io.rs`（严格读 + 原子写 + 每路径 RMW 锁，4 单测）；`agent_providers`/`auth_store`/`approval_rules` 迁移（CFG-01/02/03）；CFG-04 内建 provider 删除守卫；`future_platform.rs` 归位错置解析器；`qualify_columns`（DUP-07）
- **方案四 ✅** — 删除 4 张死表（`DROPPED_TABLES` 迁移 + 测试）；PRODUCT/ER/CLAUDE/APPROVAL_PLAN 回写（DOC-01~06；口径：不做第三方付费市场，仅官方平台目录）
- **方案五 ✅（部分）** — `useDropUpMenu`（DUP-01）、`NavButton`（消 5 处重复）、`lib/errors.ts errorMessage`（DUP-03，已迁移本次改动涉及文件）。**未做**：ActivityRail 多文件拆分（纯行数、暂缓）、`formatBytes` 合并（两处语义不同、非真重复）、BackButton/IconButton 变体、errorMessage 全量 23 处迁移
- **追加批次 ✅** — FE-10（附件截断改 UTF-8 字符边界安全 `truncateToBytes`，PDF 加 `MAX_PDF_PAGES=100`）、FE-08（abort 部分文本落库不再受 `isCurrentSend` 门控，切线程不丢失）、FE-11（缩略图 key 改 SHA-256 前缀，抗碰撞）、STR-03（ActivityRail 826→484 行，拆出 `ThreadListItem.tsx` / `ActivityRailMenus.tsx`）
- **DUP-08 ✅** — 新增 `store/record_macro.rs` 的 `sql_record!` 宏：从单一字段列表同时生成 `*_COLUMNS` 与 `*_from_row`，两者不可能漂移；14 条记录中 13 条已转换（`research_resource` 因列名带 `r.`/`c.` 表别名保持手写）。含自包含宏测试（列序 + 位置映射 + bool/NULL）
- **仍暂缓** — SEC-02（远程零鉴权，功能未开发完）、SEC-04/05（端口冒充 / 更新包哈希，后者需流水线先发布 hash）、FE-09（仅 legacy 数据）、STR-04/07、DOC-07、HYG-02

验证：`cargo test`（91 通过）+ clippy + fmt 全绿；`tsc` + `eslint` + `vitest`（50 通过）全绿。

处置批次定义：
- **批次0** — P0 独立小改动，立即修
- **方案一** — Run 状态机 CAS 统一
- **方案二** — useAgentThreadState 拆分 + 发送管线竞态修复
- **方案三** — 配置文件 IO 层统一 + agent_providers 拆分
- **方案四** — Skill 收口（承认商店路线，回写 PRODUCT/ER，删死表）
- **方案五** — ActivityRail 拆分 + 前端重复代码合并
- **暂缓** — 本轮不做，保留记录
- **记录** — 低价值/边缘场景，择机处理

---

## 一、安全

| ID | 严重度 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| SEC-01 | 高 | `src-tauri/src/skills.rs:158,167-184`；入口 `commands/skills.rs:18-25` | Skill 安装/卸载对来自远程目录的 `id` 零校验，直接 `join` 后 `remove_dir_all`。`id="../../Documents"` 可删除任意目录；绝对路径 `join` 整体替换危害相同 | **批次0**（方案四含后续） |
| SEC-02 | 高 | `src-tauri/src/remote/mod.rs:171-282`；`store/app_settings.rs:46` | 远程控制命令通道（list_sessions/get_messages/prompt）零鉴权，唯一隔离是 NATS subject 前缀；默认 pair id 为常量 `"DEVPAIR"`；NATS 连接不要求 TLS/凭据；`prompt` 等价 RCE | **暂缓**（功能未开发完；放开入口前必须完成鉴权） |
| SEC-03 | 中 | `src-tauri/src/commands/files.rs:32,65,130,153,178-200` | 文件类 Tauri 命令无路径约束：`read_file_base64` 任意读、`export_artifact_file` 任意写、`open_path` 任意启动。webview 一旦 XSS 即升级为任意读写原语，绕过 approval/sandbox 体系。对照组内正确做法：`delete_temp_attachment`（:113-127）有 canonicalize + 前缀约束 | **批次0**（简单修复：canonicalize + 敏感目录 deny-list） |
| SEC-04 | 低 | `src-tauri/src/agent_supervisor.rs:34-46` | Agent 端口探测即信任：127.0.0.1:50051 有监听就 attach，本机任意进程可冒充 agent 接收全部 prompt 流量 | 暂缓 |
| SEC-05 | 低 | `src-tauri/src/commands/update.rs:144-205` | 更新包仅靠 HTTPS + URL 前缀保护，manifest 与安装包无签名/哈希校验（`file_name` 已正确防目录逃逸） | 暂缓 |

## 二、Run 状态机 / 事件流（→ 方案一）

| ID | 严重度 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| RUN-01 | 高 | `src-tauri/src/agent_bridge/run_control.rs:37-43` | `abort_run` 用无守卫 `update_run_status` 无条件写 `cancelled`，可把已 `completed` 的 run 改写为"已停止"并级联取消 approvals/tool_calls。代码库已有 CAS 版本 `update_run_status_if_active`（`store/runs.rs:229-232`）未被使用 | **方案一** |
| RUN-02 | 中 | `src-tauri/src/agent_bridge/persist.rs:108-115` | 迟到的 `approval_request` 事件把已 `cancelled` 的 run 复活为 `waiting_approval` 并插入 pending 审批；agent 已 abort 永不回决策 → run 永久卡死 | **方案一** |
| RUN-03 | 中 | `src-tauri/src/commands/runs.rs:17-21`（前端 `useAgentThreadState.ts:353` 调用）、`remote/mod.rs:345,366` | 其余无守卫 `update_run_status` 调用点：完成/失败路径可覆盖并发 abort 的 `cancelled` 终态（前端 :395-397 的先读后写是 TOCTOU） | **方案一** |
| RUN-04 | 中 | `src-tauri/src/store/cleanup.rs:133-168` + 前端 `useAgentThreadState.ts:87-94,504-523` | GUI 崩溃/被杀后遗留的 `running` 僵尸 run 永不收敛（启动清理只覆盖带 pending approval 的）→ 重启后线程永久"生成中"、composer 永久禁用、轮询空转 | **方案一** |
| RUN-05 | 中 | `src-tauri/src/agent_bridge/stream.rs:110-116` | 事件流正常关闭但未见 `agent_end` 时，半截文本按成功返回 → run 标 `completed`、截断回复作为完整消息落库，无任何错误标记 | **方案一**（附带） |
| RUN-06 | 低 | `src-tauri/src/store/approvals.rs:158-163` | `decide_approval_request` 无 `AND status='pending'` CAS，已决策审批记录可被反复改写 | **方案一**（附带） |
| RUN-07 | 低 | `src-tauri/src/shadow_review/maintenance.rs:90-139`；`lib.rs:148` | 启动恢复后台线程与启动瞬间发出的首个 run 竞态：新 run 可能被误判"中断"置 cancelled 并捕获错误 after 快照（窗口数百毫秒，后续正常路径会覆盖） | **方案一**（附带：启动时先同步快照 run id 集合） |

## 三、配置文件读写（→ 方案三）

| ID | 严重度 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| CFG-01 | 中 | `src-tauri/src/agent_providers.rs:744-749` + 写路径 `:311,:494,:559,:571,:577` | `read_json` 损坏即静默重建 `{}`，写路径以其为基底整文件覆盖 → models.json 损坏后任意一次保存清空全部自定义 provider / baseUrl override。对照：`auth_store::read()` 严格报错是正确范本 | **方案三** |
| CFG-02 | 中 | `agent_providers.rs:751-761`；`auth_store.rs:76` | 并发读-改-写丢失更新：进程内无 Mutex，tmp 文件名仅含 pid（两个并发保存互相截断）；跨进程 GUI 与 CLI `auth login` 互相覆盖 | **方案三** |
| CFG-03 | 中 | `src-tauri/src/approval_rules.rs:24-28,43-47`；`commands/approvals.rs:37-50` | approval_rule.json（用户预期手编）损坏即静默重建 → 一次 GUI"允许"无声删除既有 deny 规则（安全语义数据丢失）；`save_approval_rule` 对 `access` 字段无 `read\|write` 枚举校验 | **方案三** |
| CFG-04 | 低 | `agent_providers.rs:564-583` | `delete_custom_provider` 缺少 id 防护：传 `"future"` 会删除 FutureGene 登录凭据；传内建 id 删除其 override（前端目前不会触发，后端防线缺失） | **方案三** |

## 四、前端逻辑（→ 方案二）

| ID | 严重度 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| FE-01 | 高 | `src/features/agent/useAgentThreadState.ts:76,182-187,269,294,391` | 单一共享 `streamTimerRef` + `clearStreamTimer` 无条件清除：线程 A 的回复返回时清掉线程 B 正在用的流式定时器 → B 的流式气泡永久冻结 | **方案二** |
| FE-02 | 中 | `useAgentThreadState.ts:622-631` + `AppShell.tsx:509-514` | pendingPrompt 的 id 编入 threadId 但消费时不校验：新会话首条消息在加载窗口期切线程会连附件发进错误会话并持久化 | **方案二** |
| FE-03 | 中 | `AgentThread.tsx:108-135` + `RunInspectPanel.tsx:176` + `useAgentThreadState.ts:174-177` | `recover-run` 事件路径绕过全部发送防护：run 进行中点"重试"制造垃圾 failed run + 失败气泡；`pendingPrompt` 撞 `sendingRef=true` 时在标记 consumed 后静默丢失 | **方案二** |
| FE-04 | 中 | `components/layout/hooks/useAgentConnection.ts:60-107,119-125` + `agent_bridge/client.rs:35-41` | 10s 轮询无 generation guard 且 connect 无超时：迟到失败响应覆盖新成功状态 → 清空模型列表、重置用户选中模型、闪断连横幅 | **方案二**（附带） |
| FE-05 | 低 | `components/layout/hooks/useThreadStore.ts:79-92` | `refreshThreadRunStatuses` 的 Promise.all 无 catch → 一败俱败 + unhandled rejection | **方案二**（附带） |
| FE-06 | 低 | `useThreadStore.ts:94-112` | `refreshStore` 无并发 guard，慢响应用陈旧列表覆盖（已删线程短暂复活）；同文件其他加载均有防护 | **方案二**（附带） |
| FE-07 | 低 | `components/layout/hooks/useApprovals.ts:34-41` | 出错时静默清空审批队列 → 审批卡片闪没闪回，紧邻 composer 有误触风险；另有线程切换双倍请求（浪费） | **方案二**（附带） |
| FE-08 | 低 | `useAgentThreadState.ts:310,336-348` | abort 路径状态仅存内存；abort 后立即切线程时已生成的部分文本永久丢失 | 记录（方案二可选） |
| FE-09 | 低 | `src/features/agent/agentActivity.ts:153-157,181` | `tool_end` 缺 tool_id 时的 fallback 会产生重复活动行；`toolcall_delta` 归属依赖单一 activeToolCallId，仅顺序流下安全（当前 agent 实为顺序执行且带 tool_id，仅 legacy 数据踩到） | 记录 |
| FE-10 | 低 | `src/features/agent/attachments.ts:177-180,199-208,254` | 文本截断按字节/UTF-16 code unit 切割可产生乱码注入 prompt；PDF 提取无页数上限 | 记录 |
| FE-11 | 低 | `attachments.ts:280-285` | 缩略图 key 用 32 位 djb2 哈希，路径碰撞显示错误缩略图 | 记录 |

## 五、模块结构 / 重复代码

| ID | 价值 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| STR-01 | 高 | `src/features/agent/useAgentThreadState.ts`（910 行） | 三块独立职责混一文件：粘性滚动子系统、264 行 `handleSend` 单函数、263 行模块级纯函数（后者因无法单测而零覆盖） | **方案二** |
| STR-02 | 高 | `src-tauri/src/agent_providers.rs`（1251 行，含 463 行内联测试） | 5 类职责混杂；`resolve_future_platform_url`（:594-613）被 skills/future_login/debug 跨模块调用，属平台层关注点错位；45 行显示名 match 表与内置目录摘要可拆 | **方案三** |
| STR-03 | 中高 | `src/components/layout/ActivityRail.tsx`（826 行） | 6 个组件 + 布局混一文件；dropUp 翻转逻辑在 `:683-690` 与 `:744-753` 逐字重复；导航按钮长 className 重复 6 次 | **方案五** |
| STR-04 | 中低 | `src-tauri/src/store/review_snapshots.rs`（651 行） | 长但单一领域，自然切线为 changesets/snapshots 两文件，收益有限 | 记录 |
| STR-05 | 中 | `src/lib/useBuildInfo.ts:1` | 唯一一处 lib → integrations 反向依赖，违反 lib 层零依赖约定 | **批次0**（移至 `integrations/tauri/`） |
| STR-06 | 低中 | `src/features/artifacts/ArtifactsPanel.tsx:16` | artifacts 跨 feature 导入 agent 的 `READ_SOURCE_MAX_BYTES` 常量 | **方案五**（常量下沉） |
| STR-07 | 低 | agent → markdown 5 处单向导入 | markdown 的引用解析子系统实质是共享基础设施；无循环，暂可接受 | 记录 |
| DUP-01 | 中 | `ActivityRail.tsx:683-690` vs `:744-753` | dropUp 菜单定位逻辑逐字重复 → 抽 `useDropUpMenu()` | **方案五** |
| DUP-02 | 低中 | `ArtifactsPanel.tsx:117` vs `ReviewPanel.tsx:407` | `formatBytes` 双实现 → `lib/format.ts` | **方案五** |
| DUP-03 | 低 | 全 src 18 处 | `error instanceof Error ? error.message : String(error)` → `lib/errors.ts` 的 `errorMessage()` | **方案五** |
| DUP-04 | 低 | `RunInspectPanel.tsx:65-72` vs `ArtifactDetailPanel.tsx:100-107` | 返回按钮 JSX 逐字重复 → `ui/BackButton` | **方案五** |
| DUP-05 | 中 | `ui/IconButton` vs `ArtifactsPanel.tsx:150,159`、`RunsPanel.tsx:163-165`、`ActivityRail.tsx:334,760` 等 | 内联 icon-button 类名与现成组件并存 → IconButton 加 size/tone 变体推广 | **方案五** |
| DUP-06 | 中 | `agent_providers.rs:751-761` vs `auth_store.rs:63-91` | Rust 原子写文件双实现（tmp 命名还不一致）→ 公共 `atomic_write_json` | **方案三** |
| DUP-07 | 中 | `store/review_snapshots.rs:365-369`、`store/markdown_refs/resolve.rs:240-244,263-267,286-290,312-316` | 列限定 hack 5 处逐字重复 → `store/util.rs` 加 `qualify_columns()` | **方案三**（附带） |
| DUP-08 | 低中 | `store/` 8+ 模块 | `*_COLUMNS` + 按序号手工 `*_from_row` 样板，加列有列序漂移风险；可选声明式宏 | 记录 |

## 六、文档一致性

| ID | 类型 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| DOC-01 | 实现偏离设计 | `schema.rs`（4 张死表）vs `ER.md §4.14-4.17,§5` vs `PRODUCT.md §8` | Skill 实际走平台商店路线（`skills.rs` 下载/安装/卸载），撞上 PRODUCT.md 明列非目标"不做 Skill marketplace"；`data_sources`/`data_credentials`/`skills`/`skill_enablements` 4 张表零 CRUD 沦为死表 | **方案四**（已拍板：承认商店路线，回写 PRODUCT/ER，删死表） |
| DOC-02 | 实现偏离设计 | `ER.md §4.19` vs `store/markdown_refs/extract.rs:112`、`resolve.rs:53-93` | `target_type` 实际存短名（tool/approval/review/research/file），文档定义长名且多出 data_source/skill 两个未实现类型 | **方案四**（顺带回写） |
| DOC-03 | 文档过期 | `ER.md §4.2/§4.4/§5` vs `schema.rs:32-33,64`、`store/app_settings.rs:40-45` | threads 缺 `thinking_level`/`agent_session_id`，runs 缺 `error_type`，app_settings 缺 4 个键 | **方案四**（顺带回写） |
| DOC-04 | 文档过期 | `gui/CLAUDE.md:25-31` vs `src/` 实际 | 漏 `features/remote`、`features/skills`、`integrations/skills`、`src/i18n`；layout 组件与 hooks 清单过期（3→8 个） | **方案四**（顺带回写） |
| DOC-05 | 文档过期 | `APPROVAL_PLAN.md:3` | 状态行仍写"待实现"，v2 实际已实现（SANDBOX_PLAN R1-R3 ✅、代码齐全） | **方案四**（顺带回写） |
| DOC-06 | 文档过期 | `PRODUCT.md §5.2` vs `ActivityRail.tsx:64,102-106` | Research/Data/Skill 导航入口已隐藏未回写；Remote 入口（非 release 显示）完全未提 | **方案四**（顺带回写） |
| DOC-07 | 轻微 | `remote-control-status.md` vs `app_settings.rs:42-44` | 文档写 camelCase 设置键，实际 DB 键为 snake_case | 记录 |
| DOC-08 | 轻微偏离 | `src/styles/globals.css:5-6,33,61,68` | 硬编码 `#172033`/`#f6f7f9`（与 token 同值重复定义，改色会漂移）；滚动条色 `#c8ced9`/`#aeb7c6` 游离于 token 体系外 | **批次0**（简单修复） |

## 七、代码卫生（整体结论：非常干净）

lint/tsc/clippy 全绿；无硬编码密钥、无凭据入日志、无项目约定违规；auth_store/future_login 凭据处理达到范本水准。仅：

| ID | 严重度 | 位置 | 问题 | 处置 |
|---|---|---|---|---|
| HYG-01 | 低 | `src-tauri/src/remote/mod.rs:263` | 死变量 `let _pair = pair_id.to_string()` | **批次0** |
| HYG-02 | 低 | `remote/mod.rs:181`、`agent_supervisor.rs:72` | 两条 info 级启动日志与错误日志混用 `eprintln!` | 记录 |

## 经核实无问题的重点项（供后续审查参考）

- SQL 注入：`store/` 全参数绑定，`format!` 仅拼常量列名；schema 迁移幂等有测试
- cleanup 防误删规则正确；`clear_finished_runs` 删除顺序 FK 安全且单事务
- auth.json：0600 + 原子写 + 严格读（除 CFG-02 的 tmp 命名/无锁）
- shadow review 并发设计自洽（workspace 锁 + prompt guard；进程内锁，多 GUI 实例为边缘场景）
- future_login：浏览器 URL scheme 白名单、key 落盘后才报 authorized
- 前端：`futureEvents` 订阅对称无泄漏；`usePolling`/`useAsyncResource` 自身实现正确（问题在调用方选型）；`upsertStreamingPreview` 收敛设计正确；事件 sequence 排序与投影逻辑有测试覆盖
- 三份计划文档（MEMORY_PLAN / ATTACHMENT_PLAN / remote-control-status）的"已实现"声明经代码核实全部属实；COLOR.md token 清单与 tailwind.config.js 完全一致
