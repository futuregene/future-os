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

/**
 * The language the fixture's *data* is written in.
 *
 * The interface has two languages (`src/i18n`), and half of a frame is the
 * agent's words: a rail whose rows say `整理两篇文献的结论对比表` under a heading
 * that says `Sessions` is neither language. So the fixture carries the session
 * list and the transcripts in both, and `?lang=en` (the same parameter
 * `shot/prefs.ts` pins the interface with) selects the English payload — the
 * capture passes one parameter and both halves agree.
 *
 * English is not a translation of the Chinese payload, it is the session an
 * English reader would be looking at: same workspaces, same ages, same
 * conversations, written in the language of the frame.
 */
const fixtureEn = typeof window !== "undefined"
  && new URLSearchParams(window.location.search).get("lang") === "en";

/** Pick the fixture string for this capture's language. */
function pick(zh: string, en: string): string {
  return fixtureEn ? en : zh;
}

/**
 * The conversation a capture asked to open — `?thread=th_parser`.
 *
 * The default state is what every other scenario starts from, and a conversation
 * added to it would move all of them. So a conversation a capture names brings
 * its own workspace with it, and only then. The backend reads the same value to
 * decide which conversation the app opens on.
 */
export function requestedThread(): string | null {
  if (typeof window === "undefined")
    return null;
  return new URLSearchParams(window.location.search).get("thread");
}

/** The expression-parser session: the conversation the two language captures open. */
export const PARSER_THREAD_ID = "th_parser";

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
  /** The model this conversation runs on; unset means the fixture's default. */
  model?: string;
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
    name: pick("多巴胺与决策", "Dopamine and Decisions"),
    kind: "user",
    path: `${HOME}/Research/dopamine-decision`,
    description: pick("风险决策的神经经济学文献与数据", "Neuroeconomics of risky choice: papers and data"),
    pinned: true,
    cleanupStatus: "active",
    lastOpenedAt: now - 2 * minute,
    createdAt: now - 30 * 24 * hour,
    updatedAt: now - 2 * minute,
  },
  {
    id: "ws_sparse",
    name: pick("稀疏注意力复现", "Sparse Attention, Reproduced"),
    kind: "user",
    path: `${HOME}/Research/sparse-attention`,
    description: pick("复现 Longformer 的稀疏注意力实验", "Reproducing Longformer's sparse attention"),
    pinned: true,
    cleanupStatus: "active",
    lastOpenedAt: now - 5 * hour,
    createdAt: now - 20 * 24 * hour,
    updatedAt: now - 5 * hour,
  },
  {
    id: "ws_sc",
    name: pick("单细胞转录组", "Single-Cell Transcriptomics"),
    kind: "user",
    path: `${HOME}/Research/single-cell`,
    cleanupStatus: "active",
    lastOpenedAt: now - 26 * hour,
    createdAt: now - 12 * 24 * hour,
    updatedAt: now - 26 * hour,
  },
  {
    id: "ws_temp",
    name: pick("临时会话", "Scratch Chats"),
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
  thread("th_review", "ws_dopamine", pick("整理两篇文献的结论对比表", "Comparing Two Papers' Conclusions"), 2, { pinned: true }),
  // A conversation tree: a parent thread and two follow-ups forked from it.
  thread("th_review_a", "ws_dopamine", pick("只看风险偏好那一段", "Just the Risk-Preference Section"), 40, {
    parentSessionId: "sess_th_review",
  }),
  thread("th_review_b", "ws_dopamine", pick("把结论改成中文表格", "Conclusions as a Chinese Table"), 75, {
    parentSessionId: "sess_th_review",
  }),
  thread("th_review_b1", "ws_dopamine", pick("表格加上样本量一列", "Add a Sample-Size Column"), 90, {
    parentSessionId: "sess_th_review_b",
  }),
  thread("th_data", "ws_dopamine", pick("把这份问卷数据清洗一下", "Clean Up This Survey Data"), 6 * 60),
  thread("th_fig", "ws_dopamine", pick("重新画一张效应量森林图", "Redraw the Effect-Size Forest Plot"), 30 * 60),

  thread("th_longformer", "ws_sparse", pick("跑通 Longformer 的 baseline", "Get the Longformer Baseline Running"), 5 * 60),
  thread("th_ablation", "ws_sparse", pick("帮我设计消融实验", "Design the Ablation Study"), 2 * 24 * 60),
  // Started two hours ago and then stopped on a question: the recency view ranks
  // the running rows by when they *started*, so this row leads the active stretch
  // even though it is the older one to have spoken. With both clocks agreeing the
  // capture would say nothing about which one the list uses.
  thread("th_sc", "ws_sc", pick("找一下这批细胞的 marker 基因", "Find Marker Genes for These Cells"), 26 * 60, {
    createdAt: now - 2 * 60 * minute,
  }),
  thread("th_temp", "ws_temp", pick("帮我把这段英文摘要改通顺", "Smooth Out This English Abstract"), 3 * 60),
  // Chat-mode conversations (the rail's own "对话" group).
  thread("th_chat1", "ws_temp", pick("基金申报书的创新点怎么写", "Writing a Grant's Innovation Section"), 20, { mode: "chat" }),
  thread("th_chat2", "ws_temp", pick("解释一下 p 值的常见误用", "Common Misuses of the p Value"), 5 * 60, { mode: "chat" }),
  thread("th_chat3", "ws_temp", pick("把这封催稿邮件改得客气一点", "Soften This Nudge Email"), 28 * 60, { mode: "chat" }),
];

