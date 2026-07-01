# future-os 手机远程控制（Remote Control）实施计划

> 目标：手机/网页作为**瘦客户端**远程接管桌面上正在进行的 agent 会话——**会话状态与工具都在 PC/Mac 本机执行，客户端只做展示与操控**。
> 文档集：**本文=总纲**（唯一真源：决策/术语/命名/阶段）· [消息中枢设计（NATS）](remote-control-relay.md) · [鉴权/配对/连接](remote-control-auth.md)。

---

## 0. 决策与术语（唯一真源 · 子文档一律引用此表）

### 0.1 已锁定决策
| 项 | 决策 |
|---|---|
| 方向 | 云端**消息中枢**转发 + 瘦客户端（先 Web 验证端、后原生 App）；工具/会话在本机 |
| 中间件 | **NATS + JetStream**（不自研路由/扇出/回放） |
| 隔离 | **账号 + 设备** = 单 NATS account + 按 `pairId` 的 **JWT subject 权限（服务端强制）**；升级路径 account-per-user 硬隔离 |
| 绑定 | **严格 1:1**（一 PC ↔ 一 App）；可逆到 1:N（登记/权限改动） |
| 事件流 | **每 pair 一流** `EVT_{pairId}`（**签发服务在配对时创建**）；bounded `MaxAge`/`MaxBytes`/`MaxMsgSize`/`dupe-window`；**不主动 purge**，旧轮靠 GC 老化 |
| 运行标识 | 每事件带 **`run_id`（每次用户 run 唯一，外层只分配一次）+ `idx`（run 内单调，集中盖章）** |
| 回放/去重 | 客户端**按最近 `run_id` 选当前轮渲染 + 按 `(run_id,idx)` 去重**（主）；JetStream `Nats-Msg-Id` + 显式 `dupe-window`（辅）。**取代主动 purge，消除慢消费者竞态** |
| 命令语义 | 流式命令（prompt/steer/follow_up）reply = **accept-ack**，完成看事件流 `agent_end`；unary（get_state/get_messages/list_sessions…）reply = 真实结果 |
| 命令投递/幂等 | cmd 订 `p.{pairId}.cmd.>`（多 token）+ **queue group** `bridge.{pairId}` + 每命令 **spawn**（防 HOL）+ 本机单实例锁 + **单飞**（同 `RpcCommand.id` 合并 in-flight/已完成） |
| 历史拉取 | `get_messages` **分页**（cursor/offset+limit），单页 < NATS `max_payload`；历史是 **LLM Message 形状**，用**独立历史 renderer**（非 StreamEvent renderer） |
| 审批 | decision 命令形态不变；**加 agent 侧 session 归属校验**（`ApprovalGate.decide` 收 `session_id`，与 `PendingApproval.session_id` 比对）；审批门**无 timeout**→ 客户端离线时 Bridge 自动 `abort`/`cancel_session`（见 §5） |
| 鉴权节奏 | **先 App 后鉴权**；L0→L1→L2（见 0.3）；L1 为可公开发布下限；L1 扫码入网不做 App 账号二次校验，但**需最小签发服务**发 scoped creds |
| 撤销 | 撤销在**下次重连/刷新生效（≤TTL）**；**已建连接需 server kick 才断**（不是"即时"）；L1 默认短期 JWT+刷新 |
| 缓冲上限 | agent `run_events` 设 `max_events`/`max_bytes`；流设 `MaxBytes`/`MaxMsgSize`/`MaxAge`；超大事件截断 + 指针指向分页 `get_messages` |
| presence | **每 session** 状态（streaming/idle）+ **每 session 的 `currentRunId`**；不用单一 activeSession（会话可并发） |
| 推送 | 退后（P6+） |
| 客户端 | 开发期 Web 验证端先行，原生 App 复用同一渲染层；保留会话列表（一 PC 多会话，App 可切换） |
| 远程端原则 | 远程端 = **已配对的本地前端**；不获得超出本地会话的权限；工具权限与审批**完全 follow 本地设置** |
| 数据边界（写实） | 中枢**不存长期历史**，只短期缓存当前/最近若干轮事件（含文本/工具输出/审批内容）；**不做 E2EE**，靠 TLS + NATS auth + TTL/配额 + 运维 |
| 前提约束（固有） | 远控**依赖桌面在线且未睡眠**（工具在本机执行）；桌面睡眠/合盖/断网 → 远控不可用（presence 掉线），**非云故障** |

