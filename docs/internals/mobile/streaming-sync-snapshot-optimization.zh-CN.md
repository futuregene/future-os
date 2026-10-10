# 冷启动／缓存失效：快照 + 增量优化及真实 A/B（2026-09-16）

## 已完成的行为修改

正常完整缓存重开继续使用原有 highWater 增量路径。没有可用基线、需要从 `-1` 恢复时，移动端不再默认下载当前 run 的全部 token 事件：

1. 保留现有历史加载：最近 **3 个用户 exchange**；向上翻页仍走历史接口。
2. 首次 `get_events_since` 带 `preferSnapshot: true` 与 `chunkedRead: true`。
3. 新 Desktop 请求 Agent 的 `get_run_snapshot`，拿到已合并的语义结果及一致游标 N。
4. Mobile 再从 N 读取一段增量，固定其边界 M，覆盖下载快照期间产生的事件。
5. 将“快照 + N…M 的完整增量”恢复为一个 projector，和游标一起提交。已排队 live events 随后按游标去重，终止状态和统计只应用一次。

这不是“截取最后几条 token”或“直接隐藏同步提示”，也没有修改保存的历史/模型上下文。它保留当前 run 的完整可见语义状态，跳过传输和逐个处理其生成过程中的大量中间增量。

**仍然存在的边界**：三轮历史不等于三条原始消息，也不等于严格的字节上限。当前 run 的大工具输出仍在其快照内，本轮没有新增工具详情懒加载。旧端或超过快照预算的特殊情况仍兼容回退到原始分页回放，不能声称所有情况下都绝不全量下载。

## 真实 A/B 结果

使用同一 SQLite 一致性备份、同一个新 Agent 可执行文件、同一个新 DesktopHost 测试程序和 Chrome，比较两种模式：

- `raw-baseline`：仅强制 `preferSnapshot=false`，走原有全量原始事件回放。
- `snapshot`：启用新路径。不是拿新 release 版与旧 debug 版比较。

从 236 个 completed run 中按事件数选择最大、第二大和中位三个样本，每组各测三次，共 18 次。第二轮倒置模式顺序，避免永远先测 raw。每次新建 SyncEngine，没有复用客户端前缀；未清空 OS 文件缓存。

| 原始事件数 | 原始回放耗时 | 快照 + 增量耗时 | 回放路径请求数（含分块） | 恢复的语义记录数 |
| ---: | ---: | ---: | ---: | ---: |
| 111,395 | 12.790–12.846 s | **0.403–0.428 s** | 112 → **6** | 787 |
| 65,632 | 7.487–7.607 s | **0.267–0.278 s** | 66 → **5** | 474 |
| 2,244 | 0.226–0.237 s | **0.049–0.060 s** | 3 → **2** | 60 |

最重样本同步耗时中位数 **12.8211 s → 0.4109 s**，下降约 **96.8%**（约 31 倍）。该样本回放路径实际响应体积 **38,782,350 → 1,204,056 bytes**，减少约 **96.9%**。

请求数口径：最大样本是“1 个快照请求 + 4 个后续分块请求 + 1 个增量核对请求”，合计 6 次；另外有状态和历史各一次请求。4 个后续分块通过 #647 的有限并发读取，不是再逐块串行。小样本无需分块，只有快照和增量两个请求。

字节数由浏览器 `arrayBuffer().byteLength` 实测，包含分块 base64 与应用 JSON 封装，不含 HTTP framing，更不是 NATS/E2EE 网络字节数。

### 一致性检查

18 次都成功完成，并且：

- 最终 committed highWater 分别为 111394、65631、2243，prefixComplete 为 true。
- 新路径没有把原始事件作为重放结果传回 SyncEngine，而是交付可恢复投影。
- 原始和快照模式的完整 `timeline.items` JSON 哈希一致，涵盖这些样本的正文、思考、工具状态、附件与终态统计等可见字段。
- 同一份快照与二进制、交替顺序的三轮结果稳定。没有只验证“提示消失”而跳过数据完整性。

### 不夸大的测量边界

- Agent：Rust 1.97.0，`cargo build -p future-agent --release`；版本字符串 `future-agent 0.0.0-6809f472+local.dirty`。测量二进制 SHA-256：`6d938c79bee1da7d93b7360493ac9d4ef6380f7db29cf4f1bf7b01adc0a920ea`。
- Desktop adapter：`cargo test --no-default-features`，仍是 unoptimized + debuginfo。两组共用这一构建，绝对耗时和各阶段占比不能直接推广到发行版 Desktop。
- Chrome 153 / V8，macOS，本机 HTTP → 真实 DesktopHost → 真正的隔离 Agent gRPC → 真实数据备份。HTTP 替代 NATS/E2EE，未注入 RTT/假时钟、未节流。
- 样本是已完成 run，通过 SyncEngine 公开 reconcile API 明确指定回放目标，不伪造 activeRun。历史窗口是该会话备份时的最新历史，不是生成当时的窗口。
- 这是历史 run 从数据库构建快照的实测；持续运行中的当前 run 可以直接读取已维护的内存投影，这条路径有并发/一致性测试，但本轮没有冒充真机 live streaming 实测。
- 没有测试手机 Hermes、React Native 渲染、蜂窝/Wi-Fi、NATS 加密与丢包。0.41 秒是本轮最大样本的中位数，不是手机 SLA 或所有会话的延迟上限。

## 协议与兼容

### Agent

新增只读 `get_run_snapshot`，使用现有 RpcCommand 的 `type/sessionId/runId` 与通用 JSON response carrier；没有添加/复用 protobuf 字段。已加入共享 command policy 的 SafeRead/Storage 分类，且不加载 LLM 历史上下文。

