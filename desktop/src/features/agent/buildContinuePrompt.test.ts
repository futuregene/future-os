import type { AgentMessage } from "@future-os/thread-projection";
import type {
  StoredRunEvent,
  StoredToolCall,
  StoredToolOutput,
} from "../../integrations/storage/threadStore";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  buildContinuePrompt,
  loadRunResumeSummary,
  previousUserForRun,
} from "./buildContinuePrompt";

const store = vi.hoisted(() => ({
  listRunEvents: vi.fn(),
  listToolCalls: vi.fn(),
  listToolOutputs: vi.fn(),
}));

vi.mock("../../integrations/storage/threadStore", () => ({
  listRunEvents: store.listRunEvents,
  listToolCalls: store.listToolCalls,
  listToolOutputs: store.listToolOutputs,
}));

function user(id: string, content = id): AgentMessage {
  return { id, role: "user", content, status: "complete" } as AgentMessage;
}

function assistant(id: string, runId?: string): AgentMessage {
  return { id, role: "assistant", content: id, runId, status: "complete" } as AgentMessage;
}

function tool(over: Partial<StoredToolCall> = {}): StoredToolCall {
  return {
    id: "t1",
    runId: "r1",
    name: "shell",
    kind: "shell",
    input: null,
    status: "completed",
    createdAt: 0,
    ...over,
  };
}

function output(over: Partial<StoredToolOutput> = {}): StoredToolOutput {
  return { id: "o1", toolCallId: "t1", kind: "stdout", content: null, createdAt: 0, ...over };
}

function event(over: Partial<StoredRunEvent> = {}): StoredRunEvent {
  return { id: "e1", runId: "r1", eventType: "agent_end", sequence: 1, createdAt: 0, ...over };
}

beforeEach(() => {
  store.listRunEvents.mockReset();
  store.listToolCalls.mockReset();
  store.listToolOutputs.mockReset();
});

describe("buildContinuePrompt", () => {
  it("is just the resume instruction when there is nothing to quote", () => {
    // boundary: no message, no summary, and whitespace-only values are all the
    // same case — the prompt must not gain an empty section.
    expect(buildContinuePrompt({})).toBe("继续上一个任务。");
    expect(buildContinuePrompt({ runId: "run-1" })).toBe("继续上一个任务。");
    expect(buildContinuePrompt({ message: user("u1", "   \n  ") })).toBe("继续上一个任务。");
    expect(buildContinuePrompt({ summary: "\t " })).toBe("继续上一个任务。");
  });

  it("quotes the trimmed failure message and the execution summary in order", () => {
    const prompt = buildContinuePrompt({
      message: user("u1", "  任务失败了  "),
      summary: "已跑测试",
    });
    expect(prompt).toBe(
      "继续上一个任务。\n\n上一条失败消息摘要:\n任务失败了\n\n已执行内容摘要:\n已跑测试",
    );
  });

  it("collapses multi-line bodies and hard-truncates at 1200 characters", () => {
    // boundary: newlines must not break the single-line quote.
    expect(buildContinuePrompt({ message: user("u1", " 第一行\n\n第二行 ") }))
      .toContain("上一条失败消息摘要:\n第一行 第二行");

    // boundary: exactly at the limit is kept, one past it is ellipsised.
    const atLimit = "x".repeat(1200);
    expect(buildContinuePrompt({ message: user("u1", atLimit) }).endsWith(atLimit)).toBe(true);
    const overLimit = buildContinuePrompt({ message: user("u1", "x".repeat(1201)) });
    expect(overLimit.endsWith(`${"x".repeat(1200)}...`)).toBe(true);

    // The suffix check above pins the *lower* bound only: `"x" * 1201 + "..."`
    // also ends with `"x" * 1200 + "..."`, so a cap raised to 1201 would slip
    // through. (An inverted probe I ran found exactly this: asserting the 1199
    // suffix passed, because repeated characters make any shorter run a suffix
    // of a longer one.) Put a marker at the cap so ONE assertion catches both
    // directions: the 1200th character must survive, and the 1201st must not.
    const marked = `${"x".repeat(1199)}Z${"x".repeat(200)}`;
    const markedResult = buildContinuePrompt({ message: user("u1", marked) });
    expect(markedResult.endsWith(`${"x".repeat(1199)}Z...`)).toBe(true);
    // Too few (cap 1199) would drop the Z; too many (cap 1201) would keep an x
    // after it. Both mutations now fail this line.
    const markedLines = markedResult.split("\n");
    expect(markedLines[markedLines.length - 1]).toBe(`${"x".repeat(1199)}Z...`);
  });

  it("truncates by code point, so CJK and astral characters survive intact", () => {
    // boundary: 1300 CJK characters is 1300 code points (not 3900 UTF-16
    // units), and the cut must not split a surrogate pair.
    const cjk = buildContinuePrompt({ message: user("u1", "字".repeat(1300)) });
    expect(cjk.endsWith(`${"字".repeat(1200)}...`)).toBe(true);

    const emoji = buildContinuePrompt({ message: user("u1", "🌊".repeat(1250)) });
    expect(emoji.endsWith(`${"🌊".repeat(1200)}...`)).toBe(true);
    expect(emoji.includes("\uFFFD")).toBe(false);
  });
});

