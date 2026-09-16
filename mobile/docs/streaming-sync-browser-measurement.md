# 真实数据浏览器同步测量（2026-09-16）

## 结论与决策

这次直接运行移动端生产 TypeScript 同步代码，不用数据库体积推算 RPC 次数，也没有注入虚拟 RTT。浏览器可用于定位同步读取与回放的成本，但不能冒充 Hermes / React Native 原生渲染或手机网络的端到端测试。

三个已完成历史 run 各重复三次。每次新建 SyncEngine，没有可复用的前缀缓存；并未清空操作系统文件缓存。结果如下：

| 样本（按已完成 run 事件数选取） | 完整事件数 | 回放逻辑页 / 分块请求 | 同步完成实测范围 |
| --- | ---: | ---: | ---: |
| 最大 | 111,395 | 112 / 0 | 12.698–12.801 秒 |
| 第二大 | 65,632 | 66 / 0 | 7.544–7.587 秒 |
| 中位 | 2,228 | 3 / 0 | 0.239–0.256 秒 |

**决策：不继续根据旧估算扩大分块或事件页预算。优先对后端回放读取路径做分段测量，并在优化构建下复核。** 本轮最大样本的成本集中在回放而非历史，且所有样本均未走 `get_read_chunk`。#647 的分块并发对此次工作负载没有可发挥作用的请求，不代表它对大历史页或大 projection 无效。

这只是本轮样本最大观测值，不是产品最大延迟或手机网络上限。

## 实际运行链路

```text
Chrome 153 / V8
  → Mobile SyncEngine.reconcile(session, "open", explicitHistoricalRun)
  → Mobile fetchEventsSince / requestReadPage / timelineFromEntries / applyReplayEvents
  → 本机 HTTP requestRetry 适配器
  → DesktopHost.execute / PagedReply / paginate_events（真实 Rust 实现）
  → Agent gRPC（真实服务）
  → SQLite 一致性备份（真实历史）
```

- 代码基线：`937b70f2`。桌面适配器为 `cargo test --no-default-features` 的 **unoptimized + debuginfo** 构建；共享业务代码不经过 GUI 外壳。
- Agent 使用安装在本机的二进制：`future v0.0.2-a0373412+local.dirty`。未证明它等同于干净的 release 构建，不能把本结果推广到 release 性能。
- Chrome：153.0.8010.37，V8 15.3.76.10，macOS。无 CPU / 网络节流。
- 只读打开用户源数据库，通过 SQLite online backup 复制到私有临时 HOME。不复制认证信息；不改动、重启或停止用户运行中的 agent。测量 agent 使用独立的新 loopback 端口。
- 备份内有 229 个 `completed` run；按实际事件数选最大、第二大与中位样本。它们不是 live streaming run。通过 SyncEngine 公开 `reconcile` API 明确指定历史回放目标，`get_state` 返回真实状态，未伪造 activeRun。历史读取的是快照时该会话最新三个 exchange，不是历史 run 当时的历史窗口。
- HTTP 只允许本轮样本范围内的四种只读命令，绑定 `127.0.0.1`；POST 校验同源与自定义请求头。它替代了 NATS/E2EE，而不是复用了真实手机传输链路。
- 不模拟 token、不使用 fake timers、不注入 RTT、不发送模型请求。浏览器界面只显示测量指标，不渲染会话正文。
- 每次同步核对返回事件总数、最终 committed highWater、成功状态。生产 SyncEngine 自身仍执行连续性/水位与回放校验。

## 最大样本的分阶段结果

| 指标 | 三轮实测范围 | 含义 |
| --- | ---: | --- |
| 状态读取阶段 | 6.3–38.4 ms | 真实 `get_state` |
| 历史阶段 | 20.3–23.7 ms | 最新三个 exchange，一次响应 |
| 首次 timeline commit | 26.7–62.3 ms | 数据提交，不是屏幕首帧 |
| 首个回放请求 | 3.618–3.670 s | 浏览器发出至收到/解析首个回放响应 |
| 获取全部回放页 | 11.252–11.323 s | 含全部 112 个请求、JSON 解析及合并 |
| 回放请求后端处理总和 | 11.097–11.166 s | DesktopHost 执行、Agent gRPC 及响应 JSON 序列化；不含 HTTP 发送 |
| 完整回放阶段 | 12.670–12.775 s | 获取之后还包括校验、投影、合作式让出等 |

