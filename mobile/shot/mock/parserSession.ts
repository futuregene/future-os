/**
 * The scripted session the TUI frames were captured with, mirrored for the
 * phone harness.
 *
 * `?parser=1` swaps this in for the whole demo catalogue (see
 * `RemoteContext.tsx`): the paired machine's name, the three workspace groups
 * and the conversation a capture opens. A phone frame and the TUI frames under
 * `.screenshots/tui-zh/` and `.screenshots/tui-en/` are then two pictures of
 * *one* session — same bug, same two turns, same code block and table, same two
 * commits — rather than two sessions that happen to share a subject.
 *
 * The words come from `tui/examples/demo_agent.rs` (`journal_zh` / `journal_en`,
 * `session_name`, `sessions`); only the shapes are the phone's (`blocks`,
 * `createdAtMs`, `runId`, the entry a tool result travels in). Each language is
 * written as that reader's frame, exactly like the TUI's two journals: the
 * English payload is not a translation of the Chinese one. Which language that
 * is comes from `./language`, the harness's one place for it.
 *
 * The rail is the TUI's newest rows per workspace rather than all 58: a phone
 * list shows a screenful, and the three groups, their titles and their ages are
 * the part that carries over.
 */

import { shotLanguage } from "./language";

/** The conversation a `?parser=1` capture opens; also the TUI's `demo-session`. */
export const parserSessionId = "sess_parser_offbyone";

/** The user's own name for the machine this conversation lives on. */
export const parserDesktopName = shotLanguage === "zh" ? "实验室的 MacBook Pro" : "Lab MacBook Pro";

/** The model the scripted session runs on, as the TUI's `state()` reports it. */
export const parserModels = [
  { id: "deepseek-flash", label: "DeepSeek Flash", provider: "future", isDefault: true, supportsImages: false },
];

const parserSessionTitle = shotLanguage === "zh"
  ? "表达式解析器里的 off-by-one"
  : "Off-by-One in the Expression Parser";

const parserWorkspaceRows = [
  {
    id: "ws_future_os",
    name: "future-os",
    path: "/Users/you/future-os",
    pinned: true,
  },
  {
    id: "ws_ui",
    name: "ui",
    path: "/Users/you/future-os/.worktrees/ui",
  },
  {
    id: "ws_notes",
    name: "notes",
    path: "/Users/you/notes",
  },
];

export const parserWorkspaces = parserWorkspaceRows;

/**
 * When each conversation last produced a reply, in minutes before load: the
 * TUI rail's own ages (`sessions()`), so the two lists read the same.
 */
const parserRailZh: [workspaceId: string, title: string, minutesAgo: number, queries: number][] = [
  ["ws_future_os", parserSessionTitle, 0, 1],
  ["ws_future_os", "重试层挡住了静默的流", 8, 23],
  ["ws_future_os", "侧栏：把一个工作区折起来", 26, 9],
  ["ws_future_os", "短查询的命盘排序", 51, 14],
  ["ws_future_os", "目标面板的覆盖率闸门", 95, 31],
  ["ws_future_os", "SQLite 视图 prepare 要 12ms", 140, 7],
  ["ws_ui", "工作台布局草图", 240, 12],
  ["ws_future_os", "静默 48 秒后重连", 300, 18],
  ["ws_future_os", "把重试层从桥里拆出来", 420, 6],
  ["ws_ui", "按键审计：ctrl+g 与 ctrl+d", 600, 11],
  ["ws_future_os", "Windows 命名管道冒烟测试", 1_500, 4],
  ["ws_future_os", "夜里那次覆盖率跑", 2_100, 4],
  ["ws_notes", "压缩数字的笔记", 3_000, 3],
  ["ws_notes", "目录结构那一页", 5_400, 2],
  ["ws_ui", "命盘需要一个过滤行", 6_200, 8],
  ["ws_notes", "每次 run 花多少额度", 10_468, 3],
];