describe("previousUserForRun", () => {
  const history = [
    user("u1"),
    assistant("a1", "run-1"),
    user("u2"),
    assistant("a2", "run-2"),
  ];

  it("returns the user message immediately preceding the run's assistant reply", () => {
    expect(previousUserForRun(history, "run-2")?.id).toBe("u2");
    expect(previousUserForRun(history, "run-1")?.id).toBe("u1");
  });

  it("falls back to the last user message when the run has no assistant reply yet", () => {
    // A crashed run may have persisted no assistant row: the recovery prompt
    // still needs the original request.
    expect(previousUserForRun(history, "run-unknown")?.id).toBe("u2");
  });

  it("returns null when there is no user message to recover", () => {
    // boundary: matched assistant at index 0 leaves nowhere to walk back to.
    expect(previousUserForRun([assistant("a1", "run-1")], "run-1")).toBeNull();
    // boundary: empty history.
    expect(previousUserForRun([], "run-1")).toBeNull();
    // boundary: the matched assistant sits at index 0 *and* later user messages
    // exist. The single-element history above cannot see this: there, the
    // "no assistant reply found" fallback also scans a list with no user
    // message and returns null, so BOTH the correct branch and the fallback
    // satisfy it. A mutation study found exactly that — replacing the `>= 0`
    // with `> 0` left all 14 tests green. With a later message present the two
    // paths diverge: the mutant walks back from the END of the thread and
    // quotes the last, unrelated user message as the thing to continue from.
    expect(previousUserForRun([assistant("a1", "run-1"), user("u2")], "run-1")).toBeNull();
    expect(
      previousUserForRun([assistant("a1", "run-1"), user("u2"), user("u3")], "run-1"),
    ).toBeNull();
  });
});

