# NATS 中继

> （[English](desktop-nats.md)）中继已在公网可达：

- dev/test：`test.future-os.cn`（客户端 `4222`，WebSocket `9090`）
- 生产：`future-os.cn`（客户端 `4222`，WebSocket `9090`）

当前远程控制（桌面端、移动端，以及仅面向测试平台提供的 Web 客户端）使用短时效、
按配对隔离的 NATS 用户 JWT。中继必须以 operator/account JWT 模式运行；旧的共享
token 不是多租户安全边界。

移动端在生产与测试环境都要求 `wss://`。上面的端口是部署侧监听细节，不是让移动端
走明文 `ws://` 连接的指引。桌面端直连服务端下发的 NATS 端点，其客户端即使在该端点
使用 `nats://` 方案时也强制校验 TLS，因此明文监听无法为**平台**配对服务。唯一例外是
自建中继签发的配对、且该中继把端点声明为 `nats://`：`connect_nats` 只对这种形状
放宽强制 TLS（见 [self-hosted-relay.zh-CN.md](../guide/self-hosted-relay.zh-CN.md)）。
无论如何签发，Future 平台和任何 `tls://` 端点仍然强制校验 TLS。此外单元测试会连接
进程内的明文假 broker，那条夹具路径与中继例外无关。桌面端的
信任根来自它自己写出的平台信任库 PEM 快照（`~/.future/app/relay-root-certs.pem`），
而不是让 NATS 客户端每次连接都去读平台信任库——在 macOS 上失败的正是这次读取，而
读取失败过去会直接让连接失败。该快照就是平台信任库本身，因此不改变任何信任判断；
删掉它没有任何代价。不要
把每一跳都描述为已加密，也不要向现有移动端客户端推荐明文 WebSocket URL。测试中继
部署中只使用测试数据。用户配对指引：[Remote](../wiki/zh/Remote.md)。

生产环境的规范模板与运维手册在 `../future-server`：

- `config/nats-jwt.conf.example`
- `docs/remote-control-deployment.md`

要把现有测试中继从旧的共享 token 配置切换过来，需要运维重建一次 NATS 容器，并同步
匹配 `platform-service` 部署。这次测试切换无需保留旧的 JetStream 数据。

## 自建替代方案

`future relay`（本仓库）是同一个中继去掉平台部分：它签发同样的「按配对限定」
NATS user JWT，走同样的 claim/token 端点，只是以 room 为单位而不是账号。桌面端
用 `futureos --relay URL`（或 `future headless --relay URL`）按**每次启动**选择它，
不写任何文件——不带这个参数就是官方平台；选择本身按配对记录，因此官方配对与自建
配对可以在同一台机器上共存。运行手册、威胁模型与
验证方法见 [../guide/self-hosted-relay.zh-CN.md](../guide/self-hosted-relay.zh-CN.md)。

`future relay init` 生成的 broker 配置就是本文生产模板描述的那套，只是去掉了账号
层：operator JWT、内存 resolver、不启用 JetStream。

## 遗留本地环境

`desktop/nats/` 下的 `nats.conf` 与 `docker-compose.yml` 仅用于隔离测试较旧的共享
token 客户端：

```bash
cd desktop/nats
docker compose up -d
docker compose logs -f nats
docker compose down
```

绝不把这套遗留本地配置暴露到公网。它不是测试 JWT 主题隔离的有效环境。
