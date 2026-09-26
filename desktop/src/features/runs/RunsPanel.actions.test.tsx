// @vitest-environment jsdom
import type { StoredRun, StoredToolCall } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RunsPanel } from "./RunsPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class ResizeObserverStub {
  disconnect() {}
  observe() {}
}

beforeEach(() => {
  Object.defineProperty(globalThis, "ResizeObserver", { configurable: true, value: ResizeObserverStub });
});

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

const SCOPE = { threadId: "thread-1", workspaceId: "workspace-1", workspacePath: "/workspace" };

function run(id: string, overrides: Partial<StoredRun> = {}): StoredRun {
  return {
    id,
    threadId: "thread-1",
    status: "completed",
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  } as StoredRun;
}

function tool(id: string, overrides: Partial<StoredToolCall> = {}): StoredToolCall {
  return {
    id,
    runId: "run-1",
    name: "shell",
    kind: "shell",
    input: JSON.stringify({ command: `echo ${id}` }),
    status: "completed",
    createdAt: 1,
    ...overrides,
  } as StoredToolCall;
}

interface Overrides {
  onArchiveFinished?: (threadId: string) => Promise<void>;
  onInspectTool?: (toolId: string) => void;
  onTerminateRun?: (threadId: string, run: StoredRun) => Promise<void>;
  runs?: StoredRun[];
  scope?: typeof SCOPE | null;
  toolsByRun?: Record<string, StoredToolCall[]>;
}

function mount(overrides: Overrides = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  const props = {
    runs: overrides.runs ?? [run("run-1")],
    toolsByRun: overrides.toolsByRun ?? { "run-1": [tool("tool-1")] },
    scope: overrides.scope === undefined ? SCOPE : overrides.scope,
    onArchiveFinished: overrides.onArchiveFinished ?? vi.fn(async () => {}),
    onInspectTool: overrides.onInspectTool ?? vi.fn(),
    onTerminateRun: overrides.onTerminateRun ?? vi.fn(async () => {}),
  };
  act(() => {
    root.render(<RunsPanel {...props} />);
  });
  return { container, props };
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent?.trim() === text) as HTMLButtonElement | undefined;
}

function rows(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLElement>("[role=button][tabindex='0']")];
}

function scrollContainer(container: HTMLElement) {
  return container.querySelector<HTMLElement>("[data-runs-scroll]")!;
}

describe("runsPanel empty states", () => {
  it.each([
    ["nothing has ever run", true, "No programs have run yet."],
    ["everything was archived", false, "Finished programs have been archived."],
  ] as const)("explains the empty list when %s", (_label, neverRun, expected) => {
    const runs = neverRun ? [] : [run("run-1", { archivedAt: 1 })];
    const { container } = mount({
      runs,
      toolsByRun: neverRun ? {} : { "run-1": [tool("tool-1")] },
    });

    expect(container.textContent).toContain("No background programs");
    expect(container.textContent).toContain(expected);
  });

  it("treats a run without tool events as nothing to show", () => {
    // A run tracks the model reply lifecycle, not a background program: with no
    // tool call there is no row, so the panel stays empty.
    const { container } = mount({ runs: [run("run-1")], toolsByRun: {} });

    expect(container.textContent).toContain("No background programs");
  });

  it("keeps the empty list scrollable so the floating scrollbar still mounts", () => {
    const { container } = mount({ runs: [], toolsByRun: {} });

    const scroller = container.querySelector<HTMLElement>(".floating-scrollbar");
    expect(scroller).not.toBeNull();
    expect(scroller!.textContent).toContain("No background programs");
  });
});

