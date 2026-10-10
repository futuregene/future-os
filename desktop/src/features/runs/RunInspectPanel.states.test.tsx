// @vitest-environment jsdom
import type { StoredRun, StoredToolCall, StoredToolOutput } from "../../integrations/storage/types";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listToolOutputs } from "../../integrations/storage/threadStore";
import { onFutureEvent } from "../../lib/futureEvents";
import { RunInspectPanel } from "./RunInspectPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("../../integrations/storage/threadStore", async importOriginal => ({
  ...await importOriginal<typeof import("../../integrations/storage/threadStore")>(),
  listToolOutputs: vi.fn(async () => []),
}));

const listToolOutputsMock = vi.mocked(listToolOutputs);

function run(overrides: Partial<StoredRun> = {}): StoredRun {
  return {
    id: "run-abcdef123456",
    status: "completed",
    createdAt: 1,
    updatedAt: 2,
    ...overrides,
  } as StoredRun;
}

function tool(id: string, overrides: Partial<StoredToolCall> = {}): StoredToolCall {
  return {
    id,
    runId: "run-abcdef123456",
    name: "shell",
    kind: "shell",
    input: JSON.stringify({ command: `echo ${id}` }),
    status: "completed",
    createdAt: 1,
    ...overrides,
  } as StoredToolCall;
}

function output(id: string, kind: string, content: string | null): StoredToolOutput {
  return { id, toolCallId: "tool-1", kind, content, createdAt: 1 };
}

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  listToolOutputsMock.mockReset();
  listToolOutputsMock.mockResolvedValue([]);
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(input: { compact?: boolean; onBack?: () => void; run?: StoredRun; tools?: StoredToolCall[] } = {}) {
  const props = input;
  const onBack = props.onBack ?? vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  const view = (next: { run?: StoredRun; tools?: StoredToolCall[] } = {}) => {
    act(() => {
      root.render(
        <RunInspectPanel
          compact={props.compact}
          onBack={onBack}
          run={next.run ?? props.run ?? run()}
          tools={next.tools ?? props.tools ?? [tool("tool-1")]}
        />,
      );
    });
  };
  view();
  return { container, onBack, view };
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent?.trim() === text) as HTMLButtonElement | undefined;
}