/**
 * The expression-parser session.
 *
 * The same two turns the TUI's scripted agent serves (`tui/examples/demo_agent.rs`,
 * `journal_zh` / `journal_en`), in the desktop's own entry shape: a user question
 * about `1+2+` returning `3`, the reasoning and the calls that answer it, a
 * diagnosis with a heading, inline code, a highlighted `rust` block and a
 * three-row table, then a second turn that lands the fix as two commits and
 * answers with a numbered list and a quote.
 *
 * Only `?thread=th_parser` adds it: the default fixture is what every other
 * scenario starts from, and it must stay what it was.
 */
export const PARSER_MODEL = "future/deepseek-flash";
const PARSER_WORKSPACE = "ws_futureos";

function sessionTitle(): string {
  return pick("表达式解析器里的 off-by-one", "Off-by-One in the Expression Parser");
}

if (requestedThread() === PARSER_THREAD_ID) {
  workspaces.push({
    id: PARSER_WORKSPACE,
    name: pick("FutureOS 仓库", "FutureOS Repo"),
    kind: "user",
    path: `${HOME}/future-os`,
    description: pick("表达式解析器、CLI 与 agent 的仓库", "The repository: parser, CLI and agent"),
    cleanupStatus: "active",
    lastOpenedAt: now,
    createdAt: now - 40 * day,
    updatedAt: now,
  });
  threads.unshift(thread(PARSER_THREAD_ID, PARSER_WORKSPACE, sessionTitle(), 0, { model: PARSER_MODEL }));
}

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

/** The diagnosis: a heading, inline code, bold and italic runs, a highlighted block, a table. */
const PARSER_DIAGNOSIS_ZH = `### 诊断

\`parse_expr\` 用 **\`<=\`** 比较光标，于是 *「到末尾」* 和 *「越过末尾」* 变成同一个状态——尾随的运算符被消费掉，而缺的那个右操作数就成了 \`lhs\`。

\`\`\`rust
// 停在最后一个 token 上，而不是越过它。
while self.peek() < self.end {
    let op = self.next();
    lhs = apply(lhs, op, self.parse_atom()?);
}
\`\`\`

| 形状 | 改前 | 改后 |
| --- | --- | --- |
| 1+2+ | 3 | UnexpectedEof |
| [1,2, | [1, 2] | UnexpectedEof |
| f(a, | f(a) | UnexpectedEof |

三种写法是**同一个错误**，所以一张表驱动的用例就覆盖了它们——见 \`src/parser/tests.rs\`。`;

const PARSER_DIAGNOSIS_EN = `### Diagnosis

\`parse_expr\` compares the cursor with **\`<=\`**, so *"at the end"* and *"past the end"* are one state — the trailing operator is consumed and its missing operand becomes \`lhs\`.

\`\`\`rust
// Stop *at* the last token, not one past it.
while self.peek() < self.end {
    let op = self.next();
    lhs = apply(lhs, op, self.parse_atom()?);
}
\`\`\`

| Shape | Before | After |
| --- | --- | --- |
| 1+2+ | 3 | UnexpectedEof |
| [1,2, | [1, 2] | UnexpectedEof |
| f(a, | f(a) | UnexpectedEof |

All three are the **same mistake**, so one table-driven test covers them — see \`src/parser/tests.rs\`.`;