const parserRailEn: [workspaceId: string, title: string, minutesAgo: number, queries: number][] = [
  ["ws_future_os", parserSessionTitle, 0, 1],
  ["ws_future_os", "Retry Layer Hides a Silent Stream", 8, 23],
  ["ws_future_os", "Rail: Fold a Workspace Shut", 26, 9],
  ["ws_future_os", "Palette Ranking for Short Queries", 51, 14],
  ["ws_future_os", "Coverage Gate for the Goal Plane", 95, 31],
  ["ws_future_os", "SQLite View Prepare Is 12ms", 140, 7],
  ["ws_ui", "Workbench Layout Sketch", 240, 12],
  ["ws_future_os", "48s of Silence, Then a Reopen", 300, 18],
  ["ws_future_os", "Split the Retry out of the Bridge", 420, 6],
  ["ws_ui", "Keymap Audit: ctrl+g and ctrl+d", 600, 11],
  ["ws_future_os", "Windows Named Pipe Smoke Test", 1_500, 4],
  ["ws_future_os", "Nightly Coverage Run", 2_100, 4],
  ["ws_notes", "Notes on the Compaction Numbers", 3_000, 3],
  ["ws_notes", "Directory Layout Page", 5_400, 2],
  ["ws_ui", "The Palette Needs a Filter Row", 6_200, 8],
  ["ws_notes", "Cost per Run, in Credit Units", 10_468, 3],
];

/**
 * The rail. Timestamps are relative to load time, like the rest of the harness,
 * so the ages the list prints come out of the surface rather than being written
 * here as text.
 *
 * The open conversation is the newest row — the TUI opens on it too — and its
 * own stamp is what the list would call "just now".
 */
export const parserSessions = (shotLanguage === "zh" ? parserRailZh : parserRailEn).map(
  ([workspaceId, title, minutesAgo, queryCount], index) => ({
    sessionId: index === 0 ? parserSessionId : `sess_parser_rail_${index}`,
    threadId: `th_parser_${index}`,
    title,
    mode: "workspace" as const,
    workspaceId,
    workspaceName: parserWorkspaceRows.find(workspace => workspace.id === workspaceId)?.name ?? null,
    createdAt: Date.now() - (minutesAgo + 60) * 60_000,
    streaming: false,
    lastMessageAt: Date.now() - minutesAgo * 60_000,
    queryCount,
  }),
);

// ── The conversation ────────────────────────────────────────────────────────
// One entry per journal entry the TUI serves, in the same order, so the phone's
// own projector (`timelineFromEntries`) folds a turn into the same slices the
// TUI's does: a thought, the calls it made, then the answer.

const now = Date.now();
const minute = 60_000;
const RUN_ONE = "run_parser_1";
const RUN_TWO = "run_parser_2";

/** A scripted timestamp inside the session, counted back from load time. */
const at = (minutesAgo: number) => now - minutesAgo * minute;

const PARSER_READ_RESULT = `142  fn parse_expr(&mut self) -> Result<Expr> {
143      let mut lhs = self.parse_atom()?;
144      while self.peek() <= self.end {
145          let op = self.next();
146          let rhs = self.parse_atom()?;
147          lhs = apply(lhs, op, rhs);
148      }
149      Ok(lhs)
150  }
`;

const PARSER_TEST_RESULT = "running 41 tests\ntest result: ok. 41 passed; 0 failed; 0 ignored\n";

const PARSER_GREP_RESULT = `src/parser.rs:144:        while self.peek() <= self.end {
src/parser.rs:318:        while self.peek() <= self.end {
src/args.rs:87:        while self.peek() <= self.end {
`;

const PARSER_EDIT_RESULT = "patched src/args.rs:87\n1 file changed, 1 insertion(+), 1 deletion(-)\n";

const PARSER_COMMIT_RESULT = `test result: ok. 44 passed; 0 failed; 0 ignored
[parser 6b1f0c2] fix(parser): the loop bound
 3 files changed, 3 insertions(+), 3 deletions(-)
`;

/** The first answer: heading, bold and italic runs, inline code, a rust block, a table. */
const PARSER_ANSWER_ZH = `### 诊断

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

const PARSER_ANSWER_EN = `### Diagnosis

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

/** The closing answer: two commits, a numbered list and a quote. */
const PARSER_CLOSING_ZH = `分成 **两个 commit** 落地，都是绿的：

1. \`6b1f0c2\` —— 三处边界，*只改行为*
2. \`d41e8aa\` —— 表驱动的用例，*只改覆盖*

> \`cargo test -p parser\`：44 passed，0 failed，0 ignored。

回滚第二个 commit 之后 \`41 passed\` 原样回来——这就是你要的那条性质，别的东西一个都没动。`;

