# 消息中枢设计（NATS + JetStream）

> 配套：[总纲](remote-control-plan.md)（决策/术语/命名真源见其 §0）· [鉴权/配对](remote-control-auth.md)。
> 定位：**纯中转**——路由、事件回放、presence。不执行工具、不跑 LLM、不存长期历史。转发的 payload 就是现有 `RpcCommand`（下行）与 `StreamEvent`（上行）。所有 subject/资源名以 [总纲 §0.4](remote-control-plan.md) 为准。

---

## 1. 选型：为什么是 NATS + JetStream
拓扑决定选型：桌面 **Bridge 是对等节点**（既订命令、又发事件）、**Rust 写**、**NAT 后只出站**。

| 中间件 | 契合 | 摩擦 | 判定 |
|---|---|---|---|
| **NATS + JetStream** | 官方 **Rust `async-nats`** + `nats.ws`（RN 可用）；request-reply 原生；JetStream 流/回放/去重；单二进制、CNCF | presence 需少量 core 心跳 | ✅ **采用** |
| Centrifugo | 电池全 | 模型是"后端发布、客户端订阅"，无法反向调 NAT 后 Bridge；无官方 Rust 客户端 | 备选 |
| Ably/Pusher/PubNub | 全托管 | SaaS 计费、国内延迟/合规、数据过三方 | 不用 |
| MQTT（EMQX） | 移动端友好 | 整段事件回放弱 | 不用 |

---

## 2. 拓扑
```
  ┌──────────────┐  nats.ws(WS)    ┌──────────────────────┐  async-nats(NATS)     ┌──────────────┐
  │ 客户端 Web/App│◀───────────────▶│ NATS + JetStream      │◀────────────────────▶│ 桌面 Bridge   │
  │ (nats.ws)    │ req cmd/订 evt  │ (future-os.cn)        │ 订 cmd/发 evt/presence│ (async-nats) │
  └──────────────┘                 │ + 签发服务(L1)        │                       └──────┬───────┘
                                    └──────────────────────┘                        gRPC(localhost)
                                                                                    ┌──────▼──────┐
                                                                                    │ Rust Agent  │ 工具/会话在此
                                                                                    └─────────────┘
```
- Bridge、客户端**都是 NATS 客户端**（对等），都**出站**连 NATS；Bridge 不开入站端口。

---

## 3. Subject / Stream 设计
payload：`RpcCommand`（`proto/future.proto:21`，camelCase）、`StreamEvent`（`{type,data,run_id,idx}`，`:286` + P1 扩字段）、`RpcResponse`（`{type,id,command,success,data,error}`，`:142`）。

| 用途 | 机制 | 名称 | 说明 |
|---|---|---|---|
| **命令 cmd/resp** | 核心 NATS request-reply | 订 `p.{pairId}.cmd.>`，单条 `.cmd.{session}` | App `request()` 发 `RpcCommand`，Bridge queue 订阅并 reply `RpcResponse` |
| **事件 event** | **JetStream 每 pair 一流** | 流 `EVT_{pairId}`（签发服务配对时建），subject `p.{pairId}.evt.{session}` | Bridge 只 **publish**；客户端 consumer（filter=某 session）消费+回放 |
| **回复 inbox** | request-reply 回复 | `p.{pairId}.rep.{device}`（客户端 InboxPrefix） | 不用默认 `_INBOX.>`（维持 pair 隔离，见 [auth §3](remote-control-auth.md)） |
| **目录/presence** | core NATS | `p.{pairId}.presence` | 值含 `{online,lastHeartbeatTs,sessions:[{id,name,streaming}]}`；Bridge 每 ~20s 写，客户端 60s 未收到视为离线 |

- **每 pair 一流**：一台已配对桌面一个流，**签发服务在配对时建、解绑时删**；2 桌面=2 流，物理隔离、按租户配额、`$JS.API` 权限天然按流切开。**Bridge 只有 publish 权，无 STREAM.CREATE/PURGE/DELETE**（最小授权）。
- **流配置**：`MaxAge`（如 30min）+ `MaxBytes` + `MaxMsgSize`（如 1MB）+ `discard=old` + **`dupe-window`**（覆盖预期 Bridge 掉线间隔，如 10min）。
- **cmd 用 `.>` 多 token**：session id 可能含 `.`（`generate_id()` 或客户端 `new_session` 自带 id，`commands.rs:167`），`.cmd.*` 单 token 会漏匹配。

---

## 4. 事件模型与生命周期（run_id 选轮 + 无主动 purge）

**为什么不 purge**：一次用户 run 会发**多次 `agent_start`**（内部 follow-up），`agent_end` 只发一次；且在活跃流上 `purge` 会删掉慢消费者/重连客户端**还没读到的尾部**——无论在 `agent_end` 还是 "下轮" 触发都有竞态。故：

