// @vitest-environment jsdom
import type { StoredRun, StoredToolCall } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { RunsPanel } from "./RunsPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

class ResizeObserverStub {
  disconnect() {}
  observe() {}
}

Object.defineProperty(globalThis, "ResizeObserver", {
  configurable: true,
  value: ResizeObserverStub,
});

function run(index: number): StoredRun {
  return {
    id: `run-${index}`,
    threadId: "thread-1",
    status: "completed",
    createdAt: index,
    updatedAt: index,
  };
}

function tool(index: number, name = "shell", input = JSON.stringify({ command: `command-${index}` })): StoredToolCall {
  return {
    id: `tool-${index}`,
    runId: `run-${index}`,
    name,
    kind: name,
    input,
    status: "completed",
    createdAt: index,
  };
}

function props(count: number) {
  const runs = Array.from({ length: count }, (_, index) => run(index));
  return {
    runs,
    toolsByRun: Object.fromEntries(runs.map((entry, index) => [entry.id, [tool(index)]])),
    scope: { threadId: "thread-1", workspaceId: "workspace-1", workspacePath: "/workspace" },
    onArchiveFinished: vi.fn(async () => {}),
    onInspectTool: vi.fn(),
    onTerminateRun: vi.fn(async () => {}),
  };
}

describe("runs panel", () => {
  it("renders the first page and appends another page near the bottom", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    act(() => root.render(<RunsPanel {...props(45)} />));

    expect(container.querySelector("[title=\"command-5\"]")).not.toBeNull();
    expect(container.querySelector("[title=\"command-4\"]")).toBeNull();

    const scroll = container.querySelector<HTMLElement>("[data-runs-scroll]")!;
    Object.defineProperties(scroll, {
      clientHeight: { configurable: true, value: 400 },
      scrollHeight: { configurable: true, value: 2400 },
      scrollTop: { configurable: true, value: 1800, writable: true },
    });
    act(() => scroll.dispatchEvent(new Event("scroll", { bubbles: true })));

    expect(container.querySelector("[title=\"command-0\"]")).not.toBeNull();
    act(() => root.unmount());
    container.remove();
  });

  it("wraps full file targets and clamps only shell commands to ten lines", () => {
    const fileRun = run(1);
    const shellRun = run(2);
    const html = renderToStaticMarkup(
      <RunsPanel
        {...props(0)}
        runs={[fileRun, shellRun]}
        toolsByRun={{
          [fileRun.id]: [tool(1, "edit", JSON.stringify({ path: "/workspace/src/features/complete-name.tsx" }))],
          [shellRun.id]: [tool(2)],
        }}
      />,
    );

    expect(html).toContain("src/features/complete-name.tsx");
    expect(html).toContain("whitespace-pre-wrap font-mono");
    expect(html).not.toContain("truncate font-mono");
    expect(html).toContain("line-clamp-10 whitespace-pre-wrap");
  });
});
