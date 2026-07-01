# 鉴权、配对与连接生命周期

> 配套：[总纲](remote-control-plan.md)（决策/术语/阶段真源见其 §0）· [消息中枢（NATS）](remote-control-relay.md)。
> 回答三件事：**(1) 鉴权虽后做但要有计划、且保证隔离；(2) 配对/持久连接方案与评估；(3) 开发期用 Web 客户端替代 App。** 鉴权成熟度 **L0/L1/L2** 见 [总纲 §0.3](remote-control-plan.md)。

---

## 1. 安全不变量（任何阶段不破坏）
| # | 不变量 | 由谁保证 |
|---|---|---|
| **I1 消息隔离** | 一个连接只能访问自己 `pairId` 的 subject，别人的**订不到、发不进** | **NATS 服务端强制**：单 account 内按 `pairId` 的 JWT subject 权限；每 pair 一流物理隔离。升级可用 account-per-user |
| **I2 设备准入** | 新设备入网需 possession（扫 QR）；L2 再加 knowledge（账号登录） | 配对流程 |
| **I3 持久可撤销** | 配对一次持久；任一方撤销/登出后失效（≤TTL 生效，活跃连接需 server kick，见 §3） | 设备凭证 + 服务端绑定记录 |
| **I4 最小授权** | 凭证只授予 `p.{pairId}.>`；回复 inbox 收进 `p.{pairId}.rep.{device}.>`（不用默认 `_INBOX.>`）；Bridge **无建/删流权** | NATS user JWT + 客户端 InboxPrefix |
| **I5 无秘密外泄** | 出站-only；设备私钥永不离开设备；QR 不含密钥/密码 | 配对协议设计 |
| **I6 L0 显式无强制** | 无鉴权仅限本地/可信网络，绝不公网 | 发布纪律（[总纲 §6](remote-control-plan.md)） |

> **I1 的"根"**是服务端强制的 JWT subject 权限（非应用逻辑）——某 pair 的凭证连订阅别的 `pairId` 都被 NATS 拒。
> **一个例外要写实（审批路径）**：agent 的 `ApprovalGate` 是**全局 HashMap、按 `entry_id` 查、无 pair/session 校验**（`approval.rs`）。1:1 下**每 agent=一桌面=一 pair**，天然 pair-scoped；但为防 `entry_id` 泄漏、也为将来 1:N，**须在 agent 的 `ApprovalGate.decide` 加 session 归属校验**（decide 收 `session_id`，与 `PendingApproval.session_id` 比对；Bridge 无 `entry_id→session` 映射，故放 agent 侧）。故审批"零新语义"仅指**命令形态**不变。另注：审批门**无 timeout**（`rx.recv()` 无期限），客户端离线时须由 **Bridge 发 `abort`/`cancel_session` 兜底**，防 session 永久挂住。

---

## 2. 鉴权成熟度与节奏（先 App、后鉴权）
| 级别 | 配对/校验 | 隔离 | 对应交付 |
|---|---|---|---|
| **L0 无鉴权/粗鉴权** | 本地无 / staging 可加单 token 或 IP 白名单 | 无（仅 subject 命名分区） | P2–P4（本地或受控 dev/staging，禁公开无鉴权） |
| **L1 扫码 + scoped creds** | 仅扫 QR（App 不做账号二次校验） | 账号+设备（服务端强制） | **P5（首个可发布）** |
| **L2 加因子** | 扫 QR + App 账号登录 + 生物识别 | 同上 + 吊销/审计 | P6 |

> **L1 不是"零鉴权"**：要让隔离在服务端成立，L1 需一个**最小签发服务**——桌面（已用 Future 账号登录）申请配对令牌，App 扫码后换**按 `p.{pairId}.>` 限定**的 NATS creds，签发服务同时**创建 `EVT_{pairId}` 流**。省的是"App 端账号登录"，省不掉"签发 scoped creds + 建流"。subject 从 L0 起就按最终 `p.{pairId}.*` 走，L1 只补签发，不改架构。

---

## 3. 身份与隔离模型（账号 + 设备，1 PC ↔ 1 App）
**已定**：单 NATS account + 按 `pairId` 的 JWT subject 权限（服务端强制、运维轻），严格 1:1。NATS subject 支持扇出，日后 1:1→1:N 只是登记/权限改动、非重构。

```
单 NATS account（起步）
  每次配对生成唯一 pairId（源自 futureAccount + desktopId）
  subject:  p.{pairId}.cmd.{session}   p.{pairId}.evt.{session}   p.{pairId}.rep.{device}
  stream:   EVT_{pairId}（签发服务在配对时创建、解绑时删）
  凭证（每设备独立 nkey，私钥不离设备）
```