### 0.2 术语
- **pairId**：一次配对的唯一标识（源自 futureAccount+desktopId），**运行时隔离单元**。
- **会话 session**：一段对话（agent 侧持久化为 JSONL）；一台 PC 可有多个，可**并发**。
- **用户 run / run_id**：**run_id 当且仅当 `is_streaming` false→true 时新分配一次**。因此一次 `prompt`、**运行中注入的 `steer`、运行中排队并被同一循环 drain 的 `follow_up`** 都**共享同一 run_id**（内部虽多次 `agent_start`）；而**空闲时到达的 `follow_up`**（`is_streaming` 已 false → `follow_up()` 直接调 `prompt()` → 再翻真）**是新 run_id**。run 到最终 `agent_end` 结束。
- **Bridge**：桌面上的新组件，对内是本地 agent 的 gRPC 客户端，对外是 NATS 客户端（出站-only）。
- **消息中枢**：= NATS + JetStream 部署（future-os.cn），**只中转**。
- **签发服务**：极小鉴权层，校验 Future 账号后签发 scoped NATS creds、并在配对时创建 `EVT_{pairId}` 流（L1 起需要）。

### 0.3 两条正交阶段轴
**交付轴**：P0–P6（§6）。**鉴权成熟度轴**：
- **L0 无鉴权/粗鉴权**：本地或受控 dev/staging（内网 / 单 token / IP 白名单）；**禁公开无鉴权暴露**。
- **L1 扫码 + scoped creds**：账号+设备隔离（服务端强制）；App 不做账号二次校验；需最小签发服务。**← 可公开发布下限**。
- **L2 加因子**：App 账号登录 + 生物识别 + 吊销/审计。

**映射**：P0–P4 跑在 L0（内部/dev）→ P5 达到 L1（首个可发布）→ P6 到 L2。

### 0.4 Subject / 资源命名（全文唯一）
| 用途 | 名称 |
|---|---|
| 命令（request-reply） | 订阅 `p.{pairId}.cmd.>`；单条 `p.{pairId}.cmd.{session}` |
| 命令回复 inbox | `p.{pairId}.rep.{device}`（客户端 **InboxPrefix**，不用默认 `_INBOX.>`） |
| 事件（JetStream 每 pair 一流） | 流 `EVT_{pairId}`（签发服务配对时建），subject `p.{pairId}.evt.{session}` |
| JetStream 发布 ack | Bridge 订自己的 `$JS.ACK.>`（publish 确认） |
| 目录/presence（KV） | 桶 `pairs`，key `{pairId}`；值含**每 session** 状态（streaming）+ **每 session 的 `currentRunId`** |
| 事件去重键 | `{run_id}:{idx}`（客户端自去重）；NATS `Nats-Msg-Id`=`{session}:{run_id}:{idx}`（辅） |

---

## 1. 市场方案对标
| 产品 | Agent loop | 工具 | 传输 | 借鉴 |
|---|---|---|---|---|
| **Claude Code Remote Control** | 本机 | 本机 | 只出站连云端并路由；多短时凭证 | 出站-only、云端路由、多设备同步、断线重连、桌面失联 ~10min 超时 |
| **Claude Dispatch** | 桌面 | 桌面 | QR 配对，任务不经服务器 | QR 配对交互 |
| **Cursor（My Machines）** | 云端 | 本机 | 工具结果+上下文过云 | 敏感数据留本机的边界 |
| **WhatsApp/Signal 链接设备** | — | — | 扫 QR + 每设备独立密钥、可远程登出 | 配对与持久连接模型（见 auth） |
| **OpenAI Codex Cloud** | 云端 | 云端沙箱 | 委派式 | 参考价值低 |

