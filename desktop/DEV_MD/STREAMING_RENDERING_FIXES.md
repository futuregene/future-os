# Desktop 流式渲染修复与影响记录

日期：2026-09-12。基线：`c5e87eec`；最初审查基线为 `26bea988`，两者相关 agent/markdown 前端代码无差异。开发分支：`claude/desktop-stream-fixes`。

## 结论与证据等级

- 5 类状态/竞态缺陷：有真实调用路径，使用真实 React hooks/pipeline/projector、受控 IPC Promise 复现。不是宣称已在原生桌面执行真实断网/休眠。
- Markdown 性能瓶颈：代码路径与浏览器生产构建基准共同确认；最终方案同时降低单表追加的主线程阻塞和排版就绪延迟。
- Worker 同步异常：故障注入确认降级缺口；未观察到当前正常打包环境实际触发构造异常，因此归类为防御性加固，不计为已确认的线上故障。
- 原生 Tauri 系统休眠/网络断开、macOS/Safari、Linux 的完整交互验证尚未执行。没有中断正在运行的 Agent，没有修改数据库或 RPC 协议。

## 修复清单与体验影响

| 项目 | 真实触发条件与修复前结果 | 修复 | 体验变化与证据 |
|---|---|---|---|
| 首次 session 绑定后收尾失效 | 新发送捕获旧 `refreshRecentRun`，发送期间 null session 绑定为实际 ID；source revision 变化使旧 callback 直接返回，气泡 complete 但 active run 仍 running，下一条被拒绝。初次绑定发生在发送期间是必要条件，并非每次新建都必现。 | callback 以 owner 校验调用资格，在每次调用时捕获最新 source version；仍丢弃跨 source 的在途读取。 | 回复完成后可以直接继续问，不用切走再切回来。`useAgentThreadState.lifecycle.test.tsx` 验证绑定、终态和第二次发送；原有 source-replacement/paging 测试验证旧 owner 仍不能写。修复前的受控场景推进 60 秒仍不恢复；修复后在收尾完成时恢复，不再依赖等待轮询。 |
| watchdog 误放行新发送 | 附件/引用处理期间新 run 尚未创建，focus/15 秒检查读到上一轮 completed；或旧检查返回时已有新发送。 | `localSendRef` 记录发送实例 identity 和 runId；未创建 run 不检查；按确切 runId 读取，并在 await 后重新验证实例。 | 避免切回窗口使任务失去 UI 所有权、重复提交尝试；仍保留 focus 对真正挂起发送的恢复。生命周期测试覆盖准备期、旧检查晚回、匹配的挂起发送。 |
| ContextPanel 停止后残留运行锁 | `ContextPanel.handleTerminateRun` 直接调用 abortRun，只刷新 context，不经过聊天的 handleAbort；pipeline cancelled 分支此前不更新 recentRun。 | cancelled 分支把已读取的权威 run 终态提交给当前发送者。 | 从右侧运行面板停止也能正常继续对话；不再出现 stopped 气泡配 running 锁。生命周期测试覆盖不调用聊天 handleAbort 的取消路径。 |
| reset 与增量读取竞争 | `resetRunProjection` 在 since=N 请求途中删除缓存；旧 tail 返回后被装进空 projector，前缀丢失。 | 返回后核对请求所用缓存实例；被 reset/LRU 淘汰则重新读取，不能把旧游标的 tail 写入新空投影。 | 重连/快照对齐不再让正文突然缺前半段。`threadRunProjection.async.test.ts` 复现 PREFIX+TAIL，验证本次和后续读取均保留完整正文。 |
| 迟到 attach/IPC 错误误报失败 | 后端 RunGone 分支先从持久化状态确定 completed/cancelled，随后仍返回 Err；旧 catch 保住数据库终态，却无条件把 UI 气泡标 failed。 | catch 优先尊重权威 success/cancelled，并尽力从 run events 恢复正文/segments，清除过期错误与重连状态。 | 避免“侧栏成功、正文失败”及错误的重试提示；取消仍显示 stopped。`sendPipeline.test.ts` 对 completed/cancelled 均验证正文、终态和不覆盖 DB 状态。 |
| live Markdown 仍在主线程解析 | worker 原来只返回字符串分块，`MarkdownContent` 对变化的整块再次同步解析；单个大表格没有分块收益。 | worker 返回完整渲染 AST；UI 直接消费。等待响应时以纯文本显示新增后缀；复用未改变块的对象 identity。初次 live 挂载不做无界同步解析。 | 长表格持续生成时 UI 不被每次整表解析阻塞。DOM 测试证明 live 挂载、追加、worker 回包和停止全过程主线程 parser 调用为 0。 |
| worker 创建/发送失败 | 构造器或 postMessage 同步 throw 不进入 onerror。正常部署中未实际观察到，只有故障注入证据。 | 捕获同步异常及 messageerror；区分旧 worker 的迟到回调；小文本受限同步 fallback，大于 8192 字符保留原文而不做无界主线程解析。 | 极端故障不致卸载会话或再次长时间卡 UI。代价：大文档在持续 worker 故障下显示纯文本，不能承诺仍有完整 Markdown 格式。 |

