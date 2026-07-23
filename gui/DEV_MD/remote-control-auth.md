# 鉴权、配对与连接生命周期

> 配套：[总纲](remote-control-plan.md) · [消息中枢](remote-control-relay.md) ·
> [实现进度](remote-control-status.md)。

## 1. 安全不变量

| # | 不变量 | 实现 |
|---|---|---|
| I1 | 一个 pair 的连接不能访问其他 pair subject | NATS user JWT 的 pub/sub allow-list，服务端强制 |
| I2 | 新设备必须持有桌面显式展示的一次性配对码 | 5 分钟 nonce，数据库单条 SQL 原子消费 |
| I3 | 每个设备有独立身份，私钥不离设备 | Desktop/Web 分别本地生成 NATS user NKey |
| I4 | 运行时设备无流管理权限 | 流由 platform-service admin creds 创建/删除 |
| I5 | 回复 inbox 不能退回宽泛 `_INBOX.>` | `p.{pairId}.rep.{deviceId}` |
| I6 | 撤销后不能继续刷新 | Bridge 需 Future API Key；Web 需仅存哈希的 refresh token |

审批路径另外在 Bridge 与 agent 两层校验 session 归属，防止泄漏的 approval id
被用于批准其他会话。

## 2. 信任与身份模型

```text
Future 账号
  └─ desktop binding (desktopId + desktop user NKey)
       └─ pairId
            └─ web/client device (deviceId + client user NKey)
```

- 单个 REMOTE NATS account 承载所有 pair。
- `pairId` 是命名空间；安全边界来自 JWT ACL，而不是 pairId 难猜。
- 当前产品仍按 1 PC ↔ 1 Web/App 绑定；重新配对同一 desktop 会撤销旧绑定。
- 需要更强合规隔离时，可升级为 account-per-user/device，业务 subject 无需改变。

## 3. 配对时序

```text
Desktop                                platform-service                       Web
   | 本地生成 desktop NKey                    |                                |
   |-- Future Bearer + desktop public key --->|                                |
   |   POST /pair/code                        |-- 建 pending binding            |
   |                                          |-- 建 EVT_{pairId}               |
   |<-- bridge JWT + v2 pairing code ---------|                                |
   |                                          |<-- nonce + client public key ---|
   |                                          |    POST /pair/claim             |
   |                                          |-- 原子消费 nonce                |
   |                                          |-- 激活 binding                  |
   |                                          |-- 签 client JWT                 |
   |                                          |-- 生成 refresh token/hash       |
   |                                          |-- JWT + refresh token --------->|
   |========== scoped NATS JWT ================================ scoped WS ======|
```

配对码的 base64url JSON：

```json
{
  "v": 2,
  "nonce": "rpn_...",
  "claim_url": "https://.../client/v1/remote/pair/claim",
  "exp": 1780000000
}
```

它不包含 pairId、账号标识、NATS 地址、JWT、refresh token 或任何 NKey seed。
nonce 仍应视为短期敏感信息，因为 L1 的准入因子就是 possession。

## 4. JWT 与权限

JWT 是 NATS 专有 claims 格式：

- header: `{"typ":"JWT","alg":"ed25519-nkey"}`
- issuer: REMOTE account public NKey
- subject: 设备 user public NKey
- `nats.type=user`, `nats.version=2`
- 默认 `exp=iat+900s`

权限矩阵：

| 角色 | Publish | Subscribe |
|---|---|---|
| Bridge | `p.{pair}.evt.>`, `p.{pair}.rep.>`, `p.{pair}.presence` | `p.{pair}.cmd.>`, `p.{pair}.rep.{desktop}.>` |
| Web/App | `p.{pair}.cmd.>` | `p.{pair}.evt.>`, `p.{pair}.rep.{device}.>`, `p.{pair}.presence` |
| platform-service admin | REMOTE account 管理默认权限 | 用于 `EVT_*` 生命周期，不下发给设备 |

