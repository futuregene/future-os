# 自建中继（无需账号的远程控制）

FutureOS 的远程控制由「NATS 中继 + 其上端到端加密」两层组成。官方中继是
`future-os.cn`：一套 NATS 部署，加上一个为已登录账号签发「按配对限定」NATS
user JWT 的平台服务。`future relay` 是同一个中继去掉账号部分——桌面端和手机
可以在你自己机器上完成配对，整条链路上没有任何 FutureOS 账号。

**线协议没有变化。** 变化的只是客户端如何取得 NATS 凭据，因此官方中继和自建
中继可以同时存在，同一台桌面也能同时服务两边。

## 它是什么、不是什么

| | 官方中继 | `future relay` |
| --- | --- | --- |
| 谁签发 NATS 凭据 | 平台服务，针对账号 | 中继，针对 room |
| 需要账号 | 是 | 否 |
| pair id 由谁生成 | 平台 | 桌面端 |
| 配对靠什么认证 | 邀请 + `Noise XXpsk0` | 邀请 + `Noise XXpsk0`（完全相同） |
| 记录层、subject、命令 | `FRE2` + JSON，按配对分 subject | 完全相同 |
| 撤销 | 云端状态 + 短 JWT | 本地解绑 + 删除 room |
| 存储 | Postgres | 一个 JSON 文件，只存哈希 |

中继**不是信任根**：它永远看不到 Noise 静态密钥、邀请 PSK，也看不到任何一个
明文字节；它也无法伪造密钥——二维码里的邀请是唯一把两端绑定起来的东西。

## 1. 创建信任配置

```bash
future relay init --dir /opt/future-relay \
  --listen 127.0.0.1:8099 \
  --public-url https://relay.example.com \
  --nats-url tls://relay.example.com:4222 \
  --nats-ws-url wss://relay.example.com:9090 \
  --tls-cert /etc/letsencrypt/live/relay.example.com/fullchain.pem \
  --tls-key  /etc/letsencrypt/live/relay.example.com/privkey.pem
```

会在 `--dir` 下写入：

| 文件 | 内容 |
| --- | --- |
| `operator.seed` | operator NKey，务必保密 |
| `account.seed` | account NKey，签发本中继的每一张凭据，务必保密 |
| `operator.jwt` | nats-server 信任的 operator JWT |
| `account.jwt` | account JWT，由 resolver 预加载 |
| `nats.conf` | 可直接使用的 nats-server 配置：operator 模式、内存 resolver、无 JetStream；给了证书就带 TLS |
| `relay.json` | 中继自身配置 |
| `rooms.json` | 首次配对时创建，room 注册表 |

`--tls-cert` 与 `--tls-key` 必须同时给出。不给则生成明文监听配置：HTTP、`ws://`
和 `nats://`。这条路径是给局域网自建用的——见下一节。

对外宣告的 scheme 和监听端必须一致，`init` 会直接校验：`tls://` / `wss://` 却没给
`--tls-cert`/`--tls-key` 会被拒绝——否则生成的 `nats.conf` 是明文监听，而每条
凭据响应都宣称了 TLS，桌面端要求 TLS、手机要求 `wss://`，配对必然失败。有证书却
宣告明文同样被拒绝。如果中继前面另有一层做 TLS 终止（反向代理、负载均衡、service
mesh），监听本来就应该是明文而对外地址不是：加 `--allow-plaintext-upstream` 跳过
这条校验。**只有确认前面真的有一层 TLS 终止时才这么用**，否则凭据和元数据是裸奔的。

两个监听端口取自 `--nats-url` / `--nats-ws-url`，因此中继发给客户端的地址就是它
实际监听的地址。

生成的 `nats.conf` 默认**不开** nats-server 的 HTTP 监控端点：中继自己从不读它，
而它的固定端口会让同一台机器上的第二个中继起不来。需要那些统计时，把文件里那行
`http:` 解除注释即可。