返回形式：

```json
{
  "runSnapshot": true,
  "events": [],
  "watermark": 1000,
  "nextSinceIdx": 1000,
  "hasMore": false,
  "projection": {
    "runId": "r",
    "cursor": 1000,
    "events": [
      {"type": "agent_start", "runId": "r", "idx": 0, "data": "{}"},
      {"type": "text_chunk", "runId": "r", "idx": 1000, "data": "{\"text\":\"已合并的回复\"}"}
    ]
  }
}
```

- 当前 run：在同一个事件 stamping 锁内克隆内存投影和游标，避免“新游标配旧正文”。
- 较早 run：一次 SQLite 事务读取一致前缀，在 Agent 内批量合并，**不把整个原始日志下载给手机**。读取/合并历史时释放当前 run 锁，不把历史折叠放在 active broadcast 临界区里。
- 恢复前验证原始索引连续和 run 身份；已知 journal 缺段或健康错误不会伪装成完整快照。
- 批量折叠与已有 live projection 的语义相同：合并相邻同流文本/思考/工具 delta，保留边界、工具终态、审批、usage、error 和 agent_end；原始 provider `text_delta` 与 `text_chunk` 重复，沿用已有投影规则排除。批量合并每个 delta 只解析一次、每段只序列化一次，避免历史回放的增长字符串反复序列化。

### Desktop / Mobile

- 仅 `sinceIdx=-1`、首个非 pinned 请求、同时支持分块的 opt-in 客户端启用快照。
- 大快照继续走已有不可变 `readChunk`，保留 session/run/bridge owner 校验。
- Mobile 验证快照身份、单调事件索引和 snapshot watermark，再开启独立的固定 watermark 增量窗口；尾部必须严格连续并到达边界。
- 快照和增量中途失败、切换会话、缺段或身份改变时，不提交部分 projector/cursor。tail 中遇到另一份替换快照会重试恢复，而不是混合两个基线。
- 修复了此前 projection 分支可能把历史中的同一条 user_message 再追加一遍的问题：按身份合并，保留权威历史附件。
- error 已使 projector 停止时，不因尚未收到 agent_end 而强行恢复 streaming=true。
- 原有同步日志成功采用投影时会标记 `replayPlan.mode="snapshot"`；原始分页为 `full`，健康缓存为 `incremental`。失败 attempt 仍可能保留初始的恢复计划，不能只看计划字段判断失败前传输了什么。

### 明确回退，不悄悄降级

- 旧 Desktop 忽略 `preferSnapshot`，原有分页照常工作。
- 新 Desktop 遇到旧 Agent 的明确 `unknown command: get_run_snapshot`，回退原有分页。
- 快照无语义事件，或序列化结果超过 **8 MiB** 预算，Agent 返回明确代码，Desktop 可回退原有分页。这样不突破 gRPC/16 MiB 传输快照上限，也不截断快照后假装游标完整。
- 其他网络、存储或快照校验错误不触发静默全量下载，仍按现有失败恢复机制处理。
- 这意味着旧端及异常超大快照仍可能较慢。本轮未完成“任意大小单轮内容都只下载最后几条”的分段投影/工具详情懒加载协议。

## 验证记录

- Mobile：类型检查、lint、95 个套件 / **1,322 个测试**通过。
- Agent：fmt、clippy `--all-targets -D warnings`，1,772 个 lib 测试及该 crate 的其他默认测试目标通过（原有忽略项保留）。
- Desktop Rust：fmt、clippy `--all-targets -D warnings`；默认 GUI 特性的 1,246 个 lib 测试、main 与 headless_cli 测试通过。新测试还直接经过真实 bridge 分块路径。
- RPC：115 个 lib 测试及 wire roundtrip/additive compatibility 测试通过；共享 command policy 的消费者 CLI、channels、loop、TUI 的默认测试目标也已检查。
- 一次多 crate 串联命令在 200 秒上限被中断，后续串联在 400 秒上限中断；它们不是测试断言失败。已只补跑未完成的目标，最后的 TUI cli_smoke 单独通过（5 个测试），doc tests 亦完成。没有把超时命令直接记为全绿。
- 新覆盖：并发 cursor/快照一致性、run 切换、持久历史与内存投影一致、坏 journal 拒绝、空/超大明确回退、旧 Agent 兼容、分块 owner 边界、无增量、增量缺段/错误 run、终止事件与 live 重叠去重、工具/思考恢复、用户条目去重、失败时保留 UI 和旧游标。

## 复现与产物

- `scripts/measure/measure-sync-snapshot.ts`：A/B 浏览器入口。
- `scripts/measure/measure-sync-browser.py`：隔离启动器，新增 `--agent-binary` 可指定新构建的独立 Agent；不会替换系统安装或停止用户的 agent。
- `streaming-sync-snapshot-ab-2026-09-16.json`（[已归档](../../archives/verification/streaming-sync-snapshot-ab-2026-09-16.json)）：18 次去身份指标及二进制信息。

```sh
# 在本分支 worktree 中，用固定工具链构建
cargo build -p future-agent --release
cd desktop/src-tauri
cargo test --no-default-features --lib serve_real_snapshot --no-run
cd ../..
node_modules/.bin/esbuild scripts/measure/measure-sync-snapshot.ts --bundle --platform=browser --outfile=target/sync-browser-measurement/bundle.js
python3 scripts/measure/measure-sync-browser.py --test-binary <上一步test executable绝对路径> --agent-binary <新future-agent绝对路径>
```

浏览器打开 ready.json 的 URL，点击 A/B 按钮。结束后停止本次 runner，确认其两个子进程停止及私有 SQLite 副本删除；保留去身份指标，清理临时日志与 bundle。