**共识**：客户端永远瘦；本机进程**只出站**；中枢**只路由**；会话可被终端+客户端同时驱动并同步。

---

## 2. 架构
```
  ┌──────────────┐  nats.ws        ┌─────────────────────┐  async-nats     ┌──────────────┐
  │ 客户端        │◀──────────────▶│ 消息中枢 (NATS+JS)   │◀───────────────▶│ 桌面 Bridge   │
  │ Web / 原生App │ ①req cmd/订 evt│ future-os.cn         │ 订 cmd / 发 evt │ (async-nats) │
  │ 瘦客户端      │ ④收 evt        │ + 签发服务 (L1)      │ KV presence     └──────┬───────┘
  └──────────────┘                └─────────────────────┘             ②③ gRPC(localhost)
                                                                          ┌──────▼──────┐
                                                                          │ Rust Agent  │ 工具/会话在此
                                                                          │ :50051      │
                                                                          └─────────────┘
```
数据流：① 客户端 `request` 发 `RpcCommand` → `p.{pairId}.cmd.{session}` → ② Bridge 转本地 `ExecuteCommand`，agent 执行工具、流式产出 → ③ agent `StreamEvents` → Bridge `publish` → `p.{pairId}.evt.{session}`（`EVT_{pairId}` 流）→ ④ NATS 投递客户端（同会话终端 TUI 走本地 gRPC，天然同步）。

> **前提约束（固有）**：Bridge+agent 必须在本机**在线且未睡眠**（工具在本机执行）。桌面睡眠/合盖/断网 → 远控不可用、presence 掉线——这是**固有前提、非云故障**（对比 Claude Code 桌面失联 ~10min 超时）。产品层需明确"需桌面在线/不睡眠"，App 做**强 presence UX**（§4.4）。

**组件职责**
| 组件 | 职责 | 复用/新增 |
|---|---|---|
| Rust Agent | agent loop、工具执行、会话持久化、事件广播 | 复用，仅 **P1 增量**（run_id/idx 集中盖章 + 当前 run 缓冲 + `get_events_since` + `get_messages` 分页） |
| 桌面 Bridge | 出站连 NATS ↔ 本地 gRPC 双向翻译、配对、重连、presence、单飞 | **新增**（Rust `async-nats`） |
| 消息中枢 | 路由/扇出/回放/水平扩展 | **采用 NATS + JetStream** |
| 签发服务 | Future 账号 → scoped creds；配对时建 `EVT_{pairId}` 流 | **新增但极小**（L1 起） |
| 客户端 | 会话列表、流式渲染、审批、composer、历史 renderer | **新增**（Web 先行；App=RN+`nats.ws`） |

---

## 3. 现有代码库就绪度（已核对源码）

### ✅ 直接复用
1. **多前端并发同一会话**：每 session 独立 `SseBroadcaster`（tokio broadcast，容量 4096，多订阅者）`agent/src/rpc/protocol.rs:128`。
2. **会话并发**：agent 支持并发 session（`new_session` 设计允许"第一段跑时开第二段"，`commands.rs:91`）→ Bridge 须**每 session 独立 pump、每命令 spawn**。
3. **命令 session 无关**：`RpcCommand.session_id`（`proto/future.proto:84`）；`RpcCommand.id ↔ RpcResponse.id` 关联已内建。
4. **历史/状态**：`get_messages` 返回 **LLM Message 形状**（`commands.rs` `ConvertToLLM`，**非 StreamEvent**）；**注意现 `session.rs` `get_messages()` 无参返回全量——分页 `offset/limit` 是新 P1 工作、非复用**（服务端切片后再序列化，保证单页 < `max_payload`）。`get_state`（注意 `is_compacting` 恒 false，`proto:176`，勿据此建 UX）。
5. **审批钩子**：`approval_request` 带 `approval_request_id`；`approval_decision`（`entry_id`=该 id、`mode`、`message`，`commands.rs:62/76`）；`abort` 取消该会话待审批（`commands.rs:57`）。**注意**：`ApprovalGate` 是**全局 HashMap**（`mod.rs:37`）、按 `entry_id` 查，无 pair/session 校验——见 §5 审批归属。
6. **身份体系**：`cli/src/commands/auth.ts` 已有 Future device-flow OAuth。

