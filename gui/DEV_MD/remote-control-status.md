# 远程控制 · 实现进度（L0 / GUI 内嵌）

> 设计真源：[plan](remote-control-plan.md) · [relay](remote-control-relay.md) · [auth](remote-control-auth.md)。
> 本文记录**已实现**的部分、怎么跑、下一步。当前处于 **L0 简单配对**：共享接入 token 提供接入控制 + 随机 pairId 命名分区，**无**服务端逐 subject 强制隔离（那需 Phase 2 JWT）；仅本地/受控网络，禁公网。

## 架构决策（本阶段确定）
- **Bridge 内嵌 GUI Tauri 后端**（不是独立进程）。原因：远程对话要落 GUI 的 SQLite 并在页面实时显示，而 GUI 的持久化是"谁发起谁落库"（`agent_prompt` → `stream.rs` 落 `run_events`）。内嵌后，手机命令走 GUI 现有 prompt 路径，天然落库+显示+镜像。
- 独立 `remote/` crate 保留为**传输验证骨架 / headless 参考**，非运行必需。

## 已完成
- **NATS dev 部署** `deploy/nats/`（docker compose + `nats.conf`；JetStream + WebSocket）。
- **独立 Bridge 骨架** `remote/`（Rust `async-nats`）—— walking-skeleton，验证 命令/事件/gRPC 桥接链路。
- **Web 验证端** `remote/web/index.html`（`nats.ws`；会话列表 / 新建会话 / 发送 / 流式渲染 / 从事件 subject 自动识别会话）。
- **GUI 内嵌远程**：
  - 侧栏 **Remote** 入口 + 页面（启停、NATS 地址、pairId、状态）：`gui/src/features/remote/{RemoteView,remoteClient}.tsx`、`ActivityRail.tsx`、`AppShell.tsx`。
  - 后端 `gui/src-tauri/src/remote/mod.rs` + `commands/remote.rs`：连 NATS、事件 tap、命令订阅/路由。
  - 参数存 `app_settings`（`remoteEnabled`/`remotePairId`/`remoteNatsUrl`，默认 `nats://localhost:4222` / `DEVPAIR`）。
  - **事件 tap**：`stream.rs::collect_agent_response` 每条事件 → `remote::publish_event` → `p.{pairId}.evt.{session}`。
  - **命令路由**：订 `p.{pairId}.cmd.>`；`list_sessions`/`get_messages`/`new_session` 读写 GUI store；`prompt` 复刻前端 `handleSend`（建 thread/run → append user → `agent_bridge::agent_prompt` → append assistant）→ 落 SQLite + tap 镜像 + `emit_remote_activity` 刷新前端。
  - `store::find_thread_by_agent_session`（新查询）、`emit_remote_activity`（新事件）。

- **P1 硬化（已完成）**：
  - agent `SseBroadcaster` 单点盖章 `run_id`（每轮，`is_streaming` false→true 边）+ 单调 `idx`（轮内），单锁保序；当前轮缓冲（`MAX_RUN_EVENTS=20000`）；`events_since` 返回 `min_idx` 供溢出检测；`get_events_since` 命令（+`truncated` 标志）。
  - GUI 中间区监听 `remote-activity`，远程驱动当前会话时实时刷新（不只侧栏转圈）。
  - GUI Bridge 事件走 **JetStream 发布** + `Nats-Msg-Id={session}:{runId}:{idx}`（幂等去重 + 落 `EVT_*` 流做回放；未建流优雅降级为实时投递）。
  - 网页：选中会话加载完整历史（`get_messages`）+ 回放进行中轮（`get_events_since`，`(runId,idx)` 去重）+ **重连自动重放**（`nc.status()` → 重跑 selectSession）+ 溢出提示。

- **Phase 1 手机端核心交互（已完成）**：
  - **abort**：转发到 `agent_bridge::abort_session`，手机端中断进行中的 run。
  - **approval_decision**：转发到 `agent_bridge::decide_approval`，手机端审批（批准/拒绝）。
  - **steer / follow_up**：转发到 `agent_bridge::steer_session` / `follow_up_session`，手机端运行中注入/排队。
  - **presence 心跳**：Bridge 每 20s 写 KV `pairs`（`{online, lastHeartbeatTs, sessions: [{id, streaming}]}`），停止时主动删除 key；Web 端 watch + 本地 10s 定时器兜底，60s 无心跳自动标离线。
  - **Web 客户端 HTTP 服务器**：GUI 启动远程时在 `127.0.0.1:8022` 起 HTTP 服务，实时从磁盘读 `remote/web/`（改完刷新浏览器即可）；停止远程时同步关闭。
  - **历史消息修复**：`get_messages` 从 agent JSONL 取（所有会话真源，TUI/CLI 会话也有），不再只读 GUI store；Web 端处理 LLM Message content-block 数组形状。
  - **重复回复修复**：只在 presence 显示 streaming 时才 backfill 当前 run，已完成的 run 历史里已有完整回复。
  - **停止崩溃修复**：`stop()` 里 `tokio::spawn` 在主线程 panic（无 runtime），改用 `tauri::async_runtime::spawn`。
  - **模型选择 / 思考等级 / 会话重命名**：
    - Bridge 新增路由：`list_models`（ agent 侧 `list_models` 命令）、`get_state`、`set_model`、`set_thinking_level`、`set_session_name`（重命名后同步 GUI store）。
    - Web 端：chat header 显示模型下拉 + 思考等级下拉（off/low/medium/high）+ 重命名按钮；切换会话时从 `get_state` 拉当前值填充；重命名同步更新列表标题。

