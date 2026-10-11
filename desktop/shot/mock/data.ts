/**
 * Demo dataset for the screenshot harness.
 *
 * A fictional but realistic "research workbench" state: workspaces, a
 * conversation tree, one fully detailed conversation (thinking + tool calls +
 * a markdown answer), runs and tool calls, skills and models.
 *
 * Timestamps are relative to load time so the UI always reads "just now".
 */

const now = Date.now();
const minute = 60_000;
const hour = 60 * minute;
const day = 24 * hour;

export interface MockWorkspace {
  id: string;
  name: string;
  kind: "user" | "temporary";
  path: string;
  description?: string | null;
  pinned?: boolean;
  cleanupStatus: "active" | "pending_cleanup" | "cleaned";
  cleanupRequestedAt?: number | null;
  cleanedAt?: number | null;
  lastOpenedAt?: number | null;
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
}

export interface MockThread {
  id: string;
  workspaceId: string;
  mode: "chat" | "workspace";
  title: string;
  status: "active" | "archived" | "deleted";
  pinned: boolean;
  readonly: boolean;
  agentSessionId?: string | null;
  parentSessionId?: string | null;
  lastMessageAt?: number | null;
  lastOpenedAt?: number | null;
  createdAt: number;
  updatedAt: number;
}

export const HOME = "/Users/lixin";

export const workspaces: MockWorkspace[] = [
  {
    id: "ws_dopamine",
    name: "多巴胺与决策",
    kind: "user",
    path: `${HOME}/Research/dopamine-decision`,
    description: "风险决策的神经经济学文献与数据",
    pinned: true,
    cleanupStatus: "active",
    lastOpenedAt: now - 2 * minute,
    createdAt: now - 30 * 24 * hour,
    updatedAt: now - 2 * minute,
  },
  {
    id: "ws_sparse",
    name: "稀疏注意力复现",
    kind: "user",
    path: `${HOME}/Research/sparse-attention`,
    description: "复现 Longformer 的稀疏注意力实验",
    pinned: true,
    cleanupStatus: "active",
    lastOpenedAt: now - 5 * hour,
    createdAt: now - 20 * 24 * hour,
    updatedAt: now - 5 * hour,
  },
  {
    id: "ws_sc",
    name: "单细胞转录组",
    kind: "user",
    path: `${HOME}/Research/single-cell`,
    cleanupStatus: "active",
    lastOpenedAt: now - 26 * hour,
    createdAt: now - 12 * 24 * hour,
    updatedAt: now - 26 * hour,
  },
  {
    id: "ws_temp",
    name: "临时会话",
    kind: "temporary",
    path: `${HOME}/.future/workspaces/chat/2026-09-17-a1b2`,
    cleanupStatus: "active",
    lastOpenedAt: now - 3 * hour,
    createdAt: now - 3 * hour,
    updatedAt: now - 3 * hour,
  },
];

function thread(id: string, workspaceId: string, title: string, ageMinutes: number, extra: Partial<MockThread> = {}): MockThread {
  return {
    id,
    workspaceId,
    mode: "workspace",
    title,
    status: "active",
    pinned: false,
    readonly: false,
    agentSessionId: `sess_${id}`,
    parentSessionId: null,
    lastMessageAt: now - ageMinutes * minute,
    lastOpenedAt: now - ageMinutes * minute,
    createdAt: now - (ageMinutes + 90) * minute,
    updatedAt: now - ageMinutes * minute,
    ...extra,
  };
}

export const threads: MockThread[] = [
  // The conversation the screenshots open on.
  thread("th_review", "ws_dopamine", "整理两篇文献的结论对比表", 2, { pinned: true }),
  // A conversation tree: a parent thread and two follow-ups forked from it.
  thread("th_review_a", "ws_dopamine", "只看风险偏好那一段", 40, {
    parentSessionId: "sess_th_review",
  }),
  thread("th_review_b", "ws_dopamine", "把结论改成中文表格", 75, {
    parentSessionId: "sess_th_review",
  }),
  thread("th_review_b1", "ws_dopamine", "表格加上样本量一列", 90, {
    parentSessionId: "sess_th_review_b",
  }),
  thread("th_data", "ws_dopamine", "把这份问卷数据清洗一下", 6 * 60),
  thread("th_fig", "ws_dopamine", "重新画一张效应量森林图", 30 * 60),

  thread("th_longformer", "ws_sparse", "跑通 Longformer 的 baseline", 5 * 60),
  thread("th_ablation", "ws_sparse", "帮我设计消融实验", 2 * 24 * 60),
  thread("th_sc", "ws_sc", "找一下这批细胞的 marker 基因", 26 * 60),
  thread("th_temp", "ws_temp", "帮我把这段英文摘要改通顺", 3 * 60),
  // Chat-mode conversations (the rail's own "对话" group).
  thread("th_chat1", "ws_temp", "基金申报书的创新点怎么写", 20, { mode: "chat" }),
  thread("th_chat2", "ws_temp", "解释一下 p 值的常见误用", 5 * 60, { mode: "chat" }),
  thread("th_chat3", "ws_temp", "把这封催稿邮件改得客气一点", 28 * 60, { mode: "chat" }),
];

/** The conversation rendered in the chat screenshots. */
export const MAIN_THREAD_ID = "th_review";