presence 使用 core subject，不再使用 KV。这样 Bridge/Web 均不需要 `$JS.API`，
也不会拥有创建或删除流的能力。

## 5. 刷新与撤销

### Bridge

`POST /auth/token` 携带 Future Bearer，服务端同时校验：

- 账号拥有该 pair；
- desktopId 一致；
- public NKey 与绑定一致；
- pairing 未撤销。

### Web/App

首次 claim 返回随机 refresh token。服务端只存其 SHA-256；刷新同时校验 pairId、
deviceId、public NKey、refresh token hash 与 active 状态。

### 撤销语义

撤销会立即：

- 将 binding 标为 revoked；
- 清空 client refresh hash；
- 删除 `EVT_{pairId}`。

桌面/账号用 Future Bearer 撤销；Web/App 可用自己的 deviceId、public NKey 与
refresh token 自助解绑。Web 验证端只有在服务端确认成功或确认凭证已无效后才清除
本地凭据。

因此任何新刷新都会失败。已建立的 NATS 连接可能存活到当前 JWT 到期；默认上界为
15 分钟。本阶段没有 resolver revocation push + server kick，文案不得宣称“即时
断开”。

## 6. 本地持久化

Desktop `~/.future/remote_pairing.json`（0600）：

```text
pairId, desktopId, nkeySeed, userJwt, natsUrl, natsWsUrl, jwtExpiresAt
```

Web 验证端 localStorage：

```text
pairId, deviceId, seed, userJwt, refreshToken, natsWsUrl, tokenUrl
```

浏览器 localStorage 无法达到系统 keychain 的安全等级，所以 Web 当前仍是验证端。
正式 App 必须把 seed/refresh token 放进 Keychain/Keystore。

## 7. 弱网与自动刷新

- Bridge 到期前刷新 JWT，建立新 NATS 连接后原子替换 command loop、JetStream
  publisher 与 presence heartbeat。
- Web 到期前刷新 JWT，关闭旧连接并用新 authenticator 重连。
- NATS 重连后的业务事件缺口仍由 `get_events_since` + `(runId,idx)` 去重补齐；
  不依赖 JWT 刷新本身保存订阅游标。

## 8. 服务端接口与存储

详见 `future-server/docs/remote-control.md`。核心端点：

- `POST /client/v1/remote/pair/code`
- `POST /client/v1/remote/pair/claim`
- `POST /client/v1/remote/auth/token`
- `POST /client/v1/remote/pair/revoke`（Future Bearer 或绑定客户端 refresh 凭据）
- `GET /client/v1/remote/devices`
- `DELETE /client/v1/remote/devices/:pair_id`

核心表：

- `remote_pairings`
- `remote_pair_nonces`

## 9. 测试部署门槛

代码完成不等于流程已跑通。当前测试部署必须同时满足：

1. NATS 切换为 operator/account JWT resolver，移除全局 token。
2. 4222 使用明文 `nats://`，9090 使用明文 `ws://`，且只传测试数据。
3. REMOTE account JWT 启用 JetStream。
4. platform-service 配置 account seed 与 admin creds 并部署。
5. 验证跨 pair pub/sub 被 NATS 拒绝。
6. 验证 nonce 重放、JWT 刷新、撤销与 15 分钟失效上界。

操作步骤见 `future-server/docs/remote-control-deployment.md`。

JWT 负责身份和 subject 授权，不提供传输加密。生产发布前必须恢复 TLS/WSS；当前
无证书配置不能作为生产方案。

## 10. 后续加固

- NATS account revocation list + resolver push + server kick，实现即时断连。
- 正式 App 账号二因子、生物识别与系统安全存储。
- refresh token rotation、设备审计日志与异常频率限制。
- Web JetStream consumer 回放；当前 `EVT_*` 已创建，但 Web 仍用 core sub +
  agent buffer 回补。
- account-per-user/device（合规触发时）。