以上逻辑修复主要改善正确性和可用性，没有把测试耗时冒充为业务性能提升。

## 排版延迟优化：为什么不能只移到 worker

第一版返回 AST 后，主线程明显更顺畅，但 worker 仍对完整表格逐次重解析，排版就绪反而变慢。该失败尝试和原始样本保留在 `streaming-rendering-benchmark.json`，没有覆盖或隐藏。

后续实现 `createStreamingMarkdownProjector()`：

1. 每个 worker 保存一个单表 checkpoint，只保留上一版文本/渲染结果，不保存所有历史版本。
2. append-only 且仍为单个顶层 GFM table 时，保留已完成行，只解析表头及上一版最后一条可变行和新增内容。
3. 重新检查 fragment 的 AST；退出 table、文本替换、出现其他顶层块或 Future-reference context 时，回到完整解析。
4. 初次完整解析得到的 mdast 可直接交给共享 Markdown 转换器，避免同一整块重复 parse。
5. worker 生命周期、latest-only 排队、过期前缀兼容校验维持不变。相同文本 live→settled 不再重复解析表格。

这是针对已实测瓶颈的有限优化，不是一个假定所有 Markdown 块都可按行拼接的通用增量解析器。带引用定义的完整文档、嵌套/混合结构、大列表等仍使用完整解析，未宣称这些类型有相同数字的加速。

## 最终性能数据

来源：`streaming-rendering-benchmark-optimized.json`。Windows x64、HeadlessChrome 153、生产构建、document visibility 为 visible；每组排除初次挂载后，测 5 次追加一行。baseline 是原先单块同步 renderer，未计入原 boundary worker 的额外开销。

| 指标 | 54KB 同步基线 | 54KB 优化后 | 109KB 同步基线 | 109KB 优化后 |
|---|---:|---:|---:|---:|
| 排版就绪中位数 | 134.8ms | **13.4ms** | 608.9ms | **24.2ms** |
| 排版就绪最慢样本 | 161.5ms | **26.8ms** | 631.2ms | **43.6ms** |
| 同步推送耗时中位数 | 133.9ms | **2.5ms** | 597.7ms | **5.4ms** |
| 每次更新的最大计时器额外延迟，中位数 | 126.7ms | **1.0ms** | 592.4ms | **6.2ms** |

排版就绪中位数分别降低 **90.1% / 96.0%**。这里比较的是同一轮中的 baseline 与 candidate。机器负载/JIT 等使本轮绝对基线与上一轮不同，不能把不同轮次的绝对值拼成性能倍率。

边界与代价：

- 文本仍可能先短暂显示为字面 Markdown，再由 worker 整合成正确格式；不是静默丢弃新 token。
- 初次打开长表格仍需完整解析；上述数字不是冷启动时间。
- 仍需传输/渲染整份表格 AST，整体开销并非严格 O(新增字符)。优化的是昂贵的重复语法解析。
- 5 个样本不是生产 p95/p99，也不是原生 WebView 测量。未量化峰值内存、耗电、100Hz 持续输入和其他平台。
- 正常静态 Markdown 的 API/行为保持兼容。新增可选 mdast 参数要求调用方提供同一原文、同一插件生成的 AST。

## 验证

在 worktree 中执行：