/**
 * Session-level token usage + amount for the conversation the screenshots open
 * on. The per-category figures are the agent's estimate from the model's
 * per-1M-token rates (input 4 / output 16 / cache read 0.4 / cache write 5 CNY),
 * billing only the non-cached input remainder; the total is what the provider
 * actually charged. So the rows are an estimate and sum to it here because this
 * demo model reports no billing of its own — the token counts are chosen so
 * every row is exact at the four decimals the dialog shows, and they add up to
 * the total rather than looking like an off-by-rounding bug.
 */
export const sessionUsage = {
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

/**
 * What the same conversation reports once a later run has been paid for — the
 * answer to a *second* read, so a capture can show the panel moving to fresh
 * figures when it opens instead of re-displaying what the header already had.
 * Each category grows by whole tokens, so the rows still add up to the total.
 */
export const refreshedSessionUsage = {
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

/**
 * The honest degraded case: a model with no prices on file. The agent reports
 * zeros per category, so the client shows tokens only and must not invent a
 * ¥0 breakdown — the billed total is still a real figure.
 */
export const unpricedSessionUsage = {
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

export interface MockBlock {
  kind: string;
  text?: string;
  toolCallId?: string;
  name?: string;
  arguments?: unknown;
  isError?: boolean;
}

export interface MockEntry {
  id: string;
  kind: string;
  role: "user" | "assistant" | "tool" | "system";
  createdAtMs: number;
  runId?: string | null;
  blocks: MockBlock[];
  metadata?: Record<string, unknown> | null;
  usage?: Record<string, unknown> | null;
  run?: Record<string, unknown> | null;
  /** Present on `kind: "compaction"` entries, same shape the agent serves. */
  checkpoint?: Record<string, unknown> | null;
}

const ANSWER = `两篇文献结论并不冲突，只是结论的**适用条件**不同：Frank 等人看的是「收益不确定」时的选择，Dreher 等人看的是「已经知道结果范围」时的选择。

## 结论对比

| 维度 | Frank 等 (2024) | Dreher 等 (2025) |
| --- | --- | --- |
| 被试 | 42 名健康成年人 | 68 名健康成年人 |
| 任务 | 轮盘赌（收益不确定） | 二选一（收益已知） |
| 多巴胺测量 | PET，纹状体结合电位 | 血液代谢物 |
| 主要结论 | 多巴胺越高，越倾向于冒险 | 多巴胺越高，越倾向于规避风险 |
| 效应量 | r = 0.41 | r = −0.33 |

## 我的判断

差异主要来自**任务设计**：当结果范围未知时，多巴胺提升的是"去探索"的动机；当结果已经明确时，多巴胺提升的是对损失的敏感度。两篇文章其实是在测同一个系统的两个不同侧面。

如果要在综述里引用，建议这样写：

> 多巴胺对风险决策的作用方向取决于任务的不确定性结构，而非简单的"促进冒险"或"促进保守"。

下面的图是两篇文献效应量的对比，我已经按你的模板重画过：

![效应量对比](/Users/lixin/Research/dopamine-decision/notes/effect-size.png)

\`\`\`python
# 复现图 3b 的效应量对比
import matplotlib.pyplot as plt
import numpy as np

studies = ["Frank 2024", "Dreher 2025"]
effects = [0.41, -0.33]
plt.barh(studies, effects, color=["#4f7cff", "#e2685f"])
plt.axvline(0, color="#888", lw=1)
plt.xlabel("效应量 r")
plt.tight_layout()
\`\`\`

需要我把这两篇的**实验流程**也画成流程图吗？`;

export const mainEntries: MockEntry[] = [
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
        {
          path: `${HOME}/Research/dopamine-decision/papers/frank-2024.pdf`,
          name: "frank-2024.pdf",
          kind: "file",
        },
        {
          path: `${HOME}/Research/dopamine-decision/papers/dreher-2025.pdf`,
          name: "dreher-2025.pdf",
          kind: "file",
        },
      ],
    },
  },
  {
    id: "e2",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 8.4 * minute,
    runId: "run_1",
    usage: { inputTokens: 18_420, outputTokens: 1_260, cacheReadTokens: 12_800 },
    run: { status: "completed", durationMs: 74_300 },
    blocks: [
      {
        kind: "reasoning",
        text: "用户给了两份 PDF，要求做结论对比表。第一步先把 PDF 的正文抽出来，确认两篇文章各自报的效应量方向和数值；注意不要只看摘要，摘要里的措辞容易过度概括。",
      },
      {
        kind: "tool_call",
        toolCallId: "tc_1",
        name: "read",
        arguments: { path: "papers/frank-2024.pdf" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_2",
        name: "read",
        arguments: { path: "papers/dreher-2025.pdf" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_3",
        name: "shell",
        arguments: { command: "ls -la data/ && wc -l data/subjects.csv" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_4",
        name: "read",
        arguments: { path: "notes/frank-summary.md" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_5",
        name: "read",
        arguments: { path: "notes/dreher-summary.md" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_6",
        name: "write",
        arguments: { path: "notes/effect-size.png" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_7",
        name: "edit",
        arguments: { path: "notes/compare.md" },
      },
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
      { kind: "tool_result", toolCallId: "tc_7" },
    ],
  },
  {
    id: "e4",
    kind: "message",
    role: "user",
    createdAtMs: now - 4 * minute,
    runId: "run_2",
    blocks: [{ kind: "text", text: "很好，那 Dreher 那篇的样本为什么比 Frank 多这么多？" }],
  },
  {
    id: "e5",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 3.2 * minute,
    runId: "run_2",
    usage: { inputTokens: 21_960, outputTokens: 640, cacheReadTokens: 19_100 },
    run: { status: "completed", durationMs: 38_900 },
    blocks: [
      {
        kind: "reasoning",
        text: "问的是样本量差异的原因。需要区分两件事：招募渠道不同（PET 实验成本高、有辐射剂量限制），以及统计功效要求（Dreher 测的是血液代谢物，方差更大）。",
      },
      {
        kind: "tool_call",
        toolCallId: "tc_8",
        name: "shell",
        arguments: { command: "grep -n \"sample size\" -A 6 notes/*.md" },
      },
      {
        kind: "tool_call",
        toolCallId: "tc_9",
        name: "read",
        arguments: { path: "notes/dreher-summary.md" },
      },
      {
        kind: "text",
        text: `主要是**测量手段的代价**不同：

- Frank 用的是 PET，每名被试都要注射示踪剂、有辐射剂量，伦理审批上通常把被试数压在 50 以内；
- Dreher 用的是血液代谢物，成本低得多，而且血液指标的个体差异比 PET 结合电位大，要拿到同样的统计功效就得多招人。

所以样本量的差别反映的是测量成本，不是结论可靠性的差别。`,
      },
    ],
  },
  {
    id: "e6",
    kind: "tool",
    role: "tool",
    createdAtMs: now - 3.1 * minute,
    runId: "run_2",
    blocks: [
      { kind: "tool_result", toolCallId: "tc_8" },
      { kind: "tool_result", toolCallId: "tc_9" },
    ],
  },
];

/** A second conversation used for the "search full history" screenshot. */
export const figureEntries: MockEntry[] = [
  {
    id: "f1",
    kind: "message",
    role: "user",
    createdAtMs: now - 31 * 60 * minute,
    runId: "run_9",
    blocks: [{ kind: "text", text: "重新画一张效应量森林图，风格用我们上次定好的那套配色。" }],
  },
  {
    id: "f2",
    kind: "message",
    role: "assistant",
    createdAtMs: now - 30 * 60 * minute,
    runId: "run_9",
    usage: { inputTokens: 9_200, outputTokens: 810 },
    run: { status: "completed", durationMs: 52_000 },
    blocks: [
      {
        kind: "tool_call",
        toolCallId: "fc_1",
        name: "read",
        arguments: { path: "figures/style.md" },
      },
      {
        kind: "tool_call",
        toolCallId: "fc_2",
        name: "write",
        arguments: { path: "figures/forest-plot.png" },
      },
      {
        kind: "text",
        text: "森林图已经重画好了，配色沿用了 `figures/style.md` 里的蓝色主色，负值用了暖红做对照。文件在 `figures/forest-plot.png`。",
      },
    ],
  },
];

export const entriesByThread: Record<string, MockEntry[]> = {
  th_review: mainEntries,
  th_fig: figureEntries,
};

/**
 * `?compactHistory=1` history: one run that compacted mid-turn (the
 * schemaVersion 3 checkpoint the agent writes today) and then kept writing.
 * The text after the divider exists so a capture can prove it still reaches the
 * transcript — a version-gated projector used to treat the divider as an
 * exchange boundary and drop everything after it.
 */
export const compactResumeEntries: MockEntry[] = [
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
      {
        kind: "text",
        text: `## 结论

压缩之后我把最后一步做完了：

- **流量**：一次冷开从 15.03 MB 降到 5.89 MB（2.55×），弱网路径上省 18 秒；
- **代价**：服务端多 35 ms，客户端解压 74 ms，任何真实链路都净赚；
- **建议**：先上 gzip 协商，再考虑先压缩后分片。

要不要我把这四步写成一份设计文档？`,
      },
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

// ─── Runs / tools (Runs panel + right context rail) ──────────────────────

export const runs = [
  {
    id: "run_1",
    threadId: "th_review",
    status: "completed",
    modelProvider: "future",
    modelId: "deepseek-v4-pro",
    startedAt: now - 9 * minute,
    endedAt: now - 7.8 * minute,
    createdAt: now - 9 * minute,
    updatedAt: now - 7.8 * minute,
  },
  {
    id: "run_2",
    threadId: "th_review",
    status: "completed",
    modelProvider: "future",
    modelId: "deepseek-v4-pro",
    startedAt: now - 4 * minute,
    endedAt: now - 3.2 * minute,
    createdAt: now - 4 * minute,
    updatedAt: now - 3.2 * minute,
  },
];

function tool(id: string, runId: string, name: string, input: string, startedAtMs: number, durationMs: number) {
  return {
    id,
    runId,
    name,
    kind: name,
    input,
    status: "completed",
    startedAt: startedAtMs,
    endedAt: startedAtMs + durationMs,
    createdAt: startedAtMs,
  };
}

export const toolCalls = [
  tool("tc_1", "run_1", "read", JSON.stringify({ path: "papers/frank-2024.pdf" }), now - 8.9 * minute, 1_400),
  tool("tc_2", "run_1", "read", JSON.stringify({ path: "papers/dreher-2025.pdf" }), now - 8.8 * minute, 1_500),
  tool("tc_3", "run_1", "shell", JSON.stringify({ command: "ls -la data/ && wc -l data/subjects.csv" }), now - 8.7 * minute, 320),
  tool("tc_4", "run_1", "read", JSON.stringify({ path: "notes/frank-summary.md" }), now - 8.6 * minute, 180),
  tool("tc_5", "run_1", "read", JSON.stringify({ path: "notes/dreher-summary.md" }), now - 8.5 * minute, 190),
  tool("tc_6", "run_1", "write", JSON.stringify({ path: "notes/effect-size.png" }), now - 8.4 * minute, 2_600),
  tool("tc_7", "run_1", "edit", JSON.stringify({ path: "notes/compare.md" }), now - 8.35 * minute, 240),
  tool("tc_8", "run_2", "shell", JSON.stringify({ command: "grep -n \"sample size\" -A 6 notes/*.md" }), now - 3.9 * minute, 260),
  tool("tc_9", "run_2", "read", JSON.stringify({ path: "notes/dreher-summary.md" }), now - 3.8 * minute, 170),
];

export const toolOutputs: Record<string, string> = {
  tc_3: `total 96
drwxr-xr-x  6 lixin  staff    192  9月 17 10:02 .
-rw-r--r--  1 lixin  staff  18422  9月 17 09:58 subjects.csv
-rw-r--r--  1 lixin  staff   9210  9月 16 21:14 questionnaire.csv
68 data/subjects.csv`,
  tc_8: `notes/frank-summary.md:12:sample size: n = 42, PET imaging, striatal binding potential
notes/dreher-summary.md:12:sample size: n = 68, plasma metabolite assay`,
};

// ─── Skills ──────────────────────────────────────────────────────────────

export const installedSkills = [
  { id: "future-paper", name: "Paper Search", nameZh: "文献检索", description: "Search academic literature and retrieve full text by DOI or PMID.", descriptionZh: "按 DOI / PMID 检索文献并取回全文", version: "1.2.0" },
  { id: "future-deep-research", name: "Deep Research", nameZh: "深度研究", description: "Evidence-driven research with traceable citations.", descriptionZh: "可溯源引用的深度研究报告", version: "2.0.1" },
  { id: "future-experimental-design", name: "Experimental Design", nameZh: "实验设计", description: "Design experiments, randomization and blocking.", descriptionZh: "实验设计、随机化与区组安排", version: "1.1.3" },
  { id: "future-image", name: "Images", nameZh: "图像处理", description: "Generate, edit and analyze images.", descriptionZh: "生成、编辑与分析图像", version: "1.2.0" },
  { id: "future-database-lookup", name: "Database Lookup", nameZh: "数据库查询", description: "Query 78 scientific databases through documented APIs.", descriptionZh: "通过官方 API 查询 78 个科研数据库", version: "1.0.4" },
];

export const availableSkills = [
  { id: "future-slides", name: "Slides", nameZh: "演示文稿", description: "Turn a report into a deck of slides.", descriptionZh: "把报告做成一整套幻灯片", category: "writing", categoryZh: "写作", latestVersion: "1.4.0" },
  { id: "future-peer-review", name: "Peer Review", nameZh: "同行评审", description: "Review manuscripts and proposals.", descriptionZh: "评审稿件与基金申请书", category: "writing", categoryZh: "写作", latestVersion: "1.1.0" },
  { id: "future-scientific-writing", name: "Scientific Writing", nameZh: "学术写作", description: "Draft and revise manuscripts.", descriptionZh: "起草与修改学术论文", category: "writing", categoryZh: "写作", latestVersion: "1.3.2" },
  { id: "future-web", name: "Web Search", nameZh: "网页检索", description: "Search the public web and verify facts.", descriptionZh: "搜索公开网页并核实信息", category: "tools", categoryZh: "工具", latestVersion: "1.0.9" },
  { id: "future-browser", name: "Browser", nameZh: "浏览器", description: "Control a local browser.", descriptionZh: "操作本机浏览器", category: "tools", categoryZh: "工具", latestVersion: "1.3.0" },
];

// ─── Models & providers ──────────────────────────────────────────────────

export const models = [
  { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro", provider: "future", reasoning: true, thinkingLevel: "medium", contextWindow: 128_000, isDefault: true, supportsImages: true, description: "推理与长上下文兼顾，适合文献综述与数据分析", descriptionEn: "Balanced reasoning and long context", recommended: true },
  { id: "deepseek-v4", label: "DeepSeek V4", provider: "future", reasoning: true, thinkingLevel: "medium", contextWindow: 128_000, supportsImages: true, description: "日常问答与改写的快模型", descriptionEn: "Fast model for everyday work" },
  { id: "future-flash", label: "Future Flash", provider: "future", reasoning: false, thinkingLevel: "off", contextWindow: 64_000, description: "低延迟，适合小改动", descriptionEn: "Low latency for small edits" },
  { id: "future-vision", label: "Future Vision", provider: "future", reasoning: true, thinkingLevel: "medium", contextWindow: 200_000, supportsImages: true, description: "看图、读图表与扫描件", descriptionEn: "Reads figures and scans" },
];

export const providersView = {
  builtin: [
    { id: "future", name: "FutureOS", baseUrl: "https://api.future-os.cn", hasApiKey: true, modelCount: 4, requiresBaseUrl: false },
    { id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com/v1", hasApiKey: false, modelCount: 6, requiresBaseUrl: false },
    { id: "anthropic", name: "Anthropic", baseUrl: "https://api.anthropic.com", hasApiKey: false, modelCount: 4, requiresBaseUrl: false },
  ],
  custom: [
    {
      id: "local-vllm",
      name: "实验室本地模型",
      api: "openai",
      baseUrl: "http://10.0.12.7:8000/v1",
      hasApiKey: true,
      models: [
        {
          id: "qwen3-32b",
          name: "Qwen3 32B",
          supportsImages: false,
          reasoning: true,
          contextWindow: 32_768,
          maxTokens: 4_096,
          // Filled in so the edit form shows the price fields populated. The
          // rates are the ones the demo session's breakdown is priced with.
          inputCost: 4,
          outputCost: 16,
          cacheReadCost: 0.4,
          cacheWriteCost: 5,
        },
      ],
    },
  ],
};

// ─── Everything else ─────────────────────────────────────────────────────

export const appSettings = {
  approvalTier: "off",
  hiddenModels: [],
  autoUpgradeSkills: true,
  autoConnectRemote: false,
  skillGuideDismissed: true,
  skillIntroDismissed: true,
  bellOnComplete: true,
  autoTitleFirstTurn: true,
  titleLanguage: "zh",
  communityEdition: false,
  // Mirrors the shipped default (PRD v1.6 §3: recommendation is on).
  skillRecommend: true,
};

export const buildInfo = {
  version: "1.1.8",
  buildNumber: "20260917",
  // A dev build surfaces the Environment tab, which hosts the community-edition
  // switch (release builds hide it).
  isRelease: false,
  target: "aarch64-apple-darwin",
};

export const workspaceFiles: Record<string, Array<{ name: string; path: string; isDir: boolean; size: number; modified: number | null }>> = {
  [`${HOME}/Research/dopamine-decision`]: [
    { name: "data", path: `${HOME}/Research/dopamine-decision/data`, isDir: true, size: 0, modified: now - 3 * hour },
    { name: "figures", path: `${HOME}/Research/dopamine-decision/figures`, isDir: true, size: 0, modified: now - 40 * minute },
    { name: "notes", path: `${HOME}/Research/dopamine-decision/notes`, isDir: true, size: 0, modified: now - 3 * minute },
    { name: "papers", path: `${HOME}/Research/dopamine-decision/papers`, isDir: true, size: 0, modified: now - 2 * hour },
    { name: "draft-v3.md", path: `${HOME}/Research/dopamine-decision/draft-v3.md`, isDir: false, size: 24_800, modified: now - 22 * minute },
    { name: "README.md", path: `${HOME}/Research/dopamine-decision/README.md`, isDir: false, size: 1_240, modified: now - 5 * hour },
  ],
  [`${HOME}/Research/dopamine-decision/figures`]: [
    { name: "plot-effect-size.py", path: `${HOME}/Research/dopamine-decision/figures/plot-effect-size.py`, isDir: false, size: 486, modified: now - 40 * minute },
  ],
  [`${HOME}/Research/dopamine-decision/notes`]: [
    { name: "compare.md", path: `${HOME}/Research/dopamine-decision/notes/compare.md`, isDir: false, size: 4_180, modified: now - 3 * minute },
    { name: "dreher-summary.md", path: `${HOME}/Research/dopamine-decision/notes/dreher-summary.md`, isDir: false, size: 2_960, modified: now - 2 * hour },
    { name: "frank-summary.md", path: `${HOME}/Research/dopamine-decision/notes/frank-summary.md`, isDir: false, size: 3_120, modified: now - 2 * hour },
    { name: "effect-size.png", path: `${HOME}/Research/dopamine-decision/notes/effect-size.png`, isDir: false, size: 62_400, modified: now - 4 * minute },
  ],
};

/**
 * Source the text preview shows for a demo file, keyed by file name. Files
 * without an entry fall back to the markdown sample. `plot-effect-size.py` is
 * the script the agent's reply quotes, so the file preview and the chat code
 * block show the same code.
 */
export const demoFileContents: Record<string, string> = {
  "plot-effect-size.py": `# 复现图 3b 的效应量对比
import matplotlib.pyplot as plt
import numpy as np

studies = ["Frank 2024", "Dreher 2025"]
effects = [0.41, -0.33]

fig, ax = plt.subplots(figsize=(6, 3))
ax.barh(studies, effects, color=["#4f7cff", "#e2685f"])
ax.axvline(0, color="#888", lw=1, label="no effect")
ax.set_xlabel("效应量 r")
ax.legend(loc="lower right", frameon=False)
fig.tight_layout()
fig.savefig("figures/effect-size.pdf", dpi=300)  # 论文里用的是矢量图
`,
};

export const reviewFiles = [
  {
    id: "rf1",
    changesetId: "cs1",
    path: "notes/compare.md",
    changeType: "modified",
    additions: 18,
    deletions: 2,
    diff: `--- a/notes/compare.md
+++ b/notes/compare.md
@@ -1,5 +1,21 @@
 # 多巴胺与风险决策：结论对比
 
+## 结论对比
+
+| 维度 | Frank 等 (2024) | Dreher 等 (2025) |
+| --- | --- | --- |
+| 被试 | 42 名健康成年人 | 68 名健康成年人 |
+| 任务 | 轮盘赌（收益不确定） | 二选一（收益已知） |
+| 主要结论 | 多巴胺越高，越倾向于冒险 | 多巴胺越高，越倾向于规避风险 |
+
+差异主要来自任务设计的不确定性结构。
`,
  },
  {
    id: "rf2",
    changesetId: "cs1",
    path: "figures/forest-plot.png",
    changeType: "added",
    additions: 0,
    deletions: 0,
    diff: "",
  },
];

// ── Tasks ────────────────────────────────────────────────────────────────────
// The shapes `commands/tasks.rs` serializes (camelCase), so the panel renders
// exactly what a real backend would hand it.

export interface MockTaskRun {
  id: string;
  kind: string;
  origin: string;
  status: string;
  dueAt: number | null;
  threadId: string | null;
  sessionId: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  promptVersion: number | null;
  resultSummary: string | null;
  errorMessage: string | null;
}

export interface MockTask {
  id: string;
  name: string;
  enabled: boolean;
  prompt: string;
  promptVersion: number;
  cwd: string;
  modelId: string | null;
  thinkingLevel: string | null;
  sessionPolicy: string;
  sessionRetention: string;
  conversationMode: string;
  triggerKind: string;
  trigger: Record<string, unknown>;
  depJoin: string;
  nextDueAt: number | null;
  queued: boolean;
  /** Upstream dependencies this task waits on (0 when it has none). */
  depCount: number;
  /** Prompt suggestions awaiting a decision (0 when there are none). */
  latestRun: MockTaskRun | null;
}

export const tasks: MockTask[] = [
  {
    id: "tsk_weekly_report",
    name: "每周进展周报",
    enabled: true,
    prompt: [
      "读一遍本目录下 notes/ 里本周新增或改动的文件，把其中的结论变化整理成中文周报。",
      "",
      "要求：",
      "- 只写本周实际发生的进展，不要复述背景；没有进展就写「本周无进展」。",
      "- 每条结论后附上对应文件路径。",
      "- 写入 reports/weekly-<信封里的 due 日期>.md，不要覆盖已有文件。",
      "- 结尾给出一个「下周待办」小节，最多三条。",
    ].join("\n"),
    promptVersion: 4,
    cwd: "~/Research/dopamine-decision",
    modelId: "future/deepseek-v4-pro",
    thinkingLevel: "high",
    sessionPolicy: "existing",
    sessionRetention: "keep",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" },
    depJoin: "all",
    nextDueAt: now + 2 * hour + 40 * minute,
    queued: false,
    depCount: 1,
    latestRun: {
      id: "trn_report_2",
      kind: "main",
      origin: "schedule",
      status: "completed",
      dueAt: now - 2 * day,
      threadId: "th_review",
      sessionId: "sess_review",
      startedAt: now - 2 * day,
      finishedAt: now - 2 * day + 3 * minute,
      promptVersion: 4,
      resultSummary: "写入 reports/weekly-2026-10-05.md：3 条结论变化、2 条新增证据；下周待办已列出。",
      errorMessage: null,
    },
  },
  {
    id: "tsk_lit_watch",
    name: "文献监控",
    enabled: true,
    prompt: [
      "检索最近 7 天与「多巴胺 风险决策」相关的新论文，逐篇给出三行摘要（问题、方法、结论）。",
      "只保留有实验数据的文章；综述类单独列一节。",
      "写入 reports/lit-watch.md（覆盖上一版，历史留在会话里）。",
      "同时把最值得读的一篇的 DOI 放到摘要开头。",
    ].join("\n"),
    promptVersion: 2,
    cwd: "~/Research/dopamine-decision",
    modelId: null,
    thinkingLevel: null,
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "interval", every_minutes: 720 },
    depJoin: "all",
    nextDueAt: now + 6 * hour,
    queued: true,
    // One suggestion is waiting: the row badges it and the detail lists it.
    depCount: 0,
    latestRun: {
      id: "trn_lit_9",
      kind: "main",
      origin: "schedule",
      status: "failed",
      dueAt: now - 6 * hour,
      threadId: "th_lit",
      sessionId: "sess_lit",
      startedAt: now - 6 * hour,
      finishedAt: now - 6 * hour + 40_000,
      promptVersion: 2,
      resultSummary: null,
      errorMessage: "上游检索服务返回 503；未写入文件。",
    },
  },
  {
    id: "tsk_month_end",
    name: "月末归档",
    enabled: true,
    prompt: [
      "把本月产生的所有 reports/ 与 figures/ 文件归档到 archive/<年-月>/ 下，并生成一份索引 README。",
      "只移动，不删除；遇到同名文件加后缀 -2 而不是覆盖。",
      "索引里按文件类型分组，并标出每个文件最后一次被修改的时间。",
    ].join("\n"),
    promptVersion: 1,
    cwd: "~/Research",
    modelId: null,
    thinkingLevel: "medium",
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "monthly", day: 31, time: "09:00" },
    depJoin: "all",
    nextDueAt: now + 24 * day,
    queued: false,
    depCount: 0,
    latestRun: null,
  },
  {
    id: "tsk_notify",
    name: "失败汇总通知",
    enabled: false,
    prompt: [
      "上游任务失败时运行：读取它的运行记录与错误摘要，写成一段不超过 80 字的中文说明。",
      "说明要包含：哪个任务、什么时候、失败原因、是否需要人工介入。",
      "写入 reports/incidents.md（追加，不覆盖）。",
    ].join("\n"),
    promptVersion: 1,
    cwd: "~/Research",
    modelId: null,
    thinkingLevel: "low",
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    triggerKind: "manual",
    trigger: {},
    depJoin: "all",
    nextDueAt: null,
    queued: false,
    depCount: 2,
    latestRun: {
      id: "trn_notify_1",
      kind: "chain",
      origin: "chain",
      status: "completed",
      dueAt: null,
      threadId: null,
      sessionId: null,
      startedAt: now - 6 * hour,
      finishedAt: now - 6 * hour + 20_000,
      promptVersion: 1,
      resultSummary: "已在 reports/incidents.md 追加一条：文献监控 503，需人工确认检索服务。",
      errorMessage: null,
    },
  },
];

/** Runs of one task, newest first (the panel's `list_task_runs`). */
export const taskRuns: Record<string, MockTaskRun[]> = {
  tsk_weekly_report: [
    tasks[0]!.latestRun!,
    {
      id: "trn_report_1",
      kind: "main",
      origin: "schedule",
      status: "completed",
      dueAt: now - 5 * day,
      threadId: "th_review",
      sessionId: "sess_review",
      startedAt: now - 5 * day,
      finishedAt: now - 5 * day + 2 * minute,
      promptVersion: 3,
      resultSummary: "写入 reports/weekly-2026-10-02.md：2 条结论变化；提示词已由建议更新到 v4。",
      errorMessage: null,
    },
  ],
  // Two runs, so the panel's per-run cards (and the suggestion that belongs to
  // exactly one of them) are what the screenshot shows.
  tsk_lit_watch: [
    tasks[1]!.latestRun!,
    {
      id: "trn_lit_8",
      kind: "main",
      origin: "schedule",
      status: "completed",
      dueAt: now - 18 * hour,
      threadId: "th_lit",
      sessionId: "sess_lit",
      startedAt: now - 18 * hour,
      finishedAt: now - 18 * hour + 52_000,
      promptVersion: 2,
      resultSummary: "写入 reports/lit-watch.md：新增 3 篇，综述 1 篇；开头给出 DOI 10.1038/s41593-026-01234-5。",
      errorMessage: null,
    },
  ],
  tsk_notify: [tasks[3]!.latestRun!],
  tsk_month_end: [],
};

/** Dependency edges, keyed by downstream task (the panel's `list_task_deps`). */
export const taskDeps: Record<string, Array<{ upstreamTaskId: string; upstreamName: string; on: string; satisfied: boolean }>> = {
  tsk_weekly_report: [
    { upstreamTaskId: "tsk_lit_watch", upstreamName: "文献监控", on: "completed", satisfied: true },
  ],
  tsk_notify: [
    { upstreamTaskId: "tsk_lit_watch", upstreamName: "文献监控", on: "failure", satisfied: false },
    { upstreamTaskId: "tsk_weekly_report", upstreamName: "每周进展周报", on: "failure", satisfied: false },
  ],
  tsk_lit_watch: [],
  tsk_month_end: [],
};

/** Prompt versions, oldest first (the panel's `list_task_revisions`). */
export const taskRevisions: Record<string, Array<{ id: string; version: number; prompt: string; source: string; status: string; reason: string | null; confidence: number | null; createdAt: number }>> = {
  tsk_weekly_report: [
    {
      id: "rev_w1",
      version: 1,
      prompt: "整理本周进展。",
      // What the prompt-change bookkeeping writes for the version being
      // replaced: who wrote it was never recorded, so the row says what is
      // certain instead of guessing.
      source: "superseded",
      status: "superseded",
      reason: null,
      confidence: null,
      createdAt: now - 30 * day,
    },
    {
      id: "rev_w3",
      version: 3,
      prompt: "读 notes/ 里本周新增或改动的文件，整理成中文周报。",
      source: "user",
      status: "superseded",
      reason: null,
      confidence: null,
      createdAt: now - 9 * day,
    },
    {
      id: "rev_w4",
      version: 4,
      prompt: "（当前生效版本，见左侧提示词）补上「下周待办」与文件路径要求。",
      source: "reflection",
      status: "active",
      reason: "上一版没写明输出路径，运行结果无处可查；同时缺一条可执行的下周计划。",
      confidence: 0.82,
      createdAt: now - 2 * day,
    },
  ],
  tsk_lit_watch: [
    {
      id: "rev_l2",
      version: 2,
      prompt: "（当前生效版本）限定 7 天内、按问题/方法/结论三行摘要，并给出 DOI。",
      source: "user",
      status: "active",
      reason: null,
      confidence: null,
      createdAt: now - 4 * day,
    },
  ],
  tsk_month_end: [],
  tsk_notify: [],
};

export const nowRef = now;

// ── The client role: this Desktop's pairings *out* to other computers ───────
//
// The host-role data above answers "who connects to me". These answer the
// opposite question, and they are what the Remote page's client half, the merged
// conversation list, the device selector and a remote conversation render from.
//
// Two hosts, on purpose: one connected with conversations and one disconnected,
// because "a host that is not reachable" is a state the list has to survive
// (its rows drop out rather than going stale) and a single-host fixture cannot
// show it.

export interface MockPeer {
  desktopId: string;
  pairId: string;
  name: string | null;
  icon: string | null;
  connected: boolean;
  error: string | null;
  bridgeInstanceId: string | null;
  features: string[];
  agentAvailable: boolean;
}

export const remotePeers: MockPeer[] = [
  {
    desktopId: "desktop_studio",
    pairId: "pair_studio",
    name: "工作室 iMac",
    icon: "laptop",
    connected: true,
    error: null,
    bridgeInstanceId: "bridge_studio",
    // The capabilities the host advertises; the UI gates optional actions on
    // these, so a fixture without them renders fewer rows than the app can.
    features: [
      "approval_tier_v1",
      "auto_approval_v1",
      "provider_management_v1",
      "workspace_create_v1",
      "workspace_pinning_v1",
    ],
    agentAvailable: true,
  },
  {
    desktopId: "desktop_macmini",
    pairId: "pair_macmini",
    name: null,
    icon: "desktop",
    connected: false,
    error: "peer_not_connected",
    bridgeInstanceId: null,
    features: [],
    agentAvailable: false,
  },
];

/** That host's conversations, as its own catalogue reports them. */
export const remoteSessions: Record<string, unknown[]> = {
  desktop_studio: [
    {
      sessionId: "rs_forest",
      threadId: "th_forest",
      title: "重画森林图并核对置信区间",
      mode: "workspace",
      workspaceId: "ws_figures",
      pinned: false,
      streaming: true,
      lastMessageAt: Date.now() - 90_000,
    },
    {
      sessionId: "rs_prisma",
      threadId: "th_prisma",
      title: "PRISMA 流程图的筛选数字对不上",
      mode: "workspace",
      workspaceId: "ws_figures",
      pinned: true,
      streaming: false,
      lastMessageAt: Date.now() - 40 * 60_000,
    },
    {
      sessionId: "rs_reviewer",
      threadId: "th_reviewer",
      title: "审稿意见逐条回复草稿",
      mode: "chat",
      workspaceId: null,
      pinned: false,
      streaming: false,
      lastMessageAt: Date.now() - 3 * 60 * 60_000,
    },
  ],
};

/** A remote conversation's transcript, in the host's own entry shape. */
export const remoteEntries: Record<string, unknown[]> = {
  rs_forest: [
    {
      id: "re_1",
      kind: "message",
      role: "user",
      createdAtMs: Date.now() - 6 * 60_000,
      runId: "rrun_1",
      blocks: [{
        kind: "text",
        text: "effect-size.R 里那张森林图，把 95% 置信区间画成了 90%，重新画一遍并核对数值。",
      }],
    },
    {
      id: "re_2",
      kind: "message",
      role: "assistant",
      createdAtMs: Date.now() - 5 * 60_000,
      // `runId` is what makes a tool row's arguments fetchable: the entry keeps
      // the call id and the run, and the command is read back by both.
      runId: "rrun_1",
      run: { status: "completed", durationMs: 42_000 },
      blocks: [
        { kind: "text", text: "先看脚本里的置信区间是怎么算的。" },
        { kind: "tool_call", toolCallId: "rtc_1", name: "read" },
        {
          kind: "text",
          text: "问题在 `ci_level <- 0.90`：画的是 90% 区间，而正文写的是 95%。改成 0.95 之后重画。",
        },
        { kind: "tool_call", toolCallId: "rtc_2", name: "shell" },
      ],
    },
    {
      id: "re_3",
      kind: "message",
      role: "user",
      createdAtMs: Date.now() - 3 * 60_000,
      runId: "rrun_2",
      blocks: [{ kind: "text", text: "再检查一下有没有别的图也用了这个变量。" }],
    },
    {
      id: "re_4",
      kind: "message",
      role: "assistant",
      createdAtMs: Date.now() - 90_000,
      runId: "rrun_2",
      run: { status: "running" },
      blocks: [{
        kind: "text",
        text: "另外两张图（`fig2-forest.R`、`figS3.R`）各自硬编码了区间，不受影响；只有这一处。",
      }],
    },
  ],
};

/**
 * One tool call's arguments, fetched back because a lean history page strips
 * them — the UI reads the target out of these.
 */
export const remoteToolArgs: Record<string, { name: string; arguments: Record<string, unknown> }> = {
  rtc_1: { name: "read", arguments: { file_path: "analysis/effect-size.R" } },
  rtc_2: { name: "shell", arguments: { command: "Rscript analysis/effect-size.R" } },
};

/** A remote conversation's model and thinking level (the host's settings). */
export const remoteConversationState = {
  model: "deepseek/deepseek-chat",
  thinkingLevel: "medium",
};

/** That host's workspaces. */
export const remoteWorkspaces = {
  desktopId: "desktop_studio",
  version: 1,
  workspaces: [
    { id: "ws_figures", kind: "user", name: "图表脚本", path: "/Users/studio/papers/figures", pinned: true },
    { id: "ws_manuscript", kind: "user", name: "投稿稿件", path: "/Users/studio/papers/submission", pinned: false },
  ],
};
