# 已有缓存重开：真实历史 trace 对照（2026-09-16）

## 要回答的问题

“已经显示一部分内容，是否可以只读其后的事件？”分成两个不同条件：

1. 当前 run 的完整 projector 状态、可见条目、连续游标与可信基线仍在：可以增量。
2. 只有展示数据，或已收到使基线失效的通知：当前代码必须重建，不能把“有文字”当成“已应用全部前缀”。

本轮验证这两条实际代码路径，而不是再次估算全量开销。

## 方法与边界

沿用 `streaming-sync-browser-measurement.md` 的隔离 Agent、真实 DesktopHost、Chrome 和 loopback HTTP 测量工具。桌面适配器仍是 debug 测试构建，NATS/E2EE、手机网络、Hermes 与原生 UI 不在测量范围内。

新入口 `scripts/measure-sync-warm.ts` 复用生产 Mobile SyncEngine、分页、游标和 projector。数据来自同一条 111,395 事件的已完成历史 run。该次数据库快照共有 231 个 completed run。

与上一轮只指定历史目标不同，本轮**明确加入以下场景控制**，不能称为正在 streaming 的真实会话抓包：

- `get_state` 仍经过真实 RPC，但测试适配器把 activeRun 指定为选定历史 run。
- 首先仅让事件 0…111193 可见，再放出 111194…111393 共 200 个真实后续事件；保留最后的 `agent_end` 不投递。
- 测试适配器对真实回放页过滤截止点并设置一致 watermark；不构造假 token、不注入 RTT、不使用 fake timers。
- 选定 run 在当前历史页中的终态 assistant 镜像被排除，避免历史终态推翻测试设置的 activeRun。其他当前历史行保留；这不是历史时刻精确还原的完整历史窗口。
- 首页仍走真实服务端无 watermark 的读取路径，后续分页沿用生产游标循环。
- 受控失效场景显式发送隐藏会话 `resend`；展示缓存场景保留同一批可见条目，刻意移除 cursor/projector/可信基线。

对每个场景各测三轮，记录真实时钟耗时。每轮都检查请求起点、事件总数、最终 highWater、prefixComplete、streaming、projector 存在性；用完整 timeline.items 的 SHA-256 验证增量、空增量和全量恢复后的显示数据一致，不输出会话正文。

## 三轮结果（增加诊断字段前）

| 场景 | 回放起点 sinceIdx | 应用事件数 | 回放请求 | 同步完成 |
| --- | ---: | ---: | ---: | ---: |
| 冷开，建立前缀 | -1 | 111,194 | 112 | 12.787–12.838 s |
| 完整缓存重开，新增 200 个事件 | 111193 | 200 | 1 | **57.3–65.6 ms** |
| 完整缓存重开，无新增事件 | 111393 | 0 | 1 | **38.5–41.2 ms** |
| 隐藏时收到显式失效通知后重开 | -1 | 111,394 | 112 | 12.667–12.723 s |
| 仅保留同样的可见内容，无恢复状态 | -1 | 111,394 | 112 | 12.644–12.803 s |

每次另有一次状态请求、一次历史请求；所有回放均无分块读取。新增 200 事件场景实际 HTTP 回复约 65.8 KB，全量恢复约 38.8 MB。

**字节数口径**：HTTP JSON 回复在测试截止点过滤前计数，因此可能包括被控制器暂缓投递的末尾事件。例如空增量场景，真实 Agent 仍返回了随后被测试保留的 terminal event，读取响应 584 bytes，但向 SyncEngine 应用的事件数为 0。不能把这个数当成真实活跃 run 的空响应大小。

该样本当前生产缓存预算估计约 0.95 MB（不是整个 JS heap）。离开时实际调用默认 `pruneCache("another-session")`，没有发生淘汰。只验证了此样本、这一缓存占用，不能排除多会话竞争、超过 8 个会话或其他大内容导致的淘汰。

## 结论

