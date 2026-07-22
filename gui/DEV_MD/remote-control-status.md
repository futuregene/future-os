# 远程控制 · 实现进度（L0 / GUI 内嵌）

> 设计真源：[plan](remote-control-plan.md) · [relay](remote-control-relay.md) · [auth](remote-control-auth.md)。
> 本文记录**已实现**的部分、怎么跑、下一步。当前处于 **L0（无鉴权，仅本地/受控网络）**。

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

## 当前能力（L0）
- GUI 里聊天 → 手机/网页**实时镜像看到**。
- 手机/网页 → **列会话 / 新建会话 / 发 prompt / 中断 / 审批 / 引导 / 排队 / 切换模型 / 切换思考等级 / 重命名** → GUI 里**出现线程 + 落库 + 显示**，并镜像回手机。（双向闭环。）
- **弱网/重连健壮**：中途加入某轮补齐前缀；断线重连自动重放；同一事件多次到达按 `(runId,idx)` 去重。

## 边界 / 未做
- **无鉴权**（L0）：仅本地/受控网络，**禁公网**。GUI 后端连 `nats://…:4222`（客户端口）；网页连 `ws://…:8080`。
- **run 边界仅对 prompt 精确（P1 review C2，已知，暂缓）**：`start_run` 只在初始 `prompt` 调；流式中 `steer`/`follow_up` 复用同一 `run_id`（会多发一个 `agent_start`），手机端会把追加回答并进同一气泡。当前手机/网页只发 `prompt`，不受影响；真手机 App 阶段再改 run 模型。
- **agent_start 非严格 idx 0（review M3）**：客户端以 `runId` 变化判新轮，不依赖 idx 0，故无实际影响。
- **超长轮（>20000 事件）回放丢前缀**：`truncated` 已提示，不静默；必要时再调大或按时长裁剪。
- **L1 鉴权未做**：签发服务在 `future-server`（见 `future-server/docs/remote-control.md`）。
- 并发：GUI 正在跑某会话时手机又发同一会话 → 被 `PromptSessionGuard` 拒（"already running"）。
- 回复 inbox 仍用默认 `_INBOX.>`（L1 时需收敛到 `p.{pairId}.rep.{device}`）。
- 附件/文件列表等右侧面板功能未做（后续开发）。

## 怎么跑（L0）
```bash
# 1) NATS（首次还需按 deploy/nats/README.md 建 EVT_DEVPAIR 流）
cd deploy/nats && docker compose up -d
# 2) GUI：Remote → 启动远程（nats://localhost:4222 / DEVPAIR）
make run-gui
# 3) Web 验证端（GUI 启动远程后自动在 localhost:8022 起服务，直接开浏览器即可）
open http://localhost:8022  →  连接 ws://localhost:8080 / DEVPAIR
```

> **JetStream 回放的前提**：事件用 JetStream 发布，重连回放依赖 `EVT_DEVPAIR` 流已建（见上 `deploy/nats/README.md`）。未建流时实时仍工作（core 订阅者照收），仅无持久化/跨轮回放。

## 下一步
1. **L1 鉴权**：future-server 签发服务 + 扫码配对 + per-pairId subject 权限 + scoped creds（plan §6 P5、auth）。
2. **真手机 App / PWA**：替代 `remote/web` 调试页（历史 / 流式 / 审批 / abort UI 复用）。
3. **reply inbox 收敛**：从默认 `_INBOX.>` 到 `p.{pairId}.rep.{device}`（为 L1 subject 权限铺路）。
4. **右侧面板**：附件上传/预览、文件列表、工具调用详情。
5. **run 边界对齐**：`steer`/`follow_up` 路径也分配独立 `run_id`（P1 review C2）。