const PARSER_CLOSING_EN = `Landed in **two commits**, both green:

1. \`6b1f0c2\` — the three bounds, *behaviour only*
2. \`d41e8aa\` — the table-driven test, *coverage only*

> \`cargo test -p parser\`: 44 passed, 0 failed, 0 ignored.

Reverting the second commit brings \`41 passed\` straight back — that is the property you asked for, and nothing else moved.`;

/** One scripted entry: the phone's shape, carrying the TUI's content. */
const entry = (
  id: string,
  role: "user" | "assistant" | "tool",
  minutesAgo: number,
  runId: string,
  blocks: Record<string, unknown>[],
) => ({ id, kind: "message", role, createdAtMs: at(minutesAgo), runId, blocks });

const parserEntriesZh = [
  entry("p1", "user", 2, RUN_ONE, [{
    kind: "text",
    text: "`1+2+` 返回的是 `3`，不是报错。表达式解析器里有个 off-by-one，找出来并补一个回归测试。",
  }]),
  entry("p2", "assistant", 1.9, RUN_ONE, [{
    kind: "reasoning",
    text: "先从 `parse_expr` 的循环边界看：光标用 `<=` 比较时，「到末尾」和「越过末尾」是同一个状态，尾随的运算符会被接受，而它的右操作数其实并不存在。",
  }]),
  entry("p3", "assistant", 1.8, RUN_ONE, [
    { kind: "tool_call", toolCallId: "call_read", name: "read", arguments: { path: "src/parser.rs", offset: 138, limit: 16 } },
  ]),
  entry("p4", "tool", 1.8, RUN_ONE, [
    { kind: "tool_result", toolCallId: "call_read", isError: false, text: PARSER_READ_RESULT },
  ]),
  entry("p5", "assistant", 1.7, RUN_ONE, [
    { kind: "tool_call", toolCallId: "call_test", name: "shell", arguments: { command: "cargo test -p parser" } },
  ]),
  entry("p6", "tool", 1.7, RUN_ONE, [
    { kind: "tool_result", toolCallId: "call_test", isError: false, text: PARSER_TEST_RESULT },
  ]),
  entry("p7", "assistant", 1.6, RUN_ONE, [{ kind: "text", text: PARSER_ANSWER_ZH }]),
  entry("p8", "user", 1.4, RUN_TWO, [{
    kind: "text",
    text: "别处还有同样的写法吗？顺手拆成两个 commit 提交——先边界，再测试。",
  }]),
  entry("p9", "assistant", 1.3, RUN_TWO, [{
    kind: "reasoning",
    text: "搜这个比较本身，而不是搜 `parse_expr`：错的是边界，不是函数。",
  }]),
  entry("p10", "assistant", 1.25, RUN_TWO, [
    { kind: "tool_call", toolCallId: "call_grep", name: "shell", arguments: { command: "rg -n 'peek\\(\\) <=' src/" } },
  ]),
  entry("p11", "tool", 1.25, RUN_TWO, [
    { kind: "tool_result", toolCallId: "call_grep", isError: false, text: PARSER_GREP_RESULT },
  ]),
  entry("p12", "assistant", 1.2, RUN_TWO, [{
    kind: "reasoning",
    text: "另外两处，都能从同一个入口到达。边界放一个 commit、用例表放另一个，这样回滚测试不会把行为改动一起带走，中间也不会留一个红的测试集。",
  }]),
  entry("p13", "assistant", 1.15, RUN_TWO, [
    { kind: "tool_call", toolCallId: "call_edit", name: "edit", arguments: { path: "src/args.rs", old: "while self.peek() <= self.end {", new: "while self.peek() < self.end {" } },
  ]),
  entry("p14", "tool", 1.15, RUN_TWO, [
    { kind: "tool_result", toolCallId: "call_edit", isError: false, text: PARSER_EDIT_RESULT },
  ]),
  entry("p15", "assistant", 1.1, RUN_TWO, [
    { kind: "tool_call", toolCallId: "call_commit", name: "shell", arguments: { command: "cargo test -p parser && git commit -am 'fix(parser): the loop bound'" } },
  ]),
  entry("p16", "tool", 1.1, RUN_TWO, [
    { kind: "tool_result", toolCallId: "call_commit", isError: false, text: PARSER_COMMIT_RESULT },
  ]),
  entry("p17", "assistant", 1, RUN_TWO, [{ kind: "text", text: PARSER_CLOSING_ZH }]),
];