/** The closing answer: a numbered list of the two commits and a quote of the test run. */
const PARSER_RESULT_ZH = `分成 **两个 commit** 落地，都是绿的：

1. \`6b1f0c2\` —— 三处边界，*只改行为*
2. \`d41e8aa\` —— 表驱动的用例，*只改覆盖*

> \`cargo test -p parser\`：44 passed，0 failed，0 ignored。

回滚第二个 commit 之后 \`41 passed\` 原样回来——这就是你要的那条性质，别的东西一个都没动。`;

const PARSER_RESULT_EN = `Landed in **two commits**, both green:

1. \`6b1f0c2\` — the three bounds, *behaviour only*
2. \`d41e8aa\` — the table-driven test, *coverage only*

> \`cargo test -p parser\`: 44 passed, 0 failed, 0 ignored.`;

/**
 * The parser session's two turns, in the desktop's entry shape.
 *
 * The words are the TUI's (`journal_zh` / `journal_en` in
 * `tui/examples/demo_agent.rs`), sentence for sentence: a caption-carrying
 * question, a reasoning block, the calls that answer it, the diagnosis, then the
 * second turn that searches for the same shape and lands the fix. What differs is
 * the *shape* of the payload, because the desktop has its own: tool results are
 * their own entry (the transcript pairs them by `toolCallId` and reads the
 * output from `toolOutputs`), and a reply's reasoning and calls fold into one
 * summary the reader opens.
 */
export function parserEntries(): MockEntry[] {
  return [
    {
      id: "p1",
      kind: "message",
      role: "user",
      createdAtMs: now - 3 * minute,
      runId: "run_parser_1",
      blocks: [{
        kind: "text",
        text: pick(
          "`1+2+` 返回的是 `3`，不是报错。表达式解析器里有个 off-by-one，找出来并补一个回归测试。",
          "`1+2+` returns `3` instead of failing. There is an off-by-one in the expression parser — find it and add a regression test.",
        ),
      }],
    },
    {
      id: "p2",
      kind: "message",
      role: "assistant",
      createdAtMs: now - 2.6 * minute,
      runId: "run_parser_1",
      usage: { inputTokens: 24_800, outputTokens: 1_480, cacheReadTokens: 21_100 },
      run: { status: "completed", durationMs: 61_400 },
      blocks: [
        {
          kind: "reasoning",
          text: pick(
            "先从 `parse_expr` 的循环边界看：光标用 `<=` 比较时，「到末尾」和「越过末尾」是同一个状态，尾随的运算符会被接受，而它的右操作数其实并不存在。",
            "Start at `parse_expr`'s loop bound. If the cursor is compared with `<=`, \"at the end\" and \"past the end\" are the same state, and a trailing operator is accepted with an operand that is not there.",
          ),
        },
        {
          kind: "tool_call",
          toolCallId: "pc_read",
          name: "read",
          arguments: { path: "src/parser.rs", offset: 138, limit: 16 },
        },
        {
          kind: "tool_call",
          toolCallId: "pc_test",
          name: "shell",
          arguments: { command: "cargo test -p parser" },
        },
        { kind: "text", text: pick(PARSER_DIAGNOSIS_ZH, PARSER_DIAGNOSIS_EN) },
      ],
    },
    {
      id: "p3",
      kind: "tool",
      role: "tool",
      createdAtMs: now - 2.5 * minute,
      runId: "run_parser_1",
      blocks: [
        { kind: "tool_result", toolCallId: "pc_read" },
        { kind: "tool_result", toolCallId: "pc_test" },
      ],
    },
    {
      id: "p4",
      kind: "message",
      role: "user",
      createdAtMs: now - 1.5 * minute,
      runId: "run_parser_2",
      blocks: [{
        kind: "text",
        text: pick(
          "别处还有同样的写法吗？顺手拆成两个 commit 提交——先边界，再测试。",
          "Does the same shape appear anywhere else? And land it as two commits — the bounds first, then the tests.",
        ),
      }],
    },
    {
      id: "p5",
      kind: "message",
      role: "assistant",
      createdAtMs: now - minute,
      runId: "run_parser_2",
      usage: { inputTokens: 31_200, outputTokens: 1_020, cacheReadTokens: 28_600 },
      run: { status: "completed", durationMs: 44_900 },
      blocks: [
        {
          kind: "reasoning",
          text: pick(
            "搜这个比较本身，而不是搜 `parse_expr`：错的是边界，不是函数。",
            "Search for the comparison itself rather than for `parse_expr`: the mistake is the bound, not the function.",
          ),
        },
        {
          kind: "tool_call",
          toolCallId: "pc_grep",
          name: "shell",
          arguments: { command: "rg -n 'peek\\(\\) <=' src/" },
        },
        {
          kind: "reasoning",
          text: pick(
            "另外两处，都能从同一个入口到达。边界放一个 commit、用例表放另一个，这样回滚测试不会把行为改动一起带走，中间也不会留一个红的测试集。",
            "Two more sites, both reachable from the same entry point. Bounds in one commit, the case table in the other, so reverting the tests cannot take the behaviour change with them — or leave a red suite in between.",
          ),
        },
        {
          kind: "tool_call",
          toolCallId: "pc_edit",
          name: "edit",
          arguments: {
            path: "src/args.rs",
            old: "while self.peek() <= self.end {",
            new: "while self.peek() < self.end {",
          },
        },
        {
          kind: "tool_call",
          toolCallId: "pc_commit",
          name: "shell",
          arguments: { command: "cargo test -p parser && git commit -am 'fix(parser): the loop bound'" },
        },
        { kind: "text", text: pick(PARSER_RESULT_ZH, PARSER_RESULT_EN) },
      ],
    },
    {
      id: "p6",
      kind: "tool",
      role: "tool",
      createdAtMs: now - 0.9 * minute,
      runId: "run_parser_2",
      blocks: [
        { kind: "tool_result", toolCallId: "pc_grep" },
        { kind: "tool_result", toolCallId: "pc_edit" },
        { kind: "tool_result", toolCallId: "pc_commit" },
      ],
    },
  ];
}