describe("runsPanel header and archive", () => {
  it("counts running and finished commands, not runs", () => {
    const active = run("run-live", { status: "running" });
    const done = run("run-done", { status: "completed" });
    const { container } = mount({
      runs: [active, done],
      toolsByRun: {
        "run-live": [tool("t-live", { runId: "run-live", status: "running" })],
        "run-done": [tool("t-done", { runId: "run-done" })],
      },
    });

    expect(container.textContent).toContain("1 running / 1 finished");
    // The terminate control rides the still-running command of the live run.
    expect(button(container, "Terminate")).toBeTruthy();
  });

  it("archives finished programs through the scope's thread", async () => {
    const onArchiveFinished = vi.fn(async () => {});
    const { container } = mount({ onArchiveFinished });

    const archive = button(container, "Archive finished")!;
    expect(archive.disabled).toBe(false);

    await act(async () => {
      archive.click();
    });

    expect(onArchiveFinished).toHaveBeenCalledExactlyOnceWith("thread-1");
    expect(button(container, "Archive finished")!.disabled).toBe(false);
  });

  it("disables archiving when nothing is finished", () => {
    const active = run("run-live", { status: "running" });
    const { container } = mount({
      runs: [active],
      toolsByRun: { "run-live": [tool("t-live", { runId: "run-live", status: "running" })] },
    });

    expect(button(container, "Archive finished")!.disabled).toBe(true);
  });

  it("reports an archive failure without dropping the finished rows", async () => {
    const onArchiveFinished = vi.fn(async () => {
      throw new Error("archive lock held");
    });
    const { container } = mount({ onArchiveFinished });

    await act(async () => {
      button(container, "Archive finished")!.click();
    });

    expect(container.textContent).toContain("archive lock held");
    expect(rows(container)).toHaveLength(1);
  });

  it("stringifies a non-Error archive failure", async () => {
    const onArchiveFinished = vi.fn(async () => {
      // eslint-disable-next-line no-throw-literal -- the rejection is deliberately a non-Error: this test pins that the panel stringifies it.
      throw "disk full";
    });
    const { container } = mount({ onArchiveFinished });

    await act(async () => {
      button(container, "Archive finished")!.click();
    });

    expect(container.textContent).toContain("disk full");
  });

  it("does nothing when the panel has no conversation scope", async () => {
    const onArchiveFinished = vi.fn(async () => {});
    const { container } = mount({ onArchiveFinished, scope: null });

    // Fault injection: drop React's disabled gate so the handler itself runs;
    // without a scope it must refuse to archive someone else's thread.
    const archive = button(container, "Archive finished")!;
    archive.removeAttribute("disabled");
    await act(async () => {
      archive.click();
    });

    expect(onArchiveFinished).not.toHaveBeenCalled();
  });
});

describe("runsPanel terminate flow", () => {
  function livePanel(onTerminateRun?: (threadId: string, run: StoredRun) => Promise<void>) {
    const active = run("run-live", { status: "running" });
    return mount({
      runs: [active],
      toolsByRun: { "run-live": [tool("t-live", { runId: "run-live", status: "running" })] },
      onTerminateRun,
    });
  }

  it("asks for confirmation and terminates through the scope's thread", async () => {
    const onTerminateRun = vi.fn<(threadId: string, run: StoredRun) => Promise<void>>(async () => {});
    const { container } = livePanel(onTerminateRun);

    act(() => button(container, "Terminate")!.click());
    expect(container.textContent).toContain("Terminate this program?");

    await act(async () => {
      // The confirm row's danger button carries the stop icon and the label.
      button(container, "Terminate")!.click();
    });

    expect(onTerminateRun).toHaveBeenCalledTimes(1);
    expect(onTerminateRun.mock.calls[0]![0]).toBe("thread-1");
    expect(onTerminateRun.mock.calls[0]![1]).toMatchObject({ id: "run-live" });
    expect(container.textContent).not.toContain("Terminate this program?");
  });

  it("can cancel the confirmation", () => {
    const { container } = livePanel();

    act(() => button(container, "Terminate")!.click());
    expect(container.textContent).toContain("Terminate this program?");

    act(() => button(container, "Cancel")!.click());

    expect(container.textContent).not.toContain("Terminate this program?");
    expect(button(container, "Terminate")).toBeTruthy();
  });

  it("shows the stopping label while the request is in flight and reports a failure", async () => {
    let rejectTerminate!: (reason: unknown) => void;
    const pending = new Promise<void>((_resolve, reject) => {
      rejectTerminate = reject;
    });
    const onTerminateRun = vi.fn(() => pending);
    const { container } = livePanel(onTerminateRun);

    act(() => button(container, "Terminate")!.click());
    await act(async () => {
      button(container, "Terminate")!.click();
    });
    expect(container.textContent).toContain("Stopping");

    await act(async () => {
      rejectTerminate(new Error("agent refused the abort"));
      await Promise.resolve();
    });

    expect(container.textContent).toContain("agent refused the abort");
    // The confirmation stays open so the user can retry.
    expect(container.textContent).toContain("Terminate this program?");
  });

  it("stringifies a non-Error termination failure", async () => {
    const onTerminateRun = vi.fn(async () => {
      // eslint-disable-next-line no-throw-literal -- the rejection is deliberately a non-Error: this test pins that the panel stringifies it.
      throw "not allowed";
    });
    const { container } = livePanel(onTerminateRun);

    act(() => button(container, "Terminate")!.click());
    await act(async () => {
      button(container, "Terminate")!.click();
    });

    expect(container.textContent).toContain("not allowed");
  });

  it("falls back to the run's own thread when the panel has no scope", async () => {
    const active = run("run-live", { status: "running" });
    const onTerminateRun = vi.fn<(threadId: string, run: StoredRun) => Promise<void>>(async () => {});
    const { container } = mount({
      runs: [active],
      toolsByRun: { "run-live": [tool("t-live", { runId: "run-live", status: "running" })] },
      onTerminateRun,
      scope: null,
    });

    act(() => button(container, "Terminate")!.click());
    await act(async () => {
      button(container, "Terminate")!.click();
    });

    expect(onTerminateRun.mock.calls[0]![0]).toBe("thread-1");
  });
});

