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
  { id: "future-paper", name: "future-paper", nameZh: "文献检索", description: "Search academic literature and retrieve full text by DOI or PMID.", descriptionZh: "按 DOI / PMID 检索文献并取回全文", version: "1.2.0" },
  { id: "future-deep-research", name: "future-deep-research", nameZh: "深度研究", description: "Evidence-driven research with traceable citations.", descriptionZh: "可溯源引用的深度研究报告", version: "2.0.1" },
  { id: "future-experimental-design", name: "future-experimental-design", nameZh: "实验设计", description: "Design experiments, randomization and blocking.", descriptionZh: "实验设计、随机化与区组安排", version: "1.1.3" },
  { id: "future-image", name: "future-image", nameZh: "图像处理", description: "Generate, edit and analyze images.", descriptionZh: "生成、编辑与分析图像", version: "1.2.0" },
  { id: "future-database-lookup", name: "future-database-lookup", nameZh: "数据库查询", description: "Query 78 scientific databases through documented APIs.", descriptionZh: "通过官方 API 查询 78 个科研数据库", version: "1.0.4" },
];

/** Skills the agent reports as loaded (the composer's "/" picker source). */
export const demoSkills = demoInstalledSkills;

/** The platform catalogue (the skills page's "available" tab). */
export const demoAvailableSkills = [
  { id: "future-slides", name: "Slides", nameZh: "演示文稿", description: "Turn a report into a deck of slides.", descriptionZh: "把报告做成一整套幻灯片", latestVersion: "1.4.0" },
  { id: "future-peer-review", name: "Peer Review", nameZh: "同行评审", description: "Review manuscripts and proposals.", descriptionZh: "评审稿件与基金申请书", latestVersion: "1.1.0" },
  { id: "future-scientific-writing", name: "Scientific Writing", nameZh: "学术写作", description: "Draft and revise manuscripts.", descriptionZh: "起草与修改学术论文", latestVersion: "1.3.2" },
  { id: "future-web", name: "Web Search", nameZh: "网页检索", description: "Search the public web and verify facts.", descriptionZh: "搜索公开网页并核实信息", latestVersion: "1.0.9" },
  { id: "future-browser", name: "Browser", nameZh: "浏览器", description: "Control a local browser.", descriptionZh: "操作本机浏览器", latestVersion: "1.3.0" },
];