/** The parser session's runs and calls: the Runs panel and the tool rows read these. */
export const parserRuns = [
  {
    id: "run_parser_1",
    threadId: PARSER_THREAD_ID,
    status: "completed",
    modelProvider: "future",
    modelId: "deepseek-flash",
    startedAt: now - 3 * minute,
    endedAt: now - 2.4 * minute,
    createdAt: now - 3 * minute,
    updatedAt: now - 2.4 * minute,
  },
  {
    id: "run_parser_2",
    threadId: PARSER_THREAD_ID,
    status: "completed",
    modelProvider: "future",
    modelId: "deepseek-flash",
    startedAt: now - 1.5 * minute,
    endedAt: now - 0.8 * minute,
    createdAt: now - 1.5 * minute,
    updatedAt: now - 0.8 * minute,
  },
];

export const parserToolCalls = [
  tool("pc_read", "run_parser_1", "read", JSON.stringify({ path: "src/parser.rs", offset: 138, limit: 16 }), now - 2.9 * minute, 140),
  tool("pc_test", "run_parser_1", "shell", JSON.stringify({ command: "cargo test -p parser" }), now - 2.8 * minute, 4_100),
  tool("pc_grep", "run_parser_2", "shell", JSON.stringify({ command: "rg -n 'peek\\(\\) <=' src/" }), now - 1.4 * minute, 120),
  tool("pc_edit", "run_parser_2", "edit", JSON.stringify({ path: "src/args.rs" }), now - 1.3 * minute, 90),
  tool("pc_commit", "run_parser_2", "shell", JSON.stringify({ command: "cargo test -p parser && git commit -am 'fix(parser): the loop bound'" }), now - 1.2 * minute, 5_200),
];

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
  ...(requestedThread() === PARSER_THREAD_ID ? { [PARSER_THREAD_ID]: parserEntries() } : {}),
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
  ...(requestedThread() === PARSER_THREAD_ID ? parserRuns : []),
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
  // A run still producing output, and one stopped on a question — the two states
  // the recency view's active stretch exists to hold. `list_latest_run_infos`
  // reads the *first* run of a thread, which is why each of these is the only
  // entry for its thread.
  {
    id: "run_longformer",
    threadId: "th_longformer",
    status: "running",
    modelProvider: "future",
    modelId: "deepseek-v4-pro",
    startedAt: now - 6 * minute,
    endedAt: null,
    createdAt: now - 6 * minute,
    updatedAt: now - minute,
  },
  {
    id: "run_sc_waiting",
    threadId: "th_sc",
    status: "waiting_approval",
    modelProvider: "future",
    modelId: "deepseek-v4-pro",
    startedAt: now - 118 * minute,
    endedAt: null,
    createdAt: now - 118 * minute,
    updatedAt: now - 40 * minute,
  },
  // A finished run on a thread that is *not* the one the screenshots open on.
  // The rail's unread dot needs both halves — a terminal run status here, and
  // the thread marked unread in the session's own set — and a run on the open
  // thread cannot show it: the hook clears that mark on mount.
  {
    id: "run_data",
    threadId: "th_data",
    status: "completed",
    modelProvider: "future",
    modelId: "deepseek-v4-pro",
    startedAt: now - 8 * minute,
    endedAt: now - 6 * minute,
    createdAt: now - 8 * minute,
    updatedAt: now - 6 * minute,
  },
  // And a *failed* one, so the other unread colour is reachable too.
  {
    id: "run_fig",
    threadId: "th_fig",
    status: "failed",
    modelProvider: "future",
    modelId: "deepseek-v4-pro",
    startedAt: now - 34 * minute,
    endedAt: now - 31 * minute,
    createdAt: now - 34 * minute,
    updatedAt: now - 31 * minute,
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
  ...(requestedThread() === PARSER_THREAD_ID ? parserToolCalls : []),
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

/**
 * A pending approval on *this* machine's open conversation.
 *
 * The harness had none, so the local approval card — the richest piece of the
 * conversation frame — could not be photographed at all. Shaped like a real
 * sandbox escalation: the sandbox refused the write, the model asked for
 * escalation and gave its reason, and the rule editor has something to offer.
 */
export const pendingApproval = {
  id: "apr_1",
  threadId: "th_review",
  runId: "run_2",
  toolCallId: "tc_9",
  kind: "shell",
  status: "pending",
  title: "运行 shell 命令",
  summary: "Rscript analysis/effect-size.R --out notes/effect-size.png",
  riskLevel: "medium",
  requestedAction: "shell",
  actionCategory: "shell",
  actionPayload: JSON.stringify({
    tool: "shell",
    category: "sandbox_escalation",
    behavior: "manage_files",
    summary: "沙箱拒绝写入 notes/effect-size.png",
    command: "Rscript analysis/effect-size.R --out notes/effect-size.png",
    justification: "脚本要把森林图写进 notes/，沙箱只允许读 papers/ 与 notes/ 之外的路径",
    escalationTrigger: "sandbox_failure",
    blockedPaths: ["notes/effect-size.png"],
    scope: {
      cwd: "/Users/lixin/dopamine-review",
      insideWorkspace: true,
      estimatedBlastRadius: "low",
    },
  }),
  sandboxBoundary: "/Users/lixin/dopamine-review",
  saveSuggestion: JSON.stringify({ path: "notes/**", access: "write" }),
  decisionNote: null,
  decidedAt: null,
  reviewer: "user",
  decisionScope: "session",
  decisionSource: "local",
  createdAt: now - 2 * minute,
  updatedAt: now - 2 * minute,
};

/**
 * A pending approval on the *host*, delivered the way a real one arrives: as an
 * `approval_request` event on the peer event stream. A remote approval has no
 * local store to read, so the fixture must push it rather than serve it.
 */
export const remoteApproval = {
  id: "rapr_1",
  sessionId: "rs_forest",
  toolName: "shell",
  title: "运行 shell 命令",
  summary: "Rscript analysis/effect-size.R --out notes/effect-size.png",
  riskLevel: "medium",
};

export const toolOutputs: Record<string, string> = {
  tc_3: `total 96
drwxr-xr-x  6 lixin  staff    192  9月 17 10:02 .
-rw-r--r--  1 lixin  staff  18422  9月 17 09:58 subjects.csv
-rw-r--r--  1 lixin  staff   9210  9月 16 21:14 questionnaire.csv
68 data/subjects.csv`,
  tc_8: `notes/frank-summary.md:12:sample size: n = 42, PET imaging, striatal binding potential
notes/dreher-summary.md:12:sample size: n = 68, plasma metabolite assay`,
  // The parser session's calls, verbatim from the TUI's `journal_*`: the file it
  // read, the suite before and after, the places the same comparison appears,
  // and the two commits.
  pc_read: `142  fn parse_expr(&mut self) -> Result<Expr> {
143      let mut lhs = self.parse_atom()?;
144      while self.peek() <= self.end {
145          let op = self.next();
146          let rhs = self.parse_atom()?;
147          lhs = apply(lhs, op, rhs);
148      }
149      Ok(lhs)
150  }
`,
  pc_test: `running 41 tests
test result: ok. 41 passed; 0 failed; 0 ignored
`,
  pc_grep: `src/parser.rs:144:        while self.peek() <= self.end {
src/parser.rs:318:        while self.peek() <= self.end {
src/args.rs:87:        while self.peek() <= self.end {
`,
  pc_edit: `patched src/args.rs:87
1 file changed, 1 insertion(+), 1 deletion(-)
`,
  pc_commit: `test result: ok. 44 passed; 0 failed; 0 ignored
[parser 6b1f0c2] fix(parser): the loop bound
 3 files changed, 3 insertions(+), 3 deletions(-)
`,
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
  // The model the parser session runs on — the same one the TUI's scripted agent
  // reports, so the two frames describe one session rather than two.
  { id: "deepseek-flash", label: "DeepSeek Flash", provider: "future", reasoning: true, thinkingLevel: "medium", contextWindow: 200_000, supportsImages: false, description: "快模型，适合定位 bug 与改代码", descriptionEn: "Fast model for bug hunting and small edits" },
  { id: "deepseek-v4", label: "DeepSeek V4", provider: "future", reasoning: true, thinkingLevel: "medium", contextWindow: 128_000, supportsImages: true, description: "日常问答与改写的快模型", descriptionEn: "Fast model for everyday work" },
  { id: "future-flash", label: "Future Flash", provider: "future", reasoning: false, thinkingLevel: "off", contextWindow: 64_000, description: "低延迟，适合小改动", descriptionEn: "Low latency for small edits" },
  { id: "future-vision", label: "Future Vision", provider: "future", reasoning: true, thinkingLevel: "medium", contextWindow: 200_000, supportsImages: true, description: "看图、读图表与扫描件", descriptionEn: "Reads figures and scans" },
];

export const providersView = {
  builtin: [
    { id: "future", name: "FutureOS", baseUrl: "https://api.future-os.cn", hasApiKey: true, modelCount: 5, requiresBaseUrl: false },
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
  // One model is hidden so the settings page has to show it with its switch
  // off — the state the picker's list cannot represent.
  hiddenModels: ["future/future-vision"],
  autoUpgradeSkills: true,
  autoConnectRemote: true,
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
    { name: "effect-size.png", path: `${HOME}/Research/dopamine-decision/figures/effect-size.png`, isDir: false, size: 62_400, modified: now - 4 * minute },
    { name: "forest-plot.png", path: `${HOME}/Research/dopamine-decision/figures/forest-plot.png`, isDir: false, size: 88_100, modified: now - 2 * minute },
  ],
  [`${HOME}/Research/dopamine-decision/notes`]: [
    { name: "compare.md", path: `${HOME}/Research/dopamine-decision/notes/compare.md`, isDir: false, size: 4_180, modified: now - 3 * minute },
    { name: "dreher-summary.md", path: `${HOME}/Research/dopamine-decision/notes/dreher-summary.md`, isDir: false, size: 2_960, modified: now - 2 * hour },
    { name: "frank-summary.md", path: `${HOME}/Research/dopamine-decision/notes/frank-summary.md`, isDir: false, size: 3_120, modified: now - 2 * hour },
    { name: "effect-size.png", path: `${HOME}/Research/dopamine-decision/notes/effect-size.png`, isDir: false, size: 62_400, modified: now - 4 * minute },
  ],
};

/**
 * The expression-parser session's checkout.
 *
 * The right-hand Files panel lists the *active workspace*, so a capture that
 * opens it needs a directory that looks like the repository the conversation is
 * about: the two files the agent read and edited (`src/parser.rs`, `src/args.rs`),
 * the test module the diagnosis points at (`src/parser/tests.rs`), and the
 * manifest and the agent guide beside them. Keyed by path, so nothing else in
 * the fixture changes.
 */
const REPO = `${HOME}/future-os`;
workspaceFiles[REPO] = [
  { name: "docs", path: `${REPO}/docs`, isDir: true, size: 0, modified: now - 6 * hour },
  { name: "src", path: `${REPO}/src`, isDir: true, size: 0, modified: now - 4 * minute },
  { name: "AGENTS.md", path: `${REPO}/AGENTS.md`, isDir: false, size: 8_960, modified: now - 2 * day },
  { name: "Cargo.toml", path: `${REPO}/Cargo.toml`, isDir: false, size: 1_940, modified: now - 3 * day },
  { name: "README.md", path: `${REPO}/README.md`, isDir: false, size: 3_400, modified: now - 9 * day },
  { name: "rust-toolchain.toml", path: `${REPO}/rust-toolchain.toml`, isDir: false, size: 132, modified: now - 11 * day },
];
workspaceFiles[`${REPO}/src`] = [
  { name: "parser", path: `${REPO}/src/parser`, isDir: true, size: 0, modified: now - 2 * minute },
  { name: "args.rs", path: `${REPO}/src/args.rs`, isDir: false, size: 4_120, modified: now - 5 * minute },
  { name: "lexer.rs", path: `${REPO}/src/lexer.rs`, isDir: false, size: 9_870, modified: now - 3 * day },
  { name: "main.rs", path: `${REPO}/src/main.rs`, isDir: false, size: 2_310, modified: now - 8 * day },
  { name: "parser.rs", path: `${REPO}/src/parser.rs`, isDir: false, size: 14_260, modified: now - 6 * minute },
];
workspaceFiles[`${REPO}/src/parser`] = [
  { name: "tests.rs", path: `${REPO}/src/parser/tests.rs`, isDir: false, size: 7_540, modified: now - 2 * minute },
];
workspaceFiles[`${REPO}/docs`] = [
  { name: "parser.md", path: `${REPO}/docs/parser.md`, isDir: false, size: 5_180, modified: now - 4 * hour },
];

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
  /**
   * Who may drive that machine, as its own presence last stated.
   *
   * Optional because every scenario written before a capture needed the held
   * state answers without it, and the UI reads its absence as "no statement" —
   * which is an older host, and means "assume ours".
   */
  control?: "active" | "standby" | null;
  /** The device holding it, when `control` is `standby`. */
  holder?: { pairId: string; name?: string; kind?: string } | null;
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
      // The host's skills go in the client's `/` menu, which is the capability
      // that decides whether they are asked for at all (the real host declares
      // it; a host that did not would have a commands-only menu).
      "skills_v1",
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
      title: pick("重画森林图并核对置信区间", "Redraw the Forest Plot, Check the Intervals"),
      mode: "workspace",
      workspaceId: "ws_figures",
      workspaceName: pick("图表脚本", "Figure Scripts"),
      // The same field the host's catalogue carries (a workspace's pin lives on
      // its conversations' rows), and the same state the mock's workspace list
      // reports: the group leads the merged list, with its marker.
      workspacePinned: true,
      // When that *workspace* was last opened there — the key the merged list
      // places the group by, so the host's group holds its rank while its
      // conversations (the newest on either machine, 90s ago) come and go.
      workspaceOrderedAt: Date.now() - 30 * 60_000,
      pinned: false,
      streaming: true,
      lastMessageAt: Date.now() - 90_000,
    },
    {
      sessionId: "rs_forest_mono",
      threadId: "th_forest_mono",
      title: pick("把森林图改成黑白版", "A Black-and-White Forest Plot"),
      mode: "workspace",
      workspaceId: "ws_figures",
      workspaceName: pick("图表脚本", "Figure Scripts"),
      workspacePinned: true,
      workspaceOrderedAt: Date.now() - 30 * 60_000,
      // A fork on that machine, as its catalogue reports it: the row is drawn
      // indented under the conversation it came from, folded by the same toggle
      // the local list uses.
      //
      // *Newer* than the conversation it was forked from, which is what a fork is:
      // it was made later, and its last message later still. An older fork hid a
      // real ordering bug — ranked by its own time it lands just below its parent
      // anyway, so a list that drew forks above their parents looked right here
      // and was wrong on every conversation somebody had actually continued.
      parentSessionId: "rs_forest",
      pinned: false,
      streaming: false,
      lastMessageAt: Date.now() - 30_000,
    },
    {
      sessionId: "rs_prisma",
      threadId: "th_prisma",
      title: pick("PRISMA 流程图的筛选数字对不上", "PRISMA Flow Diagram: Counts Do Not Add Up"),
      mode: "workspace",
      workspaceId: "ws_figures",
      workspaceName: pick("图表脚本", "Figure Scripts"),
      workspacePinned: true,
      workspaceOrderedAt: Date.now() - 30 * 60_000,
      pinned: true,
      streaming: false,
      lastMessageAt: Date.now() - 40 * 60_000,
    },
    {
      sessionId: "rs_reviewer",
      threadId: "th_reviewer",
      title: pick("审稿意见逐条回复草稿", "Point-by-Point Reply to the Reviewer"),
      mode: "chat",
      workspaceId: null,
      pinned: false,
      streaming: false,
      lastMessageAt: Date.now() - 3 * 60 * 60_000,
    },
  ],
};