describe("runsPanel row interaction", () => {
  it("inspects a tool from the row, the chevron and the keyboard", () => {
    const onInspectTool = vi.fn();
    const { container } = mount({ onInspectTool });
    const row = rows(container)[0]!;

    act(() => row.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(onInspectTool).toHaveBeenLastCalledWith("tool-1");

    act(() => container.querySelector<HTMLButtonElement>("button[aria-label='View details']")!.click());
    expect(onInspectTool).toHaveBeenCalledTimes(2);

    act(() => row.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(onInspectTool).toHaveBeenCalledTimes(3);

    act(() => row.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })));
    expect(onInspectTool).toHaveBeenCalledTimes(4);

    // Any other key is ignored (no scroll hijacking, no double inspect).
    act(() => row.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true })));
    expect(onInspectTool).toHaveBeenCalledTimes(4);
  });

  it("does not inspect when the activation lands on a nested control", () => {
    const onInspectTool = vi.fn();
    const active = run("run-live", { status: "running" });
    const { container } = mount({
      runs: [active],
      toolsByRun: { "run-live": [tool("t-live", { runId: "run-live", status: "running" })] },
      onInspectTool,
    });
    const row = rows(container)[0]!;
    const terminate = button(container, "Terminate")!;

    // Clicking the button must ask for confirmation, not open the inspector.
    act(() => terminate.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(onInspectTool).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Terminate this program?");

    // The keyboard guard matters too: Enter on an inner button is the button's.
    const cancel = button(container, "Cancel")!;
    act(() => cancel.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    act(() => row.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(onInspectTool).toHaveBeenCalledTimes(1);
  });
});

describe("runsPanel row content", () => {
  it("shows a shell command verbatim and a file target workspace-relative", () => {
    const shellRun = run("run-shell");
    const fileRun = run("run-file");
    const { container } = mount({
      runs: [shellRun, fileRun],
      toolsByRun: {
        "run-shell": [tool("t-shell", { runId: "run-shell", input: JSON.stringify({ command: "npm test" }) })],
        "run-file": [tool("t-file", { runId: "run-file", name: "write", kind: "write", input: JSON.stringify({ path: "/workspace/src/index.ts" }) })],
      },
    });

    expect(container.textContent).toContain("npm test");
    expect(container.textContent).toContain("src/index.ts");
    expect(container.textContent).not.toContain("/workspace/src/index.ts");
  });

  it("keeps an absolute path for a file outside the workspace", () => {
    const fileRun = run("run-file");
    const { container } = mount({
      runs: [fileRun],
      toolsByRun: {
        "run-file": [tool("t-file", { runId: "run-file", name: "write", kind: "write", input: JSON.stringify({ path: "/elsewhere/notes.md" }) })],
      },
    });

    expect(container.textContent).toContain("/elsewhere/notes.md");
  });

  it("renders every status variant's label next to the tool name", () => {
    const statuses = ["running", "completed", "failed", "cancelled"] as const;
    const runs = statuses.map((_status, index) => run(`run-${index}`, { status: "completed" }));
    const { container } = mount({
      runs,
      toolsByRun: Object.fromEntries(statuses.map((status, index) => [
        `run-${index}`,
        [tool(`t-${index}`, { runId: `run-${index}`, status })],
      ])),
    });

    // A tool still marked running inside a finished run is "Interrupted": the
    // panel cannot tell a user abort from a real failure.
    expect(container.textContent).toContain("Interrupted");
    expect(container.textContent).toContain("Completed");
    expect(container.textContent).toContain("Failed");
    expect(container.textContent).toContain("Cancelled");
  });

  it("falls back to a generic label for a tool the agent did not name", () => {
    const { container } = mount({
      runs: [run("run-1")],
      toolsByRun: {
        "run-1": [tool("t-1", { name: "", input: JSON.stringify({ command: "ls" }) })],
      },
    });

    expect(container.textContent).toContain("Tool");
    expect(container.textContent).toContain("ls");
  });

  it("shows a tool's own label when the input has no displayable field", () => {
    const { container } = mount({
      runs: [run("run-1")],
      toolsByRun: {
        "run-1": [
          tool("t-null", { input: null }),
          tool("t-json", { input: "{\"partial\":" }),
        ],
      },
    });

    // `null` input ⇒ the tool label stands in; an unparseable JSON fragment ⇒ a
    // blank line (not a raw blob) while the call is still streaming.
    const texts = rows(container).map(row => row.textContent);
    expect(texts.some(text => text?.includes("Shell"))).toBe(true);
    expect(texts.some(text => text?.includes("{"))).toBe(false);
  });

  it("orders live commands above finished ones, newest first within each group", () => {
    const active = run("run-live", { status: "running" });
    const done = run("run-done", { status: "completed" });
    const { container } = mount({
      runs: [done, active],
      toolsByRun: {
        "run-live": [tool("t-old", { runId: "run-live", status: "running", startedAt: 10, input: JSON.stringify({ command: "old-live" }) })],
        "run-done": [tool("t-new", { runId: "run-done", startedAt: 99, input: JSON.stringify({ command: "new-done" }) })],
      },
    });

    const titles = rows(container).map(row => row.querySelector("[title]")?.getAttribute("title"));
    expect(titles).toEqual(["old-live", "new-done"]);
  });
});

describe("runsPanel pagination guards", () => {
  it("does not page when every row is already rendered", () => {
    // Five rows fit in one page, so a scroll near the bottom must not schedule a
    // redundant page (which would re-render the whole list for nothing).
    const { container } = mount({
      runs: [run("run-1")],
      toolsByRun: { "run-1": [tool("t-1"), tool("t-2"), tool("t-3")] },
    });
    const scroll = scrollContainer(container);
    Object.defineProperties(scroll, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 1000 },
      scrollTop: { configurable: true, value: 590, writable: true },
    });

    act(() => scroll.dispatchEvent(new Event("scroll", { bubbles: true })));

    expect(rows(container)).toHaveLength(3);
  });

  it("keeps the rendered window when the content still overflows after paging", () => {
    const runs = Array.from({ length: 90 }, (_, index) => run(`run-${index}`, { createdAt: index }));
    const { container } = mount({
      runs,
      toolsByRun: Object.fromEntries(runs.map((entry, index) => [
        entry.id,
        [tool(`t-${index}`, { runId: entry.id, startedAt: index })],
      ])),
    });
    expect(rows(container)).toHaveLength(40);

    const scroll = scrollContainer(container);
    // 4000 - 400 - 3600 = 0 remaining, inside the 240px pre-fetch threshold.
    Object.defineProperties(scroll, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 4000 },
      scrollTop: { configurable: true, value: 3600, writable: true },
    });
    act(() => scroll.dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(rows(container)).toHaveLength(80);

    // Scrolling again in the same scope appends the third page.
    act(() => {
      scroll.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    expect(rows(container)).toHaveLength(90);
  });
});