`--listen` 默认 `127.0.0.1:8099`，`--public-url` 默认是对应的回环 origin——这是
「桌面端在同一台机器上」的默认值。局域网里的手机要连得上，就得让中继监听一个可达
地址：`--listen 0.0.0.0:8099 --public-url http://<本机局域网地址>:8099`，因为
`--public-url` 的地址会被原样写进每张邀请码。

### 明文还是 TLS

TLS 与 Noise 保护的不是同一件事。二维码里的 Noise 静态密钥和邀请 PSK 才决定
「对面是不是那台桌面」，内容层与传输层无关；TLS 保护的是**凭据和元数据**：join
token、`refresh_token`、NATS JWT，以及 pair id、设备名和流量形态。

所以两种部署的取舍是：

| | 带 `--tls-cert/--tls-key` | 不给证书（明文） |
| --- | --- | --- |
| 内容是否加密 | 是（Noise） | 是（Noise） |
| 凭据与元数据 | 被 TLS 保护 | **同一网段的任何人都能抓到并重放** |
| 谁的证书 | 需要客户端信任的 CA | 不需要 |
| 手机 | 要求系统信任该证书 | 直接可用 |

明文之所以存在，是因为局域网里的自签证书在手机上几乎无法让应用信任：Android 自
API 24 起对普通应用忽略用户安装的 CA，自签或私有 CA 要么装不进信任链，要么只能
靠改变整个 App 的网络配置来实现。结果不是「更安全」，而是「用不了」。自建中继
因此允许明文中继：手机（`http://` + `ws://`）和桌面端都以邀请里声明的那个 origin
为准，指向别处的地址仍然被拒绝。

这条放宽只覆盖**中继自己声明的 scheme**：声明 `https://` 的中继仍然必须回
`wss://`，官方平台更是始终要求 `https://` + `wss://`。要做公网部署就用公共 CA，
那条路径完全不受影响。

一份证书会写进每个监听器。`--tls-cert/--tls-key` 在生成的 `nats.conf` 里出现
**两次**：一次给客户端端口，一次在 `websocket` 块内部——nats-server 不允许
websocket 流量继承顶层的 `tls`，而声明了 `wss://` 却没有 websocket TLS 配置会
直接拒绝启动。手工改这个文件时两块都要留；写成空的 `tls {}`（没有证书路径）虽然
能启动，但每次握手都会被拒，日志是 `tls: no certificates configured`。

私有 CA 会同时覆盖 TLS 部署的两段链路。`--relay-ca` / `FUTURE_RELAY_CA` 给出的
根证书既追加到 NATS 连接，也追加到注册房间、领取邀请所用的 HTTP 客户端，所以由
同一个私有 CA 签发的 API 不需要把证书装进操作系统就能用。它是在信任集里**追加**，
而不是放宽校验：链不到它的证书照样被拒。如果这台机器上还有别的工具需要该根证书，
macOS 上仍建议把它装进钥匙串。

## 2. 启动

```bash
nats-server -c /opt/future-relay/nats.conf      # 或 systemd / docker
future relay serve --config /opt/future-relay/relay.json
```

`future relay serve` 不需要别的东西：没有数据库、没有账号、自身也不访问网络。
它监听 `--listen`，提供以下路由：

| 路由 | 认证方式 | 用途 |
| --- | --- | --- |
| `GET  /v1/health` | 无 | 存活检查 + room 数 |
| `POST /v1/rooms` | 桌面端 NKey 签名 | 创建/刷新某个配对的 room |
| `GET  /v1/rooms/{pair}` | 桌面端 NKey 签名 | 查看这个 room 里有谁 |
| `DELETE /v1/rooms/{pair}` | 桌面端 NKey 签名 | 删除 room |
| `POST /client/v1/remote/pair/claim` | 一次性 join token | 手机成为该 room 的一个设备 |
| `POST /client/v1/remote/auth/token` | bridge 用签名 / client 用 refresh 凭据 | 轮换短期 NATS JWT |