**NATS 权限矩阵**（签发服务据此签 JWT；每 pair 隔离；最小授权）
| 主体 | pub | sub | JetStream `$JS.API` | KV |
|---|---|---|---|---|
| **Bridge** | `p.{pairId}.evt.>`（发事件到流）、`p.{pairId}.rep.>`（回复） | `p.{pairId}.cmd.>`、自己的 `$JS.ACK.>` | **无**（不建/删/purge 流；仅靠 publish + ack） | 写 `$KV.pairs.{pairId}` |
| **App/客户端** | `p.{pairId}.cmd.>` | `p.{pairId}.evt.>`、`p.{pairId}.rep.{device}.>`（自己的回复 inbox） | 在该流建/读 consumer：`$JS.API.CONSUMER.CREATE.EVT_{pairId}.*`、`.INFO.*`、`.MSG.NEXT.EVT_{pairId}.*` | 读/watch `$KV.pairs.{pairId}` |
| **签发服务** | — | — | 建/删流：`$JS.API.STREAM.CREATE/DELETE.EVT_{pairId}` | 建桶 `pairs` |

> - **回复 inbox 不用默认 `_INBOX.>`**：那会给宽泛订阅权、破坏隔离。改用 `p.{pairId}.rep.{device}.>` + 客户端 `request()` 设 **InboxPrefix=`p.{pairId}.rep.{device}`**——回复也锁进 pair 命名空间。
> - **流生命周期归签发服务**（非 Bridge）：Bridge 只 publish，满足最小授权（I4）。`$JS.API`/`$KV` 具体 subject 语法按 NATS 标准。

- **"account+设备"的含义**：Future 账号决定"谁能发起配对"；设备 pair 的 subject/流隔离运行时消息。
- **1:1 在登记层强制**：一桌面至多一个生效 App；重新配对=作废旧绑定（需桌面重新出示 QR）。
- **升级路径**：若要"同一用户多台 PC 也硬隔离"，把每账号（或每设备）提升为独立 NATS account——subject/流不变、隔离更强，代价是多层 account 运维。

**凭证与撤销生命周期**
```
Future 账号 OAuth（已有 cli/auth）→ 证明账号归属
  → 签发服务（持 account 签名密钥）签 设备 user JWT（+设备自持 nkey seed）= .creds（持久）
  → 每次连接：NATS 校验签名&权限 → 授权

撤销（写实）：持久 JWT 仅记录撤销**不会自动踢已建连接**。二选一：
  ① 短期 user JWT（分钟级）+ 刷新——停刷新使下次重连/刷新失效（生效延迟 ≤TTL）【L1 默认】；
  ② NATS account 吊销列表推送 resolver + **server kick**——踢掉已建连接、真正即时【需强即时时】。
  ⇒ 措辞：撤销"≤TTL 生效"，"活跃连接需 server kick 才断"——**不是即时**。
```

> **签发服务的落点（已定）**：实现在 **`future-server`（platform-service 的一个路由模块）**，复用其账号/session/device-flow OAuth/Postgres/密钥体系（`resolve_user_from_session`、`routes/device_flow.rs`、`config.rs`）。它持有 **NATS account 签名密钥**（签 user JWT）+ NATS admin creds（配对时建/删 `EVT_{pairId}` 流）；只在**配对/鉴权控制面**，**不在消息数据面**（运行时客户端/Bridge 直连 NATS）。**唯一技术不确定点**：Rust 侧签出 NATS 专有格式的 user JWT——**开工前先 spike**。future-server 侧完整需求见 `future-server/docs/remote-control.md`。

---

## 4. 配对与持久连接（详细 + 评估）
### 4.1 你的方案 & 评估
> App 扫桌面 QR →（L2）输账号密码登录 → 建立**持久连接**（无需常连网、无需再扫码）→ App/桌面重启不影响（除非主动断开或一方退出）。

**成立，是成熟的「链接设备」模型**（WhatsApp/Signal/Telegram/Claude Trusted Devices）。

| 你的点 | 评估 | 落地 |
|---|---|---|
| 扫 QR 建信任 | ✅（possession 因子） | QR 只放非秘密：natsWsUrl、accountId(不透明)、desktopId、一次性 nonce、exp、签名 |
| 含"账号信息" | ⚠️ 可含标识、不可含凭证 | 账号归属由登录证明（L2），不由 QR |
| 输密码登录（一期可无） | ✅（L2 knowledge 因子） | 一期(L1)跳过；L2 复用 Future OAuth，登录后存刷新型凭证、之后免密 |
| 持久、免再扫、重启不影响 | ✅ | 持久物 = 设备凭证（私钥+JWT）+ 服务端绑定；连接无状态可重建 |
| 一方退出即失效 | ✅（生效 ≤TTL 或 server kick，见 §3） | 登出=删本地凭证+服务端撤销；"已链接设备"列表可远程登出 |

**要点**
1. **1:1**：放弃"一手机控多机/多手机看一机"，换最简最稳；可逆到 1:N。安全协同：1:1 + **单次原子消费**的 nonce ⇒ QR 只对第一个扫描者生效，压低 L1"仅扫码入网"风险面。
2. **双因子（L2）**：L1 只扫码=单因子；L2 加账号登录，使"仅凭盗号密码远程入网"不成立。（"GhostPairing" 类攻击提醒：QR 必须短 TTL、单次、显式触发。）