- 正常完整缓存的增量复用已经有效：111k 前缀不会重读，只补后续 200 个事件。
- “仍有相同可见内容”与“可以安全增量续接”不是同一个状态。即使可见条目完全相同，基线失效仍可触发全量。
- 本轮没有复现正常缓存导航错误地全量回放，也没有观察到该样本被默认缓存预算淘汰。
- 人工触发的失效对照解释了可能出现的开销，但**不能认定用户真实慢会话就发生了这个失效**。
- 保留已有完整缓存策略；不要继续扩大分块/事件页预算，也不要删除基线校验来制造“秒开”。

首次打开或只能拿到展示数据的路径仍有真正的优化空间：由服务端返回可恢复的 run 投影快照及与其一致的 cursor，客户端恢复后只接后续事件。Agent 已有内存语义投影实现，可作为后续设计依据；仍需验证原子水位、工具参数/思考/终态完整性、run 切换、旧端兼容及快照体积。本轮没有新增该协议，也没有把普通历史文本冒充这种快照。

## 本轮代码修改：只增加可定位的诊断

现有 `[remote] session timeline sync timing` 日志增加可选 `replayPlan`，在选择读取方式时记录，而不是用完成后缓存倒推：

```json
{
  "mode": "full",
  "sinceIdx": -1,
  "cachedHighWater": 111393,
  "prefixDecision": "baseline-untrusted"
}
```

`prefixDecision` 返回第一个未满足的条件；它不是完整的生命周期溯源：

| 值 | 含义 |
| --- | --- |
| `reused` | 完整前缀复用，增量读取 |
| `no-cache` | 没有缓存 timeline；可能首次打开、清空或淘汰，单凭该值无法区分 |
| `baseline-untrusted` | 有内容但未建立可信基线，或已有基线失效 |
| `run-inactive-or-changed` | 服务端 activeRun 与缓存不一致，或 run 已不活跃 |
| `missing-projector` | 没有可恢复的累积状态 |
| `not-streaming` | 缓存或 projector 已不是 streaming |
| `prefix-incomplete` | 游标前缀不完整 |
| `missing-assistant` | projector 对应的可见 assistant 条目不存在 |
| `truncated` | 当前 run 有截断标记 |
| `reconcile-requires-full` | 当前 reconcile 原因不属于允许前缀复用的 open/reconnect |

- 不改变原有复用资格、游标、回放或提示完成条件。
- 仅为 history-refresh 后选择的 replay 记录该字段；idle 仅刷新历史、不读 replay 时没有 `replayPlan`，其他纯 tail reconcile 不声称作过完整缓存选择。
- 保持既有日志节流：开发版每次输出，发行版仍仅输出至少 1 秒的 attempt。没有聊天正文、认证信息或新传输协议。
- 增加字段后再跑一轮五种浏览器场景：正常增量为 `reused`；初次为 `no-cache`；显式失效与仅展示缓存为 `baseline-untrusted`。事件数量、请求起点、恢复后显示内容仍全部一致。这轮用于功能验证，不混进前三轮性能范围。
- Mobile 类型检查、lint、94 个测试套件 / 1,306 个测试通过；浏览器测量脚本单独通过 TypeScript 检查。

原始去身份指标见 `streaming-sync-warm-measurement-2026-09-16.json`。

## 复现

使用上一份报告中的隔离启动步骤，只替换浏览器 bundle 的入口：

```sh
node_modules/.bin/esbuild scripts/measure-sync-warm.ts --bundle --platform=browser --outfile=target/sync-browser-measurement/bundle.js
python3 scripts/measure-sync-browser.py --test-binary <已构建的ignored测试可执行文件>
```

打开 ready.json 中的 URL 后点击按钮，默认三轮；在 URL 加 `#rounds=1` 可做一次五场景诊断验证。样本必须有足够长的真实前缀、连续游标和末尾 `agent_end`；测试不满足这些条件会失败，不会伪造成功。

测量后停止本次 runner（不要停止用户原有 agent），确认其两个子进程结束、私有数据库副本删除。保留去身份指标，清理临时日志和 bundle。
