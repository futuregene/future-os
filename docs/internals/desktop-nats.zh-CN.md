# NATS 中继

> （[English](desktop-nats.md)）中继已在公网可达：

- dev/test：`test.future-os.cn`（客户端 `4222`，WebSocket `9090`）
- 生产：`future-os.cn`（客户端 `4222`，WebSocket `9090`）

当前桌面端／移动端／Web 的远程控制使用短时效、按配对隔离的 NATS 用户 JWT。
中继必须以 operator/account JWT 模式运行；旧的共享 token 不是多租户安全边界。

移动端在生产与测试环境都要求 `wss://`。上面的端口是部署侧监听细节，不是让移动端
走明文 `ws://` 连接的指引。桌面端直连配置的 NATS 端点；该链路上的 TLS 由部署控制，
客户端并不无条件强制。请显式保护好这条链路。不要把每一跳都描述为已加密，也不要向
现有移动端客户端推荐明文 WebSocket URL。测试中继部署中只使用测试数据。用户配对
指引：[Remote](../wiki/zh/Remote.md)。

生产环境的规范模板与运维手册在 `../future-server`：

- `config/nats-jwt.conf.example`
- `docs/remote-control-deployment.md`

要把现有测试中继从旧的共享 token 配置切换过来，需要运维重建一次 NATS 容器，并同步
匹配 `platform-service` 部署。这次测试切换无需保留旧的 JetStream 数据。

## 遗留本地环境

本目录下的 `nats.conf` 与 `docker-compose.yml` 仅用于隔离测试较旧的共享 token 客户端：

```bash
cd desktop/nats
docker compose up -d
docker compose logs -f nats
docker compose down
```

绝不把这套遗留本地配置暴露到公网。它不是测试 JWT 主题隔离的有效环境。