`/client/v1/...` 这些路径**故意**与官方平台一致，JSON 形状也一致，因此手机和
桌面端的 client 角色不需要任何特判：换成自建中继只改变请求**发往哪里**，不改变
请求**说什么**。

## 3. 让桌面端连上它

```bash
futureos --relay https://relay.example.com           # GUI
future headless --relay https://relay.example.com  # 无 GUI 服务器
```

地址只对**本次启动**生效，不写任何文件。所以下次不带这个参数启动就回到官方平台——一次调试用的自建中继不会悄悄变成这台机器的长期选择。`--relay ""` 等同于不写这个参数。私有 CA 用 `--relay-ca /path/to/ca.pem`（或环境变量 `FUTURE_RELAY_CA`）追加，它同时作用于 NATS 连接和上面那些 relay HTTP 请求（注册房间、领取邀请），所以 `https://` 的 API 也认这个私有 CA，不需要先装进系统钥匙串。
容器或测试里也可以用环境变量 `FUTURE_RELAY_URL` 直接指定（优先级高于参数，空值表示官方中继）。

自建模式下：

* 远程控制不需要 FutureOS 登录，`future headless` 直接跳过登录门槛；
* 桌面端自己生成 pair id、Noise 身份、邀请 PSK 和 join token；
* 邀请是 `futureos://remote/pair?v=3`。其 payload 仍保持官方 v2 的形状（所以
  claim 路径不变），并额外带 `relay`——即 claim 端点所属的 origin；
* 二维码里仍然是 `desktopKey`、`secureKey`、`secret`；中继只拿到 join token 的
  **哈希**。

通过官方平台配对的设备继续可用：选择是按**配对**记录在 `remote_pairings.json`
的 `controlUrl` 里的，所以一台桌面可以同时持有一个官方设备和一个自建设备，各自
找当初签发它的中继。

在终端管理这些配对，两种模式下是同一条命令：

```bash
future remote devices                  # 每台设备的 endpoint 都会标出来
future remote devices add              # 在当前中继上签发 v3 邀请
future remote devices remove <id|序号>  # 用该配对自己的密钥签名，删掉它的房间
future remote devices remove --all     # 全部设备，各自交给自己的签发方
```

全程不涉及 FutureOS 登录：中继没有账号，`add` 也只是去问运行中的 Desktop。
中继使用私有 CA 时这些命令也不需要额外参数——用 `FUTURE_RELAY_CA` 指定即可，它只作用于这次请求。设置同样不跨次保留：自建中继靠每次启动的 `--relay` 选择，不带就是官方平台。

终端是另一个进程，所以当它需要自己判断（`future remote enable` 会先确认这台机器能
不能在不依赖账号的情况下提供服务）时，用同样的方式指定中继：

```bash
FUTURE_RELAY_URL=https://relay.example.com future remote enable
```

只去问运行中 Desktop 的命令不需要额外提示：Desktop 知道自己是用哪个中继启动的。

## 4. 官方中继与自建中继共存

不需要任何额外操作。一台机器可以持有多个配对，每个配对有自己的中继、自己的
NATS 连接、心跳和凭据轮换。

* 开发版默认连 `test.future-os.cn`；设置页切换环境不会影响已经带 `controlUrl`
  的配对。
* 手机可以同时配对多台桌面，官方和自建可以混用；选择器展示的是机器，不是中继。
* 某个中继不可用只影响它自己的配对，其余照常服务。

## 5. 中继能做什么、不能做什么

内容层由 `Noise XXpsk0`/`IK` 和 `FRE2` 记录层保护，两者都没有改动。一个恶意的
中继运营者可以拿到：

* **没有明文**，也无法伪造命令、回复、文件分片或 presence/catalog 事件（AEAD 把
  NATS subject 作为附加数据，回复上下文再把回复绑定到请求）；
* **无法替换密钥**——桌面端的 Noise 静态公钥来自二维码，向中继索要公钥的客户端
  本身就不符合协议；
* **元数据**：pair id、设备公钥、设备名、subject 名称、消息大小与时间、流量模式。
  TLS 能挡住网络观察者，挡不住中继本身；
