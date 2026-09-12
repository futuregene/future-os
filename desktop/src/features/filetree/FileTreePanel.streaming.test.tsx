// @vitest-environment jsdom
import type { StoredRunEvent } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { buildStreamingPreview, resetRunProjection } from "../agent/threadRunProjection";
import { FileTreePanel } from "./FileTreePanel";

const storage = vi.hoisted(() => ({ listDirectory: vi.fn(async () => []), listRunEventsSince: vi.fn() }));
vi.mock("../../integrations/storage/files", async original => ({
  ...await original<typeof import("../../integrations/storage/files")>(),
  listDirectory: storage.listDirectory,
}));
vi.mock("../../integrations/storage/threadStore", async original => ({
  ...await original<typeof import("../../integrations/storage/threadStore")>(),
  listRunEventsSince: storage.listRunEventsSince,
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("does not rescan directories during 120 seconds of text after a tool completes", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
  const runId = "filetree-text-after-tool";
  resetRunProjection(runId);
  const host = document.createElement("div");
  const root = createRoot(host);
  let sequence = 0;
  async function push(eventType: string, payload: Record<string, unknown>) {
    const event: StoredRunEvent = {
      id: `event-${sequence}`,
      runId,
      sequence: sequence++,
      eventType,
      payload: JSON.stringify(payload),
      createdAt: 1,
    };
    storage.listRunEventsSince.mockResolvedValueOnce([event]);
    await buildStreamingPreview(runId);
  }
  try {
    await act(async () => root.render(<FileTreePanel rootPath="C:/synthetic-filetree-profile" isWorkspace />));
    await act(async () => {
      await push("toolcall_start", { tool_id: "tool-1", tool_name: "write", tool_args: { path: "a.txt" } });
      await push("tool_end", { tool_id: "tool-1", tool_name: "write", text: "done" });
      await vi.advanceTimersByTimeAsync(2000);
    });
    // Initial mount plus the coalesced tool refresh.
    expect(storage.listDirectory).toHaveBeenCalledTimes(2);
    storage.listDirectory.mockClear();
    for (let tick = 0; tick < 1200; tick++) {
      await act(async () => {
        await push(tick % 2 ? "text_chunk" : "thinking_delta", { text: "x" });
        await vi.advanceTimersByTimeAsync(100);
      });
    }
    expect(storage.listDirectory).not.toHaveBeenCalled();
    await act(async () => {
      await push("toolcall_start", { tool_id: "tool-2", tool_name: "shell", tool_args: { command: "generate files" } });
      await push("tool_end", { tool_id: "tool-2", tool_name: "shell", text: "done" });
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect(storage.listDirectory).toHaveBeenCalledTimes(1);
  }
  finally {
    act(() => root.unmount());
    resetRunProjection(runId);
  }
}, 15000);
