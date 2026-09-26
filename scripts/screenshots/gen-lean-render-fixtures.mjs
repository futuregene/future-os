#!/usr/bin/env node
/**
 * Author the *full-feed* render fixtures and derive the *lean* ones from them
 * through the shipping Rust trims.
 *
 * The mobile screenshot harness (SHOT_WEB=1) feeds its conversation screen from
 * the real projection code; the fixtures here are what it feeds it. Two slices:
 *
 *   render-full-lane-events.json      live-lane wire events, full feed
 *   render-full-history-entries.json  a history page, full feed
 *   render-lean-lane-events.json      <- lean_event_data() applied per event
 *   render-lean-history-entries.json  <- lean_entries() applied in place
 *
 * The lean files are generated, never hand-edited: `desktop/src-tauri`'s
 * ignored `remote_host::lean::tests::generate_render_fixtures` test writes them,
 * so a render capture is fed the exact bytes a phone would receive from the
 * code under test. Regenerate after any trim change:
 *
 *   node scripts/screenshots/gen-lean-render-fixtures.mjs
 *
 * Wire shapes are those the agent really emits, not an idealised version:
 * `agent/src/rpc/prompt_helpers.rs` (RunEvent/ModelStreamEvent -> SSE), the
 * `[exit: N]` footer from `agent/src/tools/mod.rs`, and the history relay's
 * entry shape (blocks with `arguments`, `isError`).
 *
 * The history page's outcome flag is the canonical post-#848/#849 shape: a
 * recorded failure says `isError: true` (`tools::outcome_is_error`), and a
 * success — or a judged soft failure — sends no flag at all, because #849
 * stops writing the uninformative `false`. A page that still carried the old
 * `isError: false` shape would let the render checks pass against a payload
 * the phone never receives.
 */
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const DIR = join(ROOT, "mobile", "shot", "mock", "fixtures");

const RUN = "run_verify_lean";
const RUN_HISTORY = "run_verify_lean_history";
const T0 = Date.parse("2026-09-26T09:12:00+08:00");

const USER_TEXT = "帮我看一下工作区的笔记，把两份结论整理成一张对比表。";

/** One wire event of the live lane, as `SseEvent` serializes it. */
const ev = (idx, type, data) => ({ type, data: JSON.stringify(data), runId: RUN, idx });

/**
 * One tool call as the lane publishes it: the model's input phase (partial
 * arguments text and a `tool_delta` fragment — both gone from the lean feed),
 * then the execution phase with the complete `tool_args`, then the outcome.
 */
function toolCall(start, { id, name, command, path, delta, end }) {
  const partial = command !== undefined
    ? `{"command":"${command.slice(0, 12)}`
    : `{"path":"${path.slice(0, 40)}`;
  const input = command !== undefined ? { command } : { path };
  const events = [
    ev(start, "tool_start", {
      type: "tool_start",
      phase: "input",
      tool_name: name,
      tool_id: id,
      tool_args: partial,
    }),
    ev(start + 1, "tool_delta", {
      snapshot: false,
      type: "tool_delta",
      text: delta,
      tool_id: id,
    }),
    ev(start + 2, "tool_start", {
      type: "tool_start",
      phase: "execution",
      tool_name: name,
      tool_id: id,
      tool_args: input,
    }),
    ev(start + 3, "tool_end", { type: "tool_end", ...end }),
  ];
  return events;
}

// ─── live lane: full feed ───────────────────────────────────────────────────

const REASONING = [
  "用户要的是一张对比表，不是复述。先把两份笔记读进来，确认各自的效应量方向，",
  "再决定按什么维度对比。Frank 2024 报 d = 0.42，Dreher 2025 报的却是条件效应——",
  "两者不冲突，适用条件不同，表格里要把这个说清楚。",
];