### ⚠️ 需新增/改造（P1，见 §4.1）
- 事件多点广播（`session_prompt.rs` `on_event`/`on_text`、`tool_event_callback`、`approval.rs`）**无单一盖章点** → 需集中。
- `agent_start` **一次用户 run 会发多次**（follow-up）、`agent_end` 只发一次 → run 边界**不能靠 `agent_start` 事件**，靠 `run_id`。
- agent 绑 localhost 明文（`main.rs:202`）→ 本方案 agent 不对外，**不改**。

---

## 4. 组件设计

### 4.1 Agent 侧改造（P1，最小，可独立交付+单测）

**目标**：为每次用户 run 提供稳定的 `run_id`/`idx` 标识与可补拉的当前-run 缓冲；让客户端能按 run_id 选轮 + 去重。

| 文件 | 改动 |
|---|---|
| `proto/future.proto` `StreamEvent` | 加 `string run_id = 3; int64 idx = 4;`（`reserved` 占位，向后兼容；TUI/GUI 忽略） |
| `proto/future.proto` `RpcCommand` | 加 `int64 since_idx = 140; string run_id = 141;`（供 `get_events_since`，不复用无关字段）；`get_messages` 加分页参数 `int64 offset = 142; int64 limit = 143;` |
| `agent/src/rpc/protocol.rs` `SseEvent` | 加 `run_id: String` + `idx: i64`；`grpc/mod.rs` 的 `StreamEvents` 映射时把二者拷进 proto `StreamEvent` |
| **集中盖章（H2）** | 在 `SseBroadcaster` 上加**唯一广播入口**：**在同一临界区内**完成 `idx=fetch_add(1)` + 盖 `run_id` + 入广播（或经单一 actor 串行化），避免"分配 idx 后、广播前"被并发抢先导致流内 idx 乱序；**所有远程广播点**（`on_event`/`on_text`/`tool_event_callback`/approval）统一走它。客户端按 idx 有序渲染、容忍小窗口乱序 |
| **run_id 生命周期（H1）** | `run_id` 仅在 **`is_streaming` false→true 边缘**分配（`session_prompt`）、`idx` 归零；同一 run 内的多次 `agent_start`、`steer`、同循环 drain 的 follow-up **共享该 run_id**（空闲 follow-up 经 `prompt()` 再翻真 = 新 run）；`ServerSession` 存 `run_id` + `run_events`（上限 `max_events`/`max_bytes`），**run_id 变化时清空**（不按 agent_start 事件清） |
| `agent/src/rpc/commands.rs` | 新增 `get_events_since{session_id, run_id, since_idx}` → `data={run_id, events:[…含 idx]}`（run_id 不匹配返回当前轮全量+新 run_id）；`get_messages` 支持分页，单页 < NATS `max_payload` |

- **去重**：客户端按 `(run_id, idx)` 自去重（主）；Bridge 发布带 `Nats-Msg-Id={session}:{run_id}:{idx}` + 流设 `dupe-window`（辅）。
- **验收**：一次用户 run 内多 follow-up → run_id 不变、idx 全序、客户端不丢头部；Bridge 断开→`get_events_since` 补齐→客户端按 (run_id,idx) 去重无重复；跨轮不误。
- **实现顺序**：proto 扩字段 + 集中盖章是 P1 第一步（当前 `proto:286` 无这些字段、无单一盖章点）。

