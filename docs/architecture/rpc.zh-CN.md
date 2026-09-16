# future-rpc

> （[English](rpc.md)）FutureAgent 与其客户端（TUI / CLI / channel 桥接 / 桌面端
> 后端）之间的线上契约。

本 crate 是 `proto/future.proto` 生成 proto 代码（tonic server 与 client 模块）以及
typed-RPC payload 契约的**唯一持有者**：共享 payload 结构体加上编解码层。原先分散在
`agent/`、`channels/` 与 `desktop/src-tauri/` 的各 crate 生成副本已退役收敛到本
crate。所有消费方都是 Rust；此前的姊妹 npm 包 `future-rpc/ts`（`@future-os/rpc`）
在 TUI/CLI 移植到 Rust 时已移除。

## Typed 响应与双写事件

`RpcResponse.payload` 与 `StreamEvent.payload`（都在 20 号字段）承载 Tier-1
命令/事件的 typed `oneof` payload。

- **命令-响应双写已退役。** Typed 命令携带 `payload`、`data` 为空；未 typed 的命令
  保留 JSON `data`。`decode::response_data` 是 typed-first 并带 JSON 回退。旧的
  data-only 客户端无法消费 typed-only 响应；保持客户端与 agent 兼容。
- **事件流仍然双写。** `data` 对 journal/NATS 保持字节稳定。`decode::event_data` 与
  `event_data_json` 优先使用现有 `data`，以 typed 重建作为回退。不要把响应的退役
  套用到事件上。
- Payload JSON 使用规范 camelCase；已移除的 legacy 别名注入/剥离不属于当前契约。

模型/agent 运行时内部已不再使用这个字符串信封：模型 provider 发出 typed 模型事件，
Agent 只在 RPC 边界把 typed run 事件投影到 `StreamEvent`。线上信封本身仍是迁移
边界，不是最终设计。退役它需要：所有 sideband 与控制面事件获得 typed payload 变体、
为 journal/replay/NATS 记录提供版本化迁移、为已发布客户端提供显式兼容窗口。在这些
前置条件完成之前，保持 `type`/`data` 双写与重放语义字节兼容。

- `encode.rs` — JSON `data` Value → typed `payload`（agent 侧）。防御式：未知/形状
  不匹配的输入返回 `None`，让客户端回退。
- `decode.rs` — 响应 typed-first 解码与事件 data-first 解码，需要时重建规范 Value。
  `event_data_json` 在仍双写期间优先使用原始 `data` 字符串（对持久化 / NATS 重发
  字节稳定）。
- `payloads.rs` / `payloads_ext.rs` / `event_payloads.rs` — encode 与 decode 共享的
  serde payload 载体（构造上即一致）。
- `events.rs` — `AgentEvent` 枚举 + `parse_agent_event`（channel 桥接视角）。

## Proto 代码生成

重新生成是可选行为，由 `REGENERATE_PROTO` 环境变量门控，正常构建不需要 `protoc`：

```sh
REGENERATE_PROTO=1 cargo build -p future-rpc   # 或：make generate-proto
```

生成产物（`src/generated/proto.rs`）已提交进 git。CI 有 freshness 门禁：重新生成并
在任何 diff 时失败。

## 契约规则

- Proto 字段号稳定且**不得复用**（见 `proto/future.proto` 头部）。Typed payload
  `oneof` 成员只增不改。
- Typed payload 以 20 号字段挂到宿主消息（`RpcResponse`、`StreamEvent`、
  `ProjectedRunEvent`、`ReplayEvent`）上；事件的 JSON `data` 字段保持双写；typed
  命令响应将其留空。
- JSON 形态区分 null/absent 与默认值的 proto3 字段声明为 `optional`，使 typed 路径
  保留 JSON 语义。
- `transport.rs` 持有共享的每用户 IPC 发现与显式 TCP 回退；其依赖包括 async/网络与
  平台 IPC 支持（见 Cargo.toml）。所有消费方（`future-agent`、`future-channel`、
  经 path dependency 的桌面端 Tauri 后端）依赖它——绝不反向。桌面端后端在自己的
  cargo workspace 里：其 `tonic`/`prost` 版本要与根 `workspace.dependencies` 钉住
  的版本对齐。