const fullLaneEvents = [
  ev(0, "user_message", {
    text: USER_TEXT,
    entry_id: "e_verify_user",
    run_id: RUN,
    created_at_ms: T0,
    attachments: [],
  }),
  ev(1, "agent_start", { started_at_ms: T0 + 400, type: "agent_start" }),
  ev(2, "thinking_start", { type: "thinking_start", block_id: "rs_1" }),
  ...REASONING.map((text, index) =>
    ev(3 + index, "thinking_delta", { type: "thinking_delta", text, block_id: "rs_1" })),
  ev(6, "thinking_end", { type: "thinking_end", block_id: "rs_1" }),

  ev(7, "text_chunk", { text: "先看看工作区里有什么。" }),
  ...toolCall(8, {
    id: "call_1",
    name: "shell",
    command: "future nosuch-subcommand --verbose",
    delta: " nosuch-subcommand --verbose",
    end: {
      exit_code: 127,
      text: "bash: future: command not found\n\n[exit: 127]",
      tool_name: "shell",
      tool_id: "call_1",
    },
  }),

  ev(12, "text_chunk", { text: "命令不在 PATH 里，换读笔记。" }),
  ...toolCall(13, {
    id: "call_2",
    name: "read",
    path: "/Users/lixin/Research/dopamine-decision/notes/frank-summary.md",
    delta: "notes/frank-summary.md\"}",
    end: {
      text: "# Frank 2024\n\n- 效应量 d = 0.42（95% CI 0.21–0.63）\n- N = 148",
      tool_name: "read",
      tool_id: "call_2",
    },
  }),

  ev(17, "text_chunk", { text: "再看第二份。" }),
  ...toolCall(18, {
    id: "call_3",
    name: "read",
    path: "/Users/lixin/Research/dopamine-decision/notes/dreher-summary.md",
    delta: "notes/dreher-summary.md\"}",
    end: {
      text: "# Dreher 2025\n\n- 不确定性高时效应反转\n- N = 96",
      tool_name: "read",
      tool_id: "call_3",
    },
  }),

  ev(22, "text_chunk", { text: "把两张表的行对齐，写成对比表。" }),
  ...toolCall(23, {
    id: "call_4",
    name: "write",
    path: "/Users/lixin/Research/dopamine-decision/notes/compare-table.md",
    delta: "notes/compare-table.md\"}",
    end: {
      target_path: "/Users/lixin/Research/dopamine-decision/notes/compare-table.md",
      text: "已写入 notes/compare-table.md（812 字节）",
      tool_name: "write",
      tool_id: "call_4",
    },
  }),

  ev(27, "text_chunk", { text: "补上适用条件那一列。" }),
  ...toolCall(28, {
    id: "call_5",
    name: "edit",
    path: "/Users/lixin/Research/dopamine-decision/notes/compare-table.md",
    delta: "notes/compare-table.md\"}",
    end: {
      target_path: "/Users/lixin/Research/dopamine-decision/notes/compare-table.md",
      text: "已修改 1 处",
      tool_name: "edit",
      tool_id: "call_5",
    },
  }),

  // A soft failure the *agent* already judged: exit 1 from a bare grep is the
  // command's no-match signal, and `is_soft_fail` says so.
  ev(32, "text_chunk", { text: "确认一下引用有没有漏。" }),
  ...toolCall(33, {
    id: "call_6",
    name: "shell",
    command: "grep -c dopamine notes/compare-table.md",
    delta: " -c dopamine notes/compare-table.md",
    end: {
      exit_code: 1,
      is_soft_fail: true,
      text: "0\n\n[exit: 1]",
      tool_name: "shell",
      tool_id: "call_6",
    },
  }),

  // The same shape from an agent that predates the flag: the client's own
  // exemption has to carry the verdict (bare `diff`, exit 1 = files differ).
  ev(37, "text_chunk", { text: "两份摘要确实不一样，这是正常的。" }),
  ...toolCall(38, {
    id: "call_7",
    name: "shell",
    command: "diff notes/frank-summary.md notes/dreher-summary.md",
    delta: " notes/frank-summary.md notes/dreher-summary.md",
    end: {
      exit_code: 1,
      text: "1,3c1,3\n< - 效应量 d = 0.42\n---\n> - 不确定性高时效应反转\n\n[exit: 1]",
      tool_name: "shell",
      tool_id: "call_7",
    },
  }),

  ev(42, "text_chunk", { text: "再确认一下输出目录还在不在。" }),
  // The pre-semantics shape: the outcome rides only in the `[exit: N]` footer.
  // The full feed still reads it; the lean trim drops it, because it drops the
  // whole output — so this row is the documented price of the lean lane, not an
  // accident (see the render report).
  ...toolCall(43, {
    id: "call_8",
    name: "shell",
    command: "ls /nonexistent-workspace-path",
    delta: " /nonexistent-workspace-path",
    end: {
      text: "ls: cannot access '/nonexistent-workspace-path': No such file or directory\n\n[exit: 2]",
      tool_name: "shell",
      tool_id: "call_8",
    },
  }),

  ev(47, "text_chunk", { text: "对比表写好了：三行两列，结论方向一致、适用条件不同。" }),
  ev(48, "usage", { usage: { output_tokens: 983 } }),
  ev(49, "agent_end", {
    type: "agent_end",
    state: "completed",
    usage: { output_tokens: 983 },
    duration_ms: 61_234,
  }),
];