* **可用性**：它可以丢、延迟、重排或拒绝。NATS Core 本来就是至多一次投递；恢复
  靠客户端重连与重放，而不是中继的承诺。

它**做不到**的事，来自 room 与凭据的作用域：

* 读写另一个配对的 subject。每张凭据的 allow 列表只覆盖一个 `p.<pair>.>` 命名
  空间，broker 强制执行——`SUB >`、跨配对订阅、抢 queue group 都会被拒；
* 以 bridge 身份从 client 连接发布，或读 client 的回复 inbox；
* 用一个 join token 配第二台设备。它按设备一次性（同一设备可以重试丢失的响应），
  五分钟后过期，中继只保存它的 SHA-256。

把这些机器暴露到公网前，你还需要对以下残留风险做决定：

| 风险 | 缓解 |
| --- | --- |
| 中继能看到 pair id、设备名和流量形态 | 自己跑；不要把中继放在共享基础设施上 |
| 中继可以拒绝服务 | 任何中继都如此；这不是机密性问题 |
| 二维码本身就是 bearer 凭据 | 五分钟过期、一次性，配对成功桌面立即删除 PSK |
| 能连到中继的人都能开 room | 在网络边界限速，把公开中继当作共享资源 |
| 手机凭据丢失 | 在桌面上解绑：它会删除 room；自建中继没有云端撤销可依赖 |

## 6. 运维要点

* **不启用 JetStream。** 远程控制只用 NATS Core，存储只会让密文和元数据在本不该
  留存的机器上堆积。
* **room 文件不是密钥库。** `rooms.json` 里只有 pair id、公开 NKey、设备名、时间
  戳和每设备两个 SHA-256。拿到副本也连不上：每张凭据都绑定到某个公钥，而对应
  私钥从不离开端点。删掉文件会一次性让所有配对失效，这本身是一种「重来」，也是
  「文件损坏报错、而不是当空表启动」的原因。
* **重启无代价。** 桌面端在下次凭据轮换时用自己的配对 NKey 重新注册同一个 room，
  已配对的手机自动重连。配对存在于两端的凭据里，不在中继里。
* **凭据短期有效。** 默认 JWT 有效期五分钟，续签是签名请求；中继侧没有长期
  bearer token。
* **一个邀请 = 一台设备**，与官方平台按配对计数的规则一致。桌面上的「添加设备」
  会创建另一个配对（独立 pair id、独立 room），而不是往同一个 room 里加第二台手机。
* **TLS 按部署决定。** 公网部署用公共 CA（或用 `--relay-ca` 指定自己的 CA）；局域
  网明文部署只在中继自己声明的 origin 上成立，不会放宽官方平台或 `https://` 中继。

## 7. 验证部署

Broker 级（需要 `nats-server` 可执行文件）：

```bash
FUTURE_RELAY_NATS_SERVER=/usr/local/bin/nats-server \
  cargo test -p future-cli --test relay_nats_auth -- --ignored --nocapture
```

它用生成的 JWT 以 operator 模式启动真实 broker，在进程内运行中继的 HTTP API，并
按**投递结果**断言 allow 列表：被禁止的 publish/subscribe 必须哪里都收不到，包括
`SUB >`、跨配对订阅、读别人的 reply inbox、抢 queue group。

端到端（需要真实运行的中继）：

```bash
future relay init --dir /tmp/relay --nats-url nats://127.0.0.1:14222
nats-server -c /tmp/relay/nats.conf
future relay serve --config /tmp/relay/relay.json

RELAY_E2E_RELAY_URL=http://127.0.0.1:8099 \
  cargo test -p future-host --lib remote::relay_e2e -- --ignored --nocapture
```

该测试跑的是真实链路：`remote::start()` 经中继生成 v3 邀请，client 角色认领并用
`Noise XXpsk0` 建立加密通道，presence 和 `list_sessions` 走加密通道，中继的 room
名册与实际一致，解绑后 room 被删除。
