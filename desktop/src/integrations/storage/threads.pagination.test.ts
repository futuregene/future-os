import { beforeEach, describe, expect, it, vi } from "vitest";
import { TAIL_WAIT_MS } from "./historyTransfer";
import { generateThreadTitle, getSessionEntriesPage } from "./threads";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();
const historyMock = vi.fn<(args: unknown, waitMs?: number) => Promise<unknown>>();

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));
vi.mock("./historyTransfer", async () => {
  const actual = await vi.importActual<typeof import("./historyTransfer")>("./historyTransfer");
  return {
    ...actual,
    readHistoryTransfer: (args: unknown, _isCurrent?: unknown, waitMs?: number) => historyMock(args, waitMs),
  };
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({});
  historyMock.mockReset();
  historyMock.mockResolvedValue({});
});

describe("generateThreadTitle", () => {
  it("asks the agent for a title in the requested language", async () => {
    const result = { title: "Deploy checklist", model: "future/gpt-5" };
    invokeMock.mockResolvedValue(result);

    await expect(generateThreadTitle("t1", "en")).resolves.toBe(result);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("generate_thread_title", { threadId: "t1", language: "en" });
  });

  it("forwards a non-English language and a CJK thread id verbatim", async () => {
    invokeMock.mockResolvedValue({ title: "部署清单", model: "m" });

    await generateThreadTitle("会话-1", "zh");

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("generate_thread_title", { threadId: "会话-1", language: "zh" });
  });

  it("propagates a failure so the caller can keep the default title", async () => {
    invokeMock.mockRejectedValue(new Error("model unavailable"));

    await expect(generateThreadTitle("t1", "en")).rejects.toThrow("model unavailable");
  });
});

describe("getSessionEntriesPage", () => {
  it("defaults to the newest page of ten entries, waiting out the running transfers", async () => {
    historyMock.mockResolvedValue({ entries: [], hasMore: false });

    await getSessionEntriesPage("t1");

    expect(historyMock).toHaveBeenCalledExactlyOnceWith({
      threadId: "t1",
      before: null,
      limit: 10,
    }, TAIL_WAIT_MS);
  });

  it("paginates backwards with an explicit cursor and limit", async () => {
    historyMock.mockResolvedValue({ entries: [{ id: 1 }], hasMore: true });

    await getSessionEntriesPage("t1", 42, 25);

    // An older page is asked for on top of a transcript that is already on
    // screen, so it keeps the short admission window (undefined selects it).
    expect(historyMock).toHaveBeenCalledExactlyOnceWith({
      threadId: "t1",
      before: 42,
      limit: 25,
    }, undefined);
  });

  it("keeps before=0 as a real cursor rather than dropping it as falsy", async () => {
    await getSessionEntriesPage("t1", 0, 1);

    expect(historyMock).toHaveBeenCalledExactlyOnceWith({
      threadId: "t1",
      before: 0,
      limit: 1,
    }, undefined);
  });

  it("returns the authoritative page shape unchanged", async () => {
    const page = { entries: [{ role: "user" }], hasMore: true, nextBefore: 7 };
    historyMock.mockResolvedValue(page);

    await expect(getSessionEntriesPage("t1")).resolves.toBe(page);
  });

  it("propagates a read failure (the transcript keeps what it already shows)", async () => {
    historyMock.mockRejectedValue(new Error("database is locked"));

    await expect(getSessionEntriesPage("t1")).rejects.toThrow("database is locked");
  });
});