function type(container: HTMLElement, value: string) {
  const input = container.querySelector<HTMLInputElement>("input")!;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("runInspectPanel full view", () => {
  it("shows the run summary, model, tool count and time range", () => {
    const { container } = mount({
      run: run({ id: "run-abcdef123456", status: "failed", modelId: "future/gpt-5", startedAt: 100, endedAt: 200 }),
      tools: [tool("tool-1"), tool("tool-2")],
    });

    expect(container.textContent).toContain("run-abcdef123456");
    expect(container.textContent).toContain("future/gpt-5");
    expect(container.textContent).toContain("Tool Calls");
    expect(container.textContent).toContain("failed");
  });

  it("falls back to the created time and a dash when the run has no start or model", () => {
    const { container } = mount({ run: run({ startedAt: undefined, modelId: null }) });

    expect(container.textContent).toContain("-");
  });

  it("banners the run's failure with its typed label", () => {
    const { container } = mount({
      run: run({ status: "failed", errorMessage: "the model refused", errorType: "model_failed" }),
    });

    expect(container.textContent).toContain("the model refused");
    expect(container.textContent).toContain("Model failed");
  });

  it.each([
    ["failed", "Retry"],
    ["cancelled", "Retry"],
  ] as const)("offers recovery for a %s run and emits the request", async (status, label) => {
    const events: CustomEvent[] = [];
    const off = onFutureEvent("recover-run", detail => events.push(detail as never));
    try {
      const { container } = mount({ run: run({ status, triggerMessageId: "msg-1" }) });

      expect(button(container, label)).toBeTruthy();
      act(() => button(container, "Continue")!.click());
      act(() => button(container, label)!.click());

      expect(events.map(event => (event as unknown as { action: string }).action)).toEqual(["continue", "retry"]);
      expect(events[0]).toMatchObject({ runId: "run-abcdef123456", triggerMessageId: "msg-1" });
    }
    finally {
      off();
    }
  });

  it("offers no recovery for a completed run", () => {
    const { container } = mount({ run: run({ status: "completed" }) });

    expect(button(container, "Retry")).toBeUndefined();
    expect(button(container, "Continue")).toBeUndefined();
  });

  it("goes back through the back button", () => {
    const { container, onBack } = mount();

    act(() => button(container, "Back")!.click());

    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("reports an output-loading failure without dropping the tool card", async () => {
    listToolOutputsMock.mockRejectedValue(new Error("outputs unavailable"));
    const { container } = mount();

    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("Tool Calls");
    expect(container.textContent).toContain("echo tool-1");
  });

  it("shows the empty-tool state and the no-match state separately", () => {
    const empty = mount({ tools: [] });
    expect(empty.container.textContent).toContain("No tool calls recorded.");

    const { container } = mount({ tools: [tool("tool-1")] });
    type(container, "nothing matches this");
    expect(container.textContent).toContain("No matching tool calls.");
    expect(container.textContent).not.toContain("echo tool-1");

    type(container, "");
    expect(container.textContent).toContain("echo tool-1");
  });

  it("searches tool names, kinds, statuses and output text", async () => {
    // Outputs are per tool, as the store returns them.
    listToolOutputsMock.mockImplementation(async (_runId: string, toolId: string) => (toolId === "tool-1"
      ? [output("out-1", "text", "compiled 12 modules")]
      : []));
    const { container } = mount({
      tools: [tool("tool-1", { name: "shell" }), tool("tool-2", { name: "write", kind: "write" })],
    });
    await act(async () => {
      await Promise.resolve();
    });

    type(container, "WRITE");
    expect(container.textContent).not.toContain("echo tool-1");
    expect(container.textContent).toContain("echo tool-2");

    type(container, "12 modules");
    expect(container.textContent).toContain("echo tool-1");
    expect(container.textContent).not.toContain("echo tool-2");

    type(container, "completed");
    expect(container.textContent).toContain("echo tool-1");
    expect(container.textContent).toContain("echo tool-2");
  });

  it("orders the tool cards by start time, not by prop order", () => {
    const { container } = mount({
      tools: [
        tool("tool-late", { startedAt: 90, input: JSON.stringify({ command: "echo late" }) }),
        tool("tool-early", { startedAt: 10, input: JSON.stringify({ command: "echo early" }) }),
      ],
    });

    const text = container.textContent!;
    expect(text.indexOf("echo early")).toBeLessThan(text.indexOf("echo late"));
  });
});

describe("runInspectPanel compact view", () => {
  it("shows only the single tool's status and hides the run chrome", () => {
    const { container } = mount({
      compact: true,
      tools: [tool("tool-1", { status: "failed" })],
      run: run({ status: "failed", errorMessage: "boom" }),
    });

    expect(container.textContent).toContain("Failed");
    expect(container.textContent).not.toContain("Tool Calls");
    expect(container.textContent).not.toContain("boom");
    expect(container.querySelector("h3")).toBeNull();
  });

  it("explains a compact panel with no tools", () => {
    const { container } = mount({ compact: true, tools: [] });

    expect(container.textContent).toContain("No tool calls recorded.");
  });

  it("uses the neutral badge tone for a running tool", () => {
    const { container } = mount({ compact: true, tools: [tool("tool-1", { status: "running" })] });

    expect(container.textContent).toContain("Running");
  });

  it("offers a compact back button", () => {
    const { container, onBack } = mount({ compact: true });

    act(() => button(container, "Back")!.click());

    expect(onBack).toHaveBeenCalledTimes(1);
  });
});

describe("runInspectPanel tool details", () => {
  it("renders the command, its cwd and exit status as fields", async () => {
    listToolOutputsMock.mockResolvedValue([
      output("out-1", "text", JSON.stringify({ stdout: "ok", cwd: "/workspace", exitStatus: 2 })),
    ]);
    const { container } = mount({
      tools: [tool("tool-1", { startedAt: 100, endedAt: 350, input: JSON.stringify({ command: "npm test", workdir: "/workspace" }) })],
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("npm test");
    expect(container.textContent).toContain("/workspace");
    expect(container.textContent).toContain("2");
    // A 250ms run still shows a duration rather than nothing.
    expect(container.textContent).toContain("250");
  });

  it("labels a file target and shows the written content for a write tool", () => {
    const { container } = mount({
      tools: [tool("tool-1", {
        name: "write",
        kind: "write",
        input: JSON.stringify({ path: "src/index.ts", content: "export const x = 1;" }),
      })],
    });

    expect(container.textContent).toContain("Target");
    expect(container.textContent).toContain("src/index.ts");
    expect(container.textContent).toContain("Content");
    expect(container.textContent).toContain("export const x = 1;");
  });

  it("joins every hunk of a batched edit", () => {
    const { container } = mount({
      tools: [tool("tool-1", {
        name: "edit",
        kind: "edit",
        input: JSON.stringify({ path: "a.ts", edits: [{ newText: "first" }, { newString: "second" }, "not an object"] }),
      })],
    });

    expect(container.textContent).toContain("first");
    expect(container.textContent).toContain("second");
  });

  it("accepts the direct newText field too", () => {
    const { container } = mount({
      tools: [tool("tool-1", { name: "edit", kind: "edit", input: JSON.stringify({ newText: "replacement" }) })],
    });

    expect(container.textContent).toContain("replacement");
  });

  it("shows the raw input when neither command nor path is parseable", () => {
    const { container } = mount({
      tools: [tool("tool-1", { name: "mcp-thing", kind: "mcp", input: "not json at all" })],
    });

    expect(container.textContent).toContain("Input");
    expect(container.textContent).toContain("not json at all");
  });

  it("falls back to the no-input label and the generic tool name", () => {
    const { container } = mount({
      tools: [tool("tool-1", { name: "  ", kind: "", input: null })],
    });

    expect(container.textContent).toContain("No input");
    expect(container.textContent).toContain("Tool");
  });

  it("renders an unparseable tool input as a blank command line", () => {
    const { container } = mount({
      tools: [tool("tool-1", { input: "{\"command\":\"partial" })],
    });

    expect(container.textContent).toContain("Input");
  });
});

describe("runInspectPanel outputs", () => {
  it("labels a text output and offers expand/collapse for a long one", async () => {
    listToolOutputsMock.mockResolvedValue([output("out-1", "text", Array.from({ length: 12 }, (_, i) => `line ${i}`).join("\n"))]);
    const { container } = mount();
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("Output");
    const expand = button(container, "Expand")!;
    expect(expand).toBeTruthy();

    act(() => expand.click());
    expect(button(container, "Collapse")).toBeTruthy();

    act(() => button(container, "Collapse")!.click());
    expect(button(container, "Expand")).toBeTruthy();
  });

  it("labels an error output and hides the toggle for a short one", async () => {
    listToolOutputsMock.mockResolvedValue([output("out-1", "error", "boom")]);
    const { container } = mount();
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("Error");
    expect(button(container, "Expand")).toBeUndefined();
  });

  it("prints an unknown output kind as-is and falls back to the kind for empty content", async () => {
    listToolOutputsMock.mockResolvedValue([output("out-1", "image", null), output("out-2", "text", null)]);
    const { container } = mount();
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("image");
    expect(container.textContent).toContain("Output");
  });

  it("drops a structured output from the preview list but keeps its fields", async () => {
    listToolOutputsMock.mockResolvedValue([
      output("out-1", "text", JSON.stringify({ stdout: "compiled", exitStatus: 0 })),
      output("out-2", "text", "plain tail"),
    ]);
    const { container } = mount();
    await act(async () => {
      await Promise.resolve();
    });

    // The JSON envelope is summarized into the field grid; only the human tail
    // is previewed verbatim.
    expect(container.textContent).toContain("plain tail");
    expect(container.textContent).toContain("compiled");
    expect(container.textContent).not.toContain("{\"stdout\"");
  });

  it("reports a duration from the output when the tool has no start/end times", async () => {
    listToolOutputsMock.mockResolvedValue([
      output("out-1", "text", JSON.stringify({ stdout: "done", durationMs: 1500 })),
    ]);
    const { container } = mount({ tools: [tool("tool-1", { startedAt: undefined, endedAt: undefined })] });
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("Duration");
    expect(container.textContent).toContain("1.5");
  });

  it("keeps a non-numeric duration verbatim", async () => {
    listToolOutputsMock.mockResolvedValue([
      output("out-1", "text", JSON.stringify({ stdout: "done", duration: "about a minute" })),
    ]);
    const { container } = mount({ tools: [tool("tool-1", { startedAt: undefined, endedAt: undefined })] });
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain("about a minute");
  });

  it("summarizes a stderr-only envelope into its own block, with no preview", async () => {
    listToolOutputsMock.mockResolvedValue([output("out-1", "text", JSON.stringify({ stderr: "warned" }))]);
    const { container } = mount();
    await act(async () => {
      await Promise.resolve();
    });

    // Structured output contributes its own field block, not a preview row.
    expect(container.textContent).toContain("warned");
    expect(container.textContent).not.toContain("Output");
    expect(container.textContent).not.toContain("{\"stderr\"");
  });
});