const parserEntriesEn = [
  entry("p1", "user", 2, RUN_ONE, [{
    kind: "text",
    text: "`1+2+` returns `3` instead of failing. There is an off-by-one in the expression parser — find it and add a regression test.",
  }]),
  entry("p2", "assistant", 1.9, RUN_ONE, [{
    kind: "reasoning",
    text: "Start at `parse_expr`'s loop bound. If the cursor is compared with `<=`, \"at the end\" and \"past the end\" are the same state, and a trailing operator is accepted with an operand that is not there.",
  }]),
  entry("p3", "assistant", 1.8, RUN_ONE, [
    { kind: "tool_call", toolCallId: "call_read", name: "read", arguments: { path: "src/parser.rs", offset: 138, limit: 16 } },
  ]),
  entry("p4", "tool", 1.8, RUN_ONE, [
    { kind: "tool_result", toolCallId: "call_read", isError: false, text: PARSER_READ_RESULT },
  ]),
  entry("p5", "assistant", 1.7, RUN_ONE, [
    { kind: "tool_call", toolCallId: "call_test", name: "shell", arguments: { command: "cargo test -p parser" } },
  ]),
  entry("p6", "tool", 1.7, RUN_ONE, [
    { kind: "tool_result", toolCallId: "call_test", isError: false, text: PARSER_TEST_RESULT },
  ]),
  entry("p7", "assistant", 1.6, RUN_ONE, [{ kind: "text", text: PARSER_ANSWER_EN }]),
  entry("p8", "user", 1.4, RUN_TWO, [{
    kind: "text",
    text: "Does the same shape appear anywhere else? And land it as two commits — the bounds first, then the tests.",
  }]),
  entry("p9", "assistant", 1.3, RUN_TWO, [{
    kind: "reasoning",
    text: "Search for the comparison itself rather than for `parse_expr`: the mistake is the bound, not the function.",
  }]),
  entry("p10", "assistant", 1.25, RUN_TWO, [
    { kind: "tool_call", toolCallId: "call_grep", name: "shell", arguments: { command: "rg -n 'peek\\(\\) <=' src/" } },
  ]),
  entry("p11", "tool", 1.25, RUN_TWO, [
    { kind: "tool_result", toolCallId: "call_grep", isError: false, text: PARSER_GREP_RESULT },
  ]),
  entry("p12", "assistant", 1.2, RUN_TWO, [{
    kind: "reasoning",
    text: "Two more sites, both reachable from the same entry point. Bounds in one commit, the case table in the other, so reverting the tests cannot take the behaviour change with them — or leave a red suite in between.",
  }]),
  entry("p13", "assistant", 1.15, RUN_TWO, [
    { kind: "tool_call", toolCallId: "call_edit", name: "edit", arguments: { path: "src/args.rs", old: "while self.peek() <= self.end {", new: "while self.peek() < self.end {" } },
  ]),
  entry("p14", "tool", 1.15, RUN_TWO, [
    { kind: "tool_result", toolCallId: "call_edit", isError: false, text: PARSER_EDIT_RESULT },
  ]),
  entry("p15", "assistant", 1.1, RUN_TWO, [
    { kind: "tool_call", toolCallId: "call_commit", name: "shell", arguments: { command: "cargo test -p parser && git commit -am 'fix(parser): the loop bound'" } },
  ]),
  entry("p16", "tool", 1.1, RUN_TWO, [
    { kind: "tool_result", toolCallId: "call_commit", isError: false, text: PARSER_COMMIT_RESULT },
  ]),
  entry("p17", "assistant", 1, RUN_TWO, [{ kind: "text", text: PARSER_CLOSING_EN }]),
];

export const parserEntries = shotLanguage === "zh" ? parserEntriesZh : parserEntriesEn;
