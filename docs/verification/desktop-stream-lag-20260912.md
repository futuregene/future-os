# Desktop 流式输出延迟排查（2026-09-12）

## 结论与验证范围

- 调查会话：`20260912-111503-622625`；基线：`9452a3ed`；修复分支：`claude/desktop-stream-lag`。
- **已复现并修复**：生产 Markdown worker 引入 DOM 专用依赖而启动失败；持续落后的 worker 结果被全部丢弃；开发环境 StrictMode 重建 worker 后保留 busy 状态。
- **支持但未做完整端到端归因**：快速、较长的推理输出会放大前端解析负担。没有证据证明这些 DeepSeek 会话发生了缓存溢出。
- 验证由本次实现者自行执行，不是独立审查。没有调用付费模型，没有重启、替换现有 agent/desktop，也没有修改真实会话数据库。
- 本地检查完成；安装后的 Windows WebView2 + DeepSeek 实时复测仍待进行。不能将下列受控实验等同于所有用户卡顿已消失。

## 1. 生产 worker 启动即失败

`streamingMarkdown.worker.ts` 经 remark 依赖到 `decode-named-character-reference@1.3.0`。Vite 的 browser 条件选择 `index.dom.js`，该模块顶层执行 `document.createElement('i')`。Web Worker 没有 `document`。

原始证据：

- 主工作区 `desktop/dist/assets/streamingMarkdown.worker-C4VrdgNY.js`，构建时间 2026-09-12 11:00:52，包含该调用。
- 用新增检查脚本执行这个**实际生产 bundle**，退出码 1：`ReferenceError: document is not defined`。
- 本地 Chrome 153 / Vite 页面也记录到三次相同 worker 错误，成功响应数为 0。随后 hook 达到 `MAX_WORKER_FAILURES = 3`，持续走同步解析。
- 这不是只对某个模型成立的缺陷；模型输出速度和累积长度影响退化后的代价。

修复：Vite 精确 alias 该依赖到 Node 解析出的 DOM-free `index.js`，用于主页面和 worker（开发模式依赖预打包共享模块）。明确声明开发依赖，避免依赖偶然的传递依赖提升布局。

`npm run build` 现在自动执行 `scripts/check-streaming-worker.mjs`，在无 DOM 的上下文中启动真实 bundle 并检查一次分块响应。普通 Vitest/Node 模块测试选择的包导出与浏览器不同，原先无法捕获这个打包错误。

修复产物 `streamingMarkdown.worker-KqABNlCu.js` 通过无 DOM 启动和分块验证。

产物 SHA-256：

- 原产物：`87BDEC3879B6FE3956A9094DFC5F0AB1CAE0770B961A2F0B6D28AF5EC27C075B`
- 修复产物：`F40E5611968C71480A1E0167A217F76A8674F932CDC0CA3C169550B8DBA856D2`

## 2. Worker 结果饥饿

原 hook 只接收 `response.id === latestId`。每次文本更新都会增加 latestId，即使新任务只是排队。如果输入快于解析，所有实际完成的结果都过期。`provisionalProjection` 只能不断把新文本拼进旧尾块，主线程仍解析越来越长的文本。

修复：接受仍是当前文本前缀的完成结果，让已完成块持续推进，只有尚未解析的后缀留在可变尾块；非前缀的文本替换仍拒绝旧结果。

### 受控浏览器对比

真实 `MarkdownContent`、真实 parser worker；固定 60 次文本追加，总计 54,816 字符。人为让 worker 响应落后两个输入 tick，避免后台标签页定时器节流改变实验条件。两组均使用修复后的 DOM-free 打包配置，只改变结果接收条件。

| 指标 | 原 latest-only 条件 | 前缀结果接收修复 |
|---|---:|---:|
| 成功交付的 worker 响应 | 29 | 30 |
| worker 错误 | 0 | 0 |
| 分块数 | 1 | 116 |
| 可变尾块字符数 | 54,816 | 2,290 |
| 拼接内容与输入一致 | 是 | 是 |
| 观察到的最长 long task | 51ms | 未观察到 >=50ms long task |

这是单次受控机制对比，不是模型吞吐基准或原用户现场的延迟测量。固定 160ms 延迟的早期尝试受后台定时器影响，没有可靠形成持续落后；未用该尝试作为性能结论。

回归测试 `makes block progress when every worker response trails the token stream` 在原逻辑失败（分块数始终 1），修复后通过；另有文本替换防串写测试。

## 3. StrictMode 生命周期

开发入口启用 React StrictMode。cleanup 终止 worker，却没有清除 `activeRef`；重新 setup 时新 worker 认为旧任务仍在执行，只排队、不发任务。新增测试复现第二个 worker 收到 0 个请求，修复 cleanup 清除 busy 状态后收到 1 个请求。

这是开发模式的附加问题，不用于解释生产可执行文件的行为。

## 4. DeepSeek 原始会话统计与缓存假设