- **Phase 1 简单配对 + 审批归属（已完成，落地见 [auth §8/§9](remote-control-auth.md)）**：
  - **单一 paired 模式（dev / 无鉴权直连已移除）**：`RemoteStartInput { access_token(必需), pair_id?(覆盖), device_id? }`。NATS 地址不再手填——bridge `nats://host:4222` 和 web `ws://host:8080` 从 `current_platform_url()`（环境切换逻辑）取 host 派生,协议端口固定。**pairId 解析** = 显式覆盖 > 已持久化配对的 pairId > 随机生成——已配对桌面重启**复用**同一 pairId（配对码稳定），首次才随机；deviceId 同理复用。
  - **简单配对凭证** `remote/pairing.rs`：复用或随机的 pairId + 共享 NATS 接入 token + 每设备 deviceId，凭证落 `~/.future/remote_pairing.json`（0600）；配对码 = base64url JSON（10min 窗口，含 `wsUrl` + `pairId` + `token`——web 粘码即得全部连接信息,无需手填任何输入）。base64url 编解码无依赖；Rust 解码仅 `cfg(test)`（客户端在 JS 解码，浏览器无 Tauri 桥）。
  - **GUI 配对 UI**：Remote 页只保留接入 token 输入 + pairId 可选覆盖 +「配对并启动」+ 配对码显示/复制 + 已配对/解绑（`remote_pairing_status` / `remote_unpair` 命令）。URL 框已移除（地址内置派生）。
  - **Web 配对**：只保留配对码粘贴框,粘码即得 `{wsUrl, pairId, token}`；connect 带 token + `inboxPrefix = p.{pairId}.rep.{deviceId}` —— **回复 inbox 已收敛**到 pair 命名空间,不再用默认 `_INBOX.>`。URL 框已移除（ws 地址由配对码提供）。
  - **NATS dev token auth**：`deploy/nats/nats.conf` 启用共享接入 token（client 4222 + websocket 8080 同值），README 同步。conf 保留 no-auth 注释替代方案（仅 NATS 层；**GUI/web 已不支持 no-auth 连接**——dev 模式已移除）。
  - **审批 session 归属校验**（agent crate）：`ApprovalGate::decide` 加 `session_id` 参数，与 `PendingApproval.session_id` 比对，跨 session 拒绝（auth I1 例外，防 `entry_id` 泄漏越权批准）；2 个单测。
  - **publish fire-and-forget**：`publish_event` 原先 `await` JetStream ack（与自身注释矛盾；无匹配流时会阻塞 agent 事件循环）改为 `tokio::spawn` 发包。故 Phase 1 **无需为每 pair 建流**——实时走 core pub/sub，重连/中途加入走 `get_events_since`（agent 侧 buffer，与 NATS 流无关）。
  - **JetStream consumer 升级（原计划 1.8）延后**：流 provision + web `deliver=all` consumer 回放推到简单配对之后；当前 core sub + backfill 已覆盖重连/中途加入。
  - **安全边界写实**：简单配对 = 接入控制（共享 token）+ 随机 pairId 命名分区，**不提供**服务端逐 subject 强制隔离（那需要 Phase 2 JWT；全局 token 下恶意多租户隔离不成立，见 auth §8.9）。

## 当前能力（L0）
- GUI 里聊天 → 手机/网页**实时镜像看到**。
- 手机/网页 → **列会话 / 新建会话 / 发 prompt / 中断 / 审批 / 引导 / 排队 / 切换模型 / 切换思考等级 / 重命名** → GUI 里**出现线程 + 落库 + 显示**，并镜像回手机。（双向闭环。）
- **弱网/重连健壮**：中途加入某轮补齐前缀；断线重连自动重放；同一事件多次到达按 `(runId,idx)` 去重。