// ─── live lane, mid-run: full feed ──────────────────────────────────────────
//
// A run at the moment a capture catches it: the read call is in flight (its
// `tool_end` has not been published) and a reasoning block has opened as the
// newest slice, with no end boundary and no deltas to carry its body — which is
// exactly what the lean lane looks like mid-thought.

const fullLaneMidEvents = [
  ev(0, "user_message", {
    text: USER_TEXT,
    entry_id: "e_verify_mid_user",
    run_id: RUN,
    created_at_ms: T0,
    attachments: [],
  }),
  ev(1, "agent_start", { started_at_ms: T0 + 400, type: "agent_start" }),
  ev(2, "text_chunk", { text: "先把第一份笔记读进来。" }),
  // The read call is in flight: args streamed, no `tool_end` yet.
  ev(3, "tool_start", {
    type: "tool_start",
    phase: "input",
    tool_name: "read",
    tool_id: "call_mid_1",
    tool_args: '{"path":"/Users/lixin/Research/dopamine-decision/notes/frank-summ',
  }),
  ev(4, "tool_delta", {
    snapshot: false,
    type: "tool_delta",
    text: "notes/frank-summary.md\"}",
    tool_id: "call_mid_1",
  }),
  ev(5, "tool_start", {
    type: "tool_start",
    phase: "execution",
    tool_name: "read",
    tool_id: "call_mid_1",
    tool_args: { path: "/Users/lixin/Research/dopamine-decision/notes/frank-summary.md" },
  }),
  ev(6, "thinking_start", { type: "thinking_start", block_id: "rs_mid" }),
  ...["先记住 d = 0.42 这个数，", "待会儿和 Dreher 的条件效应放在一起看。"].map((text, index) =>
    ev(7 + index, "thinking_delta", { type: "thinking_delta", text, block_id: "rs_mid" })),
];

// ─── history page: full feed ────────────────────────────────────────────────