### 4.2 配对时序（L1；L2 增第 4–5 步账号校验）
```
1. 桌面 future remote pair → 签发服务 申请 pairNonce（单次, TTL~5min, 绑 accountId+desktopId, 签名）
   桌面渲染 QR = { v, natsWsUrl, accountId, desktopId, pairNonce, exp, sig }
2. App 扫 QR → 校验 sig + exp
3. App 本地生成 nkey（seed 留本地）
4. [L2] App 账号登录 → accountProof
5. App → 签发服务 /pair/claim { pairNonce, devicePubKey, accountProof?, deviceMeta }
6. 签发服务：**原子消费 nonce**（单条 `UPDATE … WHERE used=false` 返回影响行数=1 才通过，防 TOCTOU）
   （L2 再校 accountProof 与 accountId 一致）→ 生成 pairId → 登记 1:1 绑定
   → **创建 EVT_{pairId} 流** → 签 scoped user JWT（限 p.{pairId}.>）→ 返回 { pairId, userJWT }
7. App 落 .creds（seed+JWT）到 keychain。绑定完成
```

### 4.3 会话连接（每次启动，不再扫码）
```
App/Bridge 启动 → 读 .creds → nats.connect(natsWsUrl, creds, inboxPrefix) → NATS 校验 → 连上
重启/断网安全：凭证持久 + 连接无状态可重建；恢复自动重连（无需重配）
```

### 4.4 撤销 / 登出（任一方）
```
App 登出       → 删本地 .creds + 通知签发服务撤销
桌面/账号 撤销  → 签发服务撤销设备 JWT（停刷新[≤TTL] 或 account 吊销列表推送+server kick[即时踢连]，见 §3）→ App 需重新配对
"已链接设备"表  → App/桌面均可查看设备+最近使用+远程登出（仿 WhatsApp）
```

---

## 5. 市场方案对照
| 方案 | 配对 | 设备身份 | 持久性 | 撤销 | 借鉴 |
|---|---|---|---|---|---|
| WhatsApp/Signal 链接设备 | 扫 QR + 主设备签名 | 每设备独立密钥 | 持久，扫一次 | 主设备远程登出 | 每设备密钥、已链接设备列表、远程登出 |
| Claude Trusted Devices/Dispatch | QR/URL；enroll+近期登录+生物识别 | 每设备凭证 | 持久 | 账号/管理台吊销 | 双因子、短时凭证、生物识别 |
| Telegram 多设备 | 扫 QR/验证码 | 每会话密钥 | 持久 | 会话列表登出 | QR 免密体验 |
| Tailscale | 设备 key 入网 | node key | 持久 | 管理台撤销 | 设备级凭证 + 集中撤销 |

---

## 6. 开发期 Web 验证客户端
**可行且推荐。** App 本质=「`nats.ws` 客户端 + 渲染」，Web 页面可完全替代它做联调。

- **形态**：`nats.ws` 网页（可挂在现有 GUI/React 一个路由）。
- **输入"设备验证信息"**：L0 = NATS WS URL + `pairId`/`session`（或粘贴 dev creds）；L1/L2 = 浏览器 OAuth 换临时 creds。
- **能力**：读 KV `pairs` → 选会话 → 分页 `get_messages`（历史 renderer）→ consumer 订 `EVT_{pairId}`（按 currentRunId 选轮 + 去重，流 renderer）→ composer 发 prompt/steer/abort、审批。
- **复用**：GUI 现有 React 渲染组件（流 + 历史两套）+ `proto` 派生 TS 类型。代码骨架见 [中枢 §6](remote-control-relay.md)。

**双重价值**：① 当下 = P2/P3 端到端测试台（无 App Store 摩擦）；② 未来 = 演进为正式 **"Remote Control on Web"**。建议 Web 端提前到 **P2**，原生 App（P4）复用同一渲染层。

---

## 7. 待定项
1. **设备凭证有效期 / 撤销即时性**：短期 JWT + 刷新（生效 ≤TTL，推荐）vs. 长期 JWT + account 吊销列表推送（需 resolver + server kick 才踢活跃连）。
2. **重复配对策略**：新 App 配对时旧绑定自动作废（推荐）vs. 需桌面显式解绑。
3. **是否升级 account 硬隔离**：同一用户多 PC 是否要 account 级硬隔离（合规触发才需）。
4. **Web 客户端定位**：仅测试工具 vs. 正式 Web 端。

---

## 附：出处
- [WhatsApp 多设备加密（每设备密钥/QR/远程登出）](https://engineering.fb.com/2021/07/14/security/whatsapp-multi-device/)
- [Signal 链接设备](https://signal.org/blog/a-synchronized-start-for-linked-devices/)
- [NATS 多租户 Accounts](https://docs.nats.io/running-a-nats-service/configuration/securing_nats/accounts)
- [NATS 去中心化 JWT 鉴权](https://docs.nats.io/running-a-nats-service/configuration/securing_nats/auth_intro/jwt)
- [Claude Code Remote Control / Trusted Devices](https://code.claude.com/docs/en/remote-control)
