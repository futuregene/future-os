# UI harness：像人一样观察产品的端到端测试

UI harness 驱动真实的桌面前端，跑在真实数据库的快照上，记录所有可观测通道，
再用视觉模型和文本模型把证据变成问题清单、测试报告，以及需要人工确认后才提交的
issue 草稿。

用法、用例格式、观测通道、分析流程、保真边界和当前缺口以
[`harness/README.md`](../../harness/README.md) 为准。本页只说明它和现有工具的关系，
以及怎么在一台已经装好应用的机器上跑起来。

## Surface

| Surface | 运行什么 | 数据 |
|---|---|---|
| `desktop` | 真实桌面前端，headless Chrome | 真实 `app.db` 副本 |
| `mobile` | 真实手机端 UI（Expo web），触摸输入 | 手机 shot mock |
| `tui` | 真实 `future tui`，跑在 pty 里 | 脚本化 `demo_agent` |
| `cli` | 真实 `future` 二进制，隔离 `FUTURE_HOME` | 真实 `app.db` + 设置快照 |
| `remote` | 同一个用例里同时开桌面端和手机端 | 两端 shot mock |

`remote` 证明两端对同一段会话的渲染一致，不是真实中继联调；`tui`/`mobile` 验证交互行为，
不验证模型或网络保真；`desktop` 和 `cli` 跑在真实数据库快照上。

## 它处在哪里

[截图工具](screenshots.zh-CN.md) 也渲染同一个前端，但它回答的是「这个界面长什么样」：
用固定的演示数据，一个场景出一张图。UI harness 回答的是另一个问题：「一个人在产品里
连续操作一段时间，会发生什么？」因此它：

- 复制 `~/.future/app/app.db`，而不是使用 `desktop/shot/mock/data.ts`；
- 在步骤之间保留状态，同时对 DOM 和数据库做断言；
- 记录时间线、录像、控制台、网络、IPC 和性能通道，而不只是静帧；
- 把失败的用例当作要报告的缺陷，而不是把断言改松的理由。

两者共用同一个页面（`desktop/shot/`）、同一份 Vite 配置和同一套无障碍优先的元素匹配，
所以在一边能找到的控件，在另一边也能找到。

## 运行

```bash
node harness/cli.mjs prepare
node harness/cli.mjs list
node harness/cli.mjs run --case create-chat-and-delete
node harness/cli.mjs analyze --run harness/runs/<id> --mode full
node harness/cli.mjs report  --run harness/runs/<id>
```

依赖：Node.js、Chrome/Chromium、录像需要 `ffmpeg`；分析步骤需要一个 release 版本与
当前 agent 匹配的 `future` 二进制（不匹配时用 `FUTURE_BIN` 指定）。Python 3 只被
共用的 Vite 启动脚本使用，harness 会自己起服务。

## 它不能证明什么

后端 Rust/SQL 行为、模型回答质量、真实网络条件和移动端都不在这个 harness 的范围内。
它是一个使用真实形状数据的 UI 行为 harness，不能替代集成测试或契约测试。报告里会
重复这条边界，避免读者把一次绿色运行误解成「后端没问题」。
