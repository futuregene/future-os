// @vitest-environment jsdom
import type { StoredThread } from "../../../integrations/storage/threadStore";
import type { ThreadDialogs } from "./useThreadDialogs";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { setLanguage } from "../../../i18n";
import { generateThreadTitle, renameThread } from "../../../integrations/storage/threadStore";
import { useThreadDialogs } from "./useThreadDialogs";

vi.mock("../../../integrations/storage/threadStore", () => ({
  generateThreadTitle: vi.fn(),
  renameThread: vi.fn(),
  batchDeleteThreads: vi.fn(),
  deleteThread: vi.fn(),
  getThreadCleanupSummary: vi.fn(),
}));
vi.mock("../../../integrations/agent/agentStateCache", () => ({ invalidateAgentState: vi.fn(), prefetchAgentState: vi.fn() }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const thread = { id: "t1", title: "Original", mode: "workspace" } as StoredThread;

beforeEach(() => {
  vi.resetAllMocks();
  setLanguage("en");
});

async function mount() {
  let api: ThreadDialogs;
  function Harness() {
    api = useThreadDialogs({ activeThreadId: "t1", refreshStore: async () => {} });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => root.render(<Harness />));
  return { api: () => api!, close: () => act(() => root.unmount()) };
}

it("only generates on click, fills the draft, and saves only after confirmation", async () => {
  const view = await mount();
  try {
    act(() => view.api().openRename(thread));
    expect(generateThreadTitle).not.toHaveBeenCalled();
    vi.mocked(generateThreadTitle).mockResolvedValue({ title: "Suggested title", model: "session-model" });
    await act(async () => view.api().generateTitle());
    expect(generateThreadTitle).toHaveBeenCalledWith("t1", "en");
    expect(view.api().renameDialog?.value).toBe("Suggested title");
    expect(renameThread).not.toHaveBeenCalled();
    await act(async () => view.api().confirmRename());
    expect(renameThread).toHaveBeenCalledWith({ threadId: "t1", title: "Suggested title" });
  }
  finally { view.close(); }
});

it("ignores results after closing and reopening even the same thread", async () => {
  const view = await mount();
  try {
    let resolve!: (value: { title: string; model: string }) => void;
    vi.mocked(generateThreadTitle).mockImplementation(() => new Promise((r) => {
      resolve = r;
    }));
    act(() => view.api().openRename(thread));
    let task!: Promise<void>;
    act(() => {
      task = view.api().generateTitle();
      void view.api().generateTitle();
    });
    expect(generateThreadTitle).toHaveBeenCalledTimes(1);
    act(() => view.api().setRenameDialog(null));
    act(() => view.api().openRename(thread));
    await act(async () => {
      resolve({ title: "Stale", model: "m" });
      await task;
    });
    expect(view.api().renameDialog?.value).toBe("Original");
  }
  finally { view.close(); }
});

it("keeps the input on error and allows retry", async () => {
  const view = await mount();
  try {
    act(() => view.api().openRename(thread));
    vi.mocked(generateThreadTitle).mockRejectedValue(new Error("offline"));
    await act(async () => view.api().generateTitle());
    expect(view.api().renameDialog?.value).toBe("Original");
    expect(view.api().renameDialog?.error).toBe("offline");
    expect(view.api().renameDialog?.generating).toBe(false);
    expect(renameThread).not.toHaveBeenCalled();
  }
  finally { view.close(); }
});