### 4.2 桌面 Bridge（Rust `async-nats`）
目录（与 `channels/` 平行）：
```
remote/src/
  main.rs         # 加载配置 → 连 NATS → 重连循环
  config.rs       # ~/.future/remote/config.json: nats_url, creds, pairId
  nats_client.rs  # async-nats：queue 订 cmd / JetStream 发 evt / KV presence / 重连
  agent_client.rs # 本地 gRPC（复用 channels/grpc_client.rs 事件映射）
  bridge.rs       # 双向翻译：每 session 长期 pump(先订阅再发命令) + 每命令 spawn + 单飞幂等 + presence
  pairing.rs      # 配对码申领 + QR + creds 落盘（L1）
```
**关键实现约束**（伪代码见 [中枢 §5](remote-control-relay.md)）：
- **每 session 长期订阅 `StreamEvents`，且在发 `execute_command` 之前 await 订阅就绪**——agent 是实时 broadcast、晚订阅会漏早期事件（`agent/src/grpc/mod.rs:175`）；P1 run buffer 兜底。（注意：`channels/feishu` 模板是 prompt 后才订阅、且 per-prompt，本项**是净新增**，非照搬。）
- **cmd 用 `p.{pairId}.cmd.>` + queue group `bridge.{pairId}`**（防多 Bridge 重复执行）+ 本机单实例锁；**每命令 spawn**（防一个慢 `ExecuteCommand` HOL 阻塞其它 session）。
- **单飞**：同 `RpcCommand.id` 合并 in-flight/已完成，超时重试不二次执行。
- **不 purge**：只 publish 事件；旧轮由流 GC 老化。presence 心跳写 KV `pairs`（每 session 状态 + `currentRunId`）。

CLI（仿 `future channel`/`future agent`）：`future remote start | stop | status | pair | unpair`。

### 4.3 消息中枢
见 [remote-control-relay.md](remote-control-relay.md)：命令走 request-reply、事件走每 pair 一流、客户端按 run_id 选轮、无主动 purge。

### 4.4 客户端（先 Web 验证端，后原生 App）
> **开发期先做 Web 验证端**（`nats.ws` 网页），无需先开发 App 即可端到端联调；原生 App 复用同一渲染层。详见 [auth §6](remote-control-auth.md)。

**原生 App 选型（推荐）**：React Native + `nats.ws`——复用 `proto` 派生 TS 类型与 GUI React 渲染。
> **RN 选型风险 → P2 做 connectivity spike（不等 P4）**：`nats.ws` + nkey/creds（ed25519 可能需 polyfill）+ WebSocket + iOS 后台保活 + 证书/代理都可能踩坑。P2 就验四件事：creds 鉴权、custom `inboxPrefix`、JetStream consumer、断线重连。过不了切 Flutter/原生 SDK。

**屏幕/流程**：
| 屏 | 内容 | 复用 |
|---|---|---|
| 配对（L1） | 扫 QR → 换 scoped creds | auth §4 |
| 会话列表 | 该桌面会话（一 PC 多会话）、**每 session 在线/streaming 态**（读 KV `pairs`）；全量列表可 `list_sessions` | KV + `list_sessions` |
| 会话/聊天 | **流式渲染**（StreamEvent renderer，**按 currentRunId 选轮 + (run_id,idx) 去重**）；**历史滚动用独立 history renderer**（分页 `get_messages`，LLM Message 形状） | GUI 两套 renderer |
| 审批 | `approval_request` → 批准/拒绝/取消 → `approval_decision` | 现有钩子 |
| Composer | prompt / steer / abort（reply 是 accept-ack，完成看 `agent_end`） | `streaming_behavior` |

> **强 presence UX**：顶部常驻 在线/重连中/离线（读 `lastHeartbeatTs`）；离线时明确文案"**桌面离线（睡眠/断网），非云故障；请唤醒电脑**"，命令 **fail-fast**；可选"远控期间保持唤醒"（macOS `caffeinate`）。

---

## 5. 安全模型
> 完整方案见 [auth](remote-control-auth.md)。**核心**：**单 NATS account + 按 `pairId` 的 JWT subject 权限（服务端强制）**——某 pair 凭证在 NATS 层就无法访问别的 `pairId`。account-per-user 硬隔离为升级路径。

