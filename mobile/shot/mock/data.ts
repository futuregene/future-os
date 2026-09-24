/**
 * Demo dataset for the mobile screenshot harness.
 *
 * A fictional but realistic "research workbench": workspaces, a conversation
 * tree, one fully detailed conversation (thinking + tool calls + a markdown
 * answer with a chart), session files, skills and models.
 *
 * Timestamps are relative to load time, so the UI always reads "just now".
 */

const now = Date.now();
const minute = 60_000;

export const demoCredentials = {
  pairId: "pair_1",
  deviceId: "device_1",
  seed: "",
  userJwt: "",
  refreshToken: "",
  natsWsUrl: "wss://www.future-os.cn:9090",
  tokenUrl: "https://www.future-os.cn/token",
  expectedDesktopId: "desktop_C76HV2R4TKPGCC55",
  expectedDesktopPublicKey: "",
};

export const demoDesktops = [
  { desktopId: "desktop_C76HV2R4TKPGCC55", pairId: "pair_1", name: "实验室的 MacBook Pro" },
  { desktopId: "desktop_9KQ2ZP7LXM4HTR31", pairId: "pair_2", name: "办公室主机" },
];

export const demoModels = [
  { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro", provider: "future", isDefault: true, supportsImages: true },
  { id: "deepseek-v4", label: "DeepSeek V4", provider: "future", supportsImages: true },
  { id: "future-flash", label: "Future Flash", provider: "future" },
];

export const demoWorkspaces = [
  { id: "ws_dopamine", name: "多巴胺与决策", path: "/Users/lixin/Research/dopamine-decision", pinned: true },
  { id: "ws_sparse", name: "稀疏注意力复现", path: "/Users/lixin/Research/sparse-attention", pinned: true },
  { id: "ws_sc", name: "单细胞转录组", path: "/Users/lixin/Research/single-cell" },
  { id: "ws_temp", name: "临时会话", path: "/Users/lixin/.future/workspaces/chat/2026-09-17-a1b2" },
];

const session = (
  sessionId: string,
  workspaceId: string,
  title: string,
  extra: Record<string, unknown> = {},
) => ({
  sessionId,
  threadId: sessionId.replace("sess_", "th_"),
  title,
  mode: "workspace" as const,
  workspaceId,
  streaming: false,
  ...extra,
});

export const sessions = [
  session("sess_dopamine_review", "ws_dopamine", "整理两篇文献的结论对比表", { pinned: true }),
  session("sess_dopamine_clean", "ws_dopamine", "把这份问卷数据清洗一下"),
  session("sess_sparse_baseline", "ws_sparse", "跑通 Longformer 的 baseline", { streaming: true }),
  session("sess_sc_marker", "ws_sc", "找一下这批细胞的 marker 基因"),
  // A conversation tree: two follow-ups forked from the first conversation.
  session("sess_branch_risk", "ws_dopamine", "只看风险偏好那一段", {
    parentSessionId: "sess_dopamine_review",
  }),
  session("sess_branch_table", "ws_dopamine", "把结论改成中文表格", {
    parentSessionId: "sess_dopamine_review",
  }),
  // Chat-mode conversations (no workspace context).
  session("sess_chat_grant", "ws_temp", "基金申报书的创新点怎么写", { mode: "chat" }),
  session("sess_chat_pvalue", "ws_temp", "解释一下 p 值的常见误用", { mode: "chat" }),
];

/**
 * Session token usage + amount, mirroring the agent's `get_state` usage object.
 *
 * The per-category figures are the agent's estimate from the model's
 * per-1M-token rates (input 4 / output 16 / cache read 0.4 / cache write 5 CNY),
 * billing only the non-cached input remainder. The token counts are chosen so
 * each row is exact at the four decimals the sheet shows and the rows add up to
 * the total, so the demo never looks like an off-by-rounding bug.
 *
 * The p-value conversation stands in for a model with no prices on file: the
 * agent reports zeros per category, so the sheet shows tokens only and must not
 * invent a ¥0 breakdown — the billed total is still a real figure.
 */
export const demoSessionUsage = {
  inputTokens: 600_050,
  outputTokens: 9_825,
  cacheReadTokens: 412_750,
  cacheWriteTokens: 18_900,
  costCny: 1.0904,
  costInputCny: 0.6736,
  costOutputCny: 0.1572,
  costCacheReadCny: 0.1651,
  costCacheWriteCny: 0.0945,
};

export const demoUnpricedSessionUsage = {
  inputTokens: 84_600,
  outputTokens: 2_140,
  cacheReadTokens: 51_300,
  cacheWriteTokens: 0,
  costCny: 0.32,
  costInputCny: 0,
  costOutputCny: 0,
  costCacheReadCny: 0,
  costCacheWriteCny: 0,
};

/**
 * What the same conversation returns once a later run has been paid for — the
 * figure a fresh read produces. Each category grows by a whole number of
 * tokens, so the rows still add up to the total at four decimals.
 */
export const demoRefreshedSessionUsage = {
  inputTokens: 677_050,
  outputTokens: 14_825,
  cacheReadTokens: 437_750,
  cacheWriteTokens: 20_900,
  costCny: 1.3904,
  costInputCny: 0.8736,
  costOutputCny: 0.2372,
  costCacheReadCny: 0.1751,
  costCacheWriteCny: 0.1045,
};

const ANSWER = `两篇文献结论并不冲突，只是结论的**适用条件**不同。

## 结论对比

| 维度 | Frank 等 (2024) | Dreher 等 (2025) |
| --- | --- | --- |
| 被试 | 42 名健康成年人 | 68 名健康成年人 |
| 任务 | 轮盘赌（收益不确定） | 二选一（收益已知） |
| 主要结论 | 多巴胺越高，越倾向于冒险 | 多巴胺越高，越倾向于规避风险 |
| 效应量 | r = 0.41 | r = −0.33 |

当结果范围未知时，多巴胺提升的是"去探索"的动机；当结果已经明确时，多巴胺提升的是对损失的敏感度。

![效应量对比](/Users/lixin/Research/dopamine-decision/figures/effect-size.png)

需要我把这两篇的**实验流程**也画成流程图吗？`;

export const demoEntries = [
  {
    id: "e1",
    kind: "message",
    role: "user",
    createdAtMs: now - 9 * minute,
    runId: "run_1",
    blocks: [{
      kind: "text",
      text: "我把这两篇讲多巴胺和风险决策的文章放进工作区了，帮我把它们的结论整理成一张对比表。Frank 2024 和 Dreher 2025。",
    }],
    metadata: {
      attachments: [
        { path: "/Users/lixin/Research/dopamine-decision/papers/frank-2024.pdf", name: "frank-2024.pdf", kind: "file" },
        { path: "/Users/lixin/Research/dopamine-decision/papers/dreher-2025.pdf", name: "dreher-2025.pdf", kind: "file" },
      ],
    },
  },
  {
    id: "e2",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 8.4 * minute,
    runId: "run_1",
    usage: { inputTokens: 18_420, outputTokens: 1_260 },
    run: { status: "completed", durationMs: 74_300 },
    blocks: [
      {
        kind: "reasoning",
        text: "用户给了两份 PDF，要求做结论对比表。先把正文抽出来，确认两篇文章各自报的效应量方向和数值。",
      },
      { kind: "tool_call", toolCallId: "tc_1", name: "read", arguments: { path: "papers/frank-2024.pdf" } },
      { kind: "tool_call", toolCallId: "tc_2", name: "read", arguments: { path: "papers/dreher-2025.pdf" } },
      { kind: "tool_call", toolCallId: "tc_3", name: "shell", arguments: { command: "wc -l data/subjects.csv" } },
      { kind: "tool_call", toolCallId: "tc_4", name: "read", arguments: { path: "notes/frank-summary.md" } },
      { kind: "tool_call", toolCallId: "tc_5", name: "read", arguments: { path: "notes/dreher-summary.md" } },
      { kind: "tool_call", toolCallId: "tc_6", name: "write", arguments: { path: "notes/compare.md" } },
      { kind: "text", text: ANSWER },
    ],
  },
  {
    id: "e3",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 8.3 * minute,
    runId: "run_1",
    blocks: [
      { kind: "tool_result", toolCallId: "tc_1" },
      { kind: "tool_result", toolCallId: "tc_2" },
      { kind: "tool_result", toolCallId: "tc_3" },
      { kind: "tool_result", toolCallId: "tc_4" },
      { kind: "tool_result", toolCallId: "tc_5" },
      { kind: "tool_result", toolCallId: "tc_6" },
    ],
  },
];

/**
 * The reply that follows an in-turn checkpoint.
 *
 * `?compactHistory=1` renders this history instead of `demoEntries`: one run
 * that compacted mid-turn (the schemaVersion 3 checkpoint the agent writes
 * today) and then kept writing. The text after the divider exists so a capture
 * can prove it still reaches the transcript — a version-gated projector used
 * to treat the divider as an exchange boundary and drop everything after it.
 */
const POST_COMPACTION_ANSWER = `## 结论

压缩之后我把最后一步做完了：

- **流量**：一次冷开从 15.03 MB 降到 5.89 MB（2.55×），弱网路径上省 18 秒；
- **代价**：服务端多 35 ms，客户端解压 74 ms，任何真实链路都净赚；
- **建议**：先上 gzip 协商，再考虑先压缩后分片。

要不要我把这四步写成一份设计文档？`;

export const compactResumeEntries = [
  {
    id: "c1",
    kind: "message",
    role: "user",
    createdAtMs: now - 22 * minute,
    runId: "run_compact",
    blocks: [{
      kind: "text",
      text: "接着上一轮：把三个方案的实测代价和收益整理成一个表格，最后给出结论。",
    }],
  },
  {
    id: "c2",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 21 * minute,
    runId: "run_compact",
    blocks: [
      {
        kind: "reasoning",
        text: "这一轮已经很长了，先把前三步的实测数字固定下来，再决定第四步怎么写。",
      },
      { kind: "tool_call", toolCallId: "cc_1", name: "read", arguments: { path: "notes/stream-traffic.md" } },
      { kind: "tool_call", toolCallId: "cc_2", name: "shell", arguments: { command: "wc -c results/*.json" } },
      { kind: "tool_call", toolCallId: "cc_3", name: "read", arguments: { path: "notes/gzip-bench.md" } },
      {
        kind: "text",
        text: "前三步已经跑完，先把中间结论记一下：gzip 协商把冷开流量压到 2.55×，服务端只多花 35 ms。",
      },
    ],
  },
  {
    id: "c3",
    kind: "compaction",
    role: "system",
    createdAtMs: now - 20.5 * minute,
    blocks: [],
    checkpoint: {
      schemaVersion: 3,
      checkpointId: "cp_shot_v3",
      coveredFromEntryId: "c1",
      cutoffEntryId: "c2",
      tokensBefore: 613_994,
      tokensAfter: 51_081,
      trigger: "automatic",
      phase: "mid_turn",
      algorithmVersion: "deterministic-evidence-v1",
    },
  },
  {
    id: "c4",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 20 * minute,
    runId: "run_compact",
    usage: { inputTokens: 131_997, outputTokens: 1_450, cacheReadTokens: 131_840 },
    run: { status: "completed", durationMs: 1_182_754 },
    blocks: [
      { kind: "tool_call", toolCallId: "cc_4", name: "write", arguments: { path: "notes/stream-summary.md" } },
      { kind: "text", text: POST_COMPACTION_ANSWER },
    ],
  },
  {
    id: "c5",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 19.9 * minute,
    runId: "run_compact",
    blocks: [
      { kind: "tool_result", toolCallId: "cc_1" },
      { kind: "tool_result", toolCallId: "cc_2" },
      { kind: "tool_result", toolCallId: "cc_3" },
      { kind: "tool_result", toolCallId: "cc_4" },
    ],
  },
];

/**
 * A conversation with several turns, for the controls that navigate between
 * questions: with one exchange there is nothing to jump to. Selected with
 * `?multiTurn=1`, so the single-turn demo stays the default.
 *
 * Every answer runs longer than the viewport, so "jump to the start of this
 * question" is a real distance rather than a one-line nudge.
 */
const MULTI_TABLE = `| 维度 | Frank 等 (2024) | Dreher 等 (2025) |
| --- | --- | --- |
| 被试 | 42 名健康成年人 | 68 名健康成年人 |
| 任务 | 轮盘赌（收益不确定） | 二选一（收益已知） |
| 主要结论 | 多巴胺越高，越倾向于冒险 | 多巴胺越高，越倾向于规避风险 |
| 效应量 | r = 0.41 | r = −0.33 |
| 测量方式 | PET，[¹¹C]raclopride 结合潜能 | 瞳孔直径 + 停药挑战 |`;

const MULTI_PROCESS = `## Dreher 2025 的实验流程

1. **筛选（D-14 ~ D-0）**：68 名被试，排除精神科用药与药物滥用史；随机分配到安慰剂组（n = 34）与多巴胺前体耗竭组（n = 34）。
2. **基线（D-0，上午 9:00）**：静息瞳孔直径、风险态度量表、一次 40 试次的二选一练习。
3. **耗竭（D-0，上午 10:00 ~ 下午 2:00）**：耗竭组口服 AMPT，安慰剂组服安慰剂，双盲；4 小时后采血确认酪氨酸羟化酶抑制程度。
4. **正式任务（D-0，下午 2:30）**：120 试次的二选一，每个试次给出确定收益（如"必得 5 元"）与随机收益（如"60% 得 12 元"）。
5. **收尾（D-0，下午 4:00）**：再次测量瞳孔与主观唤醒度，核对盲态。

> 关键设计：收益的**取值范围**在正式任务里始终已知，而这正是两篇文献结论相反的原因。`;

export const multiTurnEntries = [
  {
    id: "m1",
    kind: "message",
    role: "user",
    createdAtMs: now - 46 * minute,
    runId: "run_m1",
    blocks: [{
      kind: "text",
      text: "我把这两篇讲多巴胺和风险决策的文章放进工作区了，帮我把它们的结论整理成一张对比表。Frank 2024 和 Dreher 2025。",
    }],
    metadata: {
      attachments: [
        { path: "/Users/lixin/Research/dopamine-decision/papers/frank-2024.pdf", name: "frank-2024.pdf", kind: "file" },
        { path: "/Users/lixin/Research/dopamine-decision/papers/dreher-2025.pdf", name: "dreher-2025.pdf", kind: "file" },
      ],
    },
  },
  {
    id: "m2",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 45 * minute,
    runId: "run_m1",
    usage: { inputTokens: 18_420, outputTokens: 1_260 },
    run: { status: "completed", durationMs: 74_300 },
    blocks: [
      {
        kind: "reasoning",
        text: "用户给了两份 PDF，要求做结论对比表。先把正文抽出来，确认两篇文章各自报的效应量方向和数值。",
      },
      { kind: "tool_call", toolCallId: "mt_1", name: "read", arguments: { path: "papers/frank-2024.pdf" } },
      { kind: "tool_call", toolCallId: "mt_2", name: "read", arguments: { path: "papers/dreher-2025.pdf" } },
      { kind: "tool_call", toolCallId: "mt_3", name: "shell", arguments: { command: "wc -l data/subjects.csv" } },
      {
        kind: "text",
        text: `两篇文献结论并不冲突，只是结论的**适用条件**不同。

## 结论对比

${MULTI_TABLE}

当结果范围未知时，多巴胺提升的是"去探索"的动机；当结果已经明确时，多巴胺提升的是对损失的敏感度。

需要我把这两篇的**实验流程**也画成流程图吗？`,
      },
    ],
  },
  {
    id: "m2t",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 44.9 * minute,
    runId: "run_m1",
    blocks: [
      { kind: "tool_result", toolCallId: "mt_1" },
      { kind: "tool_result", toolCallId: "mt_2" },
      { kind: "tool_result", toolCallId: "mt_3" },
    ],
  },
  {
    id: "m3",
    kind: "message",
    role: "user",
    createdAtMs: now - 32 * minute,
    runId: "run_m2",
    blocks: [{
      kind: "text",
      text: "Frank 那篇报的 r = 0.41 是怎么算出来的？他们的样本量够支撑这个效应量吗？",
    }],
  },
  {
    id: "m4",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 31 * minute,
    runId: "run_m2",
    usage: { inputTokens: 21_880, outputTokens: 1_640 },
    run: { status: "completed", durationMs: 96_500 },
    blocks: [
      {
        kind: "reasoning",
        text: "用户问的是效应量的算法和样本量，先看方法部分报的是 Pearson r 还是偏相关，再核对他们的检验力说明。",
      },
      { kind: "tool_call", toolCallId: "mt_4", name: "read", arguments: { path: "papers/frank-2024.pdf", pages: "4-6" } },
      { kind: "tool_call", toolCallId: "mt_5", name: "shell", arguments: { command: "python tools/power.py --r 0.41 --n 42" } },
      {
        kind: "text",
        text: `## 那个 r = 0.41 的来源

Frank 等报的是**结合潜能与冒险比例之间的 Pearson 相关**，不是回归系数，也不是组间差异的效应量：

- 自变量：11 名被试因为 PET 数据质量被剔除后剩 **n = 42**；
- 因变量：轮盘赌任务里选择"高方差选项"的试次比例（每人 60 试次）；
- 相关系数 r = 0.41，对应 r² ≈ 0.168，即解释了约 17% 的个体差异。

## 样本量够不够

| 判据 | 数值 | 结论 |
| --- | --- | --- |
| 检验力（α = .05，双侧） | 0.81 | 勉强够 |
| 该效应量达到 0.80 检验力所需 n | 40 | 刚好越过 |
| 95% 置信区间 | [0.12, 0.63] | 下限离 0 不远 |

所以：**方向上可信，量级别当真**。置信区间下沿只有 0.12，说明真实相关完全可能是弱相关；报告里说"中等偏强"是偏乐观的读法。

如果要把这个数字写进综述，建议同时给出区间，并注明 PET 样本小、被试间基线差异未完全控制。`,
      },
    ],
  },
  {
    id: "m4t",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 30.9 * minute,
    runId: "run_m2",
    blocks: [
      { kind: "tool_result", toolCallId: "mt_4" },
      { kind: "tool_result", toolCallId: "mt_5" },
    ],
  },
  {
    id: "m5",
    kind: "message",
    role: "user",
    createdAtMs: now - 18 * minute,
    runId: "run_m3",
    blocks: [{
      kind: "text",
      text: "把 Dreher 的实验流程画成流程图，每一步标上时间窗。",
    }],
  },
  {
    id: "m6",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 17 * minute,
    runId: "run_m3",
    usage: { inputTokens: 24_310, outputTokens: 1_430 },
    run: { status: "completed", durationMs: 88_100 },
    blocks: [
      {
        kind: "reasoning",
        text: "流程图用步骤列表加时间窗比真画图更清楚，先确认方法部分每一步的时间点。",
      },
      { kind: "tool_call", toolCallId: "mt_6", name: "read", arguments: { path: "papers/dreher-2025.pdf" } },
      { kind: "text", text: MULTI_PROCESS },
    ],
  },
  {
    id: "m6t",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 16.9 * minute,
    runId: "run_m3",
    blocks: [{ kind: "tool_result", toolCallId: "mt_6" }],
  },
  {
    id: "m7",
    kind: "message",
    role: "user",
    createdAtMs: now - 6 * minute,
    runId: "run_m4",
    blocks: [{
      kind: "text",
      text: "最后把这两个结论写成一页中文摘要，说清楚什么条件下两篇会给出相反的建议。",
    }],
  },
  {
    id: "m8",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 4.5 * minute,
    runId: "run_m4",
    usage: { inputTokens: 27_640, outputTokens: 1_820 },
    run: { status: "completed", durationMs: 102_400 },
    blocks: [
      {
        kind: "reasoning",
        text: "摘要要落到条件差异上，先按“结果范围是否已知”把两篇的结论摆在一起，再给实验设计的建议。",
      },
      { kind: "tool_call", toolCallId: "mt_7", name: "write", arguments: { path: "notes/dopamine-summary.md" } },
      {
        kind: "text",
        text: `## 摘要

多巴胺与风险决策的关系在两篇文献里方向相反，但这不是数据冲突，而是**任务把"风险"定义成了两件事**。

### 一句话结论

- 结果范围**未知**时，多巴胺升高 → 更愿意**探索**（Frank 2024，轮盘赌，r = 0.41）；
- 结果范围**已知**时，多巴胺升高 → 更**规避损失**（Dreher 2025，二选一，r = −0.33）。

### 为什么会相反

1. 轮盘赌里，被试不知道收益的上限，选择高方差选项被解读为"去试试看"；二选一里，两个结果都写明了，选择高方差选项变成"接受一个明确的坏结果的风险"。
2. 两篇的操纵方向也不同：Frank 用 PET 测基线多巴胺水平，Dreher 用耗竭法把多巴胺压低。**降下来**的动作更容易放大损失敏感度。
3. 被试量都偏小（42 与 68），两个效应量的置信区间都碰到 0 附近，把它们读成"方向相反"比读成"强度相反"更稳妥。

### 写进综述时怎么措辞

> 多巴胺对风险选择的作用取决于收益分布是否已知：在未知分布下促进探索，在已知分布下提升损失敏感度。

这句话两边都能引用，也不需要在正文里判谁对。`,
      },
    ],
  },
  {
    id: "m8t",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 4.4 * minute,
    runId: "run_m4",
    blocks: [{ kind: "tool_result", toolCallId: "mt_7" }],
  },
];

export const demoFiles = {
  rootPath: "/Users/lixin/Research/dopamine-decision",
  path: "/Users/lixin/Research/dopamine-decision",
  entries: [
    { name: "data", path: "/Users/lixin/Research/dopamine-decision/data", isDir: true, size: 0 },
    { name: "figures", path: "/Users/lixin/Research/dopamine-decision/figures", isDir: true, size: 0 },
    { name: "notes", path: "/Users/lixin/Research/dopamine-decision/notes", isDir: true, size: 0 },
    { name: "draft-v3.md", path: "/Users/lixin/Research/dopamine-decision/draft-v3.md", isDir: false, size: 24_800 },
    { name: "README.md", path: "/Users/lixin/Research/dopamine-decision/README.md", isDir: false, size: 1_240 },
  ],
};

/**
 * Skills the agent reports as loaded (the composer's "/" picker source and the
 * skills settings page). `name` is the command name the composer inserts, so it
 * must stay a single token — the picker drops anything with spaces.
 */
export const demoInstalledSkills = [
  { id: "future-software-install", name: "future-software-install", nameZh: "轻量软件安装与镜像配置", description: "Install or update lightweight software, command-line tools, browser automation prerequisites, and document-processing libraries for FutureOS across Windows, macOS, and Linux. Use when the user asks to install a tool, configure Homebrew or a package manager, set up Chrome/headless browsing, add Python document libraries, or use mainland China download mirrors.", descriptionZh: "跨 Windows、macOS 和 Linux 安装轻量命令行工具、Chrome 与文档处理库，并提供中国大陆网络镜像策略", version: "1.0.1" },
  { id: "future-paper", name: "future-paper", nameZh: "文献检索", description: "Search academic literature and retrieve full text by DOI or PMID.", descriptionZh: "按 DOI / PMID 检索文献并取回全文", version: "1.2.0" },
  { id: "future-deep-research", name: "future-deep-research", nameZh: "深度研究", description: "Evidence-driven research with traceable citations.", descriptionZh: "可溯源引用的深度研究报告", version: "2.0.1" },
  { id: "future-experimental-design", name: "future-experimental-design", nameZh: "实验设计", description: "Design experiments, randomization and blocking.", descriptionZh: "实验设计、随机化与区组安排", version: "1.1.3" },
  { id: "future-image", name: "future-image", nameZh: "图像处理", description: "Generate, edit and analyze images.", descriptionZh: "生成、编辑与分析图像", version: "1.2.0" },
  { id: "future-database-lookup", name: "future-database-lookup", nameZh: "数据库查询", description: "Query 78 scientific databases through documented APIs.", descriptionZh: "通过官方 API 查询 78 个科研数据库", version: "1.0.4" },
];

/** Skills the agent reports as loaded (the composer's "/" picker source). */
export const demoSkills = demoInstalledSkills;

/**
 * Provider configuration as the desktop reports it: built-in catalog providers
 * plus one custom provider with its models. `hasApiKey` is only ever a boolean
 * — the phone never receives key material.
 */
export const demoProviders = {
  builtin: [
    { id: "future", name: "Future", baseUrl: "https://future-os.cn/api/v1", hasApiKey: true, modelCount: 9, requiresBaseUrl: false },
    { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", hasApiKey: true, modelCount: 3, requiresBaseUrl: false },
    { id: "anthropic", name: "Anthropic", baseUrl: "https://api.anthropic.com/v1", hasApiKey: false, modelCount: 4, requiresBaseUrl: false },
    { id: "azure-openai-responses", name: "Azure OpenAI Responses", baseUrl: "https://YOUR_RESOURCE.openai.azure.com/openai", hasApiKey: false, modelCount: 1, requiresBaseUrl: true },
  ],
  custom: [
    {
      id: "acme",
      name: "Acme Gateway",
      api: "openai-completions",
      baseUrl: "https://gateway.acme.example.com/v1",
      hasApiKey: true,
      models: [
        { id: "acme-large", name: "Acme Large", supportsImages: true, reasoning: true, contextWindow: 128000, maxTokens: 16384, inputCost: 1.5, outputCost: 6, cacheReadCost: 0.15, cacheWriteCost: 0 },
        { id: "acme-mini", name: "Acme Mini", supportsImages: false, reasoning: false, contextWindow: 32000, maxTokens: 4096, inputCost: 0, outputCost: 0, cacheReadCost: 0, cacheWriteCost: 0 },
      ],
    },
  ],
};

/** The platform catalogue (the skills page's "available" tab). */
export const demoAvailableSkills = [
  { id: "future-slides", name: "Slides", nameZh: "演示文稿", description: "Turn a report into a deck of slides.", descriptionZh: "把报告做成一整套幻灯片", latestVersion: "1.4.0" },
  { id: "future-peer-review", name: "Peer Review", nameZh: "同行评审", description: "Review manuscripts and proposals.", descriptionZh: "评审稿件与基金申请书", latestVersion: "1.1.0" },
  { id: "future-scientific-writing", name: "Scientific Writing", nameZh: "学术写作", description: "Draft and revise manuscripts.", descriptionZh: "起草与修改学术论文", latestVersion: "1.3.2" },
  { id: "future-web", name: "Web Search", nameZh: "网页检索", description: "Search the public web and verify facts.", descriptionZh: "搜索公开网页并核实信息", latestVersion: "1.0.9" },
  { id: "future-browser", name: "Browser", nameZh: "浏览器", description: "Control a local browser.", descriptionZh: "操作本机浏览器", latestVersion: "1.3.0" },
];