/** A remote conversation's transcript, in the host's own entry shape. */

/**
 * More of that host's conversation, so the transcript overflows its viewport.
 *
 * The scroll behaviour is about content taller than the window: with a short
 * conversation `scrollHeight === clientHeight`, every "is it at the bottom?"
 * check is trivially true, and the read-anchor behaviour cannot happen at all.
 * These exchanges are generated rather than hand-written because their only
 * property that matters is their length.
 */
function remoteFillerExchanges(): unknown[] {
  const out: unknown[] = [];
  for (let index = 1; index <= 14; index += 1) {
    const at = Date.now() - (20 - index) * 60_000;
    out.push({
      id: `re_fill_u${index}`,
      kind: "message",
      role: "user",
      createdAtMs: at,
      runId: `rrun_fill_${index}`,
      blocks: [{ kind: "text", text: `第 ${index} 轮：把 ${index} 号图的分组标签核对一遍，并把结论写进表格。` }],
    });
    out.push({
      id: `re_fill_a${index}`,
      kind: "message",
      role: "assistant",
      createdAtMs: at + 30_000,
      runId: `rrun_fill_${index}`,
      run: { durationMs: 20_000, status: "completed" },
      blocks: [
        { kind: "text", text: `${index} 号图核对完成：分组标签与图注一致，已写入表格第 ${index} 行。` },
        { kind: "tool_call", toolCallId: `rtc_fill_${index}`, name: "read" },
      ],
    });
  }
  return out;
}