- **远程端 = 已配对的本地前端**：不获得超出本地会话的权限；工具权限与审批**完全 follow 本地设置**（`permission_level`、审批门），远程不放宽也不默认收紧（"远程更严"为显式可选，默认关）。
- **审批归属（H3）**：`ApprovalGate` 按全局 `entry_id` 查、无 pair/session 校验。1:1 下每 agent=一桌面=一 pair，天然 pair-scoped；仍加校验，且应放在 **agent 的 `ApprovalGate.decide`**（收 `session_id`，与 `PendingApproval.session_id`（`approval.rs:29`）比对不符则拒——一行签名改动，比 Bridge 记账便宜：Bridge 看不到 `entry_id→session` 映射）。故"零新语义"仅指**命令形态**不变。
- **审批门无 timeout（H5 复核）**：`approval.rs` 的 `rx.recv()` **无期限**——手机 RTT 不会触发 agent 超时（好）；但**未应答审批会永久挂住该 session**（block_in_place 线程）直到 `abort`。→ **Bridge 在 presence 显示客户端离线时，对该 session 待审批自动发 `abort`/`cancel_session`**。
- **本机 agent 不对外**：只 localhost 明文；对外全在 NATS creds + WSS/TLS；桌面**只出站**。
- **每设备独立密钥**（私钥不离设备）+ 可撤销持久凭证；QR 不含任何秘密。
- **数据边界（写实）**：中枢**不存长期历史**，只短期缓存当前/最近若干轮事件（含文本/工具输出/审批内容）；**不做 E2EE**，靠 TLS + NATS auth + TTL/配额 + 运维；历史/文件始终在本机（分页 `get_messages`）。

---

## 6. 分阶段路线图
| 交付阶段 | 交付物 | 鉴权 | 验收 | 依赖 | 规模* |
|---|---|---|---|---|---|
| **P0 兜底** | 现有 `channels/` 手机续接；**飞书交互卡回调→`approval_decision`（净新增，非"补"）** | — | 手机在飞书续接+审批 | — | S–M |
| **P1 agent 缓冲** | run_id/idx 集中盖章 + 当前 run 缓冲 + `get_events_since` + `get_messages` 分页（§4.1） | — | 多 follow-up 不丢头部；断线补齐+去重；单测 | — | M |
| **P2 Bridge+Web** | `remote/`（async-nats）+ `future remote` CLI + **Web 验证端** + **RN connectivity spike** | **L0** | 端到端；RN spike 过四关 | P1 | M |
| **P3 中枢落地** | future-os.cn 起 NATS+JetStream（每 pair 流/KV/dupe-window）+ 运维 | **L0（受控 dev/staging，非公开）** | Bridge 与 Web 端受控网络互通 | 与 P2 并行 | S–M |
| **P4 原生 App** | RN+`nats.ws`：列表/双 renderer/审批/composer | **L0（内部）** | 端到端跑通（内部/dev，未公开） | P2+P3 | L |
| **P5 L1 鉴权** | 签发服务（含建流）+ 扫码配对 + scoped creds | **L1** | 服务端强制隔离；**首个可公开发布** | P4 | M |
| **P6 L2 加固** | 账号二因子 + 生物识别 + 吊销/审计 + 弱网 + 推送 | **L2** | 安全评审通过 | P5 | M–L |

\* 单人粗估。**建议顺序**：P1→(P2+P3 并行)→P4→P5→P6。
> ⚠️ **发布纪律**：P2–P4 的 L0 部署必须**受控**（内网/单 token/白名单），**绝不公开无鉴权**；公开发布必须先到 P5（L1）。

---

## 7. 测试与联调
- **本地 L0 NATS**：`docker run nats -js` + websocket，Bridge/Web 端直连端到端（[中枢 §8](remote-control-relay.md)）。
- **Web 验证端替代 App**：`nats.ws` 网页，改一行刷新即测。
- **契约测试**：subject 命名 + `RpcCommand`/`StreamEvent`/`RpcResponse` 编解码 → 三端共享小库 + fixtures。
- **正确性重点**：① 一次 run 多 follow-up 不丢头部、run_id 不变；② Bridge 断线 `get_events_since` 补齐 + `(run_id,idx)` 去重；③ 慢消费者/无 purge 下 deliver=all 只渲染当前轮；④ `get_messages` 分页每页 < `max_payload`；⑤ 审批门无 timeout：验证客户端离线时 Bridge 能自动 cancel 待审批（避免 session 永久挂住）。

