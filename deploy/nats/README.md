# 本地 L0 dev NATS（远程控制联调用）

> L0 = 无鉴权，**仅限本地/受控网络，勿公网裸跑**。详见 `docs/remote-control-*.md`。

## 前置
- Docker（Docker Desktop 或 colima）：`docker version` 能输出即可。
- `nats` CLI（建流/验证用）：
  ```bash
  brew install nats-io/nats-tools/nats
  # 验证：nats --version
  ```

## 起 NATS
```bash
cd deploy/nats
docker compose up -d
docker compose logs -f nats   # 看到 "Server is ready" + JetStream 启用即可，Ctrl-C 退出日志
```

## 验证连通 + JetStream
```bash
nats server check jetstream        # 默认连 nats://localhost:4222
# 或浏览器打开 http://localhost:8222 （监控页）
```

## 建 dev 事件流（对应一个 pairId=DEVPAIR）
```bash
nats stream add EVT_DEVPAIR \
  --subjects 'p.DEVPAIR.evt.>' \
  --storage file --retention limits --discard old \
  --max-age 30m --max-bytes 64MB --max-msg-size 1MB --dupe-window 10m \
  --defaults
nats stream ls                     # 应看到 EVT_DEVPAIR
nats stream info EVT_DEVPAIR
```

## 快速手测 pub/sub（可选，确认链路）
```bash
# 终端 A：
nats sub 'p.DEVPAIR.evt.>'
# 终端 B：
nats pub p.DEVPAIR.evt.s1 'hello'
# 终端 A 应收到 hello
```

## 停止 / 清理
```bash
docker compose down          # 停容器（保留 JetStream 数据卷）
docker compose down -v       # 停容器 + 删数据卷（彻底清空）
```