```
每事件带 run_id（每次用户 run 唯一，agent 外层只分配一次）+ idx（run 内单调，集中盖章）
Bridge：只 publish 到 p.{pairId}.evt.{session}，Nats-Msg-Id={session}:{run_id}:{idx}；不 purge
流：MaxAge/MaxBytes/MaxMsgSize/dupe-window 兜底，旧轮自然 GC 老化
客户端：
  - 消费 deliver_policy=all（流里只留最近若干轮，量小）
  - 按事件流里“最近开始的 run”（最后一个 agent_start）确定 currentRunId
  - 只渲染 run_id==currentRunId 的事件，并按 (run_id, idx) 去重
  - 更早历史/滚动 → 分页 get_messages（LLM Message 形状，用历史 renderer）
```
- **回放**：deliver=all → 客户端拿到最近若干轮，自己按 currentRunId 过滤=回放当前轮，无需算 seq，也无删数据竞态。
- **重连（客户端）**：consume deliver=all → 过滤当前轮渲染；去重幂等，晚到/重发不影响。
- **重连（Bridge，掉线时 agent 仍在跑）**：`get_events_since(session, run_id, since_idx)`（P1）补拉缺口 → 重发（客户端按 (run_id,idx) 去重）；`run_id` 不一致=换轮，按新轮重对齐。
- **超大事件**：单事件超 `MaxMsgSize` 则 Bridge 截断并附"完整见分页 `get_messages`"指针。

---

## 5. Bridge 集成（Rust / async-nats）
```rust
let nc = ConnectOptions::with_jwt(jwt, sign_with_device_nkey)
    .custom_inbox_prefix(format!("p.{pairId}.rep.{desktopId}"))
    .connect(nats_url).await?;
let js = jetstream::new(nc.clone());
let inflight = SingleFlight::new();                     // RpcCommand.id → in-flight/已完成，合并重试

// 每 session 一个长期事件泵：先订阅、后才允许发命令，避免漏早期 agent_start/text_chunk
async fn pump_events(session) {                         // 幂等启动一次，长期运行
    let mut evs = agent.stream_events(session).await;   // 现有 StreamEvents（实时 broadcast）
    while let Some(ev) = evs.next().await {              // ev 已带 run_id/idx（P1 集中盖章）
        js.publish(subj(pairId, "evt", session), json(&ev))
          .header("Nats-Msg-Id", format!("{session}:{ev.run_id}:{ev.idx}")).await?;
    }
}

// 命令：queue group 防多 Bridge 重复执行；每命令 spawn 防跨 session HOL 阻塞
let mut sub = nc.queue_subscribe(subj(pairId, "cmd", ">"), format!("bridge.{pairId}")).await?;
while let Some(msg) = sub.next().await {
    tokio::spawn(async move {                           // 不阻塞消费循环
        let cmd: RpcCommand = json(&msg.payload);
        let resp = inflight.run(&cmd.id, || async {     // 单飞：同 id 合并 in-flight/已完成
            ensure_pump_started(cmd.session_id).await;  // 先确保订阅就绪，再执行
            json(&agent.execute_command(cmd).await)     // 本地 gRPC（prompt 是 accept-ack，见 §9.2）
        }).await;
        if let Some(r) = msg.reply { nc.publish(r, resp).await?; }
    });
}

// presence 心跳（core subject；运行设备无需 KV/$JS.API 权限）
loop { nc.publish(format!("p.{pairId}.presence"), directory_json).await?; sleep(20).await; }
```
Bridge 是 `channels/` 的兄弟组件（Rust）；但**订阅-先于-命令**、**每 session 长期订阅**是净新增（`feishu` 模板是 prompt 后才 per-prompt 订阅，正是本项要避免的竞态）。
> - **`ensure_pump_started` 幂等**：用每 session `OnceCell`/锁，防并发首命令重复订阅；该 session 的**第一条命令 await pump 就绪**（run buffer 兜底初始 `agent_start`）。
> - 流由 platform-service 在 `/pair/code` 返回成功前创建；Bridge 不拥有建/删流权限。

## 6. 客户端集成（Web / RN，nats.ws）
```ts
const nc = await connect({
  servers: wsUrl,
  inboxPrefix: `p.${pairId}.rep.${device}`,
  authenticator: jwtAuthenticator(userJwt, deviceSeed),
});

// 命令：prompt reply 只是 accept-ack；完成看事件流的 agent_end
await nc.request(`p.${pairId}.cmd.${session}`, enc(promptCmd), { timeout: 5000 });

// 当前实现：core 订实时事件；重连缺口调用 get_events_since 回补
const seen = new Set();
const events = nc.subscribe(`p.${pairId}.evt.>`);
for await (const m of events) {
  const ev = dec(m) as StreamEvent;
  const k = `${ev.run_id}:${ev.idx}`; if (seen.has(k)) continue; seen.add(k);
  streamRenderer.renderByIdx(ev);
}

// 历史滚动：分页 get_messages（LLM Message 形状）→ 独立历史 renderer
const page = await nc.request(`p.${pairId}.cmd.${session}`, enc(getMessages({offset,limit})));
historyRenderer.render(page);
// 目录/presence
for await (const e of nc.subscribe(`p.${pairId}.presence`)) updateSessionList(dec(e));
```