describe("loadRunResumeSummary", () => {
  it("summarises tool calls with their outputs and the terminal events", async () => {
    store.listRunEvents.mockResolvedValue([
      event({ id: "e1", eventType: "message_delta", payload: "ignored" }),
      event({ id: "e2", eventType: "tool_end", payload: "ran ls" }),
      event({ id: "e3", eventType: "error", payload: "boom" }),
    ]);
    store.listToolCalls.mockResolvedValue([
      tool({ id: "t1", name: "shell", input: JSON.stringify({ command: "npm test" }), status: "failed" }),
      tool({ id: "t2", name: "read_file", input: "not json" }),
    ]);
    store.listToolOutputs.mockImplementation(async (_runId: string, toolId: string) =>
      toolId === "t1"
        ? [output({ content: "  3 failed  " }), output({ id: "o2", kind: "stderr" })]
        : []);

    const summary = await loadRunResumeSummary("r1");
    expect(summary).toBe([
      "工具调用:",
      "- shell [failed]: npm test => 3 failed | stderr",
      "- read_file [completed]: not json",
      "最近事件:",
      "- tool_end: ran ls",
      "- error: boom",
    ].join("\n"));
    // The `message_delta` noise is dropped and the outputs are read per call.
    expect(store.listToolOutputs).toHaveBeenCalledWith("r1", "t1");
  });

  it("falls back to the raw input, then the tool name, when no command is present", async () => {
    store.listRunEvents.mockResolvedValue([]);
    store.listToolCalls.mockResolvedValue([
      tool({ id: "t1", name: "writer", input: JSON.stringify({ path: "a.txt" }) }),
      tool({ id: "t2", name: "nameless", input: null }),
    ]);
    store.listToolOutputs.mockResolvedValue([]);

    const summary = await loadRunResumeSummary("r1");
    expect(summary).toContain(`- writer [completed]: ${JSON.stringify({ path: "a.txt" })}`);
    expect(summary).toContain("- nameless [completed]: nameless");
    expect(summary).not.toContain("最近事件:");
  });

  it("reads at most eight tools and reports the ones it left out", async () => {
    // boundary: 9 tool calls — the 9th is summarised as a count, not a row.
    const calls = Array.from({ length: 9 }, (_, index) =>
      tool({ id: `t${index}`, name: `tool${index}` }));
    store.listRunEvents.mockResolvedValue([]);
    store.listToolCalls.mockResolvedValue(calls);
    store.listToolOutputs.mockResolvedValue([]);

    const summary = await loadRunResumeSummary("r1");
    expect(summary).toContain("- tool7 [completed]: tool7");
    expect(summary).not.toContain("- tool8");
    expect(summary).toContain("- 还有 1 个工具调用未展开。");
    expect(store.listToolOutputs).toHaveBeenCalledTimes(8);
  });

  it("keeps only the last six terminal events", async () => {
    const events = Array.from({ length: 9 }, (_, index) =>
      event({ id: `e${index}`, eventType: "agent_error", payload: `p${index}`, sequence: index }));
    store.listRunEvents.mockResolvedValue(events);
    store.listToolCalls.mockResolvedValue([]);
    store.listToolOutputs.mockResolvedValue([]);

    const summary = await loadRunResumeSummary("r1");
    expect(summary).toContain("- agent_error: p8");
    expect(summary).toContain("- agent_error: p3");
    expect(summary).not.toContain("- agent_error: p2");
    expect(summary.split("\n")).toHaveLength(7);
  });

  it("renders a terminal event that carries no payload", async () => {
    // boundary: `payload` is optional on the row, so an event recorded without
    // one must still produce a labelled line rather than "undefined".
    store.listRunEvents.mockResolvedValue([
      event({ eventType: "agent_end", payload: undefined }),
      event({ id: "e2", eventType: "error", payload: null }),
    ]);
    store.listToolCalls.mockResolvedValue([]);
    store.listToolOutputs.mockResolvedValue([]);

    const summary = await loadRunResumeSummary("r1");
    expect(summary).toBe("最近事件:\n- agent_end: \n- error: ");
  });

  it("degrades a tool whose outputs fail to load into an output-less row", async () => {
    // error-path: one broken output read must not lose the whole summary.
    store.listRunEvents.mockResolvedValue([]);
    store.listToolCalls.mockResolvedValue([
      tool({ id: "t1", name: "broken" }),
      tool({ id: "t2", name: "fine" }),
    ]);
    store.listToolOutputs.mockImplementation(async (_runId: string, toolId: string) => {
      if (toolId === "t1")
        throw new Error("no such tool output");
      return [output({ toolCallId: "t2", content: "ok" })];
    });

    const summary = await loadRunResumeSummary("r1");
    expect(summary).toContain("- broken [completed]: broken");
    expect(summary).toContain("- fine [completed]: fine => ok");
  });

  it("returns an explanatory line when the run history itself cannot be read", async () => {
    store.listRunEvents.mockRejectedValue(new Error("db is locked"));
    store.listToolCalls.mockResolvedValue([]);

    await expect(loadRunResumeSummary("r1")).resolves.toBe("Run 摘要加载失败：db is locked");
    // The prompt must never be empty even for a non-Error rejection.
    store.listRunEvents.mockRejectedValue("disk gone");
    await expect(loadRunResumeSummary("r1")).resolves.toBe("Run 摘要加载失败：disk gone");
  });
});
