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

## 当前能力（L0）
- GUI 里聊天 → 手机/网页**实时镜像看到**。
- 手机/网页 → **列会话 / 新建会话 / 发 prompt** → GUI 里**出现线程 + 落库 + 显示**，并镜像回手机。（双向闭环。）

## 边界 / 未做
- **无鉴权**（L0）：仅本地/受控网络，**禁公网**。GUI 后端连 `nats://…:4222`（客户端口）；网页连 `ws://…:8080`。
- **P1 硬化未做**：事件目前是 `{type,data}` 核心 pub/sub，无 `run_id`/`idx`/去重/JetStream 回放 → 弱网/重连不健壮（见 plan §4.1）。
- **手机端 abort / 审批未做**：审批目前在 GUI 里点；简单对话不涉及。
- **L1 鉴权未做**：签发服务在 `future-server`（见 `future-server/docs/remote-control.md`）。
- 并发：GUI 正在跑某会话时手机又发同一会话 → 被 `PromptSessionGuard` 拒（"already running"）。

## 怎么跑（L0）
```bash
# 1) NATS（首次还需按 deploy/nats/README.md 建 EVT_DEVPAIR 流）
cd deploy/nats && docker compose up -d
# 2) GUI：Remote → 启动远程（nats://localhost:4222 / DEVPAIR）
make run-gui
# 3) Web 验证端
cd remote/web && python3 -m http.server 5500   # 浏览器开 http://localhost:5500 → 连接 ws://localhost:8080 / DEVPAIR
```

## 下一步
1. **P1**：agent `run_id`/`idx` 集中盖章 + 当前轮缓冲 + `get_events_since`（plan §4.1）；事件改 JetStream 发布（`Nats-Msg-Id` 去重）+ 网页 consumer 回放（选轮 + 去重）。
2. 手机端 **abort / 审批** 转发。
3. **L1 鉴权**：future-server 签发服务 + 扫码配对 + scoped creds（plan §6 P5、auth）。