`EVT_{pairId}` 已由服务端创建并接收 Bridge JetStream publish；Web 直接 consumer
回放仍是后续增强，当前正确性依赖 agent buffer + `get_events_since`。

---

## 7. 鉴权（L1 起，见 [auth](remote-control-auth.md)）
- **核心**：单 NATS account + 按 `pairId` 的 JWT subject 权限（服务端强制）；account-per-user 硬隔离为升级路径。
- **签发服务**：校验 Future 账号（`cli/src/commands/auth.ts` 已有）后签 scoped creds（限 `p.{pairId}.>`）+ **配对时创建 `EVT_{pairId}` 流**；吊销即撤 creds。
- **Bridge 最小授权**：pub `p.{pairId}.evt.>`/`p.{pairId}.rep.>`、sub `p.{pairId}.cmd.>` + 自己的 `$JS.ACK.>`；**不给 STREAM.CREATE/PURGE/DELETE**。
- **L1 代码已完成**：一次性 nonce、双设备 NKey、短期 scoped JWT、分角色刷新与撤销。
- **部署待办**：测试 Relay 需切换 operator/account resolver；当前先用明文
  `nats://`/`ws://` 跑通，详见 future-server 部署 runbook。

---

## 8. 部署后联调
```bash
# GUI 登录 Future 账号，Remote 页点击“配对并启动”。
# Web 验证端打开 http://localhost:8022，粘贴一次性配对码。
```
→ NATS/platform-service 未按 runbook 部署前，配对会返回 remote unavailable。

---

## 9. 关键流程
### 9.1 接入 + 拉历史 + 订实时
```
客户端连 NATS → 订 p.{pairId}.presence（每 session 在线/streaming 态）
consume EVT_{pairId} filter=p.{pairId}.evt.{session} deliver=all → 按 currentRunId 选当前轮 + 去重渲染
更早历史 → 分页 get_messages（历史 renderer）
```
### 9.2 命令往返（prompt = accept-ack）
```
request(cmd.prompt) → Bridge → agent.execute_command 立即返回 ok（accept-ack；agent 内部 spawn 执行）
agent StreamEvents（带 run_id/idx）→ Bridge publish p.{pairId}.evt.{session} → 客户端实时渲染
完成以事件流的 agent_end 为准（不是 reply）
（同会话终端 TUI 走本地 gRPC，天然同步）
```
### 9.3 重连
```
客户端重连: consume deliver=all → 按 currentRunId 选轮 + (run_id,idx) 去重（幂等）
Bridge 重连: get_events_since(session, run_id, since_idx) 补缺口 → 重发（客户端去重）
```

---

## 10. 安全（见 [auth](remote-control-auth.md)）
- **隔离**：单 NATS account + 按 `pairId` 的 JWT subject 权限，服务端强制；每 pair 一流物理隔离；回复 inbox 收进 `p.{pairId}.rep.{device}`。
- **出站-only**：Bridge 只出站连 NATS，不开入站端口。
- **传输**：测试阶段为明文 WS/NATS，只允许测试数据；creds 按
  `p.{pairId}.>` 最小授权（Bridge 无建/删流权）；撤销见 auth §3。
- **数据边界（写实）**：不存长期历史，只短期缓存最近若干轮事件（含文本/工具/
  审批内容）；不做 E2EE。当前靠 NATS auth + TTL/配额验证流程，生产必须再加
  TLS/WSS；历史/文件在本机（分页 `get_messages`）。

## 11. 我们仍需自建的（很少）
1. Subject/资源命名（[总纲 §0.4](remote-control-plan.md)）+ 编解码小库（复用 proto 类型）。
2. presence 心跳（Bridge 发 core `p.{pairId}.presence`，每 session 态）。
3. 客户端 run_id 选轮 + (run_id,idx) 去重逻辑（几行）。
4. ~~最小签发服务~~（已在 future-server 实现：Future 账号 → scoped JWT + 建/删流）。
5. 客户端双 renderer（StreamEvent 流 / Message 历史，复用 GUI）。（推送退后。）
> 路由、扇出、回放、去重（辅）、水平扩展、GC 老化——全由 NATS/JetStream 提供。

## 12. 待定项
1. **NATS 部署**：自建集群 vs. Synadia Cloud（评估国内可用性）。
2. **`dupe-window` 取值**：应覆盖预期 Bridge 掉线窗口；超窗重发会在流里产生重复条目、与 `discard=old` 一起可能更快挤掉 run 头部——**以客户端 `(run_id,idx)` 去重为准，服务端去重当 best-effort**。

---

## 附：出处
- [NATS JetStream / Streams（dedup window、purge、WebSocket）](https://docs.nats.io/nats-concepts/jetstream)
- [nats.ws（WebSocket 客户端）](https://github.com/nats-io/nats.ws)
- [async-nats（Rust 客户端）](https://docs.nats.io)