---

## 8. 风险登记
| 风险 | 影响 | 缓解 |
|---|---|---|
| 中枢单点/成本 | 全量不可用 | 成熟 NATS（可集群/自建），自研极少 |
| 事件顺序/去重/回放错乱 | 上下文错乱 | run_id 集中盖章 + 客户端按 (run_id,idx) 去重 + deliver=all 选当前轮（无 purge 竞态） |
| 大历史/大事件超 `max_payload` | reply 失败/事件发不出 | `get_messages` 分页 + 事件截断+指针 + 服务端 `max_payload`/流 `MaxMsgSize` 配足 |
| 审批 id 泄漏被越权批准 | 本机风险 | 每 agent=一 pair 天然隔离 + Bridge/agent 侧 session 归属校验 |
| 桌面睡眠/离线 | 用户误解为云故障 | 产品明确"需桌面在线"；强 presence UX + fail-fast + 保持唤醒开关 |
| RN + `nats.ws` 踩坑 | P4 返工 | P2 提前 spike 验四关 |
| L0 误暴露公网 | 越权访问 | 发布纪律：公开必须 ≥L1 |

---

## 9. 待定项
1. **Bridge 落位**：独立 `remote/`+CLI（推荐）vs. 内嵌 agent 的 `--remote-control` 开关。
2. **NATS 部署**：自建集群 vs. Synadia Cloud（评估国内可用性）。
3. **App 框架**：RN+`nats.ws`（推荐）vs. Flutter（P2 spike 后定）。
4. **设备凭证有效期/重复配对策略/是否升级 account 硬隔离**：见 [auth §7](remote-control-auth.md)。

---

## 10. 开工前 checklist（P1/P2 前必须定死）
- [ ] proto 扩字段：`StreamEvent{run_id,idx}`、`RpcCommand{since_idx,run_id,offset,limit}`（`reserved` 占位）。
- [ ] **集中盖章**：所有远程广播点走 `SseBroadcaster` 唯一入口，原子 idx + 盖 run_id。
- [ ] **run_id 外层分配一次**（`is_streaming` 翻真处），非 run_loop 每轮；run 缓冲按 run_id 变化清。
- [ ] **不主动 purge**；流 `MaxAge`/`MaxBytes`/`MaxMsgSize`/`dupe-window` 配齐；客户端按 run_id 选轮 + (run_id,idx) 去重。
- [ ] cmd 订 `p.{pairId}.cmd.>` + queue group + 每命令 spawn + 单实例锁 + 单飞。
- [ ] 回复 InboxPrefix=`p.{pairId}.rep.{device}`（不用默认 `_INBOX.>`）。
- [ ] `get_messages` 分页；客户端**双 renderer**（StreamEvent 流 / Message 历史）。
- [ ] 审批：agent `decide` 加 session 归属校验；审批门**无 timeout**→ 客户端离线时 Bridge 自动 cancel（复用 `cancel_session`）。
- [ ] 权限矩阵：签发服务建/删流，**Bridge 只 publish**（不给 STREAM.CREATE/PURGE/DELETE）。
- [ ] 撤销措辞：≤TTL 生效 + server kick 才断活跃连；nonce 原子消费。

---

## 11. 一句话总结
现有架构（per-session 广播、`id`/`session_id` 内建、`get_messages` 回放、审批钩子、`channels/` 范式、Future 账号）已把 ~80% 地基打好。**中枢采用 NATS/JetStream 不自研**后，新增量为：**agent 的 run_id/idx 集中盖章 + 当前 run 缓冲 + 分页（P1）**、**`async-nats` Bridge（P2）**、**起 NATS + Web 端（P3）**、**原生 App（P4）**、**L1 鉴权+建流（P5，可发布）**。核心 agent 改动小但**必须做对 run_id/盖章/去重**（否则事件会丢头/重复/错轮）。