export const remoteEntries: Record<string, unknown[]> = {
  rs_forest: [
    ...remoteFillerExchanges(),
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
      // A reasoning block followed by a burst of tool calls, so the host's
      // transcript renders the two things a reader compares against a local
      // reply: the thinking block, and the folded step summary. The host's
      // entries carried neither before — no `reasoning` block existed and every
      // tool call stood alone, so a fold could not form and the remote
      // conversation showed *only* the unfolded shape.
      blocks: [
        {
          kind: "reasoning",
          text: "先看脚本里的置信区间是怎么算的——区间水平多半写成参数，直接读文件比在正文里猜快。三个候选文件都读一遍，免得只改到一张图。",
        },
        { kind: "tool_call", toolCallId: "rtc_1", name: "read" },
        { kind: "tool_call", toolCallId: "rtc_3", name: "read" },
        { kind: "tool_call", toolCallId: "rtc_4", name: "read" },
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
  rtc_3: { name: "read", arguments: { file_path: "analysis/fig2-forest.R" } },
  rtc_4: { name: "read", arguments: { file_path: "analysis/figS3.R" } },
  // The call the mid-flight run makes while the reader is watching (`remoteStream.ts`):
  // its row is drawn from the push lane, and fetched back by identity once the
  // transcript's own page carries it.
  call_note: { name: "write", arguments: { path: "analysis/forest-ci-note.md" } },
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
    { id: "ws_figures", kind: "user", name: pick("图表脚本", "Figure Scripts"), path: "/Users/studio/papers/figures", pinned: true },
    { id: "ws_manuscript", kind: "user", name: pick("投稿稿件", "Submission"), path: "/Users/studio/papers/submission", pinned: false },
  ],
};

/**
 * One directory inside that host's conversation.
 *
 * Two files the overlay can render and one it cannot, which is the distinction the
 * panel draws: `forest-plot.png` is a real PNG under `shot/assets/`, so the
 * preview the panel opens paints an actual picture rather than a broken image.
 * The folder is the third kind of row: it carries neither a size label nor a
 * download control, so a listing of files alone could not show whether all three
 * kinds are drawn at one height.
 */
export const remoteFileListing = {
  rootPath: "/Users/studio/papers/figures",
  path: "/Users/studio/papers/figures",
  entries: [
    { name: "raw", path: "/Users/studio/papers/figures/raw", isDir: true, size: 0 },
    { name: "forest-plot.png", path: "/Users/studio/papers/figures/forest-plot.png", isDir: false, size: 184_320 },
    { name: "effect-size.R", path: "/Users/studio/papers/figures/effect-size.R", isDir: false, size: 3_411 },
    { name: "archive.zip", path: "/Users/studio/papers/figures/archive.zip", isDir: false, size: 2_048_576 },
  ],
};