## 边界 / 未做
- **简单配对接入控制**（L0，无服务端 subject 强制）：仅本地/受控网络，**禁公网**。GUI 后端连 `nats://…:4222`（客户端口）；网页连 `ws://…:8080`。无 dev/无鉴权直连路径（已移除）。
- **run 边界仅对 prompt 精确（P1 review C2，已知，暂缓）**：`start_run` 只在初始 `prompt` 调；流式中 `steer`/`follow_up` 复用同一 `run_id`（会多发一个 `agent_start`），手机端会把追加回答并进同一气泡。当前手机/网页只发 `prompt`，不受影响；真手机 App 阶段再改 run 模型。
- **agent_start 非严格 idx 0（review M3）**：客户端以 `runId` 变化判新轮，不依赖 idx 0，故无实际影响。
- **超长轮（>20000 事件）回放丢前缀**：`truncated` 已提示，不静默；必要时再调大或按时长裁剪。
- **L1 鉴权部分完成**：简单配对（共享接入 token + 复用/随机 pairId 分区，Phase 1）已做；**JWT 签发服务 + 服务端逐 subject 强制隔离未做**（在 `future-server`，见 `future-server/docs/remote-control.md` 与 [auth §9 Phase 2](remote-control-auth.md)）。简单配对**禁公网**（全局 token 无恶意多租户隔离）。
- 并发：GUI 正在跑某会话时手机又发同一会话 → 被 `PromptSessionGuard` 拒（"already running"）。
- **回复 inbox 已收敛**：客户端 connect 设 `inboxPrefix = p.{pairId}.rep.{deviceId}`，Bridge reply 跟随该 inbox，不再用默认 `_INBOX.>`。
- **JetStream 回放延后**：事件目前走 core pub/sub（无 NATS 流），重连/中途加入靠 `get_events_since`；`EVT_{pairId}` 流 provision + web consumer 回放延后（见 Phase 1 简单配对段）。
- 附件/文件列表等右侧面板功能未做（后续开发）。

## 怎么跑（L0 / 简单配对）
```bash
# 1) NATS（nats.conf 默认启用共享接入 token = devpairingtoken；client+ws 同值）
cd deploy/nats && docker compose up -d
#    （要纯无鉴权 NATS 测试：注释掉 nats.conf 里两个 authorization 块——仅限其他 NATS 客户端；
#     GUI/web 已不支持 no-auth 连接，dev 模式已移除。）
# 2) GUI：Remote → 填接入 token（devpairingtoken），可选填 pairId 覆盖 →「配对并启动」
#    NATS 地址自动从平台环境派生（dev build → test.future-os.cn,生产 → future-os.cn）。
#    配对成功后页面显示配对码，点复制。
make run-gui
# 3) Web 验证端（GUI 启动远程后自动在 localhost:8022 起服务）
open http://localhost:8022
#    粘贴配对码 → 点「连接」。配对码含 wsUrl + pairId + token,无需手填任何地址或 token。
```

> **无 NATS 流也能跑**：事件走 core pub/sub，重连/中途加入靠 `get_events_since`（agent 侧 buffer）。JetStream 流 + consumer 回放延后。

## 下一步
1. **L1 鉴权 Phase 2（最后做）**：future-server 签发服务（`/pair/nonce` + `/pair/claim` + `/pair/revoke`）+ scoped user JWT + **服务端逐 subject 强制隔离** + 流/桶生命周期迁签发服务 + 短期 JWT 刷新/撤销 + 已链接设备列表（[auth §9](remote-control-auth.md)）。
2. **JetStream 回放**：Bridge 自建 `EVT_{pairId}` 流 + web 升级 `deliver=all` consumer（替代当前 core sub + `get_events_since`；当前已可回放/重连,consumer 是增强）。
3. **真手机 App / PWA**：替代 `remote/web` 调试页（历史 / 流式 / 审批 / abort UI 复用）。Web 验证端现已是完整远程客户端——会话列表 / 新建 / 发 prompt / 中断 / 审批 / 切模型 / 切思考 / 重命名 / presence / 简单配对接入控制。
4. **`pairId` 持久化**：bridge 每次 start 时保存 pairId 到 app_settings 在 GUI 发生时机异步,须确保 GUI 重启后 pairId 与配对码一致，可考虑在 save_creds 时同时写回。
5. **run 边界对齐**：`steer`/`follow_up` 路径也分配独立 `run_id`（P1 review C2）。
6. **附件 / 文件列表 / 右侧面板**：后续开发。
7. **web 移动端适配 / UI 精修**：`remote/web/index.html` 目前以桌面浏览器为主，可进一步调整触屏交互和布局。
