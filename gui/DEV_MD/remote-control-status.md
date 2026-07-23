# 远程控制 · 实现进度

> 设计真源：[plan](remote-control-plan.md) · [relay](remote-control-relay.md) · [auth](remote-control-auth.md)。

## 当前阶段

L1 JWT 鉴权代码已在 FutureOS GUI、Web 验证端与
`future-server/platform-service` 三端落地。要进行公网端到端验证，仍需按
`future-server/docs/remote-control-deployment.md` 完成 NATS operator/account
切换、部署 platform-service 并重启相关服务。

旧的共享 NATS token、手填 pairId、按平台域名推导明文 `nats://`/`ws://`
地址均已从主流程移除。

## 已实现

### Bridge 与业务闭环

- Bridge 内嵌 GUI Tauri 后端，远程 prompt 复用 GUI 持久化与 agent 执行
  路径；远程对话自动落 SQLite 并通知当前页面刷新。
- 命令：会话列表/历史、新建、prompt、abort、审批、模型、思考等级、重命名。
- 运行中语义与 GUI 对齐：同一会话运行中再次发送会被拒，不存在 steer、
  follow-up 或排队。
- 审批在 Bridge 与 agent 两层校验 session 归属。
- 命令按请求 id 单飞并缓存响应，避免重试重复执行。
- agent 为每轮事件盖 `run_id` 与单调 `idx`；客户端按 `(run_id, idx)` 去重，
  并用 `get_events_since` 处理重连缺口。

### JWT 配对与设备身份

- GUI 必须先登录 Future 账号。首次“配对并启动”时在本机生成 desktop NKey；
  seed 仅保存到 `~/.future/remote_pairing.json`（0600）。
- GUI 以 Future API Key 调用
  `POST /client/v1/remote/pair/code`，得到 Bridge scoped JWT 与一次性配对码。
- 配对码版本为 v2，只含 `{nonce, claim_url, exp}`；5 分钟过期且只能消费一次，
  不含账号密钥、NATS JWT、pairId 或设备私钥。
- Web 本地生成独立 NKey，调用 `/pair/claim` 后保存自己的 JWT、seed 与刷新
  token。浏览器存储为 localStorage，仅定位为验证端；正式 App 应使用 keychain。
- Bridge 刷新 JWT 需要 Future API Key；Web 刷新需要高熵设备刷新 token，服务端
  只存 SHA-256 哈希。默认 JWT 有效期 15 分钟，两端在到期前刷新并重连。
- GUI 与 Web 解绑都先调用服务端撤销，再删本地凭证；撤销后不再允许刷新，活跃
  连接最迟在当前 JWT 到期时失效（本阶段未做 NATS server kick）。

### 服务端强制隔离

- 单 REMOTE NATS account；`future-server` 使用 account seed 签标准 NATS user
  JWT。
- Bridge：
  - pub `p.{pairId}.evt.>`、`p.{pairId}.rep.>`、`p.{pairId}.presence`
  - sub `p.{pairId}.cmd.>`、`p.{pairId}.rep.{desktopId}.>`
- Web：
  - pub `p.{pairId}.cmd.>`
  - sub `p.{pairId}.evt.>`、`p.{pairId}.rep.{deviceId}.>`、
    `p.{pairId}.presence`
- inboxPrefix 收敛到 `p.{pairId}.rep.{deviceId}`。跨 pair 发布/订阅由 NATS
  服务端拒绝，不再依赖 pairId 不可猜。
- presence 改为 core NATS 心跳，不使用 KV；因此运行时设备无需任何
  `$JS.API` 管理权限。

### future-server 控制面

- 新增 `/client/v1/remote` 路由：配对码、claim、刷新、撤销、设备列表。
- `remote_pair_nonces` 以单条 SQL 原子消费 nonce，避免 TOCTOU。
- `remote_pairings` 记录账号/桌面/Web 公钥、刷新哈希与生命周期；同一账号下同一
  desktop 只允许一个 pending/active 绑定，重新配对会撤销旧绑定。
- 配对时由 platform-service admin credential 创建 `EVT_{pairId}`；撤销时删除。
  Bridge 无建/删流权限。
- Rust JWT encoder 已用 NATS 官方 CLI 生成的带过期时间 user JWT 校验 JTI
  算法和字段结构。

## 当前验证

- `future-server`: `cargo check -p future-platform-service` 通过。
- `future-server`: remote JWT/identifier 单测通过。
- GUI Tauri: `cargo check` 通过。
- Web/React 仍需完成前端 build/lint 与真实 JWT-mode NATS 端到端验证。

## 运行方式（完成部署后）

```bash
make run-gui
```

1. GUI 登录 Future 账号。
2. Remote → “配对并启动”，复制一次性配对码。
3. 打开 `http://localhost:8022`，粘贴配对码并连接。
4. 后续浏览器可用本地设备凭证重连，无需再次扫码；显式重新配对或撤销后例外。

## 尚未完成

- 测试 NATS 切换到 operator/account JWT（暂用明文 `nats://`/`ws://`）并部署
  platform-service（由运维执行）。
- 真实环境跨 pair 越权测试、JWT 到期刷新/重连测试与撤销测试。
- Web JetStream consumer 回放；当前仍以 core event sub +
  `get_events_since` 回补，`EVT_*` 已持久化但 Web 尚未直接消费。
- NATS server kick/账号 revocation push；当前撤销保证为 `≤ JWT TTL`。
- 正式移动 App/keychain、生物识别、审计、附件与右侧文件面板。
