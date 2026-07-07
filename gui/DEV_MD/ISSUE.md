# GUI 待办问题（短期跟踪，非长期文档）

> 这是一次代码审查后的**滚动待办清单**，只保留**仍未处理、仍值得做**的问题。已完成、已决定不做、以及可忽略的低价值项都已从本文移除。真正需要长期沉淀的结论请写进对应代码注释或 `PRODUCT.md` / `ER.md` / `PLAN.md` / `CLAUDE.md`，不要反过来引用本文。

## 安全（均暂缓，各有前置条件）

| ID | 严重度 | 位置 | 问题 | 前置 / 处置 |
|---|---|---|---|---|
| SEC-02 | 高 | `src-tauri/src/remote/mod.rs`；`store/app_settings.rs`（`DEFAULT_REMOTE_PAIR_ID = "DEVPAIR"`） | 远程控制命令通道（list_sessions/get_messages/prompt）零鉴权，唯一隔离是 NATS subject 前缀；默认 pair id 为常量；NATS 连接不要求 TLS/凭据；`prompt` 等价 RCE | 远程功能尚未开发完，入口仅非 release 显示。**放开入口前必须完成**：强制随机 pair id + 连接凭据/消息签名 |
| SEC-05 | 低（影响大） | `src-tauri/src/commands/update.rs` | 更新包仅靠 HTTPS + URL 前缀保护，manifest（`latest.json` 仅含 `version`）与安装包无签名/哈希校验；OSS bucket 被攻破即可投毒安装包 → 装上即 RCE | **前置**：先改发布流水线（`.github/workflows/build.yml`）为每个安装包发布 SHA-256，客户端再校验后放行 |
| SEC-04 | 低 | `src-tauri/src/agent_supervisor.rs` | Agent 端口探测即信任：127.0.0.1:50051 有监听就 attach，本机任意进程可冒充 agent 接收全部 prompt 流量（本地攻击面） | 暂缓；需要 agent 侧握手/令牌配合 |
