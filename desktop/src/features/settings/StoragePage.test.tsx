// @vitest-environment jsdom
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StoragePage } from "./StoragePage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ invokeCommand: vi.fn() }));

vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invokeCommand }));

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  mocks.invokeCommand.mockReset();
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function mount(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  act(() => root.render(node));
  return container;
}

function renderPage() {
  return mount(
    <StoragePage
      archiveRunLogCleanup
      autoArchiveEnabled
      autoArchiveIdleDays={30}
      onChangeAutoArchiveIdleDays={() => {}}
      onToggleArchiveRunLogCleanup={() => {}}
      onToggleAutoArchive={() => {}}
    />,
  );
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement | undefined;
}

describe("storagePage archive settings", () => {
  it("keeps the archive rows and the batch pass on one page", () => {
    const container = renderPage();

    expect(container.textContent).toContain("Clear run logs when archiving");
    expect(container.textContent).toContain("Archive idle conversations automatically");
    expect(container.textContent).toContain("Organize database");
  });
});

describe("storagePage database organization", () => {
  it("processes conversations sequentially and continues past busy and failed items", async () => {
    let resolvePlan!: (value: unknown) => void;
    const plan = new Promise<unknown>((resolve) => {
      resolvePlan = resolve;
    });
    mocks.invokeCommand
      .mockReturnValueOnce(plan)
      .mockResolvedValueOnce({ status: "done", hasMore: true })
      .mockResolvedValueOnce({ status: "done", hasMore: false })
      .mockResolvedValueOnce({ status: "skipped", reason: "busy", hasMore: false })
      .mockResolvedValueOnce({ status: "failed", reason: "invalid body", hasMore: false })
      .mockRejectedValueOnce(new Error("write failure"))
      .mockResolvedValueOnce({ status: "done", hasMore: false })
      .mockResolvedValueOnce({ hasMore: true })
      .mockResolvedValueOnce({ hasMore: false, compacted: true, storage: { totalBytes: 2048 } });
    const container = renderPage();
    expect(container.textContent).toContain("Organize database");
    await act(async () => button(container, "Organize database")!.click());
    await act(async () => resolvePlan({ sessionIds: ["a", "b", "c", "d", "e"], storage: { totalBytes: 8192 } }));
    expect(mocks.invokeCommand.mock.calls).toEqual([
      ["organize_agent_database", { phase: "plan", sessionId: null }],
      ...["a", "a", "b", "c", "d", "e"].map(sessionId => ["organize_agent_database", { phase: "session", sessionId }]),
      ["organize_agent_database", { phase: "finish", sessionId: null }],
      ["organize_agent_database", { phase: "finish", sessionId: null }],
    ]);
    expect(container.textContent).toContain("Processed 5 / 5 conversations");
    expect(container.textContent).toContain("Busy skipped 1 · Failed skipped 2");
    expect(container.textContent).toContain("6.0 KiB");
    expect(container.textContent).toContain("invalid body");
    expect(container.textContent).toContain("write failure");
    expect(container.textContent).toContain("Organization complete.");
    expect(button(container, "Organize database")!.disabled).toBe(false);
  });

  it("reports deferred file compaction and keeps the action available", async () => {
    mocks.invokeCommand
      .mockResolvedValueOnce({ sessionIds: [], storage: { totalBytes: 8192 } })
      .mockResolvedValueOnce({ hasMore: false, compacted: false, storage: { totalBytes: 4096 } });
    const container = renderPage();
    await act(async () => button(container, "Organize database")!.click());
    expect(container.textContent).toContain("file compaction was deferred");
    expect(container.textContent).not.toContain("Organization complete.");
    expect(button(container, "Organize database")!.disabled).toBe(false);
  });
});