- desktop `tsc --noEmit`：通过。
- `packages/markdown` `tsc --noEmit`：通过。
- desktop `eslint "src/**/*.{ts,tsx}"`：通过。
- desktop 全量 Vitest：**105 个文件、909 个测试通过**。
- 生产 Vite build：通过；构建器仍报告既有 `::highlight` CSS 优化警告。
- `scripts/check-streaming-worker.mjs`：通过，验证实际生产 worker 无 DOM 启动、AST、500+ 行表格追加和最终停止。
- 新增 18 个 projector 测试：逐字符将结果对照完整 parser；覆盖 CRLF/CR、不同缩进、代码/转义 pipe、中文、公式、链接、定义、表格终止、第二张表、文本替换、live→settled、30 批次连续追加。
- 已有 A→B→A 会话切换、history ownership、worker 排队/StrictMode/错误恢复测试保持通过。

没有运行 Rust 测试，因为未修改 Rust；也没有把 headless 浏览器基准标成完整原生桌面验收。

## 重跑基准

在 desktop 目录、依赖安装完成后，用 Vite JS API 构建专用页面（PowerShell）：

```powershell
@'
import { build } from 'vite';
await build({build:{outDir:'benchmark-dist',rollupOptions:{input:'scripts/streaming-markdown-benchmark.html'}}});
'@ | node --input-type=module
npx vite preview --host 127.0.0.1 --port 5190 --outDir benchmark-dist
```

打开 `/scripts/streaming-markdown-benchmark.html`。前台保持可见；每种 rows 先测 baseline 再测 worker。再次测 baseline 前必须重新加载页面，避免共享 parser 的缓存命中污染基线。结果在页面及 console 中；生产模式的 React Profiler 数字为 0，应忽略，使用 push/formatReady/heartbeat 三项。完成后删除 `benchmark-dist`。

原有审查及复现记录来自本任务会话；影响记录不意味着已安装新桌面程序或已合并到 main。

## 后续优化：停止文字输出引起的无效目录扫描

基线：合并后的 `11fabb04`（PR #570）。

真实调用链：`projectRunForLivePreview` 在历史 activityItems 非空时无条件发出 `file-tree-refresh`；`FileTreePanel` 虽将事件合并到每 2 秒一次，但 `useFileTree.refresh` 仍会重读根目录及所有展开目录。因此一次工具操作后，后续纯文本/推理输出会一直触发目录读取，与实际文件变化无关。

修复只依据**新摄入事件**决定失效：工具 start/end/result，以及可能包含中断写入的 agent_end/error。文字、推理、工具参数增量和空读取不再触发目录刷新。空增量读取同时复用已有投影 snapshot，而不是重新分配 segments/activityItems。

验证使用真实 `FileTreePanel`、`useFileTree`、事件总线与 run projector，替换 IPC 为 spy 并使用虚拟时钟：

| 场景 | 修复前 listDirectory 调用 | 修复后 |
|---|---:|---:|
| 初次打开 + 首个工具操作完成 | 2 | 2 |
| 随后 120 秒，每 100ms 一个文本/推理更新，共 1200 次 | **60** | **0** |
| 下一个工具完成后的刷新 | 1 | 1 |

单根目录的纯输出阶段消除 100% 的多余目录请求；不将此请求数变化宣称为 CPU、磁盘吞吐或 FPS 百分比。对于展开目录，实际一次 refresh 会读取根及展开目录，但多目录倍率未在这个 fixture 中实测。

新增测试 `features/filetree/FileTreePanel.streaming.test.tsx` 先在基线上复现 60 次非预期调用，再在修复后通过；另验证 tool_end/tool_result/agent_end/error 仍触发刷新、空读取复用 snapshot。保留现有 2 秒刷新合并和手动刷新行为，避免为省请求而漏掉真正工具操作的文件变化。本轮 desktop 类型检查、lint 通过，全量测试为 106 个文件、915 个测试通过。

## 后续优化：长列表/代码的 live 缓存与混合文档首开

基线：`3a66814e`（PR #571）。按用户要求，不做本机部署验收，不安装/重启 desktop，也不操作正在运行的 Agent。

### 已确认原因与改动