回放获取之后约 1.37–1.51 秒不是纯 CPU 指标：其中包含浏览器调度/让出，不能用来估算 Hermes 的 CPU 耗时。后端数字也混合了数据库读取、Agent 和 Desktop 的转换/序列化，尚未区分哪个内部步骤最重。

源代码可定位一个下一步测量点：`remote_host/business.rs` 首次 `get_events_since` 在没有 watermark 时走 `agent_bridge::get_events_since` 全尾读取；带 watermark 的后续请求走 `get_events_since_page`。本轮已测出首请求明显更慢，但未单独测量其内部 SQL/反序列化/全尾合并，不能据此直接宣称最终根因或优化收益。

## 实际字节数与旧估算纠错

浏览器对收到的 JSON 响应调用 `arrayBuffer().byteLength`，不是对 SQLite TEXT 调用 `length()`：

| 样本 | 所有回放响应 JSON 合计 | 最大单次响应 JSON | 当前历史响应 JSON |
| --- | ---: | ---: | ---: |
| 最大 | 38,782,350 bytes | 402,591 bytes | 238,002 bytes |
| 第二大 | 23,479,228 bytes | 400,398 bytes | 111,673 bytes |
| 中位 | 860,817 bytes | 396,425 bytes | 199,211 bytes |

这些数包括测试适配器的 `success/data/error` JSON 封装，不含 HTTP/NATS framing 或加密开销。最大样本真实响应体积约 37 MiB，与此前直接用库内记录估算的 22.37 MB 不同。

所有回放响应都低于 512 KiB 的分块触发阈值。**192 KiB 是启用分块后的块大小，不是触发阈值。** 之前“112 页 → 179 次 RPC”的推算错误；本轮实际是 112 次页请求、0 次分块请求。历史页也不能由 `block_records` 简单求和还原：本次走真实生产呈现路径，三个当前历史窗口都低于 512 KiB。

## 复现与清理

测量文件：

- `scripts/measure-sync-browser.ts`：导入真实移动端生产同步代码。
- `scripts/measure-sync-browser.html`：只显示指标的浏览器外壳。
- `scripts/measure-sync-browser.py`：SQLite backup、隔离 agent、只读 loopback probe 的启动与清理。
- `desktop/src-tauri/src/remote_host/sync_measurement.rs`：默认 ignored 的浏览器测试入口，不加入生产行为。
- 同目录 `streaming-sync-browser-measurement-2026-09-16.json`：去除会话/run 身份与正文后的九次测量记录。

在隔离 worktree 中、已有项目依赖时：

```sh
# 使用 rust-toolchain.toml 固定的工具链。
cd desktop/src-tauri
cargo test --no-default-features --lib serve_real_snapshot --no-run
cd ../..

# 可复用已安装的 esbuild；不需要 react-native-web 或新装依赖。
node_modules/.bin/esbuild scripts/measure-sync-browser.ts --bundle --platform=browser --outfile=target/sync-browser-measurement/bundle.js
python3 scripts/measure-sync-browser.py --test-binary <上一步打印的测试可执行文件绝对路径>
```

等待 `target/sync-browser-measurement/ready.json` 出现，打开其中 URL，点击测量按钮。脚本打印的 `runnerPid` 是本次创建的专用进程；测量后向它发送 SIGTERM，脚本会停止自己的两个子进程并删除私有数据库快照。也有 30 分钟服务生存时间上限。不要停止任何既有 agent。指标另存后删除本次 `target/sync-browser-measurement` 临时文件。

## 尚未测量

- 真正 live run 的缓存重开、接收 live events 与回补交叠。
- 原生手机 Hermes、React Native 时间线渲染和提示条实际可见时间。
- 实际 NATS/WSS、加密、蜂窝/Wi-Fi、丢包重连。
- 优化构建的端到端结果；本次 debug 桌面适配器会影响绝对耗时与各阶段占比。
- 全部会话的最大值：只测了按事件数选出的三个 run，事件内容、网络和重试也会影响耗时。