const fullHistoryEntries = [
  {
    id: "he_user",
    kind: "message",
    role: "user",
    createdAtMs: T0,
    runId: RUN_HISTORY,
    blocks: [{ kind: "text", text: USER_TEXT }],
  },
  {
    id: "he_assistant",
    kind: "message",
    role: "assistant",
    createdAtMs: T0 + 4_000,
    runId: RUN_HISTORY,
    usage: { inputTokens: 18_420, outputTokens: 1_260 },
    run: { status: "completed", durationMs: 74_300 },
    blocks: [
      { kind: "reasoning", text: REASONING.join("") },
      { kind: "text", text: "先看看工作区里有什么。" },
      {
        kind: "tool_call",
        toolCallId: "htc_1",
        name: "shell",
        arguments: {
          command: "future nosuch-subcommand --verbose",
          timeout: 30,
          cwd: "/Users/lixin/Research/dopamine-decision",
        },
      },
      { kind: "text", text: "命令不在 PATH 里，换读笔记。" },
      {
        kind: "tool_call",
        toolCallId: "htc_2",
        name: "read",
        arguments: {
          path: "/Users/lixin/Research/dopamine-decision/notes/frank-summary.md",
          offset: 0,
          limit: 200,
        },
      },
      { kind: "text", text: "写成对比表。" },
      {
        kind: "tool_call",
        toolCallId: "htc_3",
        name: "write",
        arguments: {
          path: "/Users/lixin/Research/dopamine-decision/notes/compare-table.md",
          content: "# 对比表\n\n| 维度 | Frank 2024 | Dreher 2025 |\n",
        },
      },
      { kind: "text", text: "对比表写好了。" },
      {
        // The soft-fail call: a bare `grep` exiting 1 is its no-match signal,
        // and the agent does not record it as an error.
        kind: "tool_call",
        toolCallId: "htc_4",
        name: "shell",
        arguments: { command: "grep -c dopamine notes/compare-table.md", timeout: 30 },
      },
      { kind: "text", text: "顺手把两份笔记差也跑一下。" },
      {
        // A burst: two consecutive completed shell calls with no prose between
        // them, which the phone folds into one "运行 2 次" row. A lean page
        // carries neither command, and the folded row has no call identity of
        // its own — so each child has to be able to fetch its own.
        kind: "tool_call",
        toolCallId: "htc_5",
        name: "shell",
        arguments: {
          command: "diff notes/frank-summary.md notes/dreher-summary.md",
          timeout: 30,
        },
      },
      {
        kind: "tool_call",
        toolCallId: "htc_6",
        name: "shell",
        arguments: { command: "wc -l notes/compare-table.md", timeout: 30 },
      },
    ],
  },
  {
    id: "he_tool",
    kind: "tool",
    role: "tool",
    createdAtMs: T0 + 5_000,
    runId: RUN_HISTORY,
    blocks: [
      {
        // The run's one recorded failure: the agent's verdict (`is_error`
        // from exit 127, not a soft-fail command) is the only outcome signal
        // a trimmed history row still carries.
        kind: "tool_result",
        toolCallId: "htc_1",
        isError: true,
        text: "bash: future: command not found\n\n[exit: 127]",
      },
      {
        kind: "tool_result",
        toolCallId: "htc_2",
        text: "# Frank 2024\n\n- 效应量 d = 0.42（95% CI 0.21–0.63）\n- N = 148",
      },
      {
        kind: "tool_result",
        toolCallId: "htc_3",
        text: "已写入 notes/compare-table.md（812 字节）",
      },
      {
        // Not an error to the agent (soft-fail grep), so no flag — and a
        // success would send no `isError: false` either (#849 omits it).
        kind: "tool_result",
        toolCallId: "htc_4",
        text: "0\n\n[exit: 1]",
      },
      {
        kind: "tool_result",
        toolCallId: "htc_5",
        text: "1,3c1,3\n< - 效应量 d = 0.42\n---\n> - 不确定性高时效应反转\n\n[exit: 1]",
      },
      {
        kind: "tool_result",
        toolCallId: "htc_6",
        text: "12 notes/compare-table.md\n\n[exit: 0]",
      },
    ],
  },
];

mkdirSync(DIR, { recursive: true });
writeFileSync(join(DIR, "render-full-lane-events.json"), `${JSON.stringify(fullLaneEvents, null, 2)}\n`);
writeFileSync(join(DIR, "render-full-lane-mid-events.json"), `${JSON.stringify(fullLaneMidEvents, null, 2)}\n`);
writeFileSync(join(DIR, "render-full-history-entries.json"), `${JSON.stringify(fullHistoryEntries, null, 2)}\n`);
console.log(`authored full fixtures in ${DIR}`);

const generated = spawnSync(
  "cargo",
  [
    "test",
    "--lib",
    "remote_host::lean::tests::generate_render_fixtures",
    "--",
    "--ignored",
    "--exact",
    "--nocapture",
  ],
  {
    cwd: join(ROOT, "desktop", "src-tauri"),
    env: { ...process.env, LEAN_RENDER_FIXTURES: DIR },
    stdio: "inherit",
  },
);
if (generated.status !== 0)
  process.exit(generated.status ?? 1);
console.log("lean fixtures derived by the shipping trims");