1. `parseFutureMarkdown` 的 512 条静态 LRU 原先也收录每个 live 版本。对于一个持续变长的列表/代码块，旧版本几乎不会再用，却一直持有完整渲染树。worker 的可变块显式绕过静态缓存。与并行进入 main 的共享解析器合并时，沿用其第三个 `cache` 布尔参数（传 `false`），不引入另一套 options 参数。静态默认缓存及已关闭的稳定块行为不变。
2. worker 现在只保留最新投影。相同文本的 live→settled→live 只调整 live 标记，所有文档类型都不再为此解析一次；不再只对单表有效。
3. 普通混合文档初次解析已有完整 mdast，原来又逐块从字符串重解析。现在直接转换对应子树。引用/脚注定义会影响跨块 tokenization，因此含这些上下文的分块保留原来的独立解析路径；不会为了快而改变已有渲染语义。
4. 定义检查只在确实需要复用多块子树时运行，并只下探能包含定义的 flow container；单个长列表/代码块不额外遍历整棵树。

### 测量结果与边界

脚本：`desktop/scripts/profile-streaming-worker.mjs`；最终成对汇总：`streaming-worker-memory-profile.json`。

测量在独立 Node v26.4.0 进程中执行实际生产 worker bundle（无 DOM），80 次追加后显式 GC。只保留最新回包，不把所有回包留在基准数组中。以下是 JS 保留堆和 worker 计算时间，不含消息结构化克隆、React 或原生 WebView。

| 输入 | 80 次追加后的保留堆：前→后 | 相同文本结束/恢复计算中位数：前→后 |
|---|---:|---:|
| 56,579 字符、800 项列表 | **78.171→1.159MiB（约 -98.5%）** | **240.561→0.003ms** |
| 52,508 字符、1500 行代码 | **8.776→0.674MiB（约 -92.3%）** | **18.286→0.004ms** |
| 10,852 字符、481 块混合文档 | 0.709→0.531MiB | **57.151→0.027ms** |

混合文档首个投影本次为 313.229→112.869ms；对应进程 CPU 为 453→203ms。冷启动只采一个样本，不能当作稳定 p95 或普遍首开加速比例。确定性收益是复用整文解析得到的 481 个子树，而不是再执行 481 次块解析。

**不宣称持续追加整体变快**：本次混合文档追加中位数为 53.240→53.255ms，几乎不变；代码为 14.608→15.507ms，增加约 0.9ms，CPU 中位数同为 16ms；列表首个投影也从 341.335 变为 361.562ms。相同文本复用和旧 live 版本释放才是本轮主要收益。此前试验的墙钟时间随机器负载显著变化，完整说明及最终成对数据保留在 JSON 中。

采样堆高水位不是 OS 峰值/RSS，也不是持续采样的真实 peak；本轮不承诺整个 app 的峰值内存降幅。静态缓存仍是 512 条上限，没有扩大为新的全局内存管理机制。

上述成对数据对应 `7ab5bfa3`。解决与 origin/main 并行共享解析器的参数冲突后，在 `4345f3e0` 重新构建并复验三种输入：列表保留堆 1.146MiB、代码 0.677MiB，相同文本转换分别为 0.004ms / 0.003ms；混合文档首个投影 100.587ms、相同文本转换 0.022ms。混合文档小规模堆数据在这次复验中为 1.126MiB，说明 runtime/JIT 对这类小差值有影响，不把它宣称为稳定内存改善。复验只确认最终实现，不与旧基线拼接计算新的加速倍数。

### 回归与复现

`streamingMarkdownCache.test.ts` 在改动前有 5 个失败断言，确认 live 缓存污染、结束时重复解析及普通块重复 parse；改动后通过。另逐字符对比混合语法，oracle 显式绕过缓存，避免误复用 candidate 结果而掩盖渲染差异。本轮 desktop 与共享 Markdown 包类型检查、desktop lint 通过；desktop 全量最初为 107 个文件、928 个测试通过，与 origin/main 的共享解析器合并后为 **108 个文件、956 个测试通过**。

在 desktop 中先构建，再执行（每条为独立进程）：

```powershell
node --expose-gc scripts/profile-streaming-worker.mjs list candidate
node --expose-gc scripts/profile-streaming-worker.mjs mixed candidate
node --expose-gc scripts/profile-streaming-worker.mjs code candidate
```

第三个可选参数是另一份生产构建的 assets 目录，例如 `... list baseline <baseline>/desktop/dist/assets`，可在同一测试窗口成对重跑。脚本输出所有追加和收尾样本以及进程 CPU 计数；Windows CPU 计数粒度较粗，不能把返回的 0 解释为真实零 CPU 消耗。
