# Streaming 会话进入同步耗时

## 2026-09-14：测量与优化

### 测量边界

本轮没有连接的 Android 设备，且本机无可用 iOS 模拟器，因此**不是手机端到端实测**。

- 延迟基准运行实际 `SyncEngine → fetchEventsSince → requestReadPage → applyReplayEvents`。
- RPC 使用受控 mock，Jest 虚拟时钟给每次 RPC 注入固定延迟。分页模拟现有 Desktop 的 100 个默认事件数及 512 KiB 事件页字节上限。
- 工作负载为 10,000 个事件（一个 `agent_start`、9,999 个短文本 delta），没有丢包、重试或工具大输出；同步完毕后 run 仍处于 streaming。
- 虚拟时钟数据反映串行 RPC 等待，不包含真实网络、服务端数据库查询、加解密、手机 CPU、React Native 渲染和动画成本。
- 本地回放耗时另用已有 `batchedReplay.test.ts` 的真实时钟测量，仅代表本机 Jest/JS 环境，不代表 Hermes 或真机。

### 瓶颈

`useConversationController.selectSession()` 调用 `SyncEngine.open()`。进入会话的完整同步依次执行：

1. `get_state`：确定当前 active run。
2. `get_session_entries`：加载最近三个用户 exchange 的历史。
3. `get_events_since`：从 `sinceIdx = -1` 回补该 run，直到固定 watermark。
4. 应用回放及排队的 live 事件，完成后才清除同步提示。

已有缓存也需要确认最新内容；目前 `open` 和 `reconnect` 都走完整刷新，不能因为缓存可读就提前宣布同步完成。

Mobile 原来没有给 `get_events_since` 指定 `limit`，因而落到 Desktop 的通用 **100 项/页** 默认值。10,000 个小事件产生 **100 次串行 RPC**，即使每次只有 150ms，也仅回补就需要 15 秒。首次历史通常早已显示，所以表现为“内容已经有了，仍长时间显示正在同步”。

代码依据：

- `mobile/src/remote/syncEngine.ts`：`open`、`runReconcile`、`isFullReplay`。
- `mobile/src/remote/replay.ts`：串行分页循环及 watermark 固定。
- `desktop/src-tauri/src/remote_host/business.rs`：`DEFAULT_MESSAGE_PAGE_LIMIT = 100`、`MESSAGES_PAGE_BYTES = 512 * 1024`、`paginate_events`。
- `agent/src/rpc/protocol.rs`：超出内存 ring 的运行记录可以从持久 journal 回补，故回补数量并不限于 2,000 个 ring 事件。

### 本轮修改与结果

Mobile 显式请求 **1,000 个事件/页**。不改变 Desktop 的独立字节限制、分块读取、固定 watermark、切换会话取消检查、重试及同步完成条件。工具输出较大时，服务端仍可返回小于 1,000 个事件的页面，客户端继续按游标读取，不能以页面不足 1,000 个事件作为结束条件。

| 每次 RPC 注入延迟 | 原回补请求数 | 新回补请求数 | 原同步完成 | 新同步完成 |
| --- | ---: | ---: | ---: | ---: |
| 20ms | 100 | 10 | 2,040ms | 240ms |
| 150ms | 100 | 10 | 15,300ms | 1,800ms |

以上总等待包含状态与历史两个 RPC；分别为 102 次与 12 次 RPC。受控场景下等待减少约 **88.2%**。历史预览首次提交时间没有改变，分别为 40ms / 300ms；这里的“提交”不等于屏幕实际完成首帧。

独立回放基准一次采样：10,002 / 50,002 / 100,002 个文本事件约 **39 / 201 / 393ms**，每组仅生成一次渲染快照，并合作式让出 JS 执行权。这说明该工作负载下，累积网络往返比本机文本回放更值得优先优化；不能据此排除真机复杂工具内容或 Markdown 渲染瓶颈。

### 复现

在配置好仓库依赖后，从 `mobile/` 执行：

```sh
npm test -- --runTestsByPath src/remote/__tests__/streamingSync.perf.test.ts
npm test -- --runTestsByPath src/remote/__tests__/batchedReplay.test.ts
```

`streamingSync.perf.test.ts` 验证完整文本、最终游标、streaming 状态及只需 10 次回补请求。旧版对照为加入 `REPLAY_PAGE_EVENTS` / `limit` 前运行同一基准的记录；当前测试的 10 次请求断言用于防止重新退回通用小页面。

### 真机诊断与后续边界

新增日志前缀：`[remote] session timeline sync timing`。

- 每次 reconcile attempt 记录 `reason`、`attempt`、`elapsedMs`、`stagesMs`、`outcome`，用单调时钟计时。
- `stagesMs.get_state`、`history`、`replay` 区分状态、历史与回补阶段；`replay` **包含网络分页及本地回放**，不是纯网络计时。
- `outcome` 区分成功、失败与已切换会话的旧工作。
- 开发版记录每次 attempt，发行版只记录耗时至少 1 秒的 attempt；只含标识符和计时，不含聊天内容或认证凭据。
- 单个 attempt 的总耗时不包含先前排队、attempt 之间的退避、之后的 live 队列应用及 UI 绘制，不能直接当作提示条完整可见时长。

真机复测应覆盖：冷开、缓存重开、长工具输出、Wi-Fi/蜂窝、丢包重试、切换会话及 run 在读取过程中结束。继续慢时先看分阶段日志：

- `get_state` 慢：检查 Desktop 响应及连接恢复；现有请求一次超时 10 秒，传输重试仍可能产生长等待。
- `history` 慢：检查大 exchange 及 192 KiB 快照分块传输。
- `replay` 慢：检查事件规模、大工具数据、网络/加解密与手机 JS 处理。

本轮未改变缓存前缀复用或并行分块读取：前者须确保历史、cursor 和 projector 一致，后者需测量真实设备上的吞吐与瞬时负载。不能为了缩短提示时间而略过完整性校验或提前隐藏提示。