通过 SQLite URI `mode=ro` 加 `PRAGMA query_only=ON` 查询本机 agent/app 数据库，仅输出事件类别、时间、计数和错误，不复制提示词或输出正文。检查最近 8 个 `future/deepseek-flash` run：

- 按事件时间戳的自然秒统计，峰值 152–310 事件/秒；不能把事件数直接当 token 数，也不能用自然秒峰值排除更短的瞬时突发。
- `run-20260912-110338-665082`：27,491 事件，其中 23,264 个 thinking_delta、3,945 个 tool_delta、50 个 tool_end，**没有 text_chunk**。最终错误是达到 50 轮工具调用限制，不是正文丢在 desktop。agent 与 desktop 的 terminal 持久化时间差约 19ms；这不代表逐帧 UI 延迟。
- `run-20260912-111008-9382ed`：7,521 事件，其中 7,075 个 thinking_delta，约 27,061 推理字符，**没有 text_chunk**；最终由用户取消。
- 部分更早的取消 run 在 desktop 被记作 completed、agent 为 cancelled，是另一个终态映射现象，本次未修改。

代码中的实际容量/节流：

- agent 当前 run 重放环：2,000 事件；越界可从持久化 journal 重放，并非超过 2,000 就丢正文。
- broadcast 环：4,096 事件；落后过多会显式报 DataLoss 并重连。
- gRPC 单消息上限：32 MiB；desktop 请求按事件数/字节预算分页。
- desktop live projection 有 100ms 合并刷新，因此少量显示延迟是设计行为。

曾考虑分页不断追赶新增事件的假设，但此次记录规模/速率及明确的 worker 复现不足以支持其为主因；没有调整任何缓存大小。现有数据不含同步 UI 帧时间与订阅游标，仍不能绝对排除传输/背压问题。

## 检查与复现入口

在修复 worktree 的 `desktop/` 中：

```powershell
npm run lint
npm test
npm run build
```

结果：完整 ESLint + `tsc --noEmit` 通过；101 个测试文件 / 873 个测试通过；生产构建及新增 worker smoke 检查通过。构建仍有现有 CSS `::highlight` 优化警告；测试有 Node localStorage 实验性提示等非失败输出。

旧产物仍在时可独立复现打包失败（参数为 assets 目录）：

```powershell
node scripts/check-streaming-worker.mjs D:/future-os/desktop/dist/assets
```

关键永久回归用例位于 `desktop/src/features/markdown/useStreamingMarkdownBlocks.test.ts`。临时浏览器入口、数据库统计脚本和 CDP 读取脚本已清理，测试 Vite 服务已停止；没有留下模型任务或付费后台任务。

下一步：将修复构建为 desktop 可执行文件，在用户方便重启 desktop 时复测同样的长推理/工具调用场景；若仍有卡顿，再同步记录 agent idx、desktop 读游标和 WebView long-task 时间，定位剩余链路。

## 补充：运行中 A → B → A 后冻结

2026-09-12 同一调查会话中，用户补充了明确的会话切换步骤。以已包含 #564 的 `84fb84ab` 为基线，独立分支 `claude/desktop-reattach-stall` 复现并修复了另外两个状态恢复问题，**不依赖 Markdown、输出速度或事件缓冲容量**。

1. **A 仍在运行**：`threadMessageCache` 恢复了本地发送创建的 `pending-*` 流式气泡。`upsertStreamingPreview` 固定使用 `stream_<runId>` 查找气泡，`streamingBubbleBase` 看到另一个 UI id 的同 run 消息就当作已持久化的结束消息，从而拒绝每次更新。修复为按同 run 的 `streaming` 消息接管原 id，保留 DOM 身份和后续增量刷新。
2. **A 已在后台结束**：初次历史刷新未启用终态校准，`reconcileThreadHistory` 无条件保留缓存的 streaming 消息。新组件一开始读到的 recentRun 已是 terminal，不会再发生 active → terminal 转换来触发结束刷新。修复为初次加载时允许权威历史校准缓存；请求期间发生的新写入仍受原有 request-time baseline 保护。

复现入口：

```powershell
cd desktop
npx vitest run src/features/agent/useRunReattach.switch.test.tsx
```

测试使用真实 `useThreadMessages`、`useRunReattach`、投影器、缓存和 reconciliation；仅替换存储 IPC 与 Tauri 事件传输。通过 React keyed 挂载/卸载执行 A → B → A。原代码的两个初始用例均失败：切回后仍显示 `before switch`，后台完成后仍是 `streaming`。修复后验证：

- 接收离开期间的增量，切回后继续接收新事件；连续两次切换仍保持单个原 id 气泡。
- A 的内容不进入 B。
- 后台 `completed`、`failed`、`cancelled` 三种状态都能结束缓存的 streaming 状态。

本轮最终检查：102 个测试文件 / 877 个测试、ESLint、TypeScript、生产构建、worker smoke 全部通过。属于前端集成复现与实现者自检，尚非安装版 WebView2 的点击端到端验证；没有修改真实数据库、调用付费模型、重启 agent 或替换用户正在运行的 desktop。
